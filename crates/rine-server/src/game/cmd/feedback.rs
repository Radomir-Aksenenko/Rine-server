//! Command output (`sendSuccess`, `sendFailure`, admin broadcast) and
//! entity display names as vanilla builds them.

use super::source::{Output, Source};
use crate::game::entity::Kind;
use crate::game::Game;
use rine_protocol::text::{click_suggest, hover_entity, Text};

fn uuid_ints(u: uuid::Uuid) -> [i32; 4] {
    let b = u.as_u128();
    [(b >> 96) as i32, (b >> 64) as i32, (b >> 32) as i32, b as i32]
}

/// `Player.getDisplayName()`: team-formatted name with `/tell` click,
/// entity hover and name insertion.
pub fn player_display_name(g: &Game, slot: usize) -> Text {
    let Some(p) = g.players.get(slot).and_then(|p| p.as_ref()) else { return Text::plain("") };
    let name = Text::plain(p.name.clone());
    let base = super::team_format(g, &p.name, name.clone());
    base.on_click(click_suggest(format!("/tell {} ", p.name)))
        .on_hover(hover_entity("minecraft:player", uuid_ints(p.uuid), Some(&name)))
        .insertion(p.name.clone())
}

/// `Entity.getName()` for non-player entities.
pub fn entity_plain_name(g: &Game, eid: i32) -> Text {
    if let Some(n) = g.ent_meta.get(&eid).and_then(|m| m.custom_name.clone()) {
        return n;
    }
    let Some(e) = g.entities.get(eid) else { return Text::plain("") };
    match &e.kind {
        Kind::Item { stack, .. } => {
            let items = &rine_data::get().items;
            let name = items.get(stack.item).map(|i| i.name.as_str()).unwrap_or("air");
            Text::key(item_description_id(name))
        }
        Kind::Tnt { .. } => Text::key("entity.minecraft.tnt"),
        Kind::Falling { .. } => Text::key("entity.minecraft.falling_block"),
    }
}

/// `item.minecraft.x` or `block.minecraft.x` for an item id (`x`).
pub fn item_description_id(item: &str) -> String {
    let name = item.strip_prefix("minecraft:").unwrap_or(item);
    if rine_data::get().blocks.block(name).is_some() {
        format!("block.minecraft.{name}")
    } else {
        format!("item.minecraft.{name}")
    }
}

/// Entity type id (`minecraft:item`) of an entity id.
pub fn entity_type_id(g: &Game, eid: i32) -> &'static str {
    if g.by_eid.contains_key(&eid) {
        return "minecraft:player";
    }
    match g.entities.get(eid).map(|e| &e.kind) {
        Some(Kind::Item { .. }) => "minecraft:item",
        Some(Kind::Tnt { .. }) => "minecraft:tnt",
        Some(Kind::Falling { .. }) => "minecraft:falling_block",
        None => "minecraft:marker",
    }
}

/// (`getScoreboardName`-like text name, `getDisplayName()`).
pub fn entity_names(g: &Game, eid: i32) -> (String, Text) {
    if let Some(&slot) = g.by_eid.get(&eid) {
        let name = g.players[slot].as_ref().map(|p| p.name.clone()).unwrap_or_default();
        return (name, player_display_name(g, slot));
    }
    let Some(e) = g.entities.get(eid) else { return (String::new(), Text::plain("")) };
    let plain = entity_plain_name(g, eid);
    let uuid = e.uuid;
    let text_name = plain.plain_string_resolved();
    let ty = entity_type_id(g, eid);
    let disp = super::team_format(g, &uuid.to_string(), plain.clone())
        .on_hover(hover_entity(ty, uuid_ints(uuid), Some(&plain)))
        .insertion(uuid.to_string());
    (text_name, disp)
}

/// Display name of any entity id.
pub fn display_name(g: &Game, eid: i32) -> Text {
    entity_names(g, eid).1
}

