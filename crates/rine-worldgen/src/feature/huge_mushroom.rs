//! `HugeRedMushroomFeature` and `HugeBrownMushroomFeature` (`minecraft:huge_red_mushroom`,
//! `minecraft:huge_brown_mushroom`), with `AbstractHugeMushroomFeature.place`.
//!
//! Cap and stem providers must be simple states. Cap states are interned once per property
//! combination at compile time (`HugeMushroomBlock` UP, WEST, EAST, NORTH, SOUTH for red, the four
//! horizontal ones for brown), so placement reads a table and never sets a property at run time.
//! Writes use `Feature.setBlock` (`setBlockAndUpdate`); the caller decides the update flags of the
//! replay.

use serde_json::Value;

use super::predicate::{self, BlockPredicate};
use super::region::Region;
use super::rng::WorldgenRng;
use super::state::Palette;
use super::state_provider::{self, StateProvider};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Red,
    Brown,
}

pub struct HugeMushroom {
    kind: Kind,
    /// Cap state per property mask (bit order in `mask`).
    caps: Vec<u16>,
    stem: u16,
    foliage_radius: i32,
    can_place_on: BlockPredicate,
    /// Tag ids: `minecraft:leaves`, `minecraft:replaceable_by_mushrooms`.
    leaves: u16,
    replaceable: u16,
}

/// Cap properties, bit `i` of the mask is the property `i` of the list.
const RED_PROPS: [&str; 5] = ["up", "west", "east", "north", "south"];
const BROWN_PROPS: [&str; 4] = ["west", "east", "north", "south"];

fn mask(bits: &[bool]) -> usize {
    bits.iter().enumerate().fold(0, |m, (i, &b)| m | ((b as usize) << i))
}

/// `Palette.with_property` chained over the list. Vanilla keeps the state when a property is
/// missing, so any missing property leaves the base state.
fn cap_variant(pal: &mut Palette, base: u16, props: &[&str], mask_value: usize) -> u16 {
    let mut s = base;
    for (i, p) in props.iter().enumerate() {
        let v = if mask_value >> i & 1 == 1 { "true" } else { "false" };
        match pal.with_property(s, p, v) {
            Some(n) => s = n,
            None => return base,
        }
    }
    s
}

fn simple(p: StateProvider, what: &str) -> Result<u16, String> {
    match p {
        StateProvider::Simple(s) => Ok(s),
        _ => Err(format!("{what} of a huge mushroom must be a simple state")),
    }
}

/// `TreeFeature`-style lookup of a tag id in the sorted bundle tag names.
fn tag_id(tag_names: &[String], name: &str) -> Result<u16, String> {
    tag_names.binary_search_by(|t| t.as_str().cmp(name)).map(|i| i as u16).map_err(|_| format!("tag {name} not bundled"))
}

impl HugeMushroom {
    /// Compiles `minecraft:huge_red_mushroom` or `minecraft:huge_brown_mushroom` from its configured JSON.
    pub fn compile(kind: Kind, cfg: &Value, tag_names: &[String], pal: &mut Palette) -> Result<HugeMushroom, String> {
        let cap = simple(state_provider::parse(&cfg["cap_provider"], tag_names, pal)?, "cap_provider")?;
        let stem = simple(state_provider::parse(&cfg["stem_provider"], tag_names, pal)?, "stem_provider")?;
        let foliage_radius = cfg["foliage_radius"].as_i64().unwrap_or(2) as i32;
        let can_place_on = predicate::parse(&cfg["can_place_on"], tag_names, pal)?;
        let props: &[&str] = match kind {
            Kind::Red => &RED_PROPS,
            Kind::Brown => &BROWN_PROPS,
        };
        let caps = (0..1usize << props.len()).map(|m| cap_variant(pal, cap, props, m)).collect();
        Ok(HugeMushroom {
            kind,
            caps,
            stem,
            foliage_radius,
            can_place_on,
            leaves: tag_id(tag_names, "minecraft:leaves")?,
            replaceable: tag_id(tag_names, "minecraft:replaceable_by_mushrooms")?,
        })
    }

    /// `foliageRadius()` (the bone meal height check reads it).
    pub fn foliage_radius(&self) -> i32 {
        self.foliage_radius
    }

    /// `AbstractHugeMushroomFeature.place`: height draw, validity, cap, trunk.
    pub fn place(&self, region: &mut Region, rng: &mut WorldgenRng, origin: (i32, i32, i32)) -> bool {
        // `getTreeHeight`: nextInt(3) + 4, doubled on 1 in 12
        let mut tree_height = rng.next_int_bound(3) + 4;
        if rng.next_int_bound(12) == 0 {
            tree_height *= 2;
        }
        if !self.valid_position(region, origin, tree_height) {
            return false;
        }
        self.make_cap(region, origin, tree_height);
        self.place_trunk(region, origin, tree_height);
        true
    }

