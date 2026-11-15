//! Crops and other plants that grow by age: `CropBlock` (wheat, carrots,
//! potatoes), `BeetrootBlock`, `TorchflowerCropBlock`, `PitcherCropBlock`,
//! `StemBlock` / `AttachedStemBlock` (pumpkin, melon), `NetherWartBlock`,
//! `CocoaBlock`, `SweetBerryBushBlock`.
//!
//! Random tick growth and `updateShape` survival (`VegetationBlock`,
//! `DoublePlantBlock`, `CocoaBlock`). Bone meal entry: `bonemeal`.
//!
//! Vanilla rolls `random.nextInt((int)(25 / growthSpeed) + 1) == 0`; Paper
//! writes the same chance as `nextFloat() < 1 / (floor(25 / speed) + 1)`.

use super::props::{geti, getv, seti, setv, vid, Pr};
use super::{block_of, block_tag, default_state, raw_brightness, Beh, Rng};
use crate::game::redstone::data::{self, step, DOWN, HORIZONTAL, UP};
use crate::game::redstone::engine::{self, fl, Lvl, UPDATE_LIMIT};
use crate::game::redstone::Rs;
use rine_protocol::BlockPos;
use rine_world::MAX_Y;
use std::sync::OnceLock;

struct T {
    supports_crops: Vec<bool>,
    grows_crops: Vec<bool>,
    supports_stem: Vec<bool>,
    supports_stem_fruit: Vec<bool>,
    supports_wart: Vec<bool>,
    supports_cocoa: Vec<bool>,
    supports_berry: Vec<bool>,
    farmland: u16,
    beetroots: u16,
    torchflower_crop: u16,
    torchflower: u16,
    pitcher: u16,
    pumpkin_stem: u16,
    melon_stem: u16,
    attached_pumpkin: u16,
    attached_melon: u16,
    pumpkin: u16,
    melon: u16,
    v_lower: u8,
    v_upper: u8,
}

fn t() -> &'static T {
    static T_: OnceLock<T> = OnceLock::new();
    T_.get_or_init(|| T {
        supports_crops: block_tag("supports_crops"),
        grows_crops: block_tag("grows_crops"),
        supports_stem: block_tag("supports_stem_crops"),
        supports_stem_fruit: block_tag("supports_stem_fruit"),
        supports_wart: block_tag("supports_nether_wart"),
        supports_cocoa: block_tag("supports_cocoa"),
        supports_berry: block_tag("supports_vegetation"),
        farmland: super::block_id("farmland"),
        beetroots: super::block_id("beetroots"),
        torchflower_crop: super::block_id("torchflower_crop"),
        torchflower: super::block_id("torchflower"),
        pitcher: super::block_id("pitcher_crop"),
        pumpkin_stem: super::block_id("pumpkin_stem"),
        melon_stem: super::block_id("melon_stem"),
        attached_pumpkin: super::block_id("attached_pumpkin_stem"),
        attached_melon: super::block_id("attached_melon_stem"),
        pumpkin: super::block_id("pumpkin"),
        melon: super::block_id("melon"),
        v_lower: vid("lower"),
        v_upper: vid("upper"),
    })
}

pub fn classify(n: &str) -> Option<Beh> {
    Some(match n {
        "wheat" | "carrots" | "potatoes" | "beetroots" | "torchflower_crop" => Beh::Crop,
        "pumpkin_stem" | "melon_stem" | "attached_pumpkin_stem" | "attached_melon_stem" => Beh::Stem,
        "nether_wart" => Beh::NetherWart,
        "cocoa" => Beh::Cocoa,
        "sweet_berry_bush" => Beh::SweetBerry,
        "pitcher_crop" => Beh::PitcherCrop,
        _ => return None,
    })
}

#[inline]
fn below(p: BlockPos) -> BlockPos {
    p.offset(0, -1, 0)
}

/// Max age of a crop block (`getMaxAge`).
fn max_age(blk: u16) -> i32 {
    let t = t();
    if blk == t.beetroots {
        3
    } else if blk == t.torchflower_crop {
        2
    } else {
        7
    }
}

/// `CropBlock.getStateForAge`: the torchflower crop becomes a torchflower.
fn crop_state_for_age(st: u16, age: i32) -> u16 {
    let t = t();
    if block_of(st) == t.torchflower_crop && age == 2 {
        return default_state("torchflower");
    }
    seti(st, Pr::Age, age)
}

