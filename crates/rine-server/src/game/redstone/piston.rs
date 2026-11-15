//! Pistons: `PistonBaseBlock`, `PistonStructureResolver`, `PistonHeadBlock`
//! and the `PistonMovingBlockEntity` ticker. Entities are not pushed.

use super::data::*;
use super::engine::*;
use rine_protocol::BlockPos;
use rine_world::{MAX_Y, MIN_Y};

const MAX_PUSH: usize = 12;

/// Reused resolver lists.
#[derive(Default)]
pub struct Scratch {
    to_push: Vec<BlockPos>,
    to_destroy: Vec<BlockPos>,
}

fn is_base(st: u16) -> bool {
    matches!(kind(st), K::Piston { .. })
}

fn sticky_block(st: u16) -> bool {
    let ids = &t().ids;
    let b = block(st);
    b == ids.slime || b == ids.honey
}

fn can_stick(a: u16, b: u16) -> bool {
    let ids = &t().ids;
    let (ba, bb) = (block(a), block(b));
    !(ba == ids.honey && bb == ids.slime) && !(ba == ids.slime && bb == ids.honey) && (sticky_block(a) || sticky_block(b))
}

fn push_reaction(st: u16) -> Push {
    t().push[block(st) as usize]
}

/// `PistonBaseBlock.isPushable`.
pub fn is_pushable(st: u16, pos: BlockPos, dir: u8, allow_destroy: bool, conn: u8) -> bool {
    if pos.y < MIN_Y || pos.y > MAX_Y {
        return false;
    }
    if is_air(st) {
        return true;
    }
    if dir == DOWN && pos.y == MIN_Y || dir == UP && pos.y == MAX_Y {
        return false;
    }
    if !is_base(st) {
        if rine_data::get().blocks.state(st).hardness < 0.0 {
            return false;
        }
        match push_reaction(st) {
            Push::Block => return false,
            Push::Destroy => return allow_destroy,
            Push::PushOnly => return dir == conn,
            _ => {}
        }
    } else if on(st, P::Extended) {
        return false;
    }
    !t().has_be[block(st) as usize]
}

/// The piston's own `getNeighborSignal` (quasi-connectivity included).
fn piston_signal<L: Lvl>(rs: &Rs, l: &L, pos: BlockPos, push: u8) -> bool {
    for d in ALL {
        if d != push && has_signal(rs, l, rel(pos, d), d) {
            return true;
        }
    }
    if has_signal(rs, l, pos, DOWN) {
        return true;
    }
    let above = rel(pos, UP);
    ALL.iter().any(|&d| d != DOWN && has_signal(rs, l, rel(above, d), d))
}

/// `PistonBaseBlock.checkIfExtend`.
pub fn check_if_extend<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    let dir = get(st, P::Facing);
    let extend = piston_signal(rs, l, pos, dir);
    let extended = on(st, P::Extended);
    if extend && !extended {
        let mut sc = std::mem::take(&mut rs.scratch);
        let ok = resolve(&mut sc, l, pos, dir, true);
        rs.scratch = sc;
        if ok {
            queue_block_event(rs, pos, block(st), 0, dir);
        }
    } else if !extend && extended {
        let pushed = rel_n(pos, dir, 2);
        let ps = l.get(pushed);
        let mut ev = 1;
        if kind(ps) == K::MovingPiston && get(ps, P::Facing) == dir {
            if let Some(m) = rs.moving.get(&pushed) {
                if m.extending && (m.progress_o < 0.5 || l.time() == m.last_ticked || rs.handling_tick) {
                    ev = 2;
                }
            }
        }
        queue_block_event(rs, pos, block(st), ev, dir);
    }
}

