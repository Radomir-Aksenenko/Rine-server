//! /playsound, /stopsound, /particle (`PlaySoundCommand`, `StopSoundCommand`,
//! `ParticleCommand`).

use crate::game::cmd::commands::random::next_u32;
use crate::game::cmd::{lit, Commands, Ctx};
use crate::out;
use bytes::BytesMut;
use rine_command::args::misc::ParticleArg;
use rine_command::args::Arg;
use rine_command::{CmdError, CmdResult};
use rine_protocol::frame::packet;
use rine_protocol::ids::play::{C_LEVEL_PARTICLES, C_SOUND};
use rine_protocol::nbt::Tag;
use rine_protocol::text::Text;
use rine_protocol::Writer;

// Clientbound play id (packets.json). Not in `rine_protocol::ids::play` yet.
const C_STOP_SOUND: i32 = 121;

/// `SoundSource` in ordinal order (the wire id is the index).
const CATEGORIES: [&str; 11] = ["master", "music", "record", "weather", "block", "hostile", "neutral", "player", "ambient", "voice", "ui"];

fn category(name: &str) -> Option<u8> {
    CATEGORIES.iter().position(|c| *c == name).map(|i| i as u8)
}

/// Particle types without options (`register(name, bool)` in `ParticleTypes`).
/// Types with a codec (dust, block, item, ...) are not encoded yet.
const SIMPLE_PARTICLES: &[&str] = &[
    "angry_villager", "bubble", "sulfur_bubbles", "noxious_gas", "noxious_gas_cloud", "cloud",
    "copper_fire_flame", "crit", "damage_indicator", "dripping_lava", "falling_lava",
    "landing_lava", "dripping_water", "falling_water", "elder_guardian", "enchanted_hit",
    "enchant", "end_rod", "explosion_emitter", "explosion", "gust", "small_gust",
    "gust_emitter_large", "gust_emitter_small", "sonic_boom", "firework", "fishing", "flame",
    "infested", "cherry_leaves", "pale_oak_leaves", "red_poplar_leaves", "orange_poplar_leaves",
    "yellow_poplar_leaves", "sculk_soul", "sculk_charge_pop", "soul_fire_flame", "soul",
    "happy_villager", "composter", "heart", "pause_mob_growth", "reset_mob_growth",
    "item_slime", "item_cobweb", "item_snowball", "large_smoke", "lava", "mycelium", "note",
    "poof", "portal", "rain", "smoke", "white_smoke", "sneeze", "spit", "squid_ink",
    "sweep_attack", "totem_of_undying", "underwater", "splash", "witch", "bubble_pop",
    "current_down", "bubble_column_up", "nautilus", "dolphin", "campfire_cosy_smoke",
    "campfire_signal_smoke", "dripping_honey", "falling_honey", "landing_honey",
    "falling_nectar", "falling_spore_blossom", "ash", "crimson_spore", "warped_spore",
    "spore_blossom_air", "dripping_obsidian_tear", "falling_obsidian_tear",
    "landing_obsidian_tear", "reverse_portal", "white_ash", "small_flame", "snowflake",
    "dripping_dripstone_lava", "falling_dripstone_lava", "dripping_dripstone_water",
    "falling_dripstone_water", "glow_squid_ink", "glow", "wax_on", "wax_off", "electric_spark",
    "scrape", "egg_crack", "dust_plume", "trial_spawner_detection",
    "trial_spawner_detection_ominous", "vault_connection", "ominous_spawning", "raid_omen",
    "trial_omen", "firefly", "sulfur_cube_goo",
];

