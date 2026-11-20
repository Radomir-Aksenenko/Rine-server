use super::{
    predicate::BlockPredicate,
    region::{Pos, Region},
    rng::WorldgenRng,
};

pub(super) struct Spike {
    pub state: u16,
    pub on: BlockPredicate,
    pub replace: BlockPredicate,
}

impl Spike {
    pub fn place(&self, level: &mut Region, rng: &mut WorldgenRng, (x, mut y, z): Pos) -> bool {
        while level.is_air(x, y, z) && y > level.min_y() + 2 {
            y -= 1;
        }
        if !self.on.test(level, x, y, z) {
            return false;
        }
        y += rng.next_int_bound(4);
        let height = rng.next_int_bound(4) + 7;
        let width = height / 4 + rng.next_int_bound(2);
        if width > 1 && rng.next_int_bound(60) == 0 {
            y += 10 + rng.next_int_bound(30);
        }
        for dy in 0..height {
            let scale = (1.0 - dy as f32 / height as f32) * width as f32;
            let r = scale.ceil() as i32;
            for dx in -r..=r {
                let sx = dx.abs() as f32 - 0.25;
                for dz in -r..=r {
                    let sz = dz.abs() as f32 - 0.25;
                    if (dx != 0 || dz != 0) && sx * sx + sz * sz > scale * scale {
                        continue;
                    }
                    if (dx.abs() == r || dz.abs() == r) && rng.next_float() > 0.75 {
                        continue;
                    }
                    let (px, pz) = (x + dx, z + dz);
                    if level.is_air(px, y + dy, pz) || self.replace.test(level, px, y + dy, pz) {
                        level.write((px, y + dy, pz), self.state, 3);
                    }
                    if dy != 0
                        && r > 1
                        && (level.is_air(px, y - dy, pz)
                            || self.replace.test(level, px, y - dy, pz))
                    {
                        level.write((px, y - dy, pz), self.state, 3);
                    }
                }
            }
        }
        let r = (width - 1).clamp(0, 1);
        for dx in -r..=r {
            for dz in -r..=r {
                let (px, pz) = (x + dx, z + dz);
                let mut py = y - 1;
                let mut length = if dx.abs() == 1 && dz.abs() == 1 {
                    rng.next_int_bound(5)
                } else {
                    50
                };
                while py > 50 {
                    let old = level.get_block(px, py, pz);
                    if !level.tags.is_air(old)
                        && !self.replace.test(level, px, py, pz)
                        && old != self.state
                    {
                        break;
                    }
                    level.write((px, py, pz), self.state, 3);
                    py -= 1;
                    length -= 1;
                    if length <= 0 {
                        py -= rng.next_int_bound(5) + 1;
                        length = rng.next_int_bound(5);
                    }
                }
            }
        }
        true
    }
}
