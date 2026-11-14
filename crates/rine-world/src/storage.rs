//! Chunk persistence. Each chunk is stored as an LZ4-compressed blob with a
//! palette per section, grouped in a folder per 32x32 region. Writes go to a
//! temp file and are renamed, so a crash never leaves a torn chunk.
//! (Anvil import/export is a planned separate tool.)

use crate::chunk::{Chunk, ChunkPos, Section, SECTIONS};
use std::path::PathBuf;

const MAGIC: &[u8; 4] = b"RIN1";

pub struct RegionStore {
    root: PathBuf,
}

impl RegionStore {
    pub fn open(dir: &str) -> Self {
        let root = PathBuf::from(dir).join("chunks");
        let _ = std::fs::create_dir_all(&root);
        Self { root }
    }

    fn path(&self, p: ChunkPos) -> PathBuf {
        self.root.join(format!("r.{}.{}", p.x >> 5, p.z >> 5)).join(format!("c.{}.{}.bin", p.x, p.z))
    }

    pub fn load(&self, p: ChunkPos) -> Option<Chunk> {
        let data = std::fs::read(self.path(p)).ok()?;
        if data.len() < 8 || &data[..4] != MAGIC {
            return None;
        }
        let raw = lz4_flex::decompress_size_prepended(&data[4..]).ok()?;
        decode(p, &raw)
    }

    pub fn save(&self, c: &Chunk) {
        let raw = encode(c);
        let mut out = Vec::with_capacity(raw.len() / 2 + 8);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&lz4_flex::compress_prepend_size(&raw));
        let path = self.path(c.pos);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, &out).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    pub fn flush(&self) {}
}

fn encode(c: &Chunk) -> Vec<u8> {
    let mut o = Vec::with_capacity(8192);
    o.push(SECTIONS as u8);
    for s in &c.sections {
        match &s.blocks {
            None => {
                o.push(0);
                o.extend_from_slice(&s.single.to_le_bytes());
            }
            Some(arr) => {
                // palette + u16 indices when the palette is small, else raw
                let mut pal: Vec<u16> = Vec::new();
                let mut idx = vec![0u16; 4096];
                let mut map = ahash::AHashMap::new();
                for (i, &st) in arr.iter().enumerate() {
                    let n = pal.len() as u16;
                    let e = *map.entry(st).or_insert_with(|| {
                        pal.push(st);
                        n
                    });
                    idx[i] = e;
                }
                if pal.len() <= 256 {
                    o.push(1);
                    o.extend_from_slice(&(pal.len() as u16).to_le_bytes());
                    for p in &pal {
                        o.extend_from_slice(&p.to_le_bytes());
                    }
                    o.extend(idx.iter().map(|&i| i as u8));
                } else {
                    o.push(2);
                    for st in arr.iter() {
                        o.extend_from_slice(&st.to_le_bytes());
                    }
                }
            }
        }
        o.extend_from_slice(&s.biomes);
    }
    o
}

fn decode(p: ChunkPos, d: &[u8]) -> Option<Chunk> {
    let mut c = Chunk::new(p);
    let mut i = 0usize;
    let rd16 = |d: &[u8], i: &mut usize| -> Option<u16> {
        let v = u16::from_le_bytes(d.get(*i..*i + 2)?.try_into().ok()?);
        *i += 2;
        Some(v)
    };
    let n = *d.get(i)? as usize;
    i += 1;
    for sy in 0..n.min(SECTIONS) {
        let kind = *d.get(i)?;
        i += 1;
        let mut s = Section::default();
        match kind {
            0 => {
                s.single = rd16(d, &mut i)?;
            }
            1 => {
                let pl = rd16(d, &mut i)? as usize;
                let mut pal = Vec::with_capacity(pl);
                for _ in 0..pl {
                    pal.push(rd16(d, &mut i)?);
                }
                let idx = d.get(i..i + 4096)?;
                i += 4096;
                let mut arr = Box::new([0u16; 4096]);
                for (k, &b) in idx.iter().enumerate() {
                    arr[k] = *pal.get(b as usize)?;
                }
                s.blocks = Some(arr);
            }
            _ => {
                let mut arr = Box::new([0u16; 4096]);
                for v in arr.iter_mut() {
                    *v = rd16(d, &mut i)?;
                }
                s.blocks = Some(arr);
            }
        }
        s.biomes.copy_from_slice(d.get(i..i + 64)?);
        i += 64;
        c.sections[sy] = s;
    }
    c.finish_bulk();
    c.dirty = false;
    crate::light::light_chunk(&mut c);
    Some(c)
}
