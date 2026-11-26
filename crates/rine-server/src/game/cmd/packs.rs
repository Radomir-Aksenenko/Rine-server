//! The data pack repository (`PackRepository`): the built-in `vanilla` pack and folders
//! `<world>/datapacks/<name>` with a `pack.mcmeta` (id `file/<name>`). The selection lives in
//! `<world>/rine_datapacks.json`: `enabled` keeps the load order of the enabled packs, `disabled`
//! the ids the operator turned off. A folder that is in neither list is new and counts as
//! enabled (`discoverNewPacks`).

use crate::game::Game;
use rine_protocol::text::Text;
use std::path::{Path, PathBuf};

const VANILLA: &str = "vanilla";
const STATE_FILE: &str = "rine_datapacks.json";

#[derive(Clone)]
pub struct Pack {
    pub id: String,
    pub title: Text,
    pub desc: Text,
    pub built_in: bool,
}

struct State {
    enabled: Vec<String>,
    disabled: Vec<String>,
}

pub fn datapacks_dir(g: &Game) -> PathBuf {
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
pub fn available(g: &Game) -> Vec<Pack> {
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
pub fn selected(g: &Game) -> Vec<Pack> {
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
pub fn persist(g: &Game, sel: &[Pack]) {
    let enabled: Vec<String> = sel.iter().map(|p| p.id.clone()).collect();
    let disabled: Vec<String> = available(g).into_iter().map(|p| p.id).filter(|id| !enabled.contains(id)).collect();
    save(g, &State { enabled, disabled });
}

/// Folders of the selected file packs, in load order: later packs override earlier ones
/// (`MultiPackResourceManager` stacks packs in selection order, `FallbackResourceManager` keeps the
/// top one). A folder whose `pack.mcmeta` has no `pack.description` is not a pack
/// (`Pack.readPackMetadata` logs "Missing metadata" and skips it), so it loads nothing.
pub fn enabled_dirs(g: &Game) -> Vec<PathBuf> {
    let root = datapacks_dir(g);
    selected(g).into_iter().filter(|p| !p.built_in).filter_map(|p| p.id.strip_prefix("file/").map(|n| root.join(n))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(root: &Path, name: &str, mcmeta: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("data")).unwrap();
        std::fs::write(dir.join("pack.mcmeta"), mcmeta).unwrap();
    }

    #[test]
    fn enabled_dirs_follow_the_selection_order() {
        let g = crate::game::redstone::game_tests::setup("packs-order");
        let root = datapacks_dir(&g);
        let _ = std::fs::remove_dir_all(&root);
        let valid = r#"{"pack":{"description":"x","pack_format":121}}"#;
        pack(&root, "a", valid);
        pack(&root, "b", valid);
        pack(&root, "d", valid);
        pack(&root, "c", r#"{"pack":{"pack_format":121}}"#);
        let state = serde_json::json!({ "enabled": ["vanilla", "file/b", "file/a"], "disabled": [] });
        std::fs::write(state_path(&g), state.to_string()).unwrap();
        // b and a in the persisted order, then the new pack d; c has no description.
        assert_eq!(enabled_dirs(&g), vec![root.join("b"), root.join("a"), root.join("d")]);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(state_path(&g));
    }
}
