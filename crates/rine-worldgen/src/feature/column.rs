//! `BlockColumnFeature` (vanilla 26.3, `levelgen/feature/BlockColumnFeature`): stacks layers of states
//! along a direction. Layer heights are drawn first, then the column is cut where `allowed_placement`
//! fails (`truncate`), then every layer is written bottom-up with its state provider.

use super::placement::{sample_int, IntProvider};
use super::predicate::BlockPredicate;
use super::region::Region;
use super::rng::WorldgenRng;
use super::state_provider::StateProvider;

pub struct Layer {
    pub height: IntProvider,
    pub state: StateProvider,
}

pub struct BlockColumn {
    pub layers: Vec<Layer>,
    /// Unit step of `Direction` (`up` is `(0, 1, 0)`).
    pub direction: (i32, i32, i32),
    pub allowed: BlockPredicate,
    pub prioritize_tip: bool,
}

/// `Direction` by name, as the configs spell it.
pub fn direction_of(name: &str) -> Result<(i32, i32, i32), String> {
    Ok(match name {
        "up" => (0, 1, 0),
        "down" => (0, -1, 0),
        "north" => (0, 0, -1),
        "south" => (0, 0, 1),
        "west" => (-1, 0, 0),
        "east" => (1, 0, 0),
        other => return Err(format!("unknown direction {other}")),
    })
}

impl BlockColumn {
    /// Returns whether the feature reported success (`true` once any height was drawn).
    pub fn place(&self, region: &mut Region, rng: &mut WorldgenRng, origin: (i32, i32, i32)) -> bool {
        let n = self.layers.len();
        let mut small = [0; 4];
        let mut large;
        let heights = if n <= small.len() {
            &mut small[..n]
        } else {
            large = vec![0; n];
            &mut large
        };
        let mut total = 0;
        for (i, layer) in self.layers.iter().enumerate() {
            heights[i] = sample_int(&layer.height, rng);
            total += heights[i];
        }
        if total == 0 {
            return false;
        }

        let (dx, dy, dz) = self.direction;
        let (mut px, mut py, mut pz) = origin;
        let (mut nx, mut ny, mut nz) = (px + dx, py + dy, pz + dz);
        for y in 0..total {
            if !self.allowed.test(region, nx, ny, nz) {
                truncate(heights, total, y, self.prioritize_tip);
                break;
            }
            nx += dx;
            ny += dy;
            nz += dz;
        }

        for (i, layer) in self.layers.iter().enumerate() {
            for _ in 0..heights[i] {
                let state = layer.state.get(region, rng, px, py, pz);
                region.set_block(px, py, pz, state);
                px += dx;
                py += dy;
                pz += dz;
            }
        }
        true
    }
}

/// `BlockColumnFeature.truncate`: removes `total - new_height` blocks from the tip (`prioritize_tip`)
/// or the base end first.
fn truncate(heights: &mut [i32], total: i32, new_height: i32, prioritize_tip: bool) {
    let mut remove = total - new_height;
    let n = heights.len() as i32;
    if prioritize_tip {
        for h in heights.iter_mut().take(n as usize) {
            if remove <= 0 {
                break;
            }
            let take = (*h).min(remove);
            remove -= take;
            *h -= take;
        }
    } else {
        for i in (0..n as usize).rev() {
            if remove <= 0 {
                break;
            }
            let take = heights[i].min(remove);
            remove -= take;
            heights[i] -= take;
        }
    }
}
