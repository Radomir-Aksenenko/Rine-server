//! Compiled density tree. Every node has two evaluators that mirror the vanilla 26.3 sampler
//! classes: `value` (`DensitySampler.sampleValue`, one block) and `volume` (`sampleVolume`, a whole
//! `DensityVolume` into a flat buffer). The two paths are NOT interchangeable in vanilla: they
//! differ in signed zero (`MulSampler`/`DivSampler` short-circuit in `sampleValue`), in the
//! Perlin/Smeared gradient summation order, in `Interpolated` corner evaluation, and in
//! `ShiftB`/`NoiseFunction` amplitude placement. Each node keeps the branch of its Java class.

use crate::density::spline::{self, Sp};
use crate::volume::Vol;
use crate::mth::{clamp_f32, java_max, java_min, java_signum, lerp, lerp3};
use crate::noise::NoiseStack;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Abs,
    Square,
    Cube,
    Sqrt,
    HalfNegative,
    QuarterNegative,
    Reciprocal,
    Negate,
    Squeeze,
    Sign,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tiling {
    ClampToEdge,
    Repeat,
    MirroredRepeat,
}

#[derive(Clone, Debug)]
pub struct InterpInfo {
    pub input: u32,
    pub cell_xz: i32,
    pub cell_y: i32,
    /// `1.0F / cellSizeXz`.
    pub inv_xz: f32,
    /// `1.0F / cellSizeY`.
    pub inv_y: f32,
}

#[derive(Clone, Debug)]
pub enum Node {
    Const(f32),
    /// A constant that Java does not recognise as `ConstantFunction` (blend_alpha, blend_offset,
    /// beardifier fall back to a context-bound sampler). Keeps the generic (non-specialised) path.
    Opaque(f32),
    /// `BlendDensityFunction` without a blender: returns the input unchanged, but is not a constant.
    Passthrough(u32),
    /// `AddSampler`: `left + right`.
    Add(u32, u32),
    /// `ConstAddSampler`: `child + c`.
    AddC(u32, f32),
    /// `SubSampler`: `left - right`.
    Sub(u32, u32),
    /// `ConstSubSampler`: `c - child`.
    SubC(f32, u32),
    /// `MulSampler`: value path returns 0 when left == 0.
    Mul(u32, u32),
    /// `ConstMulSampler`: `child * c` (also used for `div` by a constant, with `1/c`).
    MulC(u32, f32),
    /// `DivSampler`: value path returns 0 when left == 0.
    Div(u32, u32),
    /// `ConstDivSampler`: `c / child`.
    DivC(f32, u32),
    /// `MinSampler(left, right, rightMin)`.
    Min(u32, u32, f32),
    /// `MaxSampler(left, right, rightMax)`.
    Max(u32, u32, f32),
    MinC(u32, f32),
    MaxC(u32, f32),
    Unary(UnaryOp, u32),
    Clamp(u32, f32, f32),
    Lerp(u32, u32, u32),
    RangeChoice {
        input: u32,
        min: f32,
        max: f32,
        in_range: u32,
        out_range: u32,
    },
    IntervalSelect {
        input: u32,
        thresholds: Vec<f32>,
        children: Vec<u32>,
    },
    Spline {
        coords: Vec<u32>,
        sp: Sp,
    },
    /// `NoiseFunction.Sampler` (no shifts).
    NoiseSample {
        noise: usize,
        xz: f64,
        y: f64,
    },
    /// `NoiseFunction.ShiftedXzSampler`.
    ShiftedXz {
        shift_x: u32,
        shift_z: u32,
        noise: usize,
        xz: f64,
        y: f64,
    },
    /// `NoiseFunction.ShiftedXyzSampler`.
    ShiftedXyz {
        shift_x: u32,
        shift_y: u32,
        shift_z: u32,
        noise: usize,
        xz: f64,
        y: f64,
    },
    /// `ShiftNoiseFunction.ShiftB` sampler (transposed 2D noise, times 4).
    ShiftB {
        noise: usize,
    },
    Gradient {
        axis: Axis,
        tiling: Tiling,
        from_coord: i32,
        min_coord: i32,
        max_coord: i32,
        coord_range: i32,
        from_value: f32,
        factor: f32,
    },
    Interpolated(InterpInfo),
    /// `FindTopSurfaceFunction` wrapped in `SliceFunction.YSampler(…, 0)`.
    FindTopSurface {
        density: u32,
        upper: u32,
        lower: i32,
        cell_height: i32,
    },
    Cache {
        id: usize,
        input: u32,
    },
}

