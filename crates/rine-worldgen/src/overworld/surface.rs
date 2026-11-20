//! `MaterialSystem.buildSurface` (vanilla 26.3): walks every column top-down, evaluates the surface rule
//! tree per solid block with the `MaterialRuleContext` state, and runs the eroded-badlands and
//! frozen-ocean extensions. Heights follow the WORLD_SURFACE_WG heightmap, updated by every placed block.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use crate::density::volume::Vol;
use crate::overworld::blocks::{Blk, AIR};
use crate::overworld::rules::{Cond, OreVein, Rule};
use crate::overworld::{BiomeMap, Overworld, Samplers};
use crate::random::{AnyRng, Rng};

/// `DimensionType.WAY_BELOW_MIN_Y` = `MIN_Y << 4`.
const WAY_BELOW_MIN_Y: i32 = -32512;

fn biome_id(o: &Overworld, name: &str) -> Option<u16> {
    o.biomes.names.iter().position(|n| n == name).map(|i| i as u16)
}

fn is_stone(o: &Overworld, b: Blk) -> bool {
    b != AIR && !o.interner().is_fluid(b)
}

fn java_round(v: f32) -> i32 {
    ((v as f64) + 0.5).floor() as i32
}

fn map_d(value: f64, from_min: f64, from_max: f64, to_min: f64, to_max: f64) -> f64 {
    let alpha = (value - from_min) / (from_max - from_min);
    to_min + alpha * (to_max - to_min)
}

struct Eval<'a, 'b> {
    o: &'a Overworld,
    s: Samplers<'a>,
    biomes: &'b BiomeMap<'a>,
    cx: i32,
    cz: i32,
    min_y: i32,
    max_y: i32,
    blocks: &'b mut Vec<Blk>,
    heights: &'b mut [i32; 256],
    narrow: Vol,
    prelim_vol: Vol,
    prelim: Option<Vec<f32>>,
    ore: HashMap<u32, (Vec<f32>, Vec<f32>)>,
    // MaterialRuleContext state.
    block_x: i32,
    block_z: i32,
    grad_x: i32,
    grad_z: i32,
    surface_depth: i32,
    min_surface: Option<i32>,
    block_y: i32,
    stone_above: i32,
    stone_below: i32,
    water_height: i32,
    /// `MaterialRuleContext` memoizes the biome per block (`biome` supplier reset by `updateY`) and the
    /// column values per `updateXZ`; the rule tree asks for them many times per block.
    biome_memo: Cell<(i32, i32, i32, u16)>,
    noise2d_memo: RefCell<Vec<(usize, i32, i32, f64)>>,
}

impl<'a, 'b> Eval<'a, 'b> {
    fn get(&self, bx: i32, by: i32, bz: i32) -> Blk {
        if by < self.min_y || by > self.max_y {
            return AIR;
        }
        let lx = bx - self.cx * 16;
        let lz = bz - self.cz * 16;
        self.blocks[(((by - self.min_y) * 16 + lz) * 16 + lx) as usize]
    }

    /// `BlockColumn.setBlock` on the proto chunk: bounds-checked, updates WORLD_SURFACE_WG.
    fn set(&mut self, bx: i32, by: i32, bz: i32, b: Blk) {
        if by < self.min_y || by > self.max_y {
            return;
        }
        let lx = bx - self.cx * 16;
        let lz = bz - self.cz * 16;
        self.blocks[(((by - self.min_y) * 16 + lz) * 16 + lx) as usize] = b;
        if b != AIR {
            let col = (lx * 16 + lz) as usize;
            if by >= self.heights[col] {
                self.heights[col] = by + 1;
            }
        }
    }

    fn height(&self, lx: i32, lz: i32) -> i32 {
        self.heights[(lx * 16 + lz) as usize]
    }

    fn noise(&self, idx: usize, x: f64, y: f64, z: f64) -> f64 {
        self.o.tree().noises[idx].get_value(x, y, z) as f64
    }

    /// A 2D noise (y = 0) at a column, memoized for the current column.
    fn noise2d(&self, idx: usize, bx: i32, bz: i32) -> f64 {
        let mut memo = self.noise2d_memo.borrow_mut();
        if let Some(&(_, _, _, v)) = memo.iter().find(|m| m.0 == idx && m.1 == bx && m.2 == bz) {
            return v;
        }
        let v = self.noise(idx, bx as f64, 0.0, bz as f64);
        if memo.first().is_some_and(|m| m.1 != bx || m.2 != bz) {
            memo.clear();
        }
        memo.push((idx, bx, bz, v));
        v
    }