pub fn register(c: &mut Commands) {
    // playsound
    c.exec("playsound sound", play_default);
    for s in CATEGORIES {
        c.exec(&format!("playsound sound {s}"), play_in);
        c.exec(&format!("playsound sound {s} targets"), play_targets);
        c.exec(&format!("playsound sound {s} targets pos"), play_pos);
        c.exec(&format!("playsound sound {s} targets pos volume"), play_volume);
        c.exec(&format!("playsound sound {s} targets pos volume pitch"), play_pitch);
        c.exec(&format!("playsound sound {s} targets pos volume pitch minVolume"), play_min_volume);
    }

    // stopsound
    c.exec("stopsound targets", stop_all);
    c.exec("stopsound targets * sound", stop_source);
    for s in CATEGORIES {
        c.exec(&format!("stopsound targets {s}"), stop_source);
        c.exec(&format!("stopsound targets {s} sound"), stop_source);
    }

    // particle
    c.exec("particle name", particle_plain);
    c.exec("particle name pos", particle_at);
    c.exec("particle name pos delta speed count", |x| particle_full(x, false));
    c.exec("particle name pos delta speed count force", |x| particle_full(x, true));
    c.exec("particle name pos delta speed count force viewers", |x| particle_full(x, true));
    c.exec("particle name pos delta speed count normal", |x| particle_full(x, false));
    c.exec("particle name pos delta speed count normal viewers", |x| particle_full(x, false));
}

// ---- playsound -----------------------------------------------------------

/// `getCallingPlayerAsCollection`: the executing player, if any.
fn calling_player(x: &Ctx) -> Vec<usize> {
    x.source_player().map(|s| vec![s]).unwrap_or_default()
}

/// Sound source of the `playsound`/`stopsound` literal at index 2.
fn category_of(x: &Ctx) -> u8 {
    category(x.lit(2)).unwrap_or(0)
}

fn play_default(x: &mut Ctx) -> CmdResult<i32> {
    let own = calling_player(x);
    let pos = x.src.pos;
    play(x, &own, 0, pos, 1.0, 1.0, 0.0)
}

fn play_in(x: &mut Ctx) -> CmdResult<i32> {
    let own = calling_player(x);
    let cat = category_of(x);
    let pos = x.src.pos;
    play(x, &own, cat, pos, 1.0, 1.0, 0.0)
}

fn play_targets(x: &mut Ctx) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    let cat = category_of(x);
    let pos = x.src.pos;
    play(x, &slots, cat, pos, 1.0, 1.0, 0.0)
}

fn play_pos(x: &mut Ctx) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    let cat = category_of(x);
    let pos = x.vec3("pos");
    play(x, &slots, cat, pos, 1.0, 1.0, 0.0)
}

fn play_volume(x: &mut Ctx) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    let cat = category_of(x);
    let pos = x.vec3("pos");
    let volume = x.float("volume");
    play(x, &slots, cat, pos, volume, 1.0, 0.0)
}

fn play_pitch(x: &mut Ctx) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    let cat = category_of(x);
    let pos = x.vec3("pos");
    let volume = x.float("volume");
    let pitch = x.float("pitch");
    play(x, &slots, cat, pos, volume, pitch, 0.0)
}

fn play_min_volume(x: &mut Ctx) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    let cat = category_of(x);
    let pos = x.vec3("pos");
    let volume = x.float("volume");
    let pitch = x.float("pitch");
    let min_volume = x.float("minVolume");
    play(x, &slots, cat, pos, volume, pitch, min_volume)
}

