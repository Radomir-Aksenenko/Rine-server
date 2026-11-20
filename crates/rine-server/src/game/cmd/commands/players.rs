//! Player list and server admin commands: /op, /deop, /ban, /ban-ip,
//! /banlist, /pardon, /pardon-ip, /whitelist, /kick, /list, /stop,
//! /save-all, /save-off, /save-on, /setidletimeout, /seed, /version, /help,
//! /transfer (`OpCommand`, `DeOpCommands`, `BanPlayerCommands`, ...).
//!
//! The lists live in `State`, behind a module static (no `Game` field yet).
//! They load from `ops.json`, `whitelist.json`, `banned-players.json` and
//! `banned-ips.json` in the working directory on first use, and every change
//! rewrites its file (vanilla `StoredUserList.save`).

mod files;
mod help;

use crate::game::cmd::{lit, select, Commands, Ctx};
use crate::game::playerdata;
use crate::game::Game;
use crate::out;
use crate::net::Out;
use rine_command::args::ParseEnv;
use rine_command::tree::ROOT;
use rine_command::{CmdError, CmdResult};
use rine_protocol::text::{click_copy, hover_text, Text};
use serde::Serialize;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use uuid::Uuid;

const OPS_FILE: &str = "ops.json";
const WHITELIST_FILE: &str = "whitelist.json";
const BANS_FILE: &str = "banned-players.json";
const IP_BANS_FILE: &str = "banned-ips.json";

/// `ClientboundTransferPacket` (play, `minecraft:transfer`): not in `ids.rs`.
const C_TRANSFER: i32 = 132;
/// `Date.toString()` of the build time in `version.json` (server time zone UTC).
const BUILD_TIME: &str = "Tue Sep 15 11:20:48 UTC 2026";

#[derive(Clone, Debug)]
pub struct Op {
    pub uuid: Uuid,
    pub name: String,
    pub level: u8,
    pub bypass: bool,
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub uuid: Uuid,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct Ban {
    pub uuid: Uuid,
    pub name: String,
    /// Epoch seconds.
    pub created: i64,
    pub source: String,
    pub expires: Option<i64>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug)]
pub struct IpBan {
    pub ip: String,
    pub created: i64,
    pub source: String,
    pub expires: Option<i64>,
    pub reason: Option<String>,
}

/// Player list state (`PlayerList` ops, whitelist, bans; `MinecraftServer`
/// whitelist, autosave and idle timeout flags).
#[derive(Default)]
pub struct State {
    pub ops: Vec<Op>,
    pub whitelist: Vec<Profile>,
    pub bans: Vec<Ban>,
    pub ip_bans: Vec<IpBan>,
    /// `white-list` (`MinecraftServer.isUsingWhitelist`).
    pub whitelist_on: bool,
    /// Autosave of the world (`setAutoSave`).
    pub autosave: bool,
    /// `setPlayerIdleTimeout`, minutes (0 = off).
    pub idle_timeout: i32,
    /// Address of each player at its last login (for IP bans).
    pub ips: HashMap<Uuid, String>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn load(g: &Game) -> State {
    let cfg = &g.shared.config;
    let now = files::now();
    let ops = files::read_list(OPS_FILE)
        .iter()
        .filter_map(|o| {
            Some(Op {
                uuid: Uuid::parse_str(&files::str_field(o, "uuid")?).ok()?,
                name: files::str_field(o, "name")?,
                level: o.get("level").and_then(|v| v.as_i64()).unwrap_or(0).clamp(0, 4) as u8,
                bypass: o.get("bypassesPlayerLimit").and_then(|v| v.as_bool()).unwrap_or(false),
            })
        })
        .collect();
    let whitelist = load_whitelist();
    let bans = files::read_list(BANS_FILE)
        .iter()
        .filter_map(|o| {
            let expires = files::str_field(o, "expires").and_then(|s| files::parse_date(&s));
            if expires.is_some_and(|e| e < now) {
                return None;
            }
            Some(Ban {
                uuid: Uuid::parse_str(&files::str_field(o, "uuid")?).ok()?,
                name: files::str_field(o, "name")?,
                created: files::str_field(o, "created").and_then(|s| files::parse_date(&s)).unwrap_or(now),
                source: files::str_field(o, "source").unwrap_or_else(|| "(Unknown)".into()),
                expires,
                reason: files::str_field(o, "reason"),
            })
        })
        .collect();
    let ip_bans = files::read_list(IP_BANS_FILE)
        .iter()
        .filter_map(|o| {
            let expires = files::str_field(o, "expires").and_then(|s| files::parse_date(&s));
            if expires.is_some_and(|e| e < now) {
                return None;
            }
            Some(IpBan {
                ip: files::str_field(o, "ip")?,
                created: files::str_field(o, "created").and_then(|s| files::parse_date(&s)).unwrap_or(now),
                source: files::str_field(o, "source").unwrap_or_else(|| "(Unknown)".into()),
                expires,
                reason: files::str_field(o, "reason"),
            })
        })
        .collect();
    State {
        ops,
        whitelist,
        bans,
        ip_bans,
        whitelist_on: cfg.white_list,
        autosave: true,
        idle_timeout: cfg.player_idle_timeout,
        ips: HashMap::new(),
    }
}

fn load_whitelist() -> Vec<Profile> {
    files::read_list(WHITELIST_FILE)
        .iter()
        .filter_map(|o| {
            Some(Profile {
                uuid: Uuid::parse_str(&files::str_field(o, "uuid")?).ok()?,
                name: files::str_field(o, "name")?,
            })
        })
        .collect()
}

/// Runs `f` on the state (loaded on first use). `f` must not call back into
/// this module (the lock is held).
fn with<R>(g: &Game, f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let st = guard.get_or_insert_with(|| load(g));
    f(st)
}

#[derive(Serialize)]
#[allow(non_snake_case)]
struct OpOut {
    uuid: String,
    name: String,
    level: u8,
    bypassesPlayerLimit: bool,
}

#[derive(Serialize)]
struct ProfileOut {
    uuid: String,
    name: String,
}

#[derive(Serialize)]
struct BanOut {
    uuid: String,
    name: String,
    created: String,
    source: String,
    expires: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Serialize)]
