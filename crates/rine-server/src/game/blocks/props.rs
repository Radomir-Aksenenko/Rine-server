//! Block state properties by arithmetic on the state id, with no string work
//! at run time. Property names are a fixed enum (`Pr`); enum values (like
//! "north", "straight") are interned to small ids once (`vid`).
//!
//! Add a property by appending a variant to `Pr` and its name to `NAMES`.

use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Pr {
    Age,
    Moisture,
    Distance,
    Persistent,
    Layers,
    Snowy,
    Lit,
    Stage,
    Hatch,
    Eggs,
    Bites,
    Candles,
    Level,
    Shape,
    Facing,
    Half,
    North,
    East,
    South,
    West,
    Up,
    Down,
    Waterlogged,
    InWall,
    Open,
    Hinge,
    Axis,
    Powered,
    Type,
    Attached,
    Hydration,
    Berries,
    Thickness,
    VerticalDirection,
    Leaves,
    Charges,
    Unstable,
    Bottom,
    Hanging,
    Tilt,
    Pickles,
    Rotation,
    HasRecord,
    HasBook,
    Drag,
    Dusted,
    FlowerAmount,
    SegmentAmount,
    HoneyLevel,
    Occupied,
    Part,
    Face,
    Power,
    Attachment,
    Disarmed,
    Conditional,
    Mode,
    Triggered,
    Enabled,
    Inverted,
    Extended,
    Short,
    Note,
    Instrument,
    Delay,
    Locked,
}

const NAMES: &[&str] = &[
    "age",
    "moisture",
    "distance",
    "persistent",
    "layers",
    "snowy",
    "lit",
    "stage",
    "hatch",
    "eggs",
    "bites",
    "candles",
    "level",
    "shape",
    "facing",
    "half",
    "north",
    "east",
    "south",
    "west",
    "up",
    "down",
    "waterlogged",
    "in_wall",
    "open",
    "hinge",
    "axis",
    "powered",
    "type",
    "attached",
    "hydration",
    "berries",
    "thickness",
    "vertical_direction",
    "leaves",
    "charges",
    "unstable",
    "bottom",
    "hanging",
    "tilt",
    "pickles",
    "rotation",
    "has_record",
    "has_book",
    "drag",
    "dusted",
    "flower_amount",
    "segment_amount",
    "honey_level",
    "occupied",
    "part",
    "face",
    "power",
    "attachment",
    "disarmed",
    "conditional",
    "mode",
    "triggered",
    "enabled",
    "inverted",
    "extended",
    "short",
    "note",
    "instrument",
    "delay",
    "locked",
];

const NPR: usize = 66;
const MAXV: usize = 32;

struct Ph {
    min_state: u32,
    stride: u32,
    n: u32,
    /// Integer value of each index (numbers parsed; true=1, false=0; enums: index).
    ints: [i16; MAXV],
    /// Interned value id of each index.
    vids: [u8; MAXV],
}

struct Tables {
    handles: Vec<[u16; NPR]>,
    ph: Vec<Ph>,
    values: Vec<String>,
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(build)
}

fn build() -> Tables {
    debug_assert_eq!(NAMES.len(), NPR);
    let b = &rine_data::get().blocks;
    let mut values: Vec<String> = Vec::new();
    let mut intern = |v: &str, values: &mut Vec<String>| -> u8 {
        match values.iter().position(|x| x == v) {
            Some(i) => i as u8,
            None => {
                values.push(v.to_string());
                assert!(values.len() < 256, "too many distinct property values");
                (values.len() - 1) as u8
            }
        }
    };
    let mut handles = Vec::with_capacity(b.blocks.len());
    let mut ph = Vec::new();
    for blk in &b.blocks {
        let mut h = [u16::MAX; NPR];
        for p in &blk.props {
            let Some(pi) = NAMES.iter().position(|n| *n == p.name) else { continue };
            assert!(p.values.len() <= MAXV);
            let mut ints = [0i16; MAXV];
            let mut vids = [0u8; MAXV];
            for (i, v) in p.values.iter().enumerate() {
                ints[i] = match v.as_str() {
                    "true" => 1,
                    "false" => 0,
                    s => s.parse::<i16>().unwrap_or(i as i16),
                };
                vids[i] = intern(v, &mut values);
            }
            h[pi] = ph.len() as u16;
            ph.push(Ph { min_state: blk.min_state, stride: p.stride, n: p.values.len() as u32, ints, vids });
        }
        handles.push(h);
    }
    Tables { handles, ph, values }
}

