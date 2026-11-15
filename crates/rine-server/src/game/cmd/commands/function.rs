//! /function, /schedule, /return, `execute if function`, the function
//! library and its tick/load tags (`FunctionCommand`, `ScheduleCommand`,
//! `ReturnCommand`, `ExecuteCommand.ExecuteIfFunctionCustomModifier`,
//! `CommandFunction`, `MacroFunction`, `StringTemplate`,
//! `ServerFunctionLibrary`, `ServerFunctionManager`, `TimerQueue`).
//!
//! Functions are parsed once at load (plain lines keep their parsed
//! chains); macro lines are substituted and parsed on instantiation with
//! the same 8-entry cache as vanilla.

use super::data;
use crate::game::cmd::exec::{self, call_cb, chain_cb, handle_error, Action, Callback, Entry, FnLine, Frame, Function, Mods};
use crate::game::cmd::source::Source;
use crate::game::cmd::{lit, Commands};
use crate::game::Game;
use ahash::AHashMap;
use rine_command::args::misc::FunctionRef;
use rine_command::args::Arg;
use rine_command::{Chain, CmdError, CmdResult};
use rine_protocol::nbt::Tag;
use rine_protocol::text::Text;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// `CommandFunction.checkCommandLineLength` limit.
const MAX_LINE: usize = 2_000_000;
const MACRO_CACHE: usize = 8;

pub fn register(c: &mut Commands) {
    c.exec_custom("function name", function_exec);
    c.exec_custom("function name arguments", function_exec);
    for k in ["entity", "block", "storage"] {
        c.exec_custom(&format!("function name with {k} source"), function_exec);
        c.exec_custom(&format!("function name with {k} source path"), function_exec);
    }
    c.exec_custom("return value", |g, src, chain, _mods, frame| {
        let v = match chain.executable.arg("value").map(|a| &a.value) {
            Some(Arg::Int(v)) => *v,
            _ => 0,
        };
        call_cb(g, &src.callback, true, v);
        frame.return_success(g, v);
        frame.discard(g);
    });
    c.exec_custom("return fail", |g, src, _chain, _mods, frame| {
        call_cb(g, &src.callback, false, 0);
        frame.return_failure(g);
        frame.discard(g);
    });
    // `return run` shares its node with `execute run` in the tree: the
    // return modifier applies when the stage is exactly `return run`,
    // otherwise sources pass through like a plain redirect
    c.modifier_custom("execute run", false, |g, original, sources, chain, stage, mods, frame| {
        let ctx = &chain.modifiers[stage];
        let tree = g.cmds.tree.clone();
        let is_return = ctx.nodes.len() == 2 && tree.node(ctx.nodes[0].0).name == "return";
        let input = chain.executable.input.clone();
        if !is_return {
            g.exec.queue_next(Entry { frame, action: Action::Build { input, chain, stage: stage + 1, mods, original, sources } });
            return;
        }
        if sources.is_empty() {
            if mods.is_return() {
                g.exec.queue_next(Entry { frame, action: Action::Fallthrough });
            }
            return;
        }
        frame.discard(g);
        g.exec.queue_next(Entry { frame, action: Action::Build { input, chain, stage: stage + 1, mods: mods.set_return(), original, sources } });
    });
    for p in ["schedule function function time", "schedule function function time append", "schedule function function time replace"] {
        c.exec(p, |x| {
            let f = match x.arg("function") {
                Arg::Function(f) => f.clone(),
                _ => unreachable!(),
            };
            let time = x.int("time");
            let replace = x.lit(x.c.nodes.len() - 1) != "append";
            schedule(x, &f, time, replace)
        });
    }
    c.exec("schedule clear function", |x| {
        let id = x.id("function").to_string();
        let n = x.g.cs.function.timers.remove(&id);
        if n == 0 {
            return Err(CmdError::tr("commands.schedule.cleared.failure", vec![lit(&id)]));
        }
        x.success(Text::tr("commands.schedule.cleared.success", vec![lit(n), lit(&id)]), true);
        Ok(n)
    });
}

// ---------------------------------------------------------------------------
// functions

/// `CommandFunction`: plain (pre-parsed) or macro.
#[derive(Clone)]
pub enum CmdFn {
    Plain(Arc<Function>),
    Macro(Arc<MacroFn>),
}

