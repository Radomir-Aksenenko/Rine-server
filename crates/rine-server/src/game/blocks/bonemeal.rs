//! Bone meal on the `BonemealableBlock` targets that place no configured
//! feature:
//! - `FlowerBedBlock` (pink petals, wildflowers) and `SeagrassBlock`;
//! - `TallFlowerBlock` (sunflower, lilac, rose bush, peony): pops the block item;
//! - `SeaPickleBlock.performBonemeal`: pickles on water above coral blocks;
//! - `TallGrassBlock` (short grass, fern): `DoublePlantBlock.placeAt` of the tall form;
//! - `GlowLichenBlock` (`MultifaceSpreader`): one new face somewhere around;
//! - hanging mangrove propagules: the age cycles (`MangrovePropaguleBlock`).
//!
//! Standing mangrove propagules go through `growth.rs` (`TreeGrower.MANGROVE`).
//! Not ported here: small and big dripleaf (`placeWithRandomHeight` and the tilt
//! states), grass block spread (the biome bone meal features).

use std::sync::OnceLock;

use rine_protocol::BlockPos;

use super::props::{geti, getb, seti, setb, Pr};
use super::{block_id, block_of, block_tag, default_state, is_air, Rng};
use crate::game::redstone::data::{self, DOWN, EAST, NORTH, SOUTH, UP, WEST};
use crate::game::redstone::engine::{self, fl, Lvl, UPDATE_LIMIT};
use crate::game::redstone::Rs;

/// `Direction.values()` order, used by `Direction.stream()`.
const FACES: [u8; 6] = [DOWN, UP, NORTH, SOUTH, WEST, EAST];

struct T {
    /// `FlowerBedBlock` by block id.
    flower_bed: Vec<bool>,
    /// `TallFlowerBlock` by block id.
    tall_flower: Vec<bool>,
    /// `BlockTags.CORAL_BLOCKS` by block id.
    coral: Vec<bool>,
    seagrass: u16,
    water: u16,
    tall_lower: u16,
    tall_upper: u16,
    sea_pickle: u16,
    short_grass: u16,
    fern: u16,
    lichen: u16,
    propagule: u16,
    /// `TallGrassBlock.getGrownBlock` halves: (lower, upper) for short grass and for fern.
    grass_lower: u16,
    grass_upper: u16,
    fern_lower: u16,
    fern_upper: u16,
    /// `BlockTags.SUPPORTS_VEGETATION` by block id.
    supports_vegetation: Vec<bool>,
}

fn t() -> &'static T {
    static T_: OnceLock<T> = OnceLock::new();
    T_.get_or_init(|| {
        let n = rine_data::get().blocks.blocks.len();
        let mut flower_bed = vec![false; n];
        for name in ["pink_petals", "wildflowers"] {
            let id = block_id(name) as usize;
            if id < n {
                flower_bed[id] = true;
            }
        }
        let mut tall_flower = vec![false; n];
        for name in ["sunflower", "lilac", "rose_bush", "peony"] {
            let id = block_id(name) as usize;
            if id < n {
                tall_flower[id] = true;
            }
        }
        let d = &rine_data::get().blocks;
        let half = |name: &str, h: &str| d.parse_state(&format!("minecraft:{name}[half={h}]")).unwrap_or_else(|| default_state(name));
        T {
            flower_bed,
            tall_flower,
            coral: block_tag("coral_blocks"),
            seagrass: block_id("seagrass"),
            water: block_id("water"),
            tall_lower: half("tall_seagrass", "lower"),
            tall_upper: half("tall_seagrass", "upper"),
            sea_pickle: block_id("sea_pickle"),
            short_grass: block_id("short_grass"),
            fern: block_id("fern"),
            lichen: block_id("glow_lichen"),
            propagule: block_id("mangrove_propagule"),
            grass_lower: half("tall_grass", "lower"),
            grass_upper: half("tall_grass", "upper"),
            fern_lower: half("large_fern", "lower"),
            fern_upper: half("large_fern", "upper"),
            supports_vegetation: block_tag("supports_vegetation"),
        }
    })
}

