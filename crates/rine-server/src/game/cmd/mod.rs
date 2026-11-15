//! Commands: vanilla 26.3 grammar (`rine-command`), executors attached by
//! tree path, the vanilla execution queue (`exec`), sources and output.
//!
//! Hot paths: the command tree packet is encoded once per op level; parse
//! results are source-independent and cached by (input, level), so command
//! blocks and functions never re-parse (vanilla re-parses command blocks on
//! every run).

pub mod cmdblock;
pub mod commands;
pub mod exec;
pub mod feedback;
pub mod rules;
pub mod select;
pub mod source;

use crate::game::Game;
use ahash::AHashMap;
use bytes::Bytes;
use exec::{Frame, Mods};
use rine_command::args::{Arg, ParseEnv};
use rine_command::dispatch::{Chain, Context};
use rine_command::{CmdError, CmdResult, NodeId, Tree};
use rine_protocol::nbt::Tag;
use rine_protocol::text::{hover_text, Text};
use source::Source;
use std::sync::Arc;

pub use rine_command::error::lit;

pub type SimpleFn = fn(&mut Ctx) -> CmdResult<i32>;
/// `CustomCommandExecutor`: runs with access to the queue and frame.
pub type CustomExecFn = fn(&mut Game, Source, Arc<Chain>, Mods, Frame);
pub type ModifierFn = fn(&mut Ctx) -> CmdResult<Vec<Source>>;
/// `CustomModifierExecutor`: (original source, current sources, chain,
/// stage index of this modifier, modifiers, frame).
pub type CustomModFn = fn(&mut Game, Source, Vec<Source>, Arc<Chain>, usize, Mods, Frame);

#[derive(Clone, Copy)]
pub enum Executor {
    Simple(SimpleFn),
    Custom(CustomExecFn),
}

#[derive(Clone, Copy)]
pub enum Modifier {
    Simple { forks: bool, f: ModifierFn },
    Custom { forks: bool, f: CustomModFn },
}

const CACHE_MAX: usize = 16384;

/// Command-visible entity state not stored in the entity arrays.
#[derive(Debug, Clone, Default)]
pub struct EntMeta {
    /// Scoreboard tags (`/tag`, `Tags`).
    pub tags: Vec<String>,
    pub custom_name: Option<Text>,
    pub custom_name_visible: bool,
    /// Rotation for entity kinds that do not track one.
    pub rot: [f32; 2],
    pub no_gravity: bool,
    pub silent: bool,
    pub glowing: bool,
    pub invulnerable: bool,
    /// Remaining NBT not modelled elsewhere (merged by `/data`).
    pub extra: Option<Tag>,
}

pub struct Commands {
    pub tree: Arc<Tree>,
    execs: Vec<Option<Executor>>,
    mods: Vec<Option<Modifier>>,
    packets: [Option<Bytes>; 5],
    cache: AHashMap<(Box<str>, u8), Result<Arc<Chain>, CmdError>>,
}

impl Commands {
    pub fn new() -> Self {
        let tree = Arc::new(Tree::vanilla());
        let n = tree.nodes.len();
        let mut c = Self { tree, execs: vec![None; n], mods: vec![None; n], packets: Default::default(), cache: AHashMap::new() };
        commands::register_all(&mut c);
        c
    }

    fn node_of(&self, path: &str) -> NodeId {
        self.tree.find(path).unwrap_or_else(|| panic!("command path not in the vanilla tree: {path:?}"))
    }

