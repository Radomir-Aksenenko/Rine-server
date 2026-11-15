//! Selector evaluation (vanilla `EntitySelector.findEntities` /
//! `findPlayers`) and text resolution (`ComponentUtils.updateForEntity`,
//! `MessageArgument.resolveComponent`).
//!
//! Volume- and distance-bounded selectors only scan the chunks they can
//! reach through the entity grid; type-restricted ones skip players or
//! entities entirely; `@p`/`@a` never touch the entity list.

use super::source::Source;
use crate::game::entity::Kind;
use crate::game::Game;
use rine_command::args::coords::SourcePose;
use rine_command::args::selector::{EntitySelector, Filter, MessageArg, ProfileArg, Sort};
use rine_command::{CmdError, CmdResult};
use rine_protocol::nbt::Tag;
use rine_protocol::text::{Content, Text};
use rine_world::ChunkPos;

/// Coordinates context of a source (anchor applied for `^`).
pub fn pose(g: &Game, src: &Source) -> SourcePose {
    // EntityAnchorArgument.Anchor.apply(source): source position, plus the
    // entity's eye height for EYES
    let anchor_pos = if src.anchor_eyes {
        match src.entity {
            Some(e) => [src.pos[0], src.pos[1] + eye_height(g, e), src.pos[2]],
            None => src.pos,
        }
    } else {
        src.pos
    };
    SourcePose { pos: src.pos, rot: src.rot, anchor_pos }
}

/// Feet position of a player or entity.
pub fn entity_pos(g: &Game, eid: i32) -> Option<[f64; 3]> {
    if let Some(&slot) = g.by_eid.get(&eid) {
        return g.players[slot].as_ref().map(|p| p.pos);
    }
    g.entities.get(eid).filter(|e| !e.removed).map(|e| e.pos)
}

/// (xRot, yRot) of a player or entity.
pub fn entity_rot(g: &Game, eid: i32) -> [f32; 2] {
    if let Some(&slot) = g.by_eid.get(&eid) {
        if let Some(p) = g.players[slot].as_ref() {
            return [p.pitch, p.yaw];
        }
    }
    g.ent_meta.get(&eid).map(|m| m.rot).unwrap_or([0.0, 0.0])
}

pub fn eye_height(g: &Game, eid: i32) -> f64 {
    if let Some(&slot) = g.by_eid.get(&eid) {
        let sneaking = g.players[slot].as_ref().is_some_and(|p| p.sneaking);
        return if sneaking { 1.27 } else { 1.62 };
    }
    match g.entities.get(eid).map(|e| &e.kind) {
        Some(Kind::Item { .. }) => 0.2125,
        Some(Kind::Tnt { .. }) => 0.15,
        Some(Kind::Falling { .. }) => 0.833,
        None => 0.0,
    }
}

/// (width, height) of the bounding box.
pub fn entity_size(g: &Game, eid: i32) -> (f64, f64) {
    if let Some(&slot) = g.by_eid.get(&eid) {
        let sneaking = g.players[slot].as_ref().is_some_and(|p| p.sneaking);
        return (0.6, if sneaking { 1.5 } else { 1.8 });
    }
    match g.entities.get(eid).map(|e| &e.kind) {
        Some(Kind::Item { .. }) => (0.25, 0.25),
        _ => (0.98, 0.98),
    }
}

fn is_alive(g: &Game, eid: i32) -> bool {
    if let Some(&slot) = g.by_eid.get(&eid) {
        return g.players[slot].as_ref().is_some_and(|p| !p.dead);
    }
    g.entities.get(eid).is_some_and(|e| !e.removed)
}

/// `Entity.getScoreboardName()`: player name or entity UUID string.
pub fn scoreboard_name(g: &Game, eid: i32) -> String {
    if let Some(&slot) = g.by_eid.get(&eid) {
        return g.players[slot].as_ref().map(|p| p.name.clone()).unwrap_or_default();
    }
    g.entities.get(eid).map(|e| e.uuid.to_string()).unwrap_or_default()
}

pub fn entity_uuid(g: &Game, eid: i32) -> Option<uuid::Uuid> {
    if let Some(&slot) = g.by_eid.get(&eid) {
        return g.players[slot].as_ref().map(|p| p.uuid);
    }
    g.entities.get(eid).map(|e| e.uuid)
}

