//! /kill, /tag, /damage, /ride, /swing (`KillCommand`, `TagCommand`,
//! `DamageCommand`, `RideCommand`, `SwingCommand`).

use crate::game::cmd::{lit, Commands, Ctx};
use crate::game::interact;
use crate::game::Game;
use crate::out;
use rine_command::error::java_float;
use rine_command::{CmdError, CmdResult};
use rine_protocol::text::{hover_text, Text};
use rine_protocol::Writer;
use std::collections::BTreeSet;

/// `ClientboundSwingAnimationPacket` (`minecraft:swing_animation`).
const C_SWING_ANIMATION: i32 = rine_protocol::ids::play::C_SWING_ANIMATION;

/// `Entity.MAX_ENTITY_TAG_COUNT` (SizeLimitedSet bound).
const MAX_TAGS: usize = 1024;

pub fn register(c: &mut Commands) {
    c.exec("kill", |x| {
        let eid = x.source_entity()?;
        kill(x, vec![eid])
    });
    c.exec("kill targets", |x| {
        let targets = x.entities("targets")?;
        kill(x, targets)
    });

    c.exec("tag targets add name", |x| {
        let targets = x.entities("targets")?;
        let name = x.str("name").to_string();
        tag_add(x, targets, &name)
    });
    c.exec("tag targets remove name", |x| {
        let targets = x.entities("targets")?;
        let name = x.str("name").to_string();
        tag_remove(x, targets, &name)
    });
    c.exec("tag targets list", |x| {
        let targets = x.entities("targets")?;
        tag_list(x, targets)
    });

    c.exec("damage target amount", |x| do_damage(x, "generic"));
    c.exec("damage target amount damageType", |x| {
        let cause = damage_cause(x);
        do_damage(x, &cause)
    });
    c.exec("damage target amount damageType at location", |x| {
        let _ = x.vec3("location");
        let cause = damage_cause(x);
        do_damage(x, &cause)
    });
    c.exec("damage target amount damageType by entity", |x| {
        let _ = x.entity("entity")?;
        let cause = damage_cause(x);
        do_damage(x, &cause)
    });
    c.exec("damage target amount damageType by entity from cause", |x| {
        let _ = x.entity("entity")?;
        let _ = x.entity("cause")?;
        let cause = damage_cause(x);
        do_damage(x, &cause)
    });

    c.exec("ride target mount vehicle", |x| {
        let target = x.entity("target")?;
        let vehicle = x.entity("vehicle")?;
        mount(x, target, vehicle)
    });
    c.exec("ride target dismount", |x| {
        let target = x.entity("target")?;
        // no vehicles exist in Rine: never riding
        Err(CmdError::tr("commands.ride.not_riding", vec![x.name_of(target)]))
    });

    c.exec("swing", |x| {
        let eid = x.source_entity()?;
        swing(x, vec![eid], 0, 1, 6)
    });
    c.exec("swing targets", |x| {
        let targets = x.entities("targets")?;
        swing(x, targets, 0, 1, 6)
    });
    c.exec("swing targets mainhand", |x| {
        let targets = x.entities("targets")?;
        swing(x, targets, 0, 1, 6)
    });
    c.exec("swing targets offhand", |x| {
        let targets = x.entities("targets")?;
        swing(x, targets, 1, 1, 6)
    });
    c.exec("swing targets mainhand animation", |x| {
        let targets = x.entities("targets")?;
        let anim = x.enum_("animation");
        swing(x, targets, 0, anim, 6)
    });
    c.exec("swing targets offhand animation", |x| {
        let targets = x.entities("targets")?;
        let anim = x.enum_("animation");
        swing(x, targets, 1, anim, 6)
    });
    c.exec("swing targets mainhand animation duration", |x| {
        let targets = x.entities("targets")?;
        let anim = x.enum_("animation");
        let dur = x.int("duration");
        swing(x, targets, 0, anim, dur)
    });
    c.exec("swing targets offhand animation duration", |x| {
        let targets = x.entities("targets")?;
        let anim = x.enum_("animation");
        let dur = x.int("duration");
        swing(x, targets, 1, anim, dur)
    });
}

/// `CommandResponseTracker` for (element, value) pairs: the non-zero
/// elements decide between the single and the multiple message.
fn nonzero(v: &[(i32, bool)]) -> Vec<i32> {
    v.iter().filter(|(_, b)| *b).map(|(e, _)| *e).collect()
}