    /// Attach an executor to an executable node.
    pub fn exec(&mut self, path: &str, f: SimpleFn) {
        let id = self.node_of(path);
        assert!(self.tree.node(id).executable, "not executable: {path:?}");
        self.execs[id as usize] = Some(Executor::Simple(f));
    }
    pub fn exec_custom(&mut self, path: &str, f: CustomExecFn) {
        let id = self.node_of(path);
        assert!(self.tree.node(id).executable, "not executable: {path:?}");
        self.execs[id as usize] = Some(Executor::Custom(f));
    }
    /// Attach a redirect modifier to a redirecting node.
    pub fn modifier(&mut self, path: &str, forks: bool, f: ModifierFn) {
        let id = self.node_of(path);
        assert!(self.tree.node(id).redirect.is_some(), "not a redirect: {path:?}");
        self.mods[id as usize] = Some(Modifier::Simple { forks, f });
    }
    pub fn modifier_custom(&mut self, path: &str, forks: bool, f: CustomModFn) {
        let id = self.node_of(path);
        assert!(self.tree.node(id).redirect.is_some(), "not a redirect: {path:?}");
        self.mods[id as usize] = Some(Modifier::Custom { forks, f });
    }

    pub fn executor(&self, node: NodeId) -> Option<Executor> {
        self.execs.get(node as usize).copied().flatten()
    }
    pub fn modifier_of(&self, node: NodeId) -> Option<Modifier> {
        self.mods.get(node as usize).copied().flatten()
    }
    pub fn is_forking(&self, node: NodeId) -> bool {
        matches!(self.modifier_of(node), Some(Modifier::Simple { forks: true, .. }) | Some(Modifier::Custom { forks: true, .. }))
    }

    /// Framed `commands` packet for an op level (encoded once).
    pub fn packet(&mut self, level: u8) -> Bytes {
        let l = level.min(4) as usize;
        if let Some(b) = &self.packets[l] {
            return b.clone();
        }
        let tree = self.tree.clone();
        let p = rine_protocol::frame::packet(rine_protocol::ids::play::C_COMMANDS, |b| {
            rine_command::packet::commands_packet_body(&tree, l as u8, b);
        });
        let f = crate::out::frame(p);
        self.packets[l] = Some(f.clone());
        f
    }

    /// Add plugin commands (literal + greedy string).
    pub fn add_plugin_command(&mut self, name: &str) {
        let mut t = (*self.tree).clone_tree();
        t.add_plugin_command(name);
        self.tree = Arc::new(t);
        let n = self.tree.nodes.len();
        self.execs.resize(n, None);
        self.mods.resize(n, None);
        if let Some(id) = self.tree.find(name) {
            self.execs[id as usize] = Some(Executor::Simple(commands::plugin_command));
        }
        if let Some(id) = self.tree.find(&format!("{name} args")) {
            self.execs[id as usize] = Some(Executor::Simple(commands::plugin_command));
        }
        self.packets = Default::default();
        self.cache.clear();
    }

    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Parse with caching (source-independent results).
    pub fn parse(&mut self, input: &str, level: u8) -> Result<Arc<Chain>, CmdError> {
        if let Some(r) = self.cache.get(&(Box::<str>::from(input), level)) {
            return r.clone();
        }
        let res = rine_command::parse(&self.tree, input, &ParseEnv::new(level)).into_chain().map(Arc::new);
        if self.cache.len() >= CACHE_MAX {
            self.cache.clear();
        }
        self.cache.insert((input.into(), level), res.clone());
        res
    }

    /// Executable paths with no executor (coverage).
    pub fn missing(&self) -> Vec<&'static str> {
        self.tree.nodes.iter().enumerate().filter(|(i, n)| n.executable && self.execs[*i].is_none()).map(|(_, n)| n.path).collect()
    }
}

impl Default for Commands {
    fn default() -> Self {
        Self::new()
    }
}

trait CloneTree {
    fn clone_tree(&self) -> Tree;
}
impl CloneTree for Tree {
    fn clone_tree(&self) -> Tree {
        let mut t = Tree::vanilla();
        // re-add plugin commands present in self
        for &c in &self.nodes[0].children {
            let n = &self.nodes[c as usize];
            if t.find(n.path).is_none() {
                t.add_plugin_command(n.name);
            }
        }
        t
    }
}

/// Executor context: the game, the executing source, the parsed context.
pub struct Ctx<'a> {
    pub g: &'a mut Game,
    pub src: &'a Source,
    pub c: &'a Context,
    pub chain: &'a Arc<Chain>,
    /// Index of this context in the chain (modifiers first).
    pub stage: usize,
}

