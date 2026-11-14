//! Block loot-table evaluation for vanilla 26.3 (`rine_data::raw::BLOCK_LOOT`).
//!
//! The JSON is parsed once (see [`loot`]) into compact trees indexed by block
//! index. Server tools carry no enchantments yet, so fortune and silk touch are
//! always absent: `table_bonus` uses its level 0 chance, silk-touch checks
//! fail, and `apply_bonus` leaves counts unchanged.

use super::player::Stack;
use ahash::AHashMap;
use rine_data::blocks::flag;
use rine_data::Data;
use serde_json::{Map, Value};
use std::cell::Cell;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// Drops of a block broken in `state` by `tool_item` (0 = empty hand).
pub fn block_drops(state: u16, tool_item: u32) -> Vec<Stack> {
    let d = rine_data::get();
    let lt = loot();
    let block = d.blocks.state(state).block as u32;
    let Some(Some(table)) = lt.tables.get(block as usize) else {
        return Vec::new();
    };
    if d.blocks.has(state, flag::TOOL_REQUIRED) && !lt.tool_correct(block, tool_item) {
        return Vec::new();
    }
    let ctx = Ctx { d, state, block, tool: tool_item, shears: lt.shears };
    let mut out = Vec::new();
    table.run(&ctx, &mut out);
    out.retain(|s| !s.is_empty());
    out
}

/// Per-tick break progress of `state` mined by `tool_item`, as vanilla's
/// `BlockState.getDestroyProgress` (no enchantments or effects; the player is
/// assumed on the ground and dry). A value >= 1 means instant break.
pub fn destroy_progress(state: u16, tool_item: u32) -> f32 {
    let d = rine_data::get();
    let st = d.blocks.state(state);
    let hard = st.hardness;
    if hard < 0.0 {
        return 0.0;
    }
    if hard == 0.0 {
        return 1.0;
    }
    let lt = loot();
    let block = st.block as u32;
    let mut speed = 1.0f32;
    if tool_item == lt.shears {
        speed = lt.shears_speed.get(&block).copied().unwrap_or(1.0);
    } else if let Some(info) = lt.tools.get(tool_item as usize) {
        if let Some(kind) = info.kind {
            if lt.mineable[kind].binary_search(&block).is_ok() {
                // wooden, stone, copper, iron, golden, diamond, netherite
                const SPEED: [f32; 7] = [2.0, 4.0, 5.0, 6.0, 12.0, 8.0, 9.0];
                speed = info.tier.map(|t| SPEED[t]).unwrap_or(1.0);
            }
        }
    }
    let harvest = !d.blocks.has(state, flag::TOOL_REQUIRED) || lt.tool_correct(block, tool_item);
    speed / hard / if harvest { 30.0 } else { 100.0 }
}

// ---------------------------------------------------------------------------
// Compiled representation
// ---------------------------------------------------------------------------

/// Tool kinds, in the order of `TOOL_KINDS`.
const TOOL_KINDS: [(&str, &str, &str); 4] = [
    ("minecraft:pickaxes", "_pickaxe", "minecraft:mineable/pickaxe"),
    ("minecraft:axes", "_axe", "minecraft:mineable/axe"),
    ("minecraft:shovels", "_shovel", "minecraft:mineable/shovel"),
    ("minecraft:hoes", "_hoe", "minecraft:mineable/hoe"),
];

/// Tool tiers (name prefix, block tag of blocks this tier cannot harvest).
const TIERS: [(&str, &str); 7] = [
    ("wooden_", "minecraft:incorrect_for_wooden_tool"),
    ("stone_", "minecraft:incorrect_for_stone_tool"),
    ("copper_", "minecraft:incorrect_for_copper_tool"),
    ("iron_", "minecraft:incorrect_for_iron_tool"),
    ("golden_", "minecraft:incorrect_for_gold_tool"),
    ("diamond_", "minecraft:incorrect_for_diamond_tool"),
    ("netherite_", "minecraft:incorrect_for_netherite_tool"),
];

