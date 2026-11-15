//! /random (`RandomCommand`, `RandomSequences`) and the server-side RNG for
//! selectors.
//!
//! Named sequences use vanilla's algorithm: Xoroshiro128++ seeded from the
//! world seed, a salt, and the MD5 of the sequence id (`RandomSupport`).
//! The sequence table is kept in a thread-local, because `commands::States`
//! has no random field yet; it is not saved to `data/random_sequences.dat`.

use crate::game::cmd::{lit, Commands, Ctx};
use md5::{Digest, Md5};
use rine_command::args::misc::IntRange;
use rine_command::args::Arg;
use rine_command::{CmdError, CmdResult};
use rine_protocol::text::Text;
use std::cell::RefCell;
use std::collections::BTreeMap;

pub fn register(c: &mut Commands) {
    c.exec("random value range", |x| sample(x, false));
    c.exec("random roll range", |x| sample(x, true));
    c.exec("random value range sequence", |x| sample_seq(x, false));
    c.exec("random roll range sequence", |x| sample_seq(x, true));
    c.exec("random reset *", reset_all);
    c.exec("random reset * seed", |x| reset_all_with(x, None));
    c.exec("random reset * seed includeWorldSeed", |x| reset_all_with(x, Some(false)));
    c.exec("random reset * seed includeWorldSeed includeSequenceId", reset_all_full);
    c.exec("random reset sequence", reset_seq);
    c.exec("random reset sequence seed", reset_seq_with);
    c.exec("random reset sequence seed includeWorldSeed", reset_seq_world);
    c.exec("random reset sequence seed includeWorldSeed includeSequenceId", reset_seq_full);
}

thread_local! {
    static STATE: RefCell<u64> = RefCell::new(0x9E37_79B9_7F4A_7C15 ^ std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1));
    static SEQS: RefCell<Seqs> = RefCell::new(Seqs::default());
}


/// `RandomSequences` state: sequences plus the defaults used for new ones.
struct Seqs {
    /// Sequence id to Xoroshiro state (lo, hi).
    map: BTreeMap<String, [u64; 2]>,
    salt: i32,
    include_world_seed: bool,
    include_sequence_id: bool,
}

impl Default for Seqs {
    fn default() -> Self {
        Self { map: BTreeMap::new(), salt: 0, include_world_seed: true, include_sequence_id: true }
    }
}

/// Fast non-cryptographic RNG (splitmix64) for shuffles.
pub fn next_u32() -> u32 {
    STATE.with(|s| {
        let mut z = s.borrow().wrapping_add(0x9E37_79B9_7F4A_7C15);
        *s.borrow_mut() = z;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 32) as u32
    })
}

/// 128 random bits (UUIDs of new boss bars).
pub fn random_u128() -> u128 {
    ((next_u32() as u128) << 96) | ((next_u32() as u128) << 64) | ((next_u32() as u128) << 32) | next_u32() as u128
}

const GOLDEN: u64 = (-7046029254386353131i64) as u64;
const SILVER: u64 = 7640891576956012809;

/// `RandomSupport.mixStafford13`.
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul((-4658895280553007687i64) as u64);
    z = (z ^ (z >> 27)).wrapping_mul((-7723592293110705685i64) as u64);
    z ^ (z >> 31)
}

/// `new XoroshiroRandomSource(upgradeSeedTo128bitUnmixed(seed).xor(key).mixed())`.
fn make_state(seed: i64, key: Option<&str>) -> [u64; 2] {
    let mut lo = (seed as u64) ^ SILVER;
    let mut hi = lo.wrapping_add(GOLDEN);
    if let Some(k) = key {
        let h: [u8; 16] = Md5::digest(k.as_bytes()).into();
        let hash_lo = u64::from_be_bytes(h[0..8].try_into().unwrap_or([0; 8]));
        let hash_hi = u64::from_be_bytes(h[8..16].try_into().unwrap_or([0; 8]));
        lo ^= hash_lo;
        hi ^= hash_hi;
    }
    lo = mix(lo);
    hi = mix(hi);
    if lo | hi == 0 {
        return [GOLDEN, SILVER];
    }
    [lo, hi]
}

/// `Xoroshiro128PlusPlus.nextLong`.
fn next_long(s: &mut [u64; 2]) -> u64 {
    let s0 = s[0];
    let mut s1 = s[1];
    let result = s0.wrapping_add(s1).rotate_left(17).wrapping_add(s0);
    s1 ^= s0;
    s[0] = s0.rotate_left(49) ^ s1 ^ (s1 << 21);
    s[1] = s1.rotate_left(28);
    result
}

/// `RandomSource.nextInt(bound)` (Lemire-style with rejection), as in
/// `XoroshiroRandomSource`.
fn bounded(mut next_int: impl FnMut() -> i32, bound: i32) -> i32 {
    let mut bits = next_int() as u32 as u64;
    let mut mult = bits * bound as u64;
    let mut frac = mult & 0xFFFF_FFFF;
    if frac < bound as u64 {
        let start = ((!(bound as u32)).wrapping_add(1)) % bound as u32;
        while frac < start as u64 {
            bits = next_int() as u32 as u64;
            mult = bits * bound as u64;
            frac = mult & 0xFFFF_FFFF;
        }
    }
    (mult >> 32) as i32
}

impl Seqs {
    fn create(&self, id: &str, world_seed: i64, salt: i32, inc_world: bool, inc_seq: bool) -> [u64; 2] {
        let seed = (if inc_world { world_seed } else { 0 }) ^ salt as i64;
        make_state(seed, if inc_seq { Some(id) } else { None })
    }
}

