//! Biome list and climate classification. Inputs are interpolated noise values;
//! the output is one of the 23 biomes listed in the generator spec.

use super::noise::{hash, unit, SALT_BIOME};
use crate::gen::terrain::SEA_LEVEL;

pub const BIOME_COUNT: usize = 23;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Biome {
    Plains,
    SunflowerPlains,
    Forest,
    BirchForest,
    DarkForest,
    Taiga,
    SnowyPlains,
    SnowyTaiga,
    Desert,
    Savanna,
    Jungle,
    Swamp,
    Beach,
    SnowyBeach,
    Ocean,
    DeepOcean,
    ColdOcean,
    FrozenOcean,
    River,
    FrozenRiver,
    StonyPeaks,
    SnowySlopes,
    WindsweptHills,
}

impl Biome {
    pub const ALL: [Biome; BIOME_COUNT] = [
        Biome::Plains,
        Biome::SunflowerPlains,
        Biome::Forest,
        Biome::BirchForest,
        Biome::DarkForest,
        Biome::Taiga,
        Biome::SnowyPlains,
        Biome::SnowyTaiga,
        Biome::Desert,
        Biome::Savanna,
        Biome::Jungle,
        Biome::Swamp,
        Biome::Beach,
        Biome::SnowyBeach,
        Biome::Ocean,
        Biome::DeepOcean,
        Biome::ColdOcean,
        Biome::FrozenOcean,
        Biome::River,
        Biome::FrozenRiver,
        Biome::StonyPeaks,
        Biome::SnowySlopes,
        Biome::WindsweptHills,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Biome::Plains => "minecraft:plains",
            Biome::SunflowerPlains => "minecraft:sunflower_plains",
            Biome::Forest => "minecraft:forest",
            Biome::BirchForest => "minecraft:birch_forest",
            Biome::DarkForest => "minecraft:dark_forest",
            Biome::Taiga => "minecraft:taiga",
            Biome::SnowyPlains => "minecraft:snowy_plains",
            Biome::SnowyTaiga => "minecraft:snowy_taiga",
            Biome::Desert => "minecraft:desert",
            Biome::Savanna => "minecraft:savanna",
            Biome::Jungle => "minecraft:jungle",
            Biome::Swamp => "minecraft:swamp",
            Biome::Beach => "minecraft:beach",
            Biome::SnowyBeach => "minecraft:snowy_beach",
            Biome::Ocean => "minecraft:ocean",
            Biome::DeepOcean => "minecraft:deep_ocean",
            Biome::ColdOcean => "minecraft:cold_ocean",
            Biome::FrozenOcean => "minecraft:frozen_ocean",
            Biome::River => "minecraft:river",
            Biome::FrozenRiver => "minecraft:frozen_river",
            Biome::StonyPeaks => "minecraft:stony_peaks",
            Biome::SnowySlopes => "minecraft:snowy_slopes",
            Biome::WindsweptHills => "minecraft:windswept_hills",
        }
    }

    /// Water body whose surface is ice at sea level.
    pub fn is_frozen_water(self) -> bool {
        matches!(self, Biome::FrozenOcean | Biome::FrozenRiver)
    }

    /// Gets a one-layer snow cover on top of the ground.
    pub fn is_snowy(self) -> bool {
        matches!(self, Biome::SnowyPlains | Biome::SnowyTaiga | Biome::SnowyBeach)
    }
}

/// Picks a biome from interpolated climate values.
/// `s` is the ground surface y, `cont` continentalness, `mount` mountain mask (0..1),
/// `river` river strength (0..1), `t` temperature and `hu` humidity (both about -1..1).
#[allow(clippy::too_many_arguments)]
pub fn classify(seed: u64, x: i32, z: i32, s: i32, cont: f32, mount: f32, river: f32, t: f32, hu: f32) -> Biome {
    use Biome::*;
    // Region-scale variant hash (8x8 blocks) so variants form patches.
    let coin = unit(hash(seed, (x >> 3) as i64, (z >> 3) as i64, 0, SALT_BIOME));

    if river > 0.5 {
        return if t < -0.3 { FrozenRiver } else { River };
    }
    if s < SEA_LEVEL {
        return if cont < -0.42 {
            DeepOcean
        } else if t < -0.3 {
            FrozenOcean
        } else if t < -0.05 {
            ColdOcean
        } else {
            Ocean
        };
    }
    if s <= 66 && hu > 0.35 && t > -0.05 && cont < 0.15 {
        return Swamp;
    }
    if s <= 66 && cont < 0.1 {
        return if t < -0.3 { SnowyBeach } else { Beach };
    }
    if mount > 0.3 && s >= 150 {
        return StonyPeaks;
    }
    if mount > 0.3 && s >= 105 {
        return if t < -0.05 { SnowySlopes } else { WindsweptHills };
    }
    if mount > 0.12 && s >= 90 {
        return WindsweptHills;
    }

    if t < -0.3 {
        return if hu > 0.1 { SnowyTaiga } else { SnowyPlains };
    }
    if t < -0.05 {
        return if hu > 0.0 { Taiga } else { Plains };
    }
    if t < 0.25 {
        if hu > 0.45 {
            return if coin < 0.5 { DarkForest } else { Forest };
        }
        if hu > 0.2 {
            return if coin < 0.35 { BirchForest } else { Forest };
        }
        if hu > -0.15 {
            return if coin < 0.3 { Forest } else { Plains };
        }
        return if coin < 0.2 { SunflowerPlains } else { Plains };
    }
    if hu < -0.25 {
        return Desert;
    }
    if hu > 0.4 {
        return if t > 0.45 { Jungle } else { Forest };
    }
    Savanna
}
