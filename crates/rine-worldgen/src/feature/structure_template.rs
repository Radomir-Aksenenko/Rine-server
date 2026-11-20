use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::Value;

use super::block_entity::BlockEntityData;
use super::predicate::BlockPredicate;
use super::region::{Pos, Region};
use super::rng::WorldgenRng;
use super::speleothem::{offset, pointed_states, read};
use super::state::Fluid;
use super::{block_set, state_provider, Compiler};
use crate::random::{Legacy, Rng};

const DATA: &str = include_str!("../../assets/worldgen/feature_templates.json");
const UPDATE_ALL: u32 = 3;
pub(super) const UPDATE_NONE: u32 = 260;
const UPDATE_KNOWN_SHAPE: u32 = 16;
const UPDATE_SKIP_ALL_SIDEEFFECTS: u32 = 816;
const UPDATE_NEIGHBORS: u32 = 1;
const SHAPE_ORDER: [usize; 6] = [4, 5, 2, 3, 0, 1];

#[derive(Deserialize)]
struct SerializedTemplate {
    size: [i32; 3],
    palette: Vec<[String; 4]>,
    blocks: Vec<[i32; 5]>,
}

#[derive(Deserialize)]
struct Bundle {
    templates: std::collections::HashMap<String, SerializedTemplate>,
    processors: Value,
}

fn bundle() -> &'static Bundle {
    static BUNDLE: OnceLock<Bundle> = OnceLock::new();
    BUNDLE.get_or_init(|| serde_json::from_str(DATA).expect("bundled feature templates"))
}

#[derive(Clone, Copy)]
pub(super) struct Bounds {
    pub min: Pos,
    pub max: Pos,
}

impl Bounds {
    fn contains(self, (x, y, z): Pos) -> bool {
        let (a, b) = (self.min, self.max);
        a.0 <= x && x <= b.0 && a.1 <= y && y <= b.1 && a.2 <= z && z <= b.2
    }
}

pub(super) fn rotate((x, y, z): Pos, rotation: usize) -> Pos {
    match rotation {
        0 => (x, y, z),
        1 => (-z, y, x),
        2 => (-x, y, -z),
        3 => (z, y, -x),
        _ => unreachable!(),
    }
}

pub(super) fn add(a: Pos, b: Pos) -> Pos {
    (a.0 + b.0, a.1 + b.1, a.2 + b.2)
}

#[derive(Clone, Copy)]
struct States {
    normal: [u16; 4],
    wet: [Option<u16>; 4],
}

pub(super) struct Template {
    pub size: [i32; 3],
    palette: Vec<States>,
    blocks: Vec<[i32; 5]>,
}

pub(super) enum Processor {
    Rot(f32),
    Protected(BlockPredicate),
    Rule {
        input: u16,
        output: u16,
        loot: Option<String>,
    },
}

pub(super) fn processors(v: &Value, c: &mut Compiler) -> Result<Vec<Processor>, String> {
    let v = if let Some(name) = v.as_str() {
        &bundle().processors[name]
    } else {
        v
    };
    let entries = v["processors"]
        .as_array()
        .ok_or("missing template processors")?;
    entries
        .iter()
        .map(|p| match p["processor_type"].as_str() {
            Some("minecraft:block_rot") if p.get("rottable_blocks").is_none() => Ok(
                Processor::Rot(p["integrity"].as_f64().ok_or("missing integrity")? as f32),
            ),
            Some("minecraft:protected_blocks") => Ok(Processor::Protected(block_set(
                &p["value"],
                &c.tag_names,
                &mut c.pal,
            )?)),
            Some("minecraft:rule") => {
                let list = p["rules"]
                    .as_array()
                    .filter(|r| r.len() == 1)
                    .ok_or("expected one template rule")?;
                let rule = &list[0];
                if rule["input_predicate"]["predicate_type"] != "minecraft:block_match"
                    || rule["location_predicate"]["predicate_type"] != "minecraft:always_true"
                    || rule.get("position_predicate").is_some()
                {
                    return Err("unsupported template rule predicates".into());
                }
                let loot = if let Some(m) = rule.get("block_entity_modifier") {
                    if m["type"] != "minecraft:append_loot" {
                        return Err("unsupported template block entity modifier".into());
                    }
                    Some(
                        m["loot_table"]
                            .as_str()
                            .ok_or("missing loot table")?
                            .to_owned(),
                    )
                } else {
                    None
                };
                Ok(Processor::Rule {
                    input: c.pal.block(
                        rule["input_predicate"]["block"]
                            .as_str()
                            .ok_or("missing rule block")?,
                    ),
                    output: state_provider::state_of(&rule["output_state"], &mut c.pal)?,
                    loot,
                })
            }
            _ => Err(format!("unsupported template processor {p}")),
        })
        .collect()
}

