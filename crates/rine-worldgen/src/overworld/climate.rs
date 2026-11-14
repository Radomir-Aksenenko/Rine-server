//! `Climate.ParameterList` / `Climate.RTree` (vanilla 26.3). The tree is built with the same sorts and
//! bucketing as `Climate.RTree.build`, so the traversal order (and therefore tie-breaking) is identical.
//! `find` is the pure form of `RTree.search`: the candidate is the leaf of the previous query in the
//! same thread, which the harness clears before every query (see `tools/noise-golden/WorldgenGolden.java`).

use std::cmp::Ordering;

pub const DIMS: usize = 7;
const CHILDREN_PER_NODE: usize = 19;

/// `(Parameter.min, Parameter.max)`, both quantized.
pub type Span = (i64, i64);
pub type Space = [Span; DIMS];

/// `Climate.quantizeCoord`: `(long)(coord * 10000.0F)`.
#[inline]
pub fn quantize(coord: f32) -> i64 {
    (coord * 10000.0f32) as i64
}

/// `Climate.TargetPoint.toParameterArray()`: six quantized climate values and a zero offset.
pub type Target = [i64; DIMS];

#[derive(Clone, Debug)]
enum Kind {
    Leaf(u16),
    Sub(Vec<u32>),
}

#[derive(Clone, Debug)]
struct Node {
    space: Space,
    kind: Kind,
}

#[derive(Clone, Debug)]
pub struct ParameterList {
    nodes: Vec<Node>,
    root: u32,
}

struct Builder {
    nodes: Vec<Node>,
}

fn center(span: Span, absolute: bool) -> i64 {
    let c = (span.0 + span.1) / 2;
    if absolute {
        c.abs()
    } else {
        c
    }
}

fn magnitude(space: &Space) -> i64 {
    space.iter().fold(0i64, |acc, s| acc.wrapping_add(center(*s, true)))
}

fn cost(space: &Space) -> i64 {
    space.iter().fold(0i64, |acc, s| acc.wrapping_add((s.1 - s.0).abs()))
}

fn param_distance(span: Span, target: i64) -> i64 {
    let above = target - span.1;
    let below = span.0 - target;
    if above > 0 {
        above
    } else {
        below.max(0)
    }
}

fn square(v: i64) -> i64 {
    v.wrapping_mul(v)
}

impl Builder {
    fn push(&mut self, space: Space, kind: Kind) -> u32 {
        self.nodes.push(Node { space, kind });
        (self.nodes.len() - 1) as u32
    }

    /// `new SubTree(children)`: the parameter space is the span of the children.
    fn sub(&mut self, children: Vec<u32>) -> u32 {
        let mut bounds: [Option<Span>; DIMS] = [None; DIMS];
        for &c in &children {
            let space = self.nodes[c as usize].space;
            for d in 0..DIMS {
                let p = space[d];
                bounds[d] = Some(match bounds[d] {
                    None => p,
                    Some(b) => (p.0.min(b.0), p.1.max(b.1)),
                });
            }
        }
        let mut space = [(0i64, 0i64); DIMS];
        for d in 0..DIMS {
            space[d] = bounds[d].expect("sub tree needs a child");
        }
        self.push(space, Kind::Sub(children))
    }

    fn sort_list(&self, list: &mut [u32], dimension: usize, absolute: bool) {
        list.sort_by(|&a, &b| {
            for d in 0..DIMS {
                let dim = (dimension + d) % DIMS;
                let ka = center(self.nodes[a as usize].space[dim], absolute);
                let kb = center(self.nodes[b as usize].space[dim], absolute);
                match ka.cmp(&kb) {
                    Ordering::Equal => {}
                    other => return other,
                }
            }
            Ordering::Equal
        });
    }

    fn bucketize(&mut self, nodes: &[u32]) -> Vec<u32> {
        let base = CHILDREN_PER_NODE as f64;
        let exponent = ((nodes.len() as f64 - 0.01).ln() / base.ln()).floor();
        let expected = base.powf(exponent) as usize;
        let mut buckets = Vec::new();
        let mut current: Vec<u32> = Vec::new();
        for &n in nodes {
            current.push(n);
            if current.len() >= expected {
                buckets.push(self.sub(std::mem::take(&mut current)));
            }
        }
        if !current.is_empty() {
            buckets.push(self.sub(current));
        }
        buckets
    }

    fn build(&mut self, mut children: Vec<u32>) -> u32 {
        if children.len() == 1 {
            return children[0];
        }
        if children.len() <= CHILDREN_PER_NODE {
            children.sort_by_key(|&c| magnitude(&self.nodes[c as usize].space));
            return self.sub(children);
        }
        let mut min_cost = i64::MAX;
        let mut min_dim = 0usize;
        let mut min_buckets: Vec<u32> = Vec::new();
        for d in 0..DIMS {
            self.sort_list(&mut children, d, false);
            let buckets = self.bucketize(&children);
            let mut total = 0i64;
            for &b in &buckets {
                total = total.wrapping_add(cost(&self.nodes[b as usize].space));
            }
            if min_cost > total {
                min_cost = total;
                min_dim = d;
                min_buckets = buckets;
            }
        }
        self.sort_list(&mut min_buckets, min_dim, true);
        let mut built = Vec::with_capacity(min_buckets.len());
        for &b in &min_buckets {
            let kids = match &self.nodes[b as usize].kind {
                Kind::Sub(k) => k.clone(),
                Kind::Leaf(_) => unreachable!("buckets are sub trees"),
            };
            built.push(self.build(kids));
        }
        self.sub(built)
    }
}

impl ParameterList {
    /// `Climate.RTree.create(values, 19)`. Leaves are created in list order.
    pub fn new(points: &[(Space, u16)]) -> Self {
        assert!(!points.is_empty(), "need at least one value");
        let mut b = Builder { nodes: Vec::new() };
        let leaves: Vec<u32> = points
            .iter()
            .map(|(space, value)| b.push(*space, Kind::Leaf(*value)))
            .collect();
        let root = b.build(leaves);
        ParameterList { nodes: b.nodes, root }
    }

    fn distance(&self, n: u32, target: &Target) -> i64 {
        let space = &self.nodes[n as usize].space;
        let mut d = 0i64;
        for i in 0..DIMS {
            d = d.wrapping_add(square(param_distance(space[i], target[i])));
        }
        d
    }

    /// `RTree.search` with the given candidate (the previous result of this thread, or `None`).
    fn search(&self, n: u32, target: &Target, candidate: Option<u32>) -> u32 {
        match &self.nodes[n as usize].kind {
            Kind::Leaf(_) => n,
            Kind::Sub(children) => {
                let mut min_distance = match candidate {
                    None => i64::MAX,
                    Some(c) => self.distance(c, target),
                };
                let mut closest = candidate;
                for &child in children {
                    let child_distance = self.distance(child, target);
                    if min_distance > child_distance {
                        let leaf = self.search(child, target, closest);
                        let leaf_distance = if leaf == child {
                            child_distance
                        } else {
                            self.distance(leaf, target)
                        };
                        if min_distance > leaf_distance {
                            min_distance = leaf_distance;
                            closest = Some(leaf);
                        }
                    }
                }
                closest.expect("search always finds a leaf")
            }
        }
    }

    /// `ParameterList.findValue` with an empty candidate: the biome index of the nearest leaf.
    pub fn find(&self, target: &Target) -> u16 {
        let leaf = self.search(self.root, target, None);
        match self.nodes[leaf as usize].kind {
            Kind::Leaf(v) => v,
            Kind::Sub(_) => unreachable!("search returns leaves"),
        }
    }
}