impl CmdFn {
    pub fn id(&self) -> &str {
        match self {
            CmdFn::Plain(f) => &f.id,
            CmdFn::Macro(m) => &m.id,
        }
    }

    /// `CommandFunction.instantiate`: errors are the
    /// `FunctionInstantiationException` message.
    pub fn instantiate(&self, g: &mut Game, args: Option<&Tag>) -> Result<Arc<Function>, Text> {
        match self {
            CmdFn::Plain(f) => Ok(f.clone()),
            CmdFn::Macro(m) => m.instantiate(g, args),
        }
    }
}

enum MacroEntry {
    Plain(FnLine),
    /// `StringTemplate` + indices of its variables in the parameter list.
    Template { segments: Vec<String>, vars: Vec<usize>, trailing: bool },
}

pub struct MacroFn {
    id: String,
    params: Vec<String>,
    entries: Vec<MacroEntry>,
    cache: Mutex<Vec<(Vec<String>, Arc<Function>)>>,
}

impl MacroFn {
    fn instantiate(&self, g: &mut Game, args: Option<&Tag>) -> Result<Arc<Function>, Text> {
        let Some(args) = args else {
            return Err(Text::tr("commands.function.error.missing_arguments", vec![lit(&self.id)]));
        };
        let mut values = Vec::with_capacity(self.params.len());
        for p in &self.params {
            match args.get(p) {
                Some(t) => values.push(stringify(t)),
                None => return Err(Text::tr("commands.function.error.missing_argument", vec![lit(&self.id), lit(p)])),
            }
        }
        if let Ok(mut c) = self.cache.lock() {
            if let Some(i) = c.iter().position(|(k, _)| *k == values) {
                let e = c.remove(i);
                let f = e.1.clone();
                c.push(e);
                return Ok(f);
            }
        }
        let level = g.shared.config.function_permission_level;
        let mut lines = Vec::with_capacity(self.entries.len());
        for e in &self.entries {
            match e {
                MacroEntry::Plain(l) => lines.push(l.clone()),
                MacroEntry::Template { segments, vars, trailing } => {
                    let mut s = String::new();
                    for (i, v) in vars.iter().enumerate() {
                        s.push_str(&segments[i]);
                        s.push_str(&values[*v]);
                    }
                    if *trailing {
                        s.push_str(segments.last().map(String::as_str).unwrap_or(""));
                    }
                    if s.len() > MAX_LINE {
                        return Err(Text::tr("commands.function.error.parse", vec![lit(&self.id), lit(&s[..512]), lit("Command too long")]));
                    }
                    match g.cmds.parse(&s, level) {
                        Ok(chain) => lines.push(FnLine { input: Arc::from(s.as_str()), chain }),
                        Err(e) => return Err(Text::tr("commands.function.error.parse", vec![lit(&self.id), lit(&s), lit(brig_message(&e))])),
                    }
                }
            }
        }
        // `id + "/" + keys.hashCode()` (only used for profiling names)
        let f = Arc::new(Function { id: format!("{}/{}", self.id, java_list_hash(&self.params)), lines });
        if let Ok(mut c) = self.cache.lock() {
            if c.len() >= MACRO_CACHE {
                c.remove(0);
            }
            c.push((values, f.clone()));
        }
        Ok(f)
    }
}

/// `MacroFunction.stringify`.
fn stringify(t: &Tag) -> String {
    match t {
        Tag::Float(v) => decimal_format(*v as f64),
        Tag::Double(v) => decimal_format(*v),
        Tag::Byte(v) => v.to_string(),
        Tag::Short(v) => v.to_string(),
        Tag::Long(v) => v.to_string(),
        Tag::String(s) => s.clone(),
        other => data::to_snbt(other),
    }
}

/// `new DecimalFormat("#")` with 15 max fraction digits, HALF_EVEN.
fn decimal_format(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "∞".into() } else { "-∞".into() };
    }
    let mut s = format!("{v:.15}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "-0".into();
    }
    s
}

fn java_list_hash(v: &[String]) -> i32 {
    let mut h: i32 = 1;
    for s in v {
        let mut sh: i32 = 0;
        for c in s.encode_utf16() {
            sh = sh.wrapping_mul(31).wrapping_add(c as i32);
        }
        h = h.wrapping_mul(31).wrapping_add(sh);
    }
    h
}

