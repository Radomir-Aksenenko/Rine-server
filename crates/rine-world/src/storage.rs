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
    // block entities (optional trailer; chunks written before it end here)
    o.extend_from_slice(BE_MARK);
    o.extend_from_slice(&(c.block_entities.len() as u32).to_le_bytes());
    for be in &c.block_entities {
        o.push(be.x);
        o.extend_from_slice(&be.y.to_le_bytes());
        o.push(be.z);
        o.extend_from_slice(&be.type_id.to_le_bytes());
        match &be.nbt {
            Some(t) => {
                o.push(1);
                t.write_network(&mut o);
            }
            None => o.push(0),
        }
    }
    o
}

const BE_MARK: &[u8; 4] = b"BENT";

fn decode_block_entities(c: &mut Chunk, d: &[u8]) -> Option<()> {
    if d.len() < 8 || &d[..4] != BE_MARK {
        return None;
    }
    let n = u32::from_le_bytes(d[4..8].try_into().ok()?) as usize;
    let mut r = rine_protocol::Reader::new(&d[8..]);
    for _ in 0..n {
        let x = r.u8().ok()?;
        let y = i16::from_le_bytes(r.bytes(2).ok()?.try_into().ok()?);
        let z = r.u8().ok()?;
        let t = r.bytes(4).ok()?;
        let type_id = u32::from_le_bytes(t.try_into().ok()?);
        let nbt = if r.u8().ok()? != 0 { rine_protocol::nbt::read::read_network(&mut r).ok()? } else { None };
        c.block_entities.push(crate::chunk::BlockEntity { x, y, z, type_id, nbt });
    }
    Some(())
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
    if let Some(rest) = d.get(i..) {
        if decode_block_entities(&mut c, rest).is_none() {
            c.block_entities.clear();
        }
    }
    c.finish_bulk();
    c.dirty = false;
    crate::light::light_chunk(&mut c);
    Some(c)
}

#[cfg(test)]
mod be_tests {
    use super::*;
    use rine_protocol::nbt::Tag;

    /// With vanilla decoration every chunk comes back from disk after a restart (it is never regenerated),
    /// so the stored chunk must equal the generated and lit one: blocks, biomes, light, heightmaps.
    #[test]
    fn generated_chunk_round_trips() {
        rine_data::init();
        let g = crate::gen::Generator::with_options(31337, true);
        for pos in [ChunkPos::new(0, 0), ChunkPos::new(-9, 14)] {
            let mut c = g.generate(pos);
            crate::light::light_chunk(&mut c);
            let back = decode(pos, &encode(&c)).expect("decodes");
            for (sy, (a, b)) in c.sections.iter().zip(&back.sections).enumerate() {
                assert!((0..4096).all(|i| a.get(i) == b.get(i)), "blocks of section {sy}");
                assert_eq!(a.biomes, b.biomes, "biomes of section {sy}");
                assert_eq!(a.sky, b.sky, "sky light of section {sy}");
                assert_eq!(a.block_light, b.block_light, "block light of section {sy}");
                assert_eq!((a.non_air, a.fluids, a.ticking), (b.non_air, b.fluids, b.ticking), "counts of section {sy}");
            }
            assert_eq!(c.motion_blocking, back.motion_blocking);
            assert_eq!(c.world_surface, back.world_surface);
            assert_eq!(c.motion_blocking_no_leaves, back.motion_blocking_no_leaves);
            assert_eq!(c.block_entities.len(), back.block_entities.len());
        }
    }

    #[test]
    fn block_entities_round_trip() {
        rine_data::init();
        let mut c = Chunk::new(ChunkPos::new(3, -2));
        let nbt = Tag::compound().with("Command", Tag::String("say hi".into())).with("auto", Tag::Byte(1));
        c.block_entities.push(crate::chunk::BlockEntity { x: 5, y: -40, z: 15, type_id: 7, nbt: Some(nbt.clone()) });
        c.block_entities.push(crate::chunk::BlockEntity { x: 0, y: 300, z: 0, type_id: 2, nbt: None });
        let back = decode(c.pos, &encode(&c)).unwrap();
        assert_eq!(back.block_entities.len(), 2);
        let a = &back.block_entities[0];
        assert_eq!((a.x, a.y, a.z, a.type_id), (5, -40, 15, 7));
        assert_eq!(a.nbt.as_ref(), Some(&nbt));
        assert_eq!(back.block_entities[1].y, 300);
        assert!(back.block_entities[1].nbt.is_none());
    }

    #[test]
    fn old_chunks_without_trailer_load() {
        rine_data::init();
        let c = Chunk::new(ChunkPos::new(0, 0));
        let mut raw = encode(&c);
        raw.truncate(raw.len() - 8); // drop "BENT" + count
        assert!(decode(c.pos, &raw).unwrap().block_entities.is_empty());
    }
}
