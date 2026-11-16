//! Beds (`BedBlock`, `AbstractBedBlock`): the block side of sleeping.
//!
//! Here: the right click on a bed half (`useWithoutItem`), the occupied bit
//! the foot copies from the head (`updateShape`), the stand-up position
//! after waking (`findStandUpPosition`) and the rules that depend only on
//! the world (dark outside, skipping to morning, the sleeper count).
//! The player side (range, obstruction, sleep timer, wake up, status
//! messages) lives in `game/sleep.rs`.

use super::interact::Res;
use super::props::{getb, name, setb, Pr};
use super::{block_of, Beh};
use crate::game::redstone::data::{self, EAST, NORTH, SOUTH, WEST};
use rine_protocol::BlockPos;

/// Bed shape height: `LivingEntity.startSleeping` puts the sleeper at
/// `bed.y + shape.max(Y) + 0.125` (shape max Y is 9/16).
pub const SLEEP_HEIGHT: f64 = 9.0 / 16.0;

/// Beds of the 16 colours. `straw_bed` ends in `_bed` too, but its rule
/// (never sets spawn, destroyed on leave) is not modelled, so it is left out.
pub fn classify(n: &str) -> Option<Beh> {
    if n.ends_with("_bed") && n != "straw_bed" {
        Some(Beh::Bed)
    } else {
        None
    }
}

/// Whether the state is a bed of any colour (`AbstractBedBlock`).
pub fn is_bed(st: u16) -> bool {
    super::beh(st) == Beh::Bed
}

/// `ServerPlayer.bedInRange`: the feet are within 3 blocks horizontally and
/// 2 vertically of the bottom centre of the head or of the foot.
pub fn in_range(feet: [f64; 3], head: BlockPos, facing: u8) -> bool {
    let foot = step_pos(head, data::opposite(facing));
    reachable(feet, head) || reachable(feet, foot)
}

fn reachable(feet: [f64; 3], b: BlockPos) -> bool {
    (feet[0] - (b.x as f64 + 0.5)).abs() <= 3.0 && (feet[1] - b.y as f64).abs() <= 2.0 && (feet[2] - (b.z as f64 + 0.5)).abs() <= 3.0
}

/// Whether the state is the head half (`BedPart.HEAD`).
pub fn is_head(st: u16) -> bool {
    name(st, Pr::Part) == Some("head")
}

/// Protocol direction the bed faces (`FACING`).
pub fn facing(st: u16) -> u8 {
    match name(st, Pr::Facing) {
        Some("north") => NORTH,
        Some("south") => SOUTH,
        Some("west") => WEST,
        _ => EAST,
    }
}

pub fn occupied(st: u16) -> bool {
    getb(st, Pr::Occupied)
}

pub fn with_occupied(st: u16, v: bool) -> u16 {
    setb(st, Pr::Occupied, v)
}

/// `AbstractBedBlock.getNeighbourDirection`: the foot points at the head.
fn neighbour_dir(head: bool, facing: u8) -> u8 {
    if head {
        data::opposite(facing)
    } else {
        facing
    }
}

/// The cell `d` steps away from `p`.
pub fn step_pos(p: BlockPos, d: u8) -> BlockPos {
    let (dx, dy, dz) = data::step(d);
    BlockPos::new(p.x + dx, p.y + dy, p.z + dz)
}

/// The head cell of the bed whose half is `st` at `pos`.
pub fn head_of(pos: BlockPos, st: u16) -> BlockPos {
    if is_head(st) {
        pos
    } else {
        step_pos(pos, facing(st))
    }
}

/// Overworld bed rule (`minecraft:gameplay/bed_rule`): `can_sleep: when_dark`,
/// `can_set_spawn: always`. `Level.isDarkOutside` is `skyDarken >= 4`
/// (`isBrightOutside` is `skyDarken < 4`).
pub fn dark_outside(sky_darken: u8) -> bool {
    sky_darken >= 4
}

/// `ServerLevel.tick` skip target: the wake-up marker (`wake_up_from_sleep`,
/// tick 0 of the 24000 period) that comes next, via `moveToTimeMarker`.
/// Exactly on the marker nothing moves (`occursAt`), so None.
pub fn skip_target(total: i64) -> Option<i64> {
    const PERIOD: i64 = 24000;
    let from = total.rem_euclid(PERIOD);
    if from == 0 {
        None
    } else {
        Some(total + (PERIOD - from))
    }
}