/// Brigadier `CommandSyntaxException.getMessage()`.
pub fn brig_message(e: &CmdError) -> String {
    let msg = crate::game::cmd::render_plain(&e.msg);
    let Some(input) = &e.input else { return msg };
    let cursor = e.cursor.min(input.len());
    let before = &input[..cursor];
    let n = before.chars().count();
    let start = if n > 10 { before.char_indices().nth(n - 10).map(|(i, _)| i).unwrap_or(0) } else { 0 };
    let ctx = format!("{}{}<--[HERE]", if n > 10 { "..." } else { "" }, &input[start..cursor]);
    format!("{msg} at position {n}: {ctx}")
}

/// `CommandFunction.fromLines`: `Err` = the load failure message.
pub fn compile(g: &mut Game, id: &str, text: &str) -> Result<CmdFn, String> {
    let level = g.shared.config.function_permission_level;
    let raw: Vec<&str> = text.lines().collect();
    let mut plain: Option<Vec<FnLine>> = Some(Vec::new());
    let mut entries: Vec<MacroEntry> = Vec::new();
    let mut params: Vec<String> = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let line_no = i + 1;
        let input = raw[i].trim();
        let line = if input.ends_with('\\') {
            let mut b = input.to_string();
            loop {
                i += 1;
                if i == raw.len() {
                    return Err("Line continuation at end of file".into());
                }
                b.pop();
                b.push_str(raw[i].trim());
                if b.len() > MAX_LINE {
                    return Err(format!("Command too long: {} characters", b.len()));
                }
                if !b.ends_with('\\') {
                    break;
                }
            }
            b
        } else {
            input.to_string()
        };
        i += 1;
        if line.len() > MAX_LINE {
            return Err(format!("Command too long: {} characters", line.len()));
        }
        let Some(first) = line.chars().next() else { continue };
        if first == '#' {
            continue;
        }
        if first == '/' {
            if line[1..].starts_with('/') {
                return Err(format!("Unknown or invalid command '{line}' on line {line_no} (if you intended to make a comment, use '#' not '//')"));
            }
            let name: String = line[1..].chars().take_while(|c| rine_command::Reader::is_allowed_in_unquoted_string(*c)).collect();
            return Err(format!("Unknown or invalid command '{line}' on line {line_no} (did you mean '{name}'? Do not use a preceding forwards slash.)"));
        }
        if first == '$' {
            let (segments, vars, trailing) = template(&line[1..]).map_err(|e| format!("Can't parse function line {line_no}: '{}': {e}", &line[1..]))?;
            if let Some(p) = plain.take() {
                entries.extend(p.into_iter().map(MacroEntry::Plain));
            }
            let idx = vars
                .iter()
                .map(|v| match params.iter().position(|p| p == v) {
                    Some(i) => i,
                    None => {
                        params.push(v.clone());
                        params.len() - 1
                    }
                })
                .collect();
            entries.push(MacroEntry::Template { segments, vars: idx, trailing });
            continue;
        }
        let chain = g.cmds.parse(&line, level).map_err(|e| format!("Whilst parsing command on line {line_no}: {}", brig_message(&e)))?;
        let l = FnLine { input: Arc::from(line.as_str()), chain };
        match &mut plain {
            Some(p) => p.push(l),
            None => entries.push(MacroEntry::Plain(l)),
        }
    }
    Ok(match plain {
        Some(lines) => CmdFn::Plain(Arc::new(Function { id: id.to_string(), lines })),
        None => CmdFn::Macro(Arc::new(MacroFn { id: id.to_string(), params, entries, cache: Mutex::new(Vec::new()) })),
    })
}

