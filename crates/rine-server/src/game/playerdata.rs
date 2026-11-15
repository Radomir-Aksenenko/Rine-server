//! Player data: position, rotation, gamemode, health, hunger, held slot and
//! inventory, stored as JSON in `<world_dir>/playerdata/<uuid>.json`.
//!
//! Items are stored by registry name, so a file keeps working when numeric
//! ids change. Every field has a serde default and unknown fields are ignored,
//! so older and newer files both load. Experience is not modelled yet.
//!
//! Writes are atomic (temp file, then rename) and run on the rayon pool, so
//! the tick never waits on disk. A process-wide pending map holds the newest
//! snapshot per player until its file is in place: a rejoin right after a
//! leave reads that snapshot, and a write only renames over the file while it
//! is still the newest one for its player, so an older write can't win.

use super::items;
use super::player::{max_stack, Player, Stack};
use super::Game;
use ahash::AHashMap;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};
use uuid::Uuid;

const VERSION: u32 = 1;
/// Inventory slots in window-0 numbering.
const SLOTS: usize = 46;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerData {
    pub version: u32,
    pub pos: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub gamemode: u8,
    pub health: f32,
    pub food: i32,
    pub saturation: f32,
    /// Hunger exhaustion accumulated since the last food drop.
    pub exhaustion: f32,
    pub held: u8,
    /// Non-empty inventory slots only.
    pub slots: Vec<SlotData>,
    /// Item on the cursor when the player left.
    pub cursor: Option<ItemData>,
    /// Ender chest contents (`EnderItems`), non-empty slots only.
    pub ender: Vec<SlotData>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SlotData {
    pub slot: u8,
    #[serde(flatten)]
    pub item: ItemData,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemData {
    /// Registry name, e.g. `minecraft:iron_pickaxe`.
    pub id: String,
    pub count: i32,
    #[serde(skip_serializing_if = "is_zero")]
    pub damage: u16,
}

fn is_zero(v: &u16) -> bool {
    *v == 0
}

impl ItemData {
    fn of(s: &Stack) -> Self {
        Self { id: items::info(s.item).map(|i| i.name.clone()).unwrap_or_default(), count: s.count, damage: s.damage }
    }

    /// `None` for names this server does not know.
    fn stack(&self) -> Option<Stack> {
        let id = items::id(&self.id)?;
        Some(Stack::damaged(id, self.count.clamp(1, max_stack(id)), self.damage))
    }
}

impl PlayerData {
    /// Snapshot of `p` on the game thread. `exhaustion` comes from survival.
    pub fn capture(p: &Player, exhaustion: f32) -> Self {
        let slots = p
            .inv
            .slots
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.is_empty())
            .map(|(i, s)| SlotData { slot: i as u8, item: ItemData::of(s) })
            .collect();
        Self {
            version: VERSION,
            pos: p.pos,
            yaw: p.yaw,
            pitch: p.pitch,
            gamemode: p.gamemode,
            health: p.health,
            food: p.food,
            saturation: p.saturation,
            exhaustion,
            held: p.held,
            slots,
            cursor: (!p.inv.cursor.is_empty()).then(|| ItemData::of(&p.inv.cursor)),
            ender: super::containers::ender_items(p.uuid)
                .iter()
                .enumerate()
                .filter(|(_, s)| !s.is_empty())
                .map(|(i, s)| SlotData { slot: i as u8, item: ItemData::of(s) })
                .collect(),
        }
    }
}

/// Puts saved state onto a freshly constructed player. The caller sends the
/// packets afterwards, so the client sees the restored inventory and position.
/// Health 0 (died offline) comes back as full health.
pub fn apply(p: &mut Player, d: &PlayerData) {
    // a player who logged out dead comes back at the spawn point (kept from Player::new)
    if d.health > 0.0 {
        p.pos = d.pos;
        p.sent_pos = d.pos;
        p.fall_distance = 0.0;
        p.yaw = d.yaw;
        p.pitch = d.pitch;
    }
    p.gamemode = d.gamemode;
    p.health = if d.health > 0.0 { d.health.min(20.0) } else { 20.0 };
    p.food = d.food.clamp(0, 20);
    p.saturation = d.saturation.clamp(0.0, 20.0);
    p.held = d.held.min(8);
    for s in &d.slots {
        let i = s.slot as usize;
        if i >= SLOTS {
            continue;
        }
        match s.item.stack() {
            Some(st) => p.inv.slots[i] = st,
            None => tracing::warn!("playerdata {}: unknown item {}", p.name, s.item.id),
        }
    }
    p.inv.cursor = d.cursor.as_ref().and_then(|c| c.stack()).unwrap_or(Stack::EMPTY);
    let mut ender = vec![Stack::EMPTY; 27];
    for s in &d.ender {
        if let (Some(st), true) = (s.item.stack(), (s.slot as usize) < 27) {
            ender[s.slot as usize] = st;
        }
    }
    super::containers::set_ender(p.uuid, ender);
}

struct Pending {
    gen: u64,
    data: Arc<PlayerData>,
}

fn pending() -> MutexGuard<'static, AHashMap<Uuid, Pending>> {
    static P: OnceLock<Mutex<AHashMap<Uuid, Pending>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(AHashMap::new())).lock().unwrap_or_else(|e| e.into_inner())
}