/// `PistonBaseBlock.triggerEvent`.
pub fn trigger_event<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, a: u8, b: u8) -> bool {
    let K::Piston { sticky } = kind(st) else { return false };
    let dir = get(st, P::Facing);
    let extended = setb(st, P::Extended, true);
    let extend = piston_signal(rs, l, pos, dir);
    if extend && (a == 1 || a == 2) {
        set_block(rs, l, pos, extended, fl::CLIENTS, UPDATE_LIMIT);
        return false;
    }
    if !extend && a == 0 {
        return false;
    }
    if a == 0 {
        if !move_blocks(rs, l, pos, dir, true) {
            return false;
        }
        set_block(rs, l, pos, extended, fl::ALL | fl::MOVE_BY_PISTON, UPDATE_LIMIT);
        let pitch = l.rand() * 0.25 + 0.6;
        l.sound("block.piston.extend", pos, 0.5, pitch, CAT_BLOCKS, false);
    } else {
        let front = rel(pos, dir);
        if rs.moving.contains_key(&front) {
            final_tick(rs, l, front);
        }
        let ids = &t().ids;
        let moving = set(set(ids.moving_default, P::Facing, dir), P::Type, sticky as u8);
        set_block(rs, l, pos, moving, fl::NONE | fl::KNOWN_SHAPE, UPDATE_LIMIT);
        let base_state = set(rine_data::get().blocks.blocks[block(st) as usize].default_state as u16, P::Facing, b & 7);
        add_moving(rs, pos, base_state, dir, false, true);
        update_neighbors_at(rs, l, pos, ids.moving_piston);
        neighbour_shapes(rs, l, moving, pos, fl::CLIENTS, UPDATE_LIMIT);
        if sticky {
            let two = rel_n(pos, dir, 2);
            let ms = l.get(two);
            let mut piece = false;
            if kind(ms) == K::MovingPiston {
                if let Some(m) = rs.moving.get(&two).copied() {
                    if m.dir == dir && m.extending {
                        final_tick(rs, l, two);
                        piece = true;
                    }
                }
            }
            if !piece {
                if a != 1 || is_air(ms) || !is_pushable(ms, two, opposite(dir), false, dir) || push_reaction(ms) != Push::Normal && !is_base(ms) {
                    remove_block(rs, l, front, false);
                } else {
                    move_blocks(rs, l, pos, dir, false);
                }
            }
        } else {
            remove_block(rs, l, front, false);
        }
        let pitch = l.rand() * 0.15 + 0.6;
        l.sound("block.piston.contract", pos, 0.5, pitch, CAT_BLOCKS, false);
    }
    true
}

fn add_moving(rs: &mut Rs, pos: BlockPos, state: u16, dir: u8, extending: bool, source: bool) {
    rs.next_uid += 1;
    let uid = rs.next_uid;
    rs.moving.insert(pos, Moving { state, dir, extending, source, progress: 0.0, progress_o: 0.0, last_ticked: 0, uid });
    rs.movers.push((pos, uid));
}

// ------------------------------------------------------------------ resolver

struct Res<'a, L: Lvl> {
    l: &'a L,
    piston: BlockPos,
    push_dir: u8,
    sc: &'a mut Scratch,
}

/// `PistonStructureResolver.resolve`; fills `sc.to_push` / `sc.to_destroy`.
fn resolve<L: Lvl>(sc: &mut Scratch, l: &L, piston: BlockPos, dir: u8, extending: bool) -> bool {
    let (push_dir, start) = if extending { (dir, rel(piston, dir)) } else { (opposite(dir), rel_n(piston, dir, 2)) };
    sc.to_push.clear();
    sc.to_destroy.clear();
    let mut r = Res { l, piston, push_dir, sc };
    let ns = l.get(start);
    if !is_pushable(ns, start, push_dir, false, dir) {
        if extending && push_reaction(ns) == Push::Destroy {
            r.sc.to_destroy.push(start);
            return true;
        }
        return false;
    }
    if !r.add_line(start, push_dir) {
        return false;
    }
    let mut i = 0;
    while i < r.sc.to_push.len() {
        let p = r.sc.to_push[i];
        if sticky_block(l.get(p)) && !r.add_branching(p) {
            return false;
        }
        i += 1;
    }
    true
}

