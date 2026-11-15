//! /datapack and /reload (`DataPackCommand`, `ReloadCommand`).
//!
//! Packs: the built-in `vanilla` pack and folders `<world>/datapacks/<name>`
//! with a `pack.mcmeta` (id `file/<name>`). The selection lives in
//! `<world>/rine_datapacks.json`: `enabled` keeps the load order of the
//! enabled packs, `disabled` the ids the operator turned off. A folder that
//! is in neither list is new and counts as enabled (`discoverNewPacks`).

use crate::game::cmd::{lit, Commands, Ctx};
use crate::game::Game;
use rine_command::{CmdError, CmdResult};
use rine_protocol::text::{hover_text, Text};
use std::path::{Path, PathBuf};

const VANILLA: &str = "vanilla";
/// Data pack format of 26.3 (`packVersion(SERVER_DATA)` major).
const DATA_FORMAT: i64 = 121;
const STATE_FILE: &str = "rine_datapacks.json";

pub fn register(c: &mut Commands) {
    c.exec("datapack list", list_all);
    c.exec("datapack list available", list_available);
    c.exec("datapack list enabled", list_enabled);
    c.exec("datapack enable name", enable);
    c.exec("datapack enable name first", enable);
    c.exec("datapack enable name last", enable);
    c.exec("datapack enable name before existing", enable);
    c.exec("datapack enable name after existing", enable);
    c.exec("datapack disable name", disable);
    c.exec("datapack create id description", create);
    c.exec("reload", reload_cmd);
}

/// Pack ids (`file/<folder>`) the operator disabled.
pub fn disabled(g: &Game) -> Vec<String> {
    load(g).disabled
}

#[derive(Clone)]
struct Pack {
    id: String,
    title: Text,
    desc: Text,
    built_in: bool,
}

struct State {
    enabled: Vec<String>,
    disabled: Vec<String>,
}

fn datapacks_dir(g: &Game) -> PathBuf {
    Path::new(&g.shared.config.world_dir).join("datapacks")
}

fn state_path(g: &Game) -> PathBuf {
    Path::new(&g.shared.config.world_dir).join(STATE_FILE)
}

fn load(g: &Game) -> State {
    let default = State { enabled: vec![VANILLA.into()], disabled: Vec::new() };
    let Ok(text) = std::fs::read_to_string(state_path(g)) else { return default };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return default };
    let list = |k: &str| -> Vec<String> {
        v.get(k).and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    State { enabled: list("enabled"), disabled: list("disabled") }
}

fn save(g: &Game, st: &State) {
    let v = serde_json::json!({ "enabled": st.enabled, "disabled": st.disabled });
    let text = serde_json::to_string_pretty(&v).unwrap_or_default();
    if let Err(e) = std::fs::write(state_path(g), text) {
        tracing::warn!("cannot save data pack selection: {e}");
    }
}

