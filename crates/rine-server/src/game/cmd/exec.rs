//! Command execution queue, a port of vanilla `ExecutionContext`,
//! `BuildContexts`, `ExecuteCommand`, `ContinuationTask`, `CallFunction`
//! and `Frame`. Same ordering, fork/return semantics and limits
//! (`maxCommandChainLength`, `maxCommandForkCount`), so `execute`,
//! functions and `return` behave exactly like vanilla.

use super::source::Source;
use super::{Executor, Modifier};
use crate::game::Game;
use rine_command::{CmdError, Chain};
use rine_protocol::text::Text;
use std::collections::VecDeque;
use std::sync::Arc;

/// `CommandResultCallback`.
pub type Callback = Option<Arc<dyn Fn(&mut Game, bool, i32) + Send + Sync>>;

pub fn chain_cb(first: &Callback, second: &Callback) -> Callback {
    match (first, second) {
        (None, s) => s.clone(),
        (f, None) => f.clone(),
        (Some(a), Some(b)) => {
            let (a, b) = (a.clone(), b.clone());
            Some(Arc::new(move |g: &mut Game, ok, v| {
                a(g, ok, v);
                b(g, ok, v);
            }))
        }
    }
}

pub fn call_cb(g: &mut Game, cb: &Callback, ok: bool, v: i32) {
    if let Some(f) = cb {
        f(g, ok, v);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameControl {
    /// Top frame at depth 0: discard clears the whole queue.
    ClearAll,
    /// Discard every queued entry at this depth or deeper.
    DiscardFrom(u32),
}

#[derive(Clone)]
pub struct Frame {
    pub depth: u32,
    pub ret: Callback,
    pub control: FrameControl,
}

impl Frame {
    pub fn return_success(&self, g: &mut Game, v: i32) {
        call_cb(g, &self.ret, true, v);
    }
    pub fn return_failure(&self, g: &mut Game) {
        call_cb(g, &self.ret, false, 0);
    }
    pub fn discard(&self, g: &mut Game) {
        match self.control {
            FrameControl::ClearAll => g.exec.queue.clear(),
            FrameControl::DiscardFrom(d) => g.exec.discard_at_depth_or_higher(d),
        }
    }
}

/// `ChainModifiers`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods(u8);
impl Mods {
    pub const FORKED: u8 = 1;
    pub const RETURN: u8 = 2;
    pub fn forked(self) -> bool {
        self.0 & Self::FORKED != 0
    }
    pub fn is_return(self) -> bool {
        self.0 & Self::RETURN != 0
    }
    pub fn set_forked(self) -> Self {
        Mods(self.0 | Self::FORKED)
    }
    pub fn set_return(self) -> Self {
        Mods(self.0 | Self::RETURN)
    }
}

/// One line of an instantiated function (`UnboundEntryAction`).
#[derive(Clone)]
pub struct FnLine {
    pub input: Arc<str>,
    pub chain: Arc<Chain>,
}

/// `InstantiatedFunction`.
pub struct Function {
    pub id: String,
    pub lines: Vec<FnLine>,
}

pub enum Action {
    /// `BuildContexts` (TopLevel / Unbound / Continuation).
    Build { input: Arc<str>, chain: Arc<Chain>, stage: usize, mods: Mods, original: Source, sources: Vec<Source> },
    /// `ExecuteCommand` bound to a source.
    Execute { input: Arc<str>, mods: Mods, chain: Arc<Chain>, source: Source },
    /// `ContinuationTask` over the lines of a function, bound to a source.
    Lines { lines: Arc<Function>, index: usize, source: Source },
    /// `ContinuationTask` over sources for one executable stage.
    ExecEach { input: Arc<str>, mods: Mods, chain: Arc<Chain>, sources: Arc<Vec<Source>>, index: usize },
    /// `CallFunction` bound to a source.
    Call { func: Arc<Function>, cb: Callback, return_parent: bool, source: Source },
    /// `FallthroughTask`.
    Fallthrough,
    /// Arbitrary task run in the entry's frame (`IsolatedCall` producers,
    /// custom executors).
    Task(Box<dyn FnOnce(&mut Game, &Frame) + Send>),
}

pub struct Entry {
    pub frame: Frame,
    pub action: Action,
}

pub const MAX_QUEUE: usize = 10_000_000;

#[derive(Default)]
pub struct ExecState {
    pub queue: VecDeque<Entry>,
    new_top: Vec<Entry>,
    pub quota: i64,
    pub limit: i64,
    pub fork_limit: i32,
    overflow: bool,
    pub current_depth: u32,
    /// A run loop is active (nested invocations just queue).
    pub active: bool,
}

impl ExecState {
    pub fn queue_next(&mut self, e: Entry) {
        if self.new_top.len() + self.queue.len() > MAX_QUEUE {
            self.overflow = true;
            self.new_top.clear();
            self.queue.clear();
        }
        if !self.overflow {
            self.new_top.push(e);
        }
    }
    pub fn discard_at_depth_or_higher(&mut self, d: u32) {
        while self.queue.front().is_some_and(|e| e.frame.depth >= d) {
            self.queue.pop_front();
        }
    }
    fn push_new(&mut self) {
        for e in self.new_top.drain(..).rev() {
            self.queue.push_front(e);
        }
    }
    pub fn increment_cost(&mut self) {
        self.quota -= 1;
    }
    /// Frame for a new top-level call (`createTopFrame`).
    pub fn top_frame(&self, ret: Callback) -> Frame {
        if self.current_depth == 0 {
            Frame { depth: 0, ret, control: FrameControl::ClearAll }
        } else {
            let d = self.current_depth + 1;
            Frame { depth: d, ret, control: FrameControl::DiscardFrom(d) }
        }
    }
}

/// `Commands.executeCommandInContext`: run `setup` against the current
/// execution context, creating a top-level one (and running its queue)
/// when none is active.
pub fn in_context(g: &mut Game, setup: impl FnOnce(&mut Game)) {
    if g.exec.active {
        setup(g);
        return;
    }
    let chain_limit = (g.rules.int("max_command_sequence_length")).max(1) as i64;
    let fork_limit = g.rules.int("max_command_forks");
    g.exec.active = true;
    g.exec.quota = chain_limit;
    g.exec.limit = chain_limit;
    g.exec.fork_limit = fork_limit;
    g.exec.overflow = false;
    setup(g);
    run_queue(g);
    g.exec.active = false;
    g.exec.queue.clear();
    g.exec.new_top.clear();
    g.exec.current_depth = 0;
}

fn run_queue(g: &mut Game) {
    g.exec.push_new();
    loop {
        if g.exec.quota <= 0 {
            tracing::info!("Command execution stopped due to limit (executed {} commands)", g.exec.limit);
            break;
        }
        let Some(entry) = g.exec.queue.pop_front() else { return };
        g.exec.current_depth = entry.frame.depth;
        run_entry(g, entry);
        if g.exec.overflow {
            tracing::error!("Command execution stopped due to command queue overflow (max {MAX_QUEUE})");
            break;
        }
        g.exec.push_new();
    }
    g.exec.current_depth = 0;
}

/// Queue a parsed command for `source` (`queueInitialCommandExecution`).
pub fn queue_command(g: &mut Game, input: Arc<str>, chain: Arc<Chain>, source: Source, ret: Callback) {
    let frame = g.exec.top_frame(ret);
    let sources = vec![source.clone()];
    g.exec.queue_next(Entry { frame, action: Action::Build { input, chain, stage: 0, mods: Mods::default(), original: source, sources } });
}

/// Queue a function call (`queueInitialFunctionCall`).
pub fn queue_function(g: &mut Game, func: Arc<Function>, source: Source, ret: Callback) {
    let frame = g.exec.top_frame(ret);
    let cb = source.callback.clone();
    g.exec.queue_next(Entry { frame, action: Action::Call { func, cb, return_parent: false, source } });
}

fn run_entry(g: &mut Game, e: Entry) {
    let Entry { frame, action } = e;
    match action {
        Action::Build { input, chain, stage, mods, original, sources } => build(g, &frame, input, chain, stage, mods, original, sources),
        Action::Execute { input, mods, chain, source } => execute(g, &input, mods, &chain, source),
        Action::Lines { lines, index, source } => {
            let line = lines.lines[index].clone();
            if index + 1 < lines.lines.len() {
                // self entry first in new_top order: queued after the line
                let me = Entry { frame: frame.clone(), action: Action::Lines { lines: lines.clone(), index: index + 1, source: source.clone() } };
                let sources = vec![source.clone()];
                g.exec.queue_next(Entry {
                    frame: frame.clone(),
                    action: Action::Build { input: line.input, chain: line.chain, stage: 0, mods: Mods::default(), original: source, sources },
                });
                g.exec.queue_next(me);
            } else {
                let sources = vec![source.clone()];
                g.exec.queue_next(Entry {
                    frame,
                    action: Action::Build { input: line.input, chain: line.chain, stage: 0, mods: Mods::default(), original: source, sources },
                });
            }
        }
        Action::ExecEach { input, mods, chain, sources, index } => {
            let src = sources[index].clone();
            g.exec.queue_next(Entry { frame: frame.clone(), action: Action::Execute { input: input.clone(), mods, chain: chain.clone(), source: src } });
            if index + 1 < sources.len() {
                g.exec.queue_next(Entry { frame, action: Action::ExecEach { input, mods, chain, sources, index: index + 1 } });
            }
        }
        Action::Call { func, cb, return_parent, source } => {
            g.exec.increment_cost();
            let new_depth = frame.depth + 1;
            let control = if return_parent { frame.control } else { FrameControl::DiscardFrom(new_depth) };
            let nf = Frame { depth: new_depth, ret: cb, control };
            match func.lines.len() {
                0 => {}
                _ => g.exec.queue_next(Entry { frame: nf, action: Action::Lines { lines: func, index: 0, source } }),
            }
        }
        Action::Fallthrough => {
            frame.return_failure(g);
            frame.discard(g);
        }
        Action::Task(f) => f(g, &frame),
    }
}

/// `BuildContexts.execute`.
#[allow(clippy::too_many_arguments)]
fn build(g: &mut Game, frame: &Frame, input: Arc<str>, chain: Arc<Chain>, mut stage: usize, mut mods: Mods, original: Source, mut sources: Vec<Source>) {
    let fork_limit = g.exec.fork_limit;
    while stage < chain.modifiers.len() {
        let ctx = &chain.modifiers[stage];
        let node = ctx.last.expect("modifier context without node");
        let tree = g.cmds.tree.clone();
        if tree.node(node).redirect.is_some() && g.cmds.is_forking(node) {
            mods = mods.set_forked();
        }
        match g.cmds.modifier_of(node) {
            Some(Modifier::Custom { f, .. }) => {
                f(g, original, sources, chain.clone(), stage, mods, frame.clone());
                return;
            }
            Some(Modifier::Simple { f, .. }) => {
                g.exec.increment_cost();
                let forked = mods.forked();
                let mut next: Vec<Source> = Vec::new();
                for src in &sources {
                    let r = {
                        let mut c = super::Ctx { g, src, c: ctx, chain: &chain, stage };
                        f(&mut c)
                    };
                    match r {
                        Ok(new) => {
                            if next.len() + new.len() >= fork_limit as usize {
                                handle_error(
                                    g,
                                    &original,
                                    &CmdError::new(Text::tr("command.forkLimit", vec![Text::plain(fork_limit.to_string())])),
                                    forked,
                                );
                                return;
                            }
                            next.extend(new);
                        }
                        Err(e) => {
                            // runModifier: forked mode swallows the error (empty
                            // result); BuildContexts passes a no-op consumer
                            if forked {
                                continue;
                            }
                            handle_error(g, src, &e, false);
                            return;
                        }
                    }
                }
                sources = next;
            }
            None => {}
        }
        stage += 1;
    }
    if sources.is_empty() {
        if mods.is_return() {
            g.exec.queue_next(Entry { frame: frame.clone(), action: Action::Fallthrough });
        }
        return;
    }
    let exe = &chain.executable;
    let node = exe.command.expect("executable without command");
    match g.cmds.executor(node) {
        Some(Executor::Custom(f)) => {
            for s in sources {
                f(g, s, chain.clone(), mods, frame.clone());
            }
        }
        _ => {
            if mods.is_return() {
                let mut first = sources.swap_remove(0);
                first.callback = chain_cb(&first.callback, &frame.ret);
                sources = vec![first];
            }
            match sources.len() {
                1 => {
                    let s = sources.pop().unwrap();
                    g.exec.queue_next(Entry { frame: frame.clone(), action: Action::Execute { input, mods, chain, source: s } });
                }
                2 => {
                    let b = sources.pop().unwrap();
                    let a = sources.pop().unwrap();
                    g.exec.queue_next(Entry { frame: frame.clone(), action: Action::Execute { input: input.clone(), mods, chain: chain.clone(), source: a } });
                    g.exec.queue_next(Entry { frame: frame.clone(), action: Action::Execute { input, mods, chain, source: b } });
                }
                _ => {
                    g.exec.queue_next(Entry {
                        frame: frame.clone(),
                        action: Action::ExecEach { input, mods, chain, sources: Arc::new(sources), index: 0 },
                    });
                }
            }
        }
    }
}

/// `ExecuteCommand.execute` + `ContextChain.runExecutable`.
fn execute(g: &mut Game, _input: &str, mods: Mods, chain: &Arc<Chain>, source: Source) {
    g.exec.increment_cost();
    let exe = &chain.executable;
    let node = exe.command.expect("executable without command");
    let Some(Executor::Simple(f)) = g.cmds.executor(node) else {
        let e = super::unimplemented_error(g, node);
        call_cb(g, &source.callback, false, 0);
        if !mods.forked() {
            handle_error(g, &source, &e, false);
        }
        return;
    };
    let r = {
        let n = chain.modifiers.len();
        let mut c = super::Ctx { g, src: &source, c: exe, chain, stage: n };
        f(&mut c)
    };
    match r {
        Ok(v) => call_cb(g, &source.callback, true, v),
        Err(e) => {
            call_cb(g, &source.callback, false, 0);
            if !mods.forked() {
                handle_error(g, &source, &e, false);
            }
        }
    }
}

/// `ExecutionCommandSource.handleError`.
pub fn handle_error(g: &mut Game, src: &Source, e: &CmdError, forked: bool) {
    if !forked {
        super::feedback::send_failure(g, src, &e.msg);
    }
}
