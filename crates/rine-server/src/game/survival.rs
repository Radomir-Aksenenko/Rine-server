//! Survival stats: hunger, saturation, exhaustion, eating, regeneration,
//! starvation, drowning, void and lava damage.
//!
//! Numbers follow vanilla 26.3 `FoodData`, `FoodConstants` and the
//! `causeFoodExhaustion` call sites in `Player` / `ServerPlayer`.
//!
//! Per-player state lives in a thread-local map. The game loop runs on one
//! thread, and every entry is keyed by slot and checked against the eid, so a
//! reused slot never inherits the previous player's state.

use super::interact;
use crate::stack::Stack;
use super::Game;
use crate::out;
use ahash::AHashMap;
use std::cell::RefCell;

const MAX_FOOD: i32 = 20;
const MAX_HEALTH: f32 = 20.0;
/// `FoodConstants.EXHAUSTION_DROP`: exhaustion spent per saturation/food point.
const EXHAUSTION_DROP: f32 = 4.0;
/// `FoodData.addExhaustion` cap.
const EXHAUSTION_CAP: f32 = 40.0;
/// `FoodConstants.EXHAUSTION_HEAL`: exhaustion caused by an unsaturated regen tick.
const EXHAUSTION_HEAL: f32 = 6.0;
const EXHAUSTION_JUMP: f32 = 0.05;
const EXHAUSTION_SPRINT_JUMP: f32 = 0.2;
/// Per metre, horizontal.
const EXHAUSTION_SPRINT: f32 = 0.1;
/// Per metre, swimming or underwater.
const EXHAUSTION_SWIM: f32 = 0.01;
/// Exhaustion for a melee hit landed by the player.
pub const EXHAUSTION_ATTACK: f32 = 0.1;
/// Exhaustion for damage taken.
pub const EXHAUSTION_DAMAGE: f32 = 0.1;
/// Exhaustion for breaking a block.
pub const EXHAUSTION_MINE: f32 = 0.005;

const HEAL_TICKS_SATURATED: u32 = 10;
const HEAL_TICKS: u32 = 80;
const HEAL_LEVEL: i32 = 18;
const STARVE_TICKS: u32 = 80;

const MAX_AIR: i32 = 300;
const AIR_RECOVER: i32 = 4;
const DROWN_AT: i32 = -20;
const DROWN_DAMAGE: f32 = 2.0;
const ENV_DAMAGE: f32 = 4.0;
/// Environmental damage repeats every 10 ticks while the player stays exposed.
const ENV_PERIOD: u32 = 10;
const VOID_Y: f64 = -128.0;
/// Position jumps larger than this are teleports, not movement.
const TELEPORT_DIST: f64 = 8.0;

const LIVING_FLAGS_INDEX: u8 = 8;
const AIR_INDEX: u8 = 1;
const LIVING_FLAG_HAND_ACTIVE: u8 = 0x01;
const LIVING_FLAG_OFF_HAND: u8 = 0x02;
const ENTITY_EVENT_USE_COMPLETE: i8 = 9;

/// Sound categories (`SoundSource` ordinals).
pub const SOUND_NEUTRAL: i32 = 6;
const SOUND_PLAYERS: i32 = 7;
const EAT_SOUND: &str = "minecraft:entity.generic.eat";
const BURP_SOUND: &str = "minecraft:entity.player.burp";
const SMALL_FALL: &str = "minecraft:entity.player.small_fall";
const BIG_FALL: &str = "minecraft:entity.player.big_fall";
const HONEY_SLIDE: &str = "minecraft:block.honey_block.slide";
/// `Attributes.SAFE_FALL_DISTANCE` default for players.
const SAFE_FALL_DISTANCE: f64 = 3.0;

/// Hunger state a player carries besides the fields on `Player`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub health: f32,
    pub food: i32,
    pub saturation: f32,
    pub exhaustion: f32,
    pub timer: u32,
}

/// Side effects `food_tick` asks the caller to apply.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HungerOut {
    /// One point of starvation damage (NORMAL difficulty, stops at 1 HP).
    pub starve: bool,
}

pub fn add_exhaustion(s: &mut Stats, amount: f32) {
    s.exhaustion = (s.exhaustion + amount).min(EXHAUSTION_CAP);
}