/// Bone meal on a block of this file. `rng` is the level random (`level.getRandom()`).
/// Returns false when the block is not a valid target.
pub fn bonemeal<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) -> bool {
    let t = t();
    let blk = block_of(st);
    let idx = blk as usize;
    if t.flower_bed.get(idx).copied().unwrap_or(false) {
        flower_bed(rs, l, st, pos);
        return true;
    }
    if t.tall_flower.get(idx).copied().unwrap_or(false) {
        // `TallFlowerBlock.performBonemeal`: `popResource(level, pos, new ItemStack(this))`.
        let item = rine_data::get().blocks.state(st).item;
        l.pop_resource(pos, item, 1);
        return true;
    }
    if blk == t.seagrass {
        // `SeagrassBlock.isValidBonemealTarget`: water above
        if block_of(l.get(pos.offset(0, 1, 0))) != t.water {
            return false;
        }
        // `performBonemeal`: tall seagrass, lower then upper, `UPDATE_CLIENTS`
        engine::set_block(rs, l, pos, t.tall_lower, fl::CLIENTS, UPDATE_LIMIT);
        engine::set_block(rs, l, pos.offset(0, 1, 0), t.tall_upper, fl::CLIENTS, UPDATE_LIMIT);
        return true;
    }
    if blk == t.sea_pickle {
        return sea_pickle(rs, l, st, pos, rng);
    }
    if blk == t.short_grass || blk == t.fern {
        return tall_grass(rs, l, blk, pos);
    }
    if blk == t.lichen {
        if !lichen_valid(&*l, st, pos) {
            return false;
        }
        // `GlowLichenBlock.performBonemeal`; `isBonemealSuccess` is always true.
        lichen_spread_random(rs, l, st, pos, rng);
        return true;
    }
    if blk == t.propagule {
        if !getb(st, Pr::Hanging) {
            // `SaplingBlock.isValidBonemealTarget` is true; the success draw and `advanceTree` are in growth.rs.
            super::growth::propagule_bonemeal(rs, l, st, pos, rng);
            return true;
        }
        // `MangrovePropaguleBlock.isValidBonemealTarget`: hanging and not fully grown.
        if geti(st, Pr::Age) == 4 {
            return false;
        }
        // `performBonemeal`: `state.cycle(AGE)`, `UPDATE_CLIENTS`.
        engine::set_block(rs, l, pos, seti(st, Pr::Age, geti(st, Pr::Age) + 1), fl::CLIENTS, UPDATE_LIMIT);
        return true;
    }
    false
}

/// `FlowerBedBlock.performBonemeal`: one more flower up to 4, then the item pops.
fn flower_bed<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    let d = &rine_data::get().blocks;
    let amount = d.get_prop(st, "flower_amount").and_then(|v| v.parse::<i32>().ok()).unwrap_or(1);
    if amount < 4 {
        if let Some(next) = d.with_prop(st, "flower_amount", &(amount + 1).to_string()) {
            engine::set_block(rs, l, pos, next, fl::CLIENTS, UPDATE_LIMIT);
        }
    } else {
        // `popResource(level, pos, new ItemStack(this))`
        l.pop_resource(pos, d.state(st).item, 1);
    }
}

/// `TallGrassBlock`: `isValidBonemealTarget` checks the tall form can survive at `pos`
/// (`DoublePlantBlock` lower half, `VegetationBlock` ground), air above, then `placeAt`.
fn tall_grass<L: Lvl>(rs: &mut Rs, l: &mut L, blk: u16, pos: BlockPos) -> bool {
    let t = t();
    let (lower, upper) = if blk == t.fern { (t.fern_lower, t.fern_upper) } else { (t.grass_lower, t.grass_upper) };
    let below = block_of(l.get(pos.offset(0, -1, 0)));
    let above = pos.offset(0, 1, 0);
    if !t.supports_vegetation.get(below as usize).copied().unwrap_or(false) || !is_air(l.get(above)) || !(-64..320).contains(&above.y) {
        return false;
    }
    // `DoublePlantBlock.placeAt`: lower, then upper, both `UPDATE_CLIENTS`.
    engine::set_block(rs, l, pos, lower, fl::CLIENTS, UPDATE_LIMIT);
    engine::set_block(rs, l, above, upper, fl::CLIENTS, UPDATE_LIMIT);
    true
}

