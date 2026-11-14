//! Packet framing: `VarInt length | [VarInt dataLength] | payload`.
//! A `Frame` is a fully framed (and compressed if needed) packet, ready to be
//! written to any socket that uses the same compression threshold. Cloning a
//! Frame is a refcount bump, so broadcasts cost one encode for N viewers.

use crate::buf::{varint_len, ProtoError, Reader, Writer};
use bytes::{Bytes, BytesMut};
use flate2::{write::ZlibEncoder, Compression};
use std::io::Write;

pub const MAX_PACKET: usize = 2_097_151; // 3-byte varint, vanilla Varint21FrameDecoder limit
pub const MAX_UNCOMPRESSED: usize = 8_388_608; // vanilla CompressionDecoder.MAXIMUM_UNCOMPRESSED_LENGTH

#[derive(Clone, Debug)]
pub struct Frame(pub Bytes);

/// Build a packet body: call `f` to write id + fields into a scratch buffer.
#[inline]
pub fn packet(id: i32, f: impl FnOnce(&mut BytesMut)) -> BytesMut {
    let mut b = BytesMut::with_capacity(64);
    b.w_varint(id);
    f(&mut b);
    b
}

/// Frame a raw `id+body` payload. `threshold < 0` means compression disabled.
pub fn encode_frame(payload: &[u8], threshold: i32) -> Frame {
    let mut out = BytesMut::with_capacity(payload.len() + 8);
    if threshold < 0 {
        out.w_varint(payload.len() as i32);
        out.w_slice(payload);
    } else if (payload.len() as i32) < threshold {
        out.w_varint(payload.len() as i32 + 1);
        out.w_u8(0);
        out.w_slice(payload);
    } else {
        // Fast compression: level 1 gives ~same ratio on chunk data at a
        // fraction of the CPU of vanilla's default level.
        let mut enc = ZlibEncoder::new(Vec::with_capacity(payload.len() / 3 + 16), Compression::new(1));
        enc.write_all(payload).unwrap();
        let comp = enc.finish().unwrap();
        let dl = payload.len() as i32;
        out.w_varint((varint_len(dl) + comp.len()) as i32);
        out.w_varint(dl);
        out.w_slice(&comp);
    }
    Frame(out.freeze())
}

/// Incremental decoder for inbound bytes (already decrypted).
pub struct FrameDecoder {
    buf: BytesMut,
    pub threshold: i32,
}

impl Default for FrameDecoder {
    fn default() -> Self {
        Self { buf: BytesMut::with_capacity(4096), threshold: -1 }
    }
}

impl FrameDecoder {
    pub fn feed(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// Returns the next decoded `id+body` payload, if a full frame is buffered.
    pub fn next(&mut self) -> Result<Option<BytesMut>, ProtoError> {
        let mut r = Reader::new(&self.buf);
        let len = match r.varint() {
            Ok(l) => l,
            Err(ProtoError::Eof) => return Ok(None),
            Err(e) => return Err(e),
        };
        if len < 0 || len as usize > MAX_PACKET {
            return Err(ProtoError::Invalid("frame length"));
        }
        let hdr = r.pos;
        if self.buf.len() < hdr + len as usize {
            return Ok(None);
        }
        let _ = self.buf.split_to(hdr);
        let frame = self.buf.split_to(len as usize);
        if self.threshold < 0 {
            return Ok(Some(frame));
        }
        let mut r = Reader::new(&frame);
        let dl = r.varint()?;
        let off = r.pos;
        if dl == 0 {
            let mut f = frame;
            let _ = f.split_to(off);
            return Ok(Some(f));
        }
        if dl < 0 || dl as usize > MAX_UNCOMPRESSED || dl < self.threshold {
            return Err(ProtoError::Invalid("bad compressed length"));
        }
        let mut out = Vec::with_capacity(dl as usize);
        let mut dec = flate2::read::ZlibDecoder::new(&frame[off..]);
        std::io::Read::read_to_end(&mut dec, &mut out).map_err(|_| ProtoError::Invalid("zlib"))?;
        if out.len() != dl as usize {
            return Err(ProtoError::Invalid("decompressed length mismatch"));
        }
        Ok(Some(BytesMut::from(&out[..])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        for th in [-1, 0, 256] {
            let payload: Vec<u8> = (0..1000u32).map(|i| (i % 7) as u8).collect();
            let f = encode_frame(&payload, th);
            let mut d = FrameDecoder { threshold: th, ..Default::default() };
            d.feed(&f.0);
            assert_eq!(&d.next().unwrap().unwrap()[..], &payload[..]);
            assert!(d.next().unwrap().is_none());
        }
    }
}