#[inline]
fn handle(st: u16, p: Pr) -> Option<&'static Ph> {
    let t = tables();
    let b = rine_data::get().blocks.states[st as usize].block as usize;
    let h = t.handles[b][p as usize];
    if h == u16::MAX {
        None
    } else {
        Some(&t.ph[h as usize])
    }
}

#[inline]
fn index_of(ph: &Ph, st: u16) -> u32 {
    ((st as u32 - ph.min_state) / ph.stride) % ph.n
}

#[inline]
fn with_index(ph: &Ph, st: u16, i: u32) -> u16 {
    let cur = index_of(ph, st);
    (st as u32 - cur * ph.stride + i * ph.stride) as u16
}

/// Interned id of a property value string ("north", "true", "3").
/// Call at init time; unknown values give 255.
pub fn vid(v: &str) -> u8 {
    tables().values.iter().position(|x| x == v).map(|i| i as u8).unwrap_or(255)
}

#[inline]
pub fn has(st: u16, p: Pr) -> bool {
    handle(st, p).is_some()
}

/// Integer value (numbers as written, booleans 1/0, enums by index); -1 if absent.
#[inline]
pub fn geti(st: u16, p: Pr) -> i32 {
    match handle(st, p) {
        Some(ph) => ph.ints[index_of(ph, st) as usize] as i32,
        None => -1,
    }
}

/// `st` with the property set to the integer value `v`; unchanged if absent or invalid.
#[inline]
pub fn seti(st: u16, p: Pr, v: i32) -> u16 {
    let Some(ph) = handle(st, p) else { return st };
    match ph.ints[..ph.n as usize].iter().position(|&x| x as i32 == v) {
        Some(i) => with_index(ph, st, i as u32),
        None => st,
    }
}

#[inline]
pub fn getb(st: u16, p: Pr) -> bool {
    geti(st, p) == 1
}

#[inline]
pub fn setb(st: u16, p: Pr, v: bool) -> u16 {
    seti(st, p, v as i32)
}

/// Interned value id (compare with `vid("...")`); 255 if absent.
#[inline]
pub fn getv(st: u16, p: Pr) -> u8 {
    match handle(st, p) {
        Some(ph) => ph.vids[index_of(ph, st) as usize],
        None => 255,
    }
}

/// `st` with the property set to the value id `v`; unchanged if absent or invalid.
#[inline]
pub fn setv(st: u16, p: Pr, v: u8) -> u16 {
    let Some(ph) = handle(st, p) else { return st };
    match ph.vids[..ph.n as usize].iter().position(|&x| x == v) {
        Some(i) => with_index(ph, st, i as u32),
        None => st,
    }
}

/// Value string (for messages and tests, not for hot paths).
pub fn name(st: u16, p: Pr) -> Option<&'static str> {
    let ph = handle(st, p)?;
    Some(tables().values[ph.vids[index_of(ph, st) as usize] as usize].as_str())
}

/// Number of values of the property in this block (0 if absent).
pub fn count(st: u16, p: Pr) -> u32 {
    handle(st, p).map(|ph| ph.n).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_and_bool_and_enum() {
        rine_data::init();
        let b = &rine_data::get().blocks;
        let wheat = b.default_state("wheat");
        assert_eq!(geti(wheat, Pr::Age), 0);
        let w5 = seti(wheat, Pr::Age, 5);
        assert_eq!(geti(w5, Pr::Age), 5);
        assert_eq!(b.get_prop(w5, "age"), Some("5"));
        let leaves = b.default_state("oak_leaves");
        assert_eq!(geti(leaves, Pr::Distance), 7);
        let l1 = setb(seti(leaves, Pr::Distance, 1), Pr::Persistent, true);
        assert_eq!(b.get_prop(l1, "distance"), Some("1"));
        assert_eq!(b.get_prop(l1, "persistent"), Some("true"));
        let stairs = b.default_state("oak_stairs");
        let s = setv(stairs, Pr::Shape, vid("outer_left"));
        assert_eq!(b.get_prop(s, "shape"), Some("outer_left"));
        assert_eq!(name(s, Pr::Shape), Some("outer_left"));
        assert_eq!(getv(s, Pr::Shape), vid("outer_left"));
        assert!(!has(wheat, Pr::Shape));
        assert_eq!(seti(wheat, Pr::Age, 99), wheat);
    }
}