/// `SeaPickleBlock.isValidBonemealTarget` and `performBonemeal`. The loop order and the
/// random draws follow vanilla: `nextInt(6)` per water candidate, `nextInt(4) + 1` per pickle.
fn sea_pickle<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) -> bool {
    let t = t();
    // `isDead` is `!WATERLOGGED`; the target also needs coral below.
    if !getb(st, Pr::Waterlogged) || !t.coral.get(block_of(l.get(pos.offset(0, -1, 0))) as usize).copied().unwrap_or(false) {
        return false;
    }
    let mut z_span = 1;
    let mut z_offset = 0;
    let mut count = 0;
    let x_start = pos.x - 2;
    for x in 0..5 {
        for z in 0..z_span {
            // startY in [pos.y - 1, pos.y + 1)
            for dy in [-1, 0] {
                let p = BlockPos { x: x_start + x, y: pos.y + dy, z: pos.z - z_offset + z };
                if p != pos && rng.next_int(6) == 0 && block_of(l.get(p)) == t.water {
                    let below = block_of(l.get(p.offset(0, -1, 0)));
                    if t.coral.get(below as usize).copied().unwrap_or(false) {
                        let pickles = rng.next_int(4) + 1;
                        let placed = seti(setb(default_state("sea_pickle"), Pr::Waterlogged, true), Pr::Pickles, pickles);
                        engine::set_block(rs, l, p, placed, fl::ALL, UPDATE_LIMIT);
                    }
                }
            }
        }
        if count < 2 {
            z_span += 2;
            z_offset += 1;
        } else {
            z_span -= 2;
            z_offset -= 1;
        }
        count += 1;
    }
    engine::set_block(rs, l, pos, seti(st, Pr::Pickles, 4), fl::CLIENTS, UPDATE_LIMIT);
    true
}

// ------------------------------------------------------------------ glow lichen

/// A `MultifaceSpreader.SpreadPos`: the position and the face it lands on (the source is not needed here).
#[derive(Clone, Copy)]
struct SpreadPos {
    pos: BlockPos,
    face: u8,
}

/// `MultifaceBlock.getFaceProperty`.
fn face_pr(d: u8) -> Pr {
    match d {
        DOWN => Pr::Down,
        UP => Pr::Up,
        NORTH => Pr::North,
        SOUTH => Pr::South,
        WEST => Pr::West,
        _ => Pr::East,
    }
}

fn has_face(st: u16, d: u8) -> bool {
    getb(st, face_pr(d))
}

fn rel(p: BlockPos, d: u8) -> BlockPos {
    engine::rel(p, d)
}

/// `MultifaceBlock.canAttachTo`: the neighbour's face toward the lichen is full. `data::sturdy`
/// stands in for `isFaceFull` of the support or collision shape (as in the vine code).
fn attach_ok<L: Lvl>(l: &L, face: u8, neighbour: BlockPos) -> bool {
    data::sturdy(l.get(neighbour), data::opposite(face))
}

/// `MultifaceBlock.isValidStateForPlacement` for the lichen block.
fn valid_for_placement<L: Lvl>(l: &L, old: u16, pos: BlockPos, face: u8) -> bool {
    let is_lichen = block_of(old) == t().lichen;
    (!is_lichen || !has_face(old, face)) && attach_ok(l, face, rel(pos, face))
}

