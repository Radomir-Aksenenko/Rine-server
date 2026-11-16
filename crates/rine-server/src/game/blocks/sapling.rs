//! Saplings: `SaplingBlock` (stage, random tick growth, bone meal) and the
//! `TreeGrower` tables of the vanilla saplings (2x2 mega trees included).
//!
//! Trees are the configured features of rine-worldgen, placed by
//! `Decorator::place_grower` into a copy of the live world (a 3x3 chunk
//! region with only the box around the sapling filled). The writes come
//! back in order and are replayed on the live world with
//! `UPDATE_ALL | UPDATE_KNOWN_SHAPE`, which is the flag `TreeFeature` writes
//! with. Removed saplings use `TreeGrower.removeSapling`, reset ones
//! `resetSaplings` (`UPDATE_NONE`).
//!
//! Not ported:
//! - bee nests: the hive is placed, no bee entities spawn.
//! - `StructureTemplate.updateShapeAtEdge` after the tree (flags 3). Leaves
//!   already hold their final distance from the rine-worldgen `updateLeaves`
//!   port; the edge pass is not done.
//! - the copy box is `BOX_RADIUS` blocks sideways and `BOX_UP` blocks up.
//!   A tree that reaches outside it reads air there.
//! - `AzaleaBlock`, `MangrovePropaguleBlock` and fungi: other families.

use std::collections::HashMap;
use std::sync::OnceLock;

use rine_protocol::BlockPos;
use rine_worldgen::feature::{rng::WorldgenRng, Decorator};

use super::props::{geti, seti, Pr};
use super::{block_of, block_tag, default_state, local_brightness, Rng};
use crate::game::redstone::engine::{self, fl, Lvl, UPDATE_LIMIT};
use crate::game::redstone::Rs;

/// `SaplingBlock.BRIGHTNESS_FOR_SAPLING_GROWTH`.
const BRIGHTNESS_FOR_SAPLING_GROWTH: u8 = 9;
/// `SaplingBlock.TICK_CHANCE_FOR_SAPLING_GROWTH` (1 in 7 random ticks).
const TICK_CHANCE: f32 = 1.0 / 7.0;
/// `SaplingBlock.isBonemealSuccess`.
const BONEMEAL_SUCCESS: f32 = 0.45;

/// Overworld build height (`min_y`, `height`).
const MIN_Y: i32 = -64;
const WORLD_HEIGHT: i32 = 384;
/// Live copy around the sapling: sideways radius, and blocks above and below.
const BOX_RADIUS: i32 = 12;
const BOX_UP: i32 = 48;
const BOX_DOWN: i32 = 1;

/// Configured features the growers use. Names are `features.json` keys.
const GROWER_FEATURES: [&str; 16] = [
    "minecraft:oak",
    "minecraft:fancy_oak",
    "minecraft:oak_bees_005",
    "minecraft:fancy_oak_bees_005",
    "minecraft:birch",
    "minecraft:birch_bees_005",
    "minecraft:spruce",
    "minecraft:mega_spruce",
    "minecraft:mega_pine",
    "minecraft:jungle_tree_no_vine",
    "minecraft:mega_jungle_tree",
    "minecraft:acacia",
    "minecraft:cherry",
    "minecraft:cherry_bees_005",
    "minecraft:dark_oak",
    "minecraft:pale_oak_bonemeal",
];

/// `TreeGrower`: weighted lists of configured features, and the shortest tree (`getMinimumHeight`).
struct Grower {
    trees: &'static [(&'static str, i32)],
    mega: &'static [(&'static str, i32)],
    flowers: &'static [(&'static str, i32)],
    shortest: Option<&'static str>,
}

