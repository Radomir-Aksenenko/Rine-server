//! /effect, /attribute (`EffectCommands`, `AttributeCommand`) and the
//! mob effect / attribute state of players.
//!
//! Effects keep duration, amplifier and flags, and expire in `tick`.
//! Attributes keep the base value and modifiers per attribute of the
//! player's supplier (`Player.createAttributes`). Server behaviour of the
//! effects is not modelled (see `tick`), and effect attribute modifiers
//! (speed, health boost...) are not applied to attributes.

use crate::game::cmd::{lit, Commands, Ctx};
use crate::game::Game;
use crate::out;
use ahash::AHashMap;
use bytes::{Bytes, BytesMut};
use rine_command::error::java_double;
use rine_command::{CmdError, CmdResult};
use rine_protocol::frame::packet;
use rine_protocol::ids::play::C_UPDATE_ATTRIBUTES;
use rine_protocol::text::Text;
use rine_protocol::Writer;

/// Not in `rine_protocol::ids::play` yet (`vanilla-data/.../packets.json`).
const C_REMOVE_MOB_EFFECT: i32 = 79;
const C_UPDATE_MOB_EFFECT: i32 = 135;

const FLAG_AMBIENT: u8 = 1;
const FLAG_VISIBLE: u8 = 2;
const FLAG_SHOW_ICON: u8 = 4;

/// `MobEffectInstance` without the hidden-effect chain.
#[derive(Clone, Debug)]
pub struct Effect {
    /// `minecraft:speed`.
    pub id: String,
    pub amp: i32,
    /// Ticks left, -1 = infinite.
    pub dur: i32,
    pub ambient: bool,
    pub visible: bool,
    pub show_icon: bool,
}

/// Base value and modifiers of one attribute.
#[derive(Clone, Debug)]
pub struct AttrInst {
    pub base: f64,
    pub mods: Vec<Modifier>,
}

#[derive(Clone, Debug)]
pub struct Modifier {
    pub id: String,
    pub amount: f64,
    /// `AttributeModifier.Operation` id: 0 add_value, 1 multiplied_base, 2 multiplied_total.
    pub op: u8,
}

/// Active effects and attribute modifiers per entity id.
#[derive(Default)]
pub struct State {
    pub effects: AHashMap<i32, Vec<Effect>>,
    /// Indexed like `SPECS`, created on first use.
    pub attrs: AHashMap<i32, Vec<AttrInst>>,
}

/// Attribute of the player supplier: registry path, default, range, and
/// whether `setSyncable(true)` (sent to clients).
struct Spec {
    name: &'static str,
    default: f64,
    min: f64,
    max: f64,
    sync: bool,
}