impl<L: Lvl> Res<'_, L> {
    fn add_line(&mut self, start: BlockPos, dir: u8) -> bool {
        let pd = self.push_dir;
        let mut ns = self.l.get(start);
        if is_air(ns) || !is_pushable(ns, start, pd, false, dir) || start == self.piston || self.sc.to_push.contains(&start) {
            return true;
        }
        let mut count = 1usize;
        if count + self.sc.to_push.len() > MAX_PUSH {
            return false;
        }
        while sticky_block(ns) {
            let p = rel_n(start, opposite(pd), count as i32);
            let prev = ns;
            ns = self.l.get(p);
            if is_air(ns) || !can_stick(prev, ns) || !is_pushable(ns, p, pd, false, opposite(pd)) || p == self.piston {
                break;
            }
            count += 1;
            if count + self.sc.to_push.len() > MAX_PUSH {
                return false;
            }
        }
        let mut added = 0usize;
        for i in (0..count).rev() {
            self.sc.to_push.push(rel_n(start, opposite(pd), i as i32));
            added += 1;
        }
        let mut i = 1;
        loop {
            let p = rel_n(start, pd, i);
            if let Some(cp) = self.sc.to_push.iter().position(|q| *q == p) {
                self.reorder(added, cp);
                for j in 0..=cp + added {
                    let bp = self.sc.to_push[j];
                    if sticky_block(self.l.get(bp)) && !self.add_branching(bp) {
                        return false;
                    }
                }
                return true;
            }
            let ns = self.l.get(p);
            if is_air(ns) {
                return true;
            }
            if !is_pushable(ns, p, pd, true, pd) || p == self.piston {
                return false;
            }
            if push_reaction(ns) == Push::Destroy {
                self.sc.to_destroy.push(p);
                return true;
            }
            if self.sc.to_push.len() >= MAX_PUSH {
                return false;
            }
            self.sc.to_push.push(p);
            added += 1;
            i += 1;
        }
    }

    fn reorder(&mut self, added: usize, cp: usize) {
        let v = &mut self.sc.to_push;
        let len = v.len();
        let mut out = Vec::with_capacity(len);
        out.extend_from_slice(&v[..cp]);
        out.extend_from_slice(&v[len - added..]);
        out.extend_from_slice(&v[cp..len - added]);
        *v = out;
    }

    fn add_branching(&mut self, from: BlockPos) -> bool {
        let fs = self.l.get(from);
        for d in ALL {
            if axis(d) != axis(self.push_dir) {
                let np = rel(from, d);
                let ns = self.l.get(np);
                if can_stick(ns, fs) && !self.add_line(np, d) {
                    return false;
                }
            }
        }
        true
    }
}

// ------------------------------------------------------------------ moving

/// `PistonBaseBlock.moveBlocks`.
fn move_blocks<L: Lvl>(rs: &mut Rs, l: &mut L, piston: BlockPos, dir: u8, extending: bool) -> bool {
    let ids = &t().ids;
    let arm = rel(piston, dir);
    if !extending && block(l.get(arm)) == ids.piston_head {
        set_block(rs, l, arm, ids.air_state, fl::NONE | fl::KNOWN_SHAPE, UPDATE_LIMIT);
    }
    let mut sc = std::mem::take(&mut rs.scratch);
    let ok = resolve(&mut sc, l, piston, dir, extending);
    let to_push = sc.to_push.clone();
    let to_destroy = sc.to_destroy.clone();
    rs.scratch = sc;
    if !ok {
        return false;
    }
    // `deleteAfterMove`: a HashMap, kept here as insertion-ordered pairs
    let mut delete_after: Vec<(BlockPos, u16)> = Vec::with_capacity(to_push.len());
    for &p in &to_push {
        delete_after.push((p, l.get(p)));
    }
    let table = java_table_size(to_push.len());
    let mut to_update: Vec<u16> = Vec::with_capacity(to_push.len() + to_destroy.len());
    let push_dir = if extending { dir } else { opposite(dir) };
    for &p in to_destroy.iter().rev() {
        let st = l.get(p);
        drop_resources(l, st, p);
        set_block(rs, l, p, ids.air_state, fl::CLIENTS | fl::KNOWN_SHAPE, UPDATE_LIMIT);
        to_update.push(st);
    }
    for &old in to_push.iter().rev() {
        let pos = rel(old, push_dir);
        delete_after.retain(|(q, _)| *q != pos);
        let actual = set(ids.moving_default, P::Facing, dir);
        set_block(rs, l, pos, actual, fl::NONE | fl::MOVE_BY_PISTON, UPDATE_LIMIT);
        let bs = l.get(old);
        if let Some(e) = delete_after.iter_mut().find(|(q, _)| *q == old) {
            e.1 = bs;
        }
        add_moving(rs, pos, bs, dir, extending, false);
        set_block(rs, l, old, ids.air_state, fl::CLIENTS | fl::KNOWN_SHAPE | fl::MOVE_BY_PISTON | fl::SKIP_ON_PLACE, UPDATE_LIMIT);
        to_update.push(bs);
    }
    if extending {
        let sticky = matches!(kind(l.get(piston)), K::Piston { sticky: true });
        let head = set(set(ids.head_default, P::Facing, dir), P::Type, sticky as u8);
        let ms = set(set(ids.moving_default, P::Facing, dir), P::Type, sticky as u8);
        delete_after.retain(|(q, _)| *q != arm);
        set_block(rs, l, arm, ms, fl::NONE | fl::MOVE_BY_PISTON, UPDATE_LIMIT);
        add_moving(rs, arm, head, dir, true, true);
    }
    let mut keys: Vec<BlockPos> = delete_after.iter().map(|(p, _)| *p).collect();
    java_hash_order(&mut keys, table);
    let air = ids.air_state;
    for &p in &keys {
        set_block(rs, l, p, air, fl::MOVE_BY_PISTON | fl::CLIENTS | fl::KNOWN_SHAPE, UPDATE_LIMIT);
    }
    for &p in &keys {
        let old = delete_after.iter().find(|(q, _)| *q == p).map(|e| e.1).unwrap_or(air);
        indirect_shapes(rs, l, old, p, fl::CLIENTS, UPDATE_LIMIT);
        neighbour_shapes(rs, l, air, p, fl::CLIENTS, UPDATE_LIMIT);
        indirect_shapes(rs, l, air, p, fl::CLIENTS, UPDATE_LIMIT);
    }
    let mut idx = 0;
    for &p in to_destroy.iter().rev() {
        let st = to_update[idx];
        idx += 1;
        after_removal(rs, l, st, p, false);
        indirect_shapes(rs, l, st, p, fl::CLIENTS, UPDATE_LIMIT);
        update_neighbors_at(rs, l, p, block(st));
    }
    for &p in to_push.iter().rev() {
        let st = to_update[idx];
        idx += 1;
        update_neighbors_at(rs, l, p, block(st));
    }
    if extending {
        update_neighbors_at(rs, l, arm, ids.piston_head);
    }
    true
}