pub(super) struct TemplatePlacer {
    barrier: u16,
    sand: u16,
    suspicious_sand: u16,
    pointed: [Pointed; 2],
    potent_sulfur: u16,
    sulfur_states: [u16; 5],
    magma: u16,
    lava: u16,
}

struct Pointed {
    block: u16,
    states: [[[u16; 2]; 5]; 2],
}

impl Pointed {
    fn decode(&self, state: u16) -> Option<(usize, usize, usize)> {
        self.states.iter().enumerate().find_map(|(dir, t)| {
            t.iter().enumerate().find_map(|(thickness, wet)| {
                wet.iter()
                    .position(|&s| s == state)
                    .map(|wet| (dir, thickness, wet))
            })
        })
    }
}

impl TemplatePlacer {
    pub fn compile(c: &mut Compiler) -> Self {
        Self {
            barrier: c.pal.default_state("minecraft:barrier"),
            sand: c.pal.block("minecraft:sand"),
            suspicious_sand: c.pal.block("minecraft:suspicious_sand"),
            potent_sulfur: c.pal.block("minecraft:potent_sulfur"),
            sulfur_states: ["dry", "wet", "dormant", "erupting", "continuous"].map(|s| {
                c.pal
                    .state_from_parts("minecraft:potent_sulfur", &[("potent_sulfur_state", s)])
            }),
            magma: c.pal.block("minecraft:magma_block"),
            lava: c.pal.block("minecraft:lava"),
            pointed: ["minecraft:sulfur_spike", "minecraft:pointed_dripstone"].map(|name| {
                Pointed {
                    block: c.pal.block(name),
                    states: pointed_states(&mut c.pal, name),
                }
            }),
        }
    }

    pub fn place(
        &self,
        template: &Template,
        rules: &[Processor],
        region: &mut Region,
        rng: &mut WorldgenRng,
        origin: Pos,
        rotation: usize,
        bounds: Option<Bounds>,
        flags: u32,
    ) -> bool {
        // Each bundled template has a single palette. Vanilla still samples it.
        rng.next_int_bound(1);
        if template.blocks.is_empty() {
            return false;
        }
        let mut plan = Vec::with_capacity(template.blocks.len());
        'blocks: for &[x, y, z, i, nbt] in &template.blocks {
            let pos = add(origin, rotate((x, y, z), rotation));
            if bounds.is_some_and(|b| !b.contains(pos)) {
                continue;
            }
            let states = template.palette[i as usize];
            let mut state = states.normal[rotation];
            let mut wet = states.wet[rotation];
            let mut payload = None;
            for rule in rules {
                match rule {
                    Processor::Rot(integrity) if rng.next_float() > *integrity => continue 'blocks,
                    Processor::Protected(pred) if pred.test(region, pos.0, pos.1, pos.2) => {
                        continue 'blocks
                    }
                    Processor::Rule {
                        input,
                        output,
                        loot,
                    } if region.tags.is_block(states.normal[0], *input) => {
                        state = *output;
                        wet = None;
                        if let Some(table) = loot {
                            let seed =
                                Legacy::new(crate::mth::get_seed(pos.0, pos.1, pos.2)).next_long();
                            payload = Some((table.as_str(), seed));
                        }
                    }
                    _ => {}
                }
            }
            plan.push((pos, state, wet, nbt != 0, payload));
        }