/// `CropBlock.getGrowthSpeed`.
pub fn growth_speed<L: Lvl>(l: &L, blk: u16, pos: BlockPos) -> f32 {
    let t = t();
    let mut speed = 1.0f32;
    let b = below(pos);
    for xx in -1..=1 {
        for zz in -1..=1 {
            let st = l.get(b.offset(xx, 0, zz));
            let mut s = 0.0f32;
            if t.grows_crops[block_of(st) as usize] {
                s = 1.0;
                if geti(st, Pr::Moisture) > 0 {
                    s = 3.0;
                }
            }
            if xx != 0 || zz != 0 {
                s /= 4.0;
            }
            speed += s;
        }
    }
    let is = |dx: i32, dz: i32| block_of(l.get(pos.offset(dx, 0, dz))) == blk;
    let horizontal = is(-1, 0) || is(1, 0);
    let vertical = is(0, -1) || is(0, 1);
    if horizontal && vertical {
        speed /= 2.0;
    } else if is(-1, -1) || is(1, -1) || is(1, 1) || is(-1, 1) {
        speed /= 2.0;
    }
    speed
}

/// `nextInt((int)(25 / speed) + 1) == 0`.
#[inline]
fn grow_roll(rng: &mut Rng, speed: f32) -> bool {
    rng.next_int((25.0f32 / speed) as i32 + 1) == 0
}

fn set_clients<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    engine::set_block(rs, l, pos, st, fl::CLIENTS, UPDATE_LIMIT);
}

fn set_all<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    engine::set_block(rs, l, pos, st, fl::ALL, UPDATE_LIMIT);
}

pub fn random_tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    let t = t();
    let blk = block_of(st);
    match super::beh(st) {
        Beh::Crop => {
            // beetroots and the torchflower crop skip two thirds of their ticks
            if (blk == t.beetroots || blk == t.torchflower_crop) && rng.next_int(3) == 0 {
                return;
            }
            if raw_brightness(l, pos) >= 9 {
                let age = geti(st, Pr::Age);
                if age < max_age(blk) && grow_roll(rng, growth_speed(l, blk, pos)) {
                    set_clients(rs, l, pos, crop_state_for_age(st, age + 1));
                }
            }
        }
        Beh::Stem => {
            if blk != t.pumpkin_stem && blk != t.melon_stem {
                return; // attached stems do not tick
            }
            if raw_brightness(l, pos) < 9 || !grow_roll(rng, growth_speed(l, blk, pos)) {
                return;
            }
            let age = geti(st, Pr::Age);
            if age < 7 {
                set_clients(rs, l, pos, seti(st, Pr::Age, age + 1));
            } else {
                stem_fruit(rs, l, blk, pos, rng);
            }
        }
        Beh::NetherWart => {
            let age = geti(st, Pr::Age);
            if age < 3 && rng.next_int(10) == 0 {
                set_clients(rs, l, pos, seti(st, Pr::Age, age + 1));
            }
        }
        Beh::Cocoa => {
            if rng.next_int(5) == 0 {
                let age = geti(st, Pr::Age);
                if age < 2 {
                    set_clients(rs, l, pos, seti(st, Pr::Age, age + 1));
                }
            }
        }
        Beh::SweetBerry => {
            let age = geti(st, Pr::Age);
            if age < 3 && rng.next_int(5) == 0 && raw_brightness(l, pos.offset(0, 1, 0)) >= 9 {
                set_clients(rs, l, pos, seti(st, Pr::Age, age + 1));
            }
        }
        Beh::PitcherCrop => {
            if grow_roll(rng, growth_speed(l, blk, pos)) {
                pitcher_grow(rs, l, st, pos, 1);
            }
        }
        _ => {}
    }
}

