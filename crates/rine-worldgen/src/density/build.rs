//! Builds the compiled `Tree` from the bundled vanilla 26.3 JSON, mirroring
//! `DensityFunctionCompiler` + the per-class `compileSampler` specialisations that change which
//! arithmetic form a node uses (constant operands, `div` by constant as multiply by reciprocal).
//!
//! Reference strings (`"minecraft:overworld/continents"`) are inlined (`INLINE_REFERENCE`); each
//! referenced file is built once and shared. Noise names resolve to `noise/<name>.json`, which is
//! instantiated as `NormalNoise.create(RandomState.random.fromHashOf(id))`, as `Noises.instantiate`
//! does. The `SLICE_UNIFORM_AXES` rewrite is skipped: it only inserts slices that do not change
//! values (the sliced input does not depend on that axis).

use std::collections::HashMap;

use serde_json::Value;

use crate::density::assets::FILES;
use crate::density::spline::Sp;
use crate::density::tree::{Axis, InterpInfo, Node, Tiling, Tree, UnaryOp};
use crate::noise::{BlendedParts, Normalization, NormalNoise, NoiseStack};
use crate::random::{AnyRng, Positional, Xoroshiro};

pub type BuildResult<T> = Result<T, String>;

const BLENDED_NOISE_SEED: &str = "minecraft:terrain";

/// Looks up a bundled file by its path relative to the worldgen data root.
pub fn bundled(rel: &str) -> BuildResult<&'static str> {
    FILES
        .iter()
        .find(|(name, _)| *name == rel)
        .map(|(_, text)| *text)
        .ok_or_else(|| format!("missing bundled file {rel}"))
}

fn strip_ns(name: &str) -> BuildResult<&str> {
    name.strip_prefix("minecraft:")
        .ok_or_else(|| format!("expected a minecraft: id, got {name}"))
}

fn parse_json(rel: &str) -> BuildResult<Value> {
    serde_json::from_str(bundled(rel)?).map_err(|e| format!("{rel}: {e}"))
}

fn f32_of(v: &Value) -> BuildResult<f32> {
    v.as_f64()
        .map(|d| d as f32)
        .ok_or_else(|| format!("expected a number, got {v}"))
}

fn f64_of(v: &Value) -> BuildResult<f64> {
    v.as_f64()
        .ok_or_else(|| format!("expected a number, got {v}"))
}

fn i32_of(v: &Value) -> BuildResult<i32> {
    v.as_i64()
        .map(|i| i as i32)
        .ok_or_else(|| format!("expected an integer, got {v}"))
}

fn field<'a>(v: &'a Value, key: &str) -> BuildResult<&'a Value> {
    v.get(key).ok_or_else(|| format!("missing field {key} in {v}"))
}

/// Compile-time state for one `RandomState` (overworld, seed-driven).
pub struct Builder {
    nodes: Vec<Node>,
    noises: Vec<NoiseStack>,
    noise_index: HashMap<String, usize>,
    names: HashMap<String, u32>,
    cache_count: usize,
    random: Positional,
}

impl Builder {
    pub fn new(seed: i64) -> Self {
        // RandomState: `XOROSHIRO.newInstance(seed).forkPositional()`.
        let random = Xoroshiro::new(seed).fork_positional();
        Self {
            nodes: Vec::new(),
            noises: Vec::new(),
            noise_index: HashMap::new(),
            names: HashMap::new(),
            cache_count: 0,
            random,
        }
    }

    pub fn cache_count(&self) -> usize {
        self.cache_count
    }

    pub fn into_tree(self) -> Tree {
        Tree {
            nodes: self.nodes,
            noises: self.noises,
            cache_count: self.cache_count,
        }
    }

    fn push(&mut self, node: Node) -> u32 {
        self.nodes.push(node);
        (self.nodes.len() - 1) as u32
    }

    fn const_of(&self, id: u32) -> Option<f32> {
        match self.nodes[id as usize] {
            Node::Const(c) => Some(c),
            _ => None,
        }
    }

    fn is_zero(&self, id: u32) -> bool {
        matches!(self.nodes[id as usize], Node::Const(c) if c.to_bits() == 0)
    }