struct IpBanOut {
    ip: String,
    created: String,
    source: String,
    expires: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

fn expires_text(e: Option<i64>) -> String {
    e.map_or_else(|| "forever".to_string(), files::format_date)
}

fn save_ops(st: &State) {
    let v: Vec<OpOut> = st
        .ops
        .iter()
        .map(|o| OpOut { uuid: o.uuid.to_string(), name: o.name.clone(), level: o.level, bypassesPlayerLimit: o.bypass })
        .collect();
    files::write_list(OPS_FILE, &v);
}

fn save_whitelist(st: &State) {
    let v: Vec<ProfileOut> = st.whitelist.iter().map(|p| ProfileOut { uuid: p.uuid.to_string(), name: p.name.clone() }).collect();
    files::write_list(WHITELIST_FILE, &v);
}

fn save_bans(st: &State) {
    let now = files::now();
    let v: Vec<BanOut> = st
        .bans
        .iter()
        .filter(|b| !b.expires.is_some_and(|e| e < now))
        .map(|b| BanOut {
            uuid: b.uuid.to_string(),
            name: b.name.clone(),
            created: files::format_date(b.created),
            source: b.source.clone(),
            expires: expires_text(b.expires),
            reason: b.reason.clone(),
        })
        .collect();
    files::write_list(BANS_FILE, &v);
}

fn save_ip_bans(st: &State) {
    let now = files::now();
    let v: Vec<IpBanOut> = st
        .ip_bans
        .iter()
        .filter(|b| !b.expires.is_some_and(|e| e < now))
        .map(|b| IpBanOut {
            ip: b.ip.clone(),
            created: files::format_date(b.created),
            source: b.source.clone(),
            expires: expires_text(b.expires),
            reason: b.reason.clone(),
        })
        .collect();
    files::write_list(IP_BANS_FILE, &v);
}

/// Drops expired bans (vanilla removes them lazily on lookup and save).
fn purge(st: &mut State, now: i64) {
    st.bans.retain(|b| !b.expires.is_some_and(|e| e < now));
    st.ip_bans.retain(|b| !b.expires.is_some_and(|e| e < now));
}

fn reason_text(reason: &Option<String>) -> Text {
    match reason {
        Some(r) => Text::plain(r.clone()),
        None => Text::key("multiplayer.disconnect.banned.reason.default"),
    }
}

/// Op permission level of a player name: the `ops.json` entry, else the
/// legacy operators from `rine.toml` / `ops.txt` (`op-permission-level`).
pub fn op_level(g: &Game, name: &str) -> Option<u8> {
    let legacy = g.is_op(name).then(|| g.shared.config.op_permission_level.clamp(1, 4));
    with(g, |st| st.ops.iter().find(|o| o.name.eq_ignore_ascii_case(name)).map(|o| o.level)).or(legacy)
}

/// `PlayerList.isOp(NameAndId)`: by UUID, or a legacy operator name.
fn is_op_profile(g: &Game, uuid: Uuid, name: &str) -> bool {
    g.is_op(name) || with(g, |st| st.ops.iter().any(|o| o.uuid == uuid))
}

fn is_listed(g: &Game, uuid: Uuid) -> bool {
    with(g, |st| st.whitelist.iter().any(|p| p.uuid == uuid))
}

/// Known profile by name (online or seen before): (name, uuid).
pub fn cached_profile(g: &Game, name: &str) -> Option<(String, Uuid)> {
    if let Some(p) = g.find_player(name).and_then(|s| g.players.get(s)).and_then(|p| p.as_ref()) {
        return Some((p.name.clone(), p.uuid));
    }
    with(g, |st| {
        let known = st
            .ops
            .iter()
            .map(|o| (&o.name, o.uuid))
            .chain(st.whitelist.iter().map(|p| (&p.name, p.uuid)))
            .chain(st.bans.iter().map(|b| (&b.name, b.uuid)));
        for (n, u) in known {
            if n.eq_ignore_ascii_case(name) {
                return Some((n.clone(), u));
            }
        }
        None
    })
}

/// `PlayerList.canPlayerLogin`: the vanilla refusal for a login, or `None`
/// when the player may join. Wire it into login (before the join is queued).
/// It also records the address of the login, which IP bans and
/// `/ban-ip <player>` need later.
pub fn check_login(g: &Game, name: &str, uuid: Uuid, ip: &str) -> Option<Text> {
    let legacy_op = g.is_op(name);
    let now = files::now();
    with(g, |st| {
        st.ips.insert(uuid, ip.to_string());
        purge(st, now);
        if let Some(b) = st.bans.iter().find(|b| b.uuid == uuid) {
            let mut t = Text::tr("multiplayer.disconnect.banned.reason", vec![reason_text(&b.reason)]);
            if let Some(e) = b.expires {
                t = t.push(Text::tr("multiplayer.disconnect.banned.expiration", vec![Text::plain(files::format_expiry(e))]));
            }
            return Some(t);
        }
        let is_op = legacy_op || st.ops.iter().any(|o| o.uuid == uuid);
        if st.whitelist_on && !is_op && !st.whitelist.iter().any(|p| p.uuid == uuid) {
            return Some(Text::key("multiplayer.disconnect.not_whitelisted"));
        }
        if let Some(b) = st.ip_bans.iter().find(|b| b.ip == ip) {
            let mut t = Text::tr("multiplayer.disconnect.banned_ip.reason", vec![reason_text(&b.reason)]);
            if let Some(e) = b.expires {
                t = t.push(Text::tr("multiplayer.disconnect.banned_ip.expiration", vec![Text::plain(files::format_expiry(e))]));
            }
            return Some(t);
        }
        None
    })
}

fn slot_of_uuid(g: &Game, uuid: Uuid) -> Option<usize> {
    g.players.iter().position(|p| p.as_ref().is_some_and(|p| p.uuid == uuid))
}

fn ip_of_slot(g: &Game, slot: usize) -> Option<String> {
    let uuid = g.players.get(slot)?.as_ref()?.uuid;
    with(g, |st| st.ips.get(&uuid).cloned())
}

/// Sends the disconnect packet with a translatable reason and closes the
/// connection (`ServerGamePacketListenerImpl.disconnect`).
pub fn disconnect(g: &mut Game, slot: usize, reason: &Text) {
    let Some(p) = g.player(slot) else { return };
    p.send(out::disconnect(reason));
    p.flush();
    let _ = p.outbound.send(Out::Close);
    g.leave(slot);
}

/// `PlayerList.sendPlayerPermissionLevel`: entity event 24+level, then the
/// command tree for the new level.
pub fn send_permission_level(g: &mut Game, slot: usize) {
    let Some((eid, name)) = g.players.get(slot).and_then(|p| p.as_ref()).map(|p| (p.eid, p.name.clone())) else { return };
    let level = op_level(g, &name).unwrap_or(0);
    if let Some(p) = g.player(slot) {
        p.op = level > 0;
        p.send(out::entity_event(eid, 24 + level as i8));
    }
    crate::game::cmd::send_tree(g, slot);
}

/// `MinecraftServer.kickUnlistedPlayers`.
fn kick_unlisted(g: &mut Game) {
    if !g.shared.config.enforce_whitelist || !with(g, |st| st.whitelist_on) {
        return;
    }
    let online: Vec<(usize, Uuid, String)> =
        g.players.iter().enumerate().filter_map(|(s, p)| p.as_ref().map(|p| (s, p.uuid, p.name.clone()))).collect();
    let mut kick = Vec::new();
    for (s, uuid, name) in online {
        if !is_listed(g, uuid) && !is_op_profile(g, uuid, &name) {
            kick.push(s);
        }
    }
    for s in kick {
        disconnect(g, s, &Text::key("multiplayer.disconnect.not_whitelisted"));
    }
}

fn sep() -> Text {
    Text::plain(", ").color("gray")
}

fn reason_arg(x: &mut Ctx) -> CmdResult<Option<String>> {
    if x.has("reason") {
        let t = x.message("reason")?;
        Ok(Some(crate::game::cmd::render_plain(&t)))
    } else {
        Ok(None)
    }
}

pub fn register(c: &mut Commands) {
    c.exec("op targets", op_players);
    c.exec("deop targets", deop_players);
    c.exec("ban targets", |x| ban_players(x, None));
    c.exec("ban targets reason", ban_players_reason);
    c.exec("ban-ip target", ban_ip_or_name);
    c.exec("ban-ip target reason", ban_ip_or_name);
    c.exec("banlist", |x| banlist(x, 0));
    c.exec("banlist players", |x| banlist(x, 1));
    c.exec("banlist ips", |x| banlist(x, 2));
    c.exec("pardon targets", pardon_players);
    c.exec("pardon-ip target", pardon_ip);
    c.exec("whitelist on", whitelist_on);
    c.exec("whitelist off", whitelist_off);
    c.exec("whitelist list", whitelist_list);
    c.exec("whitelist add targets", whitelist_add);
    c.exec("whitelist remove targets", whitelist_remove);
    c.exec("whitelist reload", whitelist_reload);
    c.exec("kick targets", |x| kick_players(x, None));
    c.exec("kick targets reason", kick_reason);
    c.exec("list", |x| list_format(x, false));
    c.exec("list uuids", |x| list_format(x, true));
    c.exec("stop", stop);
    c.exec("save-all", |x| save_all(x, false));
    c.exec("save-all flush", |x| save_all(x, true));
    c.exec("save-off", save_off);
    c.exec("save-on", save_on);
    c.exec("setidletimeout minutes", setidletimeout);
    c.exec("seed", seed);
    c.exec("version", version);
    c.exec("help", help_all);
    c.exec("help command", help_cmd);
    c.exec("transfer hostname", |x| {
        let host = x.str("hostname").to_string();
        let slot = x.source_player()?;
        transfer_to(x, host, 25565, vec![slot])
    });
    c.exec("transfer hostname port", |x| {
        let host = x.str("hostname").to_string();
        let port = x.int("port");
        let slot = x.source_player()?;
        transfer_to(x, host, port, vec![slot])
    });
    c.exec("transfer hostname port players", |x| {
        let host = x.str("hostname").to_string();
        let port = x.int("port");
        let slots = x.players("players")?;
        transfer_to(x, host, port, slots)
    });
}

// ---- op / deop

fn op_players(x: &mut Ctx) -> CmdResult<i32> {
    let profiles = x.profiles("targets")?;
    let level = x.g.shared.config.op_permission_level.min(4);
    let mut count = 0;
    for (name, uuid) in profiles {
        if is_op_profile(x.g, uuid, &name) {
            continue;
        }
        with(x.g, |st| {
            st.ops.retain(|o| o.uuid != uuid);
            st.ops.push(Op { uuid, name: name.clone(), level, bypass: false });
            save_ops(st);
        });
        if let Some(slot) = slot_of_uuid(x.g, uuid) {
            send_permission_level(x.g, slot);
        }
        count += 1;
        x.success(Text::tr("commands.op.success", vec![Text::plain(name)]), true);
    }
    if count == 0 {
        Err(CmdError::key("commands.op.failed"))
    } else {
        Ok(count)
    }
}

fn deop_players(x: &mut Ctx) -> CmdResult<i32> {
    let profiles = x.profiles("targets")?;
    let mut count = 0;
    for (name, uuid) in profiles {
        if !is_op_profile(x.g, uuid, &name) {
            continue;
        }
        let removed = with(x.g, |st| {
            let before = st.ops.len();
            st.ops.retain(|o| o.uuid != uuid);
            let removed = st.ops.len() != before;
            if removed {
                save_ops(st);
            }
            removed
        });
        if removed {
            if let Some(slot) = slot_of_uuid(x.g, uuid) {
                send_permission_level(x.g, slot);
            }
        }
        count += 1;
        x.success(Text::tr("commands.deop.success", vec![Text::plain(name)]), true);
    }
    if count == 0 {
        return Err(CmdError::key("commands.deop.failed"));
    }
    kick_unlisted(x.g);
    Ok(count)
}

// ---- ban / ban-ip / banlist / pardon

fn ban_players_reason(x: &mut Ctx) -> CmdResult<i32> {
    let reason = reason_arg(x)?;
    ban_players(x, reason)
}

fn ban_players(x: &mut Ctx, reason: Option<String>) -> CmdResult<i32> {
    let profiles = x.profiles("targets")?;
    let source = x.src.text_name.to_string();
    let now = files::now();
    let mut count = 0;
    for (name, uuid) in profiles {
        let banned = with(x.g, |st| {
            purge(st, now);
            st.bans.iter().any(|b| b.uuid == uuid)
        });
        if banned {
            continue;
        }
        with(x.g, |st| {
            st.bans.push(Ban { uuid, name: name.clone(), created: now, source: source.clone(), expires: None, reason: reason.clone() });
            save_bans(st);
        });
        count += 1;
        x.success(Text::tr("commands.ban.success", vec![Text::plain(name), reason_text(&reason)]), true);
        if let Some(slot) = slot_of_uuid(x.g, uuid) {
            disconnect(x.g, slot, &Text::key("multiplayer.disconnect.banned"));
        }
    }
    if count == 0 {
        Err(CmdError::key("commands.ban.failed"))
    } else {
        Ok(count)
    }
}

fn ban_ip_or_name(x: &mut Ctx) -> CmdResult<i32> {
    let target = x.str("target").to_string();
    let reason = reason_arg(x)?;
    if target.parse::<IpAddr>().is_ok() {
        return ban_ip(x, target, reason);
    }
    let ip = x.g.find_player(&target).and_then(|slot| ip_of_slot(x.g, slot));
    match ip {
        Some(ip) => ban_ip(x, ip, reason),
        None => Err(CmdError::key("commands.banip.invalid")),
    }
}

fn ban_ip(x: &mut Ctx, ip: String, reason: Option<String>) -> CmdResult<i32> {
    let now = files::now();
    let banned = with(x.g, |st| {
        purge(st, now);
        st.ip_bans.iter().any(|b| b.ip == ip)
    });
    if banned {
        return Err(CmdError::key("commands.banip.failed"));
    }
    let g: &Game = x.g;
    let slots: Vec<usize> = (0..g.players.len()).filter(|&s| ip_of_slot(g, s).as_deref() == Some(ip.as_str())).collect();
    let source = x.src.text_name.to_string();
    with(x.g, |st| {
        st.ip_bans.push(IpBan { ip: ip.clone(), created: now, source: source.clone(), expires: None, reason: reason.clone() });
        save_ip_bans(st);
    });
    x.success(Text::tr("commands.banip.success", vec![lit(&ip), reason_text(&reason)]), true);
    if !slots.is_empty() {
        let eids: Vec<i32> = slots.iter().filter_map(|&s| x.g.players[s].as_ref().map(|p| p.eid)).collect();
        let names = select::join_names(x.g, &eids, None);
        x.success(Text::tr("commands.banip.info", vec![lit(slots.len()), names]), true);
    }
    for &s in &slots {
        disconnect(x.g, s, &Text::key("multiplayer.disconnect.ip_banned"));
    }
    Ok(slots.len() as i32)
}

/// `BanListCommands`: 0 all (players then IPs), 1 players, 2 IPs.
fn banlist(x: &mut Ctx, which: u8) -> CmdResult<i32> {
    let now = files::now();
    let rows: Vec<(Text, String, Option<String>)> = with(x.g, |st| {
        purge(st, now);
        let mut v = Vec::new();
        if which != 2 {
            for b in &st.bans {
                v.push((Text::plain(b.name.clone()), b.source.clone(), b.reason.clone()));
            }
        }
        if which != 1 {
            for b in &st.ip_bans {
                v.push((Text::plain(b.ip.clone()), b.source.clone(), b.reason.clone()));
            }
        }
        v
    });
    if rows.is_empty() {
        x.success(Text::key("commands.banlist.none"), false);
        return Ok(0);
    }
    let n = rows.len();
    x.success(Text::tr("commands.banlist.list", vec![lit(n)]), false);
    for (disp, source, reason) in rows {
        x.success(Text::tr("commands.banlist.entry", vec![disp, Text::plain(source), reason_text(&reason)]), false);
    }
    Ok(n as i32)
}

fn pardon_players(x: &mut Ctx) -> CmdResult<i32> {
    let profiles = x.profiles("targets")?;
    let now = files::now();
    let mut count = 0;
    for (name, uuid) in profiles {
        let removed = with(x.g, |st| {
            purge(st, now);
            let before = st.bans.len();
            st.bans.retain(|b| b.uuid != uuid);
            let removed = st.bans.len() != before;
            if removed {
                save_bans(st);
            }
            removed
        });
        if removed {
            count += 1;
            x.success(Text::tr("commands.pardon.success", vec![Text::plain(name)]), true);
        }
    }
    if count == 0 {
        Err(CmdError::key("commands.pardon.failed"))
    } else {
        Ok(count)
    }
}

fn pardon_ip(x: &mut Ctx) -> CmdResult<i32> {
    let ip = x.str("target").to_string();
    if ip.parse::<IpAddr>().is_err() {
        return Err(CmdError::key("commands.pardonip.invalid"));
    }
    let now = files::now();
    let removed = with(x.g, |st| {
        purge(st, now);
        let before = st.ip_bans.len();
        st.ip_bans.retain(|b| b.ip != ip);
        let removed = st.ip_bans.len() != before;
        if removed {
            save_ip_bans(st);
        }
        removed
    });
    if !removed {
        return Err(CmdError::key("commands.pardonip.failed"));
    }
    x.success(Text::tr("commands.pardonip.success", vec![lit(&ip)]), true);
    Ok(1)
}

// ---- whitelist

fn whitelist_on(x: &mut Ctx) -> CmdResult<i32> {
    let was = with(x.g, |st| std::mem::replace(&mut st.whitelist_on, true));
    if was {
        return Err(CmdError::key("commands.whitelist.alreadyOn"));
    }
    x.success(Text::key("commands.whitelist.enabled"), true);
    kick_unlisted(x.g);
    Ok(1)
}

fn whitelist_off(x: &mut Ctx) -> CmdResult<i32> {
    let was = with(x.g, |st| std::mem::replace(&mut st.whitelist_on, false));
    if !was {
        return Err(CmdError::key("commands.whitelist.alreadyOff"));
    }
    x.success(Text::key("commands.whitelist.disabled"), true);
    Ok(1)
}

fn whitelist_list(x: &mut Ctx) -> CmdResult<i32> {
    let names: Vec<String> = with(x.g, |st| st.whitelist.iter().map(|p| p.name.clone()).collect());
    if names.is_empty() {
        x.success(Text::key("commands.whitelist.none"), false);
    } else {
        x.success(Text::tr("commands.whitelist.list", vec![lit(names.len()), lit(names.join(", "))]), false);
    }
    Ok(names.len() as i32)
}

fn whitelist_add(x: &mut Ctx) -> CmdResult<i32> {
    let profiles = x.profiles("targets")?;
    let mut count = 0;
    for (name, uuid) in profiles {
        if is_listed(x.g, uuid) {
            continue;
        }
        with(x.g, |st| {
            st.whitelist.push(Profile { uuid, name: name.clone() });
            save_whitelist(st);
        });
        count += 1;
        x.success(Text::tr("commands.whitelist.add.success", vec![Text::plain(name)]), true);
    }
    if count == 0 {
        Err(CmdError::key("commands.whitelist.add.failed"))
    } else {
        Ok(count)
    }
}

fn whitelist_remove(x: &mut Ctx) -> CmdResult<i32> {
    let profiles = x.profiles("targets")?;
    let mut count = 0;
    for (name, uuid) in profiles {
        if !is_listed(x.g, uuid) {
            continue;
        }
        with(x.g, |st| {
            st.whitelist.retain(|p| p.uuid != uuid);
            save_whitelist(st);
        });
        count += 1;
        x.success(Text::tr("commands.whitelist.remove.success", vec![Text::plain(name)]), true);
    }
    if count == 0 {
        return Err(CmdError::key("commands.whitelist.remove.failed"));
    }
    kick_unlisted(x.g);
    Ok(count)
}

fn whitelist_reload(x: &mut Ctx) -> CmdResult<i32> {
    let list = load_whitelist();
    with(x.g, |st| st.whitelist = list);
    x.success(Text::key("commands.whitelist.reloaded"), true);
    kick_unlisted(x.g);
    Ok(1)
}

// ---- kick / list

fn kick_reason(x: &mut Ctx) -> CmdResult<i32> {
    let reason = x.message("reason")?;
    kick_players(x, Some(reason))
}

fn kick_players(x: &mut Ctx, reason: Option<Text>) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    let reason = reason.unwrap_or_else(|| Text::key("multiplayer.disconnect.kicked"));
    let mut count = 0;
    for slot in slots {
        let name = x.player_name(slot);
        if x.g.players.get(slot).and_then(|p| p.as_ref()).is_none() {
            continue;
        }
        disconnect(x.g, slot, &reason);
        x.success(Text::tr("commands.kick.success", vec![name, reason.clone()]), true);
        count += 1;
    }
    if count == 0 {
        Err(CmdError::key("commands.kick.owner.failed"))
    } else {
        Ok(count)
    }
}