/// `LivingEntity.createLivingAttributes` plus `Player.createAttributes`
/// (later entries override earlier ones there). Spigot max values.
const SPECS: &[Spec] = &[
    Spec { name: "max_health", default: 20.0, min: 1.0, max: 1024.0, sync: true },
    Spec { name: "knockback_resistance", default: 0.0, min: -2.0, max: 1.0, sync: false },
    Spec { name: "movement_speed", default: 0.1f32 as f64, min: 0.0, max: 1024.0, sync: true },
    Spec { name: "armor", default: 0.0, min: 0.0, max: 30.0, sync: true },
    Spec { name: "armor_toughness", default: 0.0, min: 0.0, max: 20.0, sync: true },
    Spec { name: "max_absorption", default: 0.0, min: 0.0, max: 2048.0, sync: true },
    Spec { name: "step_height", default: 0.6, min: 0.0, max: 10.0, sync: true },
    Spec { name: "scale", default: 1.0, min: 0.0625, max: 16.0, sync: true },
    Spec { name: "gravity", default: 0.08, min: -1.0, max: 1.0, sync: true },
    Spec { name: "safe_fall_distance", default: 3.0, min: -1024.0, max: 1024.0, sync: true },
    Spec { name: "fall_damage_multiplier", default: 1.0, min: 0.0, max: 100.0, sync: true },
    Spec { name: "jump_strength", default: 0.42f32 as f64, min: 0.0, max: 32.0, sync: true },
    Spec { name: "entity_interaction_range", default: 3.0, min: 0.0, max: 64.0, sync: true },
    Spec { name: "oxygen_bonus", default: 0.0, min: 0.0, max: 1024.0, sync: true },
    Spec { name: "burning_time", default: 1.0, min: 0.0, max: 1024.0, sync: true },
    Spec { name: "explosion_knockback_resistance", default: 0.0, min: 0.0, max: 1.0, sync: true },
    Spec { name: "water_movement_efficiency", default: 0.0, min: 0.0, max: 1.0, sync: true },
    Spec { name: "movement_efficiency", default: 0.0, min: 0.0, max: 1.0, sync: true },
    Spec { name: "attack_knockback", default: 0.0, min: 0.0, max: 5.0, sync: false },
    Spec { name: "camera_distance", default: 4.0, min: 0.0, max: 32.0, sync: true },
    Spec { name: "waypoint_transmit_range", default: 6.0E7, min: 0.0, max: 6.0E7, sync: false },
    Spec { name: "bounciness", default: 0.0, min: 0.0, max: 1.0, sync: true },
    Spec { name: "air_drag_modifier", default: 1.0, min: 0.0, max: 2048.0, sync: true },
    Spec { name: "friction_modifier", default: 1.0, min: 0.0, max: 2048.0, sync: true },
    Spec { name: "name_tag_distance", default: 64.0, min: 0.0, max: 512.0, sync: true },
    Spec { name: "below_name_distance", default: 10.0, min: 0.0, max: 512.0, sync: true },
    Spec { name: "attack_damage", default: 1.0, min: 0.0, max: 2048.0, sync: false },
    Spec { name: "attack_speed", default: 4.0, min: 0.0, max: 1024.0, sync: true },
    Spec { name: "luck", default: 0.0, min: -1024.0, max: 1024.0, sync: true },
    Spec { name: "block_interaction_range", default: 4.5, min: 0.0, max: 64.0, sync: true },
    Spec { name: "block_break_speed", default: 1.0, min: 0.0, max: 1024.0, sync: true },
    Spec { name: "submerged_mining_speed", default: 0.2, min: 0.0, max: 20.0, sync: true },
    Spec { name: "sneaking_speed", default: 0.3, min: 0.0, max: 1.0, sync: true },
    Spec { name: "mining_efficiency", default: 0.0, min: 0.0, max: 1024.0, sync: true },
    Spec { name: "sweeping_damage_ratio", default: 0.0, min: 0.0, max: 1.0, sync: true },
    Spec { name: "waypoint_receive_range", default: 6.0E7, min: 0.0, max: 6.0E7, sync: false },
];

pub fn register(c: &mut Commands) {
    c.exec("effect clear", |x| {
        let e = x.source_entity()?;
        clear_all(x, vec![e])
    });
    c.exec("effect clear targets", |x| {
        let e = x.entities("targets")?;
        clear_all(x, e)
    });
    c.exec("effect clear targets effect", |x| {
        let e = x.entities("targets")?;
        let id = x.id("effect").to_string();
        clear_one(x, e, id)
    });
    c.exec("effect give targets effect", |x| give(x, None, 0, true));
    c.exec("effect give targets effect seconds", |x| {
        let s = x.int("seconds");
        give(x, Some(s), 0, true)
    });
    c.exec("effect give targets effect seconds amplifier", |x| {
        let s = x.int("seconds");
        let a = x.int("amplifier");
        give(x, Some(s), a, true)
    });
    c.exec("effect give targets effect seconds amplifier hideParticles", |x| {
        let s = x.int("seconds");
        let a = x.int("amplifier");
        let hide = x.bool("hideParticles");
        give(x, Some(s), a, !hide)
    });
    c.exec("effect give targets effect infinite", |x| give(x, Some(-1), 0, true));
    c.exec("effect give targets effect infinite amplifier", |x| {
        let a = x.int("amplifier");
        give(x, Some(-1), a, true)
    });
    c.exec("effect give targets effect infinite amplifier hideParticles", |x| {
        let a = x.int("amplifier");
        let hide = x.bool("hideParticles");
        give(x, Some(-1), a, !hide)
    });

    c.exec("attribute target attribute get", |x| get_value(x, 1.0));
    c.exec("attribute target attribute get scale", |x| {
        let s = x.double("scale");
        get_value(x, s)
    });
    c.exec("attribute target attribute base get", |x| base_get(x, 1.0));
    c.exec("attribute target attribute base get scale", |x| {
        let s = x.double("scale");
        base_get(x, s)
    });
    c.exec("attribute target attribute base set value", base_set);
    c.exec("attribute target attribute base reset", base_reset);
    c.exec("attribute target attribute modifier add id value add_value", |x| add_mod(x, 0));
    c.exec("attribute target attribute modifier add id value add_multiplied_base", |x| add_mod(x, 1));
    c.exec("attribute target attribute modifier add id value add_multiplied_total", |x| add_mod(x, 2));
    c.exec("attribute target attribute modifier remove id", remove_mod);
    c.exec("attribute target attribute modifier value get id", |x| mod_get(x, 1.0));
    c.exec("attribute target attribute modifier value get id scale", |x| {
        let s = x.double("scale");
        mod_get(x, s)
    });
}