    /// `BiomeManager.getBiome` at a block, memoized for the last block asked.
    fn biome(&self, bx: i32, by: i32, bz: i32) -> u16 {
        let (mx, my, mz, b) = self.biome_memo.get();
        if (mx, my, mz) == (bx, by, bz) {
            return b;
        }
        let b = self.biomes.fiddled(bx, by, bz);
        self.biome_memo.set((bx, by, bz, b));
        b
    }

    fn surface_depth_at(&self, bx: i32, bz: i32) -> i32 {
        let nv = self.noise(self.o.noises.surface, bx as f64, 0.0, bz as f64);
        let mut r = self.o.root_random().at(bx, 0, bz);
        let v = nv * 2.75 + 3.0 + r.next_double() * 0.25;
        v as i32
    }

    fn update_xz(&mut self, bx: i32, bz: i32, gx: i32, gz: i32) {
        self.block_x = bx;
        self.block_z = bz;
        self.grad_x = gx;
        self.grad_z = gz;
        self.surface_depth = self.surface_depth_at(bx, bz);
        self.min_surface = None;
    }

    fn update_y(&mut self, stone_above: i32, stone_below: i32, water_height: i32, by: i32) {
        self.block_y = by;
        self.stone_above = stone_above;
        self.stone_below = stone_below;
        self.water_height = water_height;
    }

    /// `MaterialRuleContext.getMinSurfaceLevel`.
    fn min_surface_level(&mut self) -> i32 {
        if let Some(v) = self.min_surface {
            return v;
        }
        if self.prelim.is_none() {
            let mut buf = vec![0f32; self.prelim_vol.size()];
            self.s.volume(self.o.router.preliminary_surface, &self.prelim_vol, &mut buf);
            self.prelim = Some(buf);
        }
        let lx = self.block_x - self.cx * 16;
        let lz = self.block_z - self.cz * 16;
        let level = self.prelim.as_ref().expect("filled")[self.prelim_vol.index(lx, 0, lz)];
        let v = (level as f64).floor() as i32 + self.surface_depth - 8;
        self.min_surface = Some(v);
        v
    }

    fn surface_secondary(&self, bx: i32, bz: i32) -> f64 {
        self.noise2d(self.o.noises.surface_secondary, bx, bz)
    }

    /// `MaterialSystem.getBand`.
    fn band(&self, bx: i32, by: i32, bz: i32) -> Blk {
        let offset = java_round(self.noise2d(self.o.noises.clay_bands_offset, bx, bz) as f32 * 4.0f32);
        let len = 192;
        let i = (by + offset + len).rem_euclid(len);
        self.o.bands()[i as usize]
    }

    fn warm(&self, bx: i32, by: i32, bz: i32) -> bool {
        let biome = self.biome(bx, by, bz);
        self.o
            .biome_source()
            .warm_enough_to_rain(biome, bx, by, bz, self.o.sea_level())
    }

    fn ore_buffers(&mut self, rule: u32, v: &OreVein) {
        if self.ore.contains_key(&rule) {
            return;
        }
        let size = self.narrow.size();
        let mut density = vec![0f32; size];
        self.s.volume(v.density, &self.narrow, &mut density);
        let mut richness = vec![0f32; size];
        self.s.volume(v.richness, &self.narrow, &mut richness);
        self.ore.insert(rule, (density, richness));
    }

    fn ore_vein(&mut self, rule: u32, v: &OreVein, bx: i32, by: i32, bz: i32) -> Option<Blk> {
        self.ore_buffers(rule, v);
        let index = self.narrow.index_of_block(bx, by, bz);
        let density = match index {
            Some(i) => self.ore[&rule].0[i],
            None => self.s.value(v.density, bx, by, bz),
        };
        if density <= 0.0 {
            return None;
        }
        let mut random: AnyRng = v.rng.at(bx, by, bz);
        if random.next_float() > density {
            return None;
        }
        let richness = match index {
            Some(i) => self.ore[&rule].1[i],
            None => self.s.value(v.richness, bx, by, bz),
        };
        if random.next_float() < richness && self.s.value(v.filler_gap, bx, by, bz) < 0.0 {
            if random.next_float() < v.raw_ore_chance {
                Some(v.raw_ore)
            } else {
                Some(v.ore)
            }
        } else {
            Some(v.filler)
        }
    }