const OAK: Grower = Grower {
    trees: &[("minecraft:oak", 9), ("minecraft:fancy_oak", 1)],
    mega: &[],
    flowers: &[("minecraft:oak_bees_005", 9), ("minecraft:fancy_oak_bees_005", 1)],
    shortest: Some("minecraft:oak"),
};
const SPRUCE: Grower = Grower {
    trees: &[("minecraft:spruce", 1)],
    mega: &[("minecraft:mega_spruce", 1), ("minecraft:mega_pine", 1)],
    flowers: &[],
    shortest: Some("minecraft:spruce"),
};
const BIRCH: Grower = Grower {
    trees: &[("minecraft:birch", 1)],
    mega: &[],
    flowers: &[("minecraft:birch_bees_005", 1)],
    shortest: Some("minecraft:birch"),
};
const JUNGLE: Grower = Grower {
    trees: &[("minecraft:jungle_tree_no_vine", 1)],
    mega: &[("minecraft:mega_jungle_tree", 1)],
    flowers: &[],
    shortest: Some("minecraft:jungle_tree_no_vine"),
};
const ACACIA: Grower = Grower {
    trees: &[("minecraft:acacia", 1)],
    mega: &[],
    flowers: &[],
    shortest: Some("minecraft:acacia"),
};
const CHERRY: Grower = Grower {
    trees: &[("minecraft:cherry", 1)],
    mega: &[],
    flowers: &[("minecraft:cherry_bees_005", 1)],
    shortest: Some("minecraft:cherry"),
};
const DARK_OAK: Grower = Grower {
    trees: &[],
    mega: &[("minecraft:dark_oak", 1)],
    flowers: &[],
    shortest: None,
};
const PALE_OAK: Grower = Grower {
    trees: &[],
    mega: &[("minecraft:pale_oak_bonemeal", 1)],
    flowers: &[],
    shortest: None,
};

/// Sapling block name and its grower. `poplar_sapling` is not here: the red, orange and
/// yellow poplar trees need the `shelf_mushroom` tree decorator, which is not ported.
const SAPLINGS: [(&str, &Grower); 8] = [
    ("oak_sapling", &OAK),
    ("spruce_sapling", &SPRUCE),
    ("birch_sapling", &BIRCH),
    ("jungle_sapling", &JUNGLE),
    ("acacia_sapling", &ACACIA),
    ("cherry_sapling", &CHERRY),
    ("dark_oak_sapling", &DARK_OAK),
    ("pale_oak_sapling", &PALE_OAK),
];

/// Tables by block id, built once.
struct T {
    flowers: Vec<bool>,
    supports_vegetation: Vec<bool>,
    /// Index into `SAPLINGS` by block id.
    grower: Vec<Option<usize>>,
    air: u16,
}

fn t() -> &'static T {
    static T_: OnceLock<T> = OnceLock::new();
    T_.get_or_init(|| {
        let n = rine_data::get().blocks.blocks.len();
        let mut grower = vec![None; n];
        for (i, (name, _)) in SAPLINGS.iter().enumerate() {
            let id = super::block_id(name);
            if (id as usize) < n {
                grower[id as usize] = Some(i);
            }
        }
        T {
            flowers: block_tag("flowers"),
            supports_vegetation: block_tag("supports_vegetation"),
            grower,
            air: default_state("air"),
        }
    })
}

/// Block name `minecraft:x_sapling` claimed by this family (the bamboo sapling is `plants`').
pub fn classify(n: &str) -> Option<super::Beh> {
    if SAPLINGS.iter().any(|(s, _)| *s == n) {
        Some(super::Beh::Sapling)
    } else {
        None
    }
}

/// Palette of the grower decorator: the live state ids in order, then the states
/// that the trees intern (parsed back to live ids).
pub(super) struct Growth {
    pub(super) dec: Decorator,
    n_live: usize,
    extra: Vec<u16>,
}

/// Canonical state string of a live state (`minecraft:oak_log[axis=y]`), props in block order.
fn canonical(b: &rine_data::blocks::Blocks, st: u16) -> String {
    let blk = b.block_of(st);
    if blk.props.is_empty() {
        return blk.name.clone();
    }
    let mut s = String::with_capacity(blk.name.len() + 16);
    s.push_str(&blk.name);
    s.push('[');
    for (i, p) in blk.props.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&p.name);
        s.push('=');
        s.push_str(b.get_prop(st, &p.name).unwrap_or(""));
    }
    s.push(']');
    s
}

/// Built on the first growth or bone meal; `None` when the decorator does not compile
/// (saplings then do nothing, which is logged once).
fn growth() -> Option<&'static Growth> {
    static G: OnceLock<Option<Growth>> = OnceLock::new();
    G.get_or_init(|| build(&GROWER_FEATURES)).as_ref()
}