/// `Entity.getName().getString()` in English.
pub fn plain_name(g: &Game, eid: i32) -> String {
    if let Some(&slot) = g.by_eid.get(&eid) {
        return g.players[slot].as_ref().map(|p| p.name.clone()).unwrap_or_default();
    }
    rine_command::lang::plain(&super::feedback::entity_plain_name(g, eid))
}

fn wrap_degrees(v: f32) -> f32 {
    let mut f = v % 360.0;
    if f >= 180.0 {
        f -= 360.0;
    }
    if f < -180.0 {
        f += 360.0;
    }
    f
}

fn type_matches(g: &Game, eid: i32, id: &str) -> bool {
    let ty = super::feedback::entity_type_id(g, eid);
    if let Some(tag) = id.strip_prefix('#') {
        return entity_tag_contains(tag, ty);
    }
    let id = if id.contains(':') { id.to_string() } else { format!("minecraft:{id}") };
    ty == id
}

/// Entity type tag membership (`#minecraft:skeletons`).
pub fn entity_tag_contains(tag: &str, ty: &str) -> bool {
    let regs = &rine_data::get().registries;
    let tag = tag.strip_prefix("minecraft:").unwrap_or(tag);
    regs.tag_contains("minecraft:entity_type", tag, ty.strip_prefix("minecraft:").unwrap_or(ty))
}

fn filter_ok(g: &Game, eid: i32, f: &Filter) -> bool {
    let player = g.by_eid.get(&eid).and_then(|&s| g.players[s].as_ref());
    match f {
        Filter::Name { name, negated } => (plain_name(g, eid) == *name) != *negated,
        Filter::Type { id, negated } => type_matches(g, eid, id) != *negated,
        Filter::Tag { tag, negated } => {
            let tags = g.ent_meta.get(&eid).map(|m| m.tags.as_slice()).unwrap_or(&[]);
            let has = if tag.is_empty() { tags.is_empty() } else { tags.iter().any(|t| t == tag) };
            has != *negated
        }
        Filter::Team { team, negated } => {
            let cur = super::commands::scoreboard::team_of(g, &scoreboard_name(g, eid));
            let has = if team.is_empty() { cur.is_none() } else { cur.as_deref() == Some(team.as_str()) };
            has != *negated
        }
        Filter::GameMode { mode, negated } => match player {
            Some(p) => (p.gamemode == *mode) != *negated,
            None => false,
        },
        Filter::Level(r) => match player {
            Some(_) => r.matches(super::commands::experience::level_of(g, eid)),
            None => false,
        },
        Filter::XRot(r) => rot_matches(r, entity_rot(g, eid)[0]),
        Filter::YRot(r) => rot_matches(r, entity_rot(g, eid)[1]),
        Filter::Scores(list) => {
            let name = scoreboard_name(g, eid);
            list.iter().all(|(obj, r)| match super::commands::scoreboard::score(g, &name, obj) {
                Some(v) => r.matches(v),
                None => false,
            })
        }
        Filter::Nbt { tag, negated } => {
            let data = super::commands::data::entity_nbt(g, eid);
            super::commands::data::compare_nbt(tag, &data, true) != *negated
        }
        Filter::Advancements(_) => player.is_some() && false,
        Filter::Predicate { negated, .. } => *negated,
    }
}

fn aabb_of(sel: &EntitySelector, pos: [f64; 3]) -> Option<[f64; 6]> {
    if sel.dx.is_none() && sel.dy.is_none() && sel.dz.is_none() {
        return None;
    }
    let (dx, dy, dz) = (sel.dx.unwrap_or(0.0), sel.dy.unwrap_or(0.0), sel.dz.unwrap_or(0.0));
    Some([
        pos[0] + dx.min(0.0),
        pos[1] + dy.min(0.0),
        pos[2] + dz.min(0.0),
        pos[0] + dx.max(0.0) + 1.0,
        pos[1] + dy.max(0.0) + 1.0,
        pos[2] + dz.max(0.0) + 1.0,
    ])
}