        let mut placed = Vec::with_capacity(plan.len());
        let mut pending = Vec::new();
        let mut locked = std::collections::HashSet::new();
        for (pos, state, wet, nbt, payload) in plan {
            let old = region.tags.fluid(read(region, pos));
            if nbt || payload.is_some() {
                region.write(pos, self.barrier, UPDATE_NONE | UPDATE_SKIP_ALL_SIDEEFFECTS);
            }
            if !region.write(pos, state, flags) {
                continue;
            }
            if let Some((table, seed)) = payload {
                region.record_block_entity(
                    pos,
                    "minecraft:brushable_block",
                    BlockEntityData::Loot {
                        table: table.into(),
                        seed,
                    },
                );
            } else if nbt {
                region.record_block_entity(
                    pos,
                    "minecraft:brushable_block",
                    BlockEntityData::Empty,
                );
            } else if region.tags.is_block(state, self.potent_sulfur) {
                region.record_block_entity(
                    pos,
                    "minecraft:potent_sulfur",
                    BlockEntityData::PotentSulfur,
                );
            }
            if matches!(region.tags.fluid(state), Fluid::Water | Fluid::Lava) {
                locked.insert(pos);
            } else if let Some(wet) = wet {
                if old == Fluid::Water {
                    region.write(pos, wet, UPDATE_ALL);
                } else if old != Fluid::Lava {
                    pending.push((pos, wet));
                }
            }
            placed.push(pos);
        }
        while !pending.is_empty() {
            let before = pending.len();
            pending.retain(|&(pos, wet)| {
                let source = [1, 2, 5, 3, 4].into_iter().any(|d| {
                    let p = offset(pos, d);
                    !locked.contains(&p) && region.tags.fluid(read(region, p)) == Fluid::Water
                });
                if source {
                    region.write(pos, wet, UPDATE_ALL);
                }
                !source
            });
            if pending.len() == before {
                break;
            }
        }
        self.update_shapes(region, placed, flags);
        true
    }

    fn update_shapes(&self, region: &mut Region, placed: Vec<Pos>, flags: u32) {
        let Some(&(x, y, z)) = placed.first() else {
            return;
        };
        let mut min = [x, y, z];
        let mut max = min;
        for &(x, y, z) in &placed {
            for (i, v) in [x, y, z].into_iter().enumerate() {
                min[i] = min[i].min(v);
                max[i] = max[i].max(v);
            }
        }
        let size = std::array::from_fn::<_, 3, _>(|i| (max[i] - min[i] + 1) as usize);
        let index = |[x, y, z]: [usize; 3]| (y * size[2] + z) * size[0] + x;
        let mut points = vec![false; size.iter().product()];
        for &(x, y, z) in &placed {
            let p = [x, y, z];
            points[index(std::array::from_fn(|i| (p[i] - min[i]) as usize))] = true;
        }
        let edge_flags = flags & !UPDATE_NEIGHBORS;
        for ((a, b, c), negative, positive) in
            [((0, 1, 2), 2, 3), ((2, 0, 1), 0, 1), ((1, 2, 0), 4, 5)]
        {
            let mut p = [0; 3];
            for av in 0..size[a] {
                p[a] = av;
                for bv in 0..size[b] {
                    p[b] = bv;
                    let mut last = false;
                    for cv in 0..=size[c] {
                        p[c] = cv;
                        let full = cv < size[c] && points[index(p)];
                        if full != last {
                            let dir = if full { negative } else { positive };
                            if last {
                                p[c] -= 1;
                            }
                            let [x, y, z] = std::array::from_fn(|i| p[i] as i32 + min[i]);
                            self.update_edge(region, (x, y, z), dir, edge_flags);
                        }
                        last = full;
                    }
                }
            }
        }
        for pos in placed {
            let old = read(region, pos);
            let mut state = old;
            for dir in SHAPE_ORDER {
                let neighbor = read(region, offset(pos, dir));
                state = self.update_shape(region, pos, state, dir, neighbor);
            }
            if state != old {
                region.write(pos, state, edge_flags | UPDATE_KNOWN_SHAPE);
            }
        }
    }

    fn update_edge(&self, region: &mut Region, pos: Pos, dir: usize, flags: u32) {
        let other = offset(pos, dir);
        let from = read(region, pos);
        let to = read(region, other);
        let new_from = self.update_shape(region, pos, from, dir, to);
        if new_from != from {
            region.write(pos, new_from, flags);
        }
        let new_to = self.update_shape(region, other, to, dir ^ 1, new_from);
        if new_to != to {
            region.write(other, new_to, flags);
        }
    }

    fn pointed_at(&self, region: &Region, state: u16) -> Option<(&Pointed, (usize, usize, usize))> {
        let block = region.tags.block_of(state);
        let pointed = self.pointed.iter().find(|p| p.block == block)?;
        Some((pointed, pointed.decode(state)?))
    }

    fn update_shape(
        &self,
        region: &mut Region,
        pos: Pos,
        state: u16,
        dir: usize,
        neighbor: u16,
    ) -> u16 {
        let block = region.tags.block_of(state);
        if block == self.sand || block == self.suspicious_sand {
            region.schedule_once(pos, state);
        }
        if block == self.potent_sulfur {
            // PotentSulfurBlock.validBlockState.
            if region.tags.fluid(read(region, offset(pos, 1))) != Fluid::Water {
                return self.sulfur_states[0];
            }
            let below = read(region, offset(pos, 0));
            if region.tags.is_block(below, self.lava) && region.tags.fluid(below) == Fluid::Lava {
                return self.sulfur_states[4];
            }
            if region.tags.is_block(below, self.magma) {
                return if state == self.sulfur_states[3] {
                    state
                } else {
                    self.sulfur_states[2]
                };
            }
            return self.sulfur_states[1];
        }
        if dir >= 2 {
            return state;
        }
        let Some((pointed, (tip, thickness, wet))) = self.pointed_at(region, state) else {
            return state;
        };
        if tip == 0 && region.has_scheduled(pos, state) {
            return state;
        }
        if dir == tip ^ 1
            && !region.tags.sturdy(neighbor, tip)
            && !self
                .pointed_at(region, neighbor)
                .is_some_and(|(p, (d, _, _))| p.block == block && d == tip)
        {
            region.schedule_once(pos, state);
            return state;
        }
        let front = self.pointed_at(region, read(region, offset(pos, tip)));
        let next = match front {
            Some((p, (d, t, _))) if p.block == pointed.block && d != tip => {
                if thickness == 4 || t == 4 {
                    4
                } else {
                    3
                }
            }
            Some((_, (d, t, _))) if d == tip => {
                if t >= 3 {
                    2
                } else if self
                    .pointed_at(region, read(region, offset(pos, tip ^ 1)))
                    .is_some_and(|(_, (d, _, _))| d == tip)
                {
                    1
                } else {
                    0
                }
            }
            _ => 3,
        };
        pointed.states[tip][next][wet]
    }
}

