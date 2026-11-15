//! Connection and shape properties of blocks that look at their neighbours:
//! stairs (`StairBlock`), fences (`FenceBlock`), panes and bars
//! (`IronBarsBlock`), walls (`WallBlock`), fence gates (`FenceGateBlock`,
//! `in_wall`), doors (`DoorBlock.getHinge` at placement), chorus plants
//! (`ChorusPlantBlock`) and huge mushroom blocks (`HugeMushroomBlock`).
//!
//! Hooks: `placement` is `getStateForPlacement` (read-only; it runs before the
//! block is set, so `l.get(pos)` is the replaced block, used for waterlogging),
//! `update_shape` is `BlockState.updateShape`, `tick` is the chorus survival tick.
//!
//! Vanilla `updateShape` schedules a water tick for waterlogged blocks. Not done
//! here: Rine fluids handle water updates.
//!
//! Tag membership (`tags.json`) is the class membership: the name suffixes used
//! by `classify` match the tags exactly for the 26.3 block list.

use super::props::{getb, getv, setb, setv, vid, Pr};
use super::{beh, block_id, block_of, block_tag, Beh, PlaceCtx};
use crate::game::redstone::data::{self, axis, clockwise, counter_clockwise, opposite, step, DOWN, EAST, HORIZONTAL, NORTH, SOUTH, UP, WEST};
use crate::game::redstone::engine::{self, prio, Lvl, UPDATE_LIMIT};
use crate::game::redstone::Rs;
use rine_data::blocks::flag;
use rine_protocol::BlockPos;
use std::sync::OnceLock;

/// Claim a block of this family by name.
pub fn classify(n: &str) -> Option<Beh> {
    Some(match n {
        "chorus_plant" => Beh::Chorus,
        "brown_mushroom_block" | "red_mushroom_block" | "mushroom_stem" => Beh::HugeMushroom,
        "glass_pane" => Beh::Bars,
        _ if n.ends_with("_stairs") => Beh::Stairs,
        _ if n.ends_with("_fence_gate") => Beh::FenceGate,
        _ if n.ends_with("_fence") => Beh::Fence,
        _ if n.ends_with("_wall") => Beh::Wall,
        _ if n.ends_with("_door") => Beh::Door,
        _ if n.ends_with("bars") || n.ends_with("stained_glass_pane") => Beh::Bars,
        _ => return None,
    })
}

struct T {
    walls: Vec<bool>,
    fences: Vec<bool>,
    wooden_fences: Vec<bool>,
    fence_gates: Vec<bool>,
    bars: Vec<bool>,
    stairs: Vec<bool>,
    doors: Vec<bool>,
    /// `Block.isExceptionForConnection`.
    exception: Vec<bool>,
    /// `BlockTags.WALL_POST_OVERRIDE`.
    post_override: Vec<bool>,
    /// `BlockTags.SUPPORTS_CHORUS_PLANT`.
    supports_chorus: Vec<bool>,
    /// Per collision shape: bit 0 = wall post covered, bit `1 + i` = arm `i`
    /// (north, east, south, west) covered by the block above.
    cover: Vec<u8>,
    chorus_flower: u16,
    v_straight: u8,
    v_inner_left: u8,
    v_inner_right: u8,
    v_outer_left: u8,
    v_outer_right: u8,
    v_lower: u8,
    v_left: u8,
    v_right: u8,
    v_none: u8,
    v_low: u8,
    v_tall: u8,
}

fn t() -> &'static T {
    static T_: OnceLock<T> = OnceLock::new();
    T_.get_or_init(build)
}

fn build() -> T {
    let d = rine_data::get();
    let mut bars = block_tag("bars");
    let mut exception = block_tag("leaves");
    let shulker = block_tag("shulker_boxes");
    for (i, b) in d.blocks.blocks.iter().enumerate() {
        let n = b.name.strip_prefix("minecraft:").unwrap_or(&b.name);
        // `IronBarsBlock` subclasses that the tag does not list
        if n == "glass_pane" || n.ends_with("stained_glass_pane") {
            bars[i] = true;
        }
        // `Block.isExceptionForConnection`: leaves, shulker boxes and the named blocks
        if shulker[i] || matches!(n, "barrier" | "carved_pumpkin" | "jack_o_lantern" | "melon" | "pumpkin") {
            exception[i] = true;
        }
    }
    T {
        walls: block_tag("walls"),
        fences: block_tag("fences"),
        wooden_fences: block_tag("wooden_fences"),
        fence_gates: block_tag("fence_gates"),
        bars,
        stairs: block_tag("stairs"),
        doors: block_tag("doors"),
        exception,
        post_override: block_tag("wall_post_override"),
        supports_chorus: block_tag("supports_chorus_plant"),
        cover: d.blocks.shapes.iter().map(|boxes| cover_mask(boxes)).collect(),
        chorus_flower: block_id("chorus_flower"),
        v_straight: vid("straight"),
        v_inner_left: vid("inner_left"),
        v_inner_right: vid("inner_right"),
        v_outer_left: vid("outer_left"),
        v_outer_right: vid("outer_right"),
        v_lower: vid("lower"),
        v_left: vid("left"),
        v_right: vid("right"),
        v_none: vid("none"),
        v_low: vid("low"),
        v_tall: vid("tall"),
    }
}