fn matches(g: &Game, eid: i32, sel: &EntitySelector, pos: [f64; 3], aabb: Option<[f64; 6]>) -> bool {
    if sel.alive_only && !is_alive(g, eid) {
        return false;
    }
    let Some(p) = entity_pos(g, eid) else { return false };
    if let Some(b) = aabb {
        let (w, h) = entity_size(g, eid);
        let e = [p[0] - w / 2.0, p[1], p[2] - w / 2.0, p[0] + w / 2.0, p[1] + h, p[2] + w / 2.0];
        if !(e[0] < b[3] && e[3] > b[0] && e[1] < b[4] && e[4] > b[1] && e[2] < b[5] && e[5] > b[2]) {
            return false;
        }
    }
    if let Some(r) = &sel.distance {
        let d2 = (p[0] - pos[0]).powi(2) + (p[1] - pos[1]).powi(2) + (p[2] - pos[2]).powi(2);
        if !r.matches_sqr(d2) {
            return false;
        }
    }
    sel.filters.iter().all(|f| filter_ok(g, eid, f))
}

fn check_permission(src: &Source, sel: &EntitySelector) -> CmdResult<()> {
    if sel.uses_selector && src.level < 2 {
        return Err(CmdError::key("argument.entity.selector.not_allowed"));
    }
    Ok(())
}

fn sel_pos(sel: &EntitySelector, src: &Source) -> [f64; 3] {
    [sel.x.unwrap_or(src.pos[0]), sel.y.unwrap_or(src.pos[1]), sel.z.unwrap_or(src.pos[2])]
}

fn sort_and_limit(g: &Game, sel: &EntitySelector, pos: [f64; 3], mut v: Vec<i32>) -> Vec<i32> {
    if v.len() > 1 {
        let d2 = |e: &i32| {
            let p = entity_pos(g, *e).unwrap_or(pos);
            (p[0] - pos[0]).powi(2) + (p[1] - pos[1]).powi(2) + (p[2] - pos[2]).powi(2)
        };
        match sel.sort {
            Sort::Arbitrary => {}
            Sort::Nearest => v.sort_by(|a, b| d2(a).total_cmp(&d2(b))),
            Sort::Furthest => v.sort_by(|a, b| d2(b).total_cmp(&d2(a))),
            Sort::Random => {
                // Fisher-Yates with the server RNG
                for i in (1..v.len()).rev() {
                    let j = (crate::game::cmd::commands::random::next_u32() as usize) % (i + 1);
                    v.swap(i, j);
                }
            }
        }
    }
    v.truncate(sel.max_results.max(0) as usize);
    v
}

/// Candidate entity ids (non-players) that can match a bounded selector.
fn entity_candidates(g: &Game, sel: &EntitySelector, pos: [f64; 3], aabb: Option<[f64; 6]>) -> Vec<i32> {
    let reach = match (aabb, sel.distance.and_then(|d| d.max)) {
        (Some(b), _) => Some(((b[0] - 2.0) as i32, (b[2] - 2.0) as i32, (b[3] + 2.0) as i32, (b[5] + 2.0) as i32)),
        (None, Some(m)) if m < 512.0 => {
            let r = m + 2.0;
            Some(((pos[0] - r) as i32, (pos[2] - r) as i32, (pos[0] + r) as i32, (pos[2] + r) as i32))
        }
        _ => None,
    };
    match reach {
        Some((x0, z0, x1, z1)) => {
            let mut v = Vec::new();
            for cx in (x0 >> 4)..=(x1 >> 4) {
                for cz in (z0 >> 4)..=(z1 >> 4) {
                    if let Some(list) = g.entities.grid.get(&ChunkPos::new(cx, cz)) {
                        v.extend_from_slice(list);
                    }
                }
            }
            // the grid is rebuilt periodically: include fresh spawns
            v.extend_from_slice(&g.entities.fresh);
            v.sort_unstable();
            v.dedup();
            v
        }
        None => g.entities.list.iter().filter(|e| !e.removed).map(|e| e.eid).collect(),
    }
}

