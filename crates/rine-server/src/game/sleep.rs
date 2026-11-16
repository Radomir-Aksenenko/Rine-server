//! Sleeping in beds, the player side: `ServerPlayer.startSleepInBed` with
//! `getBedResult`, `LivingEntity.startSleeping` / `stopSleeping`,
//! `ServerPlayer.stopSleepInBed`, the sleep part of `Player.tick`, the sleep
//! block of `ServerLevel.tick`, `SleepStatus` and the respawn point.
//! The block side (click, occupied bit, stand-up cell) is `blocks/bed.rs`.
//!
//! Hooks: `level_tick` and `player_tick` run from `cmd::commands::world::tick`
//! (weather, then this, then `tickTime`); `bed_use` from `interact::use_item_on`;
//! `leave_bed` from the player command packet (action 0); `stop_sleep` when a
//! sleeper is hurt; `on_leave` and `update_status` around join and leave.

use super::blocks::bed::{self, BedUse};
use super::track;
use super::Game;
use crate::game::cmd::commands::world::Respawn;
use crate::out;
use rine_data::blocks::flag;
use rine_protocol::text::Text;
use rine_protocol::BlockPos;

/// `ServerboundPlayerCommandPacket.Action.STOP_SLEEPING` (action id 0).
pub const ACTION_STOP_SLEEPING: i32 = 0;
/// `ClientboundAnimatePacket.WAKE_UP`.
const ANIM_WAKE_UP: u8 = 2;
/// `Player.SLEEP_DURATION`: ticks in bed before the sleeper counts as deep asleep.
const SLEEP_DURATION: i32 = 100;
/// `Player.tick`: the wake-up counter after leaving the bed resets at 110.
const WAKE_UP_COUNTER_END: i32 = 110;
/// `Pose.SLEEPING` id in entity data index 6.
pub const POSE_SLEEPING: i32 = 2;
/// `Player.stopSleepInBed` countdown after a normal wake-up.
const AWAKE_COUNTDOWN: i32 = 100;

/// `SleepStatus`: the counts of the last update.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    active: i32,
    sleeping: i32,
}

impl Status {
    pub fn sleeping(&self) -> i32 {
        self.sleeping
    }
    /// `sleepersNeeded`.
    pub fn needed(&self, pct: i32) -> i32 {
        sleepers_needed(self.active, pct)
    }
    /// `areEnoughSleeping`.
    pub fn enough_sleeping(&self, pct: i32) -> bool {
        self.sleeping >= self.needed(pct)
    }
    /// `areEnoughDeepSleeping`, with the count of players asleep 100 ticks or more.
    pub fn enough_deep(&self, pct: i32, deep: i32) -> bool {
        deep >= self.needed(pct)
    }
    /// `removeAllSleepers`.
    pub fn remove_all_sleepers(&mut self) {
        self.sleeping = 0;
    }
}

/// `SleepStatus.sleepersNeeded`: `max(1, ceil(active * pct / 100f))`, with the
/// Java int product (it wraps on overflow) divided as a float.
pub fn sleepers_needed(active: i32, pct: i32) -> i32 {
    let f = active.wrapping_mul(pct) as f32 / 100.0;
    (f.ceil() as i32).max(1)
}

/// `SleepStatus.update` on new counts. Returns the new status and whether
/// someone is asleep and a count moved (then the action bar is updated).
fn recount(old: Status, active: i32, sleeping: i32) -> (Status, bool) {
    let changed = (old.sleeping > 0 || sleeping > 0) && (old.active != active || old.sleeping != sleeping);
    (Status { active, sleeping }, changed)
}

fn sleep_percentage(g: &Game) -> i32 {
    g.rules.int("players_sleeping_percentage")
}

/// `Level.isDarkOutside` for the overworld (`BedRule` `when_dark`).
fn dark_now(g: &Game) -> bool {
    let w = &g.cs.world.weather;
    bed::dark_outside(super::blocks::sky_darken(g.day_time, w.rain_level, w.thunder_level))
}

/// `BlockState.isSuffocating` for the cell above a bed (`Player.freeAt`), approximated by the full-cube flag.
fn suffocates(g: &Game, p: BlockPos) -> bool {
    rine_data::get().blocks.has(g.world.get_block(p.x, p.y, p.z), flag::FULL_CUBE)
}

fn overlay(g: &mut Game, slot: usize, key: &'static str) {
    if let Some(p) = g.player(slot) {
        p.send(out::action_bar(&Text::key(key)));
    }
}