/// `BedBlock.useWithoutItem` for a bed half at `pos`. Only reads the world;
/// the caller runs the player side. A broken pair (other half missing or
/// not a half of the same bed) swallows the click.
pub fn use_without_item(get: impl Fn(BlockPos) -> u16, st: u16, pos: BlockPos) -> Res {
    let other_pos = step_pos(pos, neighbour_dir(is_head(st), facing(st)));
    let other = get(other_pos);
    if block_of(other) != block_of(st) || is_head(other) == is_head(st) {
        return Res::swing();
    }
    let head = head_of(pos, st);
    let head_st = if head == pos { st } else { get(head) };
    if occupied(head_st) {
        Res::Bed(BedUse::Occupied)
    } else {
        Res::Bed(BedUse::Sleep { head, facing: facing(st) })
    }
}

/// `BlockState.updateShape` of a bed half. Only the occupied copy from the
/// other half is modelled. Vanilla turns the half into air when the other
/// half is gone; the break code in `game/interact.rs` already removes both
/// halves, so that branch is not repeated here.
pub fn update_shape(st: u16, dir: u8, nst: u16) -> u16 {
    if dir != neighbour_dir(is_head(st), facing(st)) {
        return st;
    }
    if block_of(nst) == block_of(st) && is_head(nst) != is_head(st) {
        with_occupied(st, occupied(nst))
    } else {
        st
    }
}

/// What a click on a bed asks the player side to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BedUse {
    /// The head is taken: `block.minecraft.bed.occupied` on the action bar.
    Occupied,
    /// Try to sleep with the head at `head` (`Player.startSleepInBed`).
    Sleep { head: BlockPos, facing: u8 },
}

/// Whether a cell has any collision box (`!shape.isEmpty()`).
fn has_collision(st: u16) -> bool {
    let b = &rine_data::get().blocks;
    !b.shapes[b.state(st).shape as usize].is_empty()
}

/// Top of the collision of a state (`shape.max(Y)`), if it has any.
fn collision_top(st: u16) -> Option<f32> {
    let b = &rine_data::get().blocks;
    let boxes = &b.shapes[b.state(st).shape as usize];
    boxes.iter().map(|bx| bx[4]).fold(None, |m: Option<f32>, v| Some(m.map_or(v, |m| m.max(v))))
}

/// `AbstractBedBlock.findStandUpPosition` for a single bed (bunk beds are
/// not handled). The sleeper stands next to the head, or on the foot side,
/// on the first cell where the feet and the head fit and the floor is the
/// top of the cell below. Returns the feet position.
///
/// Approximation: vanilla tests the player's full box with
/// `DismountHelper`; here a cell fits when it has no collision at all, and
/// a floor cell is only accepted when the cell itself is empty.
pub fn stand_up(get: impl Fn(BlockPos) -> u16, head: BlockPos, facing: u8, yaw: f32) -> Option<[f64; 3]> {
    let (fx, fz) = horizontal(facing);
    let right = clockwise(facing);
    let side = if facing_angle(right, yaw) { data::opposite(right) } else { right };
    let (sx, sz) = horizontal(side);
    let offsets: [(i32, i32); 12] = [
        (sx, sz),
        (sx - fx, sz - fz),
        (sx - fx * 2, sz - fz * 2),
        (-fx * 2, -fz * 2),
        (-sx - fx * 2, -sz - fz * 2),
        (-sx - fx, -sz - fz),
        (-sx, -sz),
        (-sx + fx, -sz + fz),
        (fx, fz),
        (sx + fx, sz + fz),
        (0, 0),
        (-fx, -fz),
    ];
    for (ox, oz) in offsets {
        let c = BlockPos::new(head.x + ox, head.y, head.z + oz);
        let cell = get(c);
        if has_collision(cell) {
            continue;
        }
        let Some(floor) = collision_top(get(BlockPos::new(c.x, c.y - 1, c.z))) else { continue };
        let body = get(BlockPos::new(c.x, c.y + 1, c.z));
        if has_collision(body) {
            continue;
        }
        return Some([c.x as f64 + 0.5, c.y as f64 + floor as f64 - 1.0, c.z as f64 + 0.5]);
    }
    None
}

/// Horizontal step (x, z) of a protocol direction.
fn horizontal(d: u8) -> (i32, i32) {
    let (x, _, z) = data::step(d);
    (x, z)
}

