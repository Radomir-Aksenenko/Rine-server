//! Signs: `SignBlock` and its four subclasses (`StandingSignBlock`,
//! `WallSignBlock`, `CeilingHangingSignBlock`, `WallHangingSignBlock`) with
//! their block entity (`SignBlockEntity`, `SignText`).
//!
//! Pure rules: placement (`StandingAndWallBlockItem.getPlacementState`),
//! the applicators (`SignBlock.useItemOn`, `useWithoutItem`), the side the
//! player faces, and the NBT shape. The block entity NBT in the chunk is the
//! only source of truth; `Side` is parsed from it on demand.
//!
//! Game glue at the bottom: editor ownership (`playerWhoMayEdit`), the
//! `sign_update` packet, the editor packet and block entity updates. The
//! caller applies `SignOp` effects from `UseOut` (see `game/interact.rs`).

use super::interact::{ItemOp, Res, UseCtx, UseOut};
use super::props::{self, Pr};
use super::{block_of, block_tag, default_state, item_id, Beh, PlaceCtx, Rng};
use crate::game::redstone::data::{self as rd, DOWN, EAST, NORTH, SOUTH, UP, WEST};
use crate::game::redstone::engine::{rel, Lvl, CAT_BLOCKS};
use crate::game::redstone::Rs;
use crate::game::Game;
use rine_data::blocks::flag;
use rine_protocol::buf::Writer;
use rine_protocol::frame::packet;
use rine_protocol::ids::play::{C_BLOCK_ENTITY_DATA, C_OPEN_SIGN_EDITOR};
use rine_protocol::nbt::Tag;
use rine_protocol::BlockPos;
use rine_world::ChunkPos;
use std::sync::{Mutex, MutexGuard, OnceLock};
use uuid::Uuid;

use crate::game::blockent;

/// `DyeColor` in id order.
pub const DYE_COLORS: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black",
];
const BLACK: u8 = 15;
/// `Style` fields a plain line keeps when its text is replaced.
const STYLE_KEYS: [&str; 11] = ["color", "bold", "italic", "underlined", "strikethrough", "obfuscated", "font", "shadow_color", "insertion", "click_event", "hover_event"];
/// `SignBlockEntity.MAX_...` is not needed here; lines are `SignText.LINES`.
const LINES: usize = 4;

/// Which of the four sign classes a block is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Standing,
    Wall,
    Ceiling,
    WallHanging,
}

impl Kind {
    fn hanging(self) -> bool {
        matches!(self, Kind::Ceiling | Kind::WallHanging)
    }
}

fn kind_of_name(n: &str) -> Option<Kind> {
    if n.ends_with("_wall_hanging_sign") {
        Some(Kind::WallHanging)
    } else if n.ends_with("_hanging_sign") {
        Some(Kind::Ceiling)
    } else if n.ends_with("_wall_sign") {
        Some(Kind::Wall)
    } else if n.ends_with("_sign") {
        Some(Kind::Standing)
    } else {
        None
    }
}

/// Claims every sign block for `Beh::Sign`.
pub fn classify(n: &str) -> Option<Beh> {
    kind_of_name(n).map(|_| Beh::Sign)
}

struct Items {
    dye: [u32; 16],
    glow: u32,
    ink: u32,
    honeycomb: u32,
}

struct Tables {
    kind: Vec<Option<Kind>>,
    all_hanging: Vec<bool>,
    wall_hanging: Vec<bool>,
    items: Items,
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let blocks = &rine_data::get().blocks;
        let kind = blocks.blocks.iter().map(|b| kind_of_name(b.name.strip_prefix("minecraft:").unwrap_or(&b.name))).collect();
        let items = Items {
            dye: std::array::from_fn(|i| item_id(&format!("{}_dye", DYE_COLORS[i]))),
            glow: item_id("glow_ink_sac"),
            ink: item_id("ink_sac"),
            honeycomb: item_id("honeycomb"),
        };
        Tables {
            kind,
            all_hanging: block_tag("all_hanging_signs"),
            wall_hanging: block_tag("wall_hanging_signs"),
            items,
        }
    })
}

/// Sign class of a block state, None for every other block.
pub fn kind(st: u16) -> Option<Kind> {
    tables().kind.get(block_of(st) as usize).copied().flatten()
}

// ------------------------------------------------------------------ NBT

fn fresh_side() -> Side {
    Side { messages: vec![Tag::String(String::new()); LINES], filtered: None, color: BLACK, glow: false }
}

/// `SignText` as saved: `messages`, `filtered_messages` (only when it
/// differs), `color`, `has_glowing_text`.
#[derive(Clone, Debug, PartialEq)]
pub struct Side {
    pub messages: Vec<Tag>,
    pub filtered: Option<Vec<Tag>>,
    pub color: u8,
    pub glow: bool,
}

impl Side {
    /// `SignText.load` from the saved compound; anything unreadable is the
    /// empty text (`loadAdditional` falls back to `SignText.EMPTY`).
    pub fn from_tag(t: Option<&Tag>) -> Self {
        let Some(t) = t else { return fresh_side() };
        let messages = match t.get("messages") {
            Some(Tag::List(l)) if l.len() == LINES => l.clone(),
            _ => return fresh_side(),
        };
        let filtered = match t.get("filtered_messages") {
            Some(Tag::List(l)) if l.len() == LINES => Some(l.clone()),
            _ => None,
        };
        let color = match t.get("color") {
            Some(Tag::String(s)) => DYE_COLORS.iter().position(|c| *c == s).map_or(BLACK, |i| i as u8),
            _ => BLACK,
        };
        let glow = matches!(t.get("has_glowing_text"), Some(Tag::Byte(b)) if *b != 0);
        Side { messages, filtered, color, glow }
    }

    pub fn to_tag(&self) -> Tag {
        let mut f = vec![("messages".to_string(), Tag::List(self.messages.clone()))];
        if let Some(fl) = &self.filtered {
            if *fl != self.messages {
                f.push(("filtered_messages".to_string(), Tag::List(fl.clone())));
            }
        }
        f.push(("color".to_string(), Tag::String(DYE_COLORS[self.color as usize].to_string())));
        f.push(("has_glowing_text".to_string(), Tag::Byte(self.glow as i8)));
        Tag::Compound(f)
    }

    /// `SignText.hasMessage(false)`: a line with any text.
    pub fn has_message(&self) -> bool {
        self.messages.iter().any(|m| plain_text(m).map_or(true, |s| !s.is_empty()))
    }

    /// `SignText.hasEditableText`: every line is plain text (or empty).
    pub fn editable(&self) -> bool {
        self.messages.iter().all(|m| plain_text(m).is_some())
    }

