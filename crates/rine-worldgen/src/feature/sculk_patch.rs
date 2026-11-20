use serde_json::Value;

use super::block_entity::BlockEntityData;
use super::region::{Pos, Region};
use super::rng::WorldgenRng;
use super::shape::DIRECTIONS;
use super::speleothem::{offset, read};
use super::state::Fluid;
use super::Compiler;

const FACES: [&str; 6] = ["down", "up", "north", "south", "west", "east"];
const NEIGHBORS: [Pos; 18] = [
    (0, -1, -1),
    (-1, 0, -1),
    (0, 0, -1),
    (1, 0, -1),
    (0, 1, -1),
    (-1, -1, 0),
    (0, -1, 0),
    (1, -1, 0),
    (-1, 0, 0),
    (1, 0, 0),
    (-1, 1, 0),
    (0, 1, 0),
    (1, 1, 0),
    (0, -1, 1),
    (-1, 0, 1),
    (0, 0, 1),
    (1, 0, 1),
    (0, 1, 1),
];
const SUBSTRATE: u8 = 1;
const WORLDGEN_SUBSTRATE: u8 = 2;
const INHIBITOR: u8 = 4;
const FIRE: u8 = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Behavior {
    Default,
    Sculk,
    Vein,
}

#[derive(Clone, Copy)]
struct Cursor {
    pos: Pos,
    charge: i32,
    decay: bool,
    cooldown: bool,
    facing: Option<u8>,
}

pub(super) struct SculkPatch {
    count: usize,
    charge: i32,
    attempts: i32,
    growth_rounds: i32,
    spread_rounds: i32,
    air: u16,
    water: u16,
    water_block: u16,
    sculk: u16,
    sculk_block: u16,
    catalyst: u16,
    piston: u16,
    vein: u16,
    vein_states: [[u16; 64]; 2],
    vein_faces: Vec<u8>,
    growth: [[u16; 2]; 2],
    tags: Vec<u8>,
}

impl SculkPatch {
    pub fn compile(v: &Value, c: &mut Compiler) -> Result<Self, String> {
        let int = |k: &str, min, max| {
            v[k].as_i64()
                .filter(|n| (min..=max).contains(n))
                .map(|n| n as i32)
                .ok_or_else(|| format!("sculk_patch: invalid {k}"))
        };
        let pal = &mut c.pal;
        let vein = pal.block("minecraft:sculk_vein");
        let vein_states = std::array::from_fn(|wet| {
            std::array::from_fn(|bits| {
                let flag = |yes| if yes { "true" } else { "false" };
                // Registry state names use alphabetical properties.
                let props =
                    [0, 5, 2, 3, 1, 4].map(|dir| (FACES[dir], flag(bits & (1 << dir) != 0)));
                let [down, east, north, south, up, west] = props;
                pal.state_from_parts(
                    "minecraft:sculk_vein",
                    &[
                        down,
                        east,
                        north,
                        south,
                        up,
                        ("waterlogged", flag(wet != 0)),
                        west,
                    ],
                )
            })
        });
        let mut vein_faces = vec![0; pal.states().len()];
        for states in vein_states {
            for (faces, state) in states.into_iter().enumerate() {
                vein_faces[state as usize] = faces as u8;
            }
        }
        let growth = ["minecraft:sculk_sensor", "minecraft:sculk_shrieker"].map(|id| {
            let state = pal.default_state(id);
            let state = pal
                .with_property(state, "can_summon", "true")
                .unwrap_or(state);
            [
                state,
                pal.with_property(state, "waterlogged", "true")
                    .expect("sculk growth has waterlogged property"),
            ]
        });
        let registries = &rine_data::init().registries;
        let mut tags = Vec::new();
        for (name, bit) in [
            ("minecraft:sculk_replaceable", SUBSTRATE),
            ("minecraft:sculk_replaceable_world_gen", WORLDGEN_SUBSTRATE),
            ("minecraft:sculk_growth_inhibitors", INHIBITOR),
            ("minecraft:fire", FIRE),
        ] {
            let members = registries
                .tag("minecraft:block", name)
                .ok_or_else(|| format!("missing {name}"))?;
            for member in members {
                let block = pal.block(member) as usize;
                if block >= tags.len() {
                    tags.resize(block + 1, 0);
                }
                tags[block] |= bit;
            }
        }
        Ok(Self {
            count: int("charge_count", 1, 32)? as usize,
            charge: int("amount_per_charge", 1, 500)?,
            attempts: int("spread_attempts", 1, 64)?,
            growth_rounds: int("growth_rounds", 0, 8)?,
            spread_rounds: int("spread_rounds", 0, 8)?,
            air: pal.default_state("minecraft:air"),
            water: pal.default_state("minecraft:water"),
            water_block: pal.block("minecraft:water"),
            sculk: pal.default_state("minecraft:sculk"),
            sculk_block: pal.block("minecraft:sculk"),
            catalyst: pal.block("minecraft:sculk_catalyst"),
            piston: pal.block("minecraft:moving_piston"),
            vein,
            vein_states,
            vein_faces,
            growth,
            tags,
        })
    }