impl Template {
    pub fn compile(name: &str, c: &mut Compiler) -> Result<Self, String> {
        let data = bundle()
            .templates
            .get(name)
            .ok_or_else(|| format!("missing template {name}"))?;
        let palette = data
            .palette
            .iter()
            .map(|states| {
                let normal = states.each_ref().map(|name| c.pal.intern(name));
                let wet = std::array::from_fn(|i| {
                    let name = &states[i];
                    if name.contains("waterlogged=false") && !name.contains("type=double") {
                        c.pal.with_property(normal[i], "waterlogged", "true")
                    } else {
                        None
                    }
                });
                States { normal, wet }
            })
            .collect();
        Ok(Self {
            size: data.size,
            palette,
            blocks: data.blocks.clone(),
        })
    }
}

pub(super) struct TemplateFeature {
    templates: Vec<(Template, Vec<usize>, i32)>,
    total: i32,
    processors: Vec<Processor>,
    placer: TemplatePlacer,
}

impl TemplateFeature {
    pub fn compile(v: &Value, c: &mut Compiler) -> Result<Self, String> {
        let entries = v["templates"].as_array().ok_or("missing templates")?;
        let mut templates = Vec::with_capacity(entries.len());
        let mut total = 0i32;
        for e in entries {
            let data = &e["data"];
            let template = Template::compile(data["id"].as_str().ok_or("missing template id")?, c)?;
            let rotations = match data.get("rotations") {
                None => (0..4).collect(),
                Some(v) => v
                    .as_array()
                    .filter(|l| !l.is_empty())
                    .ok_or("empty template rotations")?
                    .iter()
                    .map(|r| match r.as_str() {
                        Some("none") => Ok(0),
                        Some("clockwise_90") => Ok(1),
                        Some("180") => Ok(2),
                        Some("counterclockwise_90") => Ok(3),
                        _ => Err("unknown template rotation"),
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            };
            let weight = e["weight"]
                .as_i64()
                .and_then(|n| i32::try_from(n).ok())
                .filter(|n| *n > 0)
                .ok_or("invalid template weight")?;
            total = total
                .checked_add(weight)
                .ok_or("template weight overflow")?;
            templates.push((template, rotations, total));
        }
        if templates.is_empty() {
            return Err("empty template feature".into());
        }
        let processors = v
            .get("processors")
            .map(|p| processors(p, c))
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            templates,
            total,
            processors,
            placer: TemplatePlacer::compile(c),
        })
    }

    pub fn place(&self, region: &mut Region, rng: &mut WorldgenRng, origin: Pos) -> bool {
        let draw = rng.next_int_bound(self.total);
        let (template, rotations, _) =
            &self.templates[self.templates.partition_point(|e| e.2 <= draw)];
        let rotation = rotations[rng.next_int_bound(rotations.len() as i32) as usize];
        let [sx, _, sz] = template.size;
        let pos = add(origin, rotate((-sx / 2, 0, -sz / 2), rotation));
        self.placer.place(
            template,
            &self.processors,
            region,
            rng,
            pos,
            rotation,
            None,
            UPDATE_ALL,
        )
    }
}