/// Drop the effects and attributes of a player that left.
pub fn on_quit(g: &mut Game, eid: i32) {
    g.cs.effect.effects.remove(&eid);
    g.cs.effect.attrs.remove(&eid);
    super::experience::forget(eid);
}

/// Count effect durations down and remove the expired ones (vanilla
/// `tickEffects`, duration only). Effects with server behaviour
/// (instant health/damage, and the attribute modifiers of speed,
/// slowness, haste, strength, health boost...) are client-side only here.
pub fn tick(g: &mut Game) {
    if g.cs.effect.effects.is_empty() {
        return;
    }
    let mut expired: Vec<(i32, String)> = Vec::new();
    for (&eid, list) in g.cs.effect.effects.iter_mut() {
        list.retain_mut(|e| {
            if e.dur < 0 {
                return true;
            }
            e.dur -= 1;
            if e.dur <= 0 {
                expired.push((eid, e.id.clone()));
                false
            } else {
                true
            }
        });
    }
    g.cs.effect.effects.retain(|_, v| !v.is_empty());
    for (eid, id) in expired {
        let f = out::frame(remove_pkt(eid, holder("minecraft:mob_effect", &id)));
        to_viewers(g, eid, f);
    }
}

// ---- helpers ----

fn holder(registry: &str, id: &str) -> u32 {
    rine_data::get().registries.id(registry, id).unwrap_or(0)
}

/// Frame to the player `eid` itself and to the players that track it.
fn to_viewers(g: &mut Game, eid: i32, f: Bytes) {
    for p in g.players.iter_mut().flatten() {
        if p.eid == eid || p.tracked.contains(&eid) {
            p.out.push(f.clone());
        }
    }
}

fn effect_pkt(eid: i32, holder: u32, e: &Effect) -> BytesMut {
    let mut flags = 0u8;
    if e.ambient {
        flags |= FLAG_AMBIENT;
    }
    if e.visible {
        flags |= FLAG_VISIBLE;
    }
    if e.show_icon {
        flags |= FLAG_SHOW_ICON;
    }
    packet(C_UPDATE_MOB_EFFECT, |b| {
        b.w_varint(eid);
        b.w_varint(holder as i32);
        b.w_varint(e.amp);
        b.w_varint(e.dur);
        b.w_u8(flags);
    })
}

fn remove_pkt(eid: i32, holder: u32) -> BytesMut {
    packet(C_REMOVE_MOB_EFFECT, |b| {
        b.w_varint(eid);
        b.w_varint(holder as i32);
    })
}

/// `ClientboundUpdateAttributesPacket` with one attribute snapshot.
fn attr_pkt(eid: i32, holder: u32, base: f64, mods: &[Modifier]) -> BytesMut {
    packet(C_UPDATE_ATTRIBUTES, |b| {
        b.w_varint(eid);
        b.w_varint(1);
        b.w_varint(holder as i32);
        b.w_f64(base);
        b.w_varint(mods.len() as i32);
        for m in mods {
            b.w_str(&m.id);
            b.w_f64(m.amount);
            b.w_varint(m.op as i32);
        }
    })
}

fn effect_display(id: &str) -> Text {
    Text::key(format!("effect.{}", id.replace(':', ".")))
}

fn is_instant(id: &str) -> bool {
    id == "minecraft:instant_health" || id == "minecraft:instant_damage"
}

