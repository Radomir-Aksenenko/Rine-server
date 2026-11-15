//! /say, /me, /msg /tell /w, /tellraw, /title (`SayCommand`, `EmoteCommands`,
//! `MsgCommand`, `TellRawCommand`, `TitleCommand`).

use crate::game::cmd::{feedback, lit, render_plain, select, Commands, Ctx};
use crate::game::Game;
use crate::out;
use bytes::BytesMut;
use rine_command::CmdResult;
use rine_protocol::frame::packet;
use rine_protocol::text::Text;
use rine_protocol::Writer;

// Clientbound play ids (packets.json). Not in `rine_protocol::ids::play` yet.
const C_CLEAR_TITLES: i32 = 14;
const C_DISGUISED_CHAT: i32 = 33;
const C_SET_SUBTITLE_TEXT: i32 = 114;
const C_SET_TITLE_TEXT: i32 = 116;
const C_SET_TITLES_ANIMATION: i32 = 117;

/// A vanilla `ChatType` as Rine sends it: its registry key, translation key
/// and the decoration's parameters and style (`ChatTypeDecoration`).
#[derive(Clone, Copy)]
struct Kind {
    registry: &'static str,
    key: &'static str,
    /// `TARGET` + `CONTENT` instead of `SENDER` + `CONTENT`.
    target_param: bool,
    /// Gray italic style of the direct message decorations.
    gray_italic: bool,
}

const SAY: Kind = Kind { registry: "minecraft:say_command", key: "chat.type.announcement", target_param: false, gray_italic: false };
const EMOTE: Kind = Kind { registry: "minecraft:emote_command", key: "chat.type.emote", target_param: false, gray_italic: false };
const MSG_IN: Kind =
    Kind { registry: "minecraft:msg_command_incoming", key: "commands.message.display.incoming", target_param: false, gray_italic: true };
const MSG_OUT: Kind =
    Kind { registry: "minecraft:msg_command_outgoing", key: "commands.message.display.outgoing", target_param: true, gray_italic: true };

/// `ChatType.decorate`: the translated line that consoles and logs see.
fn decorate(k: Kind, name: &Text, target: Option<&Text>, content: &Text) -> Text {
    let first = if k.target_param { target.cloned().unwrap_or_else(Text::empty) } else { name.clone() };
    let t = Text::tr(k.key, vec![first, content.clone()]);
    if k.gray_italic {
        t.color("gray").italic()
    } else {
        t
    }
}

/// `ClientboundDisguisedChatPacket`: unsigned content with its bound chat type
/// (holder reference, sender name, optional target name).
fn disguised(k: Kind, name: &Text, target: Option<&Text>, content: &Text) -> Option<BytesMut> {
    let id = rine_data::get().registries.synced("minecraft:chat_type").id(k.registry)?;
    Some(packet(C_DISGUISED_CHAT, |b| {
        out::write_text(b, content);
        b.w_varint(id as i32 + 1);
        out::write_text(b, name);
        match target {
            Some(t) => {
                b.w_bool(true);
                out::write_text(b, t);
            }
            None => b.w_bool(false),
        }
    }))
}

/// `PlayerList.broadcastChatMessage` for an unsigned message: log it, then send
/// it to every player.
fn broadcast(g: &mut Game, k: Kind, name: &Text, content: &Text) {
    // Unsigned messages are logged with the "Not Secure" tag, as in vanilla.
    tracing::info!("[Not Secure] {}", render_plain(&decorate(k, name, None, content)));
    if let Some(p) = disguised(k, name, None, content) {
        g.broadcast.push(out::frame(p));
    }
}

/// `CommandSourceStack.sendChatMessage`: a player gets the chat packet, any
/// other source gets the decorated line as a system message.
fn send_to_source(x: &mut Ctx, k: Kind, name: &Text, target: Option<&Text>, content: &Text) {
    if x.src.silent {
        return;
    }
    match x.source_player() {
        Ok(slot) => {
            if let Some(p) = disguised(k, name, target, content) {
                if let Some(pl) = x.g.player(slot) {
                    pl.send(p);
                }
            }
        }
        Err(_) => feedback::send_system(x.g, x.src, &decorate(k, name, target, content)),
    }
}

fn say(x: &mut Ctx) -> CmdResult<i32> {
    let msg = x.message("message")?;
    broadcast(x.g, SAY, &x.src.display, &msg);
    Ok(1)
}

fn emote(x: &mut Ctx) -> CmdResult<i32> {
    let msg = x.message("action")?;
    broadcast(x.g, EMOTE, &x.src.display, &msg);
    Ok(1)
}

