//! /experience, /xp (`ExperienceCommand`) and player XP state.
//!
//! XP is not stored on `Player`, so it lives in a module-level map keyed by
//! entity id. Entries are dropped by `effect::on_quit`.

use crate::game::cmd::{lit, Commands, Ctx};
use crate::game::track;
use crate::game::Game;
use rine_command::{CmdError, CmdResult};
use rine_protocol::frame::packet;
use rine_protocol::ids::play::C_SET_EXPERIENCE;
use rine_protocol::text::Text;
use rine_protocol::Writer;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// `Player.experienceLevel`, `experienceProgress`, `totalExperience`,
/// `lastLevelUpTime`.
#[derive(Clone, Copy, Debug)]
struct Xp {
    level: i32,
    progress: f32,
    total: i32,
    last_up: i64,
}

impl Default for Xp {
    fn default() -> Self {
        Self { level: 0, progress: 0.0, total: 0, last_up: i64::MIN }
    }
}

/// XP by entity id of players.
static XP: Mutex<BTreeMap<i32, Xp>> = Mutex::new(BTreeMap::new());

fn with<R>(eid: i32, f: impl FnOnce(&mut Xp) -> R) -> R {
    let mut map = XP.lock().unwrap_or_else(|e| e.into_inner());
    f(map.entry(eid).or_default())
}

/// Drop the XP of a player that left.
pub fn forget(eid: i32) {
    XP.lock().unwrap_or_else(|e| e.into_inner()).remove(&eid);
}

/// `Player.getXpNeededForNextLevel`.
fn xp_needed(level: i32) -> i32 {
    if level >= 30 {
        112 + (level - 30) * 9
    } else if level >= 15 {
        37 + (level - 15) * 5
    } else {
        7 + level * 2
    }
}

impl Xp {
    fn needed(&self) -> i32 {
        xp_needed(self.level)
    }

    /// `Player.giveExperienceLevels`. Pushes the level when a level-up
    /// sound is due (`lastLevelUpTime` throttle).
    fn give_levels(&mut self, amount: i32, tick: i64, sounds: &mut Vec<i32>) {
        self.level = self.level.saturating_add(amount);
        if self.level < 0 {
            self.level = 0;
            self.progress = 0.0;
            self.total = 0;
        }
        if amount > 0 && self.level % 5 == 0 && self.last_up < tick - 100 {
            self.last_up = tick;
            sounds.push(self.level);
        }
    }

    /// `Player.giveExperiencePoints`.
    fn give_points(&mut self, amount: i32, tick: i64, sounds: &mut Vec<i32>) {
        self.progress += amount as f32 / self.needed() as f32;
        self.total = (self.total as i64 + amount as i64).clamp(0, i32::MAX as i64) as i32;
        while self.progress < 0.0 {
            let remaining = self.progress * self.needed() as f32;
            if self.level > 0 {
                self.give_levels(-1, tick, sounds);
                self.progress = 1.0 + remaining / self.needed() as f32;
            } else {
                self.give_levels(-1, tick, sounds);
                self.progress = 0.0;
            }
        }
        while self.progress >= 1.0 {
            self.progress = (self.progress - 1.0) * self.needed() as f32;
            self.give_levels(1, tick, sounds);
            self.progress /= self.needed() as f32;
        }
    }

    /// `ServerPlayer.setExperiencePoints` guarded by `ExperienceCommand`:
    /// false when the amount reaches the next level.
    fn set_points(&mut self, amount: i32) -> bool {
        if amount >= self.needed() {
            return false;
        }
        let limit = self.needed() as f32;
        let max = (limit - 1.0) / limit;
        self.progress = (amount as f32 / limit).max(0.0).min(max);
        true
    }

    fn points_shown(&self) -> i32 {
        (self.progress * self.needed() as f32).floor() as i32
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Points,
    Levels,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Points => "points",
            Kind::Levels => "levels",
        }
    }
}

pub fn register(c: &mut Commands) {
    c.exec("experience add target amount", |x| add(x, Kind::Points));
    c.exec("experience add target amount points", |x| add(x, Kind::Points));
    c.exec("experience add target amount levels", |x| add(x, Kind::Levels));
    c.exec("experience set target amount", |x| set(x, Kind::Points));
    c.exec("experience set target amount points", |x| set(x, Kind::Points));
    c.exec("experience set target amount levels", |x| set(x, Kind::Levels));
    c.exec("experience query target points", |x| query(x, Kind::Points));
    c.exec("experience query target levels", |x| query(x, Kind::Levels));
}

