//! Cauldrons: `AbstractCauldronBlock.useItemOn` with the four dispatchers of
//! `CauldronInteractions` (EMPTY, WATER, LAVA, POWDER_SNOW), bucket
//! interactions only.
//!
//! Not ported: glass bottles and potions, dyed armour, banners and shulker
//! boxes (they need item components), and the game events and statistics of
//! every interaction. Those items return `TryEmptyHand`.
//!
//! Known gap: vanilla `CONSUME` (lava or powder snow bucket with water above)
//! does not swing the arm; `Res` has no such variant, so it returns a plain
//! `Done` and the arm swings.
//!
//! Drips from stalactites are `misc.rs` (`cauldron_tick`), rain and snow are
//! `snow.rs` (`handle_precipitation`); both act on the same `Beh::Cauldron`.

use super::interact::{ItemOp, Res, UseCtx, UseOut};
use super::props::{geti, seti, Pr};
use super::{block_id, block_of, default_state, item_id, Beh, Rng};
use crate::game::redstone::engine::{self, fl, Lvl, CAT_BLOCKS, UPDATE_LIMIT};
use crate::game::redstone::Rs;
use rine_data::blocks::flag;
use rine_protocol::BlockPos;
use std::sync::OnceLock;

struct T {
    cauldron: u16,
    water_cauldron: u16,
    lava_cauldron: u16,
    powder_cauldron: u16,
    /// `Blocks.CAULDRON.defaultBlockState()`.
    empty: u16,
    /// Water cauldron at level 3 (`WATER_CAULDRON` with `LEVEL` 3).
    water_full: u16,
    /// `Blocks.LAVA_CAULDRON.defaultBlockState()`.
    lava: u16,
    /// Powder snow cauldron at level 3.
    powder_full: u16,
    bucket: u32,
    water_bucket: u32,
    lava_bucket: u32,
    powder_bucket: u32,
}

fn t() -> &'static T {
    static T_: OnceLock<T> = OnceLock::new();
    T_.get_or_init(|| T {
        cauldron: block_id("cauldron"),
        water_cauldron: block_id("water_cauldron"),
        lava_cauldron: block_id("lava_cauldron"),
        powder_cauldron: block_id("powder_snow_cauldron"),
        empty: default_state("cauldron"),
        water_full: seti(default_state("water_cauldron"), Pr::Level, 3),
        lava: default_state("lava_cauldron"),
        powder_full: seti(default_state("powder_snow_cauldron"), Pr::Level, 3),
        bucket: item_id("bucket"),
        water_bucket: item_id("water_bucket"),
        lava_bucket: item_id("lava_bucket"),
        powder_bucket: item_id("powder_snow_bucket"),
    })
}

pub fn classify(n: &str) -> Option<Beh> {
    Some(match n {
        "cauldron" | "water_cauldron" | "lava_cauldron" | "powder_snow_cauldron" => Beh::Cauldron,
        _ => return None,
    })
}

/// Which `CauldronInteractions` dispatcher a cauldron block uses.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Disp {
    Empty,
    Water,
    Lava,
    Powder,
}

fn disp_of(blk: u16) -> Option<Disp> {
    let t = t();
    Some(if blk == t.cauldron {
        Disp::Empty
    } else if blk == t.water_cauldron {
        Disp::Water
    } else if blk == t.lava_cauldron {
        Disp::Lava
    } else if blk == t.powder_cauldron {
        Disp::Powder
    } else {
        return None;
    })
}