/// A decorator with the configured features `growers` (`Decorator::new_growers`) over the live palette.
/// Logged and `None` when it does not compile.
pub(super) fn build(growers: &[&str]) -> Option<Growth> {
    let b = &rine_data::get().blocks;
    let names: Vec<String> = (0..b.states.len()).map(|s| canonical(b, s as u16)).collect();
    let defaults: HashMap<String, String> = b.blocks.iter().map(|blk| (blk.name.clone(), canonical(b, blk.default_state as u16))).collect();
    let dec = match Decorator::new_growers(&names, defaults, growers) {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("block growth disabled: {e}");
            return None;
        }
    };
    let pal = dec.palette();
    if pal.len() < names.len() || pal[..names.len()] != names[..] {
        tracing::error!("block growth disabled: palette does not match the live state ids");
        return None;
    }
    let extra = pal[names.len()..].iter().map(|s| b.parse_state(s).unwrap_or(0)).collect();
    Some(Growth { dec, n_live: names.len(), extra })
}

/// Live state id of a palette id of the grower decorator.
#[inline]
pub(super) fn live_id(g: &Growth, pal: u16) -> u16 {
    let i = pal as usize;
    if i < g.n_live {
        pal
    } else {
        g.extra[i - g.n_live]
    }
}

fn grower_of(blk: u16) -> Option<&'static Grower> {
    let i = (*t().grower.get(blk as usize)?)?;
    Some(SAPLINGS[i].1)
}

/// `TreeGrower.getRandom`: a weighted pick, no draw for an empty list.
fn pick(list: &'static [(&'static str, i32)], rng: &mut Rng) -> Option<&'static str> {
    let total: i32 = list.iter().map(|e| e.1).sum();
    if list.is_empty() || total <= 0 {
        return None;
    }
    let mut sel = rng.next_int(total);
    for &(name, w) in list {
        sel -= w;
        if sel < 0 {
            return Some(name);
        }
    }
    list.last().map(|e| e.0)
}

/// `TreeGrower.hasFlowers`: a flower in the 5x3x5 box around the sapling.
fn has_flowers<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    let f = &t().flowers;
    for dy in -1..=1 {
        for dz in -2..=2 {
            for dx in -2..=2 {
                if f[block_of(l.get(pos.offset(dx, dy, dz))) as usize] {
                    return true;
                }
            }
        }
    }
    false
}

/// `TreeGrower.findTwoByTwoSaplingPos`: the 2x2 origin offset when the four blocks around `pos` are the
/// same sapling block. Tries offsets (0,0), (0,-1), (-1,0), (-1,-1) in that order.
fn two_by_two<L: Lvl>(l: &L, pos: BlockPos, blk: u16) -> Option<(i32, i32)> {
    for dx in [0, -1] {
        for dz in [0, -1] {
            let ok = [(dx, dz), (dx + 1, dz), (dx, dz + 1), (dx + 1, dz + 1)]
                .iter()
                .all(|&(x, z)| block_of(l.get(pos.offset(x, 0, z))) == blk);
            if ok {
                return Some((dx, dz));
            }
        }
    }
    None
}

/// Box of live blocks copied around `origin` (inclusive bounds), clipped to the nine chunks.
fn box_bounds(origin: BlockPos) -> (i32, i32, i32, i32, i32, i32) {
    let (cx, cz) = (origin.x.div_euclid(16), origin.z.div_euclid(16));
    let (x0, x1) = ((cx - 1) * 16, (cx + 2) * 16 - 1);
    let (z0, z1) = ((cz - 1) * 16, (cz + 2) * 16 - 1);
    let (xa, xb) = ((origin.x - BOX_RADIUS).max(x0), (origin.x + BOX_RADIUS).min(x1));
    let (za, zb) = ((origin.z - BOX_RADIUS).max(z0), (origin.z + BOX_RADIUS).min(z1));
    let ya = (origin.y - BOX_DOWN).max(MIN_Y);
    let yb = (origin.y + BOX_UP).min(MIN_Y + WORLD_HEIGHT - 1);
    (xa, xb, ya, yb, za, zb)
}

#[inline]
fn cell(cx: i32, cz: i32, x: i32, y: i32, z: i32) -> (usize, usize) {
    let c = ((z.div_euclid(16) - cz + 1) * 3 + (x.div_euclid(16) - cx + 1)) as usize;
    let i = ((y - MIN_Y) as usize * 16 + (z & 15) as usize) * 16 + (x & 15) as usize;
    (c, i)
}

/// Nine all-air chunk arrays, reused between growths: a growth used to
/// allocate and fill about 1.8 MB; now it only touches the copied box and
/// the cells the feature wrote, and puts the arrays back clean.
static POOL: std::sync::Mutex<Option<Vec<Vec<u16>>>> = std::sync::Mutex::new(None);