const SHEARS_TAGS: [&str; 3] = [
    "minecraft:shears_minor_breaking_speed",
    "minecraft:shears_major_breaking_speed",
    "minecraft:shears_extreme_breaking_speed",
];

#[derive(Clone, Copy, Default)]
struct ToolInfo {
    kind: Option<usize>,
    tier: Option<usize>,
}

struct Loot {
    /// Indexed by block index (`BlockState::block`); `None` = no loot table.
    tables: Vec<Option<Table>>,
    /// Indexed by item id.
    tools: Vec<ToolInfo>,
    /// Sorted block indices per tool kind (`TOOL_KINDS`).
    mineable: Vec<Vec<u32>>,
    /// Sorted block indices per tier (`TIERS`).
    incorrect: Vec<Vec<u32>>,
    /// Sorted block indices the shears speed up.
    shears_blocks: Vec<u32>,
    /// Item id of `minecraft:shears`, `u32::MAX` if absent.
    shears: u32,
    /// Shears mining speed per block index (minor 2, major 5, extreme 15).
    shears_speed: AHashMap<u32, f32>,
}

static LOOT: OnceLock<Loot> = OnceLock::new();

fn loot() -> &'static Loot {
    LOOT.get_or_init(|| Loot::build(rine_data::get()))
}

impl Loot {
    fn build(d: &Data) -> Loot {
        let items: AHashMap<String, u32> = d.items.items.iter().map(|i| (i.name.clone(), i.id)).collect();
        let b = Builder { d, items: &items };

        let raw: Map<String, Value> =
            serde_json::from_str(rine_data::raw::BLOCK_LOOT).expect("block_loot.json");
        let mut tables: Vec<Option<Table>> = (0..d.blocks.blocks.len()).map(|_| None).collect();
        for (name, v) in &raw {
            if let Some(&idx) = d.blocks.by_name.get(name.as_str()) {
                tables[idx as usize] = Some(b.table(v));
            }
        }

        let mut tools = vec![ToolInfo::default(); d.items.items.len()];
        for it in &d.items.items {
            if let Some(slot) = tools.get_mut(it.id as usize) {
                *slot = tool_info(d, &it.name);
            }
        }

        let mineable = TOOL_KINDS
            .iter()
            .map(|k| b.block_names(tag_members(d, "minecraft:block", k.2)))
            .collect();
        let incorrect = TIERS
            .iter()
            .map(|t| b.block_names(tag_members(d, "minecraft:block", t.1)))
            .collect();
        let shears_blocks = sorted(
            SHEARS_TAGS
                .iter()
                .flat_map(|t| b.block_names(tag_members(d, "minecraft:block", t)))
                .collect(),
        );
        let shears = items.get("minecraft:shears").copied().unwrap_or(u32::MAX);

        let mut shears_speed = AHashMap::new();
        for (t, sp) in SHEARS_TAGS.iter().zip([2.0f32, 5.0, 15.0]) {
            for blk in b.block_names(tag_members(d, "minecraft:block", t)) {
                shears_speed.insert(blk, sp);
            }
        }
        Loot { tables, tools, mineable, incorrect, shears_blocks, shears, shears_speed }
    }

    /// Whether `tool` may harvest `block` when the block requires a correct tool.
    fn tool_correct(&self, block: u32, tool: u32) -> bool {
        if tool == self.shears {
            return self.shears_blocks.binary_search(&block).is_ok();
        }
        let Some(info) = self.tools.get(tool as usize) else {
            return false;
        };
        let Some(kind) = info.kind else {
            return false;
        };
        if self.mineable[kind].binary_search(&block).is_err() {
            return false;
        }
        match info.tier {
            Some(tier) => self.incorrect[tier].binary_search(&block).is_err(),
            None => true,
        }
    }
}

fn tag_members<'a>(d: &'a Data, reg: &str, tag: &str) -> &'a [String] {
    d.registries
        .tags
        .get(reg)
        .and_then(|m| m.get(tag))
        .map(|v| v.as_slice())
        .unwrap_or(&[])
}

