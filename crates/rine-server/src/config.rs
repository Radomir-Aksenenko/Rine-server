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
    /// server.properties `op-permission-level`: level given by /op.
    pub op_permission_level: u8,
    /// `function-permission-level`.
    pub function_permission_level: u8,
    /// `enable-command-block`.
    pub enable_command_block: bool,
    /// `broadcast-console-to-ops`.
    pub broadcast_console_to_ops: bool,
    /// `broadcast-rcon-to-ops`.
    pub broadcast_rcon_to_ops: bool,
    /// `white-list` / `enforce-whitelist`.
    pub white_list: bool,
    pub enforce_whitelist: bool,
    /// `difficulty` (peaceful, easy, normal, hard).
    pub difficulty: String,
    /// `player-idle-timeout` minutes (0 = off).
    pub player_idle_timeout: i32,
    /// New chunks get vanilla carvers and vegetation from `rine-worldgen`
    /// (about 14 ms per chunk in release, as fast as the legacy generator).
    /// Chunks generated before keep their terrain; `false` restores the legacy
    /// generator.
    pub vanilla_vegetation: bool,
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
            op_permission_level: 4,
            function_permission_level: 2,
            enable_command_block: false,
            broadcast_console_to_ops: true,
            broadcast_rcon_to_ops: true,
            white_list: false,
            enforce_whitelist: false,
            difficulty: "easy".into(),
            player_idle_timeout: 0,
            vanilla_vegetation: true,
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