    /// `Noises.instantiate`: the noise for a `minecraft:` id, built once.
    pub fn noise(&mut self, name: &str) -> BuildResult<usize> {
        if let Some(&i) = self.noise_index.get(name) {
            return Ok(i);
        }
        let short = strip_ns(name)?;
        let v = parse_json(&format!("noise/{short}.json"))?;
        let base_amplitude = f64_of(field(&v, "base_amplitude")?)?;
        let base_octave = i32_of(field(&v, "base_octave")?)?;
        // NormalNoise.Builder default octave_count is 1 when the JSON omits it.
        let octave_count = match v.get("octave_count") {
            Some(c) => i32_of(c)? as usize,
            None => 1,
        };
        let modifiers: Vec<f64> = match v.get("amplitude_modifiers") {
            Some(arr) => arr
                .as_array()
                .ok_or("amplitude_modifiers must be an array")?
                .iter()
                .map(f64_of)
                .collect::<BuildResult<_>>()?,
            None => Vec::new(),
        };
        let normal = NormalNoise::new(
            base_amplitude,
            base_octave,
            octave_count,
            Normalization::Enabled,
            &modifiers,
        );
        let mut rng = AnyRng::Xoro(self.random_from_hash(name));
        let stack = normal.create(&mut rng);
        self.noises.push(stack);
        let idx = self.noises.len() - 1;
        self.noise_index.insert(name.to_string(), idx);
        Ok(idx)
    }

    fn random_from_hash(&self, name: &str) -> Xoroshiro {
        match self.random.from_hash_of(name) {
            AnyRng::Xoro(x) => x,
            AnyRng::Legacy(_) => unreachable!("overworld uses the xoroshiro source"),
        }
    }

    /// Resolves `minecraft:<path>` to a built density function (shared between references).
    pub fn reference(&mut self, name: &str) -> BuildResult<u32> {
        if let Some(&id) = self.names.get(name) {
            return Ok(id);
        }
        let short = strip_ns(name)?;
        let v = parse_json(&format!("density_function/{short}.json"))?;
        let id = self.df(&v)?;
        self.names.insert(name.to_string(), id);
        Ok(id)
    }

    /// Builds a density function JSON value (number, reference string, or typed object).
    pub fn df(&mut self, v: &Value) -> BuildResult<u32> {
        match v {
            Value::Number(_) => {
                let c = f32_of(v)?;
                Ok(self.push(Node::Const(c)))
            }
            Value::String(s) => self.reference(s),
            Value::Object(_) => self.typed(v),
            _ => Err(format!("unsupported density function {v}")),
        }
    }

    fn df_field(&mut self, v: &Value, key: &str) -> BuildResult<u32> {
        let inner = field(v, key)?;
        self.df(inner)
    }

