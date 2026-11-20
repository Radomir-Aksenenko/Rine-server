use super::state::Palette;

// Vanilla BlockState ids, two masks per state (tools/noise-golden/FeatureShapes.java).
const SHAPES: &[u8] = include_bytes!("../../assets/worldgen/feature_shapes.bin");
const OCCLUSION: &[u8] = include_bytes!("../../assets/worldgen/feature_occlusion.bin");
pub const DIRECTIONS: [(i32, i32, i32); 6] = [
    (0, -1, 0), (0, 1, 0), (0, 0, -1), (0, 0, 1), (-1, 0, 0), (1, 0, 0),
];

pub fn palette_shapes(palette: &Palette) -> Vec<[u8; 3]> {
    let blocks = &rine_data::init().blocks;
    palette.states().iter().map(|name| {
        blocks.parse_state(name).and_then(|state| {
            let i = state as usize * 2;
            let solid = u8::from(blocks.has(state, rine_data::blocks::flag::SOLID)) << 6;
            Some([*SHAPES.get(i)?, *SHAPES.get(i + 1)? | solid, *OCCLUSION.get(state as usize)?])
        }).unwrap_or_default()
    }).collect()
}
