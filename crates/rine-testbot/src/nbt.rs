//! Network NBT helpers: skip arbitrary payloads and pull text out of components.
//! Network NBT is a nameless root (type byte + payload). Names and TAG_String
//! values are u16 big-endian length + modified UTF-8 (decoded lossily here).

use anyhow::{bail, Result};
use rine_protocol::Reader;

/// Reads a u16-length-prefixed NBT string.
pub fn read_nbt_string(r: &mut Reader) -> Result<String> {
    let len = r.u16()? as usize;
    let b = r.bytes(len)?;
    Ok(String::from_utf8_lossy(b).into_owned())
}

fn nbt_len(r: &mut Reader) -> Result<usize> {
    let n = r.i32()?;
    if n < 0 {
        bail!("negative NBT length {n}");
    }
    Ok(n as usize)
}

/// Skips the payload of a tag of type `tag` (the type byte is NOT read here).
pub fn skip_nbt(r: &mut Reader, tag: u8) -> Result<()> {
    match tag {
        0 => {}
        1 => {
            r.bytes(1)?;
        }
        2 => {
            r.bytes(2)?;
        }
        3 | 5 => {
            r.bytes(4)?;
        }
        4 | 6 => {
            r.bytes(8)?;
        }
        7 => {
            let n = nbt_len(r)?;
            r.bytes(n)?;
        }
        8 => {
            read_nbt_string(r)?;
        }
        9 => {
            let et = r.u8()?;
            let n = nbt_len(r)?;
            if et == 0 && n > 0 {
                bail!("list of TAG_End with {n} elements");
            }
            for _ in 0..n {
                skip_nbt(r, et)?;
            }
        }
        10 => loop {
            let t = r.u8()?;
            if t == 0 {
                break;
            }
            read_nbt_string(r)?;
            skip_nbt(r, t)?;
        },
        11 => {
            let n = nbt_len(r)?;
            r.bytes(n.saturating_mul(4))?;
        }
        12 => {
            let n = nbt_len(r)?;
            r.bytes(n.saturating_mul(8))?;
        }
        other => bail!("unknown NBT tag type {other}"),
    }
    Ok(())
}

/// Reads a nameless network NBT text component (configuration/play disconnect,
/// system chat). Returns the text for a TAG_String, or for a compound whose
/// top-level `text` entry is a TAG_String. Anything else is skipped and yields None.
pub fn read_text_component(r: &mut Reader) -> Result<Option<String>> {
    let t = r.u8()?;
    match t {
        0 => Ok(None),
        8 => Ok(Some(read_nbt_string(r)?)),
        10 => {
            let mut text = None;
            loop {
                let ct = r.u8()?;
                if ct == 0 {
                    break;
                }
                let name = read_nbt_string(r)?;
                if name == "text" && ct == 8 {
                    text = Some(read_nbt_string(r)?);
                } else {
                    skip_nbt(r, ct)?;
                }
            }
            Ok(text)
        }
        other => {
            skip_nbt(r, other)?;
            Ok(None)
        }
    }
}