    fn behavior(&self, region: &Region, state: u16) -> Behavior {
        let block = region.tags.block_of(state);
        if block == self.vein {
            Behavior::Vein
        } else if block == self.sculk_block {
            Behavior::Sculk
        } else {
            Behavior::Default
        }
    }

    fn in_tag(&self, region: &Region, state: u16, bit: u8) -> bool {
        self.tags
            .get(region.tags.block_of(state) as usize)
            .is_some_and(|v| v & bit != 0)
    }

    fn faces(&self, region: &Region, state: u16) -> u8 {
        if region.tags.is_block(state, self.vein) {
            self.vein_faces.get(state as usize).copied().unwrap_or(0)
        } else {
            0
        }
    }

    pub fn place(&self, region: &mut Region, rng: &mut WorldgenRng, origin: Pos) -> bool {
        let state = read(region, origin);
        if self.behavior(region, state) == Behavior::Default
            && !((region.tags.is_air(state) || state == self.water)
                && (0..6).any(|d| region.tags.full_collision(read(region, offset(origin, d)))))
        {
            return false;
        }
        let initial = Cursor {
            pos: origin,
            charge: self.charge,
            decay: true,
            cooldown: false,
            facing: None,
        };
        for round in 0..self.spread_rounds + self.growth_rounds {
            let mut cursors = [initial; 32];
            let mut count = self.count;
            for _ in 0..self.attempts {
                let mut kept = 0;
                for i in 0..count {
                    let mut cursor = cursors[i];
                    self.update(region, rng, origin, &mut cursor, round < self.spread_rounds);
                    if cursor.charge > 0 {
                        cursors[kept] = cursor;
                        kept += 1;
                    }
                }
                count = kept;
                if count == 0 {
                    break;
                }
            }
        }
        true
    }

    fn update(
        &self,
        region: &mut Region,
        rng: &mut WorldgenRng,
        origin: Pos,
        cursor: &mut Cursor,
        spread: bool,
    ) {
        if cursor.cooldown {
            cursor.cooldown = false;
            return;
        }
        let pos = cursor.pos;
        let mut state = read(region, pos);
        let mut behavior = self.behavior(region, state);
        if spread
            && self.spread(region, pos, state, behavior, cursor.facing)
            && behavior != Behavior::Sculk
        {
            state = read(region, pos);
            behavior = self.behavior(region, state);
        }
        cursor.charge = match behavior {
            Behavior::Default => {
                if cursor.decay {
                    cursor.charge
                } else {
                    0
                }
            }
            Behavior::Sculk => self.use_sculk_charge(region, rng, origin, cursor),
            Behavior::Vein => {
                if spread && self.convert_substrate(region, rng, pos) {
                    cursor.charge - 1
                } else if rng.next_int_bound(5) == 0 {
                    cursor.charge / 2
                } else {
                    cursor.charge
                }
            }
        };
        if cursor.charge <= 0 {
            self.discharge(region, pos, state);
        } else if let Some(next) = self.move_to(region, rng, origin, pos) {
            self.discharge(region, pos, state);
            cursor.pos = next;
            state = read(region, next);
        } else {
            self.discharge(region, pos, state);
            cursor.charge = 0;
            return;
        }
        if self.behavior(region, state) != Behavior::Default {
            cursor.facing = Some(self.faces(region, state));
        }
        cursor.decay = behavior != Behavior::Default;
        cursor.cooldown = true;
    }

