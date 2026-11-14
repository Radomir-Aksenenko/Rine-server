//! Java plugin bridge for Rine.
//!
//! A single JVM runs on a dedicated `rine-plugin` thread. The game never calls into Java on
//! its tick path:
//!
//! * [`PluginHost::post`] queues events. [`PluginHost::tick`] hands the queue to the plugin
//!   thread without waiting and returns whatever actions the plugins produced earlier.
//! * [`PluginHost::sync_cancel`] is the only blocking call. The game uses it only when
//!   [`PluginHost::wants_sync`] says a plugin listens to that kind, and it gives up after 50 ms.
//!
//! The plugin API (`rine.api`, `org.bukkit` compatibility, and the `rine.host.Host` JNI entry
//! points) is compiled from `plugin-api/src` by `plugin-api/build.ps1` and embedded here.
//! Run that script before building this crate.
//!
//! Only one [`PluginHost`] can exist per process, because a process can create only one JVM.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{anyhow, Context};
use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use jni::objects::{JClass, JObject, JObjectArray, JString};
use jni::{jni_sig, jni_str, Env, InitArgsBuilder, JNIVersion, JValue, JavaVM};
use parking_lot::{Mutex, RwLock};

/// rine-api.jar produced by `plugin-api/build.ps1`.
static API_JAR: &[u8] = include_bytes!("../../../plugin-api/rine-api.jar");

/// Field separator in the packed string format shared with `rine.api.Wire` (U+0001).
const SEP: char = '\u{1}';

/// How long the game waits for a synchronous plugin check.
const SYNC_TIMEOUT: Duration = Duration::from_millis(50);

/// Java home used when `JAVA_HOME` is unset or does not contain a JVM.
#[cfg(windows)]
const DEFAULT_JAVA_HOME: &str = r"C:\Program Files\Java\jdk-25.0.4";
#[cfg(not(windows))]
const DEFAULT_JAVA_HOME: &str = "/usr/lib/jvm/default";

#[cfg(windows)]
const LIBJVM_REL: &str = r"bin\server\jvm.dll";
#[cfg(target_os = "macos")]
const LIBJVM_REL: &str = "lib/server/libjvm.dylib";
#[cfg(all(not(windows), not(target_os = "macos")))]
const LIBJVM_REL: &str = "lib/server/libjvm.so";

/// Game -> plugins. Queued by [`PluginHost::post`] and delivered in one batch per tick.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Join { name: String, uuid: String },
    Quit { name: String },
    /// Asynchronous notification, sent after the broadcast.
    Chat { name: String, message: String },
    /// A command the server did not handle itself.
    Command { name: String, line: String },
    BlockBreak { name: String, x: i32, y: i32, z: i32, block: String },
    BlockPlace { name: String, x: i32, y: i32, z: i32, block: String },
}

/// Plugins -> game. Applied by the game at the start of the next tick.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Broadcast(String),
    Message { player: String, text: String },
    SetBlock { x: i32, y: i32, z: i32, block: String },
    Teleport { player: String, x: f64, y: f64, z: f64 },
    Give { player: String, item: String, count: i32 },
    Kick { player: String, reason: String },
    RunCommand { player: String, line: String },
}

/// Event kinds that the game may check synchronously.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncKind {
    Chat,
    BlockBreak,
    BlockPlace,
    Command,
}

impl SyncKind {
    /// Bit in the mask published by the plugin thread. Must match `rine.host.Host.MASK_*`.
    fn bit(self) -> u32 {
        match self {
            SyncKind::Chat => 1,
            SyncKind::BlockBreak => 2,
            SyncKind::BlockPlace => 4,
            SyncKind::Command => 8,
        }
    }
}

enum Msg {
    Batch(Vec<String>),
    Sync(String, Sender<bool>),
    Shutdown,
}

pub struct PluginHost {
    tx: Sender<Msg>,
    pending: Mutex<Vec<String>>,
    actions: Receiver<Action>,
    mask: Arc<AtomicU32>,
    commands: Arc<RwLock<Vec<String>>>,
    thread: Option<JoinHandle<()>>,
    done: Receiver<()>,
}