/// `Direction.getClockWise()` for the horizontal directions.
fn clockwise(d: u8) -> u8 {
    match d {
        NORTH => EAST,
        EAST => SOUTH,
        SOUTH => WEST,
        _ => NORTH,
    }
}

/// `Direction.isFacingAngle(yaw)`: the horizontal normal of `d` points into
/// the half-plane the yaw looks at.
fn facing_angle(d: u8, yaw: f32) -> bool {
    let (nx, nz) = horizontal(d);
    let rad = yaw * std::f32::consts::PI / 180.0;
    let dx = -rad.sin();
    let dz = rad.cos();
    nx as f32 * dx + nz as f32 * dz > 0.0
}

/// `ServerPlayer.RespawnPosAngle.of` yaw: the player looks at the bed from
/// the stand-up position (`Mth.wrapDegrees(atan2 - 90)`).
pub fn look_at_yaw(feet: [f64; 3], bed: BlockPos) -> f32 {
    let (tx, ty, tz) = (bed.x as f64 + 0.5, bed.y as f64, bed.z as f64 + 0.5);
    let (dx, dy, dz) = (tx - feet[0], ty - feet[1], tz - feet[2]);
    let len = (dx * dx + dy * dy + dz * dz).sqrt();
    let (nx, nz) = (dx / len, dz / len);
    let mut d = nz.atan2(nx) * 180.0 / std::f64::consts::PI - 90.0;
    d %= 360.0;
    if d >= 180.0 {
        d -= 360.0;
    }
    if d < -180.0 {
        d += 360.0;
    }
    d as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: i32, y: i32, z: i32) -> BlockPos {
        BlockPos::new(x, y, z)
    }

    /// Foot and head states of a red bed facing north (occupied as given).
    fn pair(_foot_occ: bool, head_occ: bool) -> (u16, u16) {
        use super::super::props::{setv, vid};
        let bed = super::super::default_state("red_bed");
        let bed = setv(bed, Pr::Facing, vid("north"));
        let foot = setv(bed, Pr::Part, vid("foot"));
        let head = with_occupied(setv(bed, Pr::Part, vid("head")), head_occ);
        (foot, head)
    }

    #[test]
    fn range_is_three_across_two_up() {
        let head = at(0, 64, 0);
        // facing north: the foot is at z = 1, its centre z = 1.5
        assert!(in_range([0.5, 64.0, 3.5], head, NORTH), "3 blocks from the head centre");
        assert!(!in_range([0.5, 64.0, 4.6], head, NORTH), "4.1 from head, 3.1 from foot");
        assert!(in_range([0.5, 66.0, 0.5], head, NORTH), "2 up");
        assert!(!in_range([0.5, 67.0, 0.5], head, NORTH), "3 up");
        // reachable from the foot side only
        assert!(in_range([0.5, 64.0, 4.4], head, NORTH), "2.9 from foot centre");
    }

    #[test]
    fn classify_takes_colour_beds_not_straw() {
        assert_eq!(classify("red_bed"), Some(Beh::Bed));
        assert_eq!(classify("white_bed"), Some(Beh::Bed));
        assert_eq!(classify("straw_bed"), None);
        assert_eq!(classify("stone"), None);
    }

    #[test]
    fn overworld_dark_window_follows_sky_darken() {
        rine_data::init();
        // (clock, expected sleep allowed with no weather)
        let cases = [(1000, false), (6000, false), (12000, false), (12600, true), (18000, true), (23000, true), (23600, false)];
        for (t, want) in cases {
            let d = crate::game::blocks::sky_darken(t, 0.0, 0.0);
            assert_eq!(dark_outside(d), want, "clock {t} darken {d}");
        }
    }

    #[test]
    fn thunder_at_noon_allows_sleep_rain_alone_does_not() {
        rine_data::init();
        let noon = 6000;
        assert!(!dark_outside(crate::game::blocks::sky_darken(noon, 0.0, 0.0)));
        // rain only: darken 3, still bright
        assert!(!dark_outside(crate::game::blocks::sky_darken(noon, 1.0, 0.0)));
        // thunder: darken 5, dark enough
        assert!(dark_outside(crate::game::blocks::sky_darken(noon, 1.0, 1.0)));
    }

    #[test]
    fn skip_moves_to_next_wake_marker() {
        // dusk: 13000 -> 24000 (11000 ticks)
        assert_eq!(skip_target(13000), Some(24000));
        // just after dawn of a day: 1000 -> 24000
        assert_eq!(skip_target(24000 + 1000), Some(48000));
        // exactly on the marker: vanilla occursAt, no move
        assert_eq!(skip_target(24000), None);
        assert_eq!(skip_target(0), None);
        // 23999 -> 24000, one tick
        assert_eq!(skip_target(23999), Some(24000));
    }

    #[test]
    fn foot_click_with_free_head_sleeps_at_head() {
        rine_data::init();
        let (foot_st, head_st) = pair(false, false);
        let foot = at(0, 64, 0);
        let head = at(0, 64, -1);
        let get = |p: BlockPos| if p == foot { foot_st } else if p == head { head_st } else { 0 };
        let r = use_without_item(get, foot_st, foot);
        assert_eq!(r, Res::Bed(BedUse::Sleep { head, facing: NORTH }));
    }

    #[test]
    fn occupied_head_is_reported_from_either_half() {
        rine_data::init();
        let (foot_st, head_st) = pair(false, true);
        let foot = at(0, 64, 0);
        let head = at(0, 64, -1);
        let get = |p: BlockPos| if p == foot { foot_st } else if p == head { head_st } else { 0 };
        assert_eq!(use_without_item(get, foot_st, foot), Res::Bed(BedUse::Occupied));
        // clicking the head itself
        let get2 = |p: BlockPos| if p == head { head_st } else if p == foot { foot_st } else { 0 };
        assert_eq!(use_without_item(get2, head_st, head), Res::Bed(BedUse::Occupied));
    }

    #[test]
    fn broken_pair_swallows_the_click() {
        rine_data::init();
        let (foot_st, _) = pair(false, false);
        let foot = at(0, 64, 0);
        let r = use_without_item(|_| 0, foot_st, foot);
        assert_eq!(r, Res::swing());
    }

    #[test]
    fn foot_copies_occupied_from_head_only() {
        rine_data::init();
        let (foot, head) = pair(false, false);
        let head_occ = with_occupied(head, true);
        // the head sits north of the foot: the foot's neighbour direction is north
        assert!(occupied(update_shape(foot, NORTH, head_occ)));
        assert!(!occupied(update_shape(foot, NORTH, with_occupied(head, false))));
        // a change on the other side does nothing
        assert_eq!(update_shape(foot, SOUTH, head_occ), foot);
    }

    #[test]
    fn occupied_bit_reaches_the_foot_through_neighbour_updates() {
        use super::super::testw::World as TW;
        use crate::game::redstone::engine::{self, fl, UPDATE_LIMIT};
        use crate::game::redstone::engine::Lvl;
        use crate::game::redstone::Rs;
        rine_data::init();
        let (foot_st, head_st) = pair(false, false);
        let foot = at(0, 64, 0);
        let head = at(0, 64, -1);
        let mut w = TW::new();
        let mut rs = Rs::default();
        w.put(foot, foot_st);
        w.put(head, head_st);
        engine::set_block(&mut rs, &mut w, head, with_occupied(head_st, true), fl::ALL, UPDATE_LIMIT);
        assert!(occupied(w.get(head)));
        assert!(occupied(w.get(foot)), "foot copies the head's occupied bit");
        engine::set_block(&mut rs, &mut w, head, with_occupied(head_st, false), fl::ALL, UPDATE_LIMIT);
        assert!(!occupied(w.get(foot)));
        assert!(!occupied(w.get(head)));
    }

    #[test]
    fn stand_up_finds_floor_next_to_bed() {
        rine_data::init();
        let stone = super::super::default_state("stone");
        let air = super::super::default_state("air");
        let head = at(0, 64, 0);
        let get = |p: BlockPos| if p.y == 63 { stone } else { air };
        let feet = stand_up(get, head, NORTH, 0.0).expect("stand-up cell");
        // east of the head first (yaw 0 looks south, so the side is east)
        assert_eq!(feet, [1.5, 64.0, 0.5]);
    }

    #[test]
    fn look_at_yaw_faces_the_bed() {
        // standing south of a bed at (0,0,0) looking north: yaw 180 would face +z
        let y = look_at_yaw([0.5, 0.0, 2.5], at(0, 0, 0));
        assert!((y - 0.0).abs() < 1e-3 || (y.abs() - 180.0).abs() < 1e-3, "{y}");
    }
}