    /// `updateMessages` with the text filter off: each stripped line is the
    /// raw text and, through `filteredOrEmpty`, the filtered text too, so
    /// `filtered_messages` is never written. Each line keeps its style.
    pub fn set_lines(&mut self, lines: &[String]) {
        let base = self.messages.clone();
        self.messages = lines.iter().take(LINES).enumerate().map(|(i, line)| styled(&base[i], line)).collect();
        self.filtered = None;
    }
}

/// An empty `front_text` / `back_text` compound (`SignText.EMPTY`).
pub fn fresh_side_tag() -> Tag {
    fresh_side().to_tag()
}

/// `SignBlockEntity.saveAdditional` for a fresh sign: both sides empty, not waxed.
pub fn fresh_nbt_value() -> Tag {
    Tag::Compound(vec![
        ("front_text".to_string(), fresh_side().to_tag()),
        ("back_text".to_string(), fresh_side().to_tag()),
        ("is_waxed".to_string(), Tag::Byte(0)),
    ])
}

/// Block entity NBT for a block just placed (`None` for other blocks).
pub fn fresh_nbt(st: u16) -> Option<Tag> {
    kind(st).map(|_| fresh_nbt_value())
}

fn side_key(front: bool) -> &'static str {
    if front {
        "front_text"
    } else {
        "back_text"
    }
}

fn side_of(nbt: Option<&Tag>, front: bool) -> Side {
    Side::from_tag(nbt.and_then(|t| t.get(side_key(front))))
}

fn is_waxed(nbt: Option<&Tag>) -> bool {
    matches!(nbt.and_then(|t| t.get("is_waxed")), Some(Tag::Byte(b)) if *b != 0)
}

fn put_side(nbt: &mut Tag, front: bool, side: &Side) {
    blockent::put(nbt, side_key(front), side.to_tag());
}

/// Plain text of a line: a string, or a text component with style keys only.
pub fn plain_text(t: &Tag) -> Option<&str> {
    match t {
        Tag::String(s) => Some(s),
        Tag::Compound(f) => {
            let mut text = None;
            for (k, v) in f {
                if k == "text" {
                    match v {
                        Tag::String(s) => text = Some(s.as_str()),
                        _ => return None,
                    }
                } else if !STYLE_KEYS.contains(&k.as_str()) {
                    return None;
                }
            }
            text
        }
        _ => None,
    }
}

/// `new Component.literal(text).setStyle(style of the old line)`.
fn styled(old: &Tag, text: &str) -> Tag {
    let Tag::Compound(f) = old else { return Tag::String(text.to_string()) };
    let mut out: Vec<(String, Tag)> = f.iter().filter(|(k, _)| STYLE_KEYS.contains(&k.as_str())).cloned().collect();
    if out.is_empty() {
        return Tag::String(text.to_string());
    }
    out.insert(0, ("text".to_string(), Tag::String(text.to_string())));
    Tag::Compound(out)
}

/// `ChatFormatting.stripFormatting`: drops `§` plus one code character
/// (`(?i)§[0-9A-FK-OR]`).
pub fn strip_formatting(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{a7}' && it.peek().is_some_and(|n| matches!(n.to_ascii_lowercase(), '0'..='9' | 'a'..='f' | 'k'..='o' | 'r')) {
            it.next();
        } else {
            out.push(c);
        }
    }
    out
}

// ------------------------------------------------------------------ facing

/// Direction of a protocol id in the vanilla 2D order (S W N E = 0..3).
fn data2d(d: u8) -> i32 {
    match d {
        SOUTH => 0,
        WEST => 1,
        NORTH => 2,
        _ => 3,
    }
}

/// `Direction.fromYRot`.
fn from_yaw(yaw: f32) -> u8 {
    const BY_2D: [u8; 4] = [SOUTH, WEST, NORTH, EAST];
    BY_2D[((f64::from(yaw) / 90.0 + 0.5).floor() as i64 & 3) as usize]
}

/// `RotationSegment.convertToSegment(Direction)` (`fromDirection`).
fn segment_of_dir(d: u8) -> i32 {
    if d == UP || d == DOWN {
        0
    } else {
        data2d(d) << 2
    }
}

/// `RotationSegment.convertToDirection`.
fn dir_of_segment(seg: i32) -> Option<u8> {
    match seg {
        0 => Some(NORTH),
        4 => Some(EAST),
        8 => Some(SOUTH),
        12 => Some(WEST),
        _ => None,
    }
}

/// `RotationSegment.convertToDegrees` (`SegmentedAnglePrecision.toDegrees`).
fn segment_degrees(seg: i32) -> f32 {
    let d = (seg & 15) as f32 * 22.5;
    if d >= 180.0 {
        d - 360.0
    } else {
        d
    }
}

fn facing_of(st: u16) -> Option<u8> {
    match props::name(st, Pr::Facing)? {
        "north" => Some(NORTH),
        "south" => Some(SOUTH),
        "west" => Some(WEST),
        "east" => Some(EAST),
        _ => None,
    }
}

fn dir_name(d: u8) -> &'static str {
    ["down", "up", "north", "south", "west", "east"][(d % 6) as usize]
}

fn with_facing(st: u16, d: u8) -> u16 {
    props::setv(st, Pr::Facing, props::vid(dir_name(d)))
}

/// `Block.boxes` bounds center of the outline (`WallSignBlock.getSignHitboxCenterPosition`).
fn hitbox_center(st: u16) -> (f64, f64, f64) {
    let blocks = &rine_data::get().blocks;
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for b in &blocks.shapes[blocks.state(st).shape as usize] {
        for a in 0..3 {
            lo[a] = lo[a].min(f64::from(b[a]));
            hi[a] = hi[a].max(f64::from(b[a + 3]));
        }
    }
    if lo[0] > hi[0] {
        return (0.5, 0.5, 0.5);
    }
    ((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0)
}

/// `SignBlock.getYRotationDegrees`.
fn yaw_degrees(st: u16, kind: Kind) -> f32 {
    match kind {
        Kind::Standing | Kind::Ceiling => segment_degrees(props::geti(st, Pr::Rotation)),
        Kind::Wall | Kind::WallHanging => facing_of(st).map_or(0.0, |f| data2d(f) as f32 * 90.0),
    }
}

/// `Mth.wrapDegrees(float)`.
fn wrap_degrees(a: f32) -> f32 {
    let mut n = a % 360.0;
    if n >= 180.0 {
        n -= 360.0;
    }
    if n < -180.0 {
        n += 360.0;
    }
    n
}

/// `Mth.fastInvSqrt` (the bit trick, one Newton step).
fn fast_inv_sqrt(x: f64) -> f64 {
    let xhalf = 0.5 * x;
    let i = (x.to_bits() as i64) >> 1;
    let y = f64::from_bits((6_910_469_410_427_058_090i64.wrapping_sub(i)) as u64);
    y * (1.5 - xhalf * y * y)
}

struct AsinTabs {
    asin: [f64; 257],
    cos: [f64; 257],
}

fn asin_tabs() -> &'static AsinTabs {
    static T: OnceLock<AsinTabs> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = AsinTabs { asin: [0.0; 257], cos: [0.0; 257] };
        for ind in 0..257 {
            let a = (ind as f64 / 256.0).asin();
            t.cos[ind] = a.cos();
            t.asin[ind] = a;
        }
        t
    })
}