/// Sequence state, created with the current defaults when missing.
fn with_seq<R>(id: &str, world_seed: i64, f: impl FnOnce(&mut [u64; 2]) -> R) -> R {
    SEQS.with(|s| {
        let mut s = s.borrow_mut();
        if !s.map.contains_key(id) {
            let st = s.create(id, world_seed, s.salt, s.include_world_seed, s.include_sequence_id);
            s.map.insert(id.to_string(), st);
        }
        let st = s.map.get_mut(id).expect("sequence inserted above");
        f(st)
    })
}

fn world_seed(x: &Ctx) -> i64 {
    x.g.shared.config.seed
}

fn sample(x: &mut Ctx, announce: bool) -> CmdResult<i32> {
    let range = match x.arg("range") {
        Arg::IntRange(r) => *r,
        _ => IntRange::default(),
    };
    let value = roll(range, None, announce, x)?;
    Ok(value)
}

fn sample_seq(x: &mut Ctx, announce: bool) -> CmdResult<i32> {
    let range = match x.arg("range") {
        Arg::IntRange(r) => *r,
        _ => IntRange::default(),
    };
    let id = x.id("sequence").to_string();
    let seed = world_seed(x);
    roll(range, Some((id, seed)), announce, x)
}

/// `RandomCommand.randomSample`.
fn roll(range: IntRange, seq: Option<(String, i64)>, announce: bool, x: &mut Ctx) -> CmdResult<i32> {
    let min = range.min.unwrap_or(i32::MIN);
    let max = range.max.unwrap_or(i32::MAX);
    let span = max as i64 - min as i64;
    if span == 0 {
        return Err(CmdError::key("commands.random.error.range_too_small"));
    }
    if span >= 2147483647 {
        return Err(CmdError::key("commands.random.error.range_too_large"));
    }
    let bound = (max - min).wrapping_add(1);
    let r = match &seq {
        Some((id, ws)) => with_seq(id, *ws, |st| bounded(|| next_long(st) as i32, bound)),
        None => bounded(|| next_u32() as i32, bound),
    };
    let value = r.wrapping_add(min);
    if announce {
        let who = (*x.src.display).clone();
        let t = Text::tr("commands.random.roll", vec![who, lit(value), lit(min), lit(max)]);
        x.g.broadcast_msg(&t);
    } else {
        x.success(Text::tr("commands.random.sample.success", vec![lit(value)]), false);
    }
    Ok(value)
}

fn reset_done(x: &mut Ctx, count: usize) -> CmdResult<i32> {
    x.success(Text::tr("commands.random.reset.all.success", vec![lit(count)]), false);
    Ok(count as i32)
}

fn clear_with_defaults(seed: Option<(i32, bool, bool)>) -> usize {
    SEQS.with(|s| {
        let mut s = s.borrow_mut();
        if let Some((salt, w, q)) = seed {
            s.salt = salt;
            s.include_world_seed = w;
            s.include_sequence_id = q;
        }
        let n = s.map.len();
        s.map.clear();
        n
    })
}

fn reset_all(x: &mut Ctx) -> CmdResult<i32> {
    let n = clear_with_defaults(None);
    reset_done(x, n)
}

fn reset_all_with(x: &mut Ctx, inc_world: Option<bool>) -> CmdResult<i32> {
    let salt = x.int("seed");
    let n = clear_with_defaults(Some((salt, inc_world.unwrap_or(true), true)));
    reset_done(x, n)
}

fn reset_all_full(x: &mut Ctx) -> CmdResult<i32> {
    let salt = x.int("seed");
    let w = x.bool("includeWorldSeed");
    let q = x.bool("includeSequenceId");
    let n = clear_with_defaults(Some((salt, w, q)));
    reset_done(x, n)
}

fn reset_seq_msg(x: &mut Ctx, id: &str) -> CmdResult<i32> {
    x.success(Text::tr("commands.random.reset.success", vec![lit(id)]), false);
    Ok(1)
}

fn reset_seq(x: &mut Ctx) -> CmdResult<i32> {
    let id = x.id("sequence").to_string();
    let seed = world_seed(x);
    SEQS.with(|s| {
        let mut s = s.borrow_mut();
        let st = s.create(&id, seed, s.salt, s.include_world_seed, s.include_sequence_id);
        s.map.insert(id.clone(), st);
    });
    reset_seq_msg(x, &id)
}

fn reset_seq_custom(x: &mut Ctx, salt: i32, inc_world: bool, inc_seq: bool) -> CmdResult<i32> {
    let id = x.id("sequence").to_string();
    let seed = world_seed(x);
    SEQS.with(|s| {
        let mut s = s.borrow_mut();
        let st = s.create(&id, seed, salt, inc_world, inc_seq);
        s.map.insert(id.clone(), st);
    });
    reset_seq_msg(x, &id)
}

fn reset_seq_with(x: &mut Ctx) -> CmdResult<i32> {
    let salt = x.int("seed");
    reset_seq_custom(x, salt, true, true)
}

fn reset_seq_world(x: &mut Ctx) -> CmdResult<i32> {
    let salt = x.int("seed");
    let w = x.bool("includeWorldSeed");
    reset_seq_custom(x, salt, w, true)
}

fn reset_seq_full(x: &mut Ctx) -> CmdResult<i32> {
    let salt = x.int("seed");
    let w = x.bool("includeWorldSeed");
    let q = x.bool("includeSequenceId");
    reset_seq_custom(x, salt, w, q)
}