/// `DefaultSpreaderConfig.canSpreadInto`: air, lichen, or a water source, and a valid face.
fn can_spread_into<L: Lvl>(l: &L, sp: SpreadPos) -> bool {
    let existing = l.get(sp.pos);
    let replaceable = is_air(existing)
        || block_of(existing) == t().lichen
        || (block_of(existing) == t().water && geti(existing, Pr::Level) == 0);
    replaceable && valid_for_placement(l, existing, sp.pos, sp.face)
}

/// `MultifaceSpreader.getSpreadFromFaceTowardDirection` for the lichen config (`isOtherBlockValidAsSource`
/// is false): the three `SpreadType`s in order, the first one `canSpreadInto` accepts.
fn spread_toward<L: Lvl>(l: &L, st: u16, pos: BlockPos, from: u8, dir: u8) -> Option<SpreadPos> {
    if dir == from || dir == data::opposite(from) {
        return None;
    }
    if !(has_face(st, from) && !has_face(st, dir)) {
        return None;
    }
    // SAME_POSITION, SAME_PLANE, WRAP_AROUND
    let candidates = [
        SpreadPos { pos, face: dir },
        SpreadPos { pos: rel(pos, dir), face: from },
        SpreadPos { pos: rel(rel(pos, dir), from), face: data::opposite(dir) },
    ];
    candidates.into_iter().find(|&c| can_spread_into(l, c))
}

/// `MultifaceSpreader.canSpreadInAnyDirection(state, level, pos, startingFace)`.
fn can_spread_any<L: Lvl>(l: &L, st: u16, pos: BlockPos, from: u8) -> bool {
    FACES.iter().any(|&dir| spread_toward(l, st, pos, from, dir).is_some())
}

/// `GlowLichenBlock.isValidBonemealTarget`.
fn lichen_valid<L: Lvl>(l: &L, st: u16, pos: BlockPos) -> bool {
    FACES.iter().any(|&face| can_spread_any(l, st, pos, data::opposite(face)))
}

/// `Util.shuffle` over a copy of `Direction.values()` (`Direction.allShuffled`).
fn shuffled_faces(rng: &mut Rng) -> [u8; 6] {
    let mut v = FACES;
    for i in (2..=6usize).rev() {
        let j = rng.next_int(i as i32) as usize;
        v.swap(i - 1, j);
    }
    v
}