fn tool_info(d: &Data, name: &str) -> ToolInfo {
    let kind = TOOL_KINDS.iter().position(|(tag, suffix, _)| {
        tag_members(d, "minecraft:item", tag).iter().any(|n| n.as_str() == name) || name.ends_with(suffix)
    });
    let Some(kind) = kind else {
        return ToolInfo::default();
    };
    let short = name.strip_prefix("minecraft:").unwrap_or(name);
    let tier = TIERS.iter().position(|(prefix, _)| short.starts_with(prefix));
    ToolInfo { kind: Some(kind), tier }
}

fn sorted(mut v: Vec<u32>) -> Vec<u32> {
    v.sort_unstable();
    v.dedup();
    v
}

fn as_list(v: &Value) -> Vec<&Value> {
    match v {
        Value::Array(a) => a.iter().collect(),
        _ => vec![v],
    }
}

/// Values of the first present keys, flattening arrays.
fn fields_list<'a>(o: &'a Map<String, Value>, keys: &[&str]) -> Vec<&'a Value> {
    let mut out = Vec::new();
    for k in keys {
        if let Some(v) = o.get(*k) {
            out.extend(as_list(v));
        }
    }
    out
}

struct Builder<'a> {
    d: &'a Data,
    items: &'a AHashMap<String, u32>,
}

