//! `Heightmap.Types` (vanilla 26.3, predicates checked against the server jar bytecode) and the
//! `Heightmap.update` / `primeHeightmaps` semantics the decoration needs.
//!
//! Which types a block write updates depends on the chunk's persisted status (`ProtoChunk.setBlockState`
//! uses `getPersistedStatus().heightmapsAfter()`): `ChunkStatus.BIOMES` (persisted while the terrain
//! task runs, so carver writes) maintains only the two `*_WG` types, and `ChunkStatus.TERRAIN` onward
//! maintains the four final types. The final types are primed at the end of the terrain task
//! (`ChunkStatusTasks.buildTerrain`), so during FEATURES they are exact for the current blocks. The `*_WG`
//! types are never touched after terrain: during decoration they hold the terrain-end values, which is
//! why `WORLD_SURFACE_WG`/`OCEAN_FLOOR_WG` placements see the pre-decoration surface.

use super::region::Tags;

/// The six `Heightmap.Types`. Discriminants are the bit positions of `Tags::opaque` masks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    WorldSurfaceWg = 0,
    OceanFloorWg = 1,
    WorldSurface = 2,
    OceanFloor = 3,
    MotionBlocking = 4,
    MotionBlockingNoLeaves = 5,
}

/// `ChunkStatus.FINAL_HEIGHTMAPS` (the non-`WG` types), in region table order.
pub const FINAL: [Kind; 4] = [Kind::WorldSurface, Kind::OceanFloor, Kind::MotionBlocking, Kind::MotionBlockingNoLeaves];

/// Terrain-end heights of the worldgen types, `[WORLD_SURFACE_WG, OCEAN_FLOOR_WG]`, 256 columns each,
/// absolute first-available heights (`getFirstAvailable`), column `(z & 15) * 16 + (x & 15)`.
pub type WgHeights = [Vec<i32>; 2];

impl Kind {
    /// Parses a `Heightmap.Types` name as written in the placement JSON.
    pub fn from_name(name: &str) -> Result<Kind, String> {
        Ok(match name {
            "WORLD_SURFACE_WG" => Kind::WorldSurfaceWg,
            "OCEAN_FLOOR_WG" => Kind::OceanFloorWg,
            "WORLD_SURFACE" => Kind::WorldSurface,
            "OCEAN_FLOOR" => Kind::OceanFloor,
            "MOTION_BLOCKING" => Kind::MotionBlocking,
            "MOTION_BLOCKING_NO_LEAVES" => Kind::MotionBlockingNoLeaves,
            other => return Err(format!("unsupported heightmap {other}")),
        })
    }

    /// Mask bit of this type in `Tags::opaque`.
    #[inline]
    pub fn bit(self) -> u8 {
        1 << (self as u8)
    }

    /// Index among the four final types, if this is one.
    pub fn final_index(self) -> Option<usize> {
        FINAL.iter().position(|&k| k == self)
    }
}

/// Predicate masks per palette state: bit `Kind::bit()` set when the state is `isOpaque` for that type.
/// `NOT_AIR` for the surface types, `BLOCKS_MOTION_IN_HEIGHTMAP` tag (`is`) for the ocean floor, and
/// tag-or-fluid for the motion-blocking types (`!getFluidState().isEmpty()`).
pub fn opaque_masks(air: &[bool], fluid_nonempty: &[bool], motion: &[bool], motion_no_leaves: &[bool]) -> Vec<u8> {
    (0..air.len())
        .map(|i| {
            let mut m = 0u8;
            if !air[i] {
                m |= Kind::WorldSurfaceWg.bit() | Kind::WorldSurface.bit();
            }
            if motion[i] {
                m |= Kind::OceanFloorWg.bit() | Kind::OceanFloor.bit() | Kind::MotionBlocking.bit();
            }
            if motion[i] || fluid_nonempty[i] {
                m |= Kind::MotionBlocking.bit();
            }
            if motion_no_leaves[i] || fluid_nonempty[i] {
                m |= Kind::MotionBlockingNoLeaves.bit();
            }
            m
        })
        .collect()
}

/// `Heightmap.primeHeightmaps` for one column: the first-available height above the topmost opaque block,
/// or `min_y` when no block is opaque. `blocks` is one chunk's block array, `(rel_y * 16 + z) * 16 + x`.
pub fn prime_column(blocks: &[u16], tags: &Tags, kind: Kind, min_y: i32, height: i32, col: usize) -> i32 {
    let (z, x) = (col / 16, col % 16);
    for rel in (0..height as usize).rev() {
        if tags.is_opaque(blocks[(rel * 16 + z) * 16 + x], kind) {
            return min_y + rel as i32 + 1;
        }
    }
    min_y
}

/// Worldgen heights of one chunk from its blocks (done once, on the terrain-end blocks).
pub fn wg_heights(blocks: &[u16], tags: &Tags, min_y: i32, height: i32) -> WgHeights {
    let surface = (0..256).map(|c| prime_column(blocks, tags, Kind::WorldSurfaceWg, min_y, height, c)).collect();
    let ocean = (0..256).map(|c| prime_column(blocks, tags, Kind::OceanFloorWg, min_y, height, c)).collect();
    [surface, ocean]
}

/// `Heightmap.update` for one final-type column after the block at `y` became `blk`. `column` holds the
/// chunk's blocks; `first` is the current first-available height. Returns the new height.
pub fn update_column(column: &[u16], tags: &Tags, kind: Kind, min_y: i32, first: i32, x: usize, y: i32, z: usize, blk: u16) -> i32 {
    if y <= first - 2 {
        return first;
    }
    if tags.is_opaque(blk, kind) {
        if y >= first {
            return y + 1;
        }
    } else if first - 1 == y {
        for yy in (min_y..y).rev() {
            let rel = (yy - min_y) as usize;
            if tags.is_opaque(column[(rel * 16 + z) * 16 + x], kind) {
                return yy + 1;
            }
        }
        return min_y;
    }
    first
}