/// One tick of vanilla `FoodData.tick` for NORMAL difficulty.
pub fn food_tick(s: &mut Stats) -> HungerOut {
    let mut out = HungerOut::default();
    if s.exhaustion > EXHAUSTION_DROP {
        s.exhaustion -= EXHAUSTION_DROP;
        if s.saturation > 0.0 {
            s.saturation = (s.saturation - 1.0).max(0.0);
        } else {
            s.food = (s.food - 1).max(0);
        }
    }
    let hurt = s.health < MAX_HEALTH;
    if hurt && s.food >= MAX_FOOD && s.saturation > 0.0 {
        s.timer += 1;
        if s.timer >= HEAL_TICKS_SATURATED {
            let spent = s.saturation.min(6.0);
            s.health = (s.health + spent / 6.0).min(MAX_HEALTH);
            add_exhaustion(s, spent);
            s.timer = 0;
        }
    } else if hurt && s.food >= HEAL_LEVEL {
        s.timer += 1;
        if s.timer >= HEAL_TICKS {
            s.health = (s.health + 1.0).min(MAX_HEALTH);
            add_exhaustion(s, EXHAUSTION_HEAL);
            s.timer = 0;
        }
    } else if s.food <= 0 {
        s.timer += 1;
        if s.timer >= STARVE_TICKS {
            out.starve = s.health > 1.0;
            s.timer = 0;
        }
    } else {
        s.timer = 0;
    }
    out
}

/// Vanilla `FoodData.add`: food is clamped to 0..=20, saturation to 0..=food.
pub fn apply_food(s: &mut Stats, nutrition: i32, saturation: f32) {
    s.food = (s.food + nutrition).clamp(0, MAX_FOOD);
    s.saturation = (s.saturation + saturation).clamp(0.0, s.food as f32);
}

/// Exhaustion caused by moving this tick. `horiz` and `dist` are in blocks.
pub fn move_exhaustion(sprinting: bool, eye_in_water: bool, feet_in_water: bool, horiz: f64, dist: f64) -> f32 {
    if eye_in_water {
        EXHAUSTION_SWIM * dist as f32
    } else if feet_in_water {
        EXHAUSTION_SWIM * horiz as f32
    } else if sprinting {
        EXHAUSTION_SPRINT * horiz as f32
    } else {
        0.0
    }
}

pub fn jump_exhaustion(sprinting: bool) -> f32 {
    if sprinting {
        EXHAUSTION_SPRINT_JUMP
    } else {
        EXHAUSTION_JUMP
    }
}

/// Air supply step. Returns true when the player takes drowning damage.
pub fn air_tick(air: &mut i32, eye_in_water: bool) -> bool {
    if eye_in_water {
        *air -= 1;
        if *air <= DROWN_AT {
            *air = 0;
            return true;
        }
    } else if *air < MAX_AIR {
        *air = (*air + AIR_RECOVER).min(MAX_AIR);
    }
    false
}

/// Exposure damage (void, lava). Hits on the first exposed tick, then every
/// `ENV_PERIOD` ticks, and the cooldown resets once the player is out.
fn env_tick(cooldown: &mut u32, exposed: bool) -> bool {
    if !exposed {
        *cooldown = 0;
        return false;
    }
    if *cooldown == 0 {
        *cooldown = ENV_PERIOD - 1;
        true
    } else {
        *cooldown -= 1;
        false
    }
}

#[derive(Clone, Copy, Debug)]
struct FoodProps {
    nutrition: i32,
    saturation: f32,
    always: bool,
}

/// Food data from the generated items table. `None` for non-food items.
fn food_of(item: u32) -> Option<FoodProps> {
    let v = rine_data::get().items.get(item)?.food.as_ref()?;
    let nutrition = v.get("nutrition")?.as_i64()? as i32;
    let saturation = v.get("saturation")?.as_f64()? as f32;
    let always = v.get("canAlwaysEat").and_then(|b| b.as_bool()).unwrap_or(false);
    Some(FoodProps { nutrition, saturation, always })
}

fn eat_ticks(item: u32) -> u32 {
    match rine_data::get().items.get(item) {
        Some(i) if i.name == "minecraft:dried_kelp" => 16,
        _ => 32,
    }
}

#[derive(Clone, Copy, Debug)]
struct Eating {
    inv_slot: usize,
    item: u32,
    off_hand: bool,
    start: u64,
    ticks: u32,
}

#[derive(Debug)]
struct PlayerSurvival {
    eid: i32,
    eating: Option<Eating>,
    exhaustion: f32,
    timer: u32,
    last_pos: Option<[f64; 3]>,
    prev_on_ground: bool,
    air: i32,
    air_sent: i32,
    env_cooldown: u32,
}

