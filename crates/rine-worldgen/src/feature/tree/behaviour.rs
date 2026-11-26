//! Block behaviour the tree code tests (`isSolidRender`, `isFaceSturdy(UP)`), from
//! `assets/worldgen/block_behaviour.tsv` (`tools/noise-golden` Java dump of the default states;
//! property-dependent shapes are not modelled).

use std::collections::HashMap;
use std::sync::OnceLock;

use super::order::Pos;
use super::Ctx;

const BEHAVIOUR: &str = include_str!("../../../assets/worldgen/block_behaviour.tsv");

/// `(isSolidRender, isFaceSturdy(UP))` per block name, default state.
/// `isFaceSturdy(UP)` of a block's default state, from the behaviour table (`None` for blocks it does not list).
fn behaviour() -> &'static HashMap<&'static str, (bool, bool)> {
    static MAP: OnceLock<HashMap<&'static str, (bool, bool)>> = OnceLock::new();
    MAP.get_or_init(|| {
        BEHAVIOUR
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| {
                let mut it = l.split('\t');
                let name = it.next().expect("name");
                let solid = it.next().expect("solid") == "true";
                let sturdy = it.next().expect("sturdy") == "true";
                (name, (solid, sturdy))
            })
            .collect()
    })
}

fn lookup(ctx: &Ctx, p: Pos) -> (bool, bool) {
    let id = ctx.id(p);
    let name = ctx.env.states.base_name(id);
    *behaviour().get(name).unwrap_or_else(|| panic!("no behaviour data for {name}"))
}

/// `BlockStateBase.isSolidRender` at `p`.
pub fn is_solid_render(ctx: &Ctx, p: Pos) -> bool {
    lookup(ctx, p).0
}

/// `isFaceSturdy(level, pos, UP)` at `p`.
pub fn is_face_sturdy_up(ctx: &Ctx, p: Pos) -> bool {
    lookup(ctx, p).1
}