/// `ListPlayersCommand.format`.
fn list_format(x: &mut Ctx, uuids: bool) -> CmdResult<i32> {
    let slots: Vec<usize> = x.g.players.iter().enumerate().filter(|(_, p)| p.is_some()).map(|(i, _)| i).collect();
    let mut items = Vec::with_capacity(slots.len());
    for &slot in &slots {
        let item = if uuids {
            match x.g.players.get(slot).and_then(|p| p.as_ref()) {
                Some(p) => Text::tr("commands.list.nameAndId", vec![Text::plain(p.name.clone()), lit(p.uuid)]),
                None => Text::empty(),
            }
        } else {
            x.player_name(slot)
        };
        items.push(item);
    }
    let list = crate::game::cmd::feedback::format_list(items, &sep());
    let n = slots.len();
    let max = x.g.shared.config.max_players;
    x.success(Text::tr("commands.list.players", vec![lit(n), lit(max), list]), false);
    Ok(n as i32)
}

// ---- server

fn stop(x: &mut Ctx) -> CmdResult<i32> {
    x.success(Text::key("commands.stop.stopping"), true);
    x.g.running.store(false, Ordering::Relaxed);
    Ok(1)
}

fn save_all(x: &mut Ctx, flush: bool) -> CmdResult<i32> {
    x.success(Text::key("commands.save.saving"), false);
    // background writes; `flush` waits for them below
    playerdata::save_all(x.g, false);
    if flush {
        x.g.world.save_all_blocking();
        playerdata::flush();
    } else {
        x.g.world.save_all();
    }
    x.success(Text::key("commands.save.success"), true);
    Ok(1)
}