/// `StringTemplate.fromString`: (segments, variables, has trailing segment).
fn template(input: &str) -> Result<(Vec<String>, Vec<String>, bool), String> {
    let b = input.as_bytes();
    let (mut segments, mut vars) = (Vec::new(), Vec::new());
    let mut start = 0;
    let mut idx = input.find('$');
    while let Some(i) = idx {
        if i != b.len() - 1 && b[i + 1] == b'(' {
            segments.push(input[start..i].to_string());
            let Some(end) = input[i + 1..].find(')').map(|e| e + i + 1) else { return Err("Unterminated macro variable".into()) };
            let var = &input[i + 2..end];
            if !var.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return Err(format!("Invalid macro variable name '{var}'"));
            }
            vars.push(var.to_string());
            start = end + 1;
            idx = input[start..].find('$').map(|j| j + start);
        } else {
            idx = input[i + 1..].find('$').map(|j| j + i + 1);
        }
    }
    if start == 0 {
        return Err("No variables in macro".into());
    }
    let trailing = start != b.len();
    if trailing {
        segments.push(input[start..].to_string());
    }
    Ok((segments, vars, trailing))
}

// ---------------------------------------------------------------------------
// library, tags, timers

#[derive(Default)]
pub struct State {
    pub functions: AHashMap<String, CmdFn>,
    pub tags: AHashMap<String, Vec<CmdFn>>,
    ticking: Vec<CmdFn>,
    post_reload: bool,
    pub timers: TimerQueue,
    loaded: bool,
}

/// `TimerQueue`: events by trigger time, then insertion order.
#[derive(Default)]
pub struct TimerQueue {
    seq: u64,
    queue: BTreeMap<(u64, u64), (String, Timer)>,
}

#[derive(Clone)]
pub enum Timer {
    Function(String),
    Tag(String),
}

impl TimerQueue {
    pub fn schedule(&mut self, id: String, time: u64, t: Timer) {
        self.seq += 1;
        self.queue.insert((time, self.seq), (id, t));
    }
    /// Remove every event with this id; returns the count.
    pub fn remove(&mut self, id: &str) -> i32 {
        let before = self.queue.len();
        self.queue.retain(|_, (k, _)| k != id);
        (before - self.queue.len()) as i32
    }
    pub fn ids(&self) -> Vec<String> {
        let mut v: Vec<String> = self.queue.values().map(|(k, _)| k.clone()).collect();
        v.sort();
        v.dedup();
        v
    }
}

/// Enabled data pack folders (`<world>/datapacks/<name>/` with a
/// `pack.mcmeta`), in load order.
pub fn pack_dirs(g: &Game) -> Vec<std::path::PathBuf> {
    let root = std::path::Path::new(&g.shared.config.world_dir).join("datapacks");
    let Ok(rd) = std::fs::read_dir(&root) else { return Vec::new() };
    let mut v: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.join("pack.mcmeta").is_file()).collect();
    v.sort();
    let disabled = super::datapack::disabled(g);
    v.retain(|p| p.file_name().and_then(|n| n.to_str()).is_none_or(|n| !disabled.iter().any(|d| d == &format!("file/{n}"))));
    v
}

/// `ServerFunctionLibrary.reload` + `ServerFunctionManager.replaceLibrary`.
pub fn reload(g: &mut Game) {
    let packs = pack_dirs(g);
    let mut sources: BTreeMap<String, String> = BTreeMap::new();
    let mut tag_files: Vec<(String, serde_json::Value)> = Vec::new();
    for pack in &packs {
        let data = pack.join("data");
        let Ok(namespaces) = std::fs::read_dir(&data) else { continue };
        for ns in namespaces.flatten() {
            let Some(nsname) = ns.file_name().to_str().map(str::to_string) else { continue };
            let fdir = ns.path().join("function");
            walk(&fdir, &mut |rel, path| {
                if let Some(p) = rel.strip_suffix(".mcfunction") {
                    if let Ok(t) = std::fs::read_to_string(path) {
                        sources.insert(format!("{nsname}:{p}"), t);
                    }
                }
            });
            let tdir = ns.path().join("tags").join("function");
            walk(&tdir, &mut |rel, path| {
                if let Some(p) = rel.strip_suffix(".json") {
                    if let Some(j) = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()) {
                        tag_files.push((format!("{nsname}:{p}"), j));
                    }
                }
            });
        }
    }
    let mut functions = AHashMap::new();
    for (id, text) in &sources {
        match compile(g, id, text) {
            Ok(f) => {
                functions.insert(id.clone(), f);
            }
            Err(e) => tracing::error!("Failed to load function {id}: {e}"),
        }
    }
    // TagLoader: entries per tag in pack order, `replace` resets
    let mut raw: BTreeMap<String, Vec<(String, bool)>> = BTreeMap::new();
    for (id, j) in tag_files {
        let entries = raw.entry(id).or_default();
        if j.get("replace").and_then(|v| v.as_bool()).unwrap_or(false) {
            entries.clear();
        }
        for v in j.get("values").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
            match v {
                serde_json::Value::String(s) => entries.push((s, true)),
                serde_json::Value::Object(o) => {
                    if let Some(s) = o.get("id").and_then(|v| v.as_str()) {
                        entries.push((s.to_string(), o.get("required").and_then(|r| r.as_bool()).unwrap_or(true)));
                    }
                }
                _ => {}
            }
        }
    }
    let mut tags: AHashMap<String, Vec<CmdFn>> = AHashMap::new();
    for id in raw.keys() {
        let mut out = Vec::new();
        let mut seen = Vec::new();
        if resolve_tag(&raw, &functions, id, &mut out, &mut seen) {
            tags.insert(id.clone(), out);
        } else {
            tracing::error!("Couldn't load tag {id} as it is missing following references");
        }
    }
    let st = &mut g.cs.function;
    st.ticking = tags.get("minecraft:tick").cloned().unwrap_or_default();
    st.functions = functions;
    st.tags = tags;
    st.post_reload = true;
    st.loaded = true;
    tracing::info!("Loaded {} functions", st.functions.len());
}