/// `SamplerContext` cache cell (`CacheCell`).
#[derive(Clone, Debug)]
struct CacheCell {
    volume: Option<Vol>,
    buffer: Vec<f32>,
    value_key: i64,
    value: f32,
}

/// `SamplerContext`. `cells == None` is `EMPTY_UNCACHED`: cache nodes pass through.
pub struct Ctx {
    cells: Option<Vec<CacheCell>>,
}

impl Ctx {
    pub fn uncached() -> Self {
        Self { cells: None }
    }

    pub fn cached(cache_count: usize) -> Self {
        Self {
            cells: Some(
                (0..cache_count)
                    .map(|_| CacheCell {
                        volume: None,
                        buffer: Vec::new(),
                        value_key: 0,
                        value: f32::NAN,
                    })
                    .collect(),
            ),
        }
    }
}

/// `BlockPos.asLong`.
fn block_key(x: i32, y: i32, z: i32) -> i64 {
    ((x as i64 & 0x3FF_FFFF) << 38) | ((z as i64 & 0x3FF_FFFF) << 12) | (y as i64 & 0xFFF)
}

/// `Mth.clamp(int, int, int)`.
#[inline]
fn clamp_i(v: i32, min: i32, max: i32) -> i32 {
    v.max(min).min(max)
}

/// `IntervalSelectFunction.selectSamplerIndex`.
#[inline]
fn select_index(thresholds: &[f32], count: usize, input: f32) -> usize {
    for (i, t) in thresholds.iter().enumerate() {
        if input < *t {
            return i;
        }
    }
    count - 1
}

impl UnaryOp {
    #[inline]
    fn apply(self, v: f32) -> f32 {
        match self {
            UnaryOp::Abs => v.abs(),
            UnaryOp::Square => v * v,
            UnaryOp::Cube => v * v * v,
            UnaryOp::Sqrt => v.sqrt(),
            UnaryOp::HalfNegative => {
                if v > 0.0 {
                    v
                } else {
                    v * 0.5
                }
            }
            UnaryOp::QuarterNegative => {
                if v > 0.0 {
                    v
                } else {
                    v * 0.25
                }
            }
            UnaryOp::Reciprocal => 1.0 / v,
            UnaryOp::Negate => -v,
            UnaryOp::Squeeze => {
                let c = clamp_f32(v, -1.0, 1.0);
                c / 2.0 - (c * c * c) / 24.0
            }
            UnaryOp::Sign => java_signum(v),
        }
    }
}

/// A compiled tree plus its noise instances.
pub struct Tree {
    pub nodes: Vec<Node>,
    pub noises: Vec<NoiseStack>,
    pub cache_count: usize,
}

impl Tree {
    /// Whether node `id` gives the same value at every y of a column (no Y gradient, every noise sampled
    /// with a zero y scale, no interpolation or surface search). Conservative: `false` when unsure.
    pub fn y_free(&self, id: u32) -> bool {
        fn walk(t: &Tree, id: u32, memo: &mut Vec<Option<bool>>) -> bool {
            if let Some(v) = memo[id as usize] {
                return v;
            }
            let mut all = |ids: &[u32], memo: &mut Vec<Option<bool>>| ids.iter().all(|&c| walk(t, c, memo));
            let v = match &t.nodes[id as usize] {
                Node::Const(_) | Node::Opaque(_) => true,
                Node::Passthrough(a) | Node::AddC(a, _) | Node::SubC(_, a) | Node::MulC(a, _) | Node::DivC(_, a) => all(&[*a], memo),
                Node::MinC(a, _) | Node::MaxC(a, _) | Node::Unary(_, a) | Node::Clamp(a, _, _) => all(&[*a], memo),
                Node::Add(a, b) | Node::Sub(a, b) | Node::Mul(a, b) | Node::Div(a, b) => all(&[*a, *b], memo),
                Node::Min(a, b, _) | Node::Max(a, b, _) => all(&[*a, *b], memo),
                Node::Lerp(a, b, c) => all(&[*a, *b, *c], memo),
                Node::RangeChoice { input, in_range, out_range, .. } => all(&[*input, *in_range, *out_range], memo),
                Node::IntervalSelect { input, children, .. } => all(&[*input], memo) && all(children, memo),
                Node::Spline { coords, .. } => all(coords, memo),
                Node::NoiseSample { y, .. } => *y == 0.0,
                Node::ShiftedXz { shift_x, shift_z, y, .. } => *y == 0.0 && all(&[*shift_x, *shift_z], memo),
                Node::ShiftedXyz { .. } => false,
                Node::ShiftB { .. } => true,
                Node::Gradient { axis, .. } => *axis != Axis::Y,
                Node::Interpolated(_) | Node::FindTopSurface { .. } => false,
                Node::Cache { input, .. } => all(&[*input], memo),
            };
            memo[id as usize] = Some(v);
            v
        }
        walk(self, id, &mut vec![None; self.nodes.len()])
    }