/// `EntitySelector.findEntities` (entity ids, players included).
pub fn find_entities(g: &Game, src: &Source, sel: &EntitySelector) -> CmdResult<Vec<i32>> {
    check_permission(src, sel)?;
    if !sel.includes_entities {
        return Ok(find_players(g, src, sel)?.into_iter().filter_map(|s| g.players[s].as_ref().map(|p| p.eid)).collect());
    }
    if let Some(name) = &sel.player_name {
        return Ok(g.find_player(name).and_then(|s| g.players[s].as_ref()).map(|p| vec![p.eid]).unwrap_or_default());
    }
    if let Some(u) = sel.entity_uuid {
        let u = uuid::Uuid::from_u128(u);
        if let Some(p) = g.players.iter().flatten().find(|p| p.uuid == u) {
            return Ok(vec![p.eid]);
        }
        return Ok(g.entities.list.iter().find(|e| e.uuid == u && !e.removed).map(|e| vec![e.eid]).unwrap_or_default());
    }
    let pos = sel_pos(sel, src);
    let aabb = aabb_of(sel, pos);
    if sel.current_entity {
        return Ok(match src.entity {
            Some(e) if matches(g, e, sel, pos, aabb) => vec![e],
            _ => vec![],
        });
    }
    let mut out = Vec::new();
    let want_players = sel.type_id.as_deref().is_none_or(|t| t == "minecraft:player");
    let want_entities = sel.type_id.as_deref() != Some("minecraft:player");
    if want_players {
        for p in g.players.iter().flatten() {
            if matches(g, p.eid, sel, pos, aabb) {
                out.push(p.eid);
            }
        }
    }
    if want_entities {
        for e in entity_candidates(g, sel, pos, aabb) {
            if matches(g, e, sel, pos, aabb) {
                out.push(e);
            }
        }
    }
    Ok(sort_and_limit(g, sel, pos, out))
}

/// `EntitySelector.findPlayers` (player slots).
pub fn find_players(g: &Game, src: &Source, sel: &EntitySelector) -> CmdResult<Vec<usize>> {
    check_permission(src, sel)?;
    if let Some(name) = &sel.player_name {
        return Ok(g.find_player(name).into_iter().collect());
    }
    if let Some(u) = sel.entity_uuid {
        let u = uuid::Uuid::from_u128(u);
        return Ok(g.players.iter().flatten().find(|p| p.uuid == u).map(|p| vec![p.slot]).unwrap_or_default());
    }
    let pos = sel_pos(sel, src);
    let aabb = aabb_of(sel, pos);
    if sel.current_entity {
        return Ok(match src.entity.and_then(|e| g.by_eid.get(&e).copied()) {
            Some(slot) if matches(g, g.players[slot].as_ref().unwrap().eid, sel, pos, aabb) => vec![slot],
            _ => vec![],
        });
    }
    let limit = if sel.sort == Sort::Arbitrary { sel.max_results.max(0) as usize } else { usize::MAX };
    let mut eids = Vec::new();
    for p in g.players.iter().flatten() {
        if eids.len() >= limit {
            break;
        }
        if matches(g, p.eid, sel, pos, aabb) {
            eids.push(p.eid);
        }
    }
    let eids = sort_and_limit(g, sel, pos, eids);
    Ok(eids.into_iter().filter_map(|e| g.by_eid.get(&e).copied()).collect())
}

/// `EntitySelector.findSingleEntity`.
pub fn find_single_entity(g: &Game, src: &Source, sel: &EntitySelector) -> CmdResult<i32> {
    let v = find_entities(g, src, sel)?;
    match v.len() {
        0 => Err(CmdError::key("argument.entity.notfound.entity")),
        1 => Ok(v[0]),
        _ => Err(CmdError::key("argument.entity.toomany")),
    }
}

/// `EntitySelector.findSinglePlayer`.
pub fn find_single_player(g: &Game, src: &Source, sel: &EntitySelector) -> CmdResult<usize> {
    let v = find_players(g, src, sel)?;
    match v.len() {
        0 => Err(CmdError::key("argument.entity.notfound.player")),
        1 => Ok(v[0]),
        _ => Err(CmdError::key("argument.player.toomany")),
    }
}

/// `GameProfileArgument`: online players by selector, or a name (online
/// player, else the user cache).
pub fn resolve_profiles(g: &Game, src: &Source, arg: &ProfileArg) -> CmdResult<Vec<(String, uuid::Uuid)>> {
    match arg {
        ProfileArg::Selector(sel) => {
            if sel.includes_entities {
                return Err(CmdError::key("argument.player.entities"));
            }
            let v: Vec<_> = find_players(g, src, sel)?
                .into_iter()
                .filter_map(|s| g.players[s].as_ref().map(|p| (p.name.clone(), p.uuid)))
                .collect();
            if v.is_empty() {
                return Err(CmdError::key("argument.entity.notfound.player"));
            }
            Ok(v)
        }
        ProfileArg::Name(name) => {
            if let Some(p) = g.find_player(name).and_then(|s| g.players[s].as_ref()) {
                return Ok(vec![(p.name.clone(), p.uuid)]);
            }
            match super::commands::players::cached_profile(g, name) {
                Some(p) => Ok(vec![p]),
                None => Err(CmdError::key("argument.player.unknown")),
            }
        }
    }
}

