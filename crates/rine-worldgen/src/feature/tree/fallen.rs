//! `FallenTreeFeature` (`minecraft:fallen_tree`).

use serde_json::Value;

use crate::feature::placement::{parse_int_provider, sample_int, IntProvider};

use super::behaviour::is_face_sturdy_up;
use super::decorator::{DecoratorCtx, TreeDecorator};
use super::order::{JavaSet, Pos};
use crate::feature::state_provider::StateProvider as Provider;
use super::{offset, relative, TreeCompile, Ctx, Dir, HORIZONTAL};

#[derive(Debug)]
pub struct FallenTree {
    pub trunk_provider: Provider,
    pub log_length: IntProvider,
    pub stump_decorators: Vec<TreeDecorator>,
    pub log_decorators: Vec<TreeDecorator>,
}

impl FallenTree {
    pub fn compile(cfg: &Value, c: &mut TreeCompile) -> Result<FallenTree, String> {
        let trunk_provider = c.provider(&cfg["trunk_provider"])?;
        let mut decs = |k: &str, c: &mut TreeCompile| -> Result<Vec<TreeDecorator>, String> {
            cfg[k].as_array().ok_or_else(|| format!("fallen {k}"))?.iter().map(|d| TreeDecorator::parse(d, c)).collect()
        };
        Ok(FallenTree {
            trunk_provider,
            log_length: parse_int_provider(&cfg["log_length"])?,
            stump_decorators: decs("stump_decorators", c)?,
            log_decorators: decs("log_decorators", c)?,
        })
    }

    /// `FallenTreeFeature.place` (always returns true, as vanilla does).
    pub fn place(&self, ctx: &mut Ctx, origin: Pos) -> bool {
        self.place_fallen_tree(ctx, origin);
        true
    }

    fn place_fallen_tree(&self, ctx: &mut Ctx, origin: Pos) {
        // placeStump
        let stump = self.place_log_block(ctx, origin, None);
        self.decorate_logs(ctx, &[stump], &self.stump_decorators);

        let direction = HORIZONTAL[ctx.rng.next_int_bound(4) as usize];
        let log_length = sample_int(&self.log_length, ctx.rng) - 2;
        let mut start = relative(origin, direction, 2 + ctx.rng.next_int_bound(2));
        // setGroundHeightForFallenLogStartPos
        start = offset(start, 0, 1, 0);
        for _ in 0..6 {
            if may_place_on(ctx, start) {
                break;
            }
            start = offset(start, 0, -1, 0);
        }
        if can_place_entire_log(ctx, log_length, start, direction) {
            self.place_fallen_log(ctx, log_length, start, direction);
        }
    }

    fn place_log_block(&self, ctx: &mut Ctx, pos: Pos, sideways: Option<Dir>) -> Pos {
        let mut id = ctx.prov(&self.trunk_provider, pos);
        if let Some(d) = sideways {
            if ctx.env.states.get(id, "axis").is_some() {
                let axis = match d {
                    Dir::North | Dir::South => "z",
                    Dir::East | Dir::West => "x",
                    Dir::Up | Dir::Down => "y",
                };
                id = ctx.env.states.with(id, "axis", axis);
            }
        }
        ctx.set(pos, id);
        pos
    }

    fn place_fallen_log(&self, ctx: &mut Ctx, log_length: i32, start: Pos, direction: Dir) {
        let mut log = JavaSet::new();
        let mut pos = start;
        for _ in 0..log_length {
            let p = self.place_log_block(ctx, pos, Some(direction));
            log.insert(p);
            pos = relative(pos, direction, 1);
        }
        let logs = log.to_vec();
        self.decorate_logs(ctx, &logs, &self.log_decorators);
    }

    fn decorate_logs(&self, ctx: &mut Ctx, logs: &[Pos], decorators: &[TreeDecorator]) {
        if decorators.is_empty() {
            return;
        }
        let mut d = DecoratorCtx::new(Vec::new(), logs.to_vec(), Vec::new(), false);
        for dec in decorators {
            dec.place(&mut d, ctx);
        }
    }
}

/// `mayPlaceOn`: `validTreePos && isOverSolidGround`.
fn may_place_on(ctx: &Ctx, p: Pos) -> bool {
    ctx.valid_tree_pos(p) && is_over_solid_ground(ctx, p)
}

/// `isOverSolidGround`: the block below has a sturdy top face.
fn is_over_solid_ground(ctx: &Ctx, p: Pos) -> bool {
    is_face_sturdy_up(ctx, offset(p, 0, -1, 0))
}

/// `canPlaceEntireFallenLog`. Leaves the start position unchanged.
fn can_place_entire_log(ctx: &Ctx, log_length: i32, start: Pos, direction: Dir) -> bool {
    let mut gap = 0;
    let mut pos = start;
    for _ in 0..log_length {
        if !ctx.valid_tree_pos(pos) {
            return false;
        }
        if !is_over_solid_ground(ctx, pos) {
            gap += 1;
            if gap > 2 {
                return false;
            }
        } else {
            gap = 0;
        }
        pos = relative(pos, direction, 1);
    }
    true
}