/// `setAutoSave`: true when the state changed.
fn set_autosave(g: &Game, on: bool) -> bool {
    with(g, |st| {
        let changed = st.autosave != on;
        st.autosave = on;
        changed
    })
}

fn save_off(x: &mut Ctx) -> CmdResult<i32> {
    if !set_autosave(x.g, false) {
        return Err(CmdError::key("commands.save.alreadyOff"));
    }
    x.success(Text::key("commands.save.disabled"), true);
    Ok(1)
}

fn save_on(x: &mut Ctx) -> CmdResult<i32> {
    if !set_autosave(x.g, true) {
        return Err(CmdError::key("commands.save.alreadyOn"));
    }
    x.success(Text::key("commands.save.enabled"), true);
    Ok(1)
}

fn setidletimeout(x: &mut Ctx) -> CmdResult<i32> {
    let t = x.int("minutes");
    with(x.g, |st| st.idle_timeout = t);
    if t > 0 {
        x.success(Text::tr("commands.setidletimeout.success", vec![lit(t)]), true);
    } else {
        x.success(Text::key("commands.setidletimeout.success.disabled"), true);
    }
    Ok(t)
}

/// `ComponentUtils.copyOnClickText`.
fn copy_on_click(s: &str) -> Text {
    let inner = Text::plain(s)
        .color("green")
        .on_click(click_copy(s))
        .on_hover(hover_text(&Text::key("chat.copy.click")))
        .insertion(s);
    Text::tr("chat.square_brackets", vec![inner])
}