/// `EffectCommands.computeDurationInTicks`.
fn duration_ticks(seconds: Option<i32>, instant: bool) -> i32 {
    match seconds {
        Some(s) if instant => s,
        Some(-1) => -1,
        Some(s) => s * 20,
        None if instant => 1,
        None => 600,
    }
}

/// `MobEffectInstance.isShorterDurationThan`.
fn shorter(this: i32, other: i32) -> bool {
    this != -1 && (this < other || other == -1)
}

/// `MobEffectInstance.update` (hidden effects are not kept). True when
/// anything changed.
fn update(old: &mut Effect, new: &Effect) -> bool {
    let mut changed = false;
    if new.amp > old.amp {
        old.amp = new.amp;
        old.dur = new.dur;
        changed = true;
    } else if shorter(old.dur, new.dur) && new.amp == old.amp {
        old.dur = new.dur;
        changed = true;
    }
    if (!new.ambient && old.ambient) || changed {
        old.ambient = new.ambient;
        changed = true;
    }
    if new.visible != old.visible {
        old.visible = new.visible;
        changed = true;
    }
    if new.show_icon != old.show_icon {
        old.show_icon = new.show_icon;
        changed = true;
    }
    changed
}

/// `LivingEntity.addEffect` for a player. False when nothing changed.
fn add_effect(g: &mut Game, eid: i32, new: Effect) -> bool {
    let list = g.cs.effect.effects.entry(eid).or_default();
    let (changed, stored) = match list.iter().position(|e| e.id == new.id) {
        Some(i) => {
            let c = update(&mut list[i], &new);
            (c, list[i].clone())
        }
        None => {
            list.push(new);
            (true, list[list.len() - 1].clone())
        }
    };
    if changed {
        let f = out::frame(effect_pkt(eid, holder("minecraft:mob_effect", &stored.id), &stored));
        to_viewers(g, eid, f);
    }
    changed
}

/// Remove one effect; sends the removal to the viewers.
fn remove_one(g: &mut Game, eid: i32, id: &str) -> bool {
    let Some(list) = g.cs.effect.effects.get_mut(&eid) else { return false };
    let Some(i) = list.iter().position(|e| e.id == id) else { return false };
    list.remove(i);
    let f = out::frame(remove_pkt(eid, holder("minecraft:mob_effect", id)));
    to_viewers(g, eid, f);
    true
}

fn remove_all(g: &mut Game, eid: i32) -> bool {
    let Some(list) = g.cs.effect.effects.remove(&eid) else { return false };
    for e in &list {
        let f = out::frame(remove_pkt(eid, holder("minecraft:mob_effect", &e.id)));
        to_viewers(g, eid, f);
    }
    !list.is_empty()
}

/// Entities that are players (the only living entities here).
fn living(g: &Game, eid: i32) -> bool {
    g.by_eid.contains_key(&eid)
}

fn clear_all(x: &mut Ctx, eids: Vec<i32>) -> CmdResult<i32> {
    let mut hit = Vec::new();
    for eid in eids {
        if living(x.g, eid) && remove_all(x.g, eid) {
            hit.push(eid);
        }
    }
    if hit.is_empty() {
        return Err(CmdError::key("commands.effect.clear.everything.failed"));
    }
    let n = hit.len() as i32;
    let msg = if n == 1 {
        Text::tr("commands.effect.clear.everything.success.single", vec![x.name_of(hit[0])])
    } else {
        Text::tr("commands.effect.clear.everything.success.multiple", vec![lit(n)])
    };
    x.success(msg, true);
    Ok(n)
}

fn clear_one(x: &mut Ctx, eids: Vec<i32>, id: String) -> CmdResult<i32> {
    let mut hit = Vec::new();
    for eid in eids {
        if living(x.g, eid) && remove_one(x.g, eid, &id) {
            hit.push(eid);
        }
    }
    if hit.is_empty() {
        return Err(CmdError::key("commands.effect.clear.specific.failed"));
    }
    let n = hit.len() as i32;
    let display = effect_display(&id);
    let msg = if n == 1 {
        Text::tr("commands.effect.clear.specific.success.single", vec![display, x.name_of(hit[0])])
    } else {
        Text::tr("commands.effect.clear.specific.success.multiple", vec![display, lit(n)])
    };
    x.success(msg, true);
    Ok(n)
}

