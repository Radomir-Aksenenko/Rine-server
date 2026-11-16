//! Player right click on a block, in the order of
//! `ServerPlayerGameMode.useItemOn`:
//!
//! 1. unless the player sneaks with something in hand:
//!    `BlockState.useItemOn(stack)` (block reacts to the held item:
//!    cauldron with a bucket, candle on a cake, flower into a pot);
//!    on `TryEmptyHand` and the main hand, `BlockState.useWithoutItem`
//!    (bell, cake, composter, respawn anchor, dragon egg);
//! 2. otherwise `ItemStack.useOn` (hoe, shovel, axe, honeycomb, bone meal,
//!    flint and steel). `Pass` here falls through to block placement.
//!
//! Families answer with a `Res`. Effects on the player (the held stack,
//! extra items, food) are returned in `UseOut` and applied by the caller
//! (`redstone::blocks_use`), so families stay testable on `testw::World`.

use super::{Beh, Rng};
use crate::game::redstone::engine::Lvl;
use crate::game::redstone::Rs;
use rine_protocol::BlockPos;

/// What the player used (`UseOnContext` / `BlockHitResult`).
#[derive(Clone, Copy, Debug)]
pub struct UseCtx {
    /// Held item id (0 = empty hand) and stack size.
    pub item: u32,
    pub count: i32,
    /// `minecraft:damage` of the held stack.
    pub damage: u16,
    pub main_hand: bool,
    pub creative: bool,
    pub sneaking: bool,
    /// Clicked face (protocol direction id) and cursor on it, 0..1.
    pub face: u8,
    pub cursor: (f32, f32, f32),
    pub yaw: f32,
    /// Player feet position.
    pub pos: [f64; 3],
    /// `Player.canEat(false)`: food bar not full (creative counts as yes).
    pub can_eat: bool,
    /// Item id in the other hand (offhand when using the main hand), 0 = empty.
    pub other: u32,
    /// The clicked sign's block entity, read by the caller (empty otherwise).
    pub sign: super::signs::SignSnap,
}

/// `InteractionResult` of a use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Res {
    /// Nothing happened; go on with the next step.
    Pass,
    /// `TRY_WITH_EMPTY_HAND`: block ignores the item, try `useWithoutItem`.
    TryEmptyHand,
    /// `FAIL`: stop, no item use, no placement.
    Fail,
    /// `SUCCESS` / `CONSUME`: done, with these effects on the player.
    Done(UseOut),
    /// `BedBlock.useWithoutItem` on a bed half: the game layer runs the player side (`game/sleep.rs`).
    Bed(super::bed::BedUse),
}

impl Res {
    pub fn done(out: UseOut) -> Self {
        Res::Done(out)
    }
    /// Done with no effect on the player (arm swing only).
    pub fn swing() -> Self {
        Res::Done(UseOut::default())
    }
}

/// Effects on the using player, applied after the block change.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UseOut {
    pub item: ItemOp,
    /// `Player.addItem`, dropped at the player when the inventory is full
    /// (flower back from a pot, honey bottle...). Item id and count.
    pub give: Option<(u32, i32)>,
    /// `FoodData.eat(nutrition, saturationModifier)` (cake slice).
    pub food: Option<(i32, f32)>,
    /// Block entity change of a sign (dye, glow, wax, open the editor).
    pub sign: Option<super::signs::SignOp>,
}

/// What happens to the held stack.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ItemOp {
    #[default]
    Keep,
    /// `stack.consume(n, player)`: shrink unless creative.
    Shrink(i32),
    /// `stack.hurtAndBreak(n, player, hand)`: wear in survival/adventure.
    Damage(i32),
    /// `ItemUtils.createFilledResult(stack, player, result)`: creative keeps
    /// the stack and gets `result` if it has none; otherwise shrink by one
    /// and the result replaces an emptied stack or goes to the inventory.
    Fill(u32),
    /// `player.setItemInHand(hand, new)` (empty hand takes the flower of a pot).
    Set(u32, i32),
}

/// Step 1a: `BlockState.useItemOn` with the held stack.
pub fn use_item_on<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, ctx: &UseCtx, rng: &mut Rng) -> Res {
    match super::beh(st) {
        Beh::Composter | Beh::FlowerPot | Beh::Cake | Beh::Candle | Beh::CandleCake | Beh::RespawnAnchor => super::uses::use_item_on(rs, l, st, pos, ctx, rng),
        super::Beh::Cauldron => super::cauldron::use_item_on(rs, l, st, pos, ctx, rng),
        Beh::Sign => super::signs::use_item_on(rs, l, st, pos, ctx, rng),
        _ => Res::TryEmptyHand,
    }
}

/// Step 1b: `BlockState.useWithoutItem` (main hand only).
pub fn use_without_item<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, ctx: &UseCtx, rng: &mut Rng) -> Res {
    match super::beh(st) {
        Beh::Bell | Beh::Composter | Beh::FlowerPot | Beh::Cake | Beh::CandleCake | Beh::RespawnAnchor => super::uses::use_without_item(rs, l, st, pos, ctx, rng),
        super::Beh::Bed => super::bed::use_without_item(|q| l.get(q), st, pos),
        super::Beh::DragonEgg => {
            super::falling::dragon_teleport(rs, l, st, pos, rng);
            Res::swing()
        }
        Beh::Sign => super::signs::use_without_item(rs, l, st, pos, ctx, rng),
        _ => Res::Pass,
    }
}

/// Step 2: `ItemStack.useOn`, routed by the held item (`tools::use_on`).
pub fn item_use_on<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, ctx: &UseCtx, rng: &mut Rng) -> Res {
    super::tools::use_on(rs, l, st, pos, ctx, rng)
}
