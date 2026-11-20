use super::{
    region::{Pos, Region},
    rng::WorldgenRng,
};
use crate::mth::clamp_f32;

const MAX_SUBMERGED_HEIGHT: usize = 18;

pub(super) struct Iceberg {
    pub state: u16,
    pub sea_level: i32,
    pub air: u16,
    pub water: u16,
    pub snow_block: u16,
    pub water_block: u16,
    pub snow_block_id: u16,
    pub ice_block: u16,
    pub snow_layer: u16,
    pub packed_ice: u16,
    pub blue_ice: u16,
}

struct Shape {
    a: i32,
    c: i32,
    width: i32,
    height: i32,
    ellipse: bool,
    snow: bool,
    angle: f64,
}

impl Iceberg {
    pub fn place(&self, level: &mut Region, rng: &mut WorldgenRng, (x, _, z): Pos) -> bool {
        let origin = (x, self.sea_level, z);
        let snow = rng.next_double() > 0.7;
        let angle = rng.next_double() * 2.0 * std::f64::consts::PI;
        let ellipse_a = 11 - rng.next_int_bound(5);
        let c = 3 + rng.next_int_bound(3);
        let ellipse = rng.next_double() > 0.7;
        let mut height = if ellipse {
            rng.next_int_bound(6) + 6
        } else {
            rng.next_int_bound(15) + 3
        };
        if !ellipse && rng.next_double() > 0.9 {
            height += rng.next_int_bound(19) + 7;
        }
        let below = (height + rng.next_int_bound(11)).min(MAX_SUBMERGED_HEIGHT as i32);
        let width = (height + rng.next_int_bound(7) - rng.next_int_bound(5)).min(11);
        let a = if ellipse { ellipse_a } else { 11 };
        let shape = Shape {
            a,
            c,
            width,
            height,
            ellipse,
            snow,
            angle,
        };
        let (sin, cos) = angle.sin_cos();
        for dx in -a..a {
            for dz in -a..a {
                let rotated = rotate(dx, dz, sin, cos);
                for dy in 0..height {
                    if ellipse {
                        let taper = if dy > 0 && height - dy <= 3 {
                            4 - (height - dy)
                        } else {
                            0
                        };
                        let distance = ellipse_distance(rotated, a, c - taper);
                        self.generate(level, rng, origin, (dx, dy, dz), &shape, distance);
                    } else {
                        let r = round_radius(rng, dy, height, width);
                        if dx < r {
                            let distance = circle_distance(dx, dz, r, rng);
                            self.generate(level, rng, origin, (dx, dy, dz), &shape, distance);
                        }
                    }
                }
            }
        }
        self.smooth(level, origin, &shape);
        let submerged = Shape {
            height: below,
            ..shape
        };
        let narrowed: [i32; MAX_SUBMERGED_HEIGHT] = std::array::from_fn(|depth| {
            (a as f32 * (1.0 - (depth * depth) as f32 / (below as f32 * 8.0))).ceil() as i32
        });
        for dx in -a..a {
            for dz in -a..a {
                let rotated = rotate(dx, dz, sin, cos);
                for depth in 1..below {
                    let r = steep_radius(rng, depth, below, width);
                    if dx < r {
                        let distance = if ellipse {
                            ellipse_distance(rotated, narrowed[depth as usize], c)
                        } else {
                            circle_distance(dx, dz, r, rng)
                        };
                        self.generate(level, rng, origin, (dx, -depth, dz), &submerged, distance);
                    }
                }
            }
        }
        if rng.next_double() > if ellipse { 0.1 } else { 0.7 } {
            self.cut_out(level, rng, origin, &shape, ellipse_a);
        }
        true
    }

    fn generate(
        &self,
        level: &mut Region,
        rng: &mut WorldgenRng,
        (x, y, z): Pos,
        (dx, dy, dz): Pos,
        shape: &Shape,
        distance: f64,
    ) {
        if !(distance < 0.0) {
            return;
        }
        let edge = if shape.ellipse {
            -0.5
        } else {
            (-6 - rng.next_int_bound(3)) as f64
        };
        if distance > edge && rng.next_double() > 0.9 {
            return;
        }
        let p = (x + dx, y + dy, z + dz);
        let old = level.get_block(p.0, p.1, p.2);
        let block = level.tags.block_of(old);
        if !level.tags.is_air(old)
            && ![self.snow_block_id, self.ice_block, self.water_block].contains(&block)
        {
            return;
        }
        let randomness = !shape.ellipse || rng.next_double() > 0.05;
        let divisor = if shape.ellipse { 3 } else { 2 };
        let snowy = shape.snow
            && block != self.water_block
            && (shape.height - dy) as f64
                <= rng.next_int_bound((shape.height / divisor).max(1)) as f64
                    + shape.height as f64 * 0.6
            && randomness;
        level.write(p, if snowy { self.snow_block } else { self.state }, 3);
    }

    fn is_iceberg(&self, block: u16) -> bool {
        [self.packed_ice, self.snow_block_id, self.blue_ice].contains(&block)
    }

