//! "Is any non-spectator player closer than r to this point" for hot block
//! ticks (`ChunkMap.anyPlayerCloseEnoughTo`, used by fire spread).
//!
//! Vanilla scans every player per call. Here the players are bucketed into
//! a horizontal grid of `r`-sized cells once per tick (built lazily on the
//! first query of the tick), so a query looks at the 3x3 cells around the
//! point and does the exact vanilla distance test only on those players.
//! Same answer: a player within distance r is within r on x and z, so it is
//! in one of the 3x3 cells.

use ahash::AHashMap;

#[derive(Default)]
pub struct Grid {
    /// Tick and radius the grid was built for.
    key: Option<(u64, i32)>,
    cells: AHashMap<(i32, i32), Vec<[f64; 3]>>,
    /// Reused cell vectors, so a rebuild does not allocate in steady state.
    spare: Vec<Vec<[f64; 3]>>,
}

impl Grid {
    /// Rebuild for `tick` and `radius` (> 0) unless already built.
    pub fn ensure<'a>(&mut self, tick: u64, radius: i32, players: impl Iterator<Item = &'a [f64; 3]>) {
        if self.key == Some((tick, radius)) {
            return;
        }
        self.key = Some((tick, radius));
        for (_, mut v) in self.cells.drain() {
            v.clear();
            self.spare.push(v);
        }
        let cs = radius as f64;
        for p in players {
            let c = ((p[0] / cs).floor() as i32, (p[2] / cs).floor() as i32);
            let spare = &mut self.spare;
            self.cells.entry(c).or_insert_with(|| spare.pop().unwrap_or_default()).push(*p);
        }
    }

    /// `anyPlayerCloseEnoughTo`: some player with `distanceTo(target) < radius`.
    /// Call `ensure` with the same radius first.
    pub fn any_within(&self, target: [f64; 3], radius: i32) -> bool {
        let cs = radius as f64;
        let (cx, cz) = ((target[0] / cs).floor() as i32, (target[2] / cs).floor() as i32);
        let r = radius as f64;
        for dx in -1..=1 {
            for dz in -1..=1 {
                let Some(v) = self.cells.get(&(cx + dx, cz + dz)) else { continue };
                for p in v {
                    let (a, b, c) = (p[0] - target[0], p[1] - target[1], p[2] - target[2]);
                    if (a * a + b * b + c * c).sqrt() < r {
                        return true;
                    }
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::Grid;

    fn brute(ps: &[[f64; 3]], t: [f64; 3], r: i32) -> bool {
        ps.iter().any(|p| ((p[0] - t[0]).powi(2) + (p[1] - t[1]).powi(2) + (p[2] - t[2]).powi(2)).sqrt() < r as f64)
    }

    #[test]
    fn grid_matches_brute_force() {
        let mut ps = Vec::new();
        let mut s: u64 = 0x1234_5678;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s % 2000) as f64 - 1000.0
        };
        for _ in 0..60 {
            ps.push([next(), next() / 8.0, next()]);
        }
        for r in [1, 16, 128, 300] {
            let mut g = Grid::default();
            g.ensure(1, r, ps.iter());
            for _ in 0..3000 {
                let t = [next(), next() / 8.0, next()];
                assert_eq!(g.any_within(t, r), brute(&ps, t, r), "r={r} t={t:?}");
            }
            // exactly on the boundary: strictly less, as vanilla
            let t = [ps[0][0] + r as f64, ps[0][1], ps[0][2]];
            assert_eq!(g.any_within(t, r), brute(&ps, t, r));
        }
    }

    #[test]
    fn rebuilds_on_new_tick() {
        let mut g = Grid::default();
        let a = [[0.0, 0.0, 0.0]];
        g.ensure(1, 128, a.iter());
        assert!(g.any_within([10.0, 0.0, 0.0], 128));
        let b = [[5000.0, 0.0, 0.0]];
        g.ensure(1, 128, b.iter());
        assert!(g.any_within([10.0, 0.0, 0.0], 128), "same tick keeps the grid");
        g.ensure(2, 128, b.iter());
        assert!(!g.any_within([10.0, 0.0, 0.0], 128));
    }
}

/// cargo test --release -p rine-server near_bench -- --ignored --nocapture
#[cfg(test)]
#[test]
#[ignore]
fn near_bench() {
    use std::time::Instant;
    let mut s: u64 = 99;
    let mut next = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s % 20_000) as f64 - 10_000.0
    };
    let ps: Vec<[f64; 3]> = (0..1000).map(|_| [next(), 64.0, next()]).collect();
    // fires near a few players, as a burning forest would be
    let qs: Vec<[f64; 3]> = (0..100_000).map(|i| { let p = ps[(i * 7919) % 1000]; [p[0] + 40.0, 70.0, p[2] - 30.0] }).collect();
    let t = Instant::now();
    let mut hit = 0;
    for q in &qs {
        if ps.iter().any(|p| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt() < 128.0) {
            hit += 1;
        }
    }
    let brute = t.elapsed();
    let t = Instant::now();
    let mut g = Grid::default();
    let mut hit2 = 0;
    for q in &qs {
        g.ensure(1, 128, ps.iter());
        if g.any_within(*q, 128) {
            hit2 += 1;
        }
    }
    let grid = t.elapsed();
    // worst case for the scan: no player near (the scan reads all 1000)
    let far: Vec<[f64; 3]> = (0..100_000).map(|_| [50_000.0, 64.0, 50_000.0]).collect();
    let t = Instant::now();
    let mut h3 = 0;
    for q in &far {
        if ps.iter().any(|p| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt() < 128.0) {
            h3 += 1;
        }
    }
    let brute_far = t.elapsed();
    let t = Instant::now();
    let mut h4 = 0;
    for q in &far {
        if g.any_within(*q, 128) {
            h4 += 1;
        }
    }
    let grid_far = t.elapsed();
    assert_eq!((hit, h3), (hit2, h4));
    println!("near player: scan {:.1} ns, grid {:.1} ns; no player near: scan {:.1} ns, grid {:.1} ns",
        brute.as_nanos() as f64 / 1e5, grid.as_nanos() as f64 / 1e5, brute_far.as_nanos() as f64 / 1e5, grid_far.as_nanos() as f64 / 1e5);
}
