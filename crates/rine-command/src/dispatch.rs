//! Brigadier `CommandDispatcher.parse` over the vanilla tree, plus
//! `ContextChain` flattening. Parse results hold only source-independent
//! values (selectors and coordinates are resolved at execution), so a
//! result can be cached and executed many times (command blocks,
//! functions).

use crate::args::{Arg, ParseEnv};
use crate::error::{builtin, CmdError, CmdResult};
use crate::reader::Reader;
use crate::tree::{NodeId, Tree, ROOT};
use std::sync::Arc;

pub const ARGUMENT_SEPARATOR: char = ' ';

#[derive(Debug, Clone)]
pub struct ParsedArg {
    pub start: usize,
    pub end: usize,
    pub value: Arg,
}

/// Brigadier `CommandContext` (built). One per redirect stage.
#[derive(Debug, Clone)]
pub struct Context {
    pub input: Arc<str>,
    pub args: Vec<(&'static str, Arc<ParsedArg>)>,
    pub nodes: Vec<(NodeId, usize, usize)>,
    /// Encompassing range of the parsed nodes.
    pub start: usize,
    pub end: usize,
    /// Node whose executor runs (last parsed node, if executable).
    pub command: Option<NodeId>,
    /// Last parsed node: carries the redirect modifier and fork flag.
    pub last: Option<NodeId>,
    pub child: Option<Box<Context>>,
}

impl Context {
    fn new(input: Arc<str>, start: usize) -> Self {
        Self { input, args: Vec::new(), nodes: Vec::new(), start, end: start, command: None, last: None, child: None }
    }
    fn with_node(&mut self, node: NodeId, start: usize, end: usize) {
        self.nodes.push((node, start, end));
        if self.nodes.len() == 1 {
            self.start = start;
            self.end = end;
        } else {
            self.start = self.start.min(start);
            self.end = self.end.max(end);
        }
        self.last = Some(node);
    }
    pub fn range_is_empty(&self) -> bool {
        self.start == self.end
    }
    pub fn arg(&self, name: &str) -> Option<&ParsedArg> {
        self.args.iter().rev().find(|(n, _)| *n == name).map(|(_, a)| &**a)
    }
    pub fn has(&self, name: &str) -> bool {
        self.arg(name).is_some()
    }
    /// The node path of the executable node ("gamemode gamemode target").
    pub fn command_path<'t>(&self, tree: &'t Tree) -> Option<&'t str> {
        self.command.map(|c| tree.node(c).path)
    }
    /// Literal at depth `i` of this context (`nodes[i]`).
    pub fn node_name<'t>(&self, tree: &'t Tree, i: usize) -> Option<&'t str> {
        self.nodes.get(i).map(|(n, _, _)| tree.node(*n).name)
    }
}

#[derive(Debug, Clone)]
pub struct ParseResults {
    pub context: Context,
    pub reader: Reader,
    /// Per-node errors in insertion order.
    pub errors: Vec<(NodeId, CmdError)>,
}

impl ParseResults {
    /// Vanilla `Commands.getParseException`.
    pub fn parse_error(&self) -> Option<CmdError> {
        if !self.reader.can_read() {
            None
        } else if self.errors.len() == 1 {
            Some(self.errors[0].1.clone())
        } else if self.context.range_is_empty() {
            Some(builtin::unknown_command(&self.reader))
        } else {
            Some(builtin::unknown_argument(&self.reader))
        }
    }

    /// Validate and flatten into a chain (vanilla `finishParsing`).
    pub fn into_chain(self) -> CmdResult<Chain> {
        if let Some(e) = self.parse_error() {
            return Err(e);
        }
        Chain::flatten(self.context).ok_or_else(|| builtin::unknown_command(&self.reader))
    }
}

/// Brigadier `ContextChain`: modifier stages then the executable stage.
#[derive(Debug, Clone)]
pub struct Chain {
    pub modifiers: Vec<Context>,
    pub executable: Context,
}

impl Chain {
    pub fn flatten(root: Context) -> Option<Chain> {
        let mut modifiers = Vec::new();
        let mut cur = root;
        loop {
            match cur.child.take() {
                None => {
                    cur.command?;
                    return Some(Chain { modifiers, executable: cur });
                }
                Some(child) => {
                    modifiers.push(cur);
                    cur = *child;
                }
            }
        }
    }
}