impl PluginHost {
    /// Starts the JVM on a dedicated thread, loads every `*.jar` in `dir`, and calls `onEnable`.
    ///
    /// Returns `Ok(None)` if `dir` has no jars or no JVM can be found (a warning is logged).
    /// Blocks until plugin loading has finished.
    pub fn start(dir: &Path) -> anyhow::Result<Option<PluginHost>> {
        let jars = list_jars(dir)?;
        if jars.is_empty() {
            return Ok(None);
        }
        let api_jar = write_api_jar()?;

        let (tx, rx) = unbounded::<Msg>();
        let (action_tx, action_rx) = unbounded::<Action>();
        let mask = Arc::new(AtomicU32::new(0));
        let commands = Arc::new(RwLock::new(Vec::new()));
        let (ready_tx, ready_rx) = bounded::<Result<bool, String>>(1);
        let (done_tx, done_rx) = bounded::<()>(1);

        let thread = std::thread::Builder::new()
            .name("rine-plugin".into())
            .spawn({
                let mask = Arc::clone(&mask);
                let commands = Arc::clone(&commands);
                move || {
                    run(Startup {
                        jars,
                        api_jar,
                        rx,
                        actions: action_tx,
                        mask,
                        commands,
                        ready: ready_tx,
                    });
                    let _ = done_tx.send(());
                }
            })
            .context("spawning rine-plugin thread")?;

        match ready_rx.recv() {
            Ok(Ok(true)) => Ok(Some(PluginHost {
                tx,
                pending: Mutex::new(Vec::new()),
                actions: action_rx,
                mask,
                commands,
                thread: Some(thread),
                done: done_rx,
            })),
            Ok(Ok(false)) => {
                let _ = thread.join();
                Ok(None)
            }
            Ok(Err(msg)) => Err(anyhow!("plugin JVM failed to start: {msg}")),
            Err(_) => Err(anyhow!("plugin thread exited before reporting readiness")),
        }
    }

    /// Queues an event. Never blocks on Java.
    pub fn post(&self, e: Event) {
        self.pending.lock().push(encode_event(&e));
    }

    /// Called once per game tick. Sends the queued events to the plugin thread without waiting
    /// for them, and returns the actions produced since the previous call.
    pub fn tick(&self) -> Vec<Action> {
        let batch = std::mem::take(&mut *self.pending.lock());
        if !batch.is_empty() {
            let _ = self.tx.send(Msg::Batch(batch));
        }
        self.actions.try_iter().collect()
    }

    /// True if at least one plugin listens to this kind, so [`sync_cancel`](Self::sync_cancel)
    /// is worth calling. Updated by the plugin thread after each batch.
    pub fn wants_sync(&self, kind: SyncKind) -> bool {
        self.mask.load(Ordering::Acquire) & kind.bit() != 0
    }

    /// Delivers `e` to plugins synchronously. Returns true if a plugin cancelled it.
    ///
    /// Delivery happens here, so do not also `post` the same event. Returns false after 50 ms
    /// if the plugins are too slow.
    pub fn sync_cancel(&self, e: &Event) -> bool {
        let (reply_tx, reply_rx) = bounded::<bool>(1);
        if self.tx.send(Msg::Sync(encode_event(e), reply_tx)).is_err() {
            return false;
        }
        match reply_rx.recv_timeout(SYNC_TIMEOUT) {
            Ok(cancelled) => cancelled,
            Err(_) => {
                tracing::warn!("plugin sync check timed out after 50 ms; event allowed");
                false
            }
        }
    }

    /// Command words registered by plugins. Refreshed after startup and after each batch.
    pub fn commands(&self) -> Vec<String> {
        self.commands.read().clone()
    }

    /// Calls `onDisable` and waits up to two seconds for the plugin thread to finish.
    pub fn shutdown(mut self) {
        let _ = self.tx.send(Msg::Shutdown);
        let _ = self.done.recv_timeout(Duration::from_secs(2));
        self.thread.take();
    }
}

// ---- wire encoding ----

fn pack(fields: &[&str]) -> String {
    let mut out = String::new();
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            out.push(SEP);
        }
        out.push_str(&f.replace(SEP, " "));
    }
    out
}

fn encode_event(e: &Event) -> String {
    match e {
        Event::Join { name, uuid } => pack(&["join", name, uuid]),
        Event::Quit { name } => pack(&["quit", name]),
        Event::Chat { name, message } => pack(&["chat", name, message]),
        Event::Command { name, line } => pack(&["command", name, line]),
        Event::BlockBreak { name, x, y, z, block } => {
            pack(&["break", name, &x.to_string(), &y.to_string(), &z.to_string(), block])
        }
        Event::BlockPlace { name, x, y, z, block } => {
            pack(&["place", name, &x.to_string(), &y.to_string(), &z.to_string(), block])
        }
    }
}