fn take_buffers(air: u16) -> Vec<Vec<u16>> {
    let pooled = POOL.lock().ok().and_then(|mut p| p.take());
    pooled.unwrap_or_else(|| {
        let n = WORLD_HEIGHT as usize * 256;
        (0..9).map(|_| vec![air; n]).collect()
    })
}

fn give_back(blocks: Vec<Vec<u16>>) {
    if let Ok(mut p) = POOL.lock() {
        *p = Some(blocks);
    }
}

/// Live copy of the box around `origin` in the nine-chunk layout of
/// `Region::for_growers`, into all-air arrays. Cells outside the box are air.
fn copy_box<L: Lvl>(l: &L, origin: BlockPos, blocks: &mut [Vec<u16>]) {
    let (cx, cz) = (origin.x.div_euclid(16), origin.z.div_euclid(16));
    let (xa, xb, ya, yb, za, zb) = box_bounds(origin);
    for y in ya..=yb {
        for z in za..=zb {
            for x in xa..=xb {
                let (c, i) = cell(cx, cz, x, y, z);
                blocks[c][i] = l.get(BlockPos { x, y, z });
            }
        }
    }
}

/// Back to all air: the copied box and every cell in `log`.
fn clear_box(origin: BlockPos, blocks: &mut [Vec<u16>], log: &[(i32, i32, i32, u16)], air: u16) {
    let (cx, cz) = (origin.x.div_euclid(16), origin.z.div_euclid(16));
    let (xa, xb, ya, yb, za, zb) = box_bounds(origin);
    for y in ya..=yb {
        for z in za..=zb {
            for x in xa..=xb {
                let (c, i) = cell(cx, cz, x, y, z);
                blocks[c][i] = air;
            }
        }
    }
    for &(x, y, z, _) in log {
        let (c, i) = cell(cx, cz, x, y, z);
        if let Some(b) = blocks.get_mut(c).and_then(|v| v.get_mut(i)) {
            *b = air;
        }
    }
}

/// `TreeGrower.growTree` for the grower `g` of the sapling at `pos` (state `st`).
/// Returns whether the tree was placed.
fn grow_tree<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16, g: &Grower, rng: &mut Rng) -> bool {
    let Some(gr) = growth() else { return false };
    let blk = block_of(st);
    // The mega feature is drawn first, then the 2x2 search.
    if let Some(mega) = pick(g.mega, rng) {
        if let Some((dx, dz)) = two_by_two(&*l, pos, blk) {
            let origin = pos.offset(dx, 0, dz);
            let cells = [pos.offset(dx, 0, dz), pos.offset(dx + 1, 0, dz), pos.offset(dx, 0, dz + 1), pos.offset(dx + 1, 0, dz + 1)];
            let saved = cells.map(|c| l.get(c));
            for c in cells {
                remove_sapling(rs, l, c);
            }
            if place_tree(rs, l, gr, mega, origin, rng) {
                return true;
            }
            for (c, s) in cells.iter().zip(saved) {
                engine::set_block(rs, l, *c, s, fl::NONE, UPDATE_LIMIT);
            }
            return false;
        }
    }
    // A flower in range picks from the flower trees.
    let list = if has_flowers(&*l, pos) && !g.flowers.is_empty() { g.flowers } else { g.trees };
    let Some(feature) = pick(list, rng) else { return false };
    remove_sapling(rs, l, pos);
    if place_tree(rs, l, gr, feature, pos, rng) {
        return true;
    }
    engine::set_block(rs, l, pos, st, fl::NONE, UPDATE_LIMIT);
    false
}

/// `TreeGrower.removeSapling`: `UPDATE_SKIP_ALL_SIDEEFFECTS | UPDATE_CLIENTS` to the (non-water) air.
fn remove_sapling<L: Lvl>(rs: &mut Rs, l: &mut L, p: BlockPos) {
    let flags = fl::KNOWN_SHAPE | fl::SUPPRESS_DROPS | fl::SKIP_BE_SIDE_EFFECTS | fl::SKIP_ON_PLACE | fl::CLIENTS;
    engine::set_block(rs, l, p, t().air, flags, UPDATE_LIMIT);
}

/// `TreeFeature` placement: the writes are replayed with `UPDATE_ALL | UPDATE_KNOWN_SHAPE`.
fn place_tree<L: Lvl>(rs: &mut Rs, l: &mut L, gr: &Growth, feature: &str, origin: BlockPos, rng: &mut Rng) -> bool {
    let Some(index) = gr.dec.grower_index(feature) else { return false };
    place_and_replay(rs, l, gr, index, origin, rng, fl::ALL | fl::KNOWN_SHAPE)
}