    fn smooth(&self, level: &mut Region, (x, y, z): Pos, shape: &Shape) {
        let a = if shape.ellipse {
            shape.a
        } else {
            shape.width / 2
        };
        for px in x - a..=x + a {
            for pz in z - a..=z + a {
                for py in y..=y + shape.height {
                    let block = level.tags.block_of(level.get_block(px, py, pz));
                    let ice = self.is_iceberg(block);
                    if !ice && block != self.snow_layer {
                        continue;
                    }
                    if level.is_air(px, py - 1, pz) {
                        level.write((px, py, pz), self.air, 3);
                        level.write((px, py + 1, pz), self.air, 3);
                    } else if ice {
                        let nearby = [(px - 1, pz), (px + 1, pz), (px, pz - 1), (px, pz + 1)];
                        let count = nearby
                            .into_iter()
                            .filter(|&(nx, nz)| {
                                self.is_iceberg(level.tags.block_of(level.get_block(nx, py, nz)))
                            })
                            .count();
                        if count <= 1 {
                            level.write((px, py, pz), self.air, 3);
                        }
                    }
                }
            }
        }
    }

    fn cut_out(
        &self,
        level: &mut Region,
        rng: &mut WorldgenRng,
        origin: Pos,
        shape: &Shape,
        ellipse_a: i32,
    ) {
        let sx = if rng.next_bool() { -1 } else { 1 };
        let sz = if rng.next_bool() { -1 } else { 1 };
        let offset = |rng: &mut WorldgenRng| {
            let half = shape.width / 2;
            let off = rng.next_int_bound((half - 2).max(1));
            if rng.next_bool() {
                half + 1 - rng.next_int_bound((shape.width - half - 1).max(1))
            } else {
                off
            }
        };
        let mut dx = offset(rng);
        let mut dz = offset(rng);
        let angle = if shape.ellipse {
            dx = rng.next_int_bound((ellipse_a - 5).max(1));
            dz = dx;
            shape.angle + std::f64::consts::FRAC_PI_2
        } else {
            rng.next_double() * 2.0 * std::f64::consts::PI
        };
        let (sin, cos) = angle.sin_cos();
        let carve = |level: &mut Region, r: i32, dy: i32| {
            let a = r + 1 + ellipse_a / 3;
            let c = i32::min(r - 3, 3) + shape.c / 2 - 1;
            for xo in -a..a {
                for zo in -a..a {
                    if ellipse_distance(rotate(xo - sx * dx, zo - sz * dz, sin, cos), a, c) < 0.0 {
                        let (x, y, z) = (origin.0 + xo, origin.1 + dy, origin.2 + zo);
                        if self.is_iceberg(level.tags.block_of(level.get_block(x, y, z))) {
                            level.write((x, y, z), if dy < 0 { self.water } else { self.air }, 3);
                            if dy >= 0
                                && level
                                    .tags
                                    .is_block(level.get_block(x, y + 1, z), self.snow_layer)
                            {
                                level.write((x, y + 1, z), self.air, 3);
                            }
                        }
                    }
                }
            }
        };
        for dy in 0..shape.height - 3 {
            carve(level, round_radius(rng, dy, shape.height, shape.width), dy);
        }
        let mut dy = -1;
        while dy > -shape.height + rng.next_int_bound(5) {
            carve(level, steep_radius(rng, -dy, shape.height, shape.width), dy);
            dy -= 1;
        }
    }
}

fn rotate(x: i32, z: i32, sin: f64, cos: f64) -> (f64, f64) {
    let (x, z) = (x as f64, z as f64);
    (x * cos - z * sin, x * sin + z * cos)
}

fn ellipse_distance((x, z): (f64, f64), a: i32, c: i32) -> f64 {
    let (x, z) = (x / a as f64, z / c as f64);
    x * x + z * z - 1.0
}

fn circle_distance(x: i32, z: i32, r: i32, rng: &mut WorldgenRng) -> f64 {
    let off = 10.0 * clamp_f32(rng.next_float(), 0.2, 0.8) / r as f32;
    off as f64 + (x * x) as f64 + (z * z) as f64 - (r * r) as f64
}

fn round_radius(rng: &mut WorldgenRng, y: i32, height: i32, width: i32) -> i32 {
    let k = 3.5 - rng.next_float();
    let mut scale = (1.0 - (y * y) as f32 / (height as f32 * k)) * width as f32;
    if height > 15 + rng.next_int_bound(5) {
        let offset = if y < 3 + rng.next_int_bound(6) {
            y / 2
        } else {
            y
        };
        scale = (1.0 - offset as f32 / (height as f32 * k * 0.4)) * width as f32;
    }
    (scale / 2.0).ceil() as i32
}

fn steep_radius(rng: &mut WorldgenRng, y: i32, height: i32, width: i32) -> i32 {
    let k = 1.0 + rng.next_float() / 2.0;
    let scale = (1.0 - y as f32 / (height as f32 * k)) * width as f32;
    (scale / 2.0).ceil() as i32
}