    /// `sampleValue` at one block position.
    pub fn value(&self, id: u32, ctx: &mut Ctx, x: i32, y: i32, z: i32) -> f32 {
        match &self.nodes[id as usize] {
            Node::Const(c) | Node::Opaque(c) => *c,
            Node::Passthrough(a) => self.value(*a, ctx, x, y, z),
            Node::Add(a, b) => {
                let l = self.value(*a, ctx, x, y, z);
                l + self.value(*b, ctx, x, y, z)
            }
            Node::AddC(a, c) => self.value(*a, ctx, x, y, z) + c,
            Node::Sub(a, b) => {
                let l = self.value(*a, ctx, x, y, z);
                l - self.value(*b, ctx, x, y, z)
            }
            Node::SubC(c, r) => c - self.value(*r, ctx, x, y, z),
            Node::Mul(a, b) => {
                let l = self.value(*a, ctx, x, y, z);
                if l == 0.0 {
                    0.0
                } else {
                    l * self.value(*b, ctx, x, y, z)
                }
            }
            Node::MulC(a, c) => self.value(*a, ctx, x, y, z) * c,
            Node::Div(a, b) => {
                let l = self.value(*a, ctx, x, y, z);
                if l == 0.0 {
                    0.0
                } else {
                    l / self.value(*b, ctx, x, y, z)
                }
            }
            Node::DivC(c, r) => c / self.value(*r, ctx, x, y, z),
            Node::Min(a, b, right_min) => {
                let l = self.value(*a, ctx, x, y, z);
                if l <= *right_min {
                    l
                } else {
                    java_min(l, self.value(*b, ctx, x, y, z))
                }
            }
            Node::Max(a, b, right_max) => {
                let l = self.value(*a, ctx, x, y, z);
                if l >= *right_max {
                    l
                } else {
                    java_max(l, self.value(*b, ctx, x, y, z))
                }
            }
            Node::MinC(a, c) => java_min(self.value(*a, ctx, x, y, z), *c),
            Node::MaxC(a, c) => java_max(self.value(*a, ctx, x, y, z), *c),
            Node::Unary(op, a) => op.apply(self.value(*a, ctx, x, y, z)),
            Node::Clamp(a, min, max) => clamp_f32(self.value(*a, ctx, x, y, z), *min, *max),
            Node::Lerp(alpha, first, second) => {
                let a = self.value(*alpha, ctx, x, y, z);
                if a == 0.0 {
                    self.value(*first, ctx, x, y, z)
                } else if a == 1.0 {
                    self.value(*second, ctx, x, y, z)
                } else {
                    let f = self.value(*first, ctx, x, y, z);
                    lerp(a, f, self.value(*second, ctx, x, y, z))
                }
            }
            Node::RangeChoice {
                input,
                min,
                max,
                in_range,
                out_range,
            } => {
                let v = self.value(*input, ctx, x, y, z);
                if v >= *min && v < *max {
                    self.value(*in_range, ctx, x, y, z)
                } else {
                    self.value(*out_range, ctx, x, y, z)
                }
            }
            Node::IntervalSelect {
                input,
                thresholds,
                children,
            } => {
                let v = self.value(*input, ctx, x, y, z);
                let idx = select_index(thresholds, children.len(), v);
                self.value(children[idx], ctx, x, y, z)
            }
            Node::Spline { coords, sp } => {
                let vals: Vec<f32> = coords.iter().map(|&c| self.value(c, ctx, x, y, z)).collect();
                spline::eval(sp, &|slot| vals[slot])
            }
            Node::NoiseSample { noise, xz, y: ys } => {
                self.noises[*noise].get_value(x as f64 * xz, y as f64 * ys, z as f64 * xz)
            }
            Node::ShiftedXz {
                shift_x,
                shift_z,
                noise,
                xz,
                y: ys,
            } => {
                let sx = self.value(*shift_x, ctx, x, y, z) as f64;
                let sz = self.value(*shift_z, ctx, x, y, z) as f64;
                let nx = x as f64 * xz + sx;
                let ny = y as f64 * ys;
                let nz = z as f64 * xz + sz;
                self.noises[*noise].get_value(nx, ny, nz)
            }
            Node::ShiftedXyz {
                shift_x,
                shift_y,
                shift_z,
                noise,
                xz,
                y: ys,
            } => {
                let sx = self.value(*shift_x, ctx, x, y, z) as f64;
                let sy = self.value(*shift_y, ctx, x, y, z) as f64;
                let sz = self.value(*shift_z, ctx, x, y, z) as f64;
                let nx = x as f64 * xz + sx;
                let ny = y as f64 * ys + sy;
                let nz = z as f64 * xz + sz;
                self.noises[*noise].get_value(nx, ny, nz)
            }
            Node::ShiftB { noise } => {
                self.noises[*noise].get_value(z as f64 * 0.25, x as f64 * 0.25, 0.0) * 4.0
            }
            Node::Gradient { .. } => gradient_value(&self.nodes[id as usize], x, y, z),
            Node::Interpolated(info) => self.interp_value(info, ctx, x, y, z),
            Node::FindTopSurface {
                density,
                upper,
                lower,
                cell_height,
            } => {
                // YSampler.sampleValue: the upper bound is evaluated at y = 0.
                let ub = self.value(*upper, ctx, x, 0, z);
                self.find_surface(*density, ctx, x, z, ub, *lower, *cell_height)
            }
            Node::Cache { id: cache_id, input } => {
                self.cache_value(*cache_id, *input, ctx, x, y, z)
            }
        }
    }