/// `ServerLevel.updateSleepingPlayerList`: recount, and announce on the action
/// bar when someone sleeps and a count moved. Call after every join, leave,
/// sleep start and wake-up that records a change (`update_list`).
pub fn update_status(g: &mut Game) {
    if !g.players.iter().any(|p| p.is_some()) {
        return;
    }
    let (mut active, mut sleeping) = (0, 0);
    for p in g.players.iter().flatten() {
        if p.gamemode != 3 {
            active += 1;
            if p.sleep.is_some() {
                sleeping += 1;
            }
        }
    }
    let (next, changed) = recount(g.sleep, active, sleeping);
    g.sleep = next;
    if changed {
        announce(g);
    }
}

/// `ServerLevel.announceSleepStatus`: skipped when the rule is above 100 (no skipping at all).
fn announce(g: &mut Game) {
    let pct = sleep_percentage(g);
    if pct > 100 {
        return;
    }
    let text = if g.sleep.enough_sleeping(pct) {
        Text::key("sleep.skipping_night")
    } else {
        Text::tr("sleep.players_sleeping", vec![Text::plain(g.sleep.sleeping().to_string()), Text::plain(g.sleep.needed(pct).to_string())])
    };
    let f = out::frame(out::action_bar(&text));
    for p in g.players.iter_mut().flatten() {
        p.send_frame(f.clone());
    }
}

/// Right click on a bed half, after `BedBlock.useWithoutItem` decided what to do.
pub fn bed_use(g: &mut Game, slot: usize, b: BedUse) {
    match b {
        BedUse::Occupied => overlay(g, slot, "block.minecraft.bed.occupied"),
        BedUse::Sleep { head, facing } => start_sleep(g, slot, head, facing),
    }
}

/// `ServerPlayer.startSleepInBed` with `getBedResult`, then `Player.startSleepInBed`.
fn start_sleep(g: &mut Game, slot: usize, head: BlockPos, facing: u8) {
    let (feet, dead, asleep, yaw, pitch) = match g.player(slot) {
        Some(p) => (p.pos, p.dead || p.health <= 0.0, p.sleep.is_some(), p.yaw, p.pitch),
        None => return,
    };
    // OTHER_PROBLEM (dead, or already asleep): no message
    if dead || asleep {
        return;
    }
    if !bed::in_range(feet, head, facing) {
        return overlay(g, slot, "block.minecraft.bed.too_far_away");
    }
    let above = bed::step_pos(head, crate::game::redstone::data::UP);
    let blocked = suffocates(g, above) || suffocates(g, bed::step_pos(above, crate::game::redstone::data::opposite(facing)));
    if blocked {
        return overlay(g, slot, "block.minecraft.bed.obstructed");
    }
    // the overworld rule can set the spawn at any time (before the night check)
    set_respawn(g, slot, Respawn { pos: [head.x, head.y, head.z], yaw, pitch, forced: false }, true);
    if !dark_now(g) {
        return overlay(g, slot, "block.minecraft.bed.no_sleep");
    }
    // the monster check of `getBedResult` has nothing to find: Rine has no hostile mobs yet

    // LivingEntity.startSleeping: occupied bit, pose, sleeping position, no motion
    let st = g.world.get_block(head.x, head.y, head.z);
    if !bed::is_bed(st) {
        return;
    }
    super::redstone::set_block(g, head, bed::with_occupied(st, true));
    let at = [head.x as f64 + 0.5, head.y as f64 + bed::SLEEP_HEIGHT + 0.125, head.z as f64 + 0.5];
    if let Some(p) = g.player(slot) {
        p.sleep = Some(head);
        p.sleep_timer = 0;
        p.vel = [0.0; 3];
        p.teleport(at[0], at[1], at[2], yaw, pitch);
    }
    track::flags_changed(g, slot);
    if sleep_percentage(g) > 100 {
        overlay(g, slot, "sleep.not_possible");
    }
    update_status(g);
}

/// `ServerPlayer.setRespawnPosition(config, showMessage)`: the chat line only
/// when the point actually moved.
fn set_respawn(g: &mut Game, slot: usize, r: Respawn, show: bool) {
    let Some(p) = g.player(slot) else { return };
    let moved = p.respawn.map_or(true, |o| o.pos != r.pos);
    p.respawn = Some(r);
    if show && moved {
        p.msg(&Text::key("block.minecraft.set_spawn"));
    }
}