impl PlayerSurvival {
    fn new(eid: i32) -> Self {
        Self {
            eid,
            eating: None,
            exhaustion: 0.0,
            timer: 0,
            last_pos: None,
            prev_on_ground: false,
            air: MAX_AIR,
            air_sent: MAX_AIR,
            env_cooldown: 0,
        }
    }

    /// Called while dead. `air_sent` is kept so the client gets the new air
    /// value after respawn.
    fn reset(&mut self) {
        self.eating = None;
        self.exhaustion = 0.0;
        self.timer = 0;
        self.last_pos = None;
        self.prev_on_ground = false;
        self.air = MAX_AIR;
        self.env_cooldown = 0;
    }
}

thread_local! {
    static STATE: RefCell<AHashMap<usize, PlayerSurvival>> = RefCell::new(AHashMap::new());
}

fn take_state(slot: usize, eid: i32) -> PlayerSurvival {
    STATE.with(|m| match m.borrow_mut().remove(&slot) {
        Some(s) if s.eid == eid => s,
        _ => PlayerSurvival::new(eid),
    })
}

fn put_state(slot: usize, st: PlayerSurvival) {
    STATE.with(|m| {
        m.borrow_mut().insert(slot, st);
    });
}

fn prune(g: &Game) {
    STATE.with(|m| {
        m.borrow_mut().retain(|slot, s| g.players.get(*slot).and_then(|p| p.as_ref()).is_some_and(|p| p.eid == s.eid));
    });
}

/// Block facts around the player, read before any mutable borrow of `Game`.
#[derive(Clone, Copy, Debug, Default)]
struct Probe {
    eye_in_water: bool,
    feet_in_water: bool,
    feet_in_lava: bool,
}

impl Probe {
    fn at(g: &Game, pos: [f64; 3], eye: [f64; 3]) -> Self {
        let b = &rine_data::get().blocks;
        let has = |p: [f64; 3], flag: u16| {
            let y = p[1].floor() as i32;
            if !(rine_world::MIN_Y..=rine_world::MAX_Y).contains(&y) {
                return false;
            }
            b.has(g.world.get_block(p[0].floor() as i32, y, p[2].floor() as i32), flag)
        };
        Self {
            eye_in_water: has(eye, rine_data::blocks::flag::WATER),
            feet_in_water: has(pos, rine_data::blocks::flag::WATER),
            feet_in_lava: has(pos, rine_data::blocks::flag::LAVA),
        }
    }
}

fn send_meta(g: &mut Game, slot: usize, frame: bytes::Bytes, to_trackers: bool) {
    let Some(p) = g.player(slot) else { return };
    let eid = p.eid;
    p.out.push(frame.clone());
    if to_trackers {
        super::track::send_to_trackers(g, eid, &frame);
    }
}

/// Living entity flags (index 8): hand active and off-hand bits drive the eating pose.
fn set_living_flags(g: &mut Game, slot: usize, flags: u8) {
    let Some(eid) = g.player(slot).map(|p| p.eid) else { return };
    let f = out::frame(out::Meta::new(eid).byte(LIVING_FLAGS_INDEX, flags).finish());
    send_meta(g, slot, f, true);
}

/// Serverbound use_item (right click in air).
pub fn use_item(g: &mut Game, slot: usize, hand: i32, seq: i32) {
    let now = g.tick;
    if super::fluids::use_bucket(g, slot, hand) {
        if let Some(p) = g.player(slot) {
            p.ack(seq);
        }
        return;
    }
    let Some(p) = g.player(slot) else { return };
    p.ack(seq);
    if p.dead || p.gamemode == 3 {
        return;
    }
    let off_hand = hand == 1;
    let inv_slot = if off_hand { 45 } else { 36 + p.held as usize };
    let stack = p.inv.slots[inv_slot];
    if stack.is_empty() {
        return;
    }
    let Some(food) = food_of(stack.item) else { return };
    // vanilla canEat: hungry, or the item can always be eaten
    if p.food >= MAX_FOOD && !food.always {
        return;
    }
    let eid = p.eid;
    let eating = Eating { inv_slot, item: stack.item, off_hand, start: now, ticks: eat_ticks(stack.item) };
    let mut st = take_state(slot, eid);
    st.eating = Some(eating);
    put_state(slot, st);
    let flags = LIVING_FLAG_HAND_ACTIVE | if off_hand { LIVING_FLAG_OFF_HAND } else { 0 };
    set_living_flags(g, slot, flags);
}