// ------------------------------------------------------------ wall test shapes

/// `TEST_SHAPE_POST` footprint in 1/16 units: x0, x1, z0, z1.
const POST_RECT: [f32; 4] = [7.0, 9.0, 7.0, 9.0];
/// `TEST_SHAPES_WALL` footprints (north, east, south, west). All are full height.
const ARM_RECTS: [[f32; 4]; 4] = [[7.0, 9.0, 0.0, 9.0], [7.0, 16.0, 7.0, 9.0], [7.0, 9.0, 7.0, 16.0], [0.0, 9.0, 7.0, 9.0]];

/// `VoxelShape.getFaceShape(DOWN)` of a collision shape (`boxes`, block units
/// 0..1) and the `isCovered` test of the four wall arms and the post.
/// A shape whose boxes all span the full height gives its footprint; any
/// other shape gives a one-voxel slice at y=0, which never contains a
/// full-height test box. Shape edges sit on the 1/16 grid, so testing the
/// centre of each 1/16 cell is exact.
fn cover_mask(boxes: &[[f32; 6]]) -> u8 {
    if boxes.is_empty() || boxes.iter().any(|b| b[1] != 0.0 || b[4] != 1.0) {
        return 0;
    }
    let mut m = 0;
    if covered(boxes, POST_RECT) {
        m |= 1;
    }
    for (i, r) in ARM_RECTS.iter().enumerate() {
        if covered(boxes, *r) {
            m |= arm_bit(i);
        }
    }
    m
}

fn covered(boxes: &[[f32; 6]], r: [f32; 4]) -> bool {
    for x in (r[0] as i32)..(r[1] as i32) {
        for z in (r[2] as i32)..(r[3] as i32) {
            let cx = (x as f32 + 0.5) / 16.0;
            let cz = (z as f32 + 0.5) / 16.0;
            if !boxes.iter().any(|b| cx >= b[0] && cx <= b[3] && cz >= b[2] && cz <= b[5]) {
                return false;
            }
        }
    }
    true
}

#[inline]
fn arm_bit(i: usize) -> u8 {
    1 << (i + 1)
}

/// Cover mask of the block at a state's collision shape.
#[inline]
fn cover_of(t: &T, st: u16) -> u8 {
    t.cover[rine_data::get().blocks.states[st as usize].shape as usize]
}

// ------------------------------------------------------------ helpers

/// `replacedFluidState.is(Fluids.WATER)` at `p` (the block being replaced).
#[inline]
fn is_source_water<L: Lvl>(l: &L, p: BlockPos) -> bool {
    let b = &rine_data::get().blocks;
    let st = l.get(p);
    // `replacedFluidState.is(Fluids.WATER)`: the source, not flowing water
    b.has(st, flag::WATER) && b.has(st, flag::FLUID_SOURCE)
}

/// `FenceGateBlock.connectsToDirection`.
#[inline]
fn gate_faces(st: u16, dir: u8) -> bool {
    axis(data::get(st, data::P::Facing)) == axis(clockwise(dir))
}

/// `FenceBlock.connectsTo` for the neighbour `n` seen from the block through
/// `back` (the direction from the neighbour back to the block).
#[inline]
fn fence_conn(t: &T, n: u16, back: u8, self_wooden: bool) -> bool {
    let b = block_of(n) as usize;
    let same = t.fences[b] && t.wooden_fences[b] == self_wooden;
    let gate = t.fence_gates[b] && gate_faces(n, back);
    (!t.exception[b] && data::sturdy(n, back)) || same || gate
}

/// `IronBarsBlock.attachsTo`.
#[inline]
fn bars_conn(t: &T, n: u16, back: u8) -> bool {
    let b = block_of(n) as usize;
    (!t.exception[b] && data::sturdy(n, back)) || t.bars[b] || t.walls[b]
}