fn decode_action(s: &str) -> Option<Action> {
    let f: Vec<&str> = s.split(SEP).collect();
    let action = match f.as_slice() {
        ["broadcast", text] => Action::Broadcast((*text).to_owned()),
        ["message", player, text] => Action::Message {
            player: (*player).to_owned(),
            text: (*text).to_owned(),
        },
        ["setblock", x, y, z, block] => Action::SetBlock {
            x: x.parse().ok()?,
            y: y.parse().ok()?,
            z: z.parse().ok()?,
            block: (*block).to_owned(),
        },
        ["teleport", player, x, y, z] => Action::Teleport {
            player: (*player).to_owned(),
            x: x.parse().ok()?,
            y: y.parse().ok()?,
            z: z.parse().ok()?,
        },
        ["give", player, item, count] => Action::Give {
            player: (*player).to_owned(),
            item: (*item).to_owned(),
            count: count.parse().ok()?,
        },
        ["kick", player, reason] => Action::Kick {
            player: (*player).to_owned(),
            reason: (*reason).to_owned(),
        },
        ["runcommand", player, line] => Action::RunCommand {
            player: (*player).to_owned(),
            line: (*line).to_owned(),
        },
        _ => {
            tracing::warn!("ignoring malformed plugin action");
            return None;
        }
    };
    Some(action)
}

// ---- startup helpers ----

fn list_jars(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut jars = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        let is_jar = path.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("jar"));
        if is_jar {
            // absolute() rather than canonicalize(): canonicalize adds a \\?\ prefix that
            // breaks Java URLs on Windows.
            jars.push(std::path::absolute(&path)?);
        }
    }
    jars.sort();
    Ok(jars)
}

fn write_api_jar() -> anyhow::Result<PathBuf> {
    let path = std::env::temp_dir().join(format!("rine-api-{}.jar", std::process::id()));
    std::fs::write(&path, API_JAR).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// Finds jvm.dll (or libjvm.so) under $JAVA_HOME, then under the default JDK path.
fn find_libjvm() -> Option<PathBuf> {
    let mut homes: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        homes.push(PathBuf::from(home));
    }
    homes.push(PathBuf::from(DEFAULT_JAVA_HOME));
    homes
        .into_iter()
        .map(|home| home.join(LIBJVM_REL))
        .find(|p| p.is_file())
}

struct Startup {
    jars: Vec<PathBuf>,
    api_jar: PathBuf,
    rx: Receiver<Msg>,
    actions: Sender<Action>,
    mask: Arc<AtomicU32>,
    commands: Arc<RwLock<Vec<String>>>,
    ready: Sender<Result<bool, String>>,
}

fn run(s: Startup) {
    let Some(libjvm) = find_libjvm() else {
        tracing::warn!(
            "no JVM found (set JAVA_HOME to a JDK); Java plugins in this directory are disabled"
        );
        let _ = s.ready.send(Ok(false));
        return;
    };

    let init_args = match InitArgsBuilder::new()
        .version(JNIVersion::V1_8)
        .option(format!("-Djava.class.path={}", s.api_jar.display()))
        .build()
    {
        Ok(args) => args,
        Err(e) => {
            let _ = s.ready.send(Err(format!("bad JVM arguments: {e}")));
            return;
        }
    };

    let vm = match JavaVM::with_libjvm(init_args, || {
        Ok::<PathBuf, jni::errors::StartJvmError>(libjvm.clone())
    }) {
        Ok(vm) => vm,
        Err(e) => {
            let _ = s.ready.send(Err(format!("{e}")));
            return;
        }
    };
    tracing::info!("JVM loaded from {}", libjvm.display());

    let mut ready = Some(s.ready.clone());
    let Startup { jars, rx, actions, mask, commands, .. } = s;
    let outcome = vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        session(env, &jars, &rx, &actions, &mask, &commands, &mut ready)
    });
    if let Err(e) = outcome {
        match ready.take() {
            Some(tx) => {
                let _ = tx.send(Err(format!("{e}")));
            }
            None => tracing::error!("plugin session ended with error: {e}"),
        }
    }
}