fn resolve_tag(raw: &BTreeMap<String, Vec<(String, bool)>>, fns: &AHashMap<String, CmdFn>, id: &str, out: &mut Vec<CmdFn>, seen: &mut Vec<String>) -> bool {
    if seen.iter().any(|s| s == id) {
        return true;
    }
    seen.push(id.to_string());
    let Some(entries) = raw.get(id) else { return false };
    for (e, required) in entries {
        if let Some(t) = e.strip_prefix('#') {
            let t = norm(t);
            if !resolve_tag(raw, fns, &t, out, seen) && *required {
                return false;
            }
        } else {
            match fns.get(&norm(e)) {
                Some(f) => {
                    if !out.iter().any(|x| x.id() == f.id()) {
                        out.push(f.clone());
                    }
                }
                None if *required => return false,
                None => {}
            }
        }
    }
    true
}

fn norm(id: &str) -> String {
    if id.contains(':') {
        id.to_string()
    } else {
        format!("minecraft:{id}")
    }
}

fn walk(dir: &std::path::Path, f: &mut dyn FnMut(&str, &std::path::Path)) {
    fn rec(base: &std::path::Path, dir: &std::path::Path, f: &mut dyn FnMut(&str, &std::path::Path)) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                rec(base, &p, f);
            } else if let Ok(rel) = p.strip_prefix(base) {
                let rel = rel.to_string_lossy().replace('\\', "/");
                f(&rel, &p);
            }
        }
    }
    rec(dir, dir, f);
}

/// `getGameLoopSender`: console source, level 2, output suppressed.
fn game_loop_source(g: &Game) -> Source {
    Source::console(g).with_level(2).with_silent()
}

/// `ServerFunctionManager.execute`.
fn run_function(g: &mut Game, f: &CmdFn, src: Source) {
    let Ok(inst) = f.instantiate(g, None) else { return };
    exec::in_context(g, |g| exec::queue_function(g, inst, src, None));
}

/// `ServerFunctionManager.tick` (load tag after reload, then tick tag).
pub fn tick_functions(g: &mut Game) {
    if !g.cs.function.loaded {
        reload(g);
    }
    if g.cs.function.post_reload {
        g.cs.function.post_reload = false;
        let load = g.cs.function.tags.get("minecraft:load").cloned().unwrap_or_default();
        for f in &load {
            let s = game_loop_source(g);
            run_function(g, f, s);
        }
    }
    if g.cs.function.ticking.is_empty() {
        return;
    }
    let ticking = g.cs.function.ticking.clone();
    for f in &ticking {
        let s = game_loop_source(g);
        run_function(g, f, s);
    }
}