/// `KillCommand.kill`. Players die through the damage path with the
/// `generic_kill` type; other entities are removed.
fn kill(x: &mut Ctx, eids: Vec<i32>) -> CmdResult<i32> {
    let single_name = if eids.len() == 1 { Some(x.name_of(eids[0])) } else { None };
    for &e in &eids {
        kill_one(x.g, e);
    }
    let n = eids.len();
    let t = match single_name {
        Some(name) => Text::tr("commands.kill.success.single", vec![name]),
        None => Text::tr("commands.kill.success.multiple", vec![lit(n)]),
    };
    x.success(t, true);
    Ok(n as i32)
}

fn kill_one(g: &mut Game, eid: i32) {
    if let Some(&slot) = g.by_eid.get(&eid) {
        // LivingEntity.kill uses generic_kill, which ignores creative and
        // spectator invulnerability: lift the mode check for this hit only.
        let Some(p) = g.player(slot) else { return };
        let saved = p.gamemode;
        p.gamemode = 0;
        interact::damage(g, slot, f32::MAX, "generic_kill");
        if let Some(p) = g.player(slot) {
            p.gamemode = saved;
        }
    } else if let Some(i) = g.entities.slot_of(eid) {
        g.entities.list[i].removed = true;
        g.ent_meta.remove(&eid);
    }
}

/// `EntitySelector` tags of an entity, in insertion order.
fn tags_of(g: &Game, eid: i32) -> Vec<String> {
    g.ent_meta.get(&eid).map(|m| m.tags.clone()).unwrap_or_default()
}

/// `formatList(tags)`: sorted, green names, gray ", " separators.
fn format_tags(tags: &BTreeSet<String>) -> Text {
    let mut t = Text::empty();
    for (i, name) in tags.iter().enumerate() {
        if i > 0 {
            t = t.push(Text::plain(", ").color("gray"));
        }
        t = t.push(Text::plain(name.clone()).color("green"));
    }
    t
}

/// `Entity.addTag`: false when present or the set is full.
fn add_tag(g: &mut Game, eid: i32, name: &str) -> bool {
    let meta = g.ent_meta.entry(eid).or_default();
    if meta.tags.len() >= MAX_TAGS || meta.tags.iter().any(|t| t == name) {
        return false;
    }
    meta.tags.push(name.to_string());
    true
}

fn remove_tag(g: &mut Game, eid: i32, name: &str) -> bool {
    match g.ent_meta.get_mut(&eid) {
        Some(m) => match m.tags.iter().position(|t| t == name) {
            Some(i) => {
                m.tags.remove(i);
                true
            }
            None => false,
        },
        None => false,
    }
}

/// `TagCommand.addTag`.
fn tag_add(x: &mut Ctx, targets: Vec<i32>, name: &str) -> CmdResult<i32> {
    let mut resp = Vec::with_capacity(targets.len());
    for &e in &targets {
        resp.push((e, add_tag(x.g, e, name)));
    }
    let changed = nonzero(&resp);
    if changed.is_empty() {
        return Err(CmdError::key("commands.tag.add.failed"));
    }
    let t = if changed.len() == 1 {
        Text::tr("commands.tag.add.success.single", vec![lit(name), x.name_of(changed[0])])
    } else {
        Text::tr("commands.tag.add.success.multiple", vec![lit(name), lit(changed.len())])
    };
    x.success(t, true);
    Ok(changed.len() as i32)
}

/// `TagCommand.removeTag`.
fn tag_remove(x: &mut Ctx, targets: Vec<i32>, name: &str) -> CmdResult<i32> {
    let mut resp = Vec::with_capacity(targets.len());
    for &e in &targets {
        resp.push((e, remove_tag(x.g, e, name)));
    }
    let changed = nonzero(&resp);
    if changed.is_empty() {
        return Err(CmdError::key("commands.tag.remove.failed"));
    }
    let t = if changed.len() == 1 {
        Text::tr("commands.tag.remove.success.single", vec![lit(name), x.name_of(changed[0])])
    } else {
        Text::tr("commands.tag.remove.success.multiple", vec![lit(name), lit(changed.len())])
    };
    x.success(t, true);
    Ok(changed.len() as i32)
}

