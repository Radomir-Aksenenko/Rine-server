use super::region::{Pos, Region, Tags};
use super::rng::WorldgenRng;
use super::shape::DIRECTIONS;
use super::state::Palette;

const HORIZONTAL: [usize; 4] = [2, 5, 3, 4];
const SIDES: [&str; 3] = ["none", "low", "tall"];
const WEIGHTS: [usize; 4] = [1, 3, 9, 27];

pub(crate) struct Carpet {
    block: u16,
    states: [[u16; 81]; 2],
    shape_by_state: Vec<u8>,
}

impl Carpet {
    pub fn compile(pal: &mut Palette, block: u16) -> Self {
        let mut carpet = Self {
            block, states: [[0; 81]; 2], shape_by_state: Vec::new(),
        };
        for base in 0..2 {
            for shape in 0..81 {
                let state = pal.state_from_parts("minecraft:pale_moss_carpet", &[
                    ("bottom", if base == 1 { "true" } else { "false" }),
                    ("east", SIDES[shape / 3 % 3]),
                    ("north", SIDES[shape % 3]),
                    ("south", SIDES[shape / 9 % 3]),
                    ("west", SIDES[shape / 27]),
                ]);
                carpet.states[base][shape] = state;
            }
        }
        carpet.shape_by_state.resize(pal.states().len(), u8::MAX);
        for (base, states) in carpet.states.iter().enumerate() {
            for (shape, &state) in states.iter().enumerate() {
                carpet.shape_by_state[state as usize] = (base * 81 + shape) as u8;
            }
        }
        carpet
    }

    fn shape(&self, state: u16) -> Option<(usize, usize)> {
        let index = *self.shape_by_state.get(state as usize)?;
        (index != u8::MAX).then_some((index as usize / 81, index as usize % 81))
    }

    pub fn is_base(&self, state: u16) -> bool {
        matches!(self.shape(state), Some((1, _)))
    }

    pub fn can_survive(&self, tags: &Tags, state: u16, below: u16) -> bool {
        match self.shape(state) {
            Some((1, _)) => !tags.is_air(below),
            Some((0, _)) => tags.is_block(below, self.block) && matches!(self.shape(below), Some((1, _))),
            _ => false,
        }
    }

    /// MossyCarpetBlock.getUpdatedState with createSides=true.
    fn updated(&self, region: &Region, (x, y, z): Pos, base: usize, previous: Option<usize>) -> usize {
        let tags = region.tags;
        let shape_at = |y| {
            let state = region.get_block(x, y, z);
            if tags.is_block(state, self.block) { self.shape(state) } else { None }
        };
        let above = shape_at(y + 1);
        let below = if base == 0 { shape_at(y - 1) } else { None };
        let mut shape = 0;
        for (index, dir) in HORIZONTAL.into_iter().enumerate() {
            let weight = WEIGHTS[index];
            let (dx, _, dz) = DIRECTIONS[dir];
            let neighbor = region.get_block(x + dx, y, z + dz);
            if !tags.face_full(neighbor, dir ^ 1) {
                continue;
            }
            let mut side = previous.map_or(1, |s| s / weight % 3);
            if side == 1 {
                if above.is_some_and(|(base, sides)| base == 0 && sides / weight % 3 != 0) {
                    side = 2;
                }
                if below.is_some_and(|(_, sides)| sides / weight % 3 == 0) {
                    side = 0;
                }
            }
            shape += side * weight;
        }
        shape
    }

    /// MossyCarpetBlock.placeAt and createTopperWithSideChance.
    pub fn place(&self, region: &mut Region, rng: &mut WorldgenRng, (x, y, z): Pos) {
        let bottom = self.updated(region, (x, y, z), 1, None);
        region.write((x, y, z), self.states[1][bottom], 2);
        if let Some(topper) = self.topper(region, (x, y, z), Some(rng)) {
            region.write((x, y + 1, z), topper, 2);
            let bottom = self.updated(region, (x, y, z), 1, None);
            region.write((x, y, z), self.states[1][bottom], 2);
        }
    }

    pub fn topper(&self, region: &mut Region, (x, y, z): Pos, mut rng: Option<&mut WorldgenRng>) -> Option<u16> {
        let above = region.get_block(x, y + 1, z);
        let tags = region.tags;
        if tags.is_block(above, self.block) {
            if matches!(self.shape(above), Some((1, _))) {
                return None;
            }
        } else if !tags.replaceable(above) {
            return None;
        }
        let mut top = self.updated(region, (x, y + 1, z), 0, None);
        for weight in WEIGHTS {
            let side = top / weight % 3;
            if side != 0 && rng.as_mut().is_some_and(|rng| !region.level_next_bool(rng)) {
                top -= side * weight;
            }
        }
        let state = self.states[0][top];
        (top != 0 && state != above).then_some(state)
    }

    pub fn update_shape(&self, region: &Region, (x, y, z): Pos, state: u16, air: u16) -> u16 {
        let Some((base, old)) = self.shape(state) else { return state };
        if !self.can_survive(region.tags, state, region.get_block(x, y - 1, z)) {
            return air;
        }
        let sides = self.updated(region, (x, y, z), base, (base == 0).then_some(old));
        if base == 0 && sides == 0 { air } else { self.states[base][sides] }
    }
}
