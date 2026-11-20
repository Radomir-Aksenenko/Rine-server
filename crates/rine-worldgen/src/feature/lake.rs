use super::{
    predicate::BlockPredicate,
    region::{Pos, Region},
    rng::WorldgenRng,
    state::Fluid,
    state_provider::StateProvider,
};

pub(super) struct Lake {
    pub fluid: StateProvider,
    pub barrier: StateProvider,
    pub can_place: BlockPredicate,
    pub replace_fluid: BlockPredicate,
    pub replace_barrier: BlockPredicate,
    pub cave_air: u16,
    pub ice: u16,
    pub water: u16,
}

impl Lake {
    pub fn place(&self, level: &mut Region, rng: &mut WorldgenRng, (x, y, z): Pos) -> bool {
        if y <= level.min_y() + 4 {
            return false;
        }
        // Rine has no structure starts or references in the decoration region.
        let (x, y, z) = (x - 8, y - 4, z - 8);
        let mut cells = [[0u8; 16]; 16];
        for _ in 0..rng.next_int_bound(4) + 4 {
            let xr = rng.next_double() * 6.0 + 3.0;
            let yr = rng.next_double() * 4.0 + 2.0;
            let zr = rng.next_double() * 6.0 + 3.0;
            let xp = rng.next_double() * (16.0 - xr - 2.0) + 1.0 + xr / 2.0;
            let yp = rng.next_double() * (8.0 - yr - 4.0) + 2.0 + yr / 2.0;
            let zp = rng.next_double() * (16.0 - zr - 2.0) + 1.0 + zr / 2.0;
            let sq = |i: usize, center: f64, diameter: f64| {
                let d = (i as f64 - center) / (diameter / 2.0);
                d * d
            };
            let xs: [f64; 16] = std::array::from_fn(|i| sq(i, xp, xr));
            let ys: [f64; 8] = std::array::from_fn(|i| sq(i, yp, yr));
            let zs: [f64; 16] = std::array::from_fn(|i| sq(i, zp, zr));
            let valid_y = (1..7).fold(0, |mask, iy| mask | (u8::from(ys[iy] < 1.0) << iy));
            for ix in (1..15).filter(|&i| xs[i] < 1.0) {
                for iz in (1..15).filter(|&i| xs[ix] + zs[i] < 1.0) {
                    for iy in bits(valid_y & !cells[ix][iz]) {
                        if xs[ix] + ys[iy] + zs[iz] < 1.0 {
                            cells[ix][iz] |= 1 << iy;
                        }
                    }
                }
            }
        }
        let boundary = std::array::from_fn::<_, 16, _>(|ix| {
            std::array::from_fn::<_, 16, _>(|iz| {
                let column = cells[ix][iz];
                let at = |a, b| {
                    cells
                        .get(a)
                        .and_then(|row: &[u8; 16]| row.get(b))
                        .copied()
                        .unwrap_or(0)
                };
                ((column << 1)
                    | (column >> 1)
                    | at(ix + 1, iz)
                    | at(ix.wrapping_sub(1), iz)
                    | at(ix, iz + 1)
                    | at(ix, iz.wrapping_sub(1)))
                    & !column
            })
        });
        let fluid = self.fluid.get(level, rng, x, y, z);
        for (ix, row) in boundary.iter().enumerate() {
            for (iz, &column) in row.iter().enumerate() {
                for iy in bits(column) {
                    let (px, py, pz) = (x + ix as i32, y + iy as i32, z + iz as i32);
                    let state = level.get_block(px, py, pz);
                    if (iy >= 4 && level.tags.liquid(state))
                        || (iy < 4 && !level.tags.solid(state) && state != fluid)
                        || !self.can_place.test(level, px, py, pz)
                    {
                        return false;
                    }
                }
            }
        }
        for (ix, row) in cells.iter().enumerate() {
            for (iz, &column) in row.iter().enumerate() {
                for iy in bits(column) {
                    let p = (x + ix as i32, y + iy as i32, z + iz as i32);
                    if self.replace_fluid.test(level, p.0, p.1, p.2) {
                        let air = iy >= 4;
                        level.write(p, if air { self.cave_air } else { fluid }, 2);
                        if air {
                            level.schedule(p, self.cave_air);
                            level.mark_above(p);
                        }
                    }
                }
            }
        }
        let barrier = self.barrier.get(level, rng, x, y, z);
        if !level.tags.is_air(barrier) {
            for (ix, row) in boundary.iter().enumerate() {
                for (iz, &column) in row.iter().enumerate() {
                    for iy in bits(column) {
                        if iy >= 4 && rng.next_int_bound(2) == 0 {
                            continue;
                        }
                        let p = (x + ix as i32, y + iy as i32, z + iz as i32);
                        if level.tags.solid(level.get_block(p.0, p.1, p.2))
                            && self.replace_barrier.test(level, p.0, p.1, p.2)
                        {
                            level.write(p, barrier, 2);
                            level.mark_above(p);
                        }
                    }
                }
            }
        }
        if matches!(level.tags.fluid(fluid), Fluid::Water | Fluid::FlowingWater) {
            for px in x..x + 16 {
                for pz in z..z + 16 {
                    let p = (px, y + 4, pz);
                    if level.should_freeze_water(p, self.water)
                        && self.replace_fluid.test(level, p.0, p.1, p.2)
                    {
                        level.write(p, self.ice, 2);
                    }
                }
            }
        }
        true
    }
}

fn bits(mut mask: u8) -> impl Iterator<Item = usize> {
    std::iter::from_fn(move || {
        if mask == 0 {
            return None;
        }
        let bit = mask.trailing_zeros() as usize;
        mask &= mask - 1;
        Some(bit)
    })
}
