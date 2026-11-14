//! Vanilla Minecraft 26.3 world generation port. Phase 1: random sources and noise primitives.
//! Phase 2: overworld density (`density`).
//!
//! Bit-exact with the vanilla server jar; see `tests/golden.rs` and `tools/noise-golden`.

pub mod density;
pub mod mth;
pub mod noise;
pub mod random;

pub use noise::{BlendedNoise, NoiseStack, NormalNoise, PerlinNoise, SimplexNoise, SmearedPerlinNoise};
pub use random::{AnyRng, Legacy, Positional, Rng, Xoroshiro};
pub use density::{ChunkDensity, RouterField, Terrain};