impl<'a> Ctx<'a> {
    pub fn arg(&self, name: &str) -> &Arg {
        &self.c.arg(name).unwrap_or_else(|| panic!("missing argument {name}")).value
    }
    pub fn has(&self, name: &str) -> bool {
        self.c.has(name)
    }
    /// Literal at position `i` in this stage's node list.
    pub fn lit(&self, i: usize) -> &'static str {
        let tree = self.g.cmds.tree.clone();
        let id = self.c.nodes[i].0;
        tree.node(id).name
    }
    /// Path of the executing node.
    pub fn path(&self) -> &'static str {
        let tree = self.g.cmds.tree.clone();
        let id = self.c.command.or(self.c.last).unwrap();
        tree.node(id).path
    }
    pub fn int(&self, n: &str) -> i32 {
        match self.arg(n) {
            Arg::Int(v) => *v,
            a => panic!("{n}: not an int: {a:?}"),
        }
    }
    pub fn int_or(&self, n: &str, d: i32) -> i32 {
        if self.has(n) {
            self.int(n)
        } else {
            d
        }
    }
    pub fn long(&self, n: &str) -> i64 {
        match self.arg(n) {
            Arg::Long(v) => *v,
            a => panic!("{n}: not a long: {a:?}"),
        }
    }
    pub fn float(&self, n: &str) -> f32 {
        match self.arg(n) {
            Arg::Float(v) => *v,
            a => panic!("{n}: not a float: {a:?}"),
        }
    }
    pub fn double(&self, n: &str) -> f64 {
        match self.arg(n) {
            Arg::Double(v) => *v,
            a => panic!("{n}: not a double: {a:?}"),
        }
    }
    pub fn bool(&self, n: &str) -> bool {
        match self.arg(n) {
            Arg::Bool(v) => *v,
            a => panic!("{n}: not a bool: {a:?}"),
        }
    }
    pub fn str(&self, n: &str) -> &str {
        match self.arg(n) {
            Arg::Str(v) => v,
            Arg::Id(v) => v,
            a => panic!("{n}: not a string: {a:?}"),
        }
    }
    pub fn id(&self, n: &str) -> &str {
        match self.arg(n) {
            Arg::Id(v) => v,
            Arg::Str(v) => v,
            a => panic!("{n}: not an id: {a:?}"),
        }
    }
    pub fn enum_(&self, n: &str) -> u8 {
        match self.arg(n) {
            Arg::Enum(v) => *v,
            a => panic!("{n}: not an enum: {a:?}"),
        }
    }
    pub fn nbt(&self, n: &str) -> &Tag {
        match self.arg(n) {
            Arg::Nbt(t) => t,
            a => panic!("{n}: not nbt: {a:?}"),
        }
    }
    pub fn coords(&self, n: &str) -> rine_command::args::Coords {
        match self.arg(n) {
            Arg::Coords(c) => *c,
            a => panic!("{n}: not coordinates: {a:?}"),
        }
    }
    pub fn pose(&self) -> rine_command::args::coords::SourcePose {
        select::pose(self.g, self.src)
    }
    /// vec3 position.
    pub fn vec3(&self, n: &str) -> [f64; 3] {
        let p = self.pose();
        self.coords(n).position(&p)
    }
    /// vec2 (x, z).
    pub fn vec2(&self, n: &str) -> [f32; 2] {
        let p = self.coords(n).position(&self.pose());
        [p[0] as f32, p[2] as f32]
    }
    /// rotation (xRot, yRot).
    pub fn rotation(&self, n: &str) -> [f32; 2] {
        self.coords(n).rotation(&self.pose())
    }
    /// `BlockPosArgument.getBlockPos` (no checks).
    pub fn block_pos_unchecked(&self, n: &str) -> [i32; 3] {
        self.coords(n).block_pos(&self.pose())
    }
    /// `getLoadedBlockPos`: loaded chunk and inside build height.
    pub fn block_pos(&self, n: &str) -> CmdResult<[i32; 3]> {
        let p = self.block_pos_unchecked(n);
        if !self.g.world.is_loaded(p[0], p[2]) {
            return Err(CmdError::key("argument.pos.unloaded"));
        }
        if !(rine_world::MIN_Y..rine_world::MIN_Y + rine_world::HEIGHT).contains(&p[1]) {
            return Err(CmdError::key("argument.pos.outofworld"));
        }
        Ok(p)
    }
    /// `getSpawnablePos`: within ±30M horizontally and ±20M vertically.
    pub fn spawnable_pos(&self, n: &str) -> CmdResult<[i32; 3]> {
        let p = self.block_pos_unchecked(n);
        if p[0].abs() >= 30_000_000 || p[2].abs() >= 30_000_000 || p[1].abs() >= 20_000_000 {
            return Err(CmdError::key("argument.pos.outofbounds"));
        }
        Ok(p)
    }
    /// `column_pos` (x, z).
    pub fn column_pos(&self, n: &str) -> [i32; 2] {
        let p = self.block_pos_unchecked(n);
        [p[0], p[2]]
    }

    /// `EntityArgument.getEntities` (error when empty).
    pub fn entities(&mut self, n: &str) -> CmdResult<Vec<i32>> {
        let v = self.optional_entities(n)?;
        if v.is_empty() {
            return Err(CmdError::key("argument.entity.notfound.entity"));
        }
        Ok(v)
    }
    /// `EntityArgument.getOptionalEntities`.
    pub fn optional_entities(&mut self, n: &str) -> CmdResult<Vec<i32>> {
        let sel = match self.arg(n) {
            Arg::Selector(s) => (**s).clone(),
            a => panic!("{n}: not a selector: {a:?}"),
        };
        select::find_entities(self.g, self.src, &sel)
    }
    /// `EntityArgument.getEntity` (exactly one).
    pub fn entity(&mut self, n: &str) -> CmdResult<i32> {
        let sel = match self.arg(n) {
            Arg::Selector(s) => (**s).clone(),
            a => panic!("{n}: not a selector: {a:?}"),
        };
        select::find_single_entity(self.g, self.src, &sel)
    }
    /// `EntityArgument.getPlayers` (player slots, error when empty).
    pub fn players(&mut self, n: &str) -> CmdResult<Vec<usize>> {
        let v = self.optional_players(n)?;
        if v.is_empty() {
            return Err(CmdError::key("argument.entity.notfound.player"));
        }
        Ok(v)
    }
    pub fn optional_players(&mut self, n: &str) -> CmdResult<Vec<usize>> {
        let sel = match self.arg(n) {
            Arg::Selector(s) => (**s).clone(),
            a => panic!("{n}: not a selector: {a:?}"),
        };
        select::find_players(self.g, self.src, &sel)
    }
    /// `EntityArgument.getPlayer` (exactly one player slot).
    pub fn player(&mut self, n: &str) -> CmdResult<usize> {
        let sel = match self.arg(n) {
            Arg::Selector(s) => (**s).clone(),
            a => panic!("{n}: not a selector: {a:?}"),
        };
        select::find_single_player(self.g, self.src, &sel)
    }
    /// `GameProfileArgument.getGameProfiles`: (name, uuid).
    pub fn profiles(&mut self, n: &str) -> CmdResult<Vec<(String, uuid::Uuid)>> {
        let arg = match self.arg(n) {
            Arg::Profile(p) => (**p).clone(),
            a => panic!("{n}: not a profile: {a:?}"),
        };
        select::resolve_profiles(self.g, self.src, &arg)
    }
    /// `MessageArgument.getMessage` resolved (selectors expanded).
    pub fn message(&mut self, n: &str) -> CmdResult<Text> {
        let m = match self.arg(n) {
            Arg::Message(m) => (**m).clone(),
            a => panic!("{n}: not a message: {a:?}"),
        };
        select::resolve_message(self.g, self.src, &m)
    }
    /// `ComponentArgument` resolved for this source.
    pub fn component(&mut self, n: &str) -> CmdResult<Text> {
        let t = self.nbt(n).clone();
        select::resolve_component(self.g, self.src, &t)
    }

    /// `sendSuccess`.
    pub fn success(&mut self, t: Text, broadcast: bool) {
        feedback::send_success(self.g, self.src, &t, broadcast);
    }
    /// `sendFailure` (most failures should be `Err(CmdError)` instead).
    pub fn failure(&mut self, t: Text) {
        feedback::send_failure(self.g, self.src, &t);
    }
    /// `sendSystemMessage`.
    pub fn system(&mut self, t: Text) {
        feedback::send_system(self.g, self.src, &t);
    }
    /// Display name of an entity / player.
    pub fn name_of(&self, eid: i32) -> Text {
        feedback::display_name(self.g, eid)
    }
    pub fn player_name(&self, slot: usize) -> Text {
        feedback::player_display_name(self.g, slot)
    }
}