/// `TagCommand.listTags`. Mirrors the tracker: the "no tags" message counts
/// non-zero elements, which is 0 when nothing has a tag.
fn tag_list(x: &mut Ctx, targets: Vec<i32>) -> CmdResult<i32> {
    let mut all = BTreeSet::new();
    let mut resp = Vec::with_capacity(targets.len());
    for &e in &targets {
        let tags = tags_of(x.g, e);
        resp.push((e, !tags.is_empty()));
        all.extend(tags);
    }
    let nz = nonzero(&resp);
    if all.is_empty() {
        let t = if nz.len() == 1 {
            Text::tr("commands.tag.list.single.empty", vec![x.name_of(nz[0])])
        } else {
            Text::tr("commands.tag.list.multiple.empty", vec![lit(nz.len())])
        };
        x.success(t, false);
    } else {
        let t = if nz.len() == 1 {
            Text::tr("commands.tag.list.single.success", vec![x.name_of(nz[0]), lit(all.len()), format_tags(&all)])
        } else {
            Text::tr("commands.tag.list.multiple.success", vec![lit(nz.len()), lit(all.len()), format_tags(&all)])
        };
        x.success(t, false);
    }
    Ok(all.len() as i32)
}

/// Damage type name (`namespace:path` without the `minecraft:` prefix).
fn damage_cause(x: &Ctx) -> String {
    let id = x.id("damageType");
    id.strip_prefix("minecraft:").unwrap_or(id).to_string()
}

/// `DamageCommand.damage`. The attacker of `by`/`from` is not passed on:
/// Rine's damage path takes no attacker.
fn do_damage(x: &mut Ctx, cause: &str) -> CmdResult<i32> {
    let target = x.entity("target")?;
    let amount = x.float("amount");
    let slot = x.g.by_eid.get(&target).copied();
    let applied = match slot {
        Some(s) => interact::damage(x.g, s, amount, cause),
        // non-living entities cannot be hurt
        None => false,
    };
    if !applied {
        return Err(CmdError::key("commands.damage.invulnerable"));
    }
    let name = x.name_of(target);
    x.success(Text::tr("commands.damage.success", vec![lit(java_float(amount)), name]), true);
    Ok(1)
}

/// `RideCommand.mount`. Vanilla `startRiding(vehicle, true, true)` succeeds
/// for any non-player vehicle; Rine has no vehicles, so the checks run and
/// the mount itself is reported as not implemented.
fn mount(x: &mut Ctx, target: i32, vehicle: i32) -> CmdResult<i32> {
    // rideCommandAllowPlayerAsVehicle defaults to false
    if x.g.by_eid.contains_key(&vehicle) {
        return Err(CmdError::key("commands.ride.mount.failure.cant_ride_players"));
    }
    if target == vehicle {
        return Err(CmdError::key("commands.ride.mount.failure.loop"));
    }
    Err(CmdError::new(Text::key("command.failed").on_hover(hover_text(&Text::plain(
        "Rine: /ride mount needs vehicles, which Rine does not have yet",
    )))))
}

/// `SwingCommand.swing`: only living entities (players) swing.
fn swing(x: &mut Ctx, targets: Vec<i32>, hand: i32, anim: u8, dur: i32) -> CmdResult<i32> {
    let mut tracked = Vec::new();
    for &e in &targets {
        if !x.g.by_eid.contains_key(&e) {
            continue;
        }
        send_swing(x.g, e, hand, anim, dur);
        tracked.push(e);
    }
    if tracked.is_empty() {
        return Err(CmdError::key("commands.swing.failed.notliving"));
    }
    let t = if tracked.len() == 1 {
        Text::tr("commands.swing.success.single", vec![x.name_of(tracked[0])])
    } else {
        Text::tr("commands.swing.success.multiple", vec![lit(tracked.len())])
    };
    x.success(t, true);
    Ok(tracked.len() as i32)
}

/// Swing packet to the swinging entity and everyone tracking it.
fn send_swing(g: &mut Game, eid: i32, hand: i32, anim: u8, dur: i32) {
    let f = out::frame(rine_protocol::frame::packet(C_SWING_ANIMATION, |b| {
        b.w_varint(eid);
        b.w_varint(hand);
        b.w_varint(anim as i32);
        b.w_varint(dur);
    }));
    crate::game::track::send_to_trackers(g, eid, &f);
    if let Some(&slot) = g.by_eid.get(&eid) {
        if let Some(p) = g.player(slot) {
            if !p.tracked.contains(&eid) {
                p.out.push(f);
            }
        }
    }
}