impl Builder<'_> {
    fn block_names(&self, names: &[String]) -> Vec<u32> {
        sorted(names.iter().filter_map(|n| self.d.blocks.by_name.get(n.as_str()).copied()).collect())
    }

    fn item_names(&self, names: &[String]) -> Vec<u32> {
        sorted(names.iter().filter_map(|n| self.items.get(n.as_str()).copied()).collect())
    }

    /// Block indices from a name, `#tag`, or list of either.
    fn block_ids(&self, v: &Value) -> Vec<u32> {
        let mut out = Vec::new();
        for s in as_list(v).into_iter().filter_map(Value::as_str) {
            if let Some(tag) = s.strip_prefix('#') {
                out.extend(self.block_names(tag_members(self.d, "minecraft:block", tag)));
            } else if let Some(&i) = self.d.blocks.by_name.get(s) {
                out.push(i);
            }
        }
        sorted(out)
    }

    /// Item ids from a name, `#tag`, or list of either.
    fn item_ids(&self, v: &Value) -> Vec<u32> {
        let mut out = Vec::new();
        for s in as_list(v).into_iter().filter_map(Value::as_str) {
            if let Some(tag) = s.strip_prefix('#') {
                out.extend(self.item_names(tag_members(self.d, "minecraft:item", tag)));
            } else if let Some(&i) = self.items.get(s) {
                out.push(i);
            }
        }
        sorted(out)
    }

    fn table(&self, v: &Value) -> Table {
        let Some(o) = v.as_object() else {
            return Table::default();
        };
        Table {
            pools: o.get("pools").map(as_list).unwrap_or_default().into_iter().map(|p| self.pool(p)).collect(),
            funcs: self.funcs(o),
        }
    }

    fn pool(&self, v: &Value) -> Pool {
        let Some(o) = v.as_object() else {
            return Pool::default();
        };
        Pool {
            rolls: o.get("rolls").map(provider).unwrap_or_default(),
            conds: self.conds(o),
            funcs: self.funcs(o),
            entries: o.get("entries").map(as_list).unwrap_or_default().into_iter().map(|e| self.entry(e)).collect(),
        }
    }

    fn children(&self, o: &Map<String, Value>) -> Vec<Entry> {
        o.get("children")
            .or_else(|| o.get("entries"))
            .map(as_list)
            .unwrap_or_default()
            .into_iter()
            .map(|e| self.entry(e))
            .collect()
    }

    fn entry(&self, v: &Value) -> Entry {
        let Some(o) = v.as_object() else {
            return Entry::new(Kind::Empty);
        };
        let kind = match o.get("type").and_then(Value::as_str).unwrap_or("") {
            "minecraft:item" => o
                .get("name")
                .and_then(Value::as_str)
                .and_then(|n| self.items.get(n).copied())
                .map_or(Kind::Empty, Kind::Item),
            "minecraft:tag" => match o.get("name").and_then(Value::as_str) {
                Some(n) => Kind::Tag(self.item_names(tag_members(self.d, "minecraft:item", n.trim_start_matches('#')))),
                None => Kind::Empty,
            },
            "minecraft:alternatives" => Kind::Alt(self.children(o)),
            "minecraft:group" => Kind::Group(self.children(o)),
            "minecraft:sequence" => Kind::Seq(self.children(o)),
            // loot_table, dynamic (container contents) and unknown types are not evaluated.
            _ => Kind::Empty,
        };
        Entry {
            conds: self.conds(o),
            funcs: self.funcs(o),
            weight: o.get("weight").and_then(Value::as_u64).unwrap_or(1) as u32,
            kind,
        }
    }

    fn conds(&self, o: &Map<String, Value>) -> Vec<Cond> {
        fields_list(o, &["condition", "conditions"]).into_iter().map(|c| self.cond(c)).collect()
    }

    fn funcs(&self, o: &Map<String, Value>) -> Vec<Func> {
        fields_list(o, &["modifier", "functions"]).into_iter().map(|f| self.func(f)).collect()
    }

    fn terms(&self, o: &Map<String, Value>) -> Vec<Cond> {
        o.get("terms").map(as_list).unwrap_or_default().into_iter().map(|t| self.cond(t)).collect()
    }

    fn cond(&self, v: &Value) -> Cond {
        // String form refers to a predicate by id.
        if let Some(s) = v.as_str() {
            return if s == "minecraft:tool/can_shear" { Cond::ToolIsShears } else { Cond::Never };
        }
        let Some(o) = v.as_object() else {
            return Cond::Never;
        };
        let num = |k: &str| o.get(k).and_then(Value::as_f64);
        match o.get("type").and_then(Value::as_str).unwrap_or("") {
            "minecraft:survives_explosion" => Cond::Always,
            "minecraft:inverted" => match o.get("term") {
                Some(t) => Cond::Not(Box::new(self.cond(t))),
                None => Cond::Never,
            },
            "minecraft:any_of" => Cond::Any(self.terms(o)),
            "minecraft:all_of" => Cond::All(self.terms(o)),
            "minecraft:random_chance" => Cond::Chance(num("chance").unwrap_or(0.0) as f32),
            // No enchantment on the tool: use the unenchanted chance.
            "minecraft:random_chance_with_enchanted_bonus" => {
                Cond::Chance(num("unenchanted_chance").unwrap_or(0.0) as f32)
            }
            // Fortune level 0: first chance only.
            "minecraft:table_bonus" => Cond::Chance(
                o.get("chances")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0) as f32,
            ),
            "minecraft:match_block" | "minecraft:block_state_property" => {
                let blocks = o.get("blocks").or_else(|| o.get("block")).map(|b| self.block_ids(b));
                let props = o
                    .get("state")
                    .or_else(|| o.get("properties"))
                    .and_then(Value::as_object)
                    .map(|m| {
                        m.iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                            .collect()
                    })
                    .unwrap_or_default();
                Cond::Block { blocks, props }
            }
            "minecraft:match_tool" => {
                let pred = o.get("predicate").and_then(Value::as_object);
                // Enchantment or component requirements cannot be met by our tools.
                if let Some(p) = pred {
                    if p.contains_key("predicates") || p.contains_key("components") {
                        return Cond::Never;
                    }
                }
                let items = pred.and_then(|p| p.get("items")).map(|v| self.item_ids(v));
                Cond::Tool { items }
            }
            // "this" is the breaking player; other entities are absent.
            "minecraft:entity_properties" => {
                if o.get("entity").and_then(Value::as_str) == Some("this") {
                    Cond::Always
                } else {
                    Cond::Never
                }
            }
            _ => Cond::Never,
        }
    }

    fn func(&self, v: &Value) -> Func {
        let Some(o) = v.as_object() else {
            return Func { conds: Vec::new(), op: Op::Noop };
        };
        let op = match o.get("type").and_then(Value::as_str).unwrap_or("") {
            "minecraft:set_count" => Op::SetCount {
                count: o.get("count").map(provider).unwrap_or(Provider::Const(1)),
                add: o.get("add").and_then(Value::as_bool).unwrap_or(false),
            },
            "minecraft:limit_count" => {
                let lim = o.get("limit").and_then(Value::as_object);
                let bound = |k: &str| {
                    lim.and_then(|l| l.get(k)).and_then(Value::as_f64).map(|x| x as i32)
                };
                Op::LimitCount { min: bound("min"), max: bound("max") }
            }
            // apply_bonus (no fortune), explosion_decay, copy_*: no effect here.
            _ => Op::Noop,
        };
        Func { conds: self.conds(o), op }
    }
}