/// Parse `input` (no leading slash) for a source with op `level`.
pub fn parse(tree: &Tree, input: &str, env: &ParseEnv) -> ParseResults {
    let r = Reader::new(input);
    let ctx = Context::new(r.input_arc(), 0);
    parse_nodes(tree, ROOT, &r, ctx, env)
}

/// Parse starting at an arbitrary node (functions, `execute run` reuse).
pub fn parse_from(tree: &Tree, node: NodeId, r: &Reader, env: &ParseEnv) -> ParseResults {
    let ctx = Context::new(r.input_arc(), r.cursor);
    parse_nodes(tree, node, r, ctx, env)
}

/// Brigadier `CommandNode.getRelevantNodes`.
fn relevant(tree: &Tree, node: NodeId, r: &Reader) -> Vec<NodeId> {
    let n = tree.node(node);
    if !n.literals.is_empty() {
        let rest = r.remaining();
        let word = match rest.find(' ') {
            Some(i) => &rest[..i],
            None => rest,
        };
        if let Some(&lit) = n.literals.get(word) {
            return vec![lit];
        }
    }
    n.arguments.clone()
}

fn parse_nodes(tree: &Tree, node: NodeId, original: &Reader, so_far: Context, env: &ParseEnv) -> ParseResults {
    let mut errors: Vec<(NodeId, CmdError)> = Vec::new();
    let mut potentials: Vec<ParseResults> = Vec::new();
    let cursor = original.cursor;
    for child in relevant(tree, node, original) {
        let cn = tree.node(child);
        if cn.level > env.level {
            continue;
        }
        let mut ctx = so_far.clone();
        let mut reader = original.clone();
        let res: CmdResult<()> = (|| {
            let start = reader.cursor;
            match cn.kind {
                crate::tree::Kind::Literal => {
                    // relevant() only returns a literal that matches the word
                    let end = start + cn.name.len();
                    reader.cursor = end;
                    ctx.with_node(child, start, end);
                }
                _ => {
                    let ty = cn.arg.as_ref().expect("argument node without type");
                    let value = crate::args::parse(ty, &mut reader, env)?;
                    let end = reader.cursor;
                    ctx.args.push((cn.name, Arc::new(ParsedArg { start, end, value })));
                    ctx.with_node(child, start, end);
                }
            }
            if reader.can_read() && reader.peek() != ARGUMENT_SEPARATOR {
                return Err(builtin::expected_separator(&reader));
            }
            Ok(())
        })();
        if let Err(e) = res {
            errors.push((child, e));
            continue;
        }
        ctx.command = if cn.executable { Some(child) } else { None };
        let need = if cn.redirect.is_none() { 2 } else { 1 };
        if reader.can_read_n(need) {
            reader.skip();
            if let Some(target) = cn.redirect {
                let child_ctx = Context::new(reader.input_arc(), reader.cursor);
                let parse = parse_nodes(tree, target, &reader, child_ctx, env);
                ctx.child = Some(Box::new(parse.context));
                return ParseResults { context: ctx, reader: parse.reader, errors: parse.errors };
            } else {
                potentials.push(parse_nodes(tree, child, &reader, ctx, env));
            }
        } else {
            potentials.push(ParseResults { context: ctx, reader, errors: Vec::new() });
        }
        let _ = cursor;
    }
    if !potentials.is_empty() {
        if potentials.len() > 1 {
            // stable: fully consumed first, then error-free first
            potentials.sort_by(|a, b| {
                use std::cmp::Ordering::*;
                match (a.reader.can_read(), b.reader.can_read()) {
                    (false, true) => return Less,
                    (true, false) => return Greater,
                    _ => {}
                }
                match (a.errors.is_empty(), b.errors.is_empty()) {
                    (true, false) => Less,
                    (false, true) => Greater,
                    _ => Equal,
                }
            });
        }
        return potentials.swap_remove(0);
    }
    ParseResults { context: so_far, reader: original.clone(), errors }
}