/// `Mth.atan2(double, double)`: table approximation, ported so the
/// front/back choice matches vanilla at the boundary too.
fn mth_atan2(y: f64, x: f64) -> f64 {
    const FRAC_BIAS: f64 = 17_592_186_044_416.0; // 2^44, the bias the vanilla table uses
    let d2 = x * x + y * y;
    if d2.is_nan() {
        return f64::NAN;
    }
    let (mut x, mut y) = (x, y);
    let neg_y = y < 0.0;
    if neg_y {
        y = -y;
    }
    let neg_x = x < 0.0;
    if neg_x {
        x = -x;
    }
    let steep = y > x;
    if steep {
        std::mem::swap(&mut x, &mut y);
    }
    let rinv = fast_inv_sqrt(d2);
    x *= rinv;
    y *= rinv;
    let yp = FRAC_BIAS + y;
    let index = (yp.to_bits() as u32) as usize;
    let t = asin_tabs();
    let phi = t.asin[index];
    let c_phi = t.cos[index];
    let s_phi = yp - FRAC_BIAS;
    let sd = y * c_phi - x * s_phi;
    let d = (6.0 + sd * sd) * sd * 0.166_666_666_666_666_66;
    let mut theta = phi + d;
    if steep {
        theta = std::f64::consts::FRAC_PI_2 - theta;
    }
    if neg_x {
        theta = std::f64::consts::PI - theta;
    }
    if neg_y {
        theta = -theta;
    }
    theta
}

/// `SignBlock.getSlotPlayerIsFacing`: true for the front text. Uses the
/// player's feet position, not the eyes.
pub fn front_side(st: u16, pos: BlockPos, player: [f64; 3]) -> bool {
    let Some(kind) = kind(st) else { return true };
    let (cx, _, cz) = match kind {
        Kind::Wall => hitbox_center(st),
        _ => (0.5, 0.5, 0.5),
    };
    let xd = player[0] - (f64::from(pos.x) + cx);
    let zd = player[2] - (f64::from(pos.z) + cz);
    let sign_rot = yaw_degrees(st, kind);
    let player_rot = ((mth_atan2(zd, xd) * 180.0) / f64::from(std::f32::consts::PI)) as f32 - 90.0;
    // degreesDifferenceAbs(signYRot, playerYRot)
    wrap_degrees(player_rot - sign_rot).abs() <= 90.0
}

// ------------------------------------------------------------------ placement

fn solid<L: Lvl>(l: &L, p: BlockPos) -> bool {
    let st = l.get(p);
    let b = &rine_data::get().blocks;
    b.has(st, flag::SOLID) || b.has(st, flag::FULL_CUBE)
}

/// `Block.isFaceFull(collisionShape, DOWN)`, cached per shape.
fn bottom_full(st: u16) -> bool {
    static T: OnceLock<Vec<bool>> = OnceLock::new();
    let t = T.get_or_init(|| {
        rine_data::get()
            .blocks
            .shapes
            .iter()
            .map(|boxes| {
                (0..16).all(|i| {
                    (0..16).all(|j| {
                        let (x, z) = ((i as f32 + 0.5) / 16.0, (j as f32 + 0.5) / 16.0);
                        boxes.iter().any(|b| b[1] <= 1e-6 && b[0] <= x && x <= b[3] && b[2] <= z && z <= b[5])
                    })
                })
            })
            .collect()
    });
    t[rine_data::get().blocks.state(st).shape as usize]
}

/// `replacedFluidState.is(Fluids.WATER)`: the water source fluid only.
fn replaced_water<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    use crate::game::fluids::{fluid_of, Kind as FluidKind};
    fluid_of(l.get(pos)).is_some_and(|f| f.kind == FluidKind::Water && f.source)
}

fn with_water(st: u16, water: bool) -> u16 {
    props::setb(st, Pr::Waterlogged, water)
}

/// Wall variant of a standing or ceiling sign item's block, default state.
fn wall_default(st: u16) -> Option<u16> {
    let name = rine_data::get().blocks.blocks.get(block_of(st) as usize)?.name.strip_prefix("minecraft:")?.to_string();
    let wall = match kind_of_name(&name)? {
        Kind::Standing => format!("{}_wall_sign", name.strip_suffix("_sign")?),
        Kind::Ceiling => format!("{}_wall_hanging_sign", name.strip_suffix("_hanging_sign")?),
        _ => return None,
    };
    Some(default_state(&wall))
}

/// `WallSignBlock.getStateForPlacement` / `WallHangingSignBlock.getStateForPlacement`.
fn wall_state<L: Lvl>(l: &L, pos: BlockPos, ctx: &PlaceCtx, wall: u16, water: bool, hanging: bool) -> Option<u16> {
    let dirs = super::plants::ordered_by_nearest(ctx.yaw, ctx.pitch);
    for d in dirs {
        if !(NORTH..=EAST).contains(&d) {
            continue;
        }
        if hanging && rd::axis(d) == rd::axis(ctx.face) {
            continue;
        }
        let s = with_facing(wall, rd::opposite(d));
        let ok = if hanging { wall_hanging_can_place(l, s, pos) } else { wall_survives(l, s, pos) };
        if ok {
            return Some(with_water(s, water));
        }
    }
    None
}

fn wall_survives<L: Lvl>(l: &L, s: u16, pos: BlockPos) -> bool {
    facing_of(s).is_some_and(|f| solid(l, rel(pos, rd::opposite(f))))
}

/// `WallHangingSignBlock.canPlace`.
fn wall_hanging_can_place<L: Lvl>(l: &L, s: u16, pos: BlockPos) -> bool {
    let Some(f) = facing_of(s) else { return false };
    let (cw, ccw) = (rd::clockwise(f), rd::counter_clockwise(f));
    can_attach(l, s, rel(pos, cw), ccw) || can_attach(l, s, rel(pos, ccw), cw)
}