/// `WallBlock.connectsTo`.
#[inline]
fn wall_conn(t: &T, n: u16, back: u8) -> bool {
    let b = block_of(n) as usize;
    let gate = t.fence_gates[b] && gate_faces(n, back);
    t.walls[b] || (!t.exception[b] && data::sturdy(n, back)) || t.bars[b] || gate
}

/// Side properties in `HORIZONTAL` order (north, east, south, west).
const SIDE_PR: [Pr; 4] = [Pr::North, Pr::East, Pr::South, Pr::West];

/// Boolean property of a direction (`PipeBlock.PROPERTY_BY_DIRECTION`).
#[inline]
fn dir_pr(d: u8) -> Pr {
    match d {
        DOWN => Pr::Down,
        UP => Pr::Up,
        NORTH => Pr::North,
        SOUTH => Pr::South,
        WEST => Pr::West,
        EAST => Pr::East,
        _ => Pr::Down,
    }
}

#[inline]
fn above(p: BlockPos) -> BlockPos {
    p.offset(0, 1, 0)
}

#[inline]
fn below(p: BlockPos) -> BlockPos {
    p.offset(0, -1, 0)
}

// ------------------------------------------------------------ stairs

/// `StairBlock.canTakeShape`.
fn can_take_shape<L: Lvl>(t: &T, l: &L, st: u16, pos: BlockPos, dir: u8) -> bool {
    let n = l.get(pos.relative(dir));
    !t.stairs[block_of(n) as usize] || data::get(n, data::P::Facing) != data::get(st, data::P::Facing) || getv(n, Pr::Half) != getv(st, Pr::Half)
}

/// `StairBlock.getStairsShape`: the shape value id.
fn stairs_shape<L: Lvl>(t: &T, l: &L, st: u16, pos: BlockPos) -> u8 {
    let facing = data::get(st, data::P::Facing);
    let half = getv(st, Pr::Half);
    let behind = l.get(pos.relative(facing));
    if t.stairs[block_of(behind) as usize] && half == getv(behind, Pr::Half) {
        let bf = data::get(behind, data::P::Facing);
        if axis(bf) != axis(facing) && can_take_shape(t, l, st, pos, opposite(bf)) {
            return if bf == counter_clockwise(facing) { t.v_outer_left } else { t.v_outer_right };
        }
    }
    let front = l.get(pos.relative(opposite(facing)));
    if t.stairs[block_of(front) as usize] && half == getv(front, Pr::Half) {
        let ff = data::get(front, data::P::Facing);
        if axis(ff) != axis(facing) && can_take_shape(t, l, st, pos, ff) {
            return if ff == counter_clockwise(facing) { t.v_inner_left } else { t.v_inner_right };
        }
    }
    t.v_straight
}

// ------------------------------------------------------------ fences, bars, walls

/// Fence or pane sides of the block at `pos` (placement).
fn sides_placed<L: Lvl>(l: &L, pos: BlockPos, st: u16, conn: impl Fn(&L, u16, u8) -> bool) -> u16 {
    let mut s = st;
    for (i, &d) in HORIZONTAL.iter().enumerate() {
        let n = l.get(pos.relative(d));
        s = setb(s, SIDE_PR[i], conn(l, n, opposite(d)));
    }
    s
}

/// Wall arms and post from the connections (`WallBlock.updateShape`, `updateSides`,
/// `shouldRaisePost`). `top` is the block above.
fn wall_with(t: &T, st: u16, conn: [bool; 4], top: u16) -> u16 {
    let cover = cover_of(t, top);
    let mut s = st;
    let mut none = [false; 4];
    let mut tall = [false; 4];
    for i in 0..4 {
        let (v, n, tl) = if !conn[i] {
            (t.v_none, true, false)
        } else if cover & arm_bit(i) != 0 {
            (t.v_tall, false, true)
        } else {
            (t.v_low, false, false)
        };
        s = setv(s, SIDE_PR[i], v);
        none[i] = n;
        tall[i] = tl;
    }
    // `shouldRaisePost`
    let up = if t.walls[block_of(top) as usize] && getb(top, Pr::Up) {
        true
    } else if (none[0] && none[1] && none[2] && none[3]) || (none[0] != none[2]) || (none[3] != none[1]) {
        // corner, end or lone wall
        true
    } else {
        let high = (tall[0] && tall[2]) || (tall[1] && tall[3]);
        !high && (t.post_override[block_of(top) as usize] || cover & 1 != 0)
    };
    setb(s, Pr::Up, up)
}

