//! `FeatureSorter.buildFeaturesPerStep` (vanilla 26.3), ported with the same node identity, edge order
//! and depth-first post-order. A node is `(step, featureIndex)`; the feature index is assigned in order
//! of first appearance across the biome sources, exactly like `Reference2IntOpenHashMap` there.

use std::collections::{BTreeMap, BTreeSet, HashMap};

type Node = (usize, usize);

/// Returns, per step, the placed-feature indices in execution order (the `StepFeatureData.features()` list).
/// `sources[i][s]` is the list of placed-feature ids of source `i` in step `s`.
pub fn build_features_per_step(sources: &[Vec<Vec<usize>>]) -> Result<Vec<Vec<usize>>, String> {
    let mut feature_index: HashMap<usize, usize> = HashMap::new();
    let mut edges: BTreeMap<Node, BTreeSet<Node>> = BTreeMap::new();
    let mut node_feature: HashMap<usize, usize> = HashMap::new();
    let mut max_step = 0usize;

    for source in sources {
        let mut list: Vec<Node> = Vec::new();
        max_step = max_step.max(source.len());
        for (step, features) in source.iter().enumerate() {
            for &feature in features {
                let next = feature_index.len();
                let idx = *feature_index.entry(feature).or_insert(next);
                node_feature.insert(idx, feature);
                list.push((step, idx));
            }
        }
        for i in 0..list.len() {
            edges.entry(list[i]).or_default();
            if i + 1 < list.len() {
                edges.get_mut(&list[i]).unwrap().insert(list[i + 1]);
            }
        }
    }

    let mut discovered: BTreeSet<Node> = BTreeSet::new();
    let mut visiting: BTreeSet<Node> = BTreeSet::new();
    let mut sorted: Vec<Node> = Vec::new();
    let keys: Vec<Node> = edges.keys().copied().collect();
    for key in keys {
        if !visiting.is_empty() {
            return Err("DFS bork: in-progress vertex set not empty".into());
        }
        if !discovered.contains(&key) && dfs(&edges, &mut discovered, &mut visiting, &mut sorted, key) {
            return Err("Feature order cycle found".into());
        }
    }
    sorted.reverse();

    let mut out = Vec::with_capacity(max_step);
    for step in 0..max_step {
        out.push(
            sorted
                .iter()
                .filter(|n| n.0 == step)
                .map(|n| node_feature[&n.1])
                .collect(),
        );
    }
    Ok(out)
}

/// `Graph.depthFirstSearch`: returns true when a cycle is found.
fn dfs(
    edges: &BTreeMap<Node, BTreeSet<Node>>,
    discovered: &mut BTreeSet<Node>,
    visiting: &mut BTreeSet<Node>,
    out: &mut Vec<Node>,
    current: Node,
) -> bool {
    if discovered.contains(&current) {
        return false;
    }
    if visiting.contains(&current) {
        return true;
    }
    visiting.insert(current);
    if let Some(next_nodes) = edges.get(&current) {
        for &next in next_nodes {
            if dfs(edges, discovered, visiting, out, next) {
                return true;
            }
        }
    }
    visiting.remove(&current);
    discovered.insert(current);
    out.push(current);
    false
}