/// player_action RELEASE_USE_ITEM (6): cancels the current meal.
pub fn release_use_item(g: &mut Game, slot: usize) {
    let Some(eid) = g.players.get(slot).and_then(|p| p.as_ref()).map(|p| p.eid) else { return };
    let mut st = take_state(slot, eid);
    let was_eating = st.eating.take().is_some();
    put_state(slot, st);
    if was_eating {
        set_living_flags(g, slot, 0);
    }
}

/// Exhaustion of the player in `slot` (0 when no state exists for `eid`).
pub fn exhaustion_of(slot: usize, eid: i32) -> f32 {
    STATE.with(|m| m.borrow().get(&slot).filter(|s| s.eid == eid).map_or(0.0, |s| s.exhaustion))
}

/// Restores saved exhaustion for a player who just joined.
pub fn set_exhaustion(slot: usize, eid: i32, value: f32) {
    let mut st = take_state(slot, eid);
    st.exhaustion = value.clamp(0.0, EXHAUSTION_CAP);
    put_state(slot, st);
}

/// Adds exhaustion for an action taken by `slot` (attack, mining, damage taken).
/// Ignored for creative, spectator and dead players, as in vanilla.
pub fn exhaust(g: &mut Game, slot: usize, amount: f32) {
    let Some(p) = g.players.get(slot).and_then(|p| p.as_ref()) else { return };
    if p.dead || !(p.gamemode == 0 || p.gamemode == 2) {
        return;
    }
    let eid = p.eid;
    let mut st = take_state(slot, eid);
    st.exhaustion = (st.exhaustion + amount).min(EXHAUSTION_CAP);
    put_state(slot, st);
}

/// Called once per tick.
pub fn tick(g: &mut Game) {
    prune(&*g);
    for slot in 0..g.players.len() {
        let Some(eid) = g.players[slot].as_ref().map(|p| p.eid) else { continue };
        // Player.tick (attack strength recovers) and ServerPlayer.tick (the hit window counts down)
        if let Some(p) = g.players[slot].as_mut() {
            p.attack_ticks = p.attack_ticks.saturating_add(1);
            p.hurt_cooldown = (p.hurt_cooldown - 1).max(0);
        }
        let mut st = take_state(slot, eid);
        tick_player(g, slot, &mut st);
        put_state(slot, st);
    }
}

fn tick_player(g: &mut Game, slot: usize, st: &mut PlayerSurvival) {
    let now = g.tick;
    let Some(p) = g.players.get(slot).and_then(|p| p.as_ref()) else { return };
    if p.dead {
        st.reset();
        return;
    }
    let eid = p.eid;
    let gm = p.gamemode;
    let pos = p.pos;
    let eye = p.eye();
    let on_ground = p.on_ground;
    let sprinting = p.sprinting;
    let held_slot = 36 + p.held as usize;
    let mut cancel_eat = false;
    let mut finish_eat = false;
    if let Some(e) = st.eating {
        let stack = p.inv.slots[e.inv_slot];
        let slot_ok = e.off_hand || e.inv_slot == held_slot;
        if gm == 3 || !slot_ok || stack.is_empty() || stack.item != e.item {
            cancel_eat = true;
        } else if now >= e.start + e.ticks as u64 {
            finish_eat = true;
        }
    }
    let probe = Probe::at(g, pos, eye);
    // Entity.updateFluidInteraction and baseTick, LivingEntity.handleOnClimbable,
    // HoneyBlock.entityInside and Player.tick: water, climbing, honey and flight
    // reset the fall, lava halves it
    let feet = block_name_at(g, pos[0].floor() as i32, pos[1].floor() as i32, pos[2].floor() as i32);
    if let Some(p) = g.player(slot) {
        if probe.feet_in_water || probe.eye_in_water || p.flying || climbable(&feet) || feet == "minecraft:honey_block" {
            p.fall_distance = 0.0;
        } else if probe.feet_in_lava {
            p.fall_distance *= 0.5;
        }
    }

    if cancel_eat {
        st.eating = None;
        set_living_flags(g, slot, 0);
    }
    if finish_eat {
        complete_meal(g, slot, st);
    }

    // survival and adventure have hunger, drowning and exposure damage
    let mortal = gm == 0 || gm == 2;

    let Some(p) = g.player(slot) else { return };
    let before = (p.health, p.food, p.saturation);
    let mut s = Stats { health: p.health, food: p.food, saturation: p.saturation, exhaustion: st.exhaustion, timer: st.timer };

    if mortal {
        if let Some(prev) = st.last_pos {
            let (dx, dy, dz) = (pos[0] - prev[0], pos[1] - prev[1], pos[2] - prev[2]);
            let horiz = (dx * dx + dz * dz).sqrt();
            let dist = (horiz * horiz + dy * dy).sqrt();
            if dist <= TELEPORT_DIST {
                add_exhaustion(&mut s, move_exhaustion(sprinting, probe.eye_in_water, probe.feet_in_water, horiz, dist));
                if st.prev_on_ground && !on_ground && dy > 0.0 {
                    add_exhaustion(&mut s, jump_exhaustion(sprinting));
                }
            }
        }
    }
    st.last_pos = Some(pos);
    st.prev_on_ground = on_ground;

    let mut starve = false;
    if mortal {
        starve = food_tick(&mut s).starve;
    }
    st.exhaustion = s.exhaustion;
    st.timer = s.timer;

    let changed = (s.health, s.food, s.saturation) != before;
    p.health = s.health;
    p.food = s.food;
    p.saturation = s.saturation;
    if changed {
        p.send(out::set_health(s.health, s.food, s.saturation));
    }

    if starve {
        interact::damage(g, slot, 1.0, "starve");
    }

    if mortal {
        let void = pos[1] < VOID_Y;
        if env_tick(&mut st.env_cooldown, void || probe.feet_in_lava) {
            let cause = if void { "void" } else { "lava" };
            interact::damage(g, slot, ENV_DAMAGE, cause);
        }
        if air_tick(&mut st.air, probe.eye_in_water) {
            interact::damage(g, slot, DROWN_DAMAGE, "drown");
        }
    }

    if st.air != st.air_sent {
        st.air_sent = st.air;
        let alive = g.player(slot).is_some_and(|p| !p.dead);
        if alive {
            let f = out::frame(out::Meta::new(eid).varint(AIR_INDEX, st.air).finish());
            send_meta(g, slot, f, false);
        }
    }
}

