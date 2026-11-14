//! Phase 2: overworld density (noise router) for Minecraft 26.3, bit-exact with the vanilla server.
//!
//! `Terrain::new(seed)` compiles the bundled overworld density graph once. `router_point` runs the
//! uncached point path (`RandomState.sampleBlockValueUncached`). `chunk_density` runs the chunk path
//! that `NoiseBasedChunkGenerator.doFill` uses (`finalDensity.sampleVolume` with caches enabled).

pub mod assets;
pub mod build;
pub mod spline;
pub mod tree;
pub mod volume;

use std::collections::HashMap;

pub use tree::{Ctx, Tree};
use volume::Vol;

/// Router outputs that `router_point` can evaluate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RouterField {
    Temperature,
    Vegetation,
    Continents,
    Erosion,
    Depth,
    Ridges,
    FinalDensity,
    ChunkSurfaceLevel,
}

impl RouterField {
    fn key(self) -> &'static str {
        match self {
            RouterField::Temperature => "temperature",
            RouterField::Vegetation => "vegetation",
            RouterField::Continents => "continents",
            RouterField::Erosion => "erosion",
            RouterField::Depth => "depth",
            RouterField::Ridges => "ridges",
            RouterField::FinalDensity => "final_density",
            RouterField::ChunkSurfaceLevel => "chunk_surface_level",
        }
    }
}

/// Densities of one 16x16 chunk column over `[min_y, min_y + height)`.
///
/// Layout: `index = (y - min_y) + (x + z * 16) * height`, the same as the Java `DensityVolume`.
#[derive(Clone, Debug)]
pub struct ChunkDensity {
    pub min_y: i32,
    pub height: i32,
    values: Vec<f32>,
}

impl ChunkDensity {
    /// Raw final density at chunk-local `x`, `z` (0..16) and absolute block `y`.
    #[inline]
    pub fn density(&self, x: i32, y: i32, z: i32) -> f32 {
        self.values[((y - self.min_y) + (x + z * 16) * self.height) as usize]
    }

    /// `final density > 0`: the block is solid noise before aquifers and surface rules.
    #[inline]
    pub fn is_solid(&self, x: i32, y: i32, z: i32) -> bool {
        self.density(x, y, z) > 0.0
    }

    /// All values in layout order.
    pub fn values(&self) -> &[f32] {
        &self.values
    }
}

/// The compiled overworld terrain for one seed.
pub struct Terrain {
    tree: Tree,
    roots: HashMap<RouterField, u32>,
    min_y: i32,
    height: i32,
}

impl Terrain {
    /// Compiles the overworld noise router for `seed` from the bundled vanilla 26.3 data.
    pub fn new(seed: i64) -> Self {
        Self::try_new(seed).expect("bundled overworld density data is valid")
    }

    pub fn try_new(seed: i64) -> Result<Self, String> {
        let mut builder = build::Builder::new(seed);
        let settings: serde_json::Value =
            serde_json::from_str(build::bundled("noise_settings/overworld.json")?).map_err(|e| e.to_string())?;
        let router = &settings["noise_router"];
        let mut roots = HashMap::new();
        for field in [
            RouterField::Temperature,
            RouterField::Vegetation,
            RouterField::Continents,
            RouterField::Erosion,
            RouterField::Depth,
            RouterField::Ridges,
            RouterField::FinalDensity,
            RouterField::ChunkSurfaceLevel,
        ] {
            // Only the requested router entries are built (chunk_surface_level needs find_top_surface).
            let name = router[field.key()]
                .as_str()
                .ok_or_else(|| format!("router has no {}", field.key()))?;
            let id = builder.reference(name)?;
            roots.insert(field, id);
        }
        let noise = &settings["noise"];
        let min_y = noise["min_y"].as_i64().ok_or("min_y")? as i32;
        let height = noise["height"].as_i64().ok_or("height")? as i32;
        Ok(Self {
            tree: builder.into_tree(),
            roots,
            min_y,
            height,
        })
    }

    /// Bottom of the generated column (`noise_settings.min_y`).
    pub fn min_y(&self) -> i32 {
        self.min_y
    }

    /// Height of the generated column (`noise_settings.height`).
    pub fn height(&self) -> i32 {
        self.height
    }

    /// One router output at a block position, through the uncached point path.
    pub fn router_point(&self, field: RouterField, x: i32, y: i32, z: i32) -> f32 {
        let root = self.roots[&field];
        self.tree.value(root, &mut Ctx::uncached(), x, y, z)
    }

    /// Final density for chunk column `(cx, cz)` over the full generated height, as
    /// `NoiseBasedChunkGenerator.doFill` samples it.
    pub fn chunk_density(&self, cx: i32, cz: i32) -> ChunkDensity {
        let vol = Vol::new(16, self.height, 16, cx * 16, self.min_y, cz * 16);
        let mut values = vec![0f32; vol.size()];
        let mut ctx = Ctx::cached(self.tree.cache_count);
        self.tree.volume(self.roots[&RouterField::FinalDensity], &mut ctx, &vol, &mut values);
        ChunkDensity {
            min_y: self.min_y,
            height: self.height,
            values,
        }
    }
}