    /// `FindTopSurfaceFunction.Sampler.findSurfaceFrom`. Probes use the point path of `density`.
    #[allow(clippy::too_many_arguments)]
    fn find_surface(
        &self,
        density: u32,
        ctx: &mut Ctx,
        x: i32,
        z: i32,
        upper: f32,
        lower: i32,
        cell_height: i32,
    ) -> f32 {
        let top_y = ((upper / cell_height as f32).floor() as i32).wrapping_mul(cell_height);
        if top_y <= lower {
            return lower as f32;
        }
        let mut probe = top_y;
        while probe >= lower {
            if self.value(density, ctx, x, probe, z) > 0.0 {
                return probe as f32;
            }
            probe -= cell_height;
        }
        lower as f32
    }

    /// `sampleVolume`: writes the value of every block of `vol` into `out` (layout of `Vol::index`).
    pub fn volume(&self, id: u32, ctx: &mut Ctx, vol: &Vol, out: &mut [f32]) {
        debug_assert_eq!(out.len(), vol.size());
        match &self.nodes[id as usize] {
            Node::Const(c) | Node::Opaque(c) => out.fill(*c),
            Node::Passthrough(a) => self.volume(*a, ctx, vol, out),
            Node::Add(a, b) => {
                self.volume(*a, ctx, vol, out);
                let r = self.tmp(*b, ctx, vol);
                for (o, r) in out.iter_mut().zip(&r) {
                    *o = *o + r;
                }
            }
            Node::AddC(a, c) => {
                self.volume(*a, ctx, vol, out);
                for o in out.iter_mut() {
                    *o = *o + c;
                }
            }
            Node::Sub(a, b) => {
                self.volume(*a, ctx, vol, out);
                let r = self.tmp(*b, ctx, vol);
                for (o, r) in out.iter_mut().zip(&r) {
                    // SubSampler adds the negated right value (addTo(-r)).
                    *o = *o + (-r);
                }
            }
            Node::SubC(c, r) => {
                self.volume(*r, ctx, vol, out);
                for o in out.iter_mut() {
                    *o = c - *o;
                }
            }
            Node::Mul(a, b) => {
                self.volume(*a, ctx, vol, out);
                let r = self.tmp(*b, ctx, vol);
                for (o, r) in out.iter_mut().zip(&r) {
                    *o = *o * r;
                }
            }
            Node::MulC(a, c) => {
                self.volume(*a, ctx, vol, out);
                for o in out.iter_mut() {
                    *o = *o * c;
                }
            }
            Node::Div(a, b) => {
                self.volume(*a, ctx, vol, out);
                let r = self.tmp(*b, ctx, vol);
                for (o, r) in out.iter_mut().zip(&r) {
                    *o = *o / r;
                }
            }
            Node::DivC(c, r) => {
                self.volume(*r, ctx, vol, out);
                for o in out.iter_mut() {
                    *o = c / *o;
                }
            }
            Node::Min(a, b, _) => {
                self.volume(*a, ctx, vol, out);
                let r = self.tmp(*b, ctx, vol);
                for (o, &r) in out.iter_mut().zip(&r) {
                    if r < *o {
                        *o = r;
                    }
                }
            }
            Node::Max(a, b, _) => {
                self.volume(*a, ctx, vol, out);
                let r = self.tmp(*b, ctx, vol);
                for (o, &r) in out.iter_mut().zip(&r) {
                    if r > *o {
                        *o = r;
                    }
                }
            }
            Node::MinC(a, c) => {
                self.volume(*a, ctx, vol, out);
                for o in out.iter_mut() {
                    if *c < *o {
                        *o = *c;
                    }
                }
            }
            Node::MaxC(a, c) => {
                self.volume(*a, ctx, vol, out);
                for o in out.iter_mut() {
                    if *c > *o {
                        *o = *c;
                    }
                }
            }
            Node::Unary(op, a) => {
                self.volume(*a, ctx, vol, out);
                for o in out.iter_mut() {
                    *o = op.apply(*o);
                }
            }
            Node::Clamp(a, min, max) => {
                self.volume(*a, ctx, vol, out);
                for o in out.iter_mut() {
                    *o = clamp_f32(*o, *min, *max);
                }
            }
            Node::Lerp(alpha, first, second) => {
                self.volume(*alpha, ctx, vol, out);
                let f = self.tmp(*first, ctx, vol);
                let s = self.tmp(*second, ctx, vol);
                for (i, o) in out.iter_mut().enumerate() {
                    let a = *o;
                    *o = if a == 0.0 {
                        f[i]
                    } else if a == 1.0 {
                        s[i]
                    } else {
                        lerp(a, f[i], s[i])
                    };
                }
            }
            Node::RangeChoice {
                input,
                min,
                max,
                in_range,
                out_range,
            } => {
                self.volume(*in_range, ctx, vol, out);
                let inp = self.tmp(*input, ctx, vol);
                let outr = self.tmp(*out_range, ctx, vol);
                for (i, o) in out.iter_mut().enumerate() {
                    let v = inp[i];
                    if !(v >= *min) || !(v < *max) {
                        *o = outr[i];
                    }
                }
            }
            Node::IntervalSelect {
                input,
                thresholds,
                children,
            } => {
                self.volume(*input, ctx, vol, out);
                let bufs: Vec<Vec<f32>> = children.iter().map(|&c| self.tmp(c, ctx, vol)).collect();
                for (i, o) in out.iter_mut().enumerate() {
                    let idx = select_index(thresholds, children.len(), *o);
                    *o = bufs[idx][i];
                }
            }
            Node::Spline { coords, sp } => {
                let bufs: Vec<Vec<f32>> = coords.iter().map(|&c| self.tmp(c, ctx, vol)).collect();
                for (i, o) in out.iter_mut().enumerate() {
                    *o = spline::eval(sp, &|slot| bufs[slot][i]);
                }
            }
            Node::NoiseSample { noise, xz, y: ys } => {
                out.fill(0.0);
                self.noises[*noise].add_to_volume(out, vol, *xz, *ys, 1.0);
            }
            Node::ShiftedXz {
                shift_x,
                shift_z,
                noise,
                xz,
                y: ys,
            } => {
                self.volume(*shift_x, ctx, vol, out);
                let sz_buf = self.tmp(*shift_z, ctx, vol);
                let n = &self.noises[*noise];
                let mut index = 0usize;
                for z in 0..vol.sz {
                    let base_z = vol.block_z(z) as f64 * xz;
                    for x in 0..vol.sx {
                        let base_x = vol.block_x(x) as f64 * xz;
                        for y in 0..vol.sy {
                            let nx = base_x + out[index] as f64;
                            let ny = vol.block_y(y) as f64 * ys;
                            let nz = base_z + sz_buf[index] as f64;
                            out[index] = n.get_value(nx, ny, nz);
                            index += 1;
                        }
                    }
                }
            }
            Node::ShiftedXyz {
                shift_x,
                shift_y,
                shift_z,
                noise,
                xz,
                y: ys,
            } => {
                self.volume(*shift_x, ctx, vol, out);
                let sy_buf = self.tmp(*shift_y, ctx, vol);
                let sz_buf = self.tmp(*shift_z, ctx, vol);
                let n = &self.noises[*noise];
                let mut index = 0usize;
                for z in 0..vol.sz {
                    let base_z = vol.block_z(z) as f64 * xz;
                    for x in 0..vol.sx {
                        let base_x = vol.block_x(x) as f64 * xz;
                        for y in 0..vol.sy {
                            let nx = base_x + out[index] as f64;
                            let ny = vol.block_y(y) as f64 * ys + sy_buf[index] as f64;
                            let nz = base_z + sz_buf[index] as f64;
                            out[index] = n.get_value(nx, ny, nz);
                            index += 1;
                        }
                    }
                }
            }
            Node::ShiftB { noise } => {
                let tv = Vol::with_step(vol.sz, vol.sx, 1, vol.mz, vol.mx, 0, vol.stz, vol.stx, 1);
                let mut tb = vec![0f32; tv.size()];
                self.noises[*noise].add_to_volume(&mut tb, &tv, 0.25, 0.25, 4.0);
                for z in 0..vol.sz {
                    for x in 0..vol.sx {
                        let v = tb[tv.index(z, x, 0)];
                        for y in 0..vol.sy {
                            out[vol.index(x, y, z)] = v;
                        }
                    }
                }
            }
            Node::Gradient { .. } => {
                let node = &self.nodes[id as usize];
                for x in 0..vol.sx {
                    for z in 0..vol.sz {
                        for y in 0..vol.sy {
                            out[vol.index(x, y, z)] =
                                gradient_value(node, vol.block_x(x), vol.block_y(y), vol.block_z(z));
                        }
                    }
                }
            }
            Node::Interpolated(info) => self.interp_volume(info, ctx, vol, out),
            Node::FindTopSurface {
                density,
                upper,
                lower,
                cell_height,
            } => {
                if vol.sy == 1 && vol.my == 0 {
                    self.find_top_surface_volume(*density, *upper, *lower, *cell_height, ctx, vol, out);
                } else {
                    // YSampler: evaluate on a single-row volume at y = 0, then broadcast down Y.
                    let row = Vol::with_step(vol.sx, 1, vol.sz, vol.mx, 0, vol.mz, vol.stx, vol.sty, vol.stz);
                    let mut buf = vec![0f32; row.size()];
                    self.find_top_surface_volume(*density, *upper, *lower, *cell_height, ctx, &row, &mut buf);
                    for z in 0..vol.sz {
                        for x in 0..vol.sx {
                            let v = buf[row.index(x, 0, z)];
                            for y in 0..vol.sy {
                                out[vol.index(x, y, z)] = v;
                            }
                        }
                    }
                }
            }
            Node::Cache { id: cache_id, input } => {
                self.cache_volume(*cache_id, *input, ctx, vol, out)
            }
        }
    }