/// `ComponentUtils.formatList(entities, Entity::getDisplayName)`.
pub fn join_names(g: &Game, eids: &[i32], sep: Option<Text>) -> Text {
    match eids.len() {
        0 => Text::empty(),
        1 => super::feedback::display_name(g, eids[0]),
        _ => {
            let sep = sep.unwrap_or_else(|| Text::plain(", ").color("gray"));
            let mut t = Text::empty();
            for (i, e) in eids.iter().enumerate() {
                if i > 0 {
                    t = t.push(sep.clone());
                }
                t = t.push(super::feedback::display_name(g, *e));
            }
            t
        }
    }
}

/// `MessageArgument.resolveComponent`.
pub fn resolve_message(g: &Game, src: &Source, m: &MessageArg) -> CmdResult<Text> {
    if m.parts.is_empty() || src.level < 2 {
        return Ok(Text::plain(m.text.clone()));
    }
    let mut out = Text::plain(m.text[..m.parts[0].0].to_string());
    let mut cursor = m.parts[0].0;
    for (start, end, sel) in &m.parts {
        let ents = find_entities(g, src, sel)?;
        let names = join_names(g, &ents, None);
        if *start > cursor {
            out = out.push(Text::plain(m.text[cursor..*start].to_string()));
        }
        out = out.push(names);
        cursor = *end;
    }
    if cursor < m.text.len() {
        out = out.push(Text::plain(m.text[cursor..].to_string()));
    }
    Ok(out)
}

/// Decode an NBT text component (string, list, or compound).
pub fn text_from_tag(t: &Tag) -> Text {
    match t {
        Tag::String(s) => Text::plain(s.clone()),
        Tag::List(items) => {
            let mut it = items.iter();
            let Some(first) = it.next() else { return Text::empty() };
            let mut base = text_from_tag(first);
            for e in it {
                base.extra.push(text_from_tag(e));
            }
            base
        }
        Tag::Compound(_) => {
            let gs = |k: &str| match t.get(k) {
                Some(Tag::String(s)) => Some(s.clone()),
                _ => None,
            };
            let gb = |k: &str| match t.get(k) {
                Some(Tag::Byte(b)) => Some(*b != 0),
                _ => None,
            };
            let mut out = Text::empty();
            if let Some(s) = gs("text") {
                out.text = s;
            } else if let Some(k) = gs("translate") {
                let with = match t.get("with") {
                    Some(Tag::List(l)) => l.iter().map(text_from_tag).collect(),
                    _ => Vec::new(),
                };
                out.content = Content::Translate { key: k.into(), fallback: gs("fallback"), with };
            } else if let Some(k) = gs("keybind") {
                out.content = Content::Keybind(k);
            } else {
                // selector / score / nbt / object: kept raw until resolved
                const STYLE: &[&str] = &[
                    "color", "bold", "italic", "underlined", "strikethrough", "obfuscated", "font", "insertion", "shadow_color",
                    "click_event", "hover_event", "extra", "type",
                ];
                if let Tag::Compound(fields) = t {
                    let raw: Vec<(String, Tag)> = fields.iter().filter(|(k, _)| !STYLE.contains(&k.as_str())).cloned().collect();
                    if !raw.is_empty() {
                        out.content = Content::Raw(raw);
                    }
                }
            }
            out.color = gs("color").map(Into::into);
            out.bold = gb("bold");
            out.italic = gb("italic");
            out.underlined = gb("underlined");
            out.strikethrough = gb("strikethrough");
            out.obfuscated = gb("obfuscated");
            out.font = gs("font");
            out.insertion = gs("insertion");
            out.shadow_color = match t.get("shadow_color") {
                Some(Tag::Int(v)) => Some(*v),
                _ => None,
            };
            out.click = t.get("click_event").cloned().map(Box::new);
            out.hover = t.get("hover_event").cloned().map(Box::new);
            if let Some(Tag::List(l)) = t.get("extra") {
                out.extra = l.iter().map(text_from_tag).collect();
            }
            out
        }
        Tag::Byte(v) => Text::plain(v.to_string()),
        Tag::Short(v) => Text::plain(v.to_string()),
        Tag::Int(v) => Text::plain(v.to_string()),
        Tag::Long(v) => Text::plain(v.to_string()),
        Tag::Float(v) => Text::plain(rine_command::error::java_float(*v)),
        Tag::Double(v) => Text::plain(rine_command::error::java_double(*v)),
        _ => Text::empty(),
    }
}