/// A fully grown stem tries to place its fruit on a random side.
fn stem_fruit<L: Lvl>(rs: &mut Rs, l: &mut L, blk: u16, pos: BlockPos, rng: &mut Rng) {
    let t = t();
    // `Direction.Plane.HORIZONTAL.getRandomDirection`: north, east, south, west
    let d = HORIZONTAL[rng.next_int(4) as usize];
    let (dx, dy, dz) = step(d);
    let rel = pos.offset(dx, dy, dz);
    if !super::is_air(l.get(rel)) || !t.supports_stem_fruit[block_of(l.get(below(rel))) as usize] {
        return;
    }
    let (fruit, attached) = if blk == t.pumpkin_stem { ("pumpkin", "attached_pumpkin_stem") } else { ("melon", "attached_melon_stem") };
    set_all(rs, l, rel, default_state(fruit));
    let a = data::set(default_state(attached), data::P::Facing, d);
    set_all(rs, l, pos, a);
}

fn pitcher_is_lower(st: u16) -> bool {
    let t = t();
    block_of(st) == t.pitcher && getv(st, Pr::Half) == t.v_lower
}

/// `PitcherCropBlock.canGrow`.
fn pitcher_can_grow<L: Lvl>(l: &L, lower: BlockPos, st: u16, new_age: i32) -> bool {
    let t = t();
    geti(st, Pr::Age) < 4
        && raw_brightness(l, lower) >= 8
        && lower.y < MAX_Y
        && (new_age < 3 || {
            let up = l.get(lower.offset(0, 1, 0));
            super::is_air(up) || block_of(up) == t.pitcher
        })
}

/// `PitcherCropBlock.grow`.
fn pitcher_grow<L: Lvl>(rs: &mut Rs, l: &mut L, lower_st: u16, lower: BlockPos, inc: i32) {
    let age = (geti(lower_st, Pr::Age) + inc).min(4);
    if !pitcher_can_grow(l, lower, lower_st, age) {
        return;
    }
    let ns = seti(lower_st, Pr::Age, age);
    set_clients(rs, l, lower, ns);
    if age >= 3 {
        set_all(rs, l, lower.offset(0, 1, 0), setv(ns, Pr::Half, t().v_upper));
    }
}

/// `canSurvive` of the plants in this family.
fn can_survive<L: Lvl>(l: &L, st: u16, pos: BlockPos) -> bool {
    let t = t();
    let blk = block_of(st);
    let on = |tag: &Vec<bool>| tag[block_of(l.get(below(pos))) as usize];
    match super::beh(st) {
        Beh::Crop => raw_brightness(l, pos) >= 8 && on(&t.supports_crops),
        Beh::Stem => on(&t.supports_stem),
        Beh::NetherWart => on(&t.supports_wart),
        Beh::SweetBerry => on(&t.supports_berry),
        Beh::Cocoa => {
            let (dx, dy, dz) = step(data::get(st, data::P::Facing));
            t.supports_cocoa[block_of(l.get(pos.offset(dx, dy, dz))) as usize]
        }
        Beh::PitcherCrop => {
            if getv(st, Pr::Half) == t.v_upper {
                // `DoublePlantBlock.canSurvive` of the upper half
                let b = l.get(below(pos));
                block_of(b) == blk && getv(b, Pr::Half) == t.v_lower
            } else {
                raw_brightness(l, pos) >= 8 && on(&t.supports_crops)
            }
        }
        _ => true,
    }
}

pub fn update_shape<L: Lvl>(_rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, dir: u8, _npos: BlockPos, nst: u16) -> u16 {
    let t = t();
    let air = 0;
    let blk = block_of(st);
    match super::beh(st) {
        Beh::Stem if blk == t.attached_pumpkin || blk == t.attached_melon => {
            // `AttachedStemBlock.updateShape`: the fruit went away
            let fruit = if blk == t.attached_pumpkin { t.pumpkin } else { t.melon };
            if block_of(nst) != fruit && dir == data::get(st, data::P::Facing) {
                let stem = if blk == t.attached_pumpkin { "pumpkin_stem" } else { "melon_stem" };
                return seti(default_state(stem), Pr::Age, 7);
            }
            if can_survive(l, st, pos) {
                st
            } else {
                air
            }
        }
        Beh::Cocoa => {
            if dir == data::get(st, data::P::Facing) && !can_survive(l, st, pos) {
                air
            } else {
                st
            }
        }
        Beh::PitcherCrop if geti(st, Pr::Age) >= 3 => {
            // `DoublePlantBlock.updateShape`
            let half_lower = getv(st, Pr::Half) == t.v_lower;
            let vertical = dir == UP || dir == DOWN;
            if !vertical || half_lower != (dir == UP) || (block_of(nst) == blk && getv(nst, Pr::Half) != getv(st, Pr::Half)) {
                if half_lower && dir == DOWN && !can_survive(l, st, pos) {
                    air
                } else if can_survive(l, st, pos) {
                    st
                } else {
                    air
                }
            } else {
                air
            }
        }
        _ => {
            // `VegetationBlock.updateShape`
            if can_survive(l, st, pos) {
                st
            } else {
                air
            }
        }
    }
}