/// `WallHangingSignBlock.canAttachTo`.
fn can_attach<L: Lvl>(l: &L, s: u16, attach: BlockPos, face: u8) -> bool {
    let st = l.get(attach);
    if tables().wall_hanging[block_of(st) as usize] {
        match (facing_of(st), facing_of(s)) {
            (Some(a), Some(b)) => rd::axis(a) == rd::axis(b),
            _ => false,
        }
    } else {
        rd::sturdy(st, face)
    }
}

/// `CeilingHangingSignBlock.getStateForPlacement`.
fn ceiling_state<L: Lvl>(l: &L, st: u16, pos: BlockPos, ctx: &PlaceCtx, water: bool) -> u16 {
    let above = rel(pos, UP);
    let above_st = l.get(above);
    let dir = from_yaw(ctx.yaw);
    let mut attached = !bottom_full(above_st) || ctx.sneaking;
    if !ctx.sneaking && tables().all_hanging[block_of(above_st) as usize] {
        if let Some(f) = facing_of(above_st) {
            if rd::axis(f) == rd::axis(dir) {
                attached = false;
            }
        } else if props::has(above_st, Pr::Rotation) {
            if let Some(d) = dir_of_segment(props::geti(above_st, Pr::Rotation)) {
                if rd::axis(d) == rd::axis(dir) {
                    attached = false;
                }
            }
        }
    }
    let seg = if !attached { segment_of_dir(rd::opposite(dir)) } else { super::super::place::rotation16(ctx.yaw) };
    let s = props::setb(props::seti(st, Pr::Rotation, seg), Pr::Attached, attached);
    with_water(s, water)
}

/// `HangingSignItem.canPlace` / `StandingAndWallBlockItem.canPlace` with the block's own rules.
fn can_place<L: Lvl>(l: &L, s: u16, pos: BlockPos) -> bool {
    match kind(s) {
        Some(Kind::WallHanging) => wall_hanging_can_place(l, s, pos),
        Some(Kind::Wall) => wall_survives(l, s, pos),
        Some(Kind::Standing) => solid(l, rel(pos, DOWN)),
        Some(Kind::Ceiling) => rd::sturdy_center(l.get(rel(pos, UP)), DOWN),
        None => false,
    }
}

/// `StandingAndWallBlockItem.getPlacementState` for the four sign items.
/// `st` is the default state of the item's block (standing or ceiling).
pub fn placement<L: Lvl>(l: &L, st: u16, pos: BlockPos, ctx: &PlaceCtx) -> Option<u16> {
    let kind = kind(st)?;
    let att = if kind.hanging() { UP } else { DOWN };
    let water = replaced_water(l, pos);
    let wall_base = wall_default(st)?;
    let wall = if kind.hanging() {
        wall_state(l, pos, ctx, wall_base, water, true)
    } else {
        wall_state(l, pos, ctx, wall_base, water, false)
    };
    let center = if kind.hanging() {
        Some(ceiling_state(l, st, pos, ctx, water))
    } else {
        Some(with_water(props::seti(st, Pr::Rotation, super::super::place::rotation16(ctx.yaw)), water))
    };
    for d in super::plants::ordered_by_nearest(ctx.yaw, ctx.pitch) {
        if d == rd::opposite(att) {
            continue;
        }
        let cand = if d == att { center } else { wall };
        if let Some(c) = cand {
            if can_place(l, c, pos) {
                return Some(c);
            }
        }
    }
    None
}

// ------------------------------------------------------------------ use

/// Dye, glow ink, ink sac or honeycomb: `SignApplicator` items.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Applicator {
    Dye(u8),
    Glow,
    Ink,
    Wax,
}

fn applicator(item: u32) -> Option<Applicator> {
    if item == 0 {
        return None;
    }
    let t = &tables().items;
    if let Some(i) = t.dye.iter().position(|&d| d == item) {
        return Some(Applicator::Dye(i as u8));
    }
    if item == t.glow {
        Some(Applicator::Glow)
    } else if item == t.ink {
        Some(Applicator::Ink)
    } else if item == t.honeycomb {
        Some(Applicator::Wax)
    } else {
        None
    }
}

fn is_hanging_item(item: u32) -> bool {
    item != 0
        && rine_data::get().registries.reg("minecraft:item").name(item).is_some_and(|n| n.ends_with("_hanging_sign"))
}

/// The state change one applicator makes to a side, if any
/// (`DyeItem.tryApplyToSign` and friends: same value means no change).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SignAct {
    Dye { front: bool, color: u8 },
    Glow { front: bool, on: bool },
    Wax,
    Edit { front: bool },
}

/// An effect on a sign, applied by the caller after the block use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SignOp {
    pub pos: BlockPos,
    pub act: SignAct,
}

/// What the caller knows about the clicked sign (`SignBlockEntity` read at
/// the click). Filled by `snapshot`, empty for other blocks.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SignSnap {
    /// A sign block entity exists at the position.
    pub present: bool,
    pub waxed: bool,
    /// Another player holds the editor (`getPlayerWhoMayEdit`).
    pub other_editing: bool,
    /// `Player.mayBuild`.
    pub may_build: bool,
    pub front: SideSnap,
    pub back: SideSnap,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SideSnap {
    pub has_message: bool,
    pub editable: bool,
    pub glow: bool,
    pub color: u8,
}

impl SideSnap {
    fn of(s: &Side) -> Self {
        SideSnap { has_message: s.has_message(), editable: s.editable(), glow: s.glow, color: s.color }
    }
}

impl SignSnap {
    fn side(&self, front: bool) -> SideSnap {
        if front {
            self.front
        } else {
            self.back
        }
    }
}