/// Wall sides as they stand (connected means the side is not `none`).
#[inline]
fn wall_side_connected(t: &T, st: u16, i: usize) -> bool {
    getv(st, SIDE_PR[i]) != t.v_none
}

// ------------------------------------------------------------ doors

/// `DoorBlock.getHinge`. The click location is the hit point on the clicked
/// block (`clicked + cursor`), relative to the placed block.
fn door_hinge<L: Lvl>(t: &T, l: &L, pos: BlockPos, place_dir: u8, ctx: &PlaceCtx) -> u8 {
    let up = above(pos);
    let left_dir = counter_clockwise(place_dir);
    let right_dir = clockwise(place_dir);
    let left = l.get(pos.relative(left_dir));
    let left_above = l.get(up.relative(left_dir));
    let right = l.get(pos.relative(right_dir));
    let right_above = l.get(up.relative(right_dir));
    let full = |s: u16| data::full_block(s) as i32;
    let balance = -full(left) - full(left_above) + full(right) + full(right_above);
    let door_left = t.doors[block_of(left) as usize] && getv(left, Pr::Half) == t.v_lower;
    let door_right = t.doors[block_of(right) as usize] && getv(right, Pr::Half) == t.v_lower;
    let right_hinge = if (!door_left || door_right) && balance <= 0 {
        if (!door_right || door_left) && balance >= 0 {
            let (sx, _, sz) = step(place_dir);
            let cx = (ctx.clicked.x - pos.x) as f64 + ctx.cursor.0 as f64;
            let cz = (ctx.clicked.z - pos.z) as f64 + ctx.cursor.2 as f64;
            // vanilla returns LEFT when this condition holds, RIGHT otherwise
            !((sx >= 0 || !(cz < 0.5)) && (sx <= 0 || !(cz > 0.5)) && (sz >= 0 || !(cx > 0.5)) && (sz <= 0 || !(cx < 0.5)))
        } else {
            false
        }
    } else {
        true
    };
    if right_hinge {
        t.v_right
    } else {
        t.v_left
    }
}

// ------------------------------------------------------------ fence gates

/// `FenceGateBlock.getStateForPlacement` `inWall`: a wall on either side,
/// perpendicular to the facing.
fn gate_in_wall<L: Lvl>(t: &T, l: &L, pos: BlockPos, facing: u8) -> bool {
    let wall = |d: u8| t.walls[block_of(l.get(pos.relative(d))) as usize];
    if axis(facing) == axis(SOUTH) {
        wall(4) || wall(5)
    } else {
        wall(NORTH) || wall(SOUTH)
    }
}

// ------------------------------------------------------------ chorus

/// `ChorusPlantBlock.canSurvive`.
fn chorus_survives<L: Lvl>(t: &T, l: &L, pos: BlockPos, blk: u16) -> bool {
    let b = l.get(below(pos));
    let block_above_or_below = !super::is_air(l.get(above(pos))) && !super::is_air(b);
    for d in HORIZONTAL {
        let np = pos.relative(d);
        if block_of(l.get(np)) == blk {
            if block_above_or_below {
                return false;
            }
            let bb = l.get(below(np));
            if block_of(bb) == blk || t.supports_chorus[block_of(bb) as usize] {
                return true;
            }
        }
    }
    block_of(b) == blk || t.supports_chorus[block_of(b) as usize]
}

// ------------------------------------------------------------ dispatch