/// `TimerQueue.tick(server, gameTime)`.
pub fn tick_schedule(g: &mut Game) {
    let now = g.tick;
    loop {
        let Some((&k, _)) = g.cs.function.timers.queue.iter().next() else { return };
        if k.0 > now {
            return;
        }
        let Some((_, t)) = g.cs.function.timers.queue.remove(&k) else { return };
        let fns: Vec<CmdFn> = match t {
            Timer::Function(id) => g.cs.function.functions.get(&id).cloned().into_iter().collect(),
            Timer::Tag(id) => g.cs.function.tags.get(&id).cloned().unwrap_or_default(),
        };
        for f in &fns {
            let s = game_loop_source(g);
            run_function(g, f, s);
        }
    }
}

/// `FunctionArgument.Result.create`: functions of a reference.
pub fn resolve(g: &Game, f: &FunctionRef) -> CmdResult<Vec<CmdFn>> {
    if f.tag {
        match g.cs.function.tags.get(&f.id) {
            Some(v) => Ok(v.clone()),
            None => Err(CmdError::tr("arguments.function.tag.unknown", vec![lit(&f.id)])),
        }
    } else {
        match g.cs.function.functions.get(&f.id) {
            Some(v) => Ok(vec![v.clone()]),
            None => Err(CmdError::tr("arguments.function.unknown", vec![lit(&f.id)])),
        }
    }
}

// ---------------------------------------------------------------------------
// commands

fn function_ref(chain: &Chain) -> FunctionRef {
    match chain.executable.arg("name").map(|a| &a.value) {
        Some(Arg::Function(f)) => f.clone(),
        _ => FunctionRef { tag: false, id: String::new() },
    }
}

/// `FunctionCustomExecutor.runGuarded` with `WithErrorHandling`.
fn function_exec(g: &mut Game, src: Source, chain: Arc<Chain>, mods: Mods, frame: Frame) {
    if let Err(e) = function_guarded(g, &src, &chain, mods, &frame) {
        handle_error(g, &src, &e, mods.forked());
        call_cb(g, &src.callback, false, 0);
    }
}

fn function_guarded(g: &mut Game, src: &Source, chain: &Arc<Chain>, mods: Mods, frame: &Frame) -> CmdResult<()> {
    let fref = function_ref(chain);
    let fns = resolve(g, &fref)?;
    if fns.is_empty() {
        return Err(CmdError::tr("commands.function.scheduled.no_functions", vec![lit(&fref.id)]));
    }
    let args = function_args(g, src, chain)?;
    let common = src.with_silent().with_max_level(2);
    let msg = if fns.len() == 1 {
        Text::tr("commands.function.scheduled.single", vec![lit(fns[0].id())])
    } else {
        Text::tr("commands.function.scheduled.multiple", vec![format_list(fns.iter().map(|f| f.id()))])
    };
    crate::game::cmd::feedback::send_success(g, src, &msg, true);
    queue_functions(g, &fns, args.as_ref(), src, &common, frame, mods)
}

/// `ComponentUtils.formatList(ids, Component::translationArg)`.
fn format_list<'a>(ids: impl Iterator<Item = &'a str>) -> Text {
    let items = ids.map(Text::plain).collect();
    crate::game::cmd::feedback::format_list(items, &crate::game::cmd::feedback::list_sep())
}

/// The `arguments` of the executed `function` variant.
fn function_args(g: &mut Game, src: &Source, chain: &Arc<Chain>) -> CmdResult<Option<Tag>> {
    let ctx = &chain.executable;
    let tree = g.cmds.tree.clone();
    let names: Vec<&str> = ctx.nodes.iter().map(|(id, _, _)| tree.node(*id).name).collect();
    if let Some(Arg::Nbt(t)) = ctx.arg("arguments").map(|a| &a.value) {
        return Ok(Some((**t).clone()));
    }
    let Some(kind) = names.iter().position(|n| *n == "with").and_then(|i| names.get(i + 1)).copied() else { return Ok(None) };
    let n = chain.modifiers.len();
    let mut x = crate::game::cmd::Ctx { g, src, c: ctx, chain, stage: n };
    let target = data::target(&mut x, kind, "source")?;
    let root = data::get_data(x.g, &target)?;
    match ctx.arg("path").map(|a| &a.value) {
        None => Ok(Some(root)),
        Some(Arg::NbtPath(p)) => {
            let tags = data::path_get(p, &root)?;
            if tags.len() > 1 {
                return Err(CmdError::key("commands.data.get.multiple"));
            }
            match tags.into_iter().next() {
                Some(t @ Tag::Compound(_)) => Ok(Some(t)),
                Some(t) => Err(CmdError::tr("commands.function.error.argument_not_compound", vec![lit(tag_type_name(&t))])),
                None => Ok(None),
            }
        }
        _ => Ok(None),
    }
}