/// `SignBlock.useItemOn` plus the hanging-sign chain check of
/// `CeilingHangingSignBlock` / `WallHangingSignBlock` (PASS, so the item
/// goes on to placement).
pub fn use_item_on<L: Lvl>(_rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, ctx: &UseCtx, _rng: &mut Rng) -> Res {
    let Some(kind) = kind(st) else { return Res::Pass };
    let snap = ctx.sign;
    if !snap.present {
        return Res::Pass;
    }
    if kind.hanging() && is_hanging_item(ctx.item) {
        let chain = match kind {
            Kind::Ceiling => ctx.face == DOWN,
            _ => facing_of(st).is_some_and(|f| rd::axis(f) != rd::axis(ctx.face)),
        };
        if chain {
            return Res::Pass;
        }
    }
    let Some(app) = applicator(ctx.item) else { return Res::TryEmptyHand };
    if !(snap.may_build && !snap.waxed && !snap.other_editing) {
        return Res::TryEmptyHand;
    }
    let front = front_side(st, pos, ctx.pos);
    let side = snap.side(front);
    // canApplyToSign: text must exist (honeycomb always applies)
    let can = app == Applicator::Wax || side.has_message;
    if !can {
        return Res::TryEmptyHand;
    }
    let act = match app {
        Applicator::Dye(c) if c != side.color => SignAct::Dye { front, color: c },
        Applicator::Glow if !side.glow => SignAct::Glow { front, on: true },
        Applicator::Ink if side.glow => SignAct::Glow { front, on: false },
        Applicator::Wax => SignAct::Wax,
        _ => return Res::TryEmptyHand,
    };
    match act {
        SignAct::Dye { .. } => l.sound("item.dye.use", pos, 1.0, 1.0, CAT_BLOCKS, false),
        SignAct::Glow { on: true, .. } => l.sound("item.glow_ink_sac.use", pos, 1.0, 1.0, CAT_BLOCKS, false),
        SignAct::Glow { on: false, .. } => l.sound("item.ink_sac.use", pos, 1.0, 1.0, CAT_BLOCKS, false),
        SignAct::Wax => {
            // PARTICLES_WAX_ON, then HONEYCOMB_WAX_ON (setWaxed, levelEvent, playSound)
            l.level_event(3003, pos, 0);
            l.sound("item.honeycomb.wax_on", pos, 1.0, 1.0, CAT_BLOCKS, false);
        }
        SignAct::Edit { .. } => {}
    }
    Res::Done(UseOut { item: ItemOp::Shrink(1), sign: Some(SignOp { pos, act }), ..UseOut::default() })
}

/// `SignBlock.useWithoutItem`: waxed signs play the failure sound, others
/// open the editor for the side the player faces.
pub fn use_without_item<L: Lvl>(_rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, ctx: &UseCtx, _rng: &mut Rng) -> Res {
    let Some(kind) = kind(st) else { return Res::Pass };
    let snap = ctx.sign;
    if !snap.present {
        return Res::Pass;
    }
    if snap.waxed {
        let sound = if kind.hanging() { "block.hanging_sign.waxed_interact_fail" } else { "block.sign.waxed_interact_fail" };
        l.sound(sound, pos, 1.0, 1.0, CAT_BLOCKS, false);
        return Res::swing();
    }
    let front = front_side(st, pos, ctx.pos);
    if !snap.other_editing && snap.may_build && snap.side(front).editable {
        return Res::Done(UseOut { sign: Some(SignOp { pos, act: SignAct::Edit { front } }), ..UseOut::default() });
    }
    Res::Pass
}

// ------------------------------------------------------------------ editors

/// `playerWhoMayEdit` for every sign: one entry per open editor.
#[derive(Default)]
pub struct Editors(Vec<(BlockPos, Uuid)>);

impl Editors {
    /// `setAllowedPlayerEditor(uuid)`.
    pub fn open(&mut self, pos: BlockPos, uuid: Uuid) {
        self.close(pos);
        self.0.push((pos, uuid));
    }

    /// `setAllowedPlayerEditor(null)`.
    pub fn close(&mut self, pos: BlockPos) {
        self.0.retain(|(p, _)| *p != pos);
    }

    pub fn get(&self, pos: BlockPos) -> Option<Uuid> {
        self.0.iter().find(|(p, _)| *p == pos).map(|(_, u)| *u)
    }

    pub fn retain(&mut self, mut keep: impl FnMut(BlockPos, Uuid) -> bool) {
        self.0.retain(|(p, u)| keep(*p, *u));
    }
}

/// Whether `player` may still edit a sign: `playerIsTooFarAwayToEdit`
/// (`isWithinBlockInteractionRange(pos, 4.0)`).
pub fn within_edit_range(eye: [f64; 3], pos: BlockPos, creative: bool) -> bool {
    let range = 4.5 + if creative { 0.5 } else { 0.0 } + 4.0;
    let mut d2 = 0.0;
    for (a, lo) in [pos.x, pos.y, pos.z].into_iter().enumerate() {
        let lo = f64::from(lo);
        let (hi, e) = (lo + 1.0, eye[a]);
        let d = if e < lo {
            lo - e
        } else if e > hi {
            e - hi
        } else {
            0.0
        };
        d2 += d * d;
    }
    d2 < range * range
}

static EDITORS: Mutex<Editors> = Mutex::new(Editors(Vec::new()));

fn editors() -> MutexGuard<'static, Editors> {
    EDITORS.lock().unwrap_or_else(|e| e.into_inner())
}

fn chunk_pos(p: BlockPos) -> ChunkPos {
    ChunkPos::new(p.x >> 4, p.z >> 4)
}

/// The editor of a sign if that player is still online and in range.
fn editor_of(g: &Game, pos: BlockPos) -> Option<Uuid> {
    let u = editors().get(pos)?;
    let p = g.players.iter().flatten().find(|p| p.uuid == u)?;
    within_edit_range(p.eye(), pos, p.gamemode == 1).then_some(u)
}

/// Drops editors whose player left, walked away, or whose sign is gone.
/// Runs once per game tick.
pub fn tick(g: &Game) {
    editors().retain(|pos, uuid| {
        kind(g.world.get_block(pos.x, pos.y, pos.z)) .is_some()
            && g.players.iter().flatten().any(|p| p.uuid == uuid && within_edit_range(p.eye(), pos, p.gamemode == 1))
    });
}

/// Client-facing snapshot of the sign at `pos` for `UseCtx::sign`.
pub fn snapshot(g: &Game, pos: BlockPos, uuid: Uuid, may_build: bool) -> SignSnap {
    let Some(slot) = g.world.chunks.get(&chunk_pos(pos)) else { return SignSnap::default() };
    let Some(be) = blockent::find(&slot.chunk, pos) else { return SignSnap::default() };
    let nbt = be.nbt.as_ref();
    SignSnap {
        present: true,
        waxed: is_waxed(nbt),
        other_editing: editor_of(g, pos).is_some_and(|u| u != uuid),
        may_build,
        front: SideSnap::of(&side_of(nbt, true)),
        back: SideSnap::of(&side_of(nbt, false)),
    }
}

/// `ClientboundBlockEntityDataPacket` (framed) for a sign.
fn be_packet(pos: BlockPos, type_id: u32, nbt: &Tag) -> bytes::Bytes {
    let p = packet(C_BLOCK_ENTITY_DATA, |b| {
        b.w_i64(pos.pack());
        b.w_varint(type_id as i32);
        nbt.write_network(b);
    });
    crate::out::frame(p)
}

/// Sends a sign's block entity data to the players tracking its chunk.
fn broadcast_sign(g: &mut Game, pos: BlockPos, type_id: u32, nbt: &Tag) {
    let f = be_packet(pos, type_id, nbt);
    g.send_chunk_viewers_except(chunk_pos(pos), &f, None);
}