/// Level of a player entity id (0 when unknown or not a player).
pub fn level_of(g: &Game, eid: i32) -> i32 {
    if !g.by_eid.contains_key(&eid) {
        return 0;
    }
    with(eid, |xp| xp.level)
}

/// `ClientboundSetExperiencePacket` (progress, level, total) to the player.
fn send_xp(g: &mut Game, slot: usize, xp: Xp) {
    let Some(p) = g.player(slot) else { return };
    let pkt = packet(C_SET_EXPERIENCE, |b| {
        b.w_f32(xp.progress);
        b.w_varint(xp.level);
        b.w_varint(xp.total);
    });
    p.send(pkt);
}

/// Apply `f` to a player's XP, play the level-up sounds it asks for and
/// send the new state. Returns what `f` returned.
fn modify(g: &mut Game, slot: usize, f: impl FnOnce(&mut Xp, &mut Vec<i32>, i64) -> bool) -> bool {
    let Some(p) = g.players.get(slot).and_then(|p| p.as_ref()) else { return false };
    let (eid, pos) = (p.eid, p.pos);
    let tick = g.tick as i64;
    let mut sounds = Vec::new();
    let (ok, xp) = with(eid, |xp| {
        let ok = f(xp, &mut sounds, tick);
        (ok, *xp)
    });
    for level in sounds {
        let vol = if level > 30 { 1.0 } else { level as f32 / 30.0 };
        track::sound_near(g, "minecraft:entity.player.levelup", 7, pos, vol * 0.75, 1.0, None);
    }
    send_xp(g, slot, xp);
    ok
}

fn add(x: &mut Ctx, kind: Kind) -> CmdResult<i32> {
    let slots = x.players("target")?;
    let amount = x.int("amount");
    for &slot in &slots {
        modify(x.g, slot, |xp, sounds, tick| {
            match kind {
                Kind::Points => xp.give_points(amount, tick, sounds),
                Kind::Levels => xp.give_levels(amount, tick, sounds),
            }
            true
        });
    }
    let n = slots.len() as i32;
    let key = format!("commands.experience.add.{}.success.", kind.name());
    let msg = if n == 1 {
        Text::tr(format!("{key}single"), vec![lit(amount), x.player_name(slots[0])])
    } else {
        Text::tr(format!("{key}multiple"), vec![lit(amount), lit(n)])
    };
    x.success(msg, true);
    Ok(n)
}

fn set(x: &mut Ctx, kind: Kind) -> CmdResult<i32> {
    let slots = x.players("target")?;
    let amount = x.int("amount");
    let mut ok_slots = Vec::with_capacity(slots.len());
    for &slot in &slots {
        let ok = modify(x.g, slot, |xp, _, _| match kind {
            Kind::Points => xp.set_points(amount),
            Kind::Levels => {
                xp.level = amount;
                true
            }
        });
        if ok {
            ok_slots.push(slot);
        }
    }
    let n = ok_slots.len() as i32;
    if n == 0 && kind == Kind::Points {
        return Err(CmdError::key("commands.experience.set.points.invalid"));
    }
    let key = format!("commands.experience.set.{}.success.", kind.name());
    let msg = if n == 1 {
        Text::tr(format!("{key}single"), vec![lit(amount), x.player_name(ok_slots[0])])
    } else {
        Text::tr(format!("{key}multiple"), vec![lit(amount), lit(n)])
    };
    x.success(msg, true);
    Ok(n)
}

fn query(x: &mut Ctx, kind: Kind) -> CmdResult<i32> {
    let slot = x.player("target")?;
    let eid = x.g.players[slot].as_ref().map(|p| p.eid).unwrap_or(0);
    let result = with(eid, |xp| match kind {
        Kind::Points => xp.points_shown(),
        Kind::Levels => xp.level,
    });
    let msg = Text::tr(format!("commands.experience.query.{}", kind.name()), vec![x.player_name(slot), lit(result)]);
    x.success(msg, false);
    Ok(result)
}