/// `MsgCommand.sendMessage`: per target, the outgoing line to the sender and
/// the incoming line to the target. Returns the number of targets.
fn msg(x: &mut Ctx) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    let content = x.message("message")?;
    let src = x.src;
    for &slot in &slots {
        let target = x.player_name(slot);
        send_to_source(x, MSG_OUT, &src.display, Some(&target), &content);
        if let Some(p) = disguised(MSG_IN, &src.display, None, &content) {
            if let Some(pl) = x.g.player(slot) {
                pl.send(p);
            }
        }
    }
    Ok(slots.len() as i32)
}

/// `TellRawCommand`: the component is resolved for each target.
fn tellraw(x: &mut Ctx) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    let raw = select::text_from_tag(x.nbt("message"));
    for &slot in &slots {
        let Some(eid) = x.g.players[slot].as_ref().map(|p| p.eid) else { continue };
        let t = select::resolve_text(x.g, x.src, &raw, Some(eid), 0)?;
        if let Some(p) = x.g.player(slot) {
            p.msg(&t);
        }
    }
    Ok(slots.len() as i32)
}

#[derive(Clone, Copy)]
enum Shown {
    Title,
    Subtitle,
    ActionBar,
}

/// `CommandResponseTracker.sendFeedback` with one tracked player per target:
/// the single-player message names the player, otherwise the count is shown.
fn feedback_players(x: &mut Ctx, slots: &[usize], single: &'static str, multiple: &'static str) -> i32 {
    let n = slots.len();
    let t = if n == 1 { Text::tr(single, vec![x.player_name(slots[0])]) } else { Text::tr(multiple, vec![lit(n)]) };
    x.success(t, true);
    n as i32
}

/// Send one frame to every player in `slots`.
fn send_all(x: &mut Ctx, slots: &[usize], pk: BytesMut) {
    let f = out::frame(pk);
    for &slot in slots {
        if let Some(p) = x.g.player(slot) {
            p.send_frame(f.clone());
        }
    }
}

/// `/title <targets> title|subtitle|actionbar <text>`.
fn show(x: &mut Ctx, what: Shown) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    let raw = select::text_from_tag(x.nbt("title"));
    for &slot in &slots {
        let Some(eid) = x.g.players[slot].as_ref().map(|p| p.eid) else { continue };
        let t = select::resolve_text(x.g, x.src, &raw, Some(eid), 0)?;
        let pk = match what {
            Shown::Title => packet(C_SET_TITLE_TEXT, |b| out::write_text(b, &t)),
            Shown::Subtitle => packet(C_SET_SUBTITLE_TEXT, |b| out::write_text(b, &t)),
            Shown::ActionBar => out::action_bar(&t),
        };
        if let Some(p) = x.g.player(slot) {
            p.send(pk);
        }
    }
    let (single, multiple) = match what {
        Shown::Title => ("commands.title.show.title.single", "commands.title.show.title.multiple"),
        Shown::Subtitle => ("commands.title.show.subtitle.single", "commands.title.show.subtitle.multiple"),
        Shown::ActionBar => ("commands.title.show.actionbar.single", "commands.title.show.actionbar.multiple"),
    };
    Ok(feedback_players(x, &slots, single, multiple))
}

/// `/title <targets> clear` (`reset` = false) and `reset` (`reset` = true).
fn clear_titles(x: &mut Ctx, reset: bool) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    send_all(x, &slots, packet(C_CLEAR_TITLES, |b| b.w_bool(reset)));
    let (single, multiple) = if reset {
        ("commands.title.reset.single", "commands.title.reset.multiple")
    } else {
        ("commands.title.cleared.single", "commands.title.cleared.multiple")
    };
    Ok(feedback_players(x, &slots, single, multiple))
}

fn clear(x: &mut Ctx) -> CmdResult<i32> {
    clear_titles(x, false)
}

fn reset(x: &mut Ctx) -> CmdResult<i32> {
    clear_titles(x, true)
}

/// `/title <targets> times <fadeIn> <stay> <fadeOut>`.
fn times(x: &mut Ctx) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    let (fade_in, stay, fade_out) = (x.int("fadeIn"), x.int("stay"), x.int("fadeOut"));
    send_all(x, &slots, packet(C_SET_TITLES_ANIMATION, |b| {
        b.w_i32(fade_in);
        b.w_i32(stay);
        b.w_i32(fade_out);
    }));
    Ok(feedback_players(x, &slots, "commands.title.times.single", "commands.title.times.multiple"))
}

pub fn register(c: &mut Commands) {
    c.exec("say message", say);
    c.exec("me action", emote);
    c.exec("msg targets message", msg);
    c.exec("tellraw targets message", tellraw);
    c.exec("title targets clear", clear);
    c.exec("title targets reset", reset);
    c.exec("title targets title title", |x| show(x, Shown::Title));
    c.exec("title targets subtitle title", |x| show(x, Shown::Subtitle));
    c.exec("title targets actionbar title", |x| show(x, Shown::ActionBar));
    c.exec("title targets times fadeIn stay fadeOut", times);
}