/// `PistonMovingBlockEntity.finalTick`.
pub fn final_tick<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos) {
    let Some(m) = rs.moving.get(&pos).copied() else { return };
    if m.progress_o >= 1.0 {
        return;
    }
    rs.moving.remove(&pos);
    if kind(l.get(pos)) == K::MovingPiston {
        let ns = if m.source { t().ids.air_state } else { update_from_neighbour_shapes(rs, l, m.state, pos) };
        set_block(rs, l, pos, ns, fl::ALL, UPDATE_LIMIT);
        neighbor_changed_at(rs, l, pos, block(ns));
    }
}

/// `PistonMovingBlockEntity.tick` for every moving block, in ticker order.
pub fn tick_movers<L: Lvl>(rs: &mut Rs, l: &mut L) {
    if rs.movers.is_empty() {
        return;
    }
    let list = std::mem::take(&mut rs.movers);
    let mut keep = Vec::with_capacity(list.len());
    let now = l.time();
    for (pos, uid) in list {
        let Some(mut m) = rs.moving.get(&pos).copied() else { continue };
        if m.uid != uid {
            continue;
        }
        m.last_ticked = now;
        m.progress_o = m.progress;
        if m.progress_o >= 1.0 {
            rs.moving.remove(&pos);
            if kind(l.get(pos)) == K::MovingPiston {
                let ns = update_from_neighbour_shapes(rs, l, m.state, pos);
                if is_air(ns) {
                    set_block(rs, l, pos, m.state, fl::INVISIBLE | fl::MOVE_BY_PISTON | fl::KNOWN_SHAPE, UPDATE_LIMIT);
                    let cur = l.get(pos);
                    update_or_destroy(rs, l, cur, ns, pos, fl::ALL, UPDATE_LIMIT);
                } else {
                    let ns = if has(ns, P::Waterlogged) { setb(ns, P::Waterlogged, false) } else { ns };
                    set_block(rs, l, pos, ns, fl::ALL | fl::MOVE_BY_PISTON, UPDATE_LIMIT);
                    neighbor_changed_at(rs, l, pos, block(ns));
                }
            }
        } else {
            m.progress = (m.progress + 0.5).min(1.0);
            rs.moving.insert(pos, m);
            keep.push((pos, uid));
        }
    }
    let fresh = std::mem::replace(&mut rs.movers, keep);
    rs.movers.extend(fresh);
}

/// `PistonHeadBlock.isFittingBase`.
pub fn fitting_base(head: u16, base: u16) -> bool {
    let ids = &t().ids;
    let want = if get(head, P::Type) == 0 { ids.piston } else { ids.sticky_piston };
    block(base) == want && on(base, P::Extended) && get(base, P::Facing) == get(head, P::Facing)
}

/// `PistonHeadBlock.canSurvive`.
pub fn head_can_survive<L: Lvl>(l: &L, st: u16, pos: BlockPos) -> bool {
    let base = l.get(rel(pos, opposite(get(st, P::Facing))));
    fitting_base(st, base) || kind(base) == K::MovingPiston && get(base, P::Facing) == get(st, P::Facing)
}
