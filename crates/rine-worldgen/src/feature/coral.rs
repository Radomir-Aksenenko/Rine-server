use serde_json::Value;

use super::{
    multiface::{relative, shuffle, HORIZONTAL},
    region::{Pos, Region},
    rng::WorldgenRng,
    state::Palette,
};

pub(super) enum Coral {
    Tree(usize),
    Claw(usize),
}

impl Coral {
    pub fn place(
        &self,
        level: &mut Region,
        rng: &mut WorldgenRng,
        origin: Pos,
        place: impl Fn(usize, &mut Region, &mut WorldgenRng, Pos) -> bool,
    ) -> bool {
        match *self {
            Self::Tree(feature) => {
                let mut top = origin;
                for _ in 0..rng.next_int_bound(3) + 1 {
                    if !place(feature, level, rng, top) {
                        return true;
                    }
                    top = relative(top, 1);
                }
                let count = rng.next_int_bound(3) as usize + 2;
                let mut directions = HORIZONTAL;
                shuffle(&mut directions, rng);
                for &d in &directions[..count] {
                    let mut p = relative(top, d);
                    let height = rng.next_int_bound(5) + 2;
                    let mut segment = 0;
                    for j in 0..height {
                        if !place(feature, level, rng, p) {
                            break;
                        }
                        segment += 1;
                        p = relative(p, 1);
                        if j == 0 || segment >= 2 && rng.next_float() < 0.25 {
                            p = relative(p, d);
                            segment = 0;
                        }
                    }
                }
            }
            Self::Claw(feature) => {
                if !place(feature, level, rng, origin) {
                    return false;
                }
                let d = rng.next_int_bound(4) as usize;
                let claw = HORIZONTAL[d];
                let count = rng.next_int_bound(2) as usize + 2;
                let mut choices = [claw, HORIZONTAL[(d + 1) % 4], HORIZONTAL[(d + 3) % 4]];
                shuffle(&mut choices, rng);
                for &branch in &choices[..count] {
                    let side_length = rng.next_int_bound(2) + 1;
                    let mut p = relative(origin, branch);
                    let (segment, length) = if branch == claw {
                        (claw, rng.next_int_bound(3) + 2)
                    } else {
                        p = relative(p, 1);
                        (
                            [branch, 1][rng.next_int_bound(2) as usize],
                            rng.next_int_bound(3) + 3,
                        )
                    };
                    for _ in 0..side_length {
                        if !place(feature, level, rng, p) {
                            break;
                        }
                        p = relative(p, segment);
                    }
                    p = relative(relative(p, segment ^ 1), 1);
                    for _ in 0..length {
                        p = relative(p, claw);
                        if !place(feature, level, rng, p) {
                            break;
                        }
                        if rng.next_float() < 0.25 {
                            p = relative(p, 1);
                        }
                    }
                }
            }
        }
        true
    }
}

pub(super) struct Decoration {
    states: Box<[u16]>,
    face: usize,
}

impl Decoration {
    pub fn compile(
        provider: &Value,
        bundle: &Value,
        pal: &mut Palette,
    ) -> Result<Option<Self>, String> {
        let (name, face) = match provider["type"].as_str() {
            Some("minecraft:random_block") if provider["blocks"] == "#minecraft:corals" => {
                ("minecraft:corals", 1)
            }
            Some("minecraft:rotated")
                if provider["state"]["type"] == "minecraft:random_block"
                    && provider["state"]["blocks"] == "#minecraft:wall_corals" =>
            {
                let direction = provider["direction"]
                    .as_str()
                    .ok_or("wall coral direction")?;
                let face = super::multiface::FACE_NAMES
                    .iter()
                    .position(|&name| name == direction)
                    .filter(|&face| face >= 2)
                    .ok_or("wall coral direction")?;
                ("minecraft:wall_corals", face)
            }
            _ => return Ok(None),
        };
        let tag = bundle["blockTags"][name]
            .as_array()
            .ok_or("missing coral tag")?;
        let mut states = Vec::with_capacity(tag.len());
        // The bundle sorts tag membership. RandomBlockProvider uses the vanilla tag's holder order.
        let suffixes: &[&str] = if face == 1 {
            &["_coral", "_coral_fan"]
        } else {
            &["_coral_wall_fan"]
        };
        for suffix in suffixes {
            for kind in ["tube", "brain", "bubble", "fire", "horn"] {
                let block = format!("minecraft:{kind}{suffix}");
                if !tag.iter().any(|b| b.as_str() == Some(&block)) {
                    return Err(format!("coral tag {name} lacks {block}"));
                }
                let state = pal.default_state(&block);
                states.push(if face == 1 {
                    state
                } else {
                    pal.with_property(state, "facing", super::multiface::FACE_NAMES[face])
                        .ok_or("wall coral facing")?
                });
            }
        }
        if states.len() != tag.len() {
            return Err(format!("unexpected size for coral tag {name}"));
        }
        Ok(Some(Self {
            states: states.into_boxed_slice(),
            face,
        }))
    }

    pub fn place(&self, level: &mut Region, rng: &mut WorldgenRng, pos: Pos) -> bool {
        let state = self.states[rng.next_int_bound(self.states.len() as i32) as usize];
        let (x, y, z) = relative(pos, self.face ^ 1);
        if !level.tags.sturdy(level.get_block(x, y, z), self.face) {
            return false;
        }
        level.write(pos, state, 2);
        true
    }
}