/// `getStateForPlacement` of this family's blocks. Always Some: the shape
/// rules never refuse a placement.
pub fn placement<L: Lvl>(l: &L, st: u16, pos: BlockPos, ctx: &PlaceCtx) -> Option<u16> {
    let t = t();
    let out = match beh(st) {
        Beh::Stairs => {
            let s = setv(st, Pr::Shape, stairs_shape(t, l, st, pos));
            setb(s, Pr::Waterlogged, is_source_water(l, pos))
        }
        Beh::Fence => {
            let w = t.wooden_fences[block_of(st) as usize];
            let s = sides_placed(l, pos, st, |_, n, back| fence_conn(t, n, back, w));
            setb(s, Pr::Waterlogged, is_source_water(l, pos))
        }
        Beh::Bars => {
            let s = sides_placed(l, pos, st, |_, n, back| bars_conn(t, n, back));
            setb(s, Pr::Waterlogged, is_source_water(l, pos))
        }
        Beh::Wall => {
            let mut conn = [false; 4];
            for (i, &d) in HORIZONTAL.iter().enumerate() {
                conn[i] = wall_conn(t, l.get(pos.relative(d)), opposite(d));
            }
            let s = setb(st, Pr::Waterlogged, is_source_water(l, pos));
            wall_with(t, s, conn, l.get(above(pos)))
        }
        Beh::FenceGate => setb(st, Pr::InWall, gate_in_wall(t, l, pos, data::get(st, data::P::Facing))),
        Beh::Door => setv(st, Pr::Hinge, door_hinge(t, l, pos, data::get(st, data::P::Facing), ctx)),
        Beh::Chorus => {
            let blk = block_of(st);
            let mut s = st;
            for d in [DOWN, UP, NORTH, SOUTH, 4, 5] {
                let n = l.get(pos.relative(d));
                let on = block_of(n) == blk || block_of(n) == t.chorus_flower || (d == DOWN && t.supports_chorus[block_of(n) as usize]);
                s = setb(s, dir_pr(d), on);
            }
            s
        }
        Beh::HugeMushroom => {
            let blk = block_of(st);
            let mut s = st;
            for d in [DOWN, UP, NORTH, SOUTH, 4, 5] {
                s = setb(s, dir_pr(d), block_of(l.get(pos.relative(d))) != blk);
            }
            s
        }
        _ => st,
    };
    Some(out)
}

/// `BlockState.updateShape` for this family. `dir` points from `pos` to the
/// changed neighbour `npos`, now `nst`.
pub fn update_shape<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, dir: u8, _npos: BlockPos, nst: u16) -> u16 {
    let t = t();
    let horizontal = dir >= NORTH;
    match beh(st) {
        Beh::Stairs => {
            if horizontal {
                setv(st, Pr::Shape, stairs_shape(t, &*l, st, pos))
            } else {
                st
            }
        }
        Beh::Fence => {
            if !horizontal {
                return st;
            }
            let w = t.wooden_fences[block_of(st) as usize];
            let back = opposite(dir);
            setb(st, dir_pr(dir), fence_conn(t, nst, back, w))
        }
        Beh::Bars => {
            if !horizontal {
                return st;
            }
            setb(st, dir_pr(dir), bars_conn(t, nst, opposite(dir)))
        }
        Beh::Wall => {
            if dir == DOWN {
                st
            } else if dir == UP {
                let conn = [0, 1, 2, 3].map(|i| wall_side_connected(t, st, i));
                wall_with(t, st, conn, nst)
            } else {
                let i = HORIZONTAL.iter().position(|&h| h == dir).unwrap_or(0);
                let mut conn = [false; 4];
                for (k, c) in conn.iter_mut().enumerate() {
                    *c = if k == i { wall_conn(t, nst, opposite(dir)) } else { wall_side_connected(t, st, k) };
                }
                wall_with(t, st, conn, l.get(above(pos)))
            }
        }
        Beh::FenceGate => {
            // `FenceGateBlock.updateShape`: only the axis perpendicular to the facing
            let facing = data::get(st, data::P::Facing);
            if axis(clockwise(facing)) != axis(dir) || dir < NORTH {
                return st;
            }
            let wall = |n: u16| t.walls[block_of(n) as usize];
            let in_wall = wall(nst) || wall(l.get(pos.relative(opposite(dir))));
            setb(st, Pr::InWall, in_wall)
        }
        Beh::Chorus => {
            if !chorus_survives(t, &*l, pos, block_of(st)) {
                // `ticks.scheduleTick(pos, this, 1)`
                engine::schedule(rs, &*l, pos, block_of(st), 1, prio::NORMAL);
                return st;
            }
            let blk = block_of(st);
            let connect = block_of(nst) == blk || block_of(nst) == t.chorus_flower || (dir == DOWN && t.supports_chorus[block_of(nst) as usize]);
            setb(st, dir_pr(dir), connect)
        }
        Beh::HugeMushroom => {
            if block_of(nst) == block_of(st) {
                setb(st, dir_pr(dir), false)
            } else {
                st
            }
        }
        _ => st,
    }
}

/// Scheduled chorus tick: `ChorusPlantBlock.tick`.
pub fn tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    let t = t();
    if !chorus_survives(t, &*l, pos, block_of(st)) {
        engine::destroy_block(rs, l, pos, true, UPDATE_LIMIT);
    }
}

#[cfg(test)]
mod tests {
    use super::super::testw::World as TW;
    use super::*;

