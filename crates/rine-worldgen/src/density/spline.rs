//! `CubicSpline` (multipoint and constant) with `SplineFunction` coordinate handling.
//! Each multipoint names a coordinate slot; slots are filled per point or per volume by the tree.

use crate::mth::{binary_search, lerp};

#[derive(Clone, Debug)]
pub enum Sp {
    Constant(f32),
    Multi {
        /// Index into the tree's coordinate slots.
        coord: usize,
        locations: Vec<f32>,
        values: Vec<Sp>,
        derivatives: Vec<f32>,
    },
}

/// `CubicSpline.Multipoint.linearExtend`.
#[inline]
fn linear_extend(input: f32, locations: &[f32], value: f32, derivatives: &[f32], index: usize) -> f32 {
    let derivative = derivatives[index];
    if derivative == 0.0 {
        value
    } else {
        value + derivative * (input - locations[index])
    }
}

/// Evaluate a spline. `coord(slot)` returns the coordinate function value for a slot.
pub fn eval(sp: &Sp, coord: &dyn Fn(usize) -> f32) -> f32 {
    match sp {
        Sp::Constant(v) => *v,
        Sp::Multi {
            coord: slot,
            locations,
            values,
            derivatives,
        } => {
            let input = coord(*slot);
            let start = binary_search(0, locations.len() as i32, |i| input < locations[i as usize]) - 1;
            let last = locations.len() - 1;
            if start < 0 {
                return linear_extend(input, locations, eval(&values[0], coord), derivatives, 0);
            }
            let start = start as usize;
            if start == last {
                return linear_extend(input, locations, eval(&values[last], coord), derivatives, last);
            }
            let x1 = locations[start];
            let x2 = locations[start + 1];
            let t = (input - x1) / (x2 - x1);
            let f1 = &values[start];
            let f2 = &values[start + 1];
            let d1 = derivatives[start];
            let d2 = derivatives[start + 1];
            let y1 = eval(f1, coord);
            let y2 = eval(f2, coord);
            let a = d1 * (x2 - x1) - (y2 - y1);
            let b = -d2 * (x2 - x1) + (y2 - y1);
            lerp(t, y1, y2) + t * (1.0 - t) * lerp(t, a, b)
        }
    }
}