    fn typed(&mut self, v: &Value) -> BuildResult<u32> {
        let ty = field(v, "type")?
            .as_str()
            .ok_or("type must be a string")?;
        let ty = strip_ns(ty)?;
        match ty {
            "abs" => self.unary(v, UnaryOp::Abs),
            "square" => self.unary(v, UnaryOp::Square),
            "cube" => self.unary(v, UnaryOp::Cube),
            "sqrt" => self.unary(v, UnaryOp::Sqrt),
            "half_negative" => self.unary(v, UnaryOp::HalfNegative),
            "quarter_negative" => self.unary(v, UnaryOp::QuarterNegative),
            "reciprocal" => self.unary(v, UnaryOp::Reciprocal),
            "negate" => self.unary(v, UnaryOp::Negate),
            "squeeze" => self.unary(v, UnaryOp::Squeeze),
            "sign" => self.unary(v, UnaryOp::Sign),
            "add" => self.binary(v, "add"),
            "sub" => self.binary(v, "sub"),
            "mul" => self.binary(v, "mul"),
            "div" => self.binary(v, "div"),
            "min" => self.binary(v, "min"),
            "max" => self.binary(v, "max"),
            "lerp" => {
                let a = self.df_field(v, "alpha")?;
                let f = self.df_field(v, "first")?;
                let s = self.df_field(v, "second")?;
                Ok(self.push(Node::Lerp(a, f, s)))
            }
            "clamp" => {
                let input = self.df_field(v, "input")?;
                let min = f32_of(field(v, "min")?)?;
                let max = f32_of(field(v, "max")?)?;
                Ok(self.push(Node::Clamp(input, min, max)))
            }
            "range_choice" => {
                let input = self.df_field(v, "input")?;
                let min = f32_of(field(v, "min_inclusive")?)?;
                let max = f32_of(field(v, "max_exclusive")?)?;
                let inr = self.df_field(v, "when_in_range")?;
                let outr = self.df_field(v, "when_out_of_range")?;
                Ok(self.push(Node::RangeChoice {
                    input,
                    min,
                    max,
                    in_range: inr,
                    out_range: outr,
                }))
            }
            "interval_select" => {
                let input = self.df_field(v, "input")?;
                let thresholds = field(v, "thresholds")?
                    .as_array()
                    .ok_or("thresholds must be an array")?
                    .iter()
                    .map(f32_of)
                    .collect::<BuildResult<Vec<f32>>>()?;
                let children = field(v, "functions")?
                    .as_array()
                    .ok_or("functions must be an array")?
                    .iter()
                    .map(|f| self.df(f))
                    .collect::<BuildResult<Vec<u32>>>()?;
                Ok(self.push(Node::IntervalSelect {
                    input,
                    thresholds,
                    children,
                }))
            }
            "spline" => {
                let mut coords = Vec::new();
                let sp = self.spline(field(v, "spline")?, &mut coords)?;
                match sp {
                    Sp::Constant(c) => Ok(self.push(Node::Opaque(c))),
                    sp => Ok(self.push(Node::Spline { coords, sp })),
                }
            }
            "noise" => {
                let name = field(v, "noise")?.as_str().ok_or("noise must be a name")?;
                let xz = f64_of(field(v, "xz_scale")?)?;
                let ys = f64_of(field(v, "y_scale")?)?;
                let noise = self.noise(name)?;
                let shift_x = self.optional_df(v, "shift_x")?;
                let shift_y = self.optional_df(v, "shift_y")?;
                let shift_z = self.optional_df(v, "shift_z")?;
                let zx = self.is_zero(shift_x);
                let zy = self.is_zero(shift_y);
                let zz = self.is_zero(shift_z);
                Ok(self.push(if zx && zy && zz {
                    Node::NoiseSample { noise, xz, y: ys }
                } else if zy {
                    Node::ShiftedXz {
                        shift_x,
                        shift_z,
                        noise,
                        xz,
                        y: ys,
                    }
                } else {
                    Node::ShiftedXyz {
                        shift_x,
                        shift_y,
                        shift_z,
                        noise,
                        xz,
                        y: ys,
                    }
                }))
            }
            "shift_a" => {
                let name = field(v, "noise")?.as_str().ok_or("noise must be a name")?;
                let noise = self.noise(name)?;
                let sampler = self.push(Node::NoiseSample {
                    noise,
                    xz: 0.25,
                    y: 0.0,
                });
                Ok(self.push(Node::MulC(sampler, 4.0)))
            }
            "shift_b" => {
                let name = field(v, "noise")?.as_str().ok_or("noise must be a name")?;
                let noise = self.noise(name)?;
                Ok(self.push(Node::ShiftB { noise }))
            }
            "shift" => {
                let name = field(v, "noise")?.as_str().ok_or("noise must be a name")?;
                let noise = self.noise(name)?;
                let sampler = self.push(Node::NoiseSample {
                    noise,
                    xz: 0.25,
                    y: 0.25,
                });
                Ok(self.push(Node::MulC(sampler, 4.0)))
            }
            "gradient" => {
                let axis = match field(v, "axis")?.as_str().ok_or("axis must be a string")? {
                    "x" => Axis::X,
                    "y" => Axis::Y,
                    "z" => Axis::Z,
                    other => return Err(format!("unknown axis {other}")),
                };
                let tiling = match v.get("tiling").and_then(Value::as_str) {
                    None | Some("clamp_to_edge") => Tiling::ClampToEdge,
                    Some("repeat") => Tiling::Repeat,
                    Some("mirrored_repeat") => Tiling::MirroredRepeat,
                    Some(other) => return Err(format!("unknown tiling {other}")),
                };
                let from = i32_of(field(v, "from_coordinate")?)?;
                let to = i32_of(field(v, "to_coordinate")?)?;
                let from_value = f32_of(field(v, "from_value")?)?;
                let to_value = f32_of(field(v, "to_value")?)?;
                let range = to - from;
                let factor = (to_value - from_value) / (range as f32);
                Ok(self.push(Node::Gradient {
                    axis,
                    tiling,
                    from_coord: from,
                    min_coord: from.min(to),
                    max_coord: from.max(to),
                    coord_range: range,
                    from_value,
                    factor,
                }))
            }
            "find_top_surface" => {
                let density = self.df_field(v, "density")?;
                let upper = self.df_field(v, "upper_bound")?;
                let lower = i32_of(field(v, "lower_bound")?)?;
                let cell_height = i32_of(field(v, "cell_height")?)?;
                Ok(self.push(Node::FindTopSurface {
                    density,
                    upper,
                    lower,
                    cell_height,
                }))
            }
            "cache" => {
                let input = self.df_field(v, "input")?;
                let id = self.cache_count;
                self.cache_count += 1;
                Ok(self.push(Node::Cache { id, input }))
            }
            "blend_density" => {
                let input = self.df_field(v, "input")?;
                Ok(self.push(Node::Passthrough(input)))
            }
            "blend_alpha" => Ok(self.push(Node::Opaque(1.0))),
            "blend_offset" | "beardifier" => Ok(self.push(Node::Opaque(0.0))),
            "interpolated" => {
                let input = self.df_field(v, "input")?;
                let cxz = i32_of(field(v, "cell_size_xz")?)?;
                let cy = i32_of(field(v, "cell_size_y")?)?;
                Ok(self.push(Node::Interpolated(InterpInfo {
                    input,
                    cell_xz: cxz,
                    cell_y: cy,
                    inv_xz: 1.0_f32 / (cxz as f32),
                    inv_y: 1.0_f32 / (cy as f32),
                })))
            }
            "old_blended_noise" => {
                let xz_scale = f64_of(field(v, "xz_scale")?)?;
                let y_scale = f64_of(field(v, "y_scale")?)?;
                let xz_factor = f64_of(field(v, "xz_factor")?)?;
                let y_factor = f64_of(field(v, "y_factor")?)?;
                let smear = f64_of(field(v, "smear_scale_multiplier")?)?;
                self.old_blended_noise(xz_scale, y_scale, xz_factor, y_factor, smear)
            }
            other => Err(format!("unsupported density function type {other}")),
        }
    }