fn provider(v: &Value) -> Provider {
    if let Some(n) = v.as_f64() {
        return Provider::Const(n as i32);
    }
    let Some(o) = v.as_object() else {
        return Provider::Const(1);
    };
    let f = |k: &str| o.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    match o.get("type").and_then(Value::as_str).unwrap_or("") {
        "minecraft:uniform" => Provider::Uniform(f("min") as i32, f("max") as i32),
        "minecraft:binomial" => Provider::Binomial(f("n") as u32, f("p") as f32),
        "minecraft:constant" => Provider::Const(f("value") as i32),
        _ => Provider::Const(1),
    }
}

#[derive(Clone, Copy)]
enum Provider {
    Const(i32),
    Uniform(i32, i32),
    Binomial(u32, f32),
}

impl Default for Provider {
    fn default() -> Self {
        Provider::Const(1)
    }
}

impl Provider {
    fn sample(&self) -> i32 {
        match *self {
            Provider::Const(c) => c,
            Provider::Uniform(lo, hi) => {
                if hi <= lo {
                    lo
                } else {
                    lo + rand_below((hi - lo) as u32 + 1) as i32
                }
            }
            Provider::Binomial(n, p) => (0..n).filter(|_| rand_f32() < p).count() as i32,
        }
    }
}

#[derive(Default)]
struct Table {
    pools: Vec<Pool>,
    funcs: Vec<Func>,
}

#[derive(Default)]
struct Pool {
    rolls: Provider,
    conds: Vec<Cond>,
    funcs: Vec<Func>,
    entries: Vec<Entry>,
}

struct Entry {
    conds: Vec<Cond>,
    funcs: Vec<Func>,
    weight: u32,
    kind: Kind,
}

enum Kind {
    Item(u32),
    /// Item ids of a tag; one is picked uniformly.
    Tag(Vec<u32>),
    /// First child whose conditions pass.
    Alt(Vec<Entry>),
    /// Every child whose conditions pass.
    Group(Vec<Entry>),
    /// Children in order, stopping at the first failing one.
    Seq(Vec<Entry>),
    Empty,
}

impl Default for Kind {
    fn default() -> Self {
        Kind::Empty
    }
}

impl Entry {
    fn new(kind: Kind) -> Self {
        Entry { conds: Vec::new(), funcs: Vec::new(), weight: 1, kind }
    }

    /// Emit this entry's stacks. The entry's own conditions were checked by the caller.
    fn gen(&self, ctx: &Ctx, out: &mut Vec<Stack>) {
        let start = out.len();
        match &self.kind {
            Kind::Item(id) => out.push(Stack::new(*id, 1)),
            Kind::Tag(ids) => {
                if !ids.is_empty() {
                    let id = ids[rand_below(ids.len() as u32) as usize];
                    out.push(Stack::new(id, 1));
                }
            }
            Kind::Alt(ch) => {
                if let Some(c) = ch.iter().find(|c| pass(&c.conds, ctx)) {
                    c.gen(ctx, out);
                }
            }
            Kind::Group(ch) => {
                for c in ch.iter().filter(|c| pass(&c.conds, ctx)) {
                    c.gen(ctx, out);
                }
            }
            Kind::Seq(ch) => {
                for c in ch {
                    if !pass(&c.conds, ctx) {
                        break;
                    }
                    c.gen(ctx, out);
                }
            }
            Kind::Empty => {}
        }
        apply_all(&self.funcs, ctx, &mut out[start..]);
    }
}