/// `EffectCommands.giveEffect`.
fn give(x: &mut Ctx, seconds: Option<i32>, amp: i32, particles: bool) -> CmdResult<i32> {
    let eids = x.entities("targets")?;
    let id = x.id("effect").to_string();
    let duration = duration_ticks(seconds, is_instant(&id));
    let mut hit = Vec::new();
    for eid in eids {
        if !living(x.g, eid) {
            continue;
        }
        let e = Effect { id: id.clone(), amp, dur: duration, ambient: false, visible: particles, show_icon: particles };
        if add_effect(x.g, eid, e) {
            hit.push(eid);
        }
    }
    if hit.is_empty() {
        return Err(CmdError::key("commands.effect.give.failed"));
    }
    let n = hit.len() as i32;
    let display = effect_display(&id);
    let secs = lit(duration / 20);
    let msg = if n == 1 {
        Text::tr("commands.effect.give.success.single", vec![display, x.name_of(hit[0]), secs])
    } else {
        Text::tr("commands.effect.give.success.multiple", vec![display, lit(n), secs])
    };
    x.success(msg, true);
    Ok(n)
}

// ---- attributes ----

fn spec_index(attr: &str) -> Option<usize> {
    let path = attr.strip_prefix("minecraft:")?;
    SPECS.iter().position(|s| s.name == path)
}

fn attr_desc(attr: &str) -> Text {
    let path = attr.strip_prefix("minecraft:").unwrap_or(attr);
    Text::key(format!("attribute.name.{path}"))
}

fn attr_holder(idx: usize) -> u32 {
    holder("minecraft:attribute", &format!("minecraft:{}", SPECS[idx].name))
}

/// Attribute instances of an entity, created at the defaults.
fn attrs_mut(g: &mut Game, eid: i32) -> &mut Vec<AttrInst> {
    g.cs.effect.attrs.entry(eid).or_insert_with(|| SPECS.iter().map(|s| AttrInst { base: s.default, mods: Vec::new() }).collect())
}

/// `AttributeInstance.calculateValue`, clamped to the attribute range.
fn calc(idx: usize, inst: &AttrInst) -> f64 {
    let mut base = inst.base;
    for m in inst.mods.iter().filter(|m| m.op == 0) {
        base += m.amount;
    }
    let mut result = base;
    for m in inst.mods.iter().filter(|m| m.op == 1) {
        result += base * m.amount;
    }
    for m in inst.mods.iter().filter(|m| m.op == 2) {
        result *= 1.0 + m.amount;
    }
    let s = &SPECS[idx];
    if result.is_nan() {
        s.min
    } else {
        result.max(s.min).min(s.max)
    }
}

/// `getLivingEntity` then `getEntityWithAttribute`: attribute index.
fn need_attr(x: &Ctx, eid: i32, attr: &str) -> CmdResult<usize> {
    if !living(x.g, eid) {
        return Err(CmdError::tr("commands.attribute.failed.entity", vec![x.name_of(eid)]));
    }
    spec_index(attr).ok_or_else(|| CmdError::tr("commands.attribute.failed.no_attribute", vec![x.name_of(eid), attr_desc(attr)]))
}

/// Target entity, attribute name and index, as the executors need them.
fn target_attr(x: &mut Ctx) -> CmdResult<(i32, String, usize)> {
    let eid = x.entity("target")?;
    let attr = x.id("attribute").to_string();
    let idx = need_attr(x, eid, &attr)?;
    Ok((eid, attr, idx))
}

/// Send the attribute to the viewers when clients see it.
fn sync_attr(g: &mut Game, eid: i32, idx: usize) {
    if !SPECS[idx].sync {
        return;
    }
    let inst = attrs_mut(g, eid)[idx].clone();
    let f = out::frame(attr_pkt(eid, attr_holder(idx), inst.base, &inst.mods));
    to_viewers(g, eid, f);
}

fn get_value(x: &mut Ctx, scale: f64) -> CmdResult<i32> {
    let (eid, attr, idx) = target_attr(x)?;
    let v = calc(idx, &attrs_mut(x.g, eid)[idx]);
    let msg = Text::tr("commands.attribute.value.get.success", vec![attr_desc(&attr), x.name_of(eid), lit(java_double(v))]);
    x.success(msg, false);
    Ok((v * scale) as i32)
}