static GEN: AtomicU64 = AtomicU64::new(1);
/// Writes handed to rayon and not finished yet.
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

fn file_of(world_dir: &str, uuid: Uuid) -> PathBuf {
    Path::new(world_dir).join("playerdata").join(format!("{uuid}.json"))
}

/// Saved state for `uuid`, if any. A corrupt file is moved aside and ignored.
pub fn read(world_dir: &str, uuid: Uuid) -> Option<PlayerData> {
    if let Some(p) = pending().get(&uuid) {
        return Some((*p.data).clone());
    }
    let path = file_of(world_dir, uuid);
    let text = fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<PlayerData>(&text) {
        Ok(d) => Some(d),
        Err(e) => {
            tracing::warn!("playerdata {} is unreadable ({e}), moved aside", path.display());
            let _ = fs::rename(&path, path.with_extension("json.bad"));
            None
        }
    }
}

/// Queues a save of `data` for `uuid`. `sync` writes on this thread (shutdown).
fn store(world_dir: &str, uuid: Uuid, data: PlayerData, sync: bool) {
    let gen = GEN.fetch_add(1, Ordering::Relaxed);
    let data = Arc::new(data);
    pending().insert(uuid, Pending { gen, data: data.clone() });
    let path = file_of(world_dir, uuid);
    if sync {
        write_job(&path, uuid, gen, &data);
    } else {
        IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
        super::world::spawn_background(move || {
            write_job(&path, uuid, gen, &data);
            IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

fn write_job(path: &Path, uuid: Uuid, gen: u64, data: &PlayerData) {
    let json = match serde_json::to_vec_pretty(data) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("playerdata {uuid}: cannot encode: {e}");
            return;
        }
    };
    let Some(dir) = path.parent() else { return };
    let tmp = dir.join(format!("{uuid}.{gen}.tmp"));
    let written = (|| -> std::io::Result<()> {
        fs::create_dir_all(dir)?;
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&json)?;
        f.sync_data()?;
        Ok(())
    })();
    if let Err(e) = written {
        tracing::warn!("playerdata {uuid}: cannot write {}: {e}", tmp.display());
        let _ = fs::remove_file(&tmp);
        return;
    }
    // check and rename under the lock: only the newest snapshot may land
    let mut m = pending();
    if matches!(m.get(&uuid), Some(p) if p.gen == gen) {
        match fs::rename(&tmp, path) {
            Ok(()) => {
                m.remove(&uuid);
            }
            Err(e) => {
                tracing::warn!("playerdata {uuid}: cannot replace {}: {e}", path.display());
                let _ = fs::remove_file(&tmp);
            }
        }
    } else {
        let _ = fs::remove_file(&tmp);
    }
}

/// Saves one player. `exhaustion` is the survival value for its slot.
pub fn save_player(world_dir: &str, p: &Player, exhaustion: f32, sync: bool) {
    store(world_dir, p.uuid, PlayerData::capture(p, exhaustion), sync);
}

/// Saves every online player (periodic save and shutdown).
pub fn save_all(g: &Game, sync: bool) {
    let dir = &g.shared.config.world_dir;
    for p in g.players.iter().flatten() {
        let exh = super::survival::exhaustion_of(p.slot, p.eid);
        save_player(dir, p, exh, sync);
    }
}

/// Waits (at most 10 s) until the queued background writes have finished.
pub fn flush() {
    let until = Instant::now() + Duration::from_secs(10);
    while IN_FLIGHT.load(Ordering::SeqCst) > 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_data_round_trips_damage_and_skips_zero() {
        let d = ItemData { id: "minecraft:iron_pickaxe".into(), count: 1, damage: 0 };
        let text = serde_json::to_string(&d).unwrap();
        assert!(!text.contains("damage"), "{text}");
        let back: ItemData = serde_json::from_str(r#"{"id":"minecraft:iron_pickaxe","count":1,"damage":37,"future":true}"#).unwrap();
        assert_eq!(back.damage, 37);
    }

    #[test]
    fn synchronous_save_lands_on_disk_and_reads_back() {
        rine_data::init();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("playerdata-test-{}", Uuid::new_v4()));
        let dir = root.to_string_lossy().to_string();
        let uuid = Uuid::new_v4();
        let mut d = PlayerData { gamemode: 1, held: 3, health: 12.5, ..Default::default() };
        d.slots.push(SlotData { slot: 39, item: ItemData { id: "minecraft:iron_pickaxe".into(), count: 1, damage: 9 } });
        store(&dir, uuid, d, true);
        // the pending map must not be the source of this read
        pending().remove(&uuid);
        let back = read(&dir, uuid).expect("saved file");
        assert_eq!(back.gamemode, 1);
        assert_eq!(back.held, 3);
        assert_eq!(back.slots[0].slot, 39);
        assert_eq!(back.slots[0].item.damage, 9);
        assert_eq!(back.slots[0].item.stack().unwrap().damage, 9);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_fields_take_defaults() {
        let d: PlayerData = serde_json::from_str(r#"{"gamemode":1}"#).unwrap();
        assert_eq!(d.gamemode, 1);
        assert!(d.slots.is_empty());
        assert!(d.cursor.is_none());
    }
}