    fn use_sculk_charge(
        &self,
        region: &mut Region,
        rng: &mut WorldgenRng,
        origin: Pos,
        cursor: &Cursor,
    ) -> i32 {
        let charge = cursor.charge;
        if rng.next_int_bound(5) != 0 {
            return charge;
        }
        let (x, y, z) = cursor.pos;
        let distance_sq = dist_sq(cursor.pos, origin);
        if distance_sq > 0 && self.can_grow(region, cursor.pos) {
            if rng.next_int_bound(50) < charge {
                let shrieker = rng.next_int_bound(11) == 0;
                let p = (x, y + 1, z);
                let wet = region.tags.fluid(read(region, p)) != Fluid::Empty;
                let state = self.growth[usize::from(shrieker)][usize::from(wet)];
                if region.write(p, state, 3) {
                    let (id, data) = if shrieker {
                        ("minecraft:sculk_shrieker", BlockEntityData::SculkShrieker)
                    } else {
                        ("minecraft:sculk_sensor", BlockEntityData::SculkSensor)
                    };
                    region.record_block_entity(p, id, data);
                }
            }
            (charge - 50).max(0)
        } else if rng.next_int_bound(10) != 0 {
            charge
        } else if distance_sq == 0 {
            charge - 1
        } else {
            let dist = (distance_sq as f64).sqrt() as f32 - 1.0;
            let factor = (dist * dist / (23 * 23) as f32).min(1.0);
            charge - ((charge as f32 * factor * 0.5) as i32).max(1)
        }
    }