fn base_get(x: &mut Ctx, scale: f64) -> CmdResult<i32> {
    let (eid, attr, idx) = target_attr(x)?;
    let v = attrs_mut(x.g, eid)[idx].base;
    let msg = Text::tr("commands.attribute.base_value.get.success", vec![attr_desc(&attr), x.name_of(eid), lit(java_double(v))]);
    x.success(msg, false);
    Ok((v * scale) as i32)
}

fn base_set(x: &mut Ctx) -> CmdResult<i32> {
    let (eid, attr, idx) = target_attr(x)?;
    let v = x.double("value");
    let changed = {
        let inst = &mut attrs_mut(x.g, eid)[idx];
        let differs = inst.base != v;
        inst.base = v;
        differs
    };
    if changed {
        sync_attr(x.g, eid, idx);
    }
    let msg = Text::tr("commands.attribute.base_value.set.success", vec![attr_desc(&attr), x.name_of(eid), lit(java_double(v))]);
    x.success(msg, false);
    Ok(1)
}

fn base_reset(x: &mut Ctx) -> CmdResult<i32> {
    let (eid, attr, idx) = target_attr(x)?;
    let def = SPECS[idx].default;
    let changed = {
        let inst = &mut attrs_mut(x.g, eid)[idx];
        let differs = inst.base != def;
        inst.base = def;
        differs
    };
    if changed {
        sync_attr(x.g, eid, idx);
    }
    let msg = Text::tr("commands.attribute.base_value.reset.success", vec![attr_desc(&attr), x.name_of(eid), lit(java_double(def))]);
    x.success(msg, false);
    Ok(1)
}

fn add_mod(x: &mut Ctx, op: u8) -> CmdResult<i32> {
    let (eid, attr, idx) = target_attr(x)?;
    let id = x.id("id").to_string();
    let amount = x.double("value");
    if attrs_mut(x.g, eid)[idx].mods.iter().any(|m| m.id == id) {
        return Err(CmdError::tr(
            "commands.attribute.failed.modifier_already_present",
            vec![Text::plain(id), attr_desc(&attr), x.name_of(eid)],
        ));
    }
    attrs_mut(x.g, eid)[idx].mods.push(Modifier { id: id.clone(), amount, op });
    sync_attr(x.g, eid, idx);
    let msg = Text::tr("commands.attribute.modifier.add.success", vec![Text::plain(id), attr_desc(&attr), x.name_of(eid)]);
    x.success(msg, false);
    Ok(1)
}

fn remove_mod(x: &mut Ctx) -> CmdResult<i32> {
    let (eid, attr, idx) = target_attr(x)?;
    let id = x.id("id").to_string();
    let removed = {
        let mods = &mut attrs_mut(x.g, eid)[idx].mods;
        let before = mods.len();
        mods.retain(|m| m.id != id);
        mods.len() != before
    };
    if !removed {
        return Err(CmdError::tr("commands.attribute.failed.no_modifier", vec![attr_desc(&attr), x.name_of(eid), Text::plain(id)]));
    }
    sync_attr(x.g, eid, idx);
    let msg = Text::tr("commands.attribute.modifier.remove.success", vec![Text::plain(id), attr_desc(&attr), x.name_of(eid)]);
    x.success(msg, false);
    Ok(1)
}

fn mod_get(x: &mut Ctx, scale: f64) -> CmdResult<i32> {
    let (eid, attr, idx) = target_attr(x)?;
    let id = x.id("id").to_string();
    let found = attrs_mut(x.g, eid)[idx].mods.iter().find(|m| m.id == id).map(|m| m.amount);
    let Some(v) = found else {
        return Err(CmdError::tr("commands.attribute.failed.no_modifier", vec![attr_desc(&attr), x.name_of(eid), Text::plain(id)]));
    };
    let msg = Text::tr(
        "commands.attribute.modifier.value.get.success",
        vec![Text::plain(id), attr_desc(&attr), x.name_of(eid), lit(java_double(v))],
    );
    x.success(msg, false);
    Ok((v * scale) as i32)
}