/// Places the configured feature `index` (`Decorator::grower_index`) into the live copy and replays its
/// writes on the live world, in order, with `flags` (`Feature.setBlock` writes `UPDATE_ALL`, `TreeFeature`
/// writes `UPDATE_ALL | UPDATE_KNOWN_SHAPE`). Writes that happen before a failed placement are kept, as in vanilla.
pub(super) fn place_and_replay<L: Lvl>(rs: &mut Rs, l: &mut L, gr: &Growth, index: usize, origin: BlockPos, rng: &mut Rng, flags: u32) -> bool {
    let air = t().air;
    let mut blocks = take_buffers(air);
    copy_box(&*l, origin, &mut blocks);
    let (cx, cz) = (origin.x.div_euclid(16), origin.z.div_euclid(16));
    let mut region = gr.dec.grower_region(cx, cz, blocks);
    let mut wrng = WorldgenRng::new(rng.next_i64());
    region.start_log();
    let placed = gr.dec.place_grower_at(index, &mut region, &mut wrng, (origin.x, origin.y, origin.z));
    let log = region.take_log();
    let mut blocks = region.into_blocks();
    clear_box(origin, &mut blocks, &log, air);
    give_back(blocks);
    for (x, y, z, pal) in log {
        engine::set_block(rs, l, BlockPos { x, y, z }, live_id(gr, pal), flags, UPDATE_LIMIT);
    }
    placed
}

/// `SaplingBlock.advanceTree`: stage 0 becomes stage 1 (`UPDATE_NONE`), stage 1 grows a tree.
pub fn advance_tree<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    if geti(st, Pr::Stage) == 0 {
        engine::set_block(rs, l, pos, seti(st, Pr::Stage, 1), fl::NONE, UPDATE_LIMIT);
        return;
    }
    let Some(g) = grower_of(block_of(st)) else { return };
    grow_tree(rs, l, pos, st, g, rng);
}

/// `SaplingBlock.randomTick`: light above at least 9, then a 1 in 7 chance.
pub fn random_tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    if local_brightness(&*l, pos.offset(0, 1, 0)) >= BRIGHTNESS_FOR_SAPLING_GROWTH && rng.next_float() < TICK_CHANCE {
        advance_tree(rs, l, st, pos, rng);
    }
}

/// `VegetationBlock.updateShape`: the sapling breaks when the block below stops supporting vegetation.
pub fn update_shape<L: Lvl>(l: &L, st: u16, pos: BlockPos) -> u16 {
    let below = block_of(l.get(pos.offset(0, -1, 0)));
    if t().supports_vegetation.get(below as usize).copied().unwrap_or(false) {
        st
    } else {
        t().air
    }
}