/// Plain English rendering (console).
pub fn render_plain(t: &Text) -> String {
    rine_command::lang::plain(t)
}

/// `PlayerTeam.formatNameForTeam`: hook for the scoreboard teams.
pub fn team_format(g: &Game, holder: &str, name: Text) -> Text {
    commands::scoreboard::team_format(g, holder, name)
}

/// Failure for a tree path that Rine has no executor for yet.
pub fn unimplemented_error(g: &Game, node: NodeId) -> CmdError {
    let path = g.cmds.tree.node(node).path;
    CmdError::new(Text::key("command.failed").on_hover(hover_text(&Text::plain(format!("Rine: /{path} is not implemented yet")))))
}

/// Parse error output (vanilla `Commands.finishParsing`): the message,
/// then the context line `...input<--[HERE]` with a suggest click.
pub fn report_parse_error(g: &mut Game, src: &Source, e: &CmdError, line: &str) {
    feedback::send_failure(g, src, &e.msg);
    if let Some(input) = &e.input {
        let cursor = e.cursor.min(input.len());
        let mut ctx = Text::empty().color("gray").on_click(rine_protocol::text::click_suggest(format!("/{line}")));
        let before = &input[..cursor];
        let nchars = before.chars().count();
        if nchars > 10 {
            ctx = ctx.push(Text::key("chat.ellipsis"));
        }
        let start = before.char_indices().nth(nchars.saturating_sub(10)).map(|(i, _)| i).unwrap_or(0);
        let start = if nchars > 10 { start } else { 0 };
        ctx = ctx.push(Text::plain(&input[start..cursor]));
        if cursor < input.len() {
            ctx = ctx.push(Text::plain(&input[cursor..]).color("red").underlined());
        }
        ctx = ctx.push(Text::key("command.context.here").color("red").italic());
        feedback::send_failure(g, src, &ctx);
    }
}

