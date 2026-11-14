//! Ports of the `net.minecraft.util.Mth` helpers that the random and noise code needs.
//! Float helpers keep Java's operand order so f32 results are bit-identical.

/// `Mth.floor(double)`: `(int)Math.floor(v)`. Rust `as` saturates and maps NaN to 0, as Java does.
#[inline]
pub fn floor(v: f64) -> i32 {
    v.floor() as i32
}

/// `Mth.getSeed(x, y, z)`. Note that `x * 3129871` is an int multiply (wrapping i32) before widening.
#[inline]
pub fn get_seed(x: i32, y: i32, z: i32) -> i64 {
    let mut seed: i64 = (x.wrapping_mul(3_129_871) as i64)
        ^ (z as i64).wrapping_mul(116_129_781)
        ^ (y as i64);
    seed = seed
        .wrapping_mul(seed)
        .wrapping_mul(42_317_861)
        .wrapping_add(seed.wrapping_mul(11));
    seed >> 16
}

/// `Mth.lerp(float, float, float)`: `p0 + alpha * (p1 - p0)`.
#[inline]
pub fn lerp(alpha: f32, p0: f32, p1: f32) -> f32 {
    p0 + alpha * (p1 - p0)
}

/// `Mth.lerp(double, double, double)`.
#[inline]
pub fn lerp_d(alpha: f64, p0: f64, p1: f64) -> f64 {
    p0 + alpha * (p1 - p0)
}

/// `Mth.lerp2(float, float, x00, x10, x01, x11)`.
#[inline]
pub fn lerp2(alpha1: f32, alpha2: f32, x00: f32, x10: f32, x01: f32, x11: f32) -> f32 {
    lerp(alpha2, lerp(alpha1, x00, x10), lerp(alpha1, x01, x11))
}

/// `Mth.lerp3(float, float, float, x000, x100, x010, x110, x001, x101, x011, x111)`.
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn lerp3(
    alpha1: f32,
    alpha2: f32,
    alpha3: f32,
    x000: f32,
    x100: f32,
    x010: f32,
    x110: f32,
    x001: f32,
    x101: f32,
    x011: f32,
    x111: f32,
) -> f32 {
    lerp(
        alpha3,
        lerp2(alpha1, alpha2, x000, x100, x010, x110),
        lerp2(alpha1, alpha2, x001, x101, x011, x111),
    )
}

/// `Mth.smoothstep(float)`: `x*x*x*(x*(x*6 - 15) + 10)`.
#[inline]
pub fn smoothstep(x: f32) -> f32 {
    x * x * x * (x * (x * 6.0 - 15.0) + 10.0)
}

/// `Mth.clamp(float, float, float)`: `value < min ? min : Math.min(value, max)`.
#[inline]
pub fn clamp_f32(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

/// `Mth.square(double)`.
#[inline]
pub fn square(x: f64) -> f64 {
    x * x
}

/// `Math.min(float, float)`: NaN propagates, and `-0.0` is smaller than `0.0`.
#[inline]
pub fn java_min(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && b.to_bits() == (-0.0_f32).to_bits() {
        return b;
    }
    if a <= b {
        a
    } else {
        b
    }
}

/// `Math.max(float, float)`: NaN propagates, and `0.0` is larger than `-0.0`.
#[inline]
pub fn java_max(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.to_bits() == (-0.0_f32).to_bits() {
        return b;
    }
    if a >= b {
        a
    } else {
        b
    }
}

/// `Math.signum(float)`.
#[inline]
pub fn java_signum(f: f32) -> f32 {
    if f == 0.0 || f.is_nan() {
        f
    } else {
        1.0_f32.copysign(f)
    }
}

/// `Mth.binarySearch(from, to, predicate)`: first index in `[from, to)` where `condition` holds.
#[inline]
pub fn binary_search(mut from: i32, to: i32, condition: impl Fn(i32) -> bool) -> i32 {
    let mut len = to - from;
    while len > 0 {
        let half = len / 2;
        let middle = from + half;
        if condition(middle) {
            len = half;
        } else {
            from = middle + 1;
            len -= half + 1;
        }
    }
    from
}