    /// `isValidPosition`: build height, `canPlaceOn` below, and air or leaves in the cap's column box.
    fn valid_position(&self, region: &Region, origin: (i32, i32, i32), tree_height: i32) -> bool {
        let (x, y, z) = origin;
        let max_y = region.min_y() + region.height() - 1;
        if y < region.min_y() + 1 || y + tree_height + 1 > max_y {
            return false;
        }
        if !self.can_place_on.test(region, x, y - 1, z) {
            return false;
        }
        for dy in 0..=tree_height {
            let radius = match self.kind {
                // `getTreeRadiusForHeight(-1, -1, ...)`: the red feature passes -1 as its tree height,
                // so no row of the probe box has a radius.
                Kind::Red => red_radius(-1, self.foliage_radius, dy),
                Kind::Brown => brown_radius(self.foliage_radius, dy),
            };
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    let s = region.get_block(x + dx, y + dy, z + dz);
                    if !region.tags.is_air(s) && !region.tags.has(s, self.leaves) {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// `placeMushroomBlock`: only air and `replaceable_by_mushrooms` take a mushroom block.
    fn place_mushroom(&self, region: &mut Region, x: i32, y: i32, z: i32, st: u16) {
        let cur = region.get_block(x, y, z);
        if region.tags.is_air(cur) || region.tags.has(cur, self.replaceable) {
            region.set_block(x, y, z, st);
        }
    }

    /// `makeCap`: the red cap is three layers of rings and a top disc, the brown cap one flat layer.
    fn make_cap(&self, region: &mut Region, origin: (i32, i32, i32), tree_height: i32) {
        let (ox, oy, oz) = origin;
        let fr = self.foliage_radius;
        match self.kind {
            Kind::Red => {
                let center = fr - 2;
                for dy in (tree_height - 3)..=tree_height {
                    let radius = if dy < tree_height { fr } else { fr - 1 };
                    for dx in -radius..=radius {
                        for dz in -radius..=radius {
                            let min_x = dx == -radius;
                            let max_x = dx == radius;
                            let min_z = dz == -radius;
                            let max_z = dz == radius;
                            let x_edge = min_x || max_x;
                            let z_edge = min_z || max_z;
                            if dy >= tree_height || x_edge != z_edge {
                                let bits = [dy >= tree_height - 1, dx < -center, dx > center, dz < -center, dz > center];
                                let st = self.caps[mask(&bits)];
                                self.place_mushroom(region, ox + dx, oy + dy, oz + dz, st);
                            }
                        }
                    }
                }
            }
            Kind::Brown => {
                for dx in -fr..=fr {
                    for dz in -fr..=fr {
                        let min_x = dx == -fr;
                        let max_x = dx == fr;
                        let min_z = dz == -fr;
                        let max_z = dz == fr;
                        let x_edge = min_x || max_x;
                        let z_edge = min_z || max_z;
                        if !x_edge || !z_edge {
                            let west = min_x || (z_edge && dx == 1 - fr);
                            let east = max_x || (z_edge && dx == fr - 1);
                            let north = min_z || (x_edge && dz == 1 - fr);
                            let south = max_z || (x_edge && dz == fr - 1);
                            let st = self.caps[mask(&[west, east, north, south])];
                            self.place_mushroom(region, ox + dx, oy + tree_height, oz + dz, st);
                        }
                    }
                }
            }
        }
    }

    /// `placeTrunk`: `treeHeight` stems from the origin up.
    fn place_trunk(&self, region: &mut Region, origin: (i32, i32, i32), tree_height: i32) {
        let (x, y, z) = origin;
        for dy in 0..tree_height {
            self.place_mushroom(region, x, y + dy, z, self.stem);
        }
    }
}

/// `HugeRedMushroomFeature.getTreeRadiusForHeight`.
fn red_radius(tree_height: i32, leaf_radius: i32, yo: i32) -> i32 {
    if yo < tree_height && yo >= tree_height - 3 {
        leaf_radius
    } else if yo == tree_height {
        leaf_radius
    } else {
        0
    }
}

/// `HugeBrownMushroomFeature.getTreeRadiusForHeight`.
fn brown_radius(leaf_radius: i32, yo: i32) -> i32 {
    if yo <= 3 {
        0
    } else {
        leaf_radius
    }
}