/// `performPrefixedCommand`: parse (cached) and execute for `src`.
pub fn perform(g: &mut Game, src: Source, line: &str) {
    let line = line.strip_prefix('/').unwrap_or(line);
    match g.cmds.parse(line, src.level) {
        Err(e) => report_parse_error(g, &src, &e, line),
        Ok(chain) => {
            let input: Arc<str> = Arc::from(line);
            exec::in_context(g, |g| exec::queue_command(g, input, chain, src, None));
        }
    }
}

/// Command typed by a player.
pub fn player_command(g: &mut Game, slot: usize, line: &str) {
    let Some(src) = Source::player(g, slot) else { return };
    tracing::info!("{} issued server command: /{line}", src.text_name);
    perform(g, src, line);
}

/// Command typed into the console.
pub fn console_command(g: &mut Game, line: &str) {
    let src = Source::console(g);
    perform(g, src, line.trim());
}

/// Send the command tree for the player's op level.
pub fn send_tree(g: &mut Game, slot: usize) {
    let Some(name) = g.players.get(slot).and_then(|p| p.as_ref()).map(|p| p.name.clone()) else { return };
    let level = g.op_level_of(&name);
    let f = g.cmds.packet(level);
    if let Some(p) = g.player(slot) {
        p.out.push(f);
    }
}