/// `ServerPlayer.stopSleepInBed` with `LivingEntity.stopSleeping`: the wake-up
/// animation, the bed's occupied bit off, the stand-up cell and its yaw.
/// `forceful` leaves the countdown at 0 (else 100), as `Player.stopSleepInBed`.
pub fn stop_sleep(g: &mut Game, slot: usize, forceful: bool, update_list: bool) {
    let Some(p) = g.player(slot) else { return };
    let Some(head) = p.sleep else { return };
    let (eid, yaw) = (p.eid, p.yaw);
    let anim = out::frame(out::animate(eid, ANIM_WAKE_UP));
    track::send_to_trackers(g, eid, &anim);
    if let Some(p) = g.player(slot) {
        p.send_frame(anim);
    }
    let st = g.world.get_block(head.x, head.y, head.z);
    let (stand, new_yaw) = if bed::is_bed(st) {
        super::redstone::set_block(g, head, bed::with_occupied(st, false));
        match bed::stand_up(|q| g.world.get_block(q.x, q.y, q.z), head, bed::facing(st), yaw) {
            Some(feet) => (feet, bed::look_at_yaw(feet, head)),
            None => fallback_stand(head, yaw),
        }
    } else {
        fallback_stand(head, yaw)
    };
    let Some(p) = g.player(slot) else { return };
    p.sleep = None;
    p.sleep_timer = countdown(forceful);
    p.teleport(stand[0], stand[1], stand[2], new_yaw, 0.0);
    track::flags_changed(g, slot);
    if update_list {
        update_status(g);
    }
}

/// `Player.stopSleepInBed` countdown: 0 after a forceful wake, else 100.
fn countdown(forceful: bool) -> i32 {
    if forceful {
        0
    } else {
        AWAKE_COUNTDOWN
    }
}

/// `LivingEntity.stopSleeping` when no stand-up cell is found: the cell above the bed.
fn fallback_stand(head: BlockPos, yaw: f32) -> ([f64; 3], f32) {
    let feet = [head.x as f64 + 0.5, head.y as f64 + 1.1, head.z as f64 + 0.5];
    (feet, yaw)
}

/// Packet action 0 (`STOP_SLEEPING`): wake up if asleep.
pub fn leave_bed(g: &mut Game, slot: usize) {
    if g.player(slot).is_some_and(|p| p.sleep.is_some()) {
        stop_sleep(g, slot, false, true);
    }
}

/// Disconnect (`ServerPlayer.disconnect`): wake up without a status update;
/// `Game::leave` recounts once the player is gone.
pub fn on_leave(g: &mut Game, slot: usize) {
    if g.player(slot).is_some_and(|p| p.sleep.is_some()) {
        stop_sleep(g, slot, true, false);
    }
}

/// `Player.tick` sleep part and `LivingEntity.checkBedExists`, once per tick.
/// Runs after the clock step, as `tickTime` comes before entity ticks in vanilla.
pub fn player_tick(g: &mut Game) {
    let slots: Vec<usize> = g.players.iter().enumerate().filter_map(|(i, p)| p.as_ref().map(|_| i)).collect();
    for slot in slots {
        let Some(p) = g.player(slot) else { continue };
        let (head, timer) = (p.sleep, p.sleep_timer);
        match head {
            Some(h) => {
                if let Some(p) = g.player(slot) {
                    p.sleep_timer = (timer + 1).min(SLEEP_DURATION);
                }
                let st = g.world.get_block(h.x, h.y, h.z);
                if bed::is_bed(st) {
                    // Player.tick: the bed no longer allows sleeping (morning came)
                    if !dark_now(g) {
                        stop_sleep(g, slot, false, true);
                    }
                } else {
                    // checkBedExists fails: the bed was removed
                    stop_sleep(g, slot, true, true);
                }
            }
            None => {
                if timer > 0 {
                    let next = timer + 1;
                    if let Some(p) = g.player(slot) {
                        p.sleep_timer = if next >= WAKE_UP_COUNTER_END { 0 } else { next };
                    }
                }
            }
        }
    }
}

/// `ServerLevel.tick` sleep block: when enough players sleep (and deep sleep
/// reached), skip to the next morning, wake everyone and reset the weather.
/// Runs after the weather step and before `tickTime`.
pub fn level_tick(g: &mut Game) {
    let pct = sleep_percentage(g);
    let deep = g.players.iter().flatten().filter(|p| p.sleep.is_some() && p.sleep_timer >= SLEEP_DURATION).count() as i32;
    if !(g.sleep.enough_sleeping(pct) && g.sleep.enough_deep(pct, deep)) {
        return;
    }
    if g.rules.bool("advance_time") {
        // moveToTimeMarker: an exact marker tick does not move; the clock is broadcast either way
        if let Some(t) = bed::skip_target(g.day_time) {
            g.day_time = t;
            g.cs.world.clock.partial = 0.0;
        }
        super::cmd::commands::world::broadcast_clock(g);
    }
    wake_all(g);
    if g.rules.bool("advance_weather") && g.cs.world.weather.rain_level > 0.2 {
        reset_weather(g);
    }
}