trait PlainResolved {
    fn plain_string_resolved(&self) -> String;
}
impl PlainResolved for Text {
    /// `Component.getString()` needs translations, which the server does
    /// not have; vanilla's English names are close enough for logs.
    fn plain_string_resolved(&self) -> String {
        match &self.content {
            rine_protocol::text::Content::Translate { key, .. } => {
                let last = key.rsplit('.').next().unwrap_or(key);
                let mut s = String::new();
                for (i, w) in last.split('_').enumerate() {
                    if i > 0 {
                        s.push(' ');
                    }
                    let mut c = w.chars();
                    if let Some(f) = c.next() {
                        s.extend(f.to_uppercase());
                        s.push_str(c.as_str());
                    }
                }
                s
            }
            _ => self.plain_string(),
        }
    }
}

fn accepts_success(g: &Game, out: &Output) -> bool {
    match out {
        Output::Player(_) => g.rules.bool("send_command_feedback"),
        Output::Console => true,
        Output::CommandBlock(pos) => g.rules.bool("send_command_feedback") && super::cmdblock::tracks_output(g, *pos),
        Output::Null => false,
    }
}

fn accepts_failure(g: &Game, out: &Output) -> bool {
    match out {
        Output::Player(_) | Output::Console => true,
        Output::CommandBlock(pos) => super::cmdblock::tracks_output(g, *pos),
        Output::Null => false,
    }
}

fn informs_admins(g: &Game, out: &Output) -> bool {
    match out {
        Output::Player(_) => true,
        Output::Console => g.shared.config.broadcast_console_to_ops,
        Output::CommandBlock(_) => g.rules.bool("command_block_output"),
        Output::Null => false,
    }
}

/// `CommandSource.sendSystemMessage` for an output sink.
pub fn output_message(g: &mut Game, out: &Output, t: &Text) {
    match out {
        Output::Player(eid) => {
            if let Some(&slot) = g.by_eid.get(eid) {
                if let Some(p) = g.player(slot) {
                    p.msg(t);
                }
            }
        }
        Output::Console => tracing::info!("{}", super::render_plain(t)),
        Output::CommandBlock(pos) => super::cmdblock::set_last_output(g, *pos, t),
        Output::Null => {}
    }
}

/// `CommandSourceStack.sendSystemMessage` (goes to the executing player
/// when there is one, else to the output).
pub fn send_system(g: &mut Game, src: &Source, t: &Text) {
    if src.silent {
        return;
    }
    if let Some(eid) = src.entity {
        if let Some(&slot) = g.by_eid.get(&eid) {
            if let Some(p) = g.player(slot) {
                p.msg(t);
            }
            return;
        }
    }
    output_message(g, &src.output, t);
}

/// `CommandSourceStack.sendSuccess`.
pub fn send_success(g: &mut Game, src: &Source, t: &Text, broadcast: bool) {
    let send = accepts_success(g, &src.output) && !src.silent;
    let bc = broadcast && informs_admins(g, &src.output) && !src.silent;
    if send {
        output_message(g, &src.output, t);
    }
    if bc {
        broadcast_to_admins(g, src, t);
    }
}

fn broadcast_to_admins(g: &mut Game, src: &Source, t: &Text) {
    let msg = Text::tr("chat.type.admin", vec![(*src.display).clone(), t.clone()]).color("gray").italic();
    if g.rules.bool("send_command_feedback") {
        for slot in 0..g.players.len() {
            let Some(p) = g.players[slot].as_ref() else { continue };
            if Output::Player(p.eid) == src.output {
                continue;
            }
            if g.op_level_of(&p.name) >= 2 {
                g.players[slot].as_mut().unwrap().msg(&msg);
            }
        }
    }
    if src.output != Output::Console && g.rules.bool("log_admin_commands") {
        tracing::info!("{}", super::render_plain(&msg));
    }
}

/// `CommandSourceStack.sendFailure`: red wrapper around the message.
pub fn send_failure(g: &mut Game, src: &Source, t: &Text) {
    if accepts_failure(g, &src.output) && !src.silent {
        let m = Text::empty().push(t.clone()).color("red");
        output_message(g, &src.output, &m);
    }
}