/// `TagType.getName()`.
fn tag_type_name(t: &Tag) -> &'static str {
    match t {
        Tag::Byte(_) => "BYTE",
        Tag::Short(_) => "SHORT",
        Tag::Int(_) => "INT",
        Tag::Long(_) => "LONG",
        Tag::Float(_) => "FLOAT",
        Tag::Double(_) => "DOUBLE",
        Tag::ByteArray(_) => "BYTE[]",
        Tag::String(_) => "STRING",
        Tag::List(_) => "LIST",
        Tag::Compound(_) => "COMPOUND",
        Tag::IntArray(_) => "INT[]",
        Tag::LongArray(_) => "LONG[]",
    }
}

/// `decorateOutputIfNeeded`: `commands.function.result` to the original
/// source unless it is silent.
fn decorate(original: &Source, id: &str, cb: Callback) -> Callback {
    if original.silent {
        return cb;
    }
    let src = original.clone();
    let id = id.to_string();
    Some(Arc::new(move |g: &mut Game, ok, v| {
        crate::game::cmd::feedback::send_success(g, &src, &Text::tr("commands.function.result", vec![lit(&id), lit(v)]), true);
        call_cb(g, &cb, ok, v);
    }))
}

/// `FunctionCommand.queueFunctions`.
pub fn queue_functions(g: &mut Game, fns: &[CmdFn], args: Option<&Tag>, original: &Source, fsrc: &Source, frame: &Frame, mods: Mods) -> CmdResult<()> {
    let no_cb = fsrc.with_callback(None);
    let inst = |g: &mut Game, f: &CmdFn| -> CmdResult<Arc<Function>> {
        f.instantiate(g, args).map_err(|reason| CmdError::tr("commands.function.instantiationFailure", vec![lit(f.id()), reason]))
    };
    if mods.is_return() {
        let out = chain_cb(&original.callback, &frame.ret);
        for f in fns {
            let func = inst(g, f)?;
            let cb = decorate(original, f.id(), out.clone());
            g.exec.queue_next(Entry { frame: frame.clone(), action: Action::Call { func, cb, return_parent: true, source: no_cb.clone() } });
        }
        g.exec.queue_next(Entry { frame: frame.clone(), action: Action::Fallthrough });
        return Ok(());
    }
    let orig_cb = original.callback.clone();
    if fns.len() == 1 || orig_cb.is_none() {
        for f in fns {
            let func = inst(g, f)?;
            let cb = decorate(original, f.id(), orig_cb.clone());
            g.exec.queue_next(Entry { frame: frame.clone(), action: Action::Call { func, cb, return_parent: false, source: no_cb.clone() } });
        }
        return Ok(());
    }
    // Accumulator: sum of results, reported once all ran
    let acc = Arc::new(Mutex::new((false, 0i32)));
    for f in fns {
        let func = inst(g, f)?;
        let a = acc.clone();
        let partial: Callback = Some(Arc::new(move |_g: &mut Game, _ok, v| {
            if let Ok(mut s) = a.lock() {
                s.0 = true;
                s.1 = s.1.wrapping_add(v);
            }
        }));
        let cb = decorate(original, f.id(), partial);
        g.exec.queue_next(Entry { frame: frame.clone(), action: Action::Call { func, cb, return_parent: false, source: no_cb.clone() } });
    }
    g.exec.queue_next(Entry {
        frame: frame.clone(),
        action: Action::Task(Box::new(move |g: &mut Game, _f: &Frame| {
            let (any, sum) = acc.lock().map(|s| *s).unwrap_or((false, 0));
            if any {
                call_cb(g, &orig_cb, true, sum);
            }
        })),
    });
    Ok(())
}