/// `PackDetector` for folders: a directory with a readable `pack.mcmeta`.
fn read_description(dir: &Path) -> Option<Text> {
    let text = std::fs::read_to_string(dir.join("pack.mcmeta")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let desc = v.get("pack")?.get("description")?;
    Some(json_text(desc))
}

/// Component JSON to text (the forms a pack description takes).
fn json_text(v: &serde_json::Value) -> Text {
    match v {
        serde_json::Value::String(s) => Text::plain(s.clone()),
        serde_json::Value::Object(m) => match (m.get("translate"), m.get("text")) {
            (Some(serde_json::Value::String(k)), _) => Text::key(k.clone()),
            (_, Some(serde_json::Value::String(s))) => Text::plain(s.clone()),
            _ => Text::plain(v.to_string()),
        },
        _ => Text::plain(v.to_string()),
    }
}

/// `PackRepository.discoverAvailable`, sorted by id.
fn available(g: &Game) -> Vec<Pack> {
    let mut v = vec![Pack {
        id: VANILLA.into(),
        title: Text::key("dataPack.vanilla.name"),
        desc: Text::key("dataPack.vanilla.description"),
        built_in: true,
    }];
    if let Ok(rd) = std::fs::read_dir(datapacks_dir(g)) {
        for e in rd.flatten() {
            let dir = e.path();
            if !dir.is_dir() {
                continue;
            }
            let Some(name) = dir.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
            let Some(desc) = read_description(&dir) else { continue };
            v.push(Pack { id: format!("file/{name}"), title: Text::plain(name), desc, built_in: false });
        }
    }
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
}

/// `PackRepository.getSelectedPacks`: enabled packs in load order.
fn selected(g: &Game) -> Vec<Pack> {
    let st = load(g);
    let av = available(g);
    let mut order: Vec<&str> = Vec::new();
    for id in &st.enabled {
        if av.iter().any(|p| &p.id == id) && !order.contains(&id.as_str()) {
            order.push(id);
        }
    }
    for p in &av {
        if !order.contains(&p.id.as_str()) && !st.disabled.contains(&p.id) {
            order.push(&p.id);
        }
    }
    order.iter().filter_map(|id| av.iter().find(|p| p.id == *id).cloned()).collect()
}

/// Writes the selection (`ReloadCommand.discoverNewPacks` keeps it per reload).
fn persist(g: &Game, sel: &[Pack]) {
    let enabled: Vec<String> = sel.iter().map(|p| p.id.clone()).collect();
    let disabled: Vec<String> = available(g).into_iter().map(|p| p.id).filter(|id| !enabled.contains(id)).collect();
    save(g, &State { enabled, disabled });
}

/// `DataPackCommand.getPack`: unknown, already enabled or already disabled.
fn get_pack(g: &Game, id: &str, enabling: bool) -> CmdResult<Pack> {
    let Some(p) = available(g).into_iter().find(|p| p.id == id) else {
        return Err(CmdError::tr("commands.datapack.unknown", vec![lit(id)]));
    };
    let enabled = selected(g).iter().any(|s| s.id == id);
    if enabling && enabled {
        return Err(CmdError::tr("commands.datapack.enable.failed", vec![lit(id)]));
    }
    if !enabling && !enabled {
        return Err(CmdError::tr("commands.datapack.disable.failed", vec![lit(id)]));
    }
    Ok(p)
}

/// `Pack.getChatLink`: `[id]` with the title and description on hover.
fn chat_link(p: &Pack, enabled: bool) -> Text {
    let source = if p.built_in { "pack.source.builtin" } else { "pack.source.world" };
    let name = Text::tr("pack.nameAndSource", vec![Text::plain(p.id.clone()), Text::key(source)]).color("gray");
    let hover = Text::empty().push(p.title.clone()).push(Text::plain("\n")).push(p.desc.clone());
    Text::tr("chat.square_brackets", vec![name])
        .color(if enabled { "green" } else { "red" })
        .insertion(escape_if_required(&p.id))
        .on_hover(hover_text(&hover))
}

/// `StringArgumentType.escapeIfRequired`.
fn escape_if_required(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+')) {
        s.to_string()
    } else {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// `ComponentUtils.formatList` with the gray `, ` separator.
fn format_list(items: Vec<Text>) -> Text {
    crate::game::cmd::feedback::format_list(items, &crate::game::cmd::feedback::list_sep())
}

fn list_all(x: &mut Ctx) -> CmdResult<i32> {
    let enabled = list_enabled(x)?;
    let available = list_available(x)?;
    Ok(enabled + available)
}

fn list_available(x: &mut Ctx) -> CmdResult<i32> {
    let sel = selected(x.g);
    let unselected: Vec<Pack> = available(x.g).into_iter().filter(|p| !sel.iter().any(|s| s.id == p.id)).collect();
    if unselected.is_empty() {
        x.success(Text::key("commands.datapack.list.available.none"), false);
    } else {
        let links = unselected.iter().map(|p| chat_link(p, false)).collect();
        let msg = Text::tr("commands.datapack.list.available.success", vec![lit(unselected.len()), format_list(links)]);
        x.success(msg, false);
    }
    Ok(unselected.len() as i32)
}

fn list_enabled(x: &mut Ctx) -> CmdResult<i32> {
    let sel = selected(x.g);
    if sel.is_empty() {
        x.success(Text::key("commands.datapack.list.enabled.none"), false);
    } else {
        let links = sel.iter().map(|p| chat_link(p, true)).collect();
        let msg = Text::tr("commands.datapack.list.enabled.success", vec![lit(sel.len()), format_list(links)]);
        x.success(msg, false);
    }
    Ok(sel.len() as i32)
}

/// `datapack enable name [first|last|before existing|after existing]`.
fn enable(x: &mut Ctx) -> CmdResult<i32> {
    let s = names(x);
    let name = x.str("name").to_string();
    let pack = get_pack(x.g, &name, true)?;
    let mut sel = selected(x.g);
    let at = match s.get(3).copied() {
        Some("first") => 0,
        Some("last") => sel.len(),
        Some("before") => {
            let existing = x.str("existing").to_string();
            let ex = get_pack(x.g, &existing, false)?;
            sel.iter().position(|p| p.id == ex.id).unwrap_or(0)
        }
        Some("after") => {
            let existing = x.str("existing").to_string();
            let ex = get_pack(x.g, &existing, false)?;
            sel.iter().position(|p| p.id == ex.id).map_or(0, |i| i + 1)
        }
        // `Pack.Position.insert`: built-in packs go to the bottom, folders to the top
        _ => {
            if pack.built_in {
                0
            } else {
                sel.len()
            }
        }
    };
    sel.insert(at, pack.clone());
    x.success(Text::tr("commands.datapack.modify.enable", vec![chat_link(&pack, true)]), true);
    persist(x.g, &sel);
    super::function::reload(x.g);
    Ok(sel.len() as i32)
}

/// `datapack disable name`.
fn disable(x: &mut Ctx) -> CmdResult<i32> {
    let name = x.str("name").to_string();
    let pack = get_pack(x.g, &name, false)?;
    let mut sel = selected(x.g);
    sel.retain(|p| p.id != pack.id);
    x.success(Text::tr("commands.datapack.modify.disable", vec![chat_link(&pack, true)]), true);
    persist(x.g, &sel);
    super::function::reload(x.g);
    Ok(sel.len() as i32)
}

/// `datapack create <id> <description>`.
fn create(x: &mut Ctx) -> CmdResult<i32> {
    let id = x.str("id").to_string();
    let desc = x.component("description")?;
    if !valid_segment(&id) {
        return Err(CmdError::tr("commands.datapack.create.invalid_name", vec![lit(&id)]));
    }
    if !portable(&id) {
        return Err(CmdError::tr("commands.datapack.create.invalid_full_name", vec![lit(&id)]));
    }
    let root = datapacks_dir(x.g);
    let dir = root.join(&id);
    if dir.exists() {
        return Err(CmdError::tr("commands.datapack.create.already_exists", vec![lit(&id)]));
    }
    let meta = serde_json::json!({
        "pack": { "description": desc_value(&desc), "min_format": DATA_FORMAT, "max_format": DATA_FORMAT }
    });
    let written = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(&root)?;
        std::fs::create_dir(&dir)?;
        std::fs::create_dir(dir.join("data"))?;
        let text = serde_json::to_string_pretty(&meta).unwrap_or_default();
        std::fs::write(dir.join("pack.mcmeta"), text)
    })();
    if written.is_err() {
        return Err(CmdError::tr("commands.datapack.create.io_failure", vec![lit(&id)]));
    }
    x.success(Text::tr("commands.datapack.create.success", vec![lit(&id)]), true);
    Ok(1)
}

