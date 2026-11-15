//! /random (`RandomCommand`) and the server-side RNG for selectors.

use crate::game::cmd::Commands;
use std::cell::Cell;

pub fn register(_c: &mut Commands) {}

thread_local! {
    static STATE: Cell<u64> = Cell::new(0x9E37_79B9_7F4A_7C15 ^ std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1));
}

/// Fast non-cryptographic RNG (splitmix64) for shuffles.
pub fn next_u32() -> u32 {
    STATE.with(|s| {
        let mut z = s.get().wrapping_add(0x9E37_79B9_7F4A_7C15);
        s.set(z);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 32) as u32
    })
}