    fn eval_cond(&mut self, c: u32, bx: i32, by: i32, bz: i32) -> bool {
        let o = self.o;
        match &o.conds[c as usize] {
            Cond::Biome(list) => {
                let biome = self.biome(bx, by, bz);
                list.contains(&biome)
            }
            Cond::NoiseThreshold { noise, min, max, is_3d } => {
                let value = if *is_3d { self.noise(*noise, bx as f64, by as f64, bz as f64) } else { self.noise2d(*noise, bx, bz) };
                value >= *min && value <= *max
            }
            Cond::VerticalGradient { rng, true_at, false_at } => {
                if by <= *true_at {
                    return true;
                }
                if by >= *false_at {
                    return false;
                }
                let probability = map_d(by as f64, *true_at as f64, *false_at as f64, 1.0, 0.0);
                let mut r = rng.at(bx, by, bz);
                (r.next_float() as f64) < probability
            }
            Cond::YAbove {
                anchor,
                multiplier,
                add_stone_depth,
            } => {
                let stone = if *add_stone_depth { self.stone_above } else { 0 };
                by + stone >= *anchor + self.surface_depth * *multiplier
            }
            Cond::Water {
                offset,
                multiplier,
                add_stone_depth,
            } => {
                if self.water_height == i32::MIN {
                    return true;
                }
                let stone = if *add_stone_depth { self.stone_above } else { 0 };
                by + stone >= self.water_height + *offset + self.surface_depth * *multiplier
            }
            Cond::Temperature => !self.warm(bx, by, bz),
            Cond::Steep => self.grad_x <= -4 || self.grad_z >= 4,
            Cond::Hole => self.surface_depth <= 0,
            Cond::AbovePreliminary => by >= self.min_surface_level(),
            Cond::StoneDepth {
                offset,
                add_surface_depth,
                secondary_range,
                ceiling,
            } => {
                let stone_depth = if *ceiling { self.stone_below } else { self.stone_above };
                let surface = if *add_surface_depth { self.surface_depth } else { 0 };
                let secondary = if *secondary_range == 0 {
                    0
                } else {
                    map_d(self.surface_secondary(bx, bz), -1.0, 1.0, 0.0, *secondary_range as f64) as i32
                };
                stone_depth <= 1 + *offset + surface + secondary
            }
            Cond::Not(inner) => {
                let inner = *inner;
                !self.eval_cond(inner, bx, by, bz)
            }
        }
    }

    /// `RuleEvaluator.tryApply` of rule `r` at a block.
    fn eval_rule(&mut self, r: u32, bx: i32, by: i32, bz: i32) -> Option<Blk> {
        let o = self.o;
        match &o.rules[r as usize] {
            Rule::Block(b) => Some(*b),
            Rule::Bandlands => Some(self.band(bx, by, bz)),
            Rule::Sequence(kids) => {
                for &k in kids {
                    if let Some(b) = self.eval_rule(k, bx, by, bz) {
                        return Some(b);
                    }
                }
                None
            }
            Rule::Condition { cond, then } => {
                let (cond, then) = (*cond, *then);
                if !self.eval_cond(cond, bx, by, bz) {
                    None
                } else {
                    self.eval_rule(then, bx, by, bz)
                }
            }
            Rule::OreVein(v) => {
                let v = v.clone();
                self.ore_vein(r, &v, bx, by, bz)
            }
        }
    }

    /// `MaterialSystem.erodedBadlandsExtension`.
    fn eroded_badlands(&mut self, bx: i32, bz: i32, height: i32) {
        let n = self.o.noises;
        let surface = self.noise(n.badlands_surface, bx as f64, 0.0, bz as f64) * 8.25;
        let pillar = (self.noise_f32(n.badlands_pillar, bx as f64 * 0.2, 0.0, bz as f64 * 0.2) * 15.0f32) as f64;
        let pillar_buffer = surface.abs().min(pillar);
        if pillar_buffer <= 0.0 {
            return;
        }
        let pillar_floor = (self.noise(n.badlands_pillar_roof, bx as f64 * 0.75, 0.0, bz as f64 * 0.75) * 1.5).abs();
        let extension_top = 64.0 + (pillar_buffer * pillar_buffer * 2.5).min((pillar_floor * 50.0).ceil() + 24.0);
        let start_y = extension_top.floor() as i32;
        if height <= start_y {
            let mut y = start_y;
            while y >= self.min_y {
                let old = self.get(bx, y, bz);
                if self.o.interner().is_block(old, "minecraft:stone") {
                    break;
                }
                if self.o.interner().is_block(old, "minecraft:water") {
                    return;
                }
                y -= 1;
            }
            let mut y = start_y;
            while y >= self.min_y && self.get(bx, y, bz) == AIR {
                let stone = self.o.ids().stone;
                self.set(bx, y, bz, stone);
                y -= 1;
            }
        }
    }