#[derive(Clone, Copy)]
enum Act {
    /// `CauldronInteraction.fillBucket`: an empty bucket takes the content of
    /// the cauldron. `need_full`: only a level 3 cauldron gives its content.
    Fill { need_full: bool, item: u32, sound: &'static str },
    /// `CauldronInteractions.emptyBucket`: the bucket's content goes in.
    /// `under_water`: lava and powder snow do nothing with water above.
    Empty { state: u16, sound: &'static str, under_water: bool },
}

/// The dispatcher entry for the held item (`Dispatcher.get`, items only).
fn act(d: Disp, item: u32) -> Option<Act> {
    let t = t();
    if item == t.bucket {
        return match d {
            Disp::Water => Some(Act::Fill { need_full: true, item: t.water_bucket, sound: "item.bucket.fill" }),
            Disp::Lava => Some(Act::Fill { need_full: false, item: t.lava_bucket, sound: "item.bucket.fill_lava" }),
            Disp::Powder => Some(Act::Fill { need_full: true, item: t.powder_bucket, sound: "item.bucket.fill_powder_snow" }),
            Disp::Empty => None,
        };
    }
    // `addDefaultInteractions`, on every dispatcher
    if item == t.lava_bucket {
        Some(Act::Empty { state: t.lava, sound: "item.bucket.empty_lava", under_water: true })
    } else if item == t.water_bucket {
        Some(Act::Empty { state: t.water_full, sound: "item.bucket.empty", under_water: false })
    } else if item == t.powder_bucket {
        Some(Act::Empty { state: t.powder_full, sound: "item.bucket.empty_powder_snow", under_water: true })
    } else {
        None
    }
}

/// `CauldronInteractions.isUnderWater`: water in the block above.
fn water_above<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    rine_data::get().blocks.has(l.get(pos.offset(0, 1, 0)), flag::WATER)
}

/// `level.setBlockAndUpdate` through `CraftEventFactory.handleCauldronLevelChangeEvent`.
fn set_all<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    engine::set_block(rs, l, pos, st, fl::ALL, UPDATE_LIMIT);
}