    fn unary(&mut self, v: &Value, op: UnaryOp) -> BuildResult<u32> {
        let input = self.df_field(v, "input")?;
        Ok(self.push(Node::Unary(op, input)))
    }

    /// `BinaryFunction.compileSampler` specialisations: constant operands select the Const*
    /// samplers; `div` by a constant becomes multiply by `1/c`.
    fn binary(&mut self, v: &Value, ty: &str) -> BuildResult<u32> {
        let l = self.df_field(v, "left")?;
        let r = self.df_field(v, "right")?;
        let lc = self.const_of(l);
        let rc = self.const_of(r);
        Ok(match ty {
            "add" => match (lc, rc) {
                (Some(c), _) => self.push(Node::AddC(r, c)),
                (None, Some(c)) => self.push(Node::AddC(l, c)),
                _ => self.push(Node::Add(l, r)),
            },
            "sub" => match (lc, rc) {
                (Some(c), _) => self.push(Node::SubC(c, r)),
                (None, Some(c)) => self.push(Node::AddC(l, -c)),
                _ => self.push(Node::Sub(l, r)),
            },
            "mul" => match (lc, rc) {
                (Some(c), _) => self.push(Node::MulC(r, c)),
                (None, Some(c)) => self.push(Node::MulC(l, c)),
                _ => self.push(Node::Mul(l, r)),
            },
            "div" => match (lc, rc) {
                (Some(c), _) => self.push(Node::DivC(c, r)),
                (None, Some(c)) => self.push(Node::MulC(l, 1.0_f32 / c)),
                _ => self.push(Node::Div(l, r)),
            },
            "min" => match (lc, rc) {
                (Some(c), _) => self.push(Node::MinC(r, c)),
                (None, Some(c)) => self.push(Node::MinC(l, c)),
                _ => self.push(Node::Min(l, r, f32::NEG_INFINITY)),
            },
            "max" => match (lc, rc) {
                (Some(c), _) => self.push(Node::MaxC(r, c)),
                (None, Some(c)) => self.push(Node::MaxC(l, c)),
                _ => self.push(Node::Max(l, r, f32::INFINITY)),
            },
            _ => unreachable!(),
        })
    }

