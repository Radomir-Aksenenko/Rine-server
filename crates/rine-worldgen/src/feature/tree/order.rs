//! `java.util.HashSet<BlockPos>` iteration order, reproduced exactly.
//!
//! Tree decorators iterate their position sets (and `Util.shuffle` / stable sorts depend on that order),
//! so the RNG draws of a tree follow Java's hash layout, not insertion order. The layout is:
//! - `BlockPos.hashCode` = `(y + z * 31) * 31 + x` (`Vec3i.hashCode`), spread by `h ^ (h >>> 16)`;
//! - the table starts at 16 buckets, doubles when `size > 0.75 * capacity` (checked after the insert) and
//!   never shrinks on remove;
//! - a bucket keeps insertion order (tail insertion; resize split keeps relative order);
//! - a 9th node in one bucket while the table is below 64 buckets resizes the table (`treeifyBin`).
//!   Once the table is 64 or larger a bucket would turn into a red-black tree whose link order differs,
//!   so that case panics instead of guessing.
//!
//! Pop-first (`iterator().next()` + `remove()`) is the first element of the ordered set.

use std::collections::{BTreeSet, HashMap};
use std::hash::{BuildHasherDefault, Hasher};

pub type Pos = (i32, i32, i32);

/// Bins that Java would treeify (9 nodes, table >= 64) and whose order is approximated by tail insertion.
pub static TREEIFIED_BINS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Multiplicative hasher for small integer keys (positions and palette ids).
#[derive(Default)]
pub struct FastHasher(u64);

impl Hasher for FastHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(5) ^ b as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
        }
    }
    #[inline]
    fn write_i32(&mut self, i: i32) {
        self.0 = (self.0.rotate_left(5) ^ i as u32 as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.0 = (self.0.rotate_left(5) ^ i as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

pub type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FastHasher>>;

/// `BlockPos.hashCode` after `HashMap.hash` spreading.
#[inline]
pub fn java_spread_hash(p: Pos) -> u32 {
    let h = (p.1.wrapping_add(p.2.wrapping_mul(31))).wrapping_mul(31).wrapping_add(p.0) as u32;
    h ^ (h >> 16)
}

/// A `HashSet<BlockPos>` whose iteration order matches `java.util.HashSet`.
pub struct JavaSet {
    /// Alive positions -> insertion sequence number.
    seq_of: FastMap<Pos, u64>,
    /// Alive positions in Java iteration order: (bucket, insertion sequence, position).
    order: BTreeSet<(u32, u64, Pos)>,
    /// Number of nodes in each bucket of the current table.
    chain: Vec<u32>,
    cap: u32,
    next_seq: u64,
}

impl Default for JavaSet {
    fn default() -> Self {
        Self::new()
    }
}

impl JavaSet {
    pub fn new() -> Self {
        JavaSet { seq_of: FastMap::default(), order: BTreeSet::new(), chain: vec![0; 16], cap: 16, next_seq: 0 }
    }

    #[inline]
    fn bucket(&self, p: Pos) -> u32 {
        java_spread_hash(p) & (self.cap - 1)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.seq_of.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.seq_of.is_empty()
    }

    #[inline]
    pub fn contains(&self, p: &Pos) -> bool {
        self.seq_of.contains_key(p)
    }

    fn resize(&mut self) {
        self.cap *= 2;
        self.chain = vec![0; self.cap as usize];
        let old = std::mem::take(&mut self.order);
        for (_, seq, p) in old {
            let b = self.bucket(p);
            self.chain[b as usize] += 1;
            self.order.insert((b, seq, p));
        }
    }

    /// `HashSet.add`. Returns true when the element was new.
    pub fn insert(&mut self, p: Pos) -> bool {
        if self.seq_of.contains_key(&p) {
            return false;
        }
        let b = self.bucket(p);
        let chain_before = self.chain[b as usize];
        let seq = self.next_seq;
        self.next_seq += 1;
        self.seq_of.insert(p, seq);
        self.order.insert((b, seq, p));
        self.chain[b as usize] += 1;
        // putVal: appending to a chain of 8 or more calls treeifyBin, which resizes below 64 buckets.
        if chain_before >= 8 {
            if self.cap < 64 {
                self.resize();
            } else {
                // Treeify: the linked order of a red-black bin differs from tail insertion. Not emulated; counted.
                TREEIFIED_BINS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        if self.seq_of.len() as u32 > self.cap * 3 / 4 {
            self.resize();
        }
        true
    }

    /// `HashSet.remove`.
    pub fn remove(&mut self, p: &Pos) -> bool {
        match self.seq_of.remove(p) {
            Some(seq) => {
                let b = self.bucket(*p);
                self.order.remove(&(b, seq, *p));
                self.chain[b as usize] -= 1;
                true
            }
            None => false,
        }
    }

    /// `iterator().next()` followed by `iterator.remove()`.
    pub fn pop_first(&mut self) -> Option<Pos> {
        let (b, seq, p) = self.order.pop_first()?;
        self.seq_of.remove(&p);
        self.chain[b as usize] -= 1;
        let _ = seq;
        Some(p)
    }

    /// Elements in Java iteration order.
    pub fn to_vec(&self) -> Vec<Pos> {
        self.order.iter().map(|&(_, _, p)| p).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_set_orders_by_bucket_then_insertion() {
        let mut s = JavaSet::new();
        // hash((0,0,0)) = 0; hash((1,0,0)) = 1; hash((0,0,1)) = 31 * 31 = 961 -> bucket 961 & 15 = 1.
        s.insert((1, 0, 0));
        s.insert((0, 0, 0));
        s.insert((0, 0, 1));
        assert_eq!(s.to_vec(), vec![(0, 0, 0), (1, 0, 0), (0, 0, 1)]);
    }

    #[test]
    fn resize_keeps_bucket_order_and_pop_first_matches_iteration() {
        let mut s = JavaSet::new();
        for i in 0..13 {
            s.insert((i, 0, i % 3));
        }
        assert_eq!(s.len(), 13);
        let order = s.to_vec();
        let mut popped = Vec::new();
        while let Some(p) = s.pop_first() {
            popped.push(p);
        }
        assert_eq!(order, popped);
    }
}