/// `ServerLevel.wakeUpAllPlayers`: no status update, each sleeper wakes with `stopSleepInBed(false, false)`.
fn wake_all(g: &mut Game) {
    g.sleep.remove_all_sleepers();
    let slots: Vec<usize> = g.players.iter().enumerate().filter_map(|(i, p)| p.as_ref().filter(|p| p.sleep.is_some()).map(|_| i)).collect();
    for s in slots {
        stop_sleep(g, s, false, false);
    }
}

/// `ServerLevel.resetWeatherCycle`: no rain or thunder, and new random timers.
/// (Rine's weather flags flip at the next weather step; vanilla keys the
/// game event on the rain level, which is not modelled here.)
fn reset_weather(g: &mut Game) {
    let w = &mut g.cs.world.weather;
    w.raining = false;
    w.rain = 0;
    w.thundering = false;
    w.thunder = 0;
}

/// Where a respawning player appears: feet, yaw, pitch. A point that no
/// longer works is dropped and the world spawn is used, as vanilla does
/// (`missingRespawnBlock`: the config is not copied and the client gets game event 0).
pub fn spawn_for(g: &mut Game, slot: usize) -> ([f64; 3], f32, f32) {
    let default = (g.spawn, 0.0, 0.0);
    let Some(r) = g.player(slot).and_then(|p| p.respawn) else { return default };
    match resolve_respawn(g, r) {
        Some(v) => v,
        None => {
            if let Some(p) = g.player(slot) {
                p.respawn = None;
                p.send(out::game_event(0, 0.0));
            }
            default
        }
    }
}

/// `ServerPlayer.findRespawnAndUseSpawnBlock` for a bed or a forced point.
fn resolve_respawn(g: &Game, r: Respawn) -> Option<([f64; 3], f32, f32)> {
    let head = BlockPos::new(r.pos[0], r.pos[1], r.pos[2]);
    let st = g.world.get_block(head.x, head.y, head.z);
    if r.forced {
        // `/spawnpoint`: the cell and the one above must not be solid (`isPossibleToRespawnInThis`)
        let above = g.world.get_block(head.x, head.y + 1, head.z);
        let b = &rine_data::get().blocks;
        if b.has(st, flag::SOLID) || b.has(above, flag::SOLID) {
            return None;
        }
        return Some(([head.x as f64 + 0.5, head.y as f64 + 0.1, head.z as f64 + 0.5], r.yaw, r.pitch));
    }
    if !bed::is_bed(st) {
        return None;
    }
    let feet = bed::stand_up(|q| g.world.get_block(q.x, q.y, q.z), head, bed::facing(st), r.yaw)?;
    Some((feet, bed::look_at_yaw(feet, head), 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sleepers_needed_rounds_up_and_keeps_one() {
        assert_eq!(sleepers_needed(2, 50), 1);
        assert_eq!(sleepers_needed(3, 50), 2);
        assert_eq!(sleepers_needed(10, 100), 10);
        assert_eq!(sleepers_needed(0, 100), 1);
        // above 100 nobody can reach the count
        assert!(sleepers_needed(4, 101) > 4);
    }

    #[test]
    fn one_of_two_players_skips_at_fifty_percent_once_deep() {
        let mut s = Status::default();
        (s, _) = recount(s, 2, 1);
        assert!(s.enough_sleeping(50));
        // not deep yet: nobody has been in bed 100 ticks
        assert!(!s.enough_deep(50, 0));
        assert!(s.enough_deep(50, 1));
        // at 100 percent the one sleeper is not enough
        assert!(!s.enough_sleeping(100));
    }

    #[test]
    fn status_announces_only_while_someone_sleeps() {
        let none = Status { active: 2, sleeping: 0 };
        assert!(recount(none, 2, 1).1, "first sleeper");
        let one = Status { active: 2, sleeping: 1 };
        assert!(!recount(one, 2, 1).1, "no change");
        assert!(recount(one, 2, 0).1, "sleeper got up");
        assert!(!recount(none, 3, 0).1, "join with nobody asleep is silent");
        assert!(recount(one, 3, 1).1, "join while someone sleeps");
    }

    #[test]
    fn leave_packet_action_is_stop_sleeping() {
        // ServerboundPlayerCommandPacket.Action: STOP_SLEEPING is the first constant
        assert_eq!(ACTION_STOP_SLEEPING, 0);
    }

    #[test]
    fn awake_countdown_depends_on_how_it_ended() {
        // stopSleepInBed(true, _) -> 0, stopSleepInBed(false, _) -> 100
        assert_eq!(countdown(true), 0);
        assert_eq!(countdown(false), 100);
    }
}