/// `PlaySoundCommand.playSound`: each player inside the sound's range hears it
/// at `pos`; a player beyond the range hears `minVolume` two blocks away from
/// themselves, towards the source, when `minVolume` is positive.
fn play(x: &mut Ctx, slots: &[usize], category: u8, pos: [f64; 3], volume: f32, pitch: f32, min_volume: f32) -> CmdResult<i32> {
    let sound = x.id("sound").to_string();
    // `SoundEvent.getRange` for a variable-range event, squared in float.
    let range = if volume > 1.0 { 16.0 * volume } else { 16.0 };
    let max_sqr = (range * range) as f64;
    // The seed is drawn once, before the loop, even when nobody hears it.
    let seed = (((next_u32() as u64) << 32) | next_u32() as u64) as i64;

    let mut sent = Vec::with_capacity(slots.len());
    for &slot in slots {
        // Every player is in the overworld: Rine has one level.
        let Some(player) = x.g.players.get(slot).and_then(|p| p.as_ref()) else { continue };
        let at = player.pos;
        let (dx, dy, dz) = (pos[0] - at[0], pos[1] - at[1], pos[2] - at[2]);
        let dist_sqr = dx * dx + dy * dy + dz * dz;
        let (loc, vol) = if dist_sqr > max_sqr {
            if min_volume <= 0.0 {
                continue;
            }
            let d = dist_sqr.sqrt();
            ([at[0] + dx / d * 2.0, at[1] + dy / d * 2.0, at[2] + dz / d * 2.0], min_volume)
        } else {
            (pos, volume)
        };
        if let Some(p) = x.g.player(slot) {
            p.send(sound_packet(&sound, category, loc, vol, pitch, seed));
        }
        sent.push(slot);
    }

    if sent.is_empty() {
        return Err(CmdError::key("commands.playsound.failed"));
    }
    let n = sent.len();
    let t = if n == 1 {
        Text::tr("commands.playsound.success.single", vec![lit(&sound), x.player_name(sent[0])])
    } else {
        Text::tr("commands.playsound.success.multiple", vec![lit(&sound), lit(n)])
    };
    x.success(t, true);
    Ok(n as i32)
}

/// `ClientboundSoundPacket` with an inline sound event (`Holder.direct`),
/// variable range.
fn sound_packet(sound: &str, category: u8, pos: [f64; 3], volume: f32, pitch: f32, seed: i64) -> BytesMut {
    packet(C_SOUND, |b| {
        b.w_varint(0); // holder: direct sound event
        b.w_str(sound);
        b.w_bool(false); // no fixed range
        b.w_varint(category as i32);
        // LOCATION_ACCURACY: positions are sent as 1/8 block
        b.w_i32((pos[0] * 8.0) as i32);
        b.w_i32((pos[1] * 8.0) as i32);
        b.w_i32((pos[2] * 8.0) as i32);
        b.w_f32(volume);
        b.w_f32(pitch);
        b.w_i64(seed);
    })
}

// ---- stopsound -----------------------------------------------------------

fn stop_all(x: &mut Ctx) -> CmdResult<i32> {
    stop(x, None, None)
}

/// `stopsound <targets> <source> [sound]` and `stopsound <targets> * <sound>`.
fn stop_source(x: &mut Ctx) -> CmdResult<i32> {
    let source = if x.lit(2) == "*" { None } else { category(x.lit(2)) };
    let sound = if x.has("sound") { Some(x.id("sound").to_string()) } else { None };
    stop(x, source, sound)
}

/// `StopSoundCommand.stopSound`.
fn stop(x: &mut Ctx, source: Option<u8>, sound: Option<String>) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    let f = out::frame(stop_packet(source, sound.as_deref()));
    for &slot in &slots {
        if let Some(p) = x.g.player(slot) {
            p.send_frame(f.clone());
        }
    }
    let t = match (source, &sound) {
        (Some(s), Some(n)) => Text::tr("commands.stopsound.success.source.sound", vec![lit(n), lit(CATEGORIES[s as usize])]),
        (Some(s), None) => Text::tr("commands.stopsound.success.source.any", vec![lit(CATEGORIES[s as usize])]),
        (None, Some(n)) => Text::tr("commands.stopsound.success.sourceless.sound", vec![lit(n)]),
        (None, None) => Text::key("commands.stopsound.success.sourceless.any"),
    };
    x.success(t, true);
    Ok(slots.len() as i32)
}

/// `ClientboundStopSoundPacket`: flags (1 = source, 2 = sound), then the fields.
fn stop_packet(source: Option<u8>, sound: Option<&str>) -> BytesMut {
    packet(C_STOP_SOUND, |b| {
        let flags = (source.is_some() as u8) | ((sound.is_some() as u8) << 1);
        b.w_u8(flags);
        if let Some(s) = source {
            b.w_varint(s as i32);
        }
        if let Some(n) = sound {
            b.w_str(n);
        }
    })
}

// ---- particle ------------------------------------------------------------

fn particle_plain(x: &mut Ctx) -> CmdResult<i32> {
    let pos = x.src.pos;
    emit(x, pos, [0.0; 3], 0.0, 0, false, None)
}