    /// `FindTopSurfaceFunction.Sampler.sampleVolume` on a volume with `sizeY == 1`.
    #[allow(clippy::too_many_arguments)]
    fn find_top_surface_volume(
        &self,
        density: u32,
        upper: u32,
        lower: i32,
        cell_height: i32,
        ctx: &mut Ctx,
        vol: &Vol,
        out: &mut [f32],
    ) {
        self.volume(upper, ctx, vol, out);
        let mut index = 0usize;
        for z in 0..vol.sz {
            let bz = vol.block_z(z);
            for x in 0..vol.sx {
                let bx = vol.block_x(x);
                let ub = out[index];
                out[index] = self.find_surface(density, ctx, bx, bz, ub, lower, cell_height);
                index += 1;
            }
        }
    }

    fn tmp(&self, id: u32, ctx: &mut Ctx, vol: &Vol) -> Vec<f32> {
        let mut b = vec![0f32; vol.size()];
        self.volume(id, ctx, vol, &mut b);
        b
    }

    /// `InterpolatedFunction.Sampler.sampleValue`.
    fn interp_value(&self, info: &InterpInfo, ctx: &mut Ctx, x: i32, y: i32, z: i32) -> f32 {
        let xi = x.rem_euclid(info.cell_xz);
        let yi = y.rem_euclid(info.cell_y);
        let zi = z.rem_euclid(info.cell_xz);
        if xi == 0 && yi == 0 && zi == 0 {
            return self.value(info.input, ctx, x, y, z);
        }
        let vol = Vol::with_step(2, 2, 2, x - xi, y - yi, z - zi, info.cell_xz, info.cell_y, info.cell_xz);
        let mut b = [0f32; 8];
        self.volume(info.input, ctx, &vol, &mut b);
        // Index of (xx, yy, zz) in a 2x2x2 volume: yy + (xx + zz * 2) * 2.
        let at = |xx: usize, yy: usize, zz: usize| b[yy + (xx + zz * 2) * 2];
        lerp3(
            xi as f32 / info.cell_xz as f32,
            yi as f32 / info.cell_y as f32,
            zi as f32 / info.cell_xz as f32,
            at(0, 0, 0),
            at(1, 0, 0),
            at(0, 1, 0),
            at(1, 1, 0),
            at(0, 0, 1),
            at(1, 0, 1),
            at(0, 1, 1),
            at(1, 1, 1),
        )
    }

