//! /dialog show and clear (`DialogCommand`).
//!
//! `ClientboundShowDialogPacket` writes a `Holder<Dialog>`: `varint(id + 1)`
//! for a registry entry (the `minecraft:dialog` synced registry), or
//! `varint(0)` followed by the dialog as network NBT for an inline value.

use super::tracker::Tracker;
use crate::game::cmd::{Commands, Ctx};
use rine_command::args::misc::IdOrInline;
use rine_command::args::Arg;
use rine_command::{CmdError, CmdResult};
use rine_protocol::frame::packet;
use rine_protocol::Writer;

/// `minecraft:show_dialog` in packets.json (26.3).
const C_SHOW_DIALOG: i32 = 143;
/// `minecraft:clear_dialog`.
const C_CLEAR_DIALOG: i32 = 142;

pub fn register(c: &mut Commands) {
    c.exec("dialog show targets dialog", show);
    c.exec("dialog clear targets", clear);
}

fn show(x: &mut Ctx) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    let holder = match x.arg("dialog") {
        Arg::IdOrInline(v) => v.clone(),
        _ => return Err(super::stub(x)),
    };
    let body = show_packet(&holder)?;
    let mut t = Tracker::new();
    for slot in slots {
        if let Some(p) = x.g.player(slot) {
            p.send(body.clone());
        }
        t.track(slot, 1);
    }
    t.send(x, None, "commands.dialog.show.single", "commands.dialog.show.multiple", &[])
}

fn clear(x: &mut Ctx) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    let body = packet(C_CLEAR_DIALOG, |_| {});
    let mut t = Tracker::new();
    for slot in slots {
        if let Some(p) = x.g.player(slot) {
            p.send(body.clone());
        }
        t.track(slot, 1);
    }
    t.send(x, None, "commands.dialog.clear.single", "commands.dialog.clear.multiple", &[])
}

/// `ClientboundShowDialogPacket` with the holder encoded for the client.
fn show_packet(holder: &IdOrInline) -> CmdResult<bytes::BytesMut> {
    let mut err = None;
    let p = packet(C_SHOW_DIALOG, |b| match holder {
        IdOrInline::Id(name) => match rine_data::get().registries.id("minecraft:dialog", name) {
            Some(id) => b.w_varint(id as i32 + 1),
            None => err = Some(CmdError::key("argument.id.invalid")),
        },
        IdOrInline::Inline(tag) => {
            b.w_varint(0);
            tag.write_network(b);
        }
    });
    match err {
        Some(e) => Err(e),
        None => Ok(p),
    }
}