/// `SignBlock.openTextEdit`: the editor belongs to this player, then the
/// client gets the block and the editor screen.
pub fn open_editor(g: &mut Game, slot: usize, pos: BlockPos, front: bool) {
    let st = g.world.get_block(pos.x, pos.y, pos.z);
    let Some(p) = g.player(slot) else { return };
    let uuid = p.uuid;
    editors().open(pos, uuid);
    p.send(crate::out::block_update(pos, st));
    p.send(packet(C_OPEN_SIGN_EDITOR, |b| {
        b.w_i64(pos.pack());
        b.w_varint(if front { 1 } else { 0 });
    }));
}

/// `SignBlock.setPlacedBy`: a new sign opens the front text for its placer
/// when it is not waxed and the front text is editable.
pub fn on_placed(g: &mut Game, slot: usize, pos: BlockPos) {
    let st = g.world.get_block(pos.x, pos.y, pos.z);
    if kind(st).is_none() {
        return;
    }
    let Some(ch) = g.world.chunks.get(&chunk_pos(pos)) else { return };
    let Some(be) = blockent::find(&ch.chunk, pos) else { return };
    let nbt = be.nbt.as_ref();
    if is_waxed(nbt) || !side_of(nbt, true).editable() {
        return;
    }
    open_editor(g, slot, pos, true);
}

/// Applies a `SignOp` from a block use (not `Edit`, which opens the editor).
pub fn apply(g: &mut Game, slot: usize, op: SignOp) {
    if let SignAct::Edit { front } = op.act {
        open_editor(g, slot, op.pos, front);
        return;
    }
    let cp = chunk_pos(op.pos);
    let (type_id, nbt) = {
        let Some(ch) = g.world.chunks.get_mut(&cp) else { return };
        let Some(be) = blockent::find_mut(&mut ch.chunk, op.pos) else { return };
        let nbt = be.nbt.get_or_insert_with(fresh_nbt_value);
        match op.act {
            SignAct::Dye { front, color } => {
                let mut s = side_of(Some(&*nbt), front);
                s.color = color;
                put_side(nbt, front, &s);
            }
            SignAct::Glow { front, on } => {
                let mut s = side_of(Some(&*nbt), front);
                s.glow = on;
                put_side(nbt, front, &s);
            }
            SignAct::Wax => blockent::put(nbt, "is_waxed", Tag::Byte(1)),
            SignAct::Edit { .. } => return,
        }
        let out = (be.type_id, nbt.clone());
        ch.chunk.dirty = true;
        out
    };
    broadcast_sign(g, op.pos, type_id, &nbt);
}

/// `ServerGamePacketListenerImpl.handleSignUpdate` -> `SignBlockEntity.updateSignText`.
/// `side` is the `SignTextSlot` id (BACK = 0, FRONT = 1). Lines are the raw
/// packet strings.
pub fn handle_update(g: &mut Game, slot: usize, pos: BlockPos, lines: Vec<String>, side: i32) {
    let Some(p) = g.player(slot) else { return };
    let uuid = p.uuid;
    let front = side == 1;
    let cp = chunk_pos(pos);
    let Some(ch) = g.world.chunks.get(&cp) else { return };
    let Some(be) = blockent::find(&ch.chunk, pos) else { return };
    if kind(g.world.get_block(pos.x, pos.y, pos.z)).is_none() {
        return;
    }
    let type_id = be.type_id;
    let nbt = be.nbt.clone();
    let waxed = is_waxed(nbt.as_ref());
    let may_edit = editor_of(g, pos) == Some(uuid);
    if !waxed && may_edit {
        let mut stripped: Vec<String> = lines.iter().map(|l| strip_formatting(l)).collect();
        stripped.resize(LINES, String::new());
        let mut nbt = nbt.unwrap_or_else(fresh_nbt_value);
        let mut s = side_of(Some(&nbt), front);
        s.set_lines(&stripped);
        put_side(&mut nbt, front, &s);
        if let Some(ch) = g.world.chunks.get_mut(&cp) {
            if let Some(be) = blockent::find_mut(&mut ch.chunk, pos) {
                be.nbt = Some(nbt.clone());
            }
            ch.chunk.dirty = true;
        }
        editors().close(pos);
        broadcast_sign(g, pos, type_id, &nbt);
    }
    // A rejected update only logs in vanilla (`LOGGER.warn`); the CraftBukkit
    // resend and the Paper distance check are not ported.
}

#[cfg(test)]
mod tests {
    use super::super::testw::World;
    use super::super::interact::UseCtx;
    use super::*;

    fn bp(x: i32, y: i32, z: i32) -> BlockPos {
        BlockPos::new(x, y, z)
    }

    fn sign_ctx(item: &str, face: u8, feet: [f64; 3], sign: SignSnap) -> UseCtx {
        UseCtx {
            item: item_id(item),
            count: 1,
            damage: 0,
            main_hand: true,
            creative: false,
            sneaking: false,
            face,
            cursor: (0.5, 0.5, 0.5),
            yaw: 0.0,
            pos: feet,
            can_eat: false,
            other: 0,
            sign,
        }
    }

    fn snap(front: SideSnap, back: SideSnap, waxed: bool) -> SignSnap {
        SignSnap { present: true, waxed, other_editing: false, may_build: true, front, back }
    }

    fn text_side() -> SideSnap {
        SideSnap { has_message: true, editable: true, glow: false, color: BLACK }
    }

    fn empty_side() -> SideSnap {
        SideSnap { has_message: false, editable: true, glow: false, color: BLACK }
    }

    #[test]
    fn fresh_nbt_shape() {
        let t = fresh_nbt_value();
        let front = t.get("front_text").unwrap();
        assert_eq!(front.get("color"), Some(&Tag::String("black".into())));
        assert_eq!(front.get("has_glowing_text"), Some(&Tag::Byte(0)));
        assert!(front.get("filtered_messages").is_none());
        assert_eq!(t.get("is_waxed"), Some(&Tag::Byte(0)));
        assert_eq!(Side::from_tag(t.get("front_text")), fresh_side());
    }

