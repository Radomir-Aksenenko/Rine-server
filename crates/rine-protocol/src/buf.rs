use bytes::{BufMut, BytesMut};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    #[error("unexpected end of packet")]
    Eof,
    #[error("varint too long")]
    VarIntTooLong,
    #[error("string too long: {0}")]
    StringTooLong(usize),
    #[error("invalid utf-8")]
    Utf8,
    #[error("invalid value: {0}")]
    Invalid(&'static str),
}

pub type Result<T> = std::result::Result<T, ProtoError>;

/// Block position packed like vanilla BlockPos.asLong: x:26 | z:26 | y:12.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BlockPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl BlockPos {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }
    pub fn pack(self) -> i64 {
        ((self.x as i64 & 0x3FF_FFFF) << 38) | ((self.z as i64 & 0x3FF_FFFF) << 12) | (self.y as i64 & 0xFFF)
    }
    pub fn unpack(v: i64) -> Self {
        let x = (v >> 38) as i32;
        let y = ((v << 52) >> 52) as i32;
        let z = ((v << 26) >> 38) as i32;
        Self { x, y, z }
    }
    pub fn offset(self, dx: i32, dy: i32, dz: i32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.z + dz)
    }
    /// Direction ids: 0 down, 1 up, 2 north(-z), 3 south(+z), 4 west(-x), 5 east(+x)
    pub fn relative(self, dir: u8) -> Self {
        match dir {
            0 => self.offset(0, -1, 0),
            1 => self.offset(0, 1, 0),
            2 => self.offset(0, 0, -1),
            3 => self.offset(0, 0, 1),
            4 => self.offset(-1, 0, 0),
            _ => self.offset(1, 0, 0),
        }
    }
}

/// Zero-copy reader over a packet body.
pub struct Reader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    #[inline]
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
    #[inline]
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(ProtoError::Eof);
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    pub fn rest(&mut self) -> &'a [u8] {
        let s = &self.data[self.pos..];
        self.pos = self.data.len();
        s
    }
    #[inline]
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }
    pub fn i8(&mut self) -> Result<i8> {
        Ok(self.u8()? as i8)
    }
    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? != 0)
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.bytes(2)?.try_into().unwrap()))
    }
    pub fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.i32()? as u32))
    }
    pub fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.i64()? as u64))
    }
    pub fn varint(&mut self) -> Result<i32> {
        let mut result: u32 = 0;
        for i in 0..5 {
            let b = self.u8()?;
            result |= ((b & 0x7F) as u32) << (7 * i);
            if b & 0x80 == 0 {
                return Ok(result as i32);
            }
        }
        Err(ProtoError::VarIntTooLong)
    }
    pub fn varlong(&mut self) -> Result<i64> {
        let mut result: u64 = 0;
        for i in 0..10 {
            let b = self.u8()?;
            result |= ((b & 0x7F) as u64) << (7 * i);
            if b & 0x80 == 0 {
                return Ok(result as i64);
            }
        }
        Err(ProtoError::VarIntTooLong)
    }
    pub fn string(&mut self, max_chars: usize) -> Result<&'a str> {
        let len = self.varint()?;
        if len < 0 || len as usize > max_chars * 3 {
            return Err(ProtoError::StringTooLong(len.max(0) as usize));
        }
        let b = self.bytes(len as usize)?;
        let s = std::str::from_utf8(b).map_err(|_| ProtoError::Utf8)?;
        if s.chars().count() > max_chars {
            return Err(ProtoError::StringTooLong(s.len()));
        }
        Ok(s)
    }
    pub fn uuid(&mut self) -> Result<Uuid> {
        let hi = self.i64()? as u64;
        let lo = self.i64()? as u64;
        Ok(Uuid::from_u64_pair(hi, lo))
    }
    pub fn block_pos(&mut self) -> Result<BlockPos> {
        Ok(BlockPos::unpack(self.i64()?))
    }
    pub fn byte_array(&mut self, max: usize) -> Result<&'a [u8]> {
        let len = self.varint()?;
        if len < 0 || len as usize > max {
            return Err(ProtoError::Invalid("byte array length"));
        }
        self.bytes(len as usize)
    }
}