/// `MultifaceSpreader.spreadFromRandomFaceTowardRandomDirection`: shuffled faces the state has,
/// then shuffled directions, stopping at the first successful placement.
fn lichen_spread_random<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    for face in shuffled_faces(rng) {
        if !has_face(st, face) {
            continue;
        }
        for dir in shuffled_faces(rng) {
            let Some(sp) = spread_toward(&*l, st, pos, face, dir) else { continue };
            // `MultifaceSpreader.placeBlock` with `UPDATE_CLIENTS`, through the spread event.
            let old = l.get(sp.pos);
            let base = if block_of(old) == t().lichen {
                old
            } else if block_of(old) == t().water {
                setb(default_state("glow_lichen"), Pr::Waterlogged, true)
            } else {
                default_state("glow_lichen")
            };
            let new = setb(base, face_pr(sp.face), true);
            engine::set_block(rs, l, sp.pos, new, fl::CLIENTS, UPDATE_LIMIT);
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::testw::World;
    use super::*;

    fn at() -> BlockPos {
        BlockPos { x: 0, y: 64, z: 0 }
    }

    #[test]
    fn flower_bed_gains_a_flower_then_pops_one() {
        rine_data::init();
        let d = &rine_data::get().blocks;
        let mut rs = Rs::default();
        let mut w = World::default();
        let mut rng = Rng::new(1);
        let p = at();
        let one = d.parse_state("minecraft:pink_petals[facing=north,flower_amount=1]").unwrap();
        w.put(p, one);
        assert!(bonemeal(&mut rs, &mut w, one, p, &mut rng));
        assert_eq!(d.get_prop(w.get(p), "flower_amount"), Some("2"));
        assert!(w.popped.is_empty());

        let four = d.parse_state("minecraft:pink_petals[facing=north,flower_amount=4]").unwrap();
        w.put(p, four);
        assert!(bonemeal(&mut rs, &mut w, four, p, &mut rng));
        assert_eq!(w.get(p), four, "a full bed stays as it is");
        assert_eq!(w.popped.len(), 1);
        assert_eq!(w.popped[0].0, p);
    }

    #[test]
    fn seagrass_needs_water_above_and_becomes_tall_seagrass() {
        rine_data::init();
        let mut rs = Rs::default();
        let mut w = World::default();
        let mut rng = Rng::new(1);
        let p = at();
        let sg = default_state("seagrass");
        w.put(p, sg);
        assert!(!bonemeal(&mut rs, &mut w, sg, p, &mut rng), "no water above");
        assert_eq!(w.get(p), sg);

        w.put(p.offset(0, 1, 0), default_state("water"));
        assert!(bonemeal(&mut rs, &mut w, sg, p, &mut rng));
        assert_eq!(w.get(p), t().tall_lower);
        assert_eq!(w.get(p.offset(0, 1, 0)), t().tall_upper);
    }

    #[test]
    fn stone_is_not_a_target() {
        rine_data::init();
        let mut rs = Rs::default();
        let mut w = World::default();
        let mut rng = Rng::new(1);
        let p = at();
        let stone = default_state("stone");
        w.put(p, stone);
        assert!(!bonemeal(&mut rs, &mut w, stone, p, &mut rng));
    }

    #[test]
    fn tall_flowers_pop_their_block_item_and_stay() {
        rine_data::init();
        for name in ["sunflower", "lilac", "rose_bush", "peony"] {
            let mut rs = Rs::default();
            let mut w = World::default();
            let mut rng = Rng::new(2);
            let p = at();
            let st = default_state(name);
            w.put(p, st);
            assert!(bonemeal(&mut rs, &mut w, st, p, &mut rng), "{name}");
            assert_eq!(w.get(p), st, "{name} keeps its state");
            assert_eq!(w.popped.len(), 1, "{name}");
            assert_eq!(w.popped[0].1, rine_data::get().blocks.state(st).item, "{name}");
        }
    }

    #[test]
    fn short_grass_on_dirt_becomes_tall_grass_and_fern_becomes_large_fern() {
        rine_data::init();
        let p = at();
        let mut rs = Rs::default();
        let mut w = World::default();
        let mut rng = Rng::new(3);
        w.put(p.offset(0, -1, 0), default_state("dirt"));
        let grass = default_state("short_grass");
        w.put(p, grass);
        assert!(bonemeal(&mut rs, &mut w, grass, p, &mut rng));
        assert_eq!(w.get(p), t().grass_lower);
        assert_eq!(w.get(p.offset(0, 1, 0)), t().grass_upper);

        let mut w = World::default();
        w.put(p.offset(0, -1, 0), default_state("dirt"));
        let fern = default_state("fern");
        w.put(p, fern);
        assert!(bonemeal(&mut rs, &mut w, fern, p, &mut rng));
        assert_eq!(w.get(p), t().fern_lower);
        assert_eq!(w.get(p.offset(0, 1, 0)), t().fern_upper);
    }

    #[test]
    fn tall_grass_needs_air_above_and_ground_that_supports_vegetation() {
        rine_data::init();
        let p = at();
        let mut rs = Rs::default();
        let grass = default_state("short_grass");

        // blocked above
        let mut w = World::default();
        w.put(p.offset(0, -1, 0), default_state("dirt"));
        w.put(p, grass);
        w.put(p.offset(0, 1, 0), default_state("stone"));
        let mut rng = Rng::new(4);
        assert!(!bonemeal(&mut rs, &mut w, grass, p, &mut rng));
        assert_eq!(w.get(p), grass);

        // stone below: not a supporting block for vegetation
        let mut w = World::default();
        w.put(p.offset(0, -1, 0), default_state("stone"));
        w.put(p, grass);
        assert!(!bonemeal(&mut rs, &mut w, grass, p, &mut rng));
        assert_eq!(w.get(p), grass);
    }

    #[test]
    fn sea_pickle_only_on_coral_and_only_in_water() {
        rine_data::init();
        let p = at();
        let pickle = seti(setb(default_state("sea_pickle"), Pr::Waterlogged, true), Pr::Pickles, 1);
        let mut rs = Rs::default();

        // dead pickle (not waterlogged): not a target
        let mut w = World::default();
        w.put(p.offset(0, -1, 0), default_state("brain_coral_block"));
        let dead = setb(pickle, Pr::Waterlogged, false);
        w.put(p, dead);
        let mut rng = Rng::new(5);
        assert!(!bonemeal(&mut rs, &mut w, dead, p, &mut rng));

        // waterlogged on sand: not a target
        let mut w = World::default();
        w.put(p.offset(0, -1, 0), default_state("sand"));
        w.put(p, pickle);
        assert!(!bonemeal(&mut rs, &mut w, pickle, p, &mut rng));
        assert_eq!(w.get(p), pickle);
    }

    #[test]
    fn sea_pickle_bonemeal_fills_water_over_coral_in_the_diamond() {
        rine_data::init();
        let p = at();
        let pickle = seti(setb(default_state("sea_pickle"), Pr::Waterlogged, true), Pr::Pickles, 1);
        let coral = default_state("brain_coral_block");
        let water = default_state("water");
        let mut placed_any = false;
        for seed in 0..16u64 {
            let mut rs = Rs::default();
            let mut w = World::default();
            let mut rng = Rng::new(seed);
            // coral floor under a water layer, and a second layer of water above the pickle
            for x in -3..=3 {
                for z in -3..=3 {
                    w.put(BlockPos { x, y: 63, z }, coral);
                    w.put(BlockPos { x, y: 64, z }, water);
                    w.put(BlockPos { x, y: 65, z }, water);
                }
            }
            w.put(p, pickle);
            // no coral under the x = +1 column: nothing may appear there
            for z in -3..=3 {
                w.put(BlockPos { x: 1, y: 63, z }, default_state("sand"));
            }
            assert!(bonemeal(&mut rs, &mut w, pickle, p, &mut rng));
            assert_eq!(geti(w.get(p), Pr::Pickles), 4, "pos becomes 4 pickles");
            for (&q, &s) in &w.cells {
                if q == p || block_of(s) != block_id("sea_pickle") {
                    continue;
                }
                placed_any = true;
                assert_eq!(q.y, 64, "only in the layer above the coral");
                let (dx, dz) = (q.x - p.x, q.z - p.z);
                assert!(dx != 1, "no coral under x = +1");
                assert!(dx.abs() <= 2 && dz.abs() <= 2, "inside the box");
                // diamond: x offset 0 or 4 allows only z offset 0 (relative to the 5 wide box)
                let zmax = match dx {
                    -2 | 2 => 0,
                    -1 | 1 => 1,
                    _ => 2,
                };
                assert!(dz.abs() <= zmax, "diamond at ({dx},{dz})");
                assert!((1..=4).contains(&geti(s, Pr::Pickles)));
            }
        }
        assert!(placed_any, "some pickles grow in 16 seeds");
    }

    #[test]
    fn hanging_propagule_ages_until_four_and_standing_one_is_not_here() {
        rine_data::init();
        let p = at();
        let mut rs = Rs::default();
        let mut rng = Rng::new(6);
        let mut w = World::default();
        let st = setb(seti(default_state("mangrove_propagule"), Pr::Age, 2), Pr::Hanging, true);
        w.put(p, st);
        assert!(bonemeal(&mut rs, &mut w, st, p, &mut rng));
        assert_eq!(geti(w.get(p), Pr::Age), 3);
        let full = seti(st, Pr::Age, 4);
        w.put(p, full);
        assert!(!bonemeal(&mut rs, &mut w, full, p, &mut rng));
        assert_eq!(w.get(p), full);
    }

    fn lichen_at(face: u8) -> u16 {
        setb(default_state("glow_lichen"), face_pr(face), true)
    }

    #[test]
    fn glow_lichen_wraps_around_a_stone_corner() {
        rine_data::init();
        let p = at();
        let stone = default_state("stone");
        let lichen = lichen_at(NORTH);
        let mut hit = false;
        for seed in 0..64u64 {
            let mut rs = Rs::default();
            let mut w = World::default();
            let mut rng = Rng::new(seed);
            w.put(p.offset(0, 0, -1), stone);
            w.put(p, lichen);
            assert!(bonemeal(&mut rs, &mut w, lichen, p, &mut rng));
            // the spread may land on the corner: (1, 64, -1) with the WEST face on the stone
            if block_of(w.get(BlockPos { x: 1, y: 64, z: -1 })) == t().lichen {
                assert!(has_face(w.get(BlockPos { x: 1, y: 64, z: -1 }), WEST));
                hit = true;
            }
        }
        assert!(hit, "the wrap-around spread happens");
    }

    #[test]
    fn glow_lichen_only_grows_faces_that_are_attached() {
        rine_data::init();
        let p = at();
        let stone = default_state("stone");
        for seed in 0..32u64 {
            let mut rs = Rs::default();
            let mut w = World::default();
            let mut rng = Rng::new(seed);
            // stone on the north and the floor, lichen on both faces
            w.put(p.offset(0, 0, -1), stone);
            w.put(p.offset(0, -1, 0), stone);
            w.put(p, setb(lichen_at(NORTH), Pr::Down, true));
            for _ in 0..3 {
                let st = w.get(p);
                if lichen_valid(&w, st, p) {
                    bonemeal(&mut rs, &mut w, st, p, &mut rng);
                }
            }
            for (&q, &s) in &w.cells {
                if block_of(s) != t().lichen {
                    continue;
                }
                let attached = FACES.iter().any(|&f| has_face(s, f) && data::sturdy(w.get(rel(q, f)), data::opposite(f)));
                assert!(attached, "seed {seed}: lichen at {q:?} has no supporting face");
            }
        }
    }

    #[test]
    fn glow_lichen_on_water_is_waterlogged_and_full_water_only_as_source() {
        rine_data::init();
        let p = at();
        let stone = default_state("stone");
        let mut rs = Rs::default();
        let mut w = World::default();
        w.put(p.offset(0, 0, -1), stone);
        w.put(p, lichen_at(NORTH));
        // a flowing water (level 3) is not a source: not replaceable
        let flowing = seti(default_state("water"), Pr::Level, 3);
        w.put(BlockPos { x: 1, y: 64, z: -1 }, flowing);
        assert!(!can_spread_into(&w, SpreadPos { pos: BlockPos { x: 1, y: 64, z: -1 }, face: WEST }));
        // a source is: the new lichen is waterlogged
        w.put(BlockPos { x: 1, y: 64, z: -1 }, default_state("water"));
        let sp = SpreadPos { pos: BlockPos { x: 1, y: 64, z: -1 }, face: WEST };
        assert!(can_spread_into(&w, sp));
        let mut hit = false;
        for seed in 0..64u64 {
            let mut w = World::default();
            let mut rng2 = Rng::new(seed);
            w.put(p.offset(0, 0, -1), stone);
            w.put(p, lichen_at(NORTH));
            w.put(BlockPos { x: 1, y: 64, z: -1 }, default_state("water"));
            let st = w.get(p);
            lichen_spread_random(&mut rs, &mut w, st, p, &mut rng2);
            let q = BlockPos { x: 1, y: 64, z: -1 };
            if block_of(w.get(q)) == t().lichen {
                assert!(getb(w.get(q), Pr::Waterlogged));
                hit = true;
            }
        }
        assert!(hit, "a water source takes the lichen");
    }
}
