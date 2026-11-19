use super::predicate::BlockPredicate;
use super::region::{Pos, Region};
use super::rng::WorldgenRng;

pub(super) struct HugeFungus {
    pub base: u16,
    pub stem: u16,
    pub hat: u16,
    pub decor: u16,
    pub air: u16,
    pub replaceable: BlockPredicate,
    pub planted: bool,
    pub vines: Option<(u16, [u16; 3])>,
}

impl HugeFungus {
    /// HugeFungusFeature.place. Planting bypasses both generator height and huge-stem checks.
    pub fn place(&self, region: &mut Region, rng: &mut WorldgenRng, (x, y, z): Pos) -> bool {
        let tags = region.tags;
        if !tags.is_block(region.get_block(x, y - 1, z), self.base) {
            return false;
        }
        let mut height = rng.next_int_between(4, 13);
        if rng.next_int_bound(12) == 0 {
            height *= 2;
        }
        if !self.planted && y + height + 1 >= region.height() {
            return false;
        }
        let huge = !self.planted && rng.next_float() < 0.06;
        region.write((x, y, z), self.air, 4 | 256);
        let radius = i32::from(huge);
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                let corner = huge && dx.abs() == radius && dz.abs() == radius;
                for dy in 0..height {
                    let p = (x + dx, y + dy, z + dz);
                    if !tags.replaceable(region.get_block(p.0, p.1, p.2)) && !self.replaceable.test(region, p.0, p.1, p.2) {
                        continue;
                    }
                    if self.planted {
                        self.destroy_if_supported(region, p);
                    } else if corner && rng.next_float() >= 0.1 {
                        continue;
                    }
                    region.write(p, self.stem, 3);
                }
            }
        }
        self.place_hat(region, rng, (x, y, z), height, huge);
        true
    }

    fn destroy_if_supported(&self, region: &mut Region, (x, y, z): Pos) {
        if !region.is_air(x, y - 1, z) {
            region.destroy((x, y, z), self.air);
        }
    }

    fn place_hat(&self, region: &mut Region, rng: &mut WorldgenRng, (x, y, z): Pos, height: i32, huge: bool) {
        let tags = region.tags;
        let hat_height = (rng.next_int_bound(1 + height / 3) + 5).min(height);
        let start = height - hat_height;
        for dy in start..=height {
            let mut radius: i32 = if dy < height - rng.next_int_bound(3) { 2 } else { 1 };
            if hat_height > 8 && dy < start + 4 {
                radius = 3;
            }
            radius += i32::from(huge);
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    let p = (x + dx, y + dy, z + dz);
                    if !tags.replaceable(region.get_block(p.0, p.1, p.2)) {
                        continue;
                    }
                    let edge_x = dx.abs() == radius;
                    let edge_z = dz.abs() == radius;
                    let inside = !edge_x && !edge_z && dy != height;
                    if self.planted {
                        self.destroy_if_supported(region, p);
                    }
                    if dy < start + 3 {
                        if inside {
                            continue;
                        }
                        if tags.block_of(region.get_block(p.0, p.1 - 1, p.2)) == tags.block_of(self.hat) {
                            region.write(p, self.hat, 3);
                        } else if (rng.next_float() as f64) < 0.15 {
                            region.write(p, self.hat, 3);
                            if self.vines.is_some() && rng.next_int_bound(11) == 0 {
                                self.place_vines(region, rng, p);
                            }
                        }
                    } else {
                        let (decor, hat, vines) = if inside {
                            (0.1, 0.2, 0.1)
                        } else if edge_x && edge_z {
                            (0.01, 0.7, 0.083)
                        } else {
                            (0.0005, 0.98, 0.07)
                        };
                        if rng.next_float() < decor {
                            region.write(p, self.decor, 3);
                        } else if rng.next_float() < hat {
                            region.write(p, self.hat, 3);
                            if rng.next_float() < if self.vines.is_some() { vines } else { 0.0 } {
                                self.place_vines(region, rng, p);
                            }
                        }
                    }
                }
            }
        }
    }

    fn place_vines(&self, region: &mut Region, rng: &mut WorldgenRng, (x, y, z): Pos) {
        let Some((body, tips)) = self.vines else { return };
        if !region.is_air(x, y - 1, z) {
            return;
        }
        let mut height = rng.next_int_between(1, 5);
        if rng.next_int_bound(7) == 0 {
            height *= 2;
        }
        for i in 0..=height {
            let py = y - 1 - i;
            if region.is_air(x, py, z) {
                if i == height || !region.is_air(x, py - 1, z) {
                    region.write((x, py, z), tips[rng.next_int_bound(3) as usize], 2);
                    break;
                }
                region.write((x, py, z), body, 2);
            }
        }
    }
}