fn seed(x: &mut Ctx) -> CmdResult<i32> {
    let seed = x.g.shared.config.seed;
    let text = copy_on_click(&seed.to_string());
    x.success(Text::tr("commands.seed.success", vec![text]), false);
    Ok(seed as i32)
}

fn version(x: &mut Ctx) -> CmdResult<i32> {
    x.system(Text::key("commands.version.header"));
    x.system(Text::tr("commands.version.id", vec![lit("26.3")]));
    x.system(Text::tr("commands.version.name", vec![lit("26.3")]));
    x.system(Text::tr("commands.version.data", vec![lit(5023)]));
    x.system(Text::tr("commands.version.series", vec![lit("main")]));
    x.system(Text::tr("commands.version.protocol", vec![lit(777), lit("0x309")]));
    x.system(Text::tr("commands.version.build_time", vec![lit(BUILD_TIME)]));
    x.system(Text::tr("commands.version.pack.resource", vec![lit("97.1")]));
    x.system(Text::tr("commands.version.pack.data", vec![lit("121.0")]));
    x.system(Text::key("commands.version.stable.yes"));
    Ok(1)
}

// ---- help

fn help_all(x: &mut Ctx) -> CmdResult<i32> {
    let tree = x.g.cmds.tree.clone();
    let lines = help::smart_children(&tree, ROOT, x.src.level);
    for l in &lines {
        x.success(Text::plain(format!("/{l}")), false);
    }
    Ok(lines.len() as i32)
}