/// Writer helpers on top of BytesMut.
pub trait Writer {
    fn w_u8(&mut self, v: u8);
    fn w_bool(&mut self, v: bool) {
        self.w_u8(v as u8)
    }
    fn w_i8(&mut self, v: i8) {
        self.w_u8(v as u8)
    }
    fn w_i16(&mut self, v: i16);
    fn w_u16(&mut self, v: u16) {
        self.w_i16(v as i16)
    }
    fn w_i32(&mut self, v: i32);
    fn w_i64(&mut self, v: i64);
    fn w_f32(&mut self, v: f32) {
        self.w_i32(v.to_bits() as i32)
    }
    fn w_f64(&mut self, v: f64) {
        self.w_i64(v.to_bits() as i64)
    }
    fn w_slice(&mut self, v: &[u8]);
    fn w_varint(&mut self, v: i32) {
        let mut v = v as u32;
        loop {
            if v & !0x7F == 0 {
                self.w_u8(v as u8);
                return;
            }
            self.w_u8((v as u8 & 0x7F) | 0x80);
            v >>= 7;
        }
    }
    fn w_varlong(&mut self, v: i64) {
        let mut v = v as u64;
        loop {
            if v & !0x7F == 0 {
                self.w_u8(v as u8);
                return;
            }
            self.w_u8((v as u8 & 0x7F) | 0x80);
            v >>= 7;
        }
    }
    fn w_str(&mut self, s: &str) {
        self.w_varint(s.len() as i32);
        self.w_slice(s.as_bytes());
    }
    fn w_uuid(&mut self, u: Uuid) {
        let (hi, lo) = u.as_u64_pair();
        self.w_i64(hi as i64);
        self.w_i64(lo as i64);
    }
    fn w_pos(&mut self, p: BlockPos) {
        self.w_i64(p.pack())
    }
    fn w_byte_array(&mut self, b: &[u8]) {
        self.w_varint(b.len() as i32);
        self.w_slice(b);
    }
    /// Rotation as "angle byte" (degrees * 256 / 360).
    fn w_angle(&mut self, deg: f32) {
        self.w_u8(((deg * 256.0 / 360.0).floor() as i32) as u8)
    }
}

impl Writer for BytesMut {
    #[inline]
    fn w_u8(&mut self, v: u8) {
        self.put_u8(v)
    }
    #[inline]
    fn w_i16(&mut self, v: i16) {
        self.put_i16(v)
    }
    #[inline]
    fn w_i32(&mut self, v: i32) {
        self.put_i32(v)
    }
    #[inline]
    fn w_i64(&mut self, v: i64) {
        self.put_i64(v)
    }
    #[inline]
    fn w_slice(&mut self, v: &[u8]) {
        self.put_slice(v)
    }
}

impl Writer for Vec<u8> {
    #[inline]
    fn w_u8(&mut self, v: u8) {
        self.push(v)
    }
    #[inline]
    fn w_i16(&mut self, v: i16) {
        self.extend_from_slice(&v.to_be_bytes())
    }
    #[inline]
    fn w_i32(&mut self, v: i32) {
        self.extend_from_slice(&v.to_be_bytes())
    }
    #[inline]
    fn w_i64(&mut self, v: i64) {
        self.extend_from_slice(&v.to_be_bytes())
    }
    #[inline]
    fn w_slice(&mut self, v: &[u8]) {
        self.extend_from_slice(v)
    }
}

pub fn varint_len(v: i32) -> usize {
    let v = v as u32;
    match v {
        0..=0x7F => 1,
        0x80..=0x3FFF => 2,
        0x4000..=0x1F_FFFF => 3,
        0x20_0000..=0xFFF_FFFF => 4,
        _ => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn varint_roundtrip() {
        for v in [0, 1, 127, 128, 255, 25565, 2097151, i32::MAX, -1, i32::MIN] {
            let mut b = Vec::new();
            b.w_varint(v);
            assert_eq!(b.len(), varint_len(v));
            assert_eq!(Reader::new(&b).varint().unwrap(), v);
        }
    }
    #[test]
    fn pos_roundtrip() {
        for p in [BlockPos::new(0, 0, 0), BlockPos::new(-1, -64, -1), BlockPos::new(30000000, 319, -30000000), BlockPos::new(18357644, 831, -20882616)] {
            assert_eq!(BlockPos::unpack(p.pack()), p);
        }
    }
}