/// Body of the plugin thread, run while the thread is attached to the JVM.
fn session(
    env: &mut Env,
    jars: &[PathBuf],
    rx: &Receiver<Msg>,
    actions: &Sender<Action>,
    mask: &AtomicU32,
    commands: &RwLock<Vec<String>>,
    ready: &mut Option<Sender<Result<bool, String>>>,
) -> jni::errors::Result<()> {
    let host = env.find_class(jni_str!("rine/host/Host"))?;

    let jar_paths: Vec<String> = jars.iter().map(|p| p.display().to_string()).collect();
    let init_result = env.with_local_frame(jar_paths.len() + 16, |env| -> jni::errors::Result<()> {
        let arr = string_array(env, &jar_paths)?;
        env.call_static_method(
            &host,
            jni_str!("init"),
            jni_sig!("([Ljava/lang/String;)V"),
            &[JValue::Object(&arr)],
        )?
        .v()
    });
    if let Err(e) = init_result {
        // An error here is a JVM-level failure, not a plugin failure: Host.init catches those.
        return Err(e);
    }
    if let Some(tx) = ready.take() {
        let _ = tx.send(Ok(true));
    }
    refresh(env, &host, mask, commands);
    after_call(env, &host, actions, mask, commands);

    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Batch(events) => {
                let result = env.with_local_frame(events.len() + 16, |env| -> jni::errors::Result<()> {
                    let arr = string_array(env, &events)?;
                    env.call_static_method(
                        &host,
                        jni_str!("dispatch"),
                        jni_sig!("([Ljava/lang/String;)V"),
                        &[JValue::Object(&arr)],
                    )?
                    .v()
                });
                log_call_error("dispatch", env, result);
                after_call(env, &host, actions, mask, commands);
            }
            Msg::Sync(encoded, reply) => {
                let result = env.with_local_frame(4, |env| -> jni::errors::Result<bool> {
                    let js = JString::from_str(env, &encoded)?;
                    env.call_static_method(
                        &host,
                        jni_str!("syncCancel"),
                        jni_sig!("(Ljava/lang/String;)Z"),
                        &[JValue::Object(&js)],
                    )?
                    .z()
                });
                let cancelled = match result {
                    Ok(c) => c,
                    Err(e) => {
                        log_call_error("syncCancel", env, Err::<(), _>(e));
                        false
                    }
                };
                let _ = reply.send(cancelled);
                after_call(env, &host, actions, mask, commands);
            }
            Msg::Shutdown => break,
        }
    }

    let result = env.with_local_frame(4, |env| -> jni::errors::Result<()> {
        env.call_static_method(&host, jni_str!("shutdown"), jni_sig!("()V"), &[])?.v()
    });
    log_call_error("shutdown", env, result);
    Ok(())
}

fn string_array<'local>(
    env: &mut Env<'local>,
    items: &[String],
) -> jni::errors::Result<JObjectArray<'local, JString<'local>>> {
    let placeholder = JString::from_str(env, "")?;
    let arr = JObjectArray::<JString>::new(env, items.len(), &placeholder)?;
    for (i, s) in items.iter().enumerate() {
        let js = JString::from_str(env, s)?;
        arr.set_element(env, i, &js)?;
    }
    Ok(arr)
}

fn read_string_array(env: &mut Env, obj: JObject) -> jni::errors::Result<Vec<String>> {
    let arr = JObjectArray::<JString>::cast_local(env, obj)?;
    let len = arr.len(env)?;
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let s = arr.get_element(env, i)?;
        out.push(s.try_to_string(env)?);
    }
    Ok(out)
}

/// Drains actions and updates the sync mask and command list. Errors are logged, not fatal.
fn after_call(
    env: &mut Env,
    host: &JClass,
    actions: &Sender<Action>,
    mask: &AtomicU32,
    commands: &RwLock<Vec<String>>,
) {
    let result = env.with_local_frame(64, |env| -> jni::errors::Result<()> {
        let raw = env
            .call_static_method(host, jni_str!("drainActions"), jni_sig!("()[Ljava/lang/String;"), &[])?
            .l()?;
        for line in read_string_array(env, raw)? {
            if let Some(action) = decode_action(&line) {
                let _ = actions.send(action);
            }
        }
        Ok(())
    });
    log_call_error("drainActions", env, result);
    refresh(env, host, mask, commands);
}

/// Re-reads the sync mask and the command list from Java.
fn refresh(env: &mut Env, host: &JClass, mask: &AtomicU32, commands: &RwLock<Vec<String>>) {
    let result = env.with_local_frame(64, |env| -> jni::errors::Result<(i32, Vec<String>)> {
        let bits = env
            .call_static_method(host, jni_str!("syncMask"), jni_sig!("()I"), &[])?
            .i()?;
        let raw = env
            .call_static_method(host, jni_str!("commands"), jni_sig!("()[Ljava/lang/String;"), &[])?
            .l()?;
        Ok((bits, read_string_array(env, raw)?))
    });
    match result {
        Ok((bits, names)) => {
            mask.store(bits as u32, Ordering::Release);
            *commands.write() = names;
        }
        Err(e) => log_call_error("refresh", env, Err::<(), _>(e)),
    }
}

/// Logs a failed JNI call and clears any pending Java exception so the thread stays usable.
fn log_call_error(what: &str, env: &mut Env, result: jni::errors::Result<()>) {
    if let Err(e) = result {
        tracing::error!("plugin call {what} failed: {e}");
        if env.exception_check() {
            env.exception_describe();
            env.exception_clear();
        }
    }
}
