//! `net.minecraft.util.Mth.sin` / `Mth.cos`: a 65536-entry float table of `Math.sin`, indexed by the
//! argument scaled by 10430.378350470453 (vanilla `SIN_SCALE`). Only the carvers use it.

use std::sync::OnceLock;

const SIN_SCALE: f64 = 10430.378350470453;
const COS_OFFSET: f64 = 16384.0;

fn table() -> &'static [f32; 65536] {
    static TABLE: OnceLock<Box<[f32; 65536]>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = Box::new([0f32; 65536]);
        for (i, v) in t.iter_mut().enumerate() {
            *v = (i as f64 / SIN_SCALE).sin() as f32;
        }
        t
    })
}

/// `Mth.sin(double)`: the argument is a float widened to double by the callers.
#[inline]
pub fn sin(v: f64) -> f32 {
    table()[((v * SIN_SCALE) as i64 & 65535) as usize]
}

/// `Mth.cos(double)`.
#[inline]
pub fn cos(v: f64) -> f32 {
    table()[((v * SIN_SCALE + COS_OFFSET) as i64 & 65535) as usize]
}

/// The raw table, for the golden check against the JVM table.
pub fn sin_table() -> &'static [f32; 65536] {
    table()
}