impl<'a> Ctx<'a> {
    /// `getPlayerOrException`: the executing player's slot.
    pub fn source_player(&self) -> CmdResult<usize> {
        self.src
            .entity
            .and_then(|e| self.g.by_eid.get(&e).copied())
            .ok_or_else(|| CmdError::key("permissions.requires.player"))
    }
    /// `ScoreHolderArgument.getNames` with a wildcard supplier.
    fn holders_with(&mut self, n: &str, wildcard: bool) -> CmdResult<Vec<commands::scoreboard::Holder>> {
        use commands::scoreboard::Holder;
        use rine_command::args::selector::ScoreHolderArg;
        let arg = match self.arg(n) {
            Arg::ScoreHolder(h) => (**h).clone(),
            a => panic!("{n}: not a score holder: {a:?}"),
        };
        let v = match arg {
            ScoreHolderArg::Wildcard => {
                if wildcard {
                    commands::scoreboard::tracked_holders(self.g)
                } else {
                    Vec::new()
                }
            }
            ScoreHolderArg::Name(name) => vec![Holder::named(name)],
            ScoreHolderArg::Selector(sel) => {
                let ents = select::find_entities(self.g, self.src, &sel)?;
                if ents.is_empty() {
                    return Err(CmdError::key("argument.entity.notfound.entity"));
                }
                ents.into_iter().map(|e| Holder { name: select::scoreboard_name(self.g, e), entity: Some(e) }).collect()
            }
        };
        if v.is_empty() {
            return Err(CmdError::key("argument.entity.notfound.entity"));
        }
        Ok(v)
    }
    /// `ScoreHolderArgument.getName` (first of the resolved names).
    pub fn score_holder(&mut self, n: &str) -> CmdResult<commands::scoreboard::Holder> {
        Ok(self.holders_with(n, false)?.swap_remove(0))
    }
    /// `ScoreHolderArgument.getNames`.
    pub fn score_holders(&mut self, n: &str) -> CmdResult<Vec<commands::scoreboard::Holder>> {
        self.holders_with(n, false)
    }
    /// `ScoreHolderArgument.getNamesWithDefaultWildcard`.
    pub fn score_holders_wild(&mut self, n: &str) -> CmdResult<Vec<commands::scoreboard::Holder>> {
        self.holders_with(n, true)
    }
    /// `ObjectiveArgument.getObjective`.
    pub fn objective(&self, n: &str) -> CmdResult<String> {
        let name = self.str(n).to_string();
        if !commands::scoreboard::objective_exists(self.g, &name) {
            return Err(CmdError::tr("arguments.objective.notFound", vec![lit(&name)]));
        }
        Ok(name)
    }
    /// `ObjectiveArgument.getWritableObjective`.
    pub fn writable_objective(&self, n: &str) -> CmdResult<String> {
        let name = self.objective(n)?;
        if !commands::scoreboard::objective_writable(self.g, &name) {
            return Err(CmdError::tr("arguments.objective.readonly", vec![lit(&name)]));
        }
        Ok(name)
    }
    /// `getEntityOrException`.
    pub fn source_entity(&self) -> CmdResult<i32> {
        self.src.entity.filter(|e| select::entity_pos(self.g, *e).is_some()).ok_or_else(|| CmdError::key("permissions.requires.entity"))
    }
}

#[cfg(test)]
mod tests {
    /// Every registered path exists and has the right node kind; prints
    /// the executable paths that still have no executor.
    #[test]
    fn registry_paths_valid() {
        rine_data::init();
        let c = super::Commands::new();
        let missing = c.missing();
        let total = c.tree.nodes.iter().filter(|n| n.executable).count();
        let mut by_cmd: std::collections::BTreeMap<&str, usize> = Default::default();
        for p in &missing {
            *by_cmd.entry(p.split(' ').next().unwrap_or("")).or_default() += 1;
        }
        println!("executors: {}/{} paths", total - missing.len(), total);
        println!("missing: {by_cmd:?}");
    }
}