    fn can_grow(&self, region: &Region, (x, y, z): Pos) -> bool {
        let above = read(region, (x, y + 1, z));
        if !region.tags.is_air(above) && above != self.water {
            return false;
        }
        let mut inhibitors = 0;
        for dy in 0..=2 {
            for dz in -4..=4 {
                for dx in -4..=4 {
                    inhibitors += usize::from(self.in_tag(
                        region,
                        read(region, (x + dx, y + dy, z + dz)),
                        INHIBITOR,
                    ));
                    if inhibitors > 2 {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn spread(
        &self,
        region: &mut Region,
        pos: Pos,
        state: u16,
        behavior: Behavior,
        faces: Option<u8>,
    ) -> bool {
        if behavior == Behavior::Default {
            match faces {
                None => return self.spread_all(region, pos, state, true),
                Some(faces) if faces != 0 => {
                    if !region.tags.is_air(state) && region.tags.fluid(state) != Fluid::Water {
                        return false;
                    }
                    let bits = (0..6).fold(0, |bits, d| {
                        if faces & (1 << d) != 0 && self.attaches(region, pos, d) {
                            bits | (1 << d)
                        } else {
                            bits
                        }
                    });
                    if bits == 0 {
                        return false;
                    }
                    let wet = region.tags.fluid(state) != Fluid::Empty;
                    region.write(pos, self.vein_states[usize::from(wet)][bits], 3);
                    return true;
                }
                _ => {}
            }
        }
        self.spread_all(region, pos, state, false)
    }

    fn attaches(&self, region: &Region, pos: Pos, dir: usize) -> bool {
        region
            .tags
            .face_full(read(region, offset(pos, dir)), dir ^ 1)
    }

    fn spread_all(&self, region: &mut Region, pos: Pos, source: u16, same_space: bool) -> bool {
        let other = !region.tags.is_block(source, self.vein);
        let source_faces = self.faces(region, source);
        let mut any = false;
        for from in 0..6 {
            if !other && source_faces & (1 << from) == 0 {
                continue;
            }
            for dir in 0..6 {
                if dir / 2 == from / 2 || !other && source_faces & (1 << dir) != 0 {
                    continue;
                }
                let plane = offset(pos, dir);
                let placements = [(pos, dir), (plane, from), (offset(plane, from), dir ^ 1)];
                for &(target, face) in &placements[..if same_space { 1 } else { 3 }] {
                    if self.spread_face(region, pos, target, face) {
                        any = true;
                        break;
                    }
                }
            }
        }
        any
    }

    fn spread_face(&self, region: &mut Region, source: Pos, target: Pos, face: usize) -> bool {
        let old = read(region, target);
        let support = read(region, offset(target, face));
        let block = region.tags.block_of(support);
        if [self.sculk_block, self.catalyst, self.piston].contains(&block) {
            return false;
        }
        if manhattan(source, target) == 2
            && region
                .tags
                .sturdy(read(region, offset(source, face ^ 1)), face)
        {
            return false;
        }
        let fluid = region.tags.fluid(old);
        let is_vein = region.tags.is_block(old, self.vein);
        if !matches!(fluid, Fluid::Empty | Fluid::Water)
            || self.in_tag(region, old, FIRE)
            || !(region.tags.replaceable(old)
                || region.tags.is_air(old)
                || is_vein
                || region.tags.is_block(old, self.water_block) && fluid == Fluid::Water)
        {
            return false;
        }
        let faces = self.faces(region, old) as usize;
        let bit = 1 << face;
        if faces & bit != 0 || !region.tags.face_full(support, face ^ 1) {
            return false;
        }
        region.write(
            target,
            self.vein_states[usize::from(fluid == Fluid::Water)][faces | bit],
            2,
        );
        true
    }

    fn convert_substrate(&self, region: &mut Region, rng: &mut WorldgenRng, pos: Pos) -> bool {
        let faces = self.faces(region, read(region, pos));
        for dir in shuffle([0, 1, 2, 3, 4, 5], rng) {
            if faces & (1 << dir) == 0 {
                continue;
            }
            let target = offset(pos, dir);
            if !self.in_tag(region, read(region, target), WORLDGEN_SUBSTRATE) {
                continue;
            }
            region.write(target, self.sculk, 3);
            self.spread_all(region, target, self.sculk, false);
            for d in 0..6 {
                if d != dir ^ 1 {
                    let p = offset(target, d);
                    self.discharge(region, p, read(region, p));
                }
            }
            return true;
        }
        false
    }

    fn discharge(&self, region: &mut Region, pos: Pos, state: u16) {
        if !region.tags.is_block(state, self.vein) {
            return;
        }
        let mut faces = self.faces(region, state) as usize;
        for dir in 0..6 {
            if faces & (1 << dir) != 0
                && region
                    .tags
                    .is_block(read(region, offset(pos, dir)), self.sculk_block)
            {
                faces &= !(1 << dir);
            }
        }
        let wet = region.tags.fluid(state) != Fluid::Empty;
        let next = if faces == 0 {
            if region.tags.fluid(read(region, pos)) == Fluid::Empty {
                self.air
            } else {
                self.water
            }
        } else {
            self.vein_states[usize::from(wet)][faces]
        };
        region.write(pos, next, 3);
    }

    fn move_to(
        &self,
        region: &Region,
        rng: &mut WorldgenRng,
        origin: Pos,
        pos: Pos,
    ) -> Option<Pos> {
        let mut result = None;
        for (dx, dy, dz) in shuffle(NEIGHBORS, rng) {
            let target = (pos.0 + dx, pos.1 + dy, pos.2 + dz);
            if (origin.0 - target.0).pow(2) + (origin.2 - target.2).pow(2) > 144 {
                continue;
            }
            let state = read(region, target);
            if self.behavior(region, state) == Behavior::Default {
                continue;
            }
            if dx.abs() + dy.abs() + dz.abs() == 2
                && !DIRECTIONS.iter().enumerate().any(|(dir, &(x, y, z))| {
                    x * dx + y * dy + z * dz == 1
                        && !region.tags.sturdy(read(region, offset(pos, dir)), dir ^ 1)
                })
            {
                continue;
            }
            result = Some(target);
            let faces = self.faces(region, state);
            if (0..6).any(|d| {
                faces & (1 << d) != 0
                    && self.in_tag(region, read(region, offset(target, d)), SUBSTRATE)
            }) {
                break;
            }
        }
        result
    }
}

fn shuffle<T, const N: usize>(mut list: [T; N], rng: &mut WorldgenRng) -> [T; N] {
    for i in (2..=N).rev() {
        list.swap(i - 1, rng.next_int_bound(i as i32) as usize);
    }
    list
}

fn manhattan(a: Pos, b: Pos) -> i32 {
    (a.0 - b.0).abs() + (a.1 - b.1).abs() + (a.2 - b.2).abs()
}

fn dist_sq(a: Pos, b: Pos) -> i32 {
    (a.0 - b.0).pow(2) + (a.1 - b.1).pow(2) + (a.2 - b.2).pow(2)
}