    fn noise_f32(&self, idx: usize, x: f64, y: f64, z: f64) -> f32 {
        self.o.tree().noises[idx].get_value(x, y, z)
    }

    /// `MaterialSystem.frozenOceanExtension`.
    fn frozen_ocean(&mut self, min_surface: i32, biome: u16, bx: i32, bz: i32, height: i32) {
        let n = self.o.noises;
        let sea = self.o.sea_level();
        let iceberg_surface = self.noise(n.iceberg_surface, bx as f64, 0.0, bz as f64) * 8.25;
        let iceberg_pillar = (self.noise_f32(n.iceberg_pillar, bx as f64 * 1.28, 0.0, bz as f64 * 1.28) * 15.0f32) as f64;
        let iceberg = iceberg_surface.abs().min(iceberg_pillar);
        if iceberg <= 1.8 {
            return;
        }
        let roof = (self.noise(n.iceberg_pillar_roof, bx as f64 * 1.17, 0.0, bz as f64 * 1.17) * 1.5).abs();
        let mut top = (iceberg * iceberg * 1.2).min((roof * 40.0).ceil() + 14.0);
        if self.o.biome_source().melts_iceberg(biome, bx, sea, bz, sea) {
            top -= 2.0;
        }
        if top <= 2.0 {
            return;
        }
        let extension_bottom = sea as f64 - top - 7.0;
        top += sea as f64;
        let extension_top = top;
        let mut random = self.o.root_random().at(bx, 0, bz);
        let max_snow_depth = 2 + random.next_int_bound(4);
        let min_snow_height = sea + 18 + random.next_int_bound(10);
        let mut snow_depth = 0;
        let start = height.max(extension_top as i32 + 1);
        let mut y = start;
        while y >= min_surface {
            let cond1 = self.get(bx, y, bz) == AIR && (y as f64) < extension_top && random.next_double() > 0.01;
            let cond2 = !cond1
                && self.o.interner().is_block(self.get(bx, y, bz), "minecraft:water")
                && (y as f64) > extension_bottom
                && y < sea
                && random.next_double() > 0.15;
            if cond1 || cond2 {
                if snow_depth <= max_snow_depth && y > min_snow_height {
                    let snow = self.o.ids().snow_block;
                    self.set(bx, y, bz, snow);
                    snow_depth += 1;
                } else {
                    let ice = self.o.ids().packed_ice;
                    self.set(bx, y, bz, ice);
                }
            }
            y -= 1;
        }
    }
}