/// `FoodData.eat(nutrition, saturationModifier)` from a block (cake slice):
/// saturation gained is `nutrition * modifier * 2`.
pub fn eat_block(g: &mut Game, slot: usize, nutrition: i32, modifier: f32) {
    let Some(eid) = g.player(slot).map(|p| p.eid) else { return };
    let mut st = take_state(slot, eid);
    if let Some(p) = g.player(slot) {
        let mut s = Stats { health: p.health, food: p.food, saturation: p.saturation, exhaustion: st.exhaustion, timer: st.timer };
        apply_food(&mut s, nutrition, nutrition as f32 * modifier * 2.0);
        p.health = s.health;
        p.food = s.food;
        p.saturation = s.saturation;
        st.exhaustion = s.exhaustion;
        st.timer = s.timer;
        p.send(out::set_health(s.health, s.food, s.saturation));
    }
    put_state(slot, st);
}

/// `Player.canEat(false)`: hungry, or invulnerable (creative).
pub fn can_eat(p: &super::player::Player) -> bool {
    p.gamemode == 1 || p.food < MAX_FOOD
}

/// Finishes the eaten item: applies nutrition, consumes one item (not in
/// creative), and sends the use-complete event.
fn complete_meal(g: &mut Game, slot: usize, st: &mut PlayerSurvival) {
    let Some(e) = st.eating.take() else { return };
    set_living_flags(g, slot, 0);
    let Some(food) = food_of(e.item) else { return };
    let Some(p) = g.player(slot) else { return };
    let eid = p.eid;
    let mut s = Stats { health: p.health, food: p.food, saturation: p.saturation, exhaustion: st.exhaustion, timer: st.timer };
    apply_food(&mut s, food.nutrition, food.saturation);
    p.health = s.health;
    p.food = s.food;
    p.saturation = s.saturation;
    st.exhaustion = s.exhaustion;
    st.timer = s.timer;

    let consumed = p.gamemode != 1;
    if consumed {
        let sl = &mut p.inv.slots[e.inv_slot];
        sl.count -= 1;
        if sl.count <= 0 {
            *sl = Stack::EMPTY;
        }
        let sid = p.inv.state_id;
        let it = p.inv.slots[e.inv_slot];
        p.send(out::container_slot(0, sid, e.inv_slot as i16, &it));
    }
    p.send(out::set_health(s.health, s.food, s.saturation));
    // ServerPlayer.completeUsingItem: the use-complete event goes to the eater only
    p.out.push(out::frame(out::entity_event(eid, ENTITY_EVENT_USE_COMPLETE)));
    if consumed {
        super::track::equipment_changed(g, slot);
    }
    let Some(at) = g.player(slot).map(|p| p.pos) else { return };
    let seed = g.tick ^ ((eid as u64) << 20);
    let rnd = |k: u64| super::entity::rand_f64(seed.wrapping_add(k));
    // Consumable.emitParticlesAndSounds: eat sound, volume 0.5 or 1.0, pitch 1 +- 0.2
    let vol = if rnd(1) < 0.5 { 0.5 } else { 1.0 };
    super::track::sound_near(g, EAT_SOUND, SOUND_NEUTRAL, at, vol, (1.0 + 0.2 * (rnd(2) - rnd(3))) as f32, None);
    // FoodProperties.onConsume: the consumable sound again (pitch 1 +- 0.4), then the burp
    super::track::sound_near(g, EAT_SOUND, SOUND_NEUTRAL, at, 1.0, (1.0 + 0.4 * (rnd(4) - rnd(5))) as f32, None);
    super::track::sound_near(g, BURP_SOUND, SOUND_PLAYERS, at, 0.5, (0.9 + 0.1 * rnd(6)) as f32, None);
}