    #[test]
    fn nbt_round_trip_keeps_raw_text_and_style() {
        let mut nbt = fresh_nbt_value();
        let mut s = fresh_side();
        // a styled first line; a control character on the third line stays
        // (no text filter: vanilla never writes filtered_messages from an edit)
        s.messages[0] = Tag::Compound(vec![("text".into(), Tag::String("a".into())), ("color".into(), Tag::String("red".into()))]);
        s.set_lines(&["a".into(), "b".into(), "c\u{1}".into(), "".into()]);
        s.color = 2;
        s.glow = true;
        put_side(&mut nbt, true, &s);
        let back = Side::from_tag(nbt.get("front_text"));
        assert_eq!(back, s);
        assert_eq!(back.filtered, None);
        assert_eq!(back.messages[2], Tag::String("c\u{1}".into()));
        assert!(back.to_tag().get("filtered_messages").is_none());
        // the red style of the first line survives the edit
        assert_eq!(plain_text(&back.messages[0]), Some("a"));
        assert!(matches!(&back.messages[0], Tag::Compound(_)));
        // a value the chunk stores with filtered_messages reads back and is kept
        let mut vanilla = fresh_side();
        vanilla.filtered = Some(vec![Tag::String("x".into()), Tag::String("".into()), Tag::String("".into()), Tag::String("".into())]);
        vanilla.messages[0] = Tag::String("x\u{1}".into());
        assert_eq!(Side::from_tag(Some(&vanilla.to_tag())), vanilla);
    }

    #[test]
    fn unreadable_text_is_empty() {
        let mut t = fresh_nbt_value();
        blockent::put(&mut t, "front_text", Tag::compound().with("messages", Tag::List(vec![Tag::String("x".into())])));
        assert_eq!(Side::from_tag(t.get("front_text")), fresh_side());
    }

    #[test]
    fn formatting_codes_are_stripped() {
        assert_eq!(strip_formatting("\u{a7}cRed\u{a7}LB\u{a7}z"), "RedB\u{a7}z");
        assert_eq!(strip_formatting("\u{a7}"), "\u{a7}");
    }

    #[test]
    fn sign_nbt_is_what_data_reads_and_writes() {
        use crate::game::cmd::commands::data::block_entity_nbt;
        use crate::game::redstone::game_tests::setup;
        let mut g = setup("signs_data");
        let o = BlockPos::new(g.spawn[0] as i32 - 2, 200, g.spawn[2] as i32 - 2);
        for x in -3..=3 {
            for z in -3..=3 {
                g.world.set_block(o.offset(x, -1, z), default_state("stone"));
            }
        }
        g.do_tick();
        let st = props::seti(default_state("oak_sign"), Pr::Rotation, 8);
        g.world.set_block(o, st);
        let t = [o.x, o.y, o.z];
        // a placed sign carries the vanilla fields
        let nbt = block_entity_nbt(&g, t).expect("sign block entity");
        assert!(nbt.get("front_text").is_some() && nbt.get("back_text").is_some(), "{nbt:?}");
        assert_eq!(nbt.get("is_waxed"), Some(&Tag::Byte(0)));
        assert!(!snapshot(&g, o, Uuid::nil(), true).front.has_message);
        // `/data merge` writes the chunk's nbt; the sign reads it back
        let cp = ChunkPos::new(o.x >> 4, o.z >> 4);
        let slot = g.world.chunks.get_mut(&cp).expect("chunk");
        let be = blockent::find_mut(&mut slot.chunk, o).expect("be");
        let mut merged = be.nbt.clone().expect("nbt");
        let mut side = Side::from_tag(merged.get("front_text"));
        side.set_lines(&["hello".into(), "".into(), "".into(), "".into()]);
        put_side(&mut merged, true, &side);
        be.nbt = Some(merged);
        let s = snapshot(&g, o, Uuid::nil(), true);
        assert!(s.front.has_message && s.front.editable);
        assert!(!s.back.has_message);
        assert_eq!(block_entity_nbt(&g, t).and_then(|n| n.get("front_text").cloned()).is_some(), true);
    }

    #[test]
    fn standing_sign_front_faces_its_rotation() {
        rine_data::init();
        // rotation 8 = 180 degrees: front looks north, so a player north of the sign is in front
        let st = props::seti(default_state("oak_sign"), Pr::Rotation, 8);
        let p = bp(0, 64, 0);
        assert!(front_side(st, p, [0.5, 64.0, -3.0]));
        assert!(!front_side(st, p, [0.5, 64.0, 3.0]));
    }

    #[test]
    fn wall_sign_front_faces_away_from_wall() {
        rine_data::init();
        // facing north: the wall is the south face of the block, front is north
        let st = with_facing(default_state("oak_wall_sign"), NORTH);
        let p = bp(0, 64, 0);
        assert!(front_side(st, p, [0.5, 64.0, -2.5]));
        assert!(!front_side(st, p, [0.5, 64.0, 2.5]));
        // the back of a south-facing wall sign is the north side
        let st2 = with_facing(default_state("oak_wall_sign"), SOUTH);
        assert!(front_side(st2, p, [0.5, 64.0, 2.5]));
        assert!(!front_side(st2, p, [0.5, 64.0, -2.5]));
    }

    #[test]
    fn atan2_matches_libm_closely() {
        for &(y, x) in &[(1.0, 1.0), (0.3, -2.0), (-4.0, 0.5), (0.0, -1.0), (2.0, 0.0)] {
            let a = mth_atan2(y, x);
            let b = (y as f64).atan2(x);
            assert!((a - b).abs() < 1e-3, "{y} {x}: {a} vs {b}");
        }
    }

    #[test]
    fn honeycomb_waxes_unwaxed_sign_and_consumes_one() {
        rine_data::init();
        let mut w = World::new();
        let p = bp(0, 64, 0);
        let st = default_state("oak_sign");
        w.put(p, st);
        let mut rs = Rs::default();
        let mut rng = Rng::derive(1, 0, 0);
        let ctx = sign_ctx("honeycomb", DOWN, [0.5, 64.0, 2.5], snap(text_side(), text_side(), false));
        let r = use_item_on(&mut rs, &mut w, st, p, &ctx, &mut rng);
        assert_eq!(r, Res::Done(UseOut { item: ItemOp::Shrink(1), sign: Some(SignOp { pos: p, act: SignAct::Wax }), ..UseOut::default() }));
        assert_eq!(w.events, vec![(3003, p, 0)]);
        assert_eq!(w.sounds.last().map(|s| s.0.as_str()), Some("item.honeycomb.wax_on"));
    }

    #[test]
    fn waxed_sign_refuses_items_and_plays_fail_sound() {
        rine_data::init();
        let mut w = World::new();
        let p = bp(0, 64, 0);
        let st = default_state("oak_sign");
        w.put(p, st);
        let mut rs = Rs::default();
        let mut rng = Rng::derive(1, 0, 0);
        let waxed = snap(text_side(), text_side(), true);
        let ctx = sign_ctx("dye_white", DOWN, [0.5, 64.0, 2.5], waxed);
        assert_eq!(use_item_on(&mut rs, &mut w, st, p, &ctx, &mut rng), Res::TryEmptyHand);
        assert!(w.sounds.is_empty());
        let r = use_without_item(&mut rs, &mut w, st, p, &sign_ctx("air", DOWN, [0.5, 64.0, 2.5], waxed), &mut rng);
        assert_eq!(r, Res::swing());
        assert_eq!(w.sounds.last().map(|s| s.0.as_str()), Some("block.sign.waxed_interact_fail"));
    }

