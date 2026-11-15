//! Coordinates: `WorldCoordinate(s)`, `LocalCoordinates`, and the
//! block_pos / column_pos / vec3 / vec2 / rotation / swizzle parsers.

use crate::error::{CmdError, CmdResult};
use crate::reader::Reader;
use rine_protocol::text::Text;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldCoord {
    pub relative: bool,
    pub value: f64,
}

impl WorldCoord {
    pub const REL_ZERO: WorldCoord = WorldCoord { relative: true, value: 0.0 };
    #[inline]
    pub fn get(&self, original: f64) -> f64 {
        if self.relative {
            self.value + original
        } else {
            self.value
        }
    }
}

/// `Coordinates`: world (`~`/absolute per axis) or local (`^`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Coords {
    /// x, y, z. For rotations: x = pitch (xRot), y = yaw (yRot).
    World([WorldCoord; 3]),
    /// left, up, forwards.
    Local([f64; 3]),
}

/// What resolving coordinates needs from a command source.
#[derive(Debug, Clone, Copy)]
pub struct SourcePose {
    pub pos: [f64; 3],
    /// (xRot = pitch, yRot = yaw), vanilla `Vec2(x, y)`.
    pub rot: [f32; 2],
    /// Position local coordinates start from (feet or eyes anchor).
    pub anchor_pos: [f64; 3],
}

/// Vanilla `Mth.sin(deg * DEG_TO_RAD)`: float radians widened to double,
/// 65536-entry table.
#[inline]
pub fn mth_sin(deg: f32) -> f32 {
    let i = (((deg * DEG_TO_RAD) as f64 * 10430.378350470453) as i64 & 65535) as usize;
    SIN_TABLE.with(|t| t[i])
}
#[inline]
pub fn mth_cos(deg: f32) -> f32 {
    let i = (((deg * DEG_TO_RAD) as f64 * 10430.378350470453 + 16384.0) as i64 & 65535) as usize;
    SIN_TABLE.with(|t| t[i])
}
pub const DEG_TO_RAD: f32 = (std::f64::consts::PI / 180.0) as f32;

thread_local! {
    static SIN_TABLE: Vec<f32> = (0..65536).map(|i| ((i as f64) / 10430.378350470453).sin() as f32).collect();
}

impl Coords {
    pub fn position(&self, s: &SourcePose) -> [f64; 3] {
        match self {
            Coords::World(c) => [c[0].get(s.pos[0]), c[1].get(s.pos[1]), c[2].get(s.pos[2])],
            Coords::Local([left, up, fwd]) => {
                let (rx, ry) = (s.rot[0], s.rot[1]);
                let y_cos = mth_cos(ry + 90.0);
                let y_sin = mth_sin(ry + 90.0);
                let x_cos = mth_cos(-rx);
                let x_sin = mth_sin(-rx);
                let x_cos_up = mth_cos(-rx + 90.0);
                let x_sin_up = mth_sin(-rx + 90.0);
                let f = [(y_cos * x_cos) as f64, x_sin as f64, (y_sin * x_cos) as f64];
                let u = [(y_cos * x_cos_up) as f64, x_sin_up as f64, (y_sin * x_cos_up) as f64];
                // left = forwards x up, scaled by -1
                let l = [-(f[1] * u[2] - f[2] * u[1]), -(f[2] * u[0] - f[0] * u[2]), -(f[0] * u[1] - f[1] * u[0])];
                let a = s.anchor_pos;
                [
                    a[0] + f[0] * fwd + u[0] * up + l[0] * left,
                    a[1] + f[1] * fwd + u[1] * up + l[1] * left,
                    a[2] + f[2] * fwd + u[2] * up + l[2] * left,
                ]
            }
        }
    }
    /// `BlockPos.containing(getPosition())`.
    pub fn block_pos(&self, s: &SourcePose) -> [i32; 3] {
        let p = self.position(s);
        [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32]
    }
    /// Rotation as (xRot, yRot); local coordinates give zero.
    pub fn rotation(&self, s: &SourcePose) -> [f32; 2] {
        match self {
            Coords::World(c) => [c[0].get(s.rot[0] as f64) as f32, c[1].get(s.rot[1] as f64) as f32],
            Coords::Local(_) => [0.0, 0.0],
        }
    }
    pub fn is_relative(&self, axis: usize) -> bool {
        match self {
            Coords::World(c) => c[axis].relative,
            Coords::Local(_) => true,
        }
    }
}

pub fn err_mixed(r: &Reader) -> CmdError {
    CmdError::at(Text::key("argument.pos.mixed"), r)
}
fn err_incomplete3(r: &Reader) -> CmdError {
    CmdError::at(Text::key("argument.pos3d.incomplete"), r)
}
fn err_incomplete2(r: &Reader) -> CmdError {
    CmdError::at(Text::key("argument.pos2d.incomplete"), r)
}

pub fn world_double(r: &mut Reader, center: bool) -> CmdResult<WorldCoord> {
    if r.can_read() && r.peek() == '^' {
        return Err(err_mixed(r));
    }
    if !r.can_read() {
        return Err(CmdError::at(Text::key("argument.pos.missing.double"), r));
    }
    let relative = is_relative(r);
    let start = r.cursor;
    let mut value = if r.can_read() && r.peek() != ' ' { r.read_double()? } else { 0.0 };
    let number = r.slice(start, r.cursor);
    if relative && number.is_empty() {
        return Ok(WorldCoord { relative: true, value: 0.0 });
    }
    if !number.contains('.') && !relative && center {
        value += 0.5;
    }
    Ok(WorldCoord { relative, value })
}