    /// `InterpolatedFunction.Sampler.sampleVolume`.
    fn interp_volume(&self, info: &InterpInfo, ctx: &mut Ctx, vol: &Vol, out: &mut [f32]) {
        let cxz = info.cell_xz;
        let cy = info.cell_y;
        if (vol.stx == cxz || vol.sx == 1)
            && (vol.sty == cy || vol.sy == 1)
            && (vol.stz == cxz || vol.sz == 1)
            && vol.mx.rem_euclid(cxz) == 0
            && vol.my.rem_euclid(cy) == 0
            && vol.mz.rem_euclid(cxz) == 0
        {
            self.volume(info.input, ctx, vol, out);
        } else if vol.stx == 1 && vol.sty == 1 && vol.stz == 1 {
            self.interp_block_step(info, ctx, vol, out);
        } else {
            let block_vol = Vol::with_step(
                vol.sx * vol.stx,
                vol.sy * vol.sty,
                vol.sz * vol.stz,
                vol.mx,
                vol.my,
                vol.mz,
                1,
                1,
                1,
            );
            let mut block = vec![0f32; block_vol.size()];
            self.interp_block_step(info, ctx, &block_vol, &mut block);
            for z in 0..vol.sz {
                for x in 0..vol.sx {
                    for y in 0..vol.sy {
                        out[vol.index(x, y, z)] =
                            block[block_vol.index(x * vol.stx, y * vol.sty, z * vol.stz)];
                    }
                }
            }
        }
    }