/// Runs the surface step over a chunk whose noise fill is already in `blocks` / `heights`.
pub(crate) fn build<'a>(
    o: &'a Overworld,
    s: Samplers<'a>,
    biomes: &BiomeMap<'a>,
    cx: i32,
    cz: i32,
    blocks: &mut Vec<Blk>,
    heights: &mut [i32; 256],
) {
    let min_y = o.min_y();
    let max_y = min_y + o.height() - 1;
    // Highest filled section decides the narrowed volume used by the ore veins.
    let top = (min_y..=max_y)
        .rev()
        .find(|&y| {
            let rel = y - min_y;
            (0..256).any(|c| blocks[(rel as usize) * 256 + c] != AIR)
        });
    let max_block_y = match top {
        Some(t) => (t >> 4) * 16 + 15,
        None => ((min_y >> 4) - 1) * 16 + 15,
    };
    let size_y = (max_block_y - min_y + 1).max(1);
    let narrow = Vol::new(16, size_y, 16, cx * 16, min_y, cz * 16);
    let prelim_vol = Vol::new(16, 1, 16, cx * 16, 0, cz * 16);

    let mut e = Eval {
        o,
        s,
        biomes,
        cx,
        cz,
        min_y,
        max_y,
        blocks,
        heights,
        narrow,
        prelim_vol,
        prelim: None,
        ore: HashMap::new(),
        block_x: 0,
        block_z: 0,
        grad_x: 0,
        grad_z: 0,
        surface_depth: 0,
        min_surface: None,
        block_y: 0,
        stone_above: 0,
        stone_below: 0,
        water_height: i32::MIN,
        biome_memo: Cell::new((i32::MIN, i32::MIN, i32::MIN, 0)),
        noise2d_memo: RefCell::new(Vec::new()),
    };
    let root = o.root_rule;
    let frozen = [biome_id(o, "minecraft:frozen_ocean"), biome_id(o, "minecraft:deep_frozen_ocean")];
    let eroded = biome_id(o, "minecraft:eroded_badlands");

    for lx in 0..16 {
        for lz in 0..16 {
            let bx = cx * 16 + lx;
            let bz = cz * 16 + lz;
            let starting = e.height(lx, lz);
            let surface_biome = e.biomes.fiddled(bx, starting, bz);
            if Some(surface_biome) == eroded {
                e.eroded_badlands(bx, bz, starting);
            }
            let height = e.height(lx, lz);
            let gx = e.height((lx + 1).min(15), lz) - e.height((lx - 1).max(0), lz);
            let gz = e.height(lx, (lz + 1).min(15)) - e.height(lx, (lz - 1).max(0));
            e.update_xz(bx, bz, gx, gz);

            let mut stone_above = 0;
            let mut water_height = i32::MIN;
            let mut next_ceiling = i32::MAX;
            let end_y = min_y;
            let mut y = height;
            while y >= end_y {
                let old = e.get(bx, y, bz);
                if old == AIR {
                    stone_above = 0;
                    water_height = i32::MIN;
                } else if o.interner().is_fluid(old) {
                    if water_height == i32::MIN {
                        water_height = y + 1;
                    }
                } else {
                    if next_ceiling >= y {
                        next_ceiling = WAY_BELOW_MIN_Y;
                        let mut la = y - 1;
                        while la >= end_y - 1 {
                            let ns = e.get(bx, la, bz);
                            if !is_stone(o, ns) {
                                next_ceiling = la + 1;
                                break;
                            }
                            la -= 1;
                        }
                    }
                    stone_above += 1;
                    let stone_below = y - next_ceiling + 1;
                    e.update_y(stone_above, stone_below, water_height, y);
                    if y >= min_y && y <= max_y {
                        if let Some(st) = e.eval_rule(root, bx, y, bz) {
                            e.set(bx, y, bz, st);
                        }
                    }
                }
                y -= 1;
            }

            if Some(surface_biome) == frozen[0] || Some(surface_biome) == frozen[1] {
                let min_surface = e.min_surface_level();
                e.frozen_ocean(min_surface, surface_biome, bx, bz, starting);
            }
        }
    }
}

/// `MaterialSystem.topMaterial` (vanilla 26.3): the surface rule evaluated at one block `(bx, by, bz)` of
/// chunk `(cx, cz)`, used by the carver step to turn the dirt under a carved grass opening into the
/// top material. Gradients read the live WORLD_SURFACE_WG heights in `heights`, and the rule context is
/// fresh (one-block volume), like the `MaterialRuleContext` built by the call.
pub(crate) fn top_material<'a>(
    o: &'a Overworld,
    s: Samplers<'a>,
    biomes: &BiomeMap<'a>,
    cx: i32,
    cz: i32,
    blocks: &mut Vec<Blk>,
    heights: &mut [i32; 256],
    bx: i32,
    by: i32,
    bz: i32,
    under_fluid: bool,
) -> Option<Blk> {
    let min_y = o.min_y();
    let max_y = min_y + o.height() - 1;
    let mut e = Eval {
        o,
        s,
        biomes,
        cx,
        cz,
        min_y,
        max_y,
        blocks,
        heights,
        narrow: Vol::new(1, 1, 1, bx, by, bz),
        prelim_vol: Vol::new(16, 1, 16, cx * 16, 0, cz * 16),
        prelim: None,
        ore: HashMap::new(),
        block_x: 0,
        block_z: 0,
        grad_x: 0,
        grad_z: 0,
        surface_depth: 0,
        min_surface: None,
        block_y: 0,
        stone_above: 0,
        stone_below: 0,
        water_height: i32::MIN,
        biome_memo: Cell::new((i32::MIN, i32::MIN, i32::MIN, 0)),
        noise2d_memo: RefCell::new(Vec::new()),
    };
    let lx = bx - cx * 16;
    let lz = bz - cz * 16;
    let gx = e.height((lx + 1).min(15), lz) - e.height((lx - 1).max(0), lz);
    let gz = e.height(lx, (lz + 1).min(15)) - e.height(lx, (lz - 1).max(0));
    e.update_xz(bx, bz, gx, gz);
    e.update_y(1, 1, if under_fluid { by + 1 } else { i32::MIN }, by);
    e.eval_rule(o.root_rule, bx, by, bz)
}
