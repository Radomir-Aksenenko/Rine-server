# Brief for block-family workers (Rine, report 01)

You implement ONE family of vanilla Minecraft 26.3 block behaviour in the Rust server Rine.
Read `CLAUDE.md` and `docs/PHILOSOPHY.md` sections 1-3 first (short). Rules that matter most:

- Result must match vanilla 26.3 exactly: same states, same chances, same tick delays, same
  update flags, same order. Read the vanilla Java class COMPLETELY before writing code:
  `Paper-26.3/paper-server/src/minecraft/java/net/minecraft/world/level/block/<Class>.java`,
  plus parents and every method it calls. Lines marked `// Paper`, `// Spigot`, `// CraftBukkit`
  are NOT vanilla: Spigot "modifier" growth code like `nextFloat() < modifier/(100*(floor(25/s)+1))`
  equals vanilla `nextInt((int)(25/s)+1) == 0` with modifier 100. Bukkit events
  (`CraftEventFactory.handleBlockGrowEvent(level,pos,state,flags)`) mean just `level.setBlock(pos,state,flags)`.
- Hot path: no allocation per block, no string comparisons at run time, no `unwrap`/`panic`.
  Resolve block ids, tags and property value ids ONCE in a `OnceLock` table (see `crops.rs`).
- Code comments in English, short. Name the vanilla class/method a function ports.

## Where your code goes

- Your family file: `crates/rine-server/src/game/blocks/<family>.rs`. Model it on
  `crates/rine-server/src/game/blocks/crops.rs` (read it fully: it is the reference pattern).
- `crates/rine-server/src/game/blocks/mod.rs`: add `pub mod <family>;`, your `Beh` variants,
  a `<family>::classify` call in `classify()`, and arms in the dispatchers you use
  (`random_tick`, `update_shape`, `tick`, `on_place`, `neighbor_changed`, `placement`,
  `fluid_random_tick`, `precipitation`). Keep those edits minimal: other workers edit the same
  dispatchers in parallel and I merge.
- `props.rs`: if you need a property missing from `Pr`, append a variant at the END of `Pr` and
  its name at the END of `NAMES`, and bump `NPR`.
- Do NOT edit other families' files, `place.rs`, `neighbors.rs`, `interact.rs`, `engine.rs`
  unless your task says so. If you think you must, stop and say why in your report.

## API you have

- `Lvl` trait (`crates/rine-server/src/game/redstone/engine.rs`): `get`, `loaded`, `sky_light`,
  `block_light`, `sky_darken`, `raining`, `thundering`, `biome`, `height_motion_blocking`,
  `rules`, `pop_resource`, `drop_block`, `sound`, `level_event`, `time`, `day_time`.
- `engine::set_block(rs, l, pos, st, flags, UPDATE_LIMIT)` = vanilla `level.setBlock(pos, st, flags)`
  with all side effects (neighbor updates, shape updates, onPlace) in vanilla order.
  Flags in `engine::fl`: `ALL` (=3, `UPDATE_ALL`/`setBlockAndUpdate`), `CLIENTS` (=2), `NEIGHBORS`, `KNOWN_SHAPE`...
  `engine::destroy_block(rs, l, pos, drops, UPDATE_LIMIT)` = `level.destroyBlock`;
  `engine::remove_block(rs, l, pos, false)` = `level.removeBlock`;
  `engine::drop_resources(l, st, pos)` = `Block.dropResources`.
  `engine::schedule(rs, l, pos, block_id, delay, prio::NORMAL)` = `scheduleTick`;
  `engine::has_scheduled(rs, pos, block_id)`.
- `super::props::{geti, seti, getb, setb, getv, setv, vid, has, Pr}` for block state properties.
- `super::{block_of, block_id, default_state, block_tag, item_id, is_air, raw_brightness, local_brightness, rule_int, rule_bool, Rng}`.
  `raw_brightness` = `level.getRawBrightness(pos, 0)`; `local_brightness` = `getMaxLocalRawBrightness`.
- `crate::game::redstone::data` has direction constants (`DOWN UP NORTH SOUTH WEST EAST`,
  `HORIZONTAL`, `step(d)`, `opposite(d)`), sturdy-face tables (`sturdy(st, face)`, `sturdy_center`)
  and redstone properties (`data::get(st, data::P::Facing)`).
- Random: the `rng: &mut Rng` parameter in random ticks; `rs.rng` in scheduled ticks.
  `rng.next_int(n)`, `next_float()`, `next_double()`, `next_bool()`, `range(lo, hi)` (inclusive).

## Player right click (`blocks/interact.rs`)

Right clicks on blocks reach the families through `blocks/interact.rs`, in the order of
`ServerPlayerGameMode.useItemOn`. Read that file fully. You add arms to its three dispatchers:
`use_item_on` (block reacts to the held item, default `TryEmptyHand`), `use_without_item`
(empty-hand use, default `Pass`) and `item_use_on` (`ItemStack.useOn`, routed by the held item id;
`Pass` falls through to block placement). Return `Res::Fail`, `Res::Pass`, `Res::TryEmptyHand` or
`Res::Done(UseOut { item, give, food })`. You never touch the player inventory yourself: the
caller applies `ItemOp::{Shrink, Damage, Fill, Set}`, `give` and `food` (see the doc comments,
they mirror `stack.consume`, `hurtAndBreak`, `ItemUtils.createFilledResult`, `setItemInHand`,
`Player.addItem`, `FoodData.eat`). `UseCtx` carries the held item, creative, sneaking, face,
cursor, player position and `can_eat`. Sounds with the player as source: `l.sound(..., skip_actor=true)`
matches vanilla `level.playSound(player, ...)` (everyone but the player hears it).
Tests: call your functions directly on `testw::World` with a hand-built `UseCtx`.

## Start of work

Your worktree branch may start at `master`. First run `git merge --ff-only blocks` in your
worktree: the brief, `blocks/` and `interact.rs` live only on `blocks`.

## Tests and build

- Unit tests in your file, on `super::super::testw::World` (in-memory level: `put`, `light`,
  `block_light`, `raining`, `darken`, `popped`, `tick(&mut rs)` for scheduled ticks).
  Test the vanilla edge cases you wrote down, not just the happy path.
- Use a shared target dir to save compile time:
  `CARGO_TARGET_DIR=E:/Projects/Rine-server/target-agents cargo test -p rine-server blocks::`
  (Git Bash syntax; in PowerShell set `$env:CARGO_TARGET_DIR` first). Then run the whole
  `cargo test -p rine-server` once at the end: all tests must pass. No new warnings in your files.
- Port 25565 is the owner's live server: never start a server on it.
- Keep each file's existing line endings (most are LF; `interact.rs` and `survival.rs` are CRLF). Never rewrite a whole file with a tool that changes them.

## Finish

Commit on your worktree branch, only your files, message `feat(blocks): <family> ...` in
English, ending with the line `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
Report back: commit hash, what is implemented (vanilla class -> function), vanilla behaviour
you left out and why, anything in vanilla you were unsure about, test results.