    #[test]
    fn dye_needs_a_different_color_and_some_text() {
        rine_data::init();
        let mut w = World::new();
        let p = bp(0, 64, 0);
        let st = default_state("oak_sign");
        w.put(p, st);
        let mut rs = Rs::default();
        let mut rng = Rng::derive(1, 0, 0);
        let feet = [0.5, 64.0, 2.5];
        // same color: black dye on black text does nothing
        let dye_black = sign_ctx("black_dye", DOWN, feet, snap(text_side(), text_side(), false));
        assert_eq!(use_item_on(&mut rs, &mut w, st, p, &dye_black, &mut rng), Res::TryEmptyHand);
        // empty text cannot be dyed
        let dye_red = sign_ctx("red_dye", DOWN, feet, snap(empty_side(), empty_side(), false));
        assert_eq!(use_item_on(&mut rs, &mut w, st, p, &dye_red, &mut rng), Res::TryEmptyHand);
        // different color on text: consumes one, changes the front side (player faces north side: the front)
        let r = use_item_on(&mut rs, &mut w, st, p, &dye_red, &mut rng);
        assert_eq!(r, Res::TryEmptyHand);
        let ok = sign_ctx("red_dye", DOWN, feet, snap(empty_side(), text_side(), false));
        let r = use_item_on(&mut rs, &mut w, st, p, &ok, &mut rng);
        assert!(matches!(r, Res::Done(UseOut { sign: Some(SignOp { act: SignAct::Dye { color: 14, .. }, .. }), .. })), "{r:?}");
        assert_eq!(w.sounds.last().map(|s| s.0.as_str()), Some("item.dye.use"));
    }

    #[test]
    fn ink_and_glow_toggle_the_glow_flag() {
        rine_data::init();
        let mut w = World::new();
        let p = bp(0, 64, 0);
        let st = default_state("oak_sign");
        w.put(p, st);
        let mut rs = Rs::default();
        let mut rng = Rng::derive(1, 0, 0);
        let feet = [0.5, 64.0, 2.5];
        let glowing = SideSnap { glow: true, ..text_side() };
        let glow_again = sign_ctx("glow_ink_sac", DOWN, feet, snap(glowing, glowing, false));
        assert_eq!(use_item_on(&mut rs, &mut w, st, p, &glow_again, &mut rng), Res::TryEmptyHand);
        let ink = sign_ctx("ink_sac", DOWN, feet, snap(glowing, glowing, false));
        assert!(matches!(use_item_on(&mut rs, &mut w, st, p, &ink, &mut rng), Res::Done(UseOut { sign: Some(SignOp { act: SignAct::Glow { on: false, .. }, .. }), .. })));
        let plain_ink = sign_ctx("ink_sac", DOWN, feet, snap(text_side(), text_side(), false));
        assert_eq!(use_item_on(&mut rs, &mut w, st, p, &plain_ink, &mut rng), Res::TryEmptyHand);
    }

    #[test]
    fn empty_hand_opens_editor_for_facing_side_only_when_editable_and_free() {
        rine_data::init();
        let mut w = World::new();
        let p = bp(0, 64, 0);
        let st = default_state("oak_sign");
        w.put(p, st);
        let mut rs = Rs::default();
        let mut rng = Rng::derive(1, 0, 0);
        let feet = [0.5, 64.0, -3.0]; // north of a rotation-0 sign: back faces north
        let s = snap(text_side(), empty_side(), false);
        let r = use_without_item(&mut rs, &mut w, st, p, &sign_ctx("air", DOWN, feet, s), &mut rng);
        let st_rot0 = props::seti(st, Pr::Rotation, 8);
        let r2 = use_without_item(&mut rs, &mut w, st_rot0, p, &sign_ctx("air", DOWN, feet, s), &mut rng);
        assert!(matches!(r2, Res::Done(UseOut { sign: Some(SignOp { act: SignAct::Edit { front: true }, .. }), .. })), "{r2:?}");
        let _ = r;
        // someone else is editing: pass
        let busy = SignSnap { other_editing: true, ..s };
        assert_eq!(use_without_item(&mut rs, &mut w, st_rot0, p, &sign_ctx("air", DOWN, feet, busy), &mut rng), Res::Pass);
        // adventure: may not build
        let adv = SignSnap { may_build: false, ..s };
        assert_eq!(use_without_item(&mut rs, &mut w, st_rot0, p, &sign_ctx("air", DOWN, feet, adv), &mut rng), Res::Pass);
    }

    #[test]
    fn editor_ownership_and_range() {
        let mut e = Editors::default();
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let p = bp(3, 70, 3);
        e.open(p, a);
        assert_eq!(e.get(p), Some(a));
        e.open(p, b);
        assert_eq!(e.get(p), Some(b));
        e.close(p);
        assert_eq!(e.get(p), None);
        // within 4.5 + 4.0 of the box (survival), not beyond 8.5
        assert!(within_edit_range([3.5, 71.6, 3.5], p, false));
        assert!(!within_edit_range([3.5, 71.6, 20.0], p, false));
        // creative adds 0.5 to the interaction range
        assert!(within_edit_range([3.5, 71.6, 12.5], p, true));
        assert!(!within_edit_range([3.5, 71.6, 13.5], p, true));
    }

    #[test]
    fn standing_sign_placed_on_floor_and_wall_sign_on_wall() {
        rine_data::init();
        let mut w = World::new();
        let support = bp(0, 63, 0);
        w.put(support, default_state("stone"));
        let pos = bp(0, 64, 0);
        // clicked the top of the block below: standing sign
        let ctx = PlaceCtx { face: UP, cursor: (0.5, 0.0, 0.5), yaw: 0.0, pitch: 0.0, sneaking: false, clicked: support };
        let st = placement(&w, default_state("oak_sign"), pos, &ctx).expect("standing");
        assert_eq!(kind(st), Some(Kind::Standing));
        assert_eq!(props::geti(st, Pr::Rotation), 8);
        // a wall at the north: the sign goes on it facing south
        w.put(bp(0, 64, -1), default_state("stone"));
        let ctx = PlaceCtx { face: SOUTH, cursor: (0.5, 0.5, 0.0), yaw: 0.0, pitch: 0.0, sneaking: false, clicked: bp(0, 64, -1) };
        w.put(support, default_state("air"));
        let st = placement(&w, default_state("oak_sign"), pos, &ctx);
        assert!(st.is_some_and(|s| kind(s) == Some(Kind::Wall)), "{st:?}");
    }
}