const DEPTH_LIMIT: u32 = 100;

/// `ComponentUtils.updateForEntity` with the source's entity.
pub fn resolve_component(g: &Game, src: &Source, t: &Tag) -> CmdResult<Text> {
    resolve_text(g, src, &text_from_tag(t), src.entity, 0)
}

pub fn resolve_text(g: &Game, src: &Source, t: &Text, entity: Option<i32>, depth: u32) -> CmdResult<Text> {
    if depth > DEPTH_LIMIT {
        return Ok(Text::key("chat.ellipsis"));
    }
    let mut base = match &t.content {
        Content::Raw(fields) => resolve_raw(g, src, fields, entity, depth + 1)?,
        Content::Translate { key, fallback, with } => {
            let mut w = Vec::with_capacity(with.len());
            for a in with {
                w.push(resolve_text(g, src, a, entity, depth + 1)?);
            }
            Text { content: Content::Translate { key: key.clone(), fallback: fallback.clone(), with: w }, ..Text::empty() }
        }
        Content::Text => Text::plain(t.text.clone()),
        Content::Keybind(k) => Text { content: Content::Keybind(k.clone()), ..Text::empty() },
    };
    for e in &t.extra {
        base.extra.push(resolve_text(g, src, e, entity, depth + 1)?);
    }
    // style: component's own style wins over the resolved content's
    let mut style = t.clone();
    style.extra.clear();
    if let Some(h) = &t.hover {
        if h.get("action") == Some(&Tag::String("show_text".into())) {
            if let Some(v) = h.get("value") {
                let r = resolve_text(g, src, &text_from_tag(v), entity, depth + 1)?;
                style.hover = Some(Box::new(rine_protocol::text::hover_text(&r)));
            }
        }
    }
    let content = std::mem::take(&mut base.content);
    let text = std::mem::take(&mut base.text);
    let extra = std::mem::take(&mut base.extra);
    Ok(Text { content, text, extra, ..style }.with_style_from(&base))
}

fn resolve_raw(g: &Game, src: &Source, fields: &[(String, Tag)], entity: Option<i32>, depth: u32) -> CmdResult<Text> {
    let get = |k: &str| fields.iter().find(|(n, _)| n == k).map(|(_, v)| v);
    if let Some(Tag::String(sel)) = get("selector") {
        let parsed = rine_command::args::selector::parse_selector(&mut rine_command::Reader::new(sel), true)?;
        let ents = find_entities(g, src, &parsed)?;
        let sep = match get("separator") {
            Some(s) => Some(resolve_text(g, src, &text_from_tag(s), entity, depth)?),
            None => None,
        };
        return Ok(join_names(g, &ents, sep));
    }
    if let Some(score) = get("score") {
        let name = match score.get("name") {
            Some(Tag::String(s)) => s.clone(),
            _ => return Ok(Text::empty()),
        };
        let obj = match score.get("objective") {
            Some(Tag::String(s)) => s.clone(),
            _ => return Ok(Text::empty()),
        };
        let holder = if name == "*" {
            match entity {
                Some(e) => scoreboard_name(g, e),
                None => return Ok(Text::empty()),
            }
        } else if name.starts_with('@') {
            let parsed = rine_command::args::selector::parse_selector(&mut rine_command::Reader::new(&name), true)?;
            let ents = find_entities(g, src, &parsed)?;
            if ents.len() != 1 {
                return Err(CmdError::key("argument.entity.toomany"));
            }
            scoreboard_name(g, ents[0])
        } else {
            name
        };
        return Ok(super::commands::scoreboard::format_score(g, &holder, &obj));
    }
    if get("nbt").is_some() {
        return super::commands::data::resolve_nbt_component(g, src, fields, entity, depth);
    }
    // object / unknown content: pass through unchanged
    Ok(Text { content: Content::Raw(fields.to_vec()), ..Text::empty() })
}

/// `EntitySelectorParser.createRotationPredicate` (bounds pre-wrapped).
fn rot_matches(r: &rine_command::args::misc::FloatRange, rot: f32) -> bool {
    let (min, max) = (r.min.unwrap_or(0.0) as f32, r.max.unwrap_or(359.0) as f32);
    let rot = wrap_degrees(rot);
    if min > max {
        rot >= min || rot <= max
    } else {
        rot >= min && rot <= max
    }
}
