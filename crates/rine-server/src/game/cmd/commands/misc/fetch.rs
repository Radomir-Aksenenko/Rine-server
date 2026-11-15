//! /fetchprofile (`FetchProfileCommand`). Rine has no Mojang profile API, so
//! name and id lookups always fail with the vanilla failure message. An
//! online player's profile is printed from what Rine knows (UUID and name,
//! no skin properties).

use super::{brackets, lit};
use crate::game::cmd::{Commands, Ctx};
use rine_command::args::Arg;
use rine_command::{CmdError, CmdResult};
use rine_protocol::text::{click_copy, click_run, Text};
use uuid::Uuid;

pub fn register(c: &mut Commands) {
    c.exec("fetchprofile name name", by_name);
    c.exec("fetchprofile id id", by_id);
    c.exec("fetchprofile entity entity", by_entity);
}

/// The vanilla lookup is asynchronous and replies with a failure message
/// while the command itself returns `SINGLE_SUCCESS`.
fn by_name(x: &mut Ctx) -> CmdResult<i32> {
    let name = x.str("name").to_string();
    x.failure(Text::tr("commands.fetchprofile.name.failure", vec![lit(name)]));
    Ok(1)
}

fn by_id(x: &mut Ctx) -> CmdResult<i32> {
    let id = match x.arg("id") {
        Arg::Uuid(v) => Uuid::from_u128(*v),
        _ => return Err(super::stub(x)),
    };
    x.failure(Text::tr("commands.fetchprofile.id.failure", vec![lit(id)]));
    Ok(1)
}

/// `printForEntity`: players print their profile, other entities fail.
fn by_entity(x: &mut Ctx) -> CmdResult<i32> {
    let eid = x.entity("entity")?;
    let Some(&slot) = x.g.by_eid.get(&eid) else {
        return Err(CmdError::tr("commands.fetchprofile.no_profile", vec![x.name_of(eid)]));
    };
    let Some(p) = x.g.players[slot].as_ref() else {
        return Err(CmdError::tr("commands.fetchprofile.no_profile", vec![x.name_of(eid)]));
    };
    let (name, uuid) = (p.name.clone(), p.uuid);
    // Approximate ResolvableProfile form: UUID as int array and name.
    let u = uuid.as_u128();
    let parts = [(u >> 96) as i32, (u >> 64) as i32, (u >> 32) as i32, u as i32];
    let encoded = format!(
        "{{id:[I;{},{},{},{}],name:\"{}\"}}",
        parts[0], parts[1], parts[2], parts[3], name
    );
    let items = [
        Text::tr("commands.fetchprofile.copy_component", vec![]).on_click(click_copy(encoded.clone())),
        Text::tr("commands.fetchprofile.give_item", vec![])
            .on_click(click_run(format!("give @s minecraft:player_head[profile={encoded}]"))),
        Text::tr("commands.fetchprofile.summon_mannequin", vec![])
            .on_click(click_run(format!("summon minecraft:mannequin ~ ~ ~ {{profile:{encoded}}}"))),
        Text::tr("commands.fetchprofile.copy_text", vec![Text::plain(name.clone()).color("white")])
            .on_click(click_copy(name.clone())),
    ];
    let mut list = Text::empty();
    for (i, item) in items.into_iter().enumerate() {
        if i > 0 {
            list = list.push(Text::plain(" "));
        }
        list = list.push(brackets(item).color("green"));
    }
    let display = x.name_of(eid);
    x.success(Text::tr("commands.fetchprofile.entity.success", vec![display, list]), false);
    Ok(1)
}