/// `/reload`: keep the discovered selection, then reload the functions.
fn reload_cmd(x: &mut Ctx) -> CmdResult<i32> {
    let sel = selected(x.g);
    persist(x.g, &sel);
    x.success(Text::key("commands.reload.success"), true);
    super::function::reload(x.g);
    Ok(0)
}

/// `pack` value of the description: a plain string when the text has no
/// formatting, the component JSON otherwise.
fn desc_value(t: &Text) -> serde_json::Value {
    let v: serde_json::Value = serde_json::from_str(&t.to_json()).unwrap_or_else(|_| serde_json::Value::String(t.plain_string()));
    if let serde_json::Value::Object(m) = &v {
        if let (1, Some(serde_json::Value::String(s))) = (m.len(), m.get("text")) {
            return serde_json::Value::String(s.clone());
        }
    }
    v
}

/// `FileUtil.isValidPathSegment`: `[-._a-z0-9]+`, not `.` or `..`.
fn valid_segment(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && s.chars().all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-' | '.' | '_'))
}

/// `FileUtil.isPathPartPortable`: no trailing dot, no Windows device names.
fn portable(s: &str) -> bool {
    if s.ends_with('.') {
        return false;
    }
    let upper = s.to_ascii_uppercase();
    let base = upper.split('.').next().unwrap_or("");
    const RESERVED: [&str; 19] = [
        "COM", "CLOCK$", "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4",
    ];
    !RESERVED.contains(&base)
}

/// Node names of the current stage (literals and arguments).
fn names(x: &Ctx) -> Vec<&'static str> {
    let tree = x.g.cmds.tree.clone();
    x.c.nodes.iter().map(|(id, _, _)| tree.node(*id).name).collect()
}
