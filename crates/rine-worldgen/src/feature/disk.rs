//! `DiskFeature` (vanilla 26.3, `levelgen/feature/DiskFeature`): a flat cylinder of states replacing the
//! target blocks, column by column. The radius is drawn once; the columns are visited in
//! `BlockPos.betweenClosed` order (z outermost, x innermost).

use super::placement::{sample_int, IntProvider};
use super::predicate::BlockPredicate;
use super::region::Region;
use super::rng::WorldgenRng;
use super::state_provider::StateProvider;

pub struct Disk {
    pub state: StateProvider,
    pub target: BlockPredicate,
    pub radius: IntProvider,
    pub half_height: i32,
}

impl Disk {
    /// Returns whether any block was placed.
    pub fn place(&self, region: &mut Region, rng: &mut WorldgenRng, origin: (i32, i32, i32)) -> bool {
        let (ox, oy, oz) = origin;
        let top = oy + self.half_height;
        let bottom = oy - self.half_height - 1;
        let r = sample_int(&self.radius, rng);
        let mut placed_any = false;
        for z in (oz - r)..=(oz + r) {
            for x in (ox - r)..=(ox + r) {
                let xd = x - ox;
                let zd = z - oz;
                if xd * xd + zd * zd <= r * r {
                    placed_any |= self.place_column(region, rng, x, z, top, bottom);
                }
            }
        }
        placed_any
    }

    fn place_column(&self, region: &mut Region, rng: &mut WorldgenRng, x: i32, z: i32, top: i32, bottom: i32) -> bool {
        // markAboveForPostProcessing is not modelled: it only matters for FULL-status neighbour updates of
        // the block above, which the decoration port does not run. The write itself is exact.
        let mut placed_any = false;
        for y in ((bottom + 1)..=top).rev() {
            if self.target.test(region, x, y, z) {
                if let Some(s) = self.state.get_optional(region, rng, x, y, z) {
                    region.set_block(x, y, z, s);
                    placed_any = true;
                }
            }
        }
        placed_any
    }
}