impl Table {
    fn run(&self, ctx: &Ctx, out: &mut Vec<Stack>) {
        let start = out.len();
        for p in &self.pools {
            p.run(ctx, out);
        }
        apply_all(&self.funcs, ctx, &mut out[start..]);
    }
}

impl Pool {
    fn run(&self, ctx: &Ctx, out: &mut Vec<Stack>) {
        if !pass(&self.conds, ctx) {
            return;
        }
        let start = out.len();
        let rolls = self.rolls.sample();
        let mut cands: Vec<usize> = Vec::with_capacity(self.entries.len());
        for _ in 0..rolls {
            cands.clear();
            cands.extend((0..self.entries.len()).filter(|&i| pass(&self.entries[i].conds, ctx)));
            let total: u32 = cands.iter().map(|&i| self.entries[i].weight).sum();
            if total == 0 {
                continue;
            }
            let mut r = rand_below(total);
            let mut chosen = cands[0];
            for &i in &cands {
                let w = self.entries[i].weight;
                if r < w {
                    chosen = i;
                    break;
                }
                r -= w;
            }
            self.entries[chosen].gen(ctx, out);
        }
        apply_all(&self.funcs, ctx, &mut out[start..]);
    }
}

struct Func {
    conds: Vec<Cond>,
    op: Op,
}

enum Op {
    SetCount { count: Provider, add: bool },
    LimitCount { min: Option<i32>, max: Option<i32> },
    Noop,
}

impl Func {
    fn apply(&self, s: &mut Stack, ctx: &Ctx) {
        if !pass(&self.conds, ctx) {
            return;
        }
        match &self.op {
            Op::SetCount { count, add } => {
                let n = count.sample();
                s.count = if *add { s.count.saturating_add(n) } else { n };
            }
            Op::LimitCount { min, max } => {
                if let Some(m) = min {
                    s.count = s.count.max(*m);
                }
                if let Some(m) = max {
                    s.count = s.count.min(*m);
                }
            }
            Op::Noop => {}
        }
    }
}

fn apply_all(funcs: &[Func], ctx: &Ctx, stacks: &mut [Stack]) {
    if funcs.is_empty() {
        return;
    }
    for s in stacks.iter_mut() {
        for f in funcs {
            f.apply(s, ctx);
        }
    }
}

enum Cond {
    Always,
    Never,
    /// Tool is `minecraft:shears`.
    ToolIsShears,
    Not(Box<Cond>),
    Any(Vec<Cond>),
    All(Vec<Cond>),
    Chance(f32),
    /// Broken block is in `blocks` (None = any) and has the given properties.
    Block { blocks: Option<Vec<u32>>, props: Vec<(String, String)> },
    /// Tool item is in `items` (None = any item, including empty hand).
    Tool { items: Option<Vec<u32>> },
}

impl Cond {
    fn test(&self, ctx: &Ctx) -> bool {
        match self {
            Cond::Always => true,
            Cond::Never => false,
            Cond::ToolIsShears => ctx.tool == ctx.shears,
            Cond::Not(c) => !c.test(ctx),
            Cond::Any(cs) => cs.iter().any(|c| c.test(ctx)),
            Cond::All(cs) => cs.iter().all(|c| c.test(ctx)),
            Cond::Chance(p) => rand_f32() < *p,
            Cond::Block { blocks, props } => {
                blocks.as_ref().map_or(true, |s| s.binary_search(&ctx.block).is_ok())
                    && props
                        .iter()
                        .all(|(k, v)| ctx.d.blocks.get_prop(ctx.state, k) == Some(v.as_str()))
            }
            Cond::Tool { items } => items.as_ref().map_or(true, |s| s.binary_search(&ctx.tool).is_ok()),
        }
    }
}

fn pass(conds: &[Cond], ctx: &Ctx) -> bool {
    conds.iter().all(|c| c.test(ctx))
}