    /// `InterpolatedFunction.Sampler.sampleWithBlockStep`.
    fn interp_block_step(&self, info: &InterpInfo, ctx: &mut Ctx, vol: &Vol, out: &mut [f32]) {
        let cxz = info.cell_xz;
        let cy = info.cell_y;
        let min_cell_x = vol.mx.div_euclid(cxz);
        let min_cell_y = vol.my.div_euclid(cy);
        let min_cell_z = vol.mz.div_euclid(cxz);
        let max_cell_x = vol.max_block_x().div_euclid(cxz);
        let max_cell_y = vol.max_block_y().div_euclid(cy);
        let max_cell_z = vol.max_block_z().div_euclid(cxz);
        let count_x = max_cell_x - min_cell_x + 1;
        let count_y = max_cell_y - min_cell_y + 1;
        let count_z = max_cell_z - min_cell_z + 1;
        let extra = |max: i32, cell: i32, count: i32| {
            if max.rem_euclid(cell) == 0 {
                count
            } else {
                count + 1
            }
        };
        let cell_vol = Vol::with_step(
            extra(vol.max_block_x(), cxz, count_x),
            extra(vol.max_block_y(), cy, count_y),
            extra(vol.max_block_z(), cxz, count_z),
            min_cell_x * cxz,
            min_cell_y * cy,
            min_cell_z * cxz,
            cxz,
            cy,
            cxz,
        );
        let mut cb = vec![0f32; cell_vol.size()];
        self.volume(info.input, ctx, &cell_vol, &mut cb);
        for cz in 0..count_z {
            let next_z = (cz + 1).min(cell_vol.sz - 1);
            for cx in 0..count_x {
                let next_x = (cx + 1).min(cell_vol.sx - 1);
                let mut v000 = cb[cell_vol.index(cx, 0, cz)];
                let mut v100 = cb[cell_vol.index(next_x, 0, cz)];
                let mut v001 = cb[cell_vol.index(cx, 0, next_z)];
                let mut v101 = cb[cell_vol.index(next_x, 0, next_z)];
                for cy_ in 0..count_y {
                    let next_y = (cy_ + 1).min(cell_vol.sy - 1);
                    let v010 = cb[cell_vol.index(cx, next_y, cz)];
                    let v110 = cb[cell_vol.index(next_x, next_y, cz)];
                    let v011 = cb[cell_vol.index(cx, next_y, next_z)];
                    let v111 = cb[cell_vol.index(next_x, next_y, next_z)];
                    fill_cell(
                        out,
                        vol,
                        &cell_vol,
                        info,
                        (cx, cy_, cz),
                        [v000, v100, v010, v110, v001, v101, v011, v111],
                    );
                    v000 = v010;
                    v100 = v110;
                    v001 = v011;
                    v101 = v111;
                }
            }
        }
    }

    /// `SamplerContext.sampleValueCached` (identity when caches are disabled).
    fn cache_value(&self, id: usize, input: u32, ctx: &mut Ctx, x: i32, y: i32, z: i32) -> f32 {
        if ctx.cells.is_none() {
            return self.value(input, ctx, x, y, z);
        }
        let key = block_key(x, y, z);
        let hit = {
            let cell = &ctx.cells.as_ref().expect("caches enabled")[id];
            if cell.value_key == key && !cell.value.is_nan() {
                return cell.value;
            }
            match cell.volume {
                Some(vol) if !cell.buffer.is_empty() => {
                    vol.index_of_block(x, y, z).map(|i| cell.buffer[i])
                }
                _ => None,
            }
        };
        if let Some(v) = hit {
            return v;
        }
        let v = self.value(input, ctx, x, y, z);
        let cells = ctx.cells.as_mut().expect("caches enabled");
        cells[id].value_key = key;
        cells[id].value = v;
        v
    }