/// Bone meal on a block of this family (`isValidBonemealTarget` +
/// `performBonemeal`). Returns false when it is not a valid target.
pub fn bonemeal<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) -> bool {
    let t = t();
    let blk = block_of(st);
    match super::beh(st) {
        Beh::Crop => {
            let max = max_age(blk);
            let age = geti(st, Pr::Age);
            if age >= max {
                return false;
            }
            let inc = if blk == t.torchflower_crop {
                1
            } else if blk == t.beetroots {
                rng.range(2, 5) / 3
            } else {
                rng.range(2, 5)
            };
            set_clients(rs, l, pos, crop_state_for_age(st, (age + inc).min(max)));
            true
        }
        Beh::Stem if blk == t.pumpkin_stem || blk == t.melon_stem => {
            let age = geti(st, Pr::Age);
            if age == 7 {
                return false;
            }
            let na = (age + rng.range(2, 5)).min(7);
            set_clients(rs, l, pos, seti(st, Pr::Age, na));
            if na == 7 {
                // `newState.randomTick(level, pos, random)`
                if raw_brightness(l, pos) >= 9 && grow_roll(rng, growth_speed(l, blk, pos)) {
                    stem_fruit(rs, l, blk, pos, rng);
                }
            }
            true
        }
        Beh::Cocoa => {
            let age = geti(st, Pr::Age);
            if age >= 2 {
                return false;
            }
            set_clients(rs, l, pos, seti(st, Pr::Age, age + 1));
            true
        }
        Beh::SweetBerry => {
            let age = geti(st, Pr::Age);
            if age >= 3 {
                return false;
            }
            set_clients(rs, l, pos, seti(st, Pr::Age, (age + 1).min(3)));
            true
        }
        Beh::PitcherCrop => {
            let (lp, ls) = if pitcher_is_lower(st) {
                (pos, st)
            } else {
                let b = below(pos);
                let bs = l.get(b);
                if !pitcher_is_lower(bs) {
                    return false;
                }
                (b, bs)
            };
            if !pitcher_can_grow(l, lp, ls, geti(ls, Pr::Age) + 1) {
                return false;
            }
            pitcher_grow(rs, l, ls, lp, 1);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::testw::World as TW;

    fn s(n: &str) -> u16 {
        default_state(n)
    }

    #[test]
    fn wheat_grows_on_lit_farmland_and_not_in_the_dark() {
        let mut w = TW::new();
        let p = BlockPos::new(0, 64, 0);
        w.put(below(p), s("farmland"));
        w.put(p, s("wheat"));
        w.light = 15;
        let mut rs = Rs::default();
        let mut rng = Rng::new(3);
        let mut grew = 0;
        for _ in 0..2000 {
            let st = w.get(p);
            random_tick(&mut rs, &mut w, st, p, &mut rng);
            grew = geti(w.get(p), Pr::Age);
        }
        assert_eq!(grew, 7);
        // a fresh crop in darkness never grows
        let mut w = TW::new();
        w.put(below(p), s("farmland"));
        w.put(p, s("wheat"));
        w.light = 8;
        for _ in 0..2000 {
            let st = w.get(p);
            random_tick(&mut rs, &mut w, st, p, &mut rng);
        }
        assert_eq!(geti(w.get(p), Pr::Age), 0);
    }

    #[test]
    fn growth_speed_matches_vanilla_layouts() {
        let mut w = TW::new();
        let p = BlockPos::new(0, 64, 0);
        let wet = seti(s("farmland"), Pr::Moisture, 7);
        for x in -1..=1 {
            for z in -1..=1 {
                w.put(BlockPos::new(x, 63, z), wet);
            }
        }
        let wheat = block_of(s("wheat"));
        // 1 + 3 (center) + 8 * 3/4 = 10
        assert_eq!(growth_speed(&w, wheat, p), 10.0);
        // a row of wheat east and west only: no penalty
        w.put(p.offset(1, 0, 0), s("wheat"));
        w.put(p.offset(-1, 0, 0), s("wheat"));
        assert_eq!(growth_speed(&w, wheat, p), 10.0);
        // also north: halved
        w.put(p.offset(0, 0, -1), s("wheat"));
        assert_eq!(growth_speed(&w, wheat, p), 5.0);
        // dry farmland, alone: 1 + 1 + 8/4
        let mut w = TW::new();
        for x in -1..=1 {
            for z in -1..=1 {
                w.put(BlockPos::new(x, 63, z), s("farmland"));
            }
        }
        assert_eq!(growth_speed(&w, wheat, p), 4.0);
        // a diagonal neighbour halves too
        w.put(p.offset(1, 0, 1), s("wheat"));
        assert_eq!(growth_speed(&w, wheat, p), 2.0);
    }

    #[test]
    fn torchflower_crop_becomes_torchflower() {
        let mut w = TW::new();
        let p = BlockPos::new(0, 64, 0);
        w.put(below(p), s("farmland"));
        w.put(p, s("torchflower_crop"));
        w.light = 15;
        let mut rs = Rs::default();
        let mut rng = Rng::new(9);
        for _ in 0..5000 {
            let st = w.get(p);
            if super::super::beh(st) != Beh::Crop {
                break;
            }
            random_tick(&mut rs, &mut w, st, p, &mut rng);
        }
        assert_eq!(w.get(p), s("torchflower"));
    }

    #[test]
    fn ripe_stem_places_fruit_and_attaches() {
        let mut w = TW::new();
        let p = BlockPos::new(0, 64, 0);
        for x in -1..=1 {
            for z in -1..=1 {
                w.put(BlockPos::new(x, 63, z), s("dirt"));
            }
        }
        w.put(below(p), s("farmland"));
        w.put(p, seti(s("pumpkin_stem"), Pr::Age, 7));
        w.light = 15;
        let mut rs = Rs::default();
        let mut rng = Rng::new(5);
        for _ in 0..5000 {
            let st = w.get(p);
            if block_of(st) != block_of(s("pumpkin_stem")) {
                break;
            }
            random_tick(&mut rs, &mut w, st, p, &mut rng);
        }
        let st = w.get(p);
        assert_eq!(block_of(st), block_of(s("attached_pumpkin_stem")));
        let d = data::get(st, data::P::Facing);
        let (dx, _, dz) = step(d);
        assert_eq!(w.get(p.offset(dx, 0, dz)), s("pumpkin"));
        // breaking the pumpkin turns the stem back into a ripe stem
        engine::set_block(&mut rs, &mut w, p.offset(dx, 0, dz), 0, fl::ALL, UPDATE_LIMIT);
        assert_eq!(w.get(p), seti(s("pumpkin_stem"), Pr::Age, 7));
    }

    #[test]
    fn crop_breaks_when_farmland_goes() {
        let mut w = TW::new();
        let p = BlockPos::new(0, 64, 0);
        w.put(below(p), s("farmland"));
        w.put(p, seti(s("wheat"), Pr::Age, 3));
        w.light = 15;
        let mut rs = Rs::default();
        engine::set_block(&mut rs, &mut w, below(p), s("dirt"), fl::ALL, UPDATE_LIMIT);
        assert_eq!(w.get(p), 0);
    }

    #[test]
    fn bonemeal_grows_crops_by_two_to_five() {
        let mut w = TW::new();
        let p = BlockPos::new(0, 64, 0);
        w.put(below(p), s("farmland"));
        w.light = 15;
        let mut rs = Rs::default();
        let mut rng = Rng::new(11);
        for _ in 0..200 {
            w.put(p, s("carrots"));
            assert!(bonemeal(&mut rs, &mut w, s("carrots"), p, &mut rng));
            let a = geti(w.get(p), Pr::Age);
            assert!((2..=5).contains(&a), "{a}");
        }
        w.put(p, seti(s("carrots"), Pr::Age, 7));
        let cur = w.get(p);
        assert!(!bonemeal(&mut rs, &mut w, cur, p, &mut rng));
    }
}