/// `ScheduleCommand.schedule`.
fn schedule(x: &mut crate::game::cmd::Ctx, f: &FunctionRef, time: i32, replace: bool) -> CmdResult<i32> {
    if time == 0 {
        return Err(CmdError::key("commands.schedule.same_tick"));
    }
    let tick_time = x.g.tick + time as u64;
    let fns = resolve(x.g, f)?;
    let st = &mut x.g.cs.function;
    let msg = if !f.tag {
        if matches!(fns.first(), Some(CmdFn::Macro(_))) {
            return Err(CmdError::key("commands.schedule.macro"));
        }
        if replace {
            st.timers.remove(&f.id);
        }
        st.timers.schedule(f.id.clone(), tick_time, Timer::Function(f.id.clone()));
        Text::tr("commands.schedule.created.function", vec![lit(&f.id), lit(time), lit(tick_time as i64)])
    } else {
        let sid = format!("#{}", f.id);
        if replace {
            st.timers.remove(&sid);
        }
        st.timers.schedule(sid, tick_time, Timer::Tag(f.id.clone()));
        Text::tr("commands.schedule.created.tag", vec![lit(&f.id), lit(time), lit(tick_time as i64)])
    };
    x.success(msg, true);
    Ok((tick_time as i64).rem_euclid(i32::MAX as i64) as i32)
}

/// `ExecuteIfFunctionCustomModifier.apply` +
/// `ExecuteCommand.scheduleFunctionConditionsAndTest`.
pub fn execute_if_function(g: &mut Game, original: Source, sources: Vec<Source>, chain: Arc<Chain>, stage: usize, mods: Mods, frame: Frame) {
    let ctx = &chain.modifiers[stage];
    let fref = match ctx.arg("name").map(|a| &a.value) {
        Some(Arg::Function(f)) => f.clone(),
        _ => FunctionRef { tag: false, id: String::new() },
    };
    let expected = {
        let tree = g.cmds.tree.clone();
        ctx.nodes.iter().any(|(id, _, _)| tree.node(*id).name == "if")
    };
    let fns = match resolve(g, &fref) {
        Ok(v) => v,
        Err(e) => {
            handle_error(g, &original, &e, mods.forked());
            return;
        }
    };
    if fns.is_empty() {
        return;
    }
    let mut inst = Vec::with_capacity(fns.len());
    for f in &fns {
        match f.instantiate(g, None) {
            Ok(i) => inst.push(i),
            Err(reason) => {
                let e = CmdError::tr("commands.execute.function.instantiationFailure", vec![lit(f.id()), reason]);
                handle_error(g, &original, &e, mods.forked());
                break;
            }
        }
    }
    let filtered: Arc<Mutex<Vec<Source>>> = Arc::new(Mutex::new(Vec::with_capacity(sources.len())));
    for src in sources {
        let fsrc = src.with_callback(None).with_silent().with_max_level(2);
        let out = filtered.clone();
        let cb: Callback = Some(Arc::new(move |_g: &mut Game, _ok, v| {
            if (v != 0) == expected {
                if let Ok(mut f) = out.lock() {
                    f.push(src.clone());
                }
            }
        }));
        let inst = inst.clone();
        // `IsolatedCall`: a new frame whose return value is the result
        g.exec.queue_next(Entry {
            frame: frame.clone(),
            action: Action::Task(Box::new(move |g: &mut Game, fr: &Frame| {
                let d = fr.depth + 1;
                let nf = Frame { depth: d, ret: cb.clone(), control: exec::FrameControl::DiscardFrom(d) };
                for func in inst {
                    g.exec.queue_next(Entry { frame: nf.clone(), action: Action::Call { func, cb: cb.clone(), return_parent: true, source: fsrc.clone() } });
                }
                g.exec.queue_next(Entry { frame: nf, action: Action::Fallthrough });
            })),
        });
    }
    let input = chain.executable.input.clone();
    g.exec.queue_next(Entry {
        frame,
        action: Action::Task(Box::new(move |g: &mut Game, fr: &Frame| {
            let next = filtered.lock().map(|v| v.clone()).unwrap_or_default();
            g.exec.queue_next(Entry { frame: fr.clone(), action: Action::Build { input, chain, stage: stage + 1, mods, original, sources: next } });
        })),
    });
}