pub fn world_int(r: &mut Reader) -> CmdResult<WorldCoord> {
    if r.can_read() && r.peek() == '^' {
        return Err(err_mixed(r));
    }
    if !r.can_read() {
        return Err(CmdError::at(Text::key("argument.pos.missing.int"), r));
    }
    let relative = is_relative(r);
    let value = if r.can_read() && r.peek() != ' ' {
        if relative {
            r.read_double()?
        } else {
            r.read_int()? as f64
        }
    } else {
        0.0
    };
    Ok(WorldCoord { relative, value })
}

fn is_relative(r: &mut Reader) -> bool {
    if r.peek() == '~' {
        r.skip();
        true
    } else {
        false
    }
}

fn three(r: &mut Reader, mut f: impl FnMut(&mut Reader, usize) -> CmdResult<WorldCoord>) -> CmdResult<Coords> {
    let start = r.cursor;
    let x = f(r, 0)?;
    if r.can_read() && r.peek() == ' ' {
        r.skip();
        let y = f(r, 1)?;
        if r.can_read() && r.peek() == ' ' {
            r.skip();
            let z = f(r, 2)?;
            return Ok(Coords::World([x, y, z]));
        }
    }
    r.cursor = start;
    Err(err_incomplete3(r))
}

fn local(r: &mut Reader) -> CmdResult<Coords> {
    let start = r.cursor;
    let read = |r: &mut Reader| -> CmdResult<f64> {
        if !r.can_read() {
            return Err(CmdError::at(Text::key("argument.pos.missing.double"), r));
        }
        if r.peek() != '^' {
            r.cursor = start;
            return Err(err_mixed(r));
        }
        r.skip();
        if r.can_read() && r.peek() != ' ' {
            r.read_double()
        } else {
            Ok(0.0)
        }
    };
    let left = read(r)?;
    if r.can_read() && r.peek() == ' ' {
        r.skip();
        let up = read(r)?;
        if r.can_read() && r.peek() == ' ' {
            r.skip();
            let fwd = read(r)?;
            return Ok(Coords::Local([left, up, fwd]));
        }
    }
    r.cursor = start;
    Err(err_incomplete3(r))
}

pub fn parse_block_pos(r: &mut Reader) -> CmdResult<Coords> {
    if r.can_read() && r.peek() == '^' {
        local(r)
    } else {
        three(r, |r, _| world_int(r))
    }
}

pub fn parse_vec3(r: &mut Reader, center: bool) -> CmdResult<Coords> {
    if r.can_read() && r.peek() == '^' {
        local(r)
    } else {
        three(r, |r, axis| world_double(r, center && axis != 1))
    }
}

pub fn parse_vec2(r: &mut Reader, center: bool) -> CmdResult<Coords> {
    let start = r.cursor;
    if !r.can_read() {
        return Err(err_incomplete2(r));
    }
    let x = world_double(r, center)?;
    if r.can_read() && r.peek() == ' ' {
        r.skip();
        let z = world_double(r, center)?;
        return Ok(Coords::World([x, WorldCoord::REL_ZERO, z]));
    }
    r.cursor = start;
    Err(err_incomplete2(r))
}

pub fn parse_column_pos(r: &mut Reader) -> CmdResult<Coords> {
    let start = r.cursor;
    if !r.can_read() {
        return Err(err_incomplete2(r));
    }
    let x = world_int(r)?;
    if r.can_read() && r.peek() == ' ' {
        r.skip();
        let z = world_int(r)?;
        return Ok(Coords::World([x, WorldCoord::REL_ZERO, z]));
    }
    r.cursor = start;
    Err(err_incomplete2(r))
}

pub fn parse_rotation(r: &mut Reader) -> CmdResult<Coords> {
    let start = r.cursor;
    if !r.can_read() {
        return Err(CmdError::at(Text::key("argument.rotation.incomplete"), r));
    }
    let y = world_double(r, false)?;
    if r.can_read() && r.peek() == ' ' {
        r.skip();
        let x = world_double(r, false)?;
        return Ok(Coords::World([x, y, WorldCoord::REL_ZERO]));
    }
    r.cursor = start;
    Err(CmdError::at(Text::key("argument.rotation.incomplete"), r))
}

/// Axis bit set: 1 = x, 2 = y, 4 = z.
pub fn parse_swizzle(r: &mut Reader) -> CmdResult<u8> {
    let mut set = 0u8;
    while r.can_read() && r.peek() != ' ' {
        let c = r.read();
        let bit = match c {
            'x' => 1,
            'y' => 2,
            'z' => 4,
            _ => return Err(CmdError::at(Text::key("arguments.swizzle.invalid"), r)),
        };
        if set & bit != 0 {
            return Err(CmdError::at(Text::key("arguments.swizzle.invalid"), r));
        }
        set |= bit;
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn centering() {
        let c = parse_vec3(&mut Reader::new("1 2 3.0"), true).unwrap();
        let s = SourcePose { pos: [0.0; 3], rot: [0.0; 2], anchor_pos: [0.0; 3] };
        assert_eq!(c.position(&s), [1.5, 2.0, 3.0]);
        let c = parse_block_pos(&mut Reader::new("~ ~1 -5")).unwrap();
        let s = SourcePose { pos: [10.7, 64.0, 0.0], rot: [0.0; 2], anchor_pos: [0.0; 3] };
        assert_eq!(c.block_pos(&s), [10, 65, -5]);
        assert!(parse_block_pos(&mut Reader::new("1 2")).is_err());
        assert!(parse_block_pos(&mut Reader::new("^ ~ ^")).is_err());
        let c = parse_vec3(&mut Reader::new("^ ^ ^1"), true).unwrap();
        let s = SourcePose { pos: [0.0; 3], rot: [0.0, 0.0], anchor_pos: [0.0; 3] };
        let p = c.position(&s);
        assert!((p[2] - 1.0).abs() < 1e-6, "{p:?}");
    }
}
