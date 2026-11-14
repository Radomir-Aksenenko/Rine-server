//! Minimal network-NBT writer (nameless root, used by protocol >= 764) and a
//! small owned tree for building compound values. Strings use Java modified
//! UTF-8; for BMP text without NUL this equals standard UTF-8, and we encode
//! supplementary characters as surrogate pairs to stay exact.

use crate::buf::Writer;

pub const END: u8 = 0;
pub const BYTE: u8 = 1;
pub const SHORT: u8 = 2;
pub const INT: u8 = 3;
pub const LONG: u8 = 4;
pub const FLOAT: u8 = 5;
pub const DOUBLE: u8 = 6;
pub const BYTE_ARRAY: u8 = 7;
pub const STRING: u8 = 8;
pub const LIST: u8 = 9;
pub const COMPOUND: u8 = 10;
pub const INT_ARRAY: u8 = 11;
pub const LONG_ARRAY: u8 = 12;

#[derive(Debug, Clone, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List(Vec<Tag>),
    Compound(Vec<(String, Tag)>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Tag {
    pub fn id(&self) -> u8 {
        match self {
            Tag::Byte(_) => BYTE,
            Tag::Short(_) => SHORT,
            Tag::Int(_) => INT,
            Tag::Long(_) => LONG,
            Tag::Float(_) => FLOAT,
            Tag::Double(_) => DOUBLE,
            Tag::ByteArray(_) => BYTE_ARRAY,
            Tag::String(_) => STRING,
            Tag::List(_) => LIST,
            Tag::Compound(_) => COMPOUND,
            Tag::IntArray(_) => INT_ARRAY,
            Tag::LongArray(_) => LONG_ARRAY,
        }
    }

    pub fn compound() -> Self {
        Tag::Compound(Vec::new())
    }

    pub fn with(mut self, k: &str, v: Tag) -> Self {
        if let Tag::Compound(ref mut c) = self {
            c.push((k.to_string(), v));
        }
        self
    }

    pub fn get(&self, k: &str) -> Option<&Tag> {
        match self {
            Tag::Compound(c) => c.iter().find(|(n, _)| n == k).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Network form: type id + payload, no root name.
    pub fn write_network<W: Writer + ?Sized>(&self, w: &mut W) {
        w.w_u8(self.id());
        self.write_payload(w);
    }

    /// Disk form: type id + empty-or-given name + payload.
    pub fn write_named<W: Writer + ?Sized>(&self, w: &mut W, name: &str) {
        w.w_u8(self.id());
        write_mutf8(w, name);
        self.write_payload(w);
    }

    pub fn write_payload<W: Writer + ?Sized>(&self, w: &mut W) {
        match self {
            Tag::Byte(v) => w.w_i8(*v),
            Tag::Short(v) => w.w_i16(*v),
            Tag::Int(v) => w.w_i32(*v),
            Tag::Long(v) => w.w_i64(*v),
            Tag::Float(v) => w.w_f32(*v),
            Tag::Double(v) => w.w_f64(*v),
            Tag::ByteArray(v) => {
                w.w_i32(v.len() as i32);
                for b in v {
                    w.w_i8(*b);
                }
            }
            Tag::String(s) => write_mutf8(w, s),
            Tag::List(l) => {
                let ty = l.first().map(|t| t.id()).unwrap_or(END);
                w.w_u8(ty);
                w.w_i32(l.len() as i32);
                for t in l {
                    t.write_payload(w);
                }
            }
            Tag::Compound(c) => {
                for (k, v) in c {
                    w.w_u8(v.id());
                    write_mutf8(w, k);
                    v.write_payload(w);
                }
                w.w_u8(END);
            }
            Tag::IntArray(v) => {
                w.w_i32(v.len() as i32);
                for x in v {
                    w.w_i32(*x);
                }
            }
            Tag::LongArray(v) => {
                w.w_i32(v.len() as i32);
                for x in v {
                    w.w_i64(*x);
                }
            }
        }
    }
}

pub fn write_mutf8<W: Writer + ?Sized>(w: &mut W, s: &str) {
    let fast = s.bytes().all(|b| b != 0) && !s.chars().any(|c| (c as u32) > 0xFFFF);
    if fast {
        w.w_u16(s.len() as u16);
        w.w_slice(s.as_bytes());
        return;
    }
    let mut out = Vec::with_capacity(s.len() + 8);
    for u in s.encode_utf16() {
        match u {
            0x0001..=0x007F => out.push(u as u8),
            0x0000 | 0x0080..=0x07FF => {
                out.push(0xC0 | ((u >> 6) & 0x1F) as u8);
                out.push(0x80 | (u & 0x3F) as u8);
            }
            _ => {
                out.push(0xE0 | ((u >> 12) & 0x0F) as u8);
                out.push(0x80 | ((u >> 6) & 0x3F) as u8);
                out.push(0x80 | (u & 0x3F) as u8);
            }
        }
    }
    w.w_u16(out.len() as u16);
    w.w_slice(&out);
}

/// Reader for NBT (network or disk). Used for loading saved data & plugin input.
pub mod read {
    use super::*;
    use crate::buf::{ProtoError, Reader};

    pub fn read_network(r: &mut Reader) -> Result<Option<Tag>, ProtoError> {
        let ty = r.u8()?;
        if ty == END {
            return Ok(None);
        }
        Ok(Some(payload(r, ty, 0)?))
    }

    pub fn read_named(r: &mut Reader) -> Result<Option<(String, Tag)>, ProtoError> {
        let ty = r.u8()?;
        if ty == END {
            return Ok(None);
        }
        let name = mutf8(r)?;
        Ok(Some((name, payload(r, ty, 0)?)))
    }

    fn mutf8(r: &mut Reader) -> Result<String, ProtoError> {
        let len = r.u16()? as usize;
        let b = r.bytes(len)?;
        if let Ok(s) = std::str::from_utf8(b) {
            if !b.windows(2).any(|w| w == [0xC0, 0x80]) && !b.contains(&0xED) {
                return Ok(s.to_string());
            }
        }
        // decode modified utf-8 via utf16
        let mut units = Vec::with_capacity(len);
        let mut i = 0;
        while i < b.len() {
            let c = b[i] as u16;
            if c < 0x80 {
                units.push(c);
                i += 1;
            } else if c & 0xE0 == 0xC0 && i + 1 < b.len() {
                units.push(((c & 0x1F) << 6) | (b[i + 1] as u16 & 0x3F));
                i += 2;
            } else if i + 2 < b.len() {
                units.push(((c & 0x0F) << 12) | ((b[i + 1] as u16 & 0x3F) << 6) | (b[i + 2] as u16 & 0x3F));
                i += 3;
            } else {
                return Err(ProtoError::Utf8);
            }
        }
        Ok(String::from_utf16_lossy(&units))
    }

    fn payload(r: &mut Reader, ty: u8, depth: u32) -> Result<Tag, ProtoError> {
        if depth > 512 {
            return Err(ProtoError::Invalid("nbt depth"));
        }
        Ok(match ty {
            BYTE => Tag::Byte(r.i8()?),
            SHORT => Tag::Short(r.i16()?),
            INT => Tag::Int(r.i32()?),
            LONG => Tag::Long(r.i64()?),
            FLOAT => Tag::Float(r.f32()?),
            DOUBLE => Tag::Double(r.f64()?),
            BYTE_ARRAY => {
                let n = r.i32()?.max(0) as usize;
                Tag::ByteArray(r.bytes(n)?.iter().map(|b| *b as i8).collect())
            }
            STRING => Tag::String(mutf8(r)?),
            LIST => {
                let et = r.u8()?;
                let n = r.i32()?.max(0) as usize;
                let mut v = Vec::with_capacity(n.min(65536));
                for _ in 0..n {
                    v.push(payload(r, et, depth + 1)?);
                }
                Tag::List(v)
            }
            COMPOUND => {
                let mut c = Vec::new();
                loop {
                    let t = r.u8()?;
                    if t == END {
                        break;
                    }
                    let k = mutf8(r)?;
                    c.push((k, payload(r, t, depth + 1)?));
                }
                Tag::Compound(c)
            }
            INT_ARRAY => {
                let n = r.i32()?.max(0) as usize;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(r.i32()?);
                }
                Tag::IntArray(v)
            }
            LONG_ARRAY => {
                let n = r.i32()?.max(0) as usize;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(r.i64()?);
                }
                Tag::LongArray(v)
            }
            _ => return Err(ProtoError::Invalid("nbt tag type")),
        })
    }
}
