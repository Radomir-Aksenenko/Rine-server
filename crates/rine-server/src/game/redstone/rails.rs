//! Rails: survival checks and the power line of powered and activator rails
//! (`BaseRailBlock`, `PoweredRailBlock`). Rail shape connection
//! (`RailState`) is not ported: rails keep the shape they were placed with.

use super::data::*;
use super::engine::*;
use rine_protocol::BlockPos;

const NS: u8 = 0;
const EW: u8 = 1;
const AE: u8 = 2;
const AW: u8 = 3;
const AN: u8 = 4;
const AS: u8 = 5;

fn is_slope(shape: u8) -> bool {
    (AE..=AS).contains(&shape)
}

fn straight(st: u16) -> bool {
    matches!(kind(st), K::PoweredRail | K::ActivatorRail | K::DetectorRail)
}

fn should_be_removed<L: Lvl>(l: &L, pos: BlockPos, shape: u8) -> bool {
    if !sturdy_rigid(l.get(rel(pos, DOWN)), UP) {
        return true;
    }
    let side = match shape {
        AE => EAST,
        AW => WEST,
        AN => NORTH,
        AS => SOUTH,
        _ => return false,
    };
    !sturdy_rigid(l.get(rel(pos, side)), UP)
}

pub fn neighbor_changed<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, _from: u16) {
    if block(l.get(pos)) != block(st) {
        return;
    }
    if should_be_removed(l, pos, get(st, P::Shape)) {
        drop_resources(l, st, pos);
        remove_block(rs, l, pos, false);
    } else {
        update_state(rs, l, st, pos);
    }
}

pub fn on_place<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    if straight(st) {
        neighbor_changed_full(rs, l, st, pos, block(st), false);
    }
}

pub fn after_removal<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, piston: bool) {
    if piston {
        return;
    }
    if is_slope(get(st, P::Shape)) {
        update_neighbors_at(rs, l, rel(pos, UP), block(st));
    }
    if straight(st) {
        update_neighbors_at(rs, l, pos, block(st));
        update_neighbors_at(rs, l, rel(pos, DOWN), block(st));
    }
}

/// `PoweredRailBlock.updateState`.
fn update_state<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    if !matches!(kind(st), K::PoweredRail | K::ActivatorRail) {
        return;
    }
    let is = on(st, P::Powered);
    let should = has_neighbor_signal(rs, l, pos) || find_signal(rs, l, pos, st, true, 0) || find_signal(rs, l, pos, st, false, 0);
    if should != is {
        set_block(rs, l, pos, setb(st, P::Powered, should), fl::ALL, UPDATE_LIMIT);
        update_neighbors_at(rs, l, rel(pos, DOWN), block(st));
        if is_slope(get(st, P::Shape)) {
            update_neighbors_at(rs, l, rel(pos, UP), block(st));
        }
    }
}

fn find_signal<L: Lvl>(rs: &Rs, l: &L, pos: BlockPos, st: u16, forward: bool, depth: u32) -> bool {
    if depth >= 8 {
        return false;
    }
    let (mut x, mut y, mut z) = (pos.x, pos.y, pos.z);
    let mut below = true;
    let mut shape = get(st, P::Shape);
    match shape {
        NS => {
            if forward {
                z += 1
            } else {
                z -= 1
            }
        }
        EW => {
            if forward {
                x -= 1
            } else {
                x += 1
            }
        }
        AE => {
            if forward {
                x -= 1;
            } else {
                x += 1;
                y += 1;
                below = false;
            }
            shape = EW;
        }
        AW => {
            if forward {
                x -= 1;
                y += 1;
                below = false;
            } else {
                x += 1;
            }
            shape = EW;
        }
        AN => {
            if forward {
                z += 1;
            } else {
                z -= 1;
                y += 1;
                below = false;
            }
            shape = NS;
        }
        AS => {
            if forward {
                z += 1;
                y += 1;
                below = false;
            } else {
                z -= 1;
            }
            shape = NS;
        }
        _ => {}
    }
    let p = BlockPos::new(x, y, z);
    same_rail_with_power(rs, l, p, st, forward, depth, shape) || below && same_rail_with_power(rs, l, p.offset(0, -1, 0), st, forward, depth, shape)
}

fn same_rail_with_power<L: Lvl>(rs: &Rs, l: &L, pos: BlockPos, me: u16, forward: bool, depth: u32, dir: u8) -> bool {
    let st = l.get(pos);
    if block(st) != block(me) {
        return false;
    }
    let my = get(st, P::Shape);
    (dir != EW || !matches!(my, NS | AN | AS))
        && (dir != NS || !matches!(my, EW | AE | AW))
        && on(st, P::Powered)
        && (has_neighbor_signal(rs, l, pos) || find_signal(rs, l, pos, st, forward, depth + 1))
}