struct Ctx {
    d: &'static Data,
    state: u16,
    /// Block index.
    block: u32,
    /// Tool item id, 0 for empty hand.
    tool: u32,
    shears: u32,
}

// ---------------------------------------------------------------------------
// RNG: per-thread xorshift64, seeded from the clock and a stack address.
// ---------------------------------------------------------------------------

thread_local! {
    static RNG: Cell<u64> = Cell::new(seed());
}

fn seed() -> u64 {
    let t = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let marker = 0u8;
    let addr = &marker as *const u8 as u64;
    let s = mix(t ^ addr.rotate_left(29));
    if s == 0 {
        1
    } else {
        s
    }
}

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn next_u64() -> u64 {
    RNG.with(|c| {
        let mut x = c.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        c.set(x);
        x
    })
}

/// Uniform in `0..n`; `n` must be non-zero.
fn rand_below(n: u32) -> u32 {
    (next_u64() % n as u64) as u32
}

/// Uniform in `[0, 1)`.
fn rand_f32() -> f32 {
    (next_u64() >> 40) as f32 * (1.0 / 16_777_216.0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn init() {
        rine_data::init();
    }

    fn item(name: &str) -> u32 {
        rine_data::get().items.by_name(name).unwrap_or_else(|| panic!("no item {name}")).id
    }

    fn state(s: &str) -> u16 {
        rine_data::get().blocks.parse_state(s).unwrap_or_else(|| panic!("no state {s}"))
    }

    fn tuples(v: &[Stack]) -> Vec<(u32, i32)> {
        v.iter().map(|s| (s.item, s.count)).collect()
    }

    #[test]
    fn loot_stone_wooden_pickaxe_gives_cobblestone() {
        init();
        let r = block_drops(state("minecraft:stone"), item("minecraft:wooden_pickaxe"));
        assert_eq!(tuples(&r), vec![(item("minecraft:cobblestone"), 1)]);
    }

    #[test]
    fn loot_stone_without_tool_gives_nothing() {
        init();
        assert!(block_drops(state("minecraft:stone"), 0).is_empty());
    }

    #[test]
    fn loot_grass_block_gives_dirt() {
        init();
        let r = block_drops(state("minecraft:grass_block"), 0);
        assert_eq!(tuples(&r), vec![(item("minecraft:dirt"), 1)]);
    }

    #[test]
    fn loot_diamond_ore_needs_iron_pickaxe() {
        init();
        let s = state("minecraft:diamond_ore");
        let r = block_drops(s, item("minecraft:iron_pickaxe"));
        assert_eq!(tuples(&r), vec![(item("minecraft:diamond"), 1)]);
        assert!(block_drops(s, item("minecraft:stone_pickaxe")).is_empty());
    }

    #[test]
    fn loot_oak_slab_double_gives_two() {
        init();
        let r = block_drops(state("minecraft:oak_slab[type=double]"), 0);
        assert_eq!(tuples(&r), vec![(item("minecraft:oak_slab"), 2)]);
    }

    #[test]
    fn loot_oak_door_halves() {
        init();
        assert!(block_drops(state("minecraft:oak_door[half=upper]"), 0).is_empty());
        let r = block_drops(state("minecraft:oak_door[half=lower]"), 0);
        assert_eq!(tuples(&r), vec![(item("minecraft:oak_door"), 1)]);
    }

    #[test]
    fn loot_short_grass_sometimes_drops_seeds() {
        init();
        let s = state("minecraft:short_grass");
        let seeds = item("minecraft:wheat_seeds");
        let hits = (0..1000)
            .filter(|_| block_drops(s, 0).iter().any(|st| st.item == seeds))
            .count();
        assert!(hits > 0, "expected some wheat seeds in 1000 breaks");
        assert!(hits < 1000, "expected some breaks without seeds");
    }

    #[test]
    fn loot_oak_leaves_with_shears_drop_leaves() {
        init();
        let r = block_drops(state("minecraft:oak_leaves"), item("minecraft:shears"));
        assert_eq!(tuples(&r), vec![(item("minecraft:oak_leaves"), 1)]);
    }
}
