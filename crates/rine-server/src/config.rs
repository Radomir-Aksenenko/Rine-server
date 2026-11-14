use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub bind: String,
    pub motd: String,
    pub max_players: u32,
    pub online_mode: bool,
    pub view_distance: u8,
    pub simulation_distance: u8,
    pub compression_threshold: i32,
    pub seed: i64,
    pub world_dir: String,
    pub gamemode: String,
    pub spawn_protection: bool,
    pub plugins_dir: String,
    /// Operator nicknames (case-insensitive): permission level 4.
    pub ops: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0:25565".into(),
            motd: "Rine: сервер нового поколения".into(),
            max_players: 10000,
            online_mode: false,
            view_distance: 10,
            simulation_distance: 8,
            compression_threshold: 256,
            seed: 0x5EED_2026,
            world_dir: "world".into(),
            gamemode: "survival".into(),
            spawn_protection: false,
            plugins_dir: "plugins".into(),
            ops: Vec::new(),
        }
    }
}

impl Config {
    pub fn load_or_create(path: &str) -> Self {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!("bad {path}: {e}, using defaults");
                Self::default()
            }),
            Err(_) => {
                let c = Self::default();
                let _ = std::fs::write(path, toml::to_string_pretty(&c).unwrap());
                c
            }
        }
    }
}