/// `LivingEntity.calculateFallDamage` (26.3): the fall beyond the safe distance,
/// times the damage modifier, rounded down. The 1e-6 keeps exact integers exact.
pub fn fall_damage(distance: f64, modifier: f32) -> i32 {
    ((distance + 1e-6 - SAFE_FALL_DISTANCE) * modifier as f64).floor() as i32
}

/// `BlockTags.CLIMBABLE` (the ones that matter for falls).
fn climbable(name: &str) -> bool {
    matches!(
        name,
        "minecraft:ladder" | "minecraft:vine" | "minecraft:scaffolding" | "minecraft:weeping_vines" | "minecraft:weeping_vines_plant" | "minecraft:twisting_vines" | "minecraft:twisting_vines_plant"
    )
}

/// Name of the block at integer coordinates; empty outside the world.
fn block_name_at(g: &Game, x: i32, y: i32, z: i32) -> String {
    if !(rine_world::MIN_Y..=rine_world::MAX_Y).contains(&y) {
        return String::new();
    }
    rine_data::get().blocks.block_of(g.world.get_block(x, y, z)).name.clone()
}

/// `Player.canCriticalAttack` terrain part: not in water, not on a climbable block.
pub fn crit_terrain_ok(g: &Game, pos: [f64; 3], eye: [f64; 3]) -> bool {
    let probe = Probe::at(g, pos, eye);
    let feet = block_name_at(g, pos[0].floor() as i32, pos[1].floor() as i32, pos[2].floor() as i32);
    !(probe.feet_in_water || probe.eye_in_water || climbable(&feet))
}

/// `Entity.checkFallDamage` for one client move, as vanilla
/// `ServerGamePacketListenerImpl.handleMovePlayer` → `doCheckFallDamage`: `dy` is
/// the vertical move of this packet and `on_ground` the client's flag.
pub fn check_fall(g: &mut Game, slot: usize, dy: f64, on_ground: bool) {
    let Some(p) = g.player(slot) else { return };
    let (pos, eye, sneaking) = (p.pos, p.eye(), p.sneaking);
    let probe = Probe::at(g, pos, eye);
    let Some(p) = g.player(slot) else { return };
    if dy < 0.0 && !(probe.feet_in_water || probe.eye_in_water) {
        p.fall_distance -= dy;
    }
    if !on_ground {
        return;
    }
    let fall = std::mem::take(&mut p.fall_distance);
    if fall > 0.0 {
        land(g, slot, fall, pos, sneaking);
    }
}

/// `Block.fallOn` for the block under the feet (vanilla `getOnPosLegacy`, y - 0.2).
fn land(g: &mut Game, slot: usize, fall: f64, pos: [f64; 3], sneaking: bool) {
    let name = block_name_at(g, pos[0].floor() as i32, (pos[1] - 0.2).floor() as i32, pos[2].floor() as i32);
    match name.as_str() {
        "minecraft:hay_block" => {
            cause_fall(g, slot, fall, 0.2);
        }
        // SlimeBlock.fallOn: sneaking skips the call, otherwise no damage
        "minecraft:slime_block" => {
            if !sneaking {
                cause_fall(g, slot, fall, 0.0);
            }
        }
        "minecraft:honey_block" => {
            super::track::sound_near(g, HONEY_SLIDE, SOUND_NEUTRAL, pos, 1.0, 1.0, None);
            cause_fall(g, slot, fall, 0.2);
        }
        // PowderSnowBlock.fallOn: sounds only, the snow takes no damage
        "minecraft:powder_snow" => {
            if fall >= 4.0 {
                let sound = if fall < 7.0 { SMALL_FALL } else { BIG_FALL };
                super::track::sound_near(g, sound, SOUND_NEUTRAL, pos, 1.0, 1.0, None);
            }
        }
        // Block.fallOn: beds halve the distance (fallDistanceReduction 0.5), modifier 1
        n => {
            let reduced = if n.ends_with("_bed") || n.contains("shelf_mushroom") { fall * 0.5 } else { fall };
            cause_fall(g, slot, reduced, 1.0);
        }
    }
}