fn particle_at(x: &mut Ctx) -> CmdResult<i32> {
    let pos = x.vec3("pos");
    emit(x, pos, [0.0; 3], 0.0, 0, false, None)
}

/// `/particle <name> <pos> <delta> <speed> <count> [force|normal] [viewers]`.
fn particle_full(x: &mut Ctx, force: bool) -> CmdResult<i32> {
    let pos = x.vec3("pos");
    let delta = x.vec3("delta");
    let speed = x.float("speed");
    let count = x.int("count");
    let viewers = if x.has("viewers") { Some(x.players("viewers")?) } else { None };
    emit(x, pos, delta, speed, count, force, viewers)
}

/// `ParticleCommand.sendParticles`: sends to the viewers that are close
/// enough (512 blocks when forced, 32 otherwise), fails when none was.
fn emit(
    x: &mut Ctx,
    pos: [f64; 3],
    delta: [f64; 3],
    speed: f32,
    count: i32,
    force: bool,
    viewers: Option<Vec<usize>>,
) -> CmdResult<i32> {
    let (type_id, name) = match x.arg("name") {
        Arg::Particle(p) => (particle_type(p)?, p.id.clone()),
        _ => return Err(CmdError::key("commands.particle.failed")),
    };
    let players = viewers.unwrap_or_else(|| x.g.players.iter().enumerate().filter_map(|(i, p)| p.as_ref().map(|_| i)).collect());

    let f = out::frame(level_particles(type_id, force, pos, delta, speed, count));
    let radius = if force { 512.0 } else { 32.0 };
    let mut n = 0;
    for slot in players {
        let Some(p) = x.g.players.get(slot).and_then(|p| p.as_ref()) else { continue };
        // `BlockPos.closerToCenterThan`: the player's block centre must be in range
        let (bx, by, bz) = (p.pos[0].floor() + 0.5, p.pos[1].floor() + 0.5, p.pos[2].floor() + 0.5);
        let (dx, dy, dz) = (bx - pos[0], by - pos[1], bz - pos[2]);
        if dx * dx + dy * dy + dz * dz < radius * radius {
            if let Some(pl) = x.g.player(slot) {
                pl.send_frame(f.clone());
                n += 1;
            }
        }
    }

    if n == 0 {
        return Err(CmdError::key("commands.particle.failed"));
    }
    x.success(Text::tr("commands.particle.success", vec![lit(&name)]), true);
    Ok(n)
}

/// Registry id of a particle type. Only types without options are encoded;
/// the others fail with `commands.particle.failed` until their codecs exist.
fn particle_type(p: &ParticleArg) -> CmdResult<u32> {
    let has_options = p.options.as_ref().is_some_and(|t| !matches!(t, Tag::Compound(v) if v.is_empty()));
    let short = p.id.strip_prefix("minecraft:").unwrap_or(&p.id);
    if has_options || !SIMPLE_PARTICLES.contains(&short) {
        return Err(CmdError::key("commands.particle.failed"));
    }
    rine_data::get().registries.id("minecraft:particle_type", &p.id).ok_or_else(|| CmdError::key("commands.particle.failed"))
}

/// `ClientboundLevelParticlesPacket` with `RandomizationType.DEFAULT`.
fn level_particles(type_id: u32, force: bool, pos: [f64; 3], delta: [f64; 3], speed: f32, count: i32) -> BytesMut {
    packet(C_LEVEL_PARTICLES, |b| {
        b.w_varint(type_id as i32);
        b.w_bool(force); // overrideLimiter
        b.w_bool(false); // alwaysShow
        b.w_f64(pos[0]);
        b.w_f64(pos[1]);
        b.w_f64(pos[2]);
        b.w_f32(delta[0] as f32);
        b.w_f32(delta[1] as f32);
        b.w_f32(delta[2] as f32);
        // the max speed is written once per axis
        b.w_f32(speed);
        b.w_f32(speed);
        b.w_f32(speed);
        b.w_varint(count);
        b.w_varint(0);
    })
}
