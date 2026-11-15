//! Operator status and the TNT stress-test helper (commands live in `cmd`).

use super::entity;
use super::Game;
use crate::out;

/// Spawn `n` primed TNT in a cube 5 blocks above `pos` (stress test).
pub fn spawn_tnt_cube(g: &mut Game, pos: [f64; 3], n: usize) -> usize {
    let side = (n as f64).cbrt().ceil() as i64;
    let mut k = 0;
    'o: for dy in 0..side {
        for dx in 0..side {
            for dz in 0..side {
                if k >= n {
                    break 'o;
                }
                entity::spawn_tnt(g, [pos[0] + (dx - side / 2) as f64, pos[1] + 5.0 + dy as f64, pos[2] + (dz - side / 2) as f64], 80);
                k += 1;
            }
        }
    }
    k
}

/// Apply operator status: entity_event 24+level makes the client offer
/// commands (and F3+F4 etc.), then resend the command tree for that level.
pub fn set_op(g: &mut Game, slot: usize, op: bool) {
    let Some(name) = g.player(slot).map(|p| p.name.clone()) else { return };
    let level = if op { g.shared.config.op_permission_level.clamp(1, 4) } else { 0 };
    let tree = g.cmds.packet(level);
    let Some(p) = g.player(slot) else { return };
    p.op = op;
    let eid = p.eid;
    p.send(out::entity_event(eid, 24 + level as i8));
    p.out.push(tree);
    let _ = name;
}
