//! `BlockStateProvider` (`levelgen/feature/stateproviders/*`) and the `BlockPredicate`s its rule-based
//! form uses. Providers compile to palette ids; `get` / `optional` mirror `getState` / `getOptionalState`.

use crate::feature::placement::{sample_int, IntProvider};
use crate::feature::region::Region;
use crate::feature::rng::WorldgenRng;

use super::state::TreeStates;

/// `BlockPredicate` subset used by the tree providers.
#[derive(Debug, Clone)]
pub enum Predicate {
    /// `matching_block_tag`: `level.isStateAtPosition(pos, state -> state.is(tag))`.
    MatchingTag(u16),
    /// `matching_blocks`: the state at `pos + offset` is one of the listed blocks (by name).
    MatchingBlocks { names: Vec<String>, offset: (i32, i32, i32) },
    /// `not`.
    Not(Box<Predicate>),
}

impl Predicate {
    pub fn test(&self, region: &Region, states: &TreeStates, pos: (i32, i32, i32)) -> bool {
        match self {
            Predicate::MatchingTag(t) => region.tags.has(region.get_block(pos.0, pos.1, pos.2), *t),
            Predicate::MatchingBlocks { names, offset } => {
                let (x, y, z) = (pos.0 + offset.0, pos.1 + offset.1, pos.2 + offset.2);
                let id = region.get_block(x, y, z);
                let name = states.base_name(id);
                names.iter().any(|n| n == name)
            }
            Predicate::Not(p) => !p.test(region, states, pos),
        }
    }
}

/// A compiled `BlockStateProvider`.
#[derive(Debug, Clone)]
pub enum Provider {
    /// `minecraft:simple_state_provider` (and the inline `{id, properties}` form).
    Simple(u16),
    /// `minecraft:weighted_state_provider`: `(state, weight)` in list order.
    Weighted(Vec<(u16, i32)>),
    /// `minecraft:rule_based_state_provider`.
    RuleBased { rules: Vec<(Predicate, Provider)>, fallback: Option<Box<Provider>> },
    /// `minecraft:randomized_int_state_provider`.
    RandomizedInt { source: Box<Provider>, property: String, values: IntProvider },
}

impl Provider {
    /// `getOptionalState`: `None` when no rule applies.
    pub fn optional(&self, region: &Region, states: &TreeStates, rng: &mut WorldgenRng, pos: (i32, i32, i32)) -> Option<u16> {
        match self {
            Provider::Simple(id) => Some(*id),
            Provider::Weighted(_) | Provider::RandomizedInt { .. } => Some(self.get(region, states, rng, pos)),
            Provider::RuleBased { rules, fallback } => {
                for (pred, then) in rules {
                    if pred.test(region, states, pos) {
                        if let Some(s) = then.optional(region, states, rng, pos) {
                            return Some(s);
                        }
                    }
                }
                fallback.as_ref().and_then(|f| f.optional(region, states, rng, pos))
            }
        }
    }

    /// `getState`: rule-based providers without a match return the block already at `pos`.
    pub fn get(&self, region: &Region, states: &TreeStates, rng: &mut WorldgenRng, pos: (i32, i32, i32)) -> u16 {
        match self {
            Provider::Simple(id) => *id,
            Provider::Weighted(entries) => {
                let total: i32 = entries.iter().map(|e| e.1).sum();
                let mut sel = rng.next_int_bound(total);
                for (id, w) in entries {
                    if sel < *w {
                        return *id;
                    }
                    sel -= *w;
                }
                unreachable!("weighted selection inside total weight")
            }
            Provider::RuleBased { .. } => match self.optional(region, states, rng, pos) {
                Some(s) => s,
                None => region.get_block(pos.0, pos.1, pos.2),
            },
            Provider::RandomizedInt { source, property, values } => {
                let base = source.get(region, states, rng, pos);
                match states.get(base, property) {
                    Some(_) => {
                        let v = sample_int(values, rng);
                        states.with(base, property, &v.to_string())
                    }
                    None => base,
                }
            }
        }
    }
}