/// `LivingEntity.causeFallDamage`: the hit, then the fall sound once it landed.
/// `playBlockFallSound` is not played: rine-data has no block sound types.
fn cause_fall(g: &mut Game, slot: usize, distance: f64, modifier: f32) -> bool {
    let dmg = fall_damage(distance, modifier);
    if dmg <= 0 || !interact::damage(g, slot, dmg as f32, "fall") {
        return false;
    }
    let Some(at) = g.player(slot).map(|p| p.pos) else { return true };
    let sound = if dmg > 4 { BIG_FALL } else { SMALL_FALL };
    super::track::sound_near(g, sound, SOUND_NEUTRAL, at, 1.0, 1.0, None);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init() {
        rine_data::init();
    }

    fn item(name: &str) -> u32 {
        rine_data::get().items.by_name(name).unwrap_or_else(|| panic!("no item {name}")).id
    }

    fn base(food: i32, sat: f32) -> Stats {
        Stats { health: 20.0, food, saturation: sat, exhaustion: 0.0, timer: 0 }
    }

    #[test]
    fn food_values_parse() {
        init();
        let b = food_of(item("minecraft:bread")).expect("bread is food");
        assert_eq!(b.nutrition, 5);
        assert!((b.saturation - 6.0).abs() < 1e-4);
        assert!(!b.always);
        let c = food_of(item("minecraft:cooked_beef")).expect("cooked beef is food");
        assert_eq!(c.nutrition, 8);
        assert!((c.saturation - 12.8).abs() < 1e-4);
        let a = food_of(item("minecraft:apple")).expect("apple is food");
        assert_eq!(a.nutrition, 4);
        assert!((a.saturation - 2.4).abs() < 1e-4);
        let g = food_of(item("minecraft:golden_apple")).expect("golden apple is food");
        assert!(g.always);
        assert!(food_of(item("minecraft:stone")).is_none());
        assert_eq!(eat_ticks(item("minecraft:bread")), 32);
        assert_eq!(eat_ticks(item("minecraft:dried_kelp")), 16);
    }

    #[test]
    fn exhaustion_spends_saturation_then_food() {
        let mut s = base(20, 5.0);
        add_exhaustion(&mut s, 4.5);
        food_tick(&mut s);
        assert!((s.exhaustion - 0.5).abs() < 1e-5);
        assert!((s.saturation - 4.0).abs() < 1e-5);
        assert_eq!(s.food, 20);

        let mut s = base(10, 0.0);
        add_exhaustion(&mut s, 4.5);
        food_tick(&mut s);
        assert_eq!(s.food, 9);
        assert!((s.exhaustion - 0.5).abs() < 1e-5);
    }

    #[test]
    fn exhaustion_is_capped() {
        let mut s = base(20, 5.0);
        add_exhaustion(&mut s, 100.0);
        assert_eq!(s.exhaustion, EXHAUSTION_CAP);
    }

    #[test]
    fn saturated_regen_every_ten_ticks() {
        let mut s = Stats { health: 10.0, food: 20, saturation: 5.0, exhaustion: 0.0, timer: 0 };
        for _ in 0..9 {
            food_tick(&mut s);
        }
        assert_eq!(s.health, 10.0);
        food_tick(&mut s);
        // saturation 5 heals 5/6 HP and costs 5 exhaustion
        assert!((s.health - (10.0 + 5.0 / 6.0)).abs() < 1e-4);
        assert!((s.exhaustion - 5.0).abs() < 1e-4);
        assert_eq!(s.timer, 0);
    }

    #[test]
    fn unsaturated_regen_every_eighty_ticks_at_food_18() {
        let mut s = Stats { health: 10.0, food: 18, saturation: 0.0, exhaustion: 0.0, timer: 0 };
        for _ in 0..79 {
            food_tick(&mut s);
        }
        assert_eq!(s.health, 10.0);
        food_tick(&mut s);
        assert_eq!(s.health, 11.0);
        assert!((s.exhaustion - EXHAUSTION_HEAL).abs() < 1e-5);
    }

    #[test]
    fn no_regen_below_food_18() {
        let mut s = Stats { health: 10.0, food: 17, saturation: 0.0, exhaustion: 0.0, timer: 0 };
        for _ in 0..200 {
            food_tick(&mut s);
        }
        assert_eq!(s.health, 10.0);
    }

    #[test]
    fn starvation_stops_at_one_hp_on_normal() {
        let mut s = Stats { health: 3.0, food: 0, saturation: 0.0, exhaustion: 0.0, timer: 0 };
        let mut hits = 0;
        for _ in 0..240 {
            if food_tick(&mut s).starve {
                hits += 1;
                s.health -= 1.0;
            }
        }
        // 3 HP: two hits leave 1 HP, and no third hit is allowed
        assert_eq!(hits, 2);
        assert_eq!(s.health, 1.0);
    }

    #[test]
    fn starvation_timer_is_eighty_ticks() {
        let mut s = Stats { health: 10.0, food: 0, saturation: 0.0, exhaustion: 0.0, timer: 0 };
        for _ in 0..79 {
            assert!(!food_tick(&mut s).starve);
        }
        assert!(food_tick(&mut s).starve);
    }

    #[test]
    fn eating_clamps_food_and_saturation() {
        let mut s = Stats { health: 20.0, food: 15, saturation: 2.0, exhaustion: 0.0, timer: 0 };
        apply_food(&mut s, 5, 6.0);
        assert_eq!(s.food, 20);
        assert!((s.saturation - 8.0).abs() < 1e-5);

        let mut s = Stats { health: 20.0, food: 19, saturation: 10.0, exhaustion: 0.0, timer: 0 };
        apply_food(&mut s, 8, 12.8);
        assert_eq!(s.food, 20);
        assert!((s.saturation - 20.0).abs() < 1e-5);
    }

    #[test]
    fn movement_and_jump_exhaustion_amounts() {
        assert!((move_exhaustion(true, false, false, 1.0, 1.0) - 0.1).abs() < 1e-6);
        assert_eq!(move_exhaustion(false, false, false, 1.0, 1.0), 0.0);
        assert!((move_exhaustion(false, false, true, 2.0, 2.0) - 0.02).abs() < 1e-6);
        assert!((move_exhaustion(false, true, true, 1.0, 3.0) - 0.03).abs() < 1e-6);
        assert!((jump_exhaustion(false) - 0.05).abs() < 1e-6);
        assert!((jump_exhaustion(true) - 0.2).abs() < 1e-6);
    }

    #[test]
    fn drowning_cycle() {
        let mut air = MAX_AIR;
        // 300 ticks of air, then 20 ticks of drowning per 2 damage
        let mut hits = 0;
        for _ in 0..(MAX_AIR + 20 * 3) {
            if air_tick(&mut air, true) {
                hits += 1;
            }
        }
        assert_eq!(hits, 3);
        assert_eq!(air, 0);
    }

    #[test]
    fn air_recovers_out_of_water() {
        let mut air = 0;
        for _ in 0..100 {
            assert!(!air_tick(&mut air, false));
        }
        assert_eq!(air, MAX_AIR);
    }

    #[test]
    fn exposure_damage_every_ten_ticks() {
        let mut cd = 0;
        let mut times = Vec::new();
        for t in 0..25 {
            if env_tick(&mut cd, true) {
                times.push(t);
            }
        }
        assert_eq!(times, vec![0, 10, 20]);
        assert!(!env_tick(&mut cd, false));
        assert!(env_tick(&mut cd, true));
    }
}

#[cfg(test)]
mod fall_tests {
    use super::*;

    #[test]
    fn fall_damage_starts_past_three_blocks() {
        assert!(fall_damage(2.0, 1.0) <= 0);
        assert_eq!(fall_damage(3.0, 1.0), 0);
        assert_eq!(fall_damage(4.0, 1.0), 1);
        assert_eq!(fall_damage(5.5, 1.0), 2);
        // hay bale: 20% of the excess; slime: nothing
        assert_eq!(fall_damage(10.0, 0.2), 1);
        assert_eq!(fall_damage(20.0, 0.0), 0);
    }

    #[test]
    fn climbable_blocks_and_sound_names_resolve() {
        rine_data::init();
        assert!(climbable("minecraft:ladder"));
        assert!(!climbable("minecraft:stone"));
        let reg = &rine_data::get().registries;
        for name in [EAT_SOUND, BURP_SOUND, SMALL_FALL, BIG_FALL, HONEY_SLIDE] {
            assert!(reg.id("minecraft:sound_event", name).is_some(), "missing sound {name}");
        }
    }
}
