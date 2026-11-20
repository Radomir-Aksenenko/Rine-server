use super::region::{Pos, Region};
use super::shape::DIRECTIONS;
use super::state::Palette;

pub struct Vines([u16; 5]);

impl Vines {
    pub fn compile(pal: &mut Palette) -> Result<Self, String> {
        let base = pal.default_state("minecraft:vine");
        let mut states = [0; 5];
        for (state, direction) in states.iter_mut().zip(["up", "north", "south", "west", "east"]) {
            *state = pal.with_property(base, direction, "true").ok_or("vine face property")?;
        }
        Ok(Self(states))
    }

    pub fn place(&self, region: &mut Region, (x, y, z): Pos) -> bool {
        if !region.is_air(x, y, z) {
            return false;
        }
        for (i, (&(dx, dy, dz), &state)) in DIRECTIONS[1..].iter().zip(&self.0).enumerate() {
            let support = region.get_block(x + dx, y + dy, z + dz);
            if region.tags.face_full(support, (i + 1) ^ 1) {
                region.write((x, y, z), state, 2);
                return true;
            }
        }
        false
    }
}
