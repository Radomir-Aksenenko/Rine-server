//! Disk store of the decoration frontier (`rine_worldgen::pipeline::SlotStore`): one LZ4 file per partial
//! chunk under `<world>/worldgen`, grouped by 32x32 region like the chunk store. A handed-over chunk keeps a
//! tiny final marker so its decoration never runs again after a restart. Writes go to a temp file and are
//! renamed, so a crash never leaves a torn record.

use rine_worldgen::pipeline::{SavedSlot, SlotStore};
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

/// Only the palette entries the record uses are written, renumbered in first-use order.
fn encode(s: &SavedSlot) -> Vec<u8> {
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

fn decode(d: &[u8]) -> Option<SavedSlot> {
    let mut r = Reader { d, i: 0 };
    let final_chunk = r.u8()? != 0;
    let decorated = r.u8()? != 0;
    if final_chunk {
        return Some(SavedSlot {
            final_chunk,
            decorated,
            palette: Vec::new(),
            blocks: Vec::new(),
            biome_palette: Vec::new(),
            biomes: Vec::new(),
            wg: [Vec::new(), Vec::new()],
            heights: Vec::new(),
            ticks: Vec::new(),
        });
    }
    let palette = r.strings()?;
    let blocks = r.u16s()?;
    let biome_palette = r.strings()?;
    let biomes = r.u16s()?;
    let wg = [r.i32s()?, r.i32s()?];
    let heights = r.i32s()?;
    let n = r.u32()? as usize;
    let mut ticks = Vec::with_capacity(n.min(1 << 16));
    for _ in 0..n {
        let p = (r.i32()?, r.i32()?, r.i32()?);
        ticks.push((p, r.u16()?));
    }
    Some(SavedSlot { final_chunk, decorated, palette, blocks, biome_palette, biomes, wg, heights, ticks })
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

struct Reader<'a> {
    d: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.d.get(self.i..self.i.checked_add(n)?)?;
        self.i += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn strings(&mut self) -> Option<Vec<String>> {
        let n = self.u32()? as usize;
        let mut v = Vec::with_capacity(n.min(1 << 16));
        for _ in 0..n {
            let len = self.u16()? as usize;
            v.push(String::from_utf8(self.take(len)?.to_vec()).ok()?);
        }
        Some(v)
    }
    fn u16s(&mut self) -> Option<Vec<u16>> {
        let n = self.u32()? as usize;
        self.take(n.checked_mul(2)?).map(|b| b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect())
    }
    fn i32s(&mut self) -> Option<Vec<i32>> {
        let n = self.u32()? as usize;
        self.take(n.checked_mul(4)?).map(|b| b.chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        };
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