    /// Block state from a string like "oak_stairs[facing=north,half=bottom]".
    fn ps(s: &str) -> u16 {
        rine_data::init();
        rine_data::get().blocks.parse_state(s).unwrap_or_else(|| panic!("bad state {s}"))
    }

    fn ctx(p: BlockPos) -> PlaceCtx {
        PlaceCtx { face: 1, cursor: (0.5, 0.5, 0.5), yaw: 0.0, pitch: 0.0, sneaking: false, clicked: p }
    }

    /// Property value of a state as text ("outer_left", "true").
    fn prop(st: u16, name: &str) -> Option<&'static str> {
        rine_data::get().blocks.get_prop(st, name)
    }

    fn origin() -> BlockPos {
        BlockPos::new(0, 64, 0)
    }

    #[test]
    fn stairs_outer_and_inner_corners_follow_getstairsshape() {
        let p = origin();
        let c = ctx(p);
        let st = ps("oak_stairs[facing=north,half=bottom]");
        // outer left: behind stairs face west (counter-clockwise of north)
        let mut w = TW::new();
        w.put(p.offset(0, 0, -1), ps("oak_stairs[facing=west,half=bottom]"));
        assert_eq!(prop(placement(&w, st, p, &c).unwrap(), "shape"), Some("outer_left"));
        // outer right: behind stairs face east
        let mut w = TW::new();
        w.put(p.offset(0, 0, -1), ps("oak_stairs[facing=east,half=bottom]"));
        assert_eq!(prop(placement(&w, st, p, &c).unwrap(), "shape"), Some("outer_right"));
        // inner left: front stairs face west
        let mut w = TW::new();
        w.put(p.offset(0, 0, 1), ps("oak_stairs[facing=west,half=bottom]"));
        assert_eq!(prop(placement(&w, st, p, &c).unwrap(), "shape"), Some("inner_left"));
        // inner right: front stairs face east
        let mut w = TW::new();
        w.put(p.offset(0, 0, 1), ps("oak_stairs[facing=east,half=bottom]"));
        assert_eq!(prop(placement(&w, st, p, &c).unwrap(), "shape"), Some("inner_right"));
    }

    #[test]
    fn stairs_corners_need_the_same_half_and_a_free_side() {
        let p = origin();
        let c = ctx(p);
        let mut w = TW::new();
        w.put(p.offset(0, 0, -1), ps("oak_stairs[facing=west,half=top]"));
        assert_eq!(prop(placement(&w, ps("oak_stairs[facing=north,half=top]"), p, &c).unwrap(), "shape"), Some("outer_left"));
        // a bottom neighbour does not make a top corner
        assert_eq!(prop(placement(&w, ps("oak_stairs[facing=north,half=bottom]"), p, &c).unwrap(), "shape"), Some("straight"));
        // same-facing stairs beside the outer block take the corner away (canTakeShape)
        let mut w = TW::new();
        w.put(p.offset(0, 0, -1), ps("oak_stairs[facing=west,half=bottom]"));
        w.put(p.offset(1, 0, 0), ps("oak_stairs[facing=north,half=bottom]"));
        assert_eq!(prop(placement(&w, ps("oak_stairs[facing=north,half=bottom]"), p, &c).unwrap(), "shape"), Some("straight"));
    }

    #[test]
    fn stairs_update_and_waterlogging() {
        let p = origin();
        let mut rs = Rs::default();
        let st = ps("oak_stairs[facing=north,half=bottom]");
        let behind = ps("oak_stairs[facing=west,half=bottom]");
        let mut w = TW::new();
        w.put(p, st);
        w.put(p.offset(0, 0, -1), behind);
        let out = update_shape(&mut rs, &mut w, st, p, NORTH, p.relative(NORTH), behind);
        assert_eq!(prop(out, "shape"), Some("outer_left"));
        // replaced source water: waterlogged; flowing water does not count
        let mut w = TW::new();
        w.put(p, ps("water[level=0]"));
        let s = placement(&w, st, p, &ctx(p)).unwrap();
        assert_eq!(prop(s, "waterlogged"), Some("true"));
        let mut w = TW::new();
        w.put(p, ps("water[level=1]"));
        let s = placement(&w, st, p, &ctx(p)).unwrap();
        assert_eq!(prop(s, "waterlogged"), Some("false"));
    }

    #[test]
    fn fence_connects_to_stone_and_fence_but_not_to_a_torch() {
        let p = origin();
        let mut w = TW::new();
        w.put(p.offset(-1, 0, 0), ps("stone"));
        w.put(p.offset(1, 0, 0), ps("oak_fence"));
        w.put(p.offset(0, 0, -1), ps("torch"));
        w.put(p.offset(0, 0, 1), ps("nether_brick_fence"));
        let s = placement(&w, ps("oak_fence"), p, &ctx(p)).unwrap();
        assert!(getb(s, Pr::West));
        assert!(getb(s, Pr::East));
        assert!(!getb(s, Pr::North));
        // nether brick fence is not the same wood type and not a full face
        assert!(!getb(s, Pr::South));
        // updateShape: the torch goes away, the stone stays connected
        let mut rs = Rs::default();
        let s2 = update_shape(&mut rs, &mut w, s, p, NORTH, p.relative(NORTH), ps("air"));
        assert!(!getb(s2, Pr::North));
        assert!(getb(s2, Pr::West));
    }

    #[test]
    fn fence_gate_in_wall_follows_its_facing() {
        let p = origin();
        let c = ctx(p);
        let gate = ps("oak_fence_gate[facing=north]");
        // facing north: the wall must be on the west or east side
        let mut w = TW::new();
        w.put(p.offset(1, 0, 0), ps("cobblestone_wall"));
        assert_eq!(prop(placement(&w, gate, p, &c).unwrap(), "in_wall"), Some("true"));
        // a wall in front of a north-facing gate does not count
        let mut w = TW::new();
        w.put(p.offset(0, 0, -1), ps("cobblestone_wall"));
        assert_eq!(prop(placement(&w, gate, p, &c).unwrap(), "in_wall"), Some("false"));
        // east-facing gate: the wall on the north side counts
        let mut w = TW::new();
        w.put(p.offset(0, 0, -1), ps("cobblestone_wall"));
        assert_eq!(prop(placement(&w, ps("oak_fence_gate[facing=east]"), p, &c).unwrap(), "in_wall"), Some("true"));
        // updateShape: a wall appears east of the gate, then goes away
        let mut rs = Rs::default();
        let mut w = TW::new();
        w.put(p, gate);
        let wall = ps("cobblestone_wall");
        w.put(p.offset(1, 0, 0), wall);
        let s = update_shape(&mut rs, &mut w, gate, p, EAST, p.relative(EAST), wall);
        assert_eq!(prop(s, "in_wall"), Some("true"));
        w.put(p.offset(1, 0, 0), 0);
        let s = update_shape(&mut rs, &mut w, s, p, EAST, p.relative(EAST), 0);
        assert_eq!(prop(s, "in_wall"), Some("false"));
    }

    #[test]
    fn panes_connect_to_panes_and_attach_to_solid_faces() {
        let p = origin();
        let c = ctx(p);
        let mut w = TW::new();
        w.put(p.offset(1, 0, 0), ps("iron_bars"));
        w.put(p.offset(0, 0, -1), ps("glass_pane"));
        w.put(p.offset(-1, 0, 0), ps("stone"));
        w.put(p.offset(0, 0, 1), ps("torch"));
        let s = placement(&w, ps("iron_bars"), p, &c).unwrap();
        assert!(getb(s, Pr::East));
        assert!(getb(s, Pr::North));
        assert!(getb(s, Pr::West));
        assert!(!getb(s, Pr::South));
        // a stained pane is an IronBarsBlock too
        let mut w = TW::new();
        w.put(p.offset(1, 0, 0), ps("red_stained_glass_pane"));
        let s = placement(&w, ps("glass_pane"), p, &c).unwrap();
        assert!(getb(s, Pr::East));
        // a wall beside a pane connects
        let mut w = TW::new();
        w.put(p.offset(0, 0, 1), ps("cobblestone_wall"));
        let s = placement(&w, ps("glass_pane"), p, &c).unwrap();
        assert!(getb(s, Pr::South));
    }

    #[test]
    fn wall_post_rules() {
        let p = origin();
        let c = ctx(p);
        let wall = ps("cobblestone_wall");
        // lone wall: a post
        let w = TW::new();
        let s = placement(&w, wall, p, &c).unwrap();
        assert!(getb(s, Pr::Up));
        assert_eq!(prop(s, "north"), Some("none"));
        // straight line north-south: arms low, no post
        let mut w = TW::new();
        w.put(p.offset(0, 0, -1), wall);
        w.put(p.offset(0, 0, 1), wall);
        let s = placement(&w, wall, p, &c).unwrap();
        assert_eq!(prop(s, "north"), Some("low"));
        assert_eq!(prop(s, "south"), Some("low"));
        assert!(!getb(s, Pr::Up));
        // a torch on top raises the post
        w.put(p.offset(0, 1, 0), ps("torch"));
        let s = placement(&w, wall, p, &c).unwrap();
        assert!(getb(s, Pr::Up));
    }

    #[test]
    fn wall_arms_are_tall_only_under_a_full_block() {
        let p = origin();
        let c = ctx(p);
        let wall = ps("cobblestone_wall");
        let mut w = TW::new();
        w.put(p.offset(0, 0, -1), wall);
        w.put(p.offset(0, 0, 1), wall);
        // under a bottom slab the arm stays low
        w.put(p.offset(0, 1, 0), ps("oak_slab[type=bottom]"));
        let s = placement(&w, wall, p, &c).unwrap();
        assert_eq!(prop(s, "north"), Some("low"));
        // under a full block the arms are tall, and a high straight wall has no post
        w.put(p.offset(0, 1, 0), ps("stone"));
        let s = placement(&w, wall, p, &c).unwrap();
        assert_eq!(prop(s, "north"), Some("tall"));
        assert_eq!(prop(s, "south"), Some("tall"));
        assert!(!getb(s, Pr::Up));
        // cover masks: a full cube covers the post and all four arms, a slab nothing
        assert_eq!(cover_of(t(), ps("stone")), 0b11111);
        assert_eq!(cover_of(t(), ps("oak_slab[type=bottom]")), 0);
    }

    #[test]
    fn door_hinge_turns_away_from_a_door_on_one_side() {
        let p = origin();
        let c = ctx(p);
        let door = ps("oak_door[facing=north,half=lower]");
        // lower door on the west (left of a north-facing door): hinge on the right
        let mut w = TW::new();
        w.put(p.offset(-1, 0, 0), ps("oak_door[facing=north,half=lower]"));
        assert_eq!(prop(placement(&w, door, p, &c).unwrap(), "hinge"), Some("right"));
        // lower door on the east: hinge on the left
        let mut w = TW::new();
        w.put(p.offset(1, 0, 0), ps("oak_door[facing=north,half=lower]"));
        assert_eq!(prop(placement(&w, door, p, &c).unwrap(), "hinge"), Some("left"));
        // solid blocks on the left (both halves): the hinge sits against the wall
        let mut w = TW::new();
        w.put(p.offset(-1, 0, 0), ps("stone"));
        w.put(p.offset(-1, 1, 0), ps("stone"));
        assert_eq!(prop(placement(&w, door, p, &c).unwrap(), "hinge"), Some("left"));
    }

    #[test]
    fn chorus_plant_connects_and_breaks_without_support() {
        let p = origin();
        let c = ctx(p);
        let mut w = TW::new();
        w.put(p.offset(0, -1, 0), ps("end_stone"));
        w.put(p.offset(0, 0, -1), ps("chorus_plant"));
        let s = placement(&w, ps("chorus_plant"), p, &c).unwrap();
        assert!(getb(s, Pr::Down));
        assert!(getb(s, Pr::North));
        assert!(!getb(s, Pr::Up));
        assert!(!getb(s, Pr::East));
        // nothing below and nothing beside: the update schedules a tick that breaks it
        let mut w = TW::new();
        let mut rs = Rs::default();
        let plant = ps("chorus_plant[down=false,east=false,north=false,south=false,up=false,west=false]");
        w.put(p, plant);
        let s = update_shape(&mut rs, &mut w, plant, p, NORTH, p.relative(NORTH), 0);
        assert_eq!(s, plant);
        w.tick(&mut rs);
        assert_eq!(w.get(p), 0);
        assert_eq!(w.dropped.len(), 1);
    }

    #[test]
    fn huge_mushroom_has_no_side_toward_its_own_block() {
        let p = origin();
        let c = ctx(p);
        let mut w = TW::new();
        w.put(p.offset(1, 0, 0), ps("red_mushroom_block"));
        let s = placement(&w, ps("red_mushroom_block"), p, &c).unwrap();
        assert!(!getb(s, Pr::East));
        assert!(getb(s, Pr::West));
        assert!(getb(s, Pr::Up));
        assert!(getb(s, Pr::Down));
        // updateShape: the block to the west becomes the same kind
        let mut rs = Rs::default();
        let s2 = update_shape(&mut rs, &mut w, s, p, WEST, p.relative(WEST), ps("red_mushroom_block"));
        assert!(!getb(s2, Pr::West));
    }
}
