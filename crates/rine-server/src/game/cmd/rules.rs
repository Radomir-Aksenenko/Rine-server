//! Game rules (vanilla `GameRules`, 26.3 snake_case ids). Stored in
//! `<world>/gamerules.txt` as `name=value` lines.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value {
    Bool(bool),
    Int(i32),
}

pub struct RuleDef {
    pub name: &'static str,
    pub default: Value,
    pub min: i32,
    pub max: i32,
}

const fn b(name: &'static str, v: bool) -> RuleDef {
    RuleDef { name, default: Value::Bool(v), min: 0, max: 0 }
}
const fn i(name: &'static str, v: i32, min: i32, max: i32) -> RuleDef {
    RuleDef { name, default: Value::Int(v), min, max }
}

/// Registration order of `GameRules.java` (minus feature-flagged rules).
pub const RULES: &[RuleDef] = &[
    b("advance_time", true),
    b("advance_weather", true),
    b("allow_entering_nether_using_portals", true),
    b("block_drops", true),
    b("block_explosion_drop_decay", true),
    b("command_blocks_work", true),
    b("command_block_output", true),
    b("drowning_damage", true),
    b("elytra_movement_check", true),
    b("ender_pearls_vanish_on_death", true),
    b("entity_drops", true),
    b("fall_damage", true),
    b("fire_damage", true),
    i("fire_spread_radius_around_player", 128, -1, i32::MAX),
    b("forgive_dead_players", true),
    b("freeze_damage", true),
    b("global_sound_events", true),
    b("immediate_respawn", false),
    b("keep_inventory", false),
    b("lava_source_conversion", false),
    b("limited_crafting", false),
    b("locator_bar", true),
    b("log_admin_commands", true),
    i("max_block_modifications", 32768, 1, i32::MAX),
    i("max_command_forks", 65536, 0, i32::MAX),
    i("max_command_sequence_length", 65536, 0, i32::MAX),
    i("max_entity_cramming", 24, 0, i32::MAX),
    i("max_snow_accumulation_height", 1, 0, 8),
    b("mob_drops", true),
    b("mob_explosion_drop_decay", true),
    b("mob_griefing", true),
    b("natural_health_regeneration", true),
    b("player_movement_check", true),
    i("players_nether_portal_creative_delay", 0, 0, i32::MAX),
    i("players_nether_portal_default_delay", 80, 0, i32::MAX),
    i("players_sleeping_percentage", 100, 0, i32::MAX),
    b("projectiles_can_break_blocks", true),
    b("pvp", true),
    b("raids", true),
    i("random_tick_speed", 3, 0, i32::MAX),
    b("reduced_debug_info", false),
    i("respawn_radius", 10, 0, i32::MAX),
    b("send_command_feedback", true),
    b("show_advancement_messages", true),
    b("show_death_messages", true),
    b("spawner_blocks_work", true),
    b("spawn_mobs", true),
    b("spawn_monsters", true),
    b("spawn_patrols", true),
    b("spawn_phantoms", true),
    b("spawn_wandering_traders", true),
    b("spawn_wardens", true),
    b("spectators_generate_chunks", true),
    b("spread_vines", true),
    b("tnt_explodes", true),
    b("tnt_explosion_drop_decay", false),
    b("universal_anger", false),
    b("water_source_conversion", true),
];

pub struct GameRules {
    values: Vec<Value>,
    path: Option<std::path::PathBuf>,
}

impl GameRules {
    pub fn load(world_dir: &str) -> Self {
        let path = std::path::Path::new(world_dir).join("gamerules.txt");
        let mut values: Vec<Value> = RULES.iter().map(|r| r.default).collect();
        if let Ok(t) = std::fs::read_to_string(&path) {
            for l in t.lines() {
                if let Some((k, v)) = l.split_once('=') {
                    if let Some(i) = RULES.iter().position(|r| r.name == k.trim()) {
                        values[i] = match RULES[i].default {
                            Value::Bool(_) => Value::Bool(v.trim() == "true"),
                            Value::Int(d) => Value::Int(v.trim().parse().unwrap_or(d)),
                        };
                    }
                }
            }
        }
        Self { values, path: Some(path) }
    }

    pub fn defaults() -> Self {
        Self { values: RULES.iter().map(|r| r.default).collect(), path: None }
    }

    /// Index of a rule by id, with or without the `minecraft:` prefix.
    pub fn index(name: &str) -> Option<usize> {
        let n = name.strip_prefix("minecraft:").unwrap_or(name);
        RULES.iter().position(|r| r.name == n)
    }

    pub fn get(&self, idx: usize) -> Value {
        self.values[idx]
    }

    pub fn set(&mut self, idx: usize, v: Value) {
        self.values[idx] = v;
        self.save();
    }

    pub fn bool(&self, name: &str) -> bool {
        match Self::index(name).map(|i| self.values[i]) {
            Some(Value::Bool(b)) => b,
            _ => false,
        }
    }

    pub fn int(&self, name: &str) -> i32 {
        match Self::index(name).map(|i| self.values[i]) {
            Some(Value::Int(v)) => v,
            _ => 0,
        }
    }

    fn save(&self) {
        let Some(p) = &self.path else { return };
        let mut t = String::new();
        for (r, v) in RULES.iter().zip(&self.values) {
            if *v != r.default {
                match v {
                    Value::Bool(b) => t.push_str(&format!("{}={b}\n", r.name)),
                    Value::Int(i) => t.push_str(&format!("{}={i}\n", r.name)),
                }
            }
        }
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Err(e) = std::fs::write(p, t) {
            tracing::warn!("cannot write {}: {e}", p.display());
        }
    }
}