fn help_cmd(x: &mut Ctx) -> CmdResult<i32> {
    let input = x.str("command").to_string();
    let tree = x.g.cmds.tree.clone();
    let res = rine_command::parse(&tree, &input, &ParseEnv::new(x.src.level));
    let Some(&(last, _, _)) = res.context.nodes.last() else {
        return Err(CmdError::key("commands.help.failed"));
    };
    let lines = help::smart_children(&tree, last, x.src.level);
    for l in &lines {
        x.success(Text::plain(format!("/{input} {l}")), false);
    }
    Ok(lines.len() as i32)
}

// ---- transfer

/// `ClientboundTransferPacket`: hostname (UTF-8 string), port (VarInt).
fn transfer_packet(host: &str, port: i32) -> bytes::BytesMut {
    rine_protocol::frame::packet(C_TRANSFER, |b| {
        use rine_protocol::Writer;
        b.w_str(host);
        b.w_varint(port);
    })
}

fn transfer_to(x: &mut Ctx, host: String, port: i32, slots: Vec<usize>) -> CmdResult<i32> {
    if slots.is_empty() {
        return Err(CmdError::key("commands.transfer.error.no_players"));
    }
    for &slot in &slots {
        let pk = transfer_packet(&host, port);
        if let Some(p) = x.g.player(slot) {
            p.send(pk);
        }
    }
    let n = slots.len();
    let text = if n == 1 {
        let name = x.player_name(slots[0]);
        Text::tr("commands.transfer.success.single", vec![name, lit(&host), lit(port)])
    } else {
        Text::tr("commands.transfer.success.multiple", vec![lit(n), lit(&host), lit(port)])
    };
    x.success(text, true);
    Ok(n as i32)
}