/// Bone meal on a sapling (`SaplingBlock.isValidBonemealTarget`, `isBonemealSuccess`, `performBonemeal`).
/// Returns false when it is not a valid target.
pub fn bonemeal<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) -> bool {
    let Some(g) = grower_of(block_of(st)) else { return false };
    let Some(gr) = growth() else { return false };
    // `TreeGrower.canGrow`: a feature in the chosen list, or no mega feature, or a 2x2 group.
    let flowers_on = has_flowers(&*l, pos) && !g.flowers.is_empty();
    let feature_ok = flowers_on || !g.trees.is_empty();
    if !(feature_ok || g.mega.is_empty() || two_by_two(&*l, pos, block_of(st)).is_some()) {
        return false;
    }
    // `TreeGrower.getMinimumHeight` (the shortest tree's trunk base height).
    let offset = g.shortest.and_then(|n| gr.dec.grower_base_height(n)).unwrap_or(0);
    let top = pos.y + offset;
    if top < MIN_Y || top >= MIN_Y + WORLD_HEIGHT {
        return false;
    }
    if rng.next_float() < BONEMEAL_SUCCESS {
        advance_tree(rs, l, st, pos, rng);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::testw::World;
    use super::*;

    fn put(w: &mut World, x: i32, y: i32, z: i32, name: &str) {
        w.put(BlockPos { x, y, z }, default_state(name));
    }

    fn count(w: &World, name: &str) -> usize {
        let b = super::super::block_id(name);
        w.cells.values().filter(|&&s| block_of(s) == b).count()
    }

    fn sapling(name: &str) -> u16 {
        // stage 1 sapling, the state that grows
        seti(default_state(name), Pr::Stage, 1)
    }

    #[test]
    fn stage_zero_advances_to_stage_one_without_growing() {
        rine_data::init();
        let mut w = World::default();
        let mut rs = Rs::default();
        let p = BlockPos { x: 0, y: 64, z: 0 };
        w.put(p, default_state("oak_sapling"));
        w.light = 15;
        let mut rng = Rng::new(1);
        advance_tree(&mut rs, &mut w, default_state("oak_sapling"), p, &mut rng);
        assert_eq!(geti(w.get(p), Pr::Stage), 1);
        assert_eq!(count(&w, "oak_log"), 0);
    }

    #[test]
    fn oak_sapling_grows_a_tree() {
        rine_data::init();
        let p = BlockPos { x: 0, y: 64, z: 0 };
        let run = |seed: u64| {
            let mut w = World::default();
            let mut rs = Rs::default();
            w.put(p, sapling("oak_sapling"));
            w.light = 15;
            let mut rng = Rng::new(seed);
            advance_tree(&mut rs, &mut w, sapling("oak_sapling"), p, &mut rng);
            w
        };
        let w = run(7);
        assert!(count(&w, "oak_log") >= 4, "trunk logs: {}", count(&w, "oak_log"));
        assert!(count(&w, "oak_leaves") >= 10, "leaves: {}", count(&w, "oak_leaves"));
        assert_ne!(block_of(w.get(p)), super::super::block_id("oak_sapling"));
        // same seed, same tree (the feature runs on the seeded family rng)
        let a = run(7);
        assert_eq!(w.cells.len(), a.cells.len());
        for (k, v) in &w.cells {
            assert_eq!(a.cells.get(k), Some(v), "cell {k:?}");
        }
        // the pooled arrays come back all air
        let air = t().air;
        let b = take_buffers(air);
        assert!(b.iter().all(|c| c.iter().all(|&x| x == air)));
        give_back(b);
    }

    #[test]
    fn spruce_2x2_saplings_grow_a_mega_tree_or_reset() {
        rine_data::init();
        let mut grew = 0;
        for seed in 0..24u64 {
            let mut w = World::default();
            let mut rs = Rs::default();
            w.light = 15;
            let cells = [(0, 64, 0), (1, 64, 0), (0, 64, 1), (1, 64, 1)];
            for (x, y, z) in cells {
                put(&mut w, x, y, z, "spruce_sapling");
            }
            let st = seti(default_state("spruce_sapling"), Pr::Stage, 1);
            let mut rng = Rng::new(seed);
            advance_tree(&mut rs, &mut w, st, BlockPos { x: 0, y: 64, z: 0 }, &mut rng);
            // either the 2x2 trunk stands, or every sapling is back in place
            let logs = count(&w, "spruce_log");
            let saplings = count(&w, "spruce_sapling");
            assert!(logs == 0 || logs >= 4, "seed {seed}: {logs} logs");
            assert!(saplings == 4 || logs >= 4, "seed {seed}: saplings {saplings}, logs {logs}");
            if logs >= 4 {
                grew += 1;
                assert_eq!(saplings, 0);
            }
        }
        assert!(grew > 0, "no 2x2 spruce tree grew in 24 seeds");
    }

    #[test]
    fn blocked_sapling_stays_a_sapling() {
        rine_data::init();
        let mut w = World::default();
        let mut rs = Rs::default();
        w.light = 15;
        let p = BlockPos { x: 0, y: 64, z: 0 };
        w.put(p, sapling("oak_sapling"));
        // stone where the trunk must grow
        put(&mut w, 0, 65, 0, "stone");
        let before = w.cells.clone();
        let mut rng = Rng::new(3);
        advance_tree(&mut rs, &mut w, sapling("oak_sapling"), p, &mut rng);
        assert_eq!(block_of(w.get(p)), super::super::block_id("oak_sapling"));
        assert_eq!(count(&w, "oak_log"), 0);
        assert_eq!(w.cells, before);
    }

    #[test]
    fn dim_light_does_not_grow() {
        rine_data::init();
        let mut w = World::default();
        let mut rs = Rs::default();
        let p = BlockPos { x: 0, y: 64, z: 0 };
        w.put(p, sapling("oak_sapling"));
        w.light = 8;
        let mut rng = Rng::new(5);
        for _ in 0..200 {
            random_tick(&mut rs, &mut w, sapling("oak_sapling"), p, &mut rng);
        }
        assert_eq!(count(&w, "oak_log"), 0);
    }
}