    fn optional_df(&mut self, v: &Value, key: &str) -> BuildResult<u32> {
        match v.get(key) {
            Some(inner) => self.df(inner),
            None => Ok(self.push(Node::Const(0.0))),
        }
    }

    /// `old_blended_noise` compiled as `Lerp(Clamp(main + 0.5, 0, 1), minLimit, maxLimit)`.
    fn old_blended_noise(
        &mut self,
        xz_scale: f64,
        y_scale: f64,
        xz_factor: f64,
        y_factor: f64,
        smear: f64,
    ) -> BuildResult<u32> {
        let mut rng = AnyRng::Xoro(self.random_from_hash(BLENDED_NOISE_SEED));
        let BlendedParts {
            min_limit,
            max_limit,
            main,
            xz_multiplier,
            y_multiplier,
            main_xz_scale,
            main_y_scale,
        } = crate::noise::BlendedNoise::new(&mut rng, xz_scale, y_scale, xz_factor, y_factor, smear)
            .parts();
        let min_idx = self.push_stack(min_limit);
        let max_idx = self.push_stack(max_limit);
        let main_idx = self.push_stack(main);
        let min_node = self.push(Node::NoiseSample {
            noise: min_idx,
            xz: xz_multiplier,
            y: y_multiplier,
        });
        let max_node = self.push(Node::NoiseSample {
            noise: max_idx,
            xz: xz_multiplier,
            y: y_multiplier,
        });
        let main_node = self.push(Node::NoiseSample {
            noise: main_idx,
            xz: main_xz_scale,
            y: main_y_scale,
        });
        let shifted = self.push(Node::AddC(main_node, 0.5));
        let choice = self.push(Node::Clamp(shifted, 0.0, 1.0));
        Ok(self.push(Node::Lerp(choice, min_node, max_node)))
    }

    fn push_stack(&mut self, stack: NoiseStack) -> usize {
        self.noises.push(stack);
        self.noises.len() - 1
    }

    /// `SplineFunction` coordinates and `CubicSpline` points. Coordinates are deduplicated by node.
    fn spline(&mut self, v: &Value, coords: &mut Vec<u32>) -> BuildResult<Sp> {
        if let Some(c) = v.as_f64() {
            return Ok(Sp::Constant(c as f32));
        }
        let coordinate = self.df_field(v, "coordinate")?;
        let slot = match coords.iter().position(|&c| c == coordinate) {
            Some(i) => i,
            None => {
                coords.push(coordinate);
                coords.len() - 1
            }
        };
        let points = field(v, "points")?
            .as_array()
            .ok_or("points must be an array")?;
        let mut locations = Vec::with_capacity(points.len());
        let mut values = Vec::with_capacity(points.len());
        let mut derivatives = Vec::with_capacity(points.len());
        for p in points {
            locations.push(f32_of(field(p, "location")?)?);
            values.push(self.spline(field(p, "value")?, coords)?);
            derivatives.push(f32_of(field(p, "derivative")?)?);
        }
        Ok(Sp::Multi {
            coord: slot,
            locations,
            values,
            derivatives,
        })
    }

    /// The `noise_router` entries of `noise_settings/overworld.json`, resolved to node ids.
    pub fn router_entries(&mut self) -> BuildResult<HashMap<String, u32>> {
        let settings = parse_json("noise_settings/overworld.json")?;
        let router = field(&settings, "noise_router")?
            .as_object()
            .ok_or("noise_router must be an object")?;
        let mut out = HashMap::new();
        for (key, value) in router {
            let name = value.as_str().ok_or("router entries are references")?;
            out.insert(key.clone(), self.reference(name)?);
        }
        Ok(out)
    }
}