    /// `SamplerContext.sampleVolumeCached`.
    fn cache_volume(&self, id: usize, input: u32, ctx: &mut Ctx, vol: &Vol, out: &mut [f32]) {
        if ctx.cells.is_none() {
            self.volume(input, ctx, vol, out);
            return;
        }
        let stale = {
            let cell = &ctx.cells.as_ref().expect("caches enabled")[id];
            cell.volume != Some(*vol) || cell.buffer.is_empty()
        };
        if stale {
            let mut buf = vec![0f32; vol.size()];
            self.volume(input, ctx, vol, &mut buf);
            let cells = ctx.cells.as_mut().expect("caches enabled");
            cells[id].volume = Some(*vol);
            cells[id].buffer = buf;
        }
        let cells = ctx.cells.as_ref().expect("caches enabled");
        out.copy_from_slice(&cells[id].buffer);
    }
}

/// `GradientFunction` sampler `compute`, evaluated at one block position.
fn gradient_value(node: &Node, x: i32, y: i32, z: i32) -> f32 {
    let Node::Gradient {
        axis,
        tiling,
        from_coord,
        min_coord,
        max_coord,
        coord_range,
        from_value,
        factor,
    } = node
    else {
        unreachable!("gradient_value on a non-gradient node")
    };
    let coord = match axis {
        Axis::X => x,
        Axis::Y => y,
        Axis::Z => z,
    };
    match tiling {
        Tiling::ClampToEdge => {
            let rel = clamp_i(coord, *min_coord, *max_coord) - from_coord;
            from_value + (rel as f32) * factor
        }
        Tiling::Repeat => {
            let rel = coord - from_coord;
            from_value + (rel.rem_euclid(*coord_range) as f32) * factor
        }
        Tiling::MirroredRepeat => {
            let rel = coord - from_coord;
            let tile = rel.div_euclid(*coord_range);
            let local = rel - tile * coord_range;
            if (tile & 1) == 0 {
                from_value + (local as f32) * factor
            } else {
                from_value + ((coord_range - local) as f32) * factor
            }
        }
    }
}

/// `InterpolatedFunction.fillCell`: trilinear fill of one cell into the output volume.
#[allow(clippy::too_many_arguments)]
fn fill_cell(
    out: &mut [f32],
    ovol: &Vol,
    cell_vol: &Vol,
    info: &InterpInfo,
    cell: (i32, i32, i32),
    v: [f32; 8],
) {
    let (cx, cy_, cz) = cell;
    let [v000, v100, v010, v110, v001, v101, v011, v111] = v;
    let cell_out_x = cell_vol.block_x(cx) - ovol.mx;
    let cell_out_y = cell_vol.block_y(cy_) - ovol.my;
    let cell_out_z = cell_vol.block_z(cz) - ovol.mz;
    let x0 = 0.max(-cell_out_x);
    let y0 = 0.max(-cell_out_y);
    let z0 = 0.max(-cell_out_z);
    let x1 = info.cell_xz.min(ovol.sx - cell_out_x) - 1;
    let y1 = info.cell_y.min(ovol.sy - cell_out_y) - 1;
    let z1 = info.cell_xz.min(ovol.sz - cell_out_z) - 1;
    for z in z0..=z1 {
        let output_z = cell_out_z + z;
        let alpha_z = z as f32 * info.inv_xz;
        let v00_ = lerp(alpha_z, v000, v001);
        let v01_ = lerp(alpha_z, v010, v011);
        let v10_ = lerp(alpha_z, v100, v101);
        let v11_ = lerp(alpha_z, v110, v111);
        for x in x0..=x1 {
            let output_x = cell_out_x + x;
            let alpha_x = x as f32 * info.inv_xz;
            let v_0_ = lerp(alpha_x, v00_, v10_);
            let v_1_ = lerp(alpha_x, v01_, v11_);
            let value_step = (v_1_ - v_0_) * info.inv_y;
            let mut value = v_0_ + value_step * (y0 as f32);
            let mut idx = ovol.index(output_x, cell_out_y + y0, output_z);
            for _ in y0..=y1 {
                out[idx] = value;
                idx += 1;
                value += value_step;
            }
        }
    }
}