/// Step 1a (`useItemOn`) of a cauldron with a bucket in hand.
pub fn use_item_on<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, ctx: &UseCtx, _rng: &mut Rng) -> Res {
    let t = t();
    let Some(d) = disp_of(block_of(st)) else { return Res::TryEmptyHand };
    let Some(a) = act(d, ctx.item) else { return Res::TryEmptyHand };
    match a {
        Act::Fill { need_full, item, sound } => {
            if need_full && geti(st, Pr::Level) != 3 {
                return Res::TryEmptyHand;
            }
            // fillBucket: the cauldron empties, the bucket is filled (ItemOp::Fill)
            set_all(rs, l, pos, t.empty);
            l.sound(sound, pos, 1.0, 1.0, CAT_BLOCKS, false);
            Res::Done(UseOut { item: ItemOp::Fill(item), ..UseOut::default() })
        }
        Act::Empty { state, sound, under_water } => {
            // vanilla CONSUME: no change, no item change (see module docs for the swing)
            if under_water && water_above(l, pos) {
                return Res::Done(UseOut::default());
            }
            // emptyBucket never checks the current level: a full water cauldron
            // takes a water bucket again and only the sound and the item change
            set_all(rs, l, pos, state);
            l.sound(sound, pos, 1.0, 1.0, CAT_BLOCKS, false);
            Res::Done(UseOut { item: ItemOp::Fill(t.bucket), ..UseOut::default() })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::interact::ItemOp;
    use super::super::testw::World as TW;
    use super::*;

    fn s(n: &str) -> u16 {
        default_state(n)
    }

    fn p(x: i32, y: i32, z: i32) -> BlockPos {
        BlockPos::new(x, y, z)
    }

    fn ctx(item: &str, creative: bool) -> UseCtx {
        UseCtx {
            item: item_id(item),
            count: 1,
            damage: 0,
            main_hand: true,
            creative,
            sneaking: false,
            face: 1,
            cursor: (0.5, 1.0, 0.5),
            yaw: 0.0,
            pos: [0.0; 3],
            can_eat: false,
            other: 0,
            sign: Default::default(),
        }
    }

    fn use_on(w: &mut TW, pos: BlockPos, item: &str, creative: bool) -> Res {
        let mut rs = Rs::default();
        let mut rng = Rng::new(1);
        let st = w.get(pos);
        use_item_on(&mut rs, w, st, pos, &ctx(item, creative), &mut rng)
    }

    fn level(w: &TW, pos: BlockPos) -> i32 {
        geti(w.get(pos), Pr::Level)
    }

    #[test]
    fn water_bucket_fills_empty_cauldron_to_three() {
        let pos = p(0, 64, 0);
        let mut w = TW::new();
        w.put(pos, s("cauldron"));
        let r = use_on(&mut w, pos, "water_bucket", false);
        assert_eq!(block_of(w.get(pos)), t().water_cauldron);
        assert_eq!(level(&w, pos), 3);
        assert_eq!(r, Res::Done(UseOut { item: ItemOp::Fill(item_id("bucket")), ..UseOut::default() }));
        assert_eq!(w.sounds.last().map(|x| x.0.as_str()), Some("item.bucket.empty"));
    }

    #[test]
    fn empty_bucket_takes_only_a_full_water_cauldron() {
        let pos = p(0, 64, 0);
        let mut w = TW::new();
        w.put(pos, seti_level("water_cauldron", 2));
        assert_eq!(use_on(&mut w, pos, "bucket", false), Res::TryEmptyHand);
        assert_eq!(level(&w, pos), 2);

        w.put(pos, seti_level("water_cauldron", 3));
        let r = use_on(&mut w, pos, "bucket", false);
        assert_eq!(block_of(w.get(pos)), t().cauldron);
        assert_eq!(r, Res::Done(UseOut { item: ItemOp::Fill(item_id("water_bucket")), ..UseOut::default() }));
        assert_eq!(w.sounds.last().map(|x| x.0.as_str()), Some("item.bucket.fill"));
    }

    #[test]
    fn creative_bucket_still_changes_the_cauldron() {
        // the stack is kept in creative by the caller's ItemOp::Fill handling;
        // the block result does not depend on the game mode
        let pos = p(0, 64, 0);
        let mut w = TW::new();
        w.put(pos, seti_level("water_cauldron", 3));
        let r = use_on(&mut w, pos, "bucket", true);
        assert_eq!(block_of(w.get(pos)), t().cauldron);
        assert!(matches!(r, Res::Done(UseOut { item: ItemOp::Fill(_), .. })));
    }

    #[test]
    fn lava_bucket_on_water_cauldron_replaces_it() {
        let pos = p(0, 64, 0);
        let mut w = TW::new();
        w.put(pos, seti_level("water_cauldron", 1));
        use_on(&mut w, pos, "lava_bucket", false);
        assert_eq!(block_of(w.get(pos)), t().lava_cauldron);
    }

    #[test]
    fn lava_bucket_with_water_above_does_nothing() {
        let pos = p(0, 64, 0);
        let mut w = TW::new();
        w.put(pos, s("cauldron"));
        w.put(p(0, 65, 0), s("water"));
        let r = use_on(&mut w, pos, "lava_bucket", false);
        assert_eq!(block_of(w.get(pos)), t().cauldron);
        assert_eq!(r, Res::Done(UseOut::default()));
        assert!(w.sounds.is_empty());
    }

    #[test]
    fn lava_cauldron_gives_lava_and_powder_snow_cauldron_gives_powder() {
        let pos = p(0, 64, 0);
        let mut w = TW::new();
        w.put(pos, s("lava_cauldron"));
        let r = use_on(&mut w, pos, "bucket", false);
        assert_eq!(block_of(w.get(pos)), t().cauldron);
        assert_eq!(r, Res::Done(UseOut { item: ItemOp::Fill(item_id("lava_bucket")), ..UseOut::default() }));

        // a level 2 powder cauldron refuses the empty bucket
        w.put(pos, seti_level("powder_snow_cauldron", 2));
        assert_eq!(use_on(&mut w, pos, "bucket", false), Res::TryEmptyHand);
        w.put(pos, seti_level("powder_snow_cauldron", 3));
        use_on(&mut w, pos, "bucket", false);
        assert_eq!(block_of(w.get(pos)), t().cauldron);
    }

    #[test]
    fn unlisted_items_fall_through_to_empty_hand() {
        let pos = p(0, 64, 0);
        let mut w = TW::new();
        w.put(pos, s("cauldron"));
        assert_eq!(use_on(&mut w, pos, "stone", false), Res::TryEmptyHand);
        assert_eq!(block_of(w.get(pos)), t().cauldron);
    }

    fn seti_level(name: &str, lvl: i32) -> u16 {
        seti(s(name), Pr::Level, lvl)
    }
}
