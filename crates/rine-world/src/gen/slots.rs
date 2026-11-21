//! Disk store of the decoration frontier (`rine_worldgen::pipeline::SlotStore`): one LZ4 file per partial
//! chunk under `<world>/worldgen`, grouped by 32x32 region like the chunk store. A handed-over chunk keeps a
//! tiny final marker so its decoration never runs again after a restart. Writes go to a temp file and are
//! renamed, so a crash never leaves a torn record.

use rine_worldgen::pipeline::{slot_codec, SavedSlot, SlotStore};
use std::path::PathBuf;

/// Magic, then one uncompressed byte (1 = final marker), then the LZ4 payload.
const MAGIC: &[u8; 4] = b"RWG1";

pub struct DiskSlots {
    root: PathBuf,
}

impl DiskSlots {
    pub fn open(world_dir: &str) -> Self {
        let root = PathBuf::from(world_dir).join("worldgen");
        let _ = std::fs::create_dir_all(&root);
        Self { root }
    }

    fn path(&self, cx: i32, cz: i32) -> PathBuf {
        self.root.join(format!("r.{}.{}", cx >> 5, cz >> 5)).join(format!("s.{cx}.{cz}.bin"))
    }
}

impl SlotStore for DiskSlots {
    fn load(&self, cx: i32, cz: i32) -> Option<SavedSlot> {
        let data = std::fs::read(self.path(cx, cz)).ok()?;
        if data.len() < 9 || &data[..4] != MAGIC {
            return None;
        }
        let raw = lz4_flex::decompress_size_prepended(&data[5..]).ok()?;
        decode(&raw)
    }

    fn is_final(&self, cx: i32, cz: i32) -> bool {
        use std::io::Read;
        let mut head = [0u8; 5];
        std::fs::File::open(self.path(cx, cz)).and_then(|mut f| f.read_exact(&mut head)).is_ok()
            && &head[..4] == MAGIC
            && head[4] == 1
    }

    fn save(&self, cx: i32, cz: i32, slot: &SavedSlot) {
        let raw = encode(slot);
        let mut out = Vec::with_capacity(raw.len() / 3 + 8);
        out.extend_from_slice(MAGIC);
        out.push(slot.final_chunk as u8);
        out.extend_from_slice(&lz4_flex::compress_prepend_size(&raw));
        let path = self.path(cx, cz);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, &out).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    fn delete(&self, cx: i32, cz: i32) {
        let _ = std::fs::remove_file(self.path(cx, cz));
    }
}

/// Payload format: `rine_worldgen::pipeline::slot_codec` (the compact palette record, block entities appended).
fn encode(s: &SavedSlot) -> Vec<u8> {
    slot_codec::encode(s).unwrap_or_else(|| vec![s.final_chunk as u8, s.decorated as u8])
}

fn decode(d: &[u8]) -> Option<SavedSlot> {
    slot_codec::decode(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The record format before block entities (`slot_codec` must keep reading it).
    fn legacy_encode(s: &SavedSlot) -> Vec<u8> {
        let mut o = Vec::with_capacity(if s.final_chunk { 2 } else { 200 * 1024 });
        o.push(s.final_chunk as u8);
        o.push(s.decorated as u8);
        if s.final_chunk {
            return o;
        }
        let mut local = vec![u16::MAX; s.palette.len()];
        let mut names: Vec<&str> = Vec::new();
        let mut id = |v: u16| -> u16 {
            let slot = &mut local[v as usize];
            if *slot == u16::MAX {
                *slot = names.len() as u16;
                names.push(&s.palette[v as usize]);
            }
            *slot
        };
        let blocks: Vec<u16> = s.blocks.iter().map(|&v| id(v)).collect();
        let ticks: Vec<((i32, i32, i32), u16)> = s.ticks.iter().map(|&(p, v)| (p, id(v))).collect();
        strings(&mut o, names.iter().copied());
        u16s(&mut o, &blocks);
        strings(&mut o, s.biome_palette.iter().map(String::as_str));
        u16s(&mut o, &s.biomes);
        i32s(&mut o, &s.wg[0]);
        i32s(&mut o, &s.wg[1]);
        i32s(&mut o, &s.heights);
        o.extend_from_slice(&(ticks.len() as u32).to_le_bytes());
        for ((x, y, z), v) in ticks {
            for c in [x, y, z] {
                o.extend_from_slice(&c.to_le_bytes());
            }
            o.extend_from_slice(&v.to_le_bytes());
        }
        o
    }

    fn strings<'a>(o: &mut Vec<u8>, it: impl ExactSizeIterator<Item = &'a str>) {
        o.extend_from_slice(&(it.len() as u32).to_le_bytes());
        for s in it {
            o.extend_from_slice(&(s.len() as u16).to_le_bytes());
            o.extend_from_slice(s.as_bytes());
        }
    }

    fn u16s(o: &mut Vec<u8>, v: &[u16]) {
        o.extend_from_slice(&(v.len() as u32).to_le_bytes());
        for x in v {
            o.extend_from_slice(&x.to_le_bytes());
        }
    }

    fn i32s(o: &mut Vec<u8>, v: &[i32]) {
        o.extend_from_slice(&(v.len() as u32).to_le_bytes());
        for x in v {
            o.extend_from_slice(&x.to_le_bytes());
        }
    }


    #[test]
    fn round_trip_with_compacted_palette() {
        let dir = std::env::temp_dir().join(format!("rine-slots-{}", std::process::id()));
        let store = DiskSlots::open(dir.to_str().unwrap());
        let palette: Vec<String> = (0..10).map(|i| format!("minecraft:b{i}")).collect();
        let slot = SavedSlot {
            final_chunk: false,
            decorated: true,
            palette: palette.clone(),
            blocks: vec![7, 7, 2, 9],
            biome_palette: vec!["minecraft:plains".into(), "minecraft:desert".into()],
            biomes: vec![1, 0],
            wg: [vec![60, 61], vec![50]],
            heights: vec![1, 2, 3],
            ticks: vec![((-17, 64, 5), 3)],
            block_entities: Vec::new(),
        };
        // A record written before block entities existed still reads back.
        let old = decode(&legacy_encode(&slot)).expect("legacy record");
        assert_eq!((old.blocks.len(), old.ticks.len(), old.heights.clone()), (4, 1, slot.heights.clone()));
        store.save(-1, 0, &slot);
        let back = store.load(-1, 0).expect("record");
        let names = |s: &SavedSlot| s.blocks.iter().map(|&v| s.palette[v as usize].clone()).collect::<Vec<_>>();
        assert_eq!(names(&back), names(&slot));
        assert_eq!(back.palette.len(), 4, "only used names are written");
        assert_eq!(back.palette[back.ticks[0].1 as usize], "minecraft:b3");
        assert_eq!((back.biomes.clone(), back.wg.clone(), back.heights.clone()), (slot.biomes.clone(), slot.wg.clone(), slot.heights.clone()));
        let marker = SavedSlot { final_chunk: true, blocks: Vec::new(), palette: Vec::new(), ..slot };
        assert!(!store.is_final(-1, 0));
        store.save(-1, 0, &marker);
        assert!(store.load(-1, 0).expect("marker").final_chunk);
        assert!(store.is_final(-1, 0) && !store.is_final(5, 5));
        let _ = std::fs::remove_dir_all(dir);
    }
}
