//! Scoreboard: objectives, scores, teams, display slots (`Scoreboard`,
//! `ServerScoreboard`, `PlayerTeam`), and the commands /scoreboard, /team,
//! /trigger, /teammsg (+ /tm, which shares the teammsg node in the tree).
//!
//! Every player is on one board, so board packets go to all players through
//! `g.broadcast`. Like `ServerScoreboard`, score updates are only sent for
//! objectives shown in a display slot ("tracked"). Save/load use the vanilla
//! `data/scoreboard.dat` layout (gzip NBT).

use crate::game::cmd::feedback;
use crate::game::cmd::select;
use crate::game::cmd::{lit, Commands, Ctx};
use crate::game::Game;
use crate::out;
use ahash::AHashMap;
use bytes::{Bytes, BytesMut};
use rine_command::args::misc::Operation;
use rine_command::args::Arg;
use rine_command::{CmdError, CmdResult};
use rine_protocol::frame::packet;
use rine_protocol::nbt::Tag;
use rine_protocol::text::{click_suggest, hover_text, Content, Text};
use rine_protocol::{Reader, Writer};
use std::path::Path;

// Clientbound play packet ids (protocol 777, vanilla packets.json). Not in
// `rine_protocol::ids::play` yet.
const C_DISGUISED_CHAT: i32 = 33;
const C_RESET_SCORE: i32 = 80;
const C_SET_DISPLAY_OBJECTIVE: i32 = 100;
const C_SET_OBJECTIVE: i32 = 108;
const C_SET_PLAYER_TEAM: i32 = 111;
const C_SET_SCORE: i32 = 112;

/// `DataVersion` of 26.3 saves (read from `vanilla-data/.../scoreboard.dat`).
const DATA_VERSION: i32 = 5023;

/// `DisplaySlot` serialized names, indexed by slot id.
pub const SLOT_NAMES: [&str; 19] = [
    "list",
    "sidebar",
    "below_name",
    "sidebar.team.black",
    "sidebar.team.dark_blue",
    "sidebar.team.dark_green",
    "sidebar.team.dark_aqua",
    "sidebar.team.dark_red",
    "sidebar.team.dark_purple",
    "sidebar.team.gold",
    "sidebar.team.gray",
    "sidebar.team.dark_gray",
    "sidebar.team.blue",
    "sidebar.team.green",
    "sidebar.team.aqua",
    "sidebar.team.red",
    "sidebar.team.light_purple",
    "sidebar.team.yellow",
    "sidebar.team.white",
];
/// `TeamColor` names, indexed by id (also the `ChatFormatting` colour ids).
const COLOR_NAMES: [&str; 16] = [
    "black",
    "dark_blue",
    "dark_green",
    "dark_aqua",
    "dark_red",
    "dark_purple",
    "gold",
    "gray",
    "dark_gray",
    "blue",
    "green",
    "aqua",
    "red",
    "light_purple",
    "yellow",
    "white",
];
/// `Team.Visibility` names, indexed by id.
const VIS_NAMES: [&str; 4] = ["always", "never", "hideForOtherTeams", "hideForOwnTeam"];
/// `Team.CollisionRule` names, indexed by id.
const COLL_NAMES: [&str; 4] = ["always", "never", "pushOtherTeams", "pushOwnTeam"];
/// `ObjectiveCriteria.RenderType` ids: 0 integer, 1 hearts.
const RENDER_NAMES: [&str; 2] = ["integer", "hearts"];

/// Team packet methods (`ClientboundSetPlayerTeamPacket`).
const TEAM_ADD: u8 = 0;
const TEAM_REMOVE: u8 = 1;
const TEAM_CHANGE: u8 = 2;
const TEAM_JOIN: u8 = 3;
const TEAM_LEAVE: u8 = 4;
/// Objective packet methods (`ClientboundSetObjectivePacket`).
const OBJ_ADD: u8 = 0;
const OBJ_REMOVE: u8 = 1;
const OBJ_CHANGE: u8 = 2;

pub fn register(c: &mut Commands) {
    c.exec("scoreboard objectives list", objectives_list);
    c.exec("scoreboard objectives add objective criteria", objectives_add);
    c.exec("scoreboard objectives add objective criteria displayName", objectives_add_named);
    c.exec("scoreboard objectives remove objective", objectives_remove);
    c.exec("scoreboard objectives modify objective displayname displayName", modify_displayname);
    c.exec("scoreboard objectives modify objective rendertype integer", |x| modify_rendertype(x, 0));
    c.exec("scoreboard objectives modify objective rendertype hearts", |x| modify_rendertype(x, 1));
    c.exec("scoreboard objectives modify objective displayautoupdate value", modify_autoupdate);
    c.exec("scoreboard objectives modify objective numberformat", |x| objective_format(x, None));
    c.exec("scoreboard objectives modify objective numberformat blank", |x| objective_format(x, Some(NumFmt::Blank)));
    c.exec("scoreboard objectives modify objective numberformat fixed contents", objective_format_fixed);
    c.exec("scoreboard objectives modify objective numberformat styled style", objective_format_styled);
    c.exec("scoreboard objectives setdisplay slot", display_clear);
    c.exec("scoreboard objectives setdisplay slot objective", display_set);

    c.exec("scoreboard players list", players_list);
    c.exec("scoreboard players list target", players_list_target);
    c.exec("scoreboard players set targets objective score", players_set);
    c.exec("scoreboard players get target objective", players_get);
    c.exec("scoreboard players add targets objective score", players_add);
    c.exec("scoreboard players remove targets objective score", players_remove);
    c.exec("scoreboard players reset targets", players_reset_all);
    c.exec("scoreboard players reset targets objective", players_reset);
    c.exec("scoreboard players enable targets objective", players_enable);
    c.exec("scoreboard players display name targets objective", display_name_clear);
    c.exec("scoreboard players display name targets objective name", display_name_set);
    c.exec("scoreboard players display numberformat targets objective", |x| score_format(x, None));
    c.exec("scoreboard players display numberformat targets objective blank", |x| score_format(x, Some(NumFmt::Blank)));
    c.exec("scoreboard players display numberformat targets objective fixed contents", score_format_fixed);
    c.exec("scoreboard players display numberformat targets objective styled style", score_format_styled);
    c.exec("scoreboard players operation targets targetObjective operation source sourceObjective", players_operation);

    c.exec("team list", team_list);
    c.exec("team list team", team_list_members);
    c.exec("team add team", team_add);
    c.exec("team add team displayName", team_add_named);
    c.exec("team remove team", team_remove);
    c.exec("team empty team", team_empty);
    c.exec("team join team", team_join_self);
    c.exec("team join team members", team_join);
    c.exec("team leave members", team_leave);
    c.exec("team modify team displayName displayName", team_set_display);
    c.exec("team modify team color reset", team_clear_color);
    c.exec("team modify team color value", team_set_color);
    c.exec("team modify team friendlyFire allowed", team_friendly_fire);
    c.exec("team modify team seeFriendlyInvisibles allowed", team_see_invisibles);
    c.exec("team modify team nametagVisibility never", |x| team_set_visibility(x, 1, 0));
    c.exec("team modify team nametagVisibility hideForOtherTeams", |x| team_set_visibility(x, 2, 0));
    c.exec("team modify team nametagVisibility hideForOwnTeam", |x| team_set_visibility(x, 3, 0));
    c.exec("team modify team nametagVisibility always", |x| team_set_visibility(x, 0, 0));
    c.exec("team modify team deathMessageVisibility never", |x| team_set_visibility(x, 1, 1));
    c.exec("team modify team deathMessageVisibility hideForOtherTeams", |x| team_set_visibility(x, 2, 1));
    c.exec("team modify team deathMessageVisibility hideForOwnTeam", |x| team_set_visibility(x, 3, 1));
    c.exec("team modify team deathMessageVisibility always", |x| team_set_visibility(x, 0, 1));
    c.exec("team modify team collisionRule never", |x| team_set_collision(x, 1));
    c.exec("team modify team collisionRule pushOwnTeam", |x| team_set_collision(x, 3));
    c.exec("team modify team collisionRule pushOtherTeams", |x| team_set_collision(x, 2));
    c.exec("team modify team collisionRule always", |x| team_set_collision(x, 0));
    c.exec("team modify team prefix prefix", team_prefix);
    c.exec("team modify team suffix suffix", team_suffix);

    c.exec("trigger objective", trigger_simple);
    c.exec("trigger objective add value", trigger_add);
    c.exec("trigger objective set value", trigger_set);

    c.exec("teammsg message", teammsg);
}

// ---------------------------------------------------------------------------
// Public types and hooks
// ---------------------------------------------------------------------------

/// `ScoreHolder`: scoreboard name plus the entity it came from (for the
/// feedback display name).
#[derive(Debug, Clone, PartialEq)]
pub struct Holder {
    pub name: String,
    pub entity: Option<i32>,
}

impl Holder {
    pub fn named(name: impl Into<String>) -> Self {
        Self { name: name.into(), entity: None }
    }
    /// `getFeedbackDisplayName()`: entity display name or the literal name.
    pub fn display(&self, g: &Game) -> Text {
        match self.entity {
            Some(e) => feedback::display_name(g, e),
            None => Text::plain(self.name.clone()),
        }
    }
}

/// Number format of a score or objective (`NumberFormat` implementations).
#[derive(Clone, Debug, PartialEq)]
pub enum NumFmt {
    Blank,
    Fixed(Text),
    /// Style compound (`StyledFormat`).
    Styled(Tag),
}

impl NumFmt {
    /// `NumberFormat.format(value)`.
    pub fn format(&self, value: i32) -> Text {
        match self {
            NumFmt::Blank => Text::empty(),
            NumFmt::Fixed(t) => t.clone(),
            NumFmt::Styled(tag) => {
                let mut t = select::text_from_tag(tag);
                t.content = Content::Text;
                t.text = value.to_string();
                t
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Criteria {
    pub name: String,
    pub trigger: bool,
    pub read_only: bool,
    /// Default render type is hearts.
    pub hearts: bool,
}

/// `ObjectiveCriteria` by name (registered customs and stat criteria).
fn criteria_of(name: &str) -> Criteria {
    let (read_only, hearts) = match name {
        "health" => (true, true),
        "food" | "air" | "armor" | "xp" | "level" => (true, false),
        _ => (false, false),
    };
    Criteria { name: name.to_string(), trigger: name == "trigger", read_only, hearts }
}

#[derive(Clone, Debug)]
pub struct Objective {
    pub name: String,
    pub criteria: Criteria,
    pub display: Text,
    /// 0 integer, 1 hearts.
    pub render: u8,
    pub auto_update: bool,
    pub format: Option<NumFmt>,
}

#[derive(Clone, Debug)]
pub struct Score {
    pub value: i32,
    pub locked: bool,
    pub display: Option<Text>,
    pub format: Option<NumFmt>,
}

impl Default for Score {
    fn default() -> Self {
        Self { value: 0, locked: true, display: None, format: None }
    }
}

#[derive(Clone, Debug)]
pub struct Team {
    pub name: String,
    pub display: Text,
    pub prefix: Text,
    pub suffix: Text,
    pub friendly_fire: bool,
    pub see_invisibles: bool,
    /// `Team.Visibility` id.
    pub name_tag: u8,
    /// `Team.Visibility` id.
    pub death_msg: u8,
    /// `Team.CollisionRule` id.
    pub collision: u8,
    /// `TeamColor` id.
    pub color: Option<u8>,
    pub members: Vec<String>,
}

impl Team {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            display: Text::plain(name),
            prefix: Text::empty(),
            suffix: Text::empty(),
            friendly_fire: true,
            see_invisibles: true,
            name_tag: 0,
            death_msg: 0,
            collision: 0,
            color: None,
            members: Vec::new(),
        }
    }
}

/// Objectives, scores, teams, display slots (`Scoreboard` plus the
/// `ServerScoreboard` tracking state, which is derived from the slots).
#[derive(Default)]
pub struct State {
    objs: Vec<Option<Objective>>,
    obj_idx: AHashMap<String, u32>,
    holder_idx: AHashMap<String, u32>,
    holder_names: Vec<String>,
    scores: AHashMap<(u32, u32), Score>,
    /// Objectives with a score for each holder.
    holder_objs: AHashMap<u32, Vec<u32>>,
    displays: [Option<u32>; 19],
    teams: Vec<Option<Team>>,
    team_idx: AHashMap<String, u32>,
    /// Holder name to team index.
    member_team: AHashMap<String, u32>,
}

impl State {
    fn obj_id(&self, name: &str) -> Option<u32> {
        self.obj_idx.get(name).copied()
    }
    fn obj(&self, id: u32) -> Option<&Objective> {
        self.objs.get(id as usize).and_then(|o| o.as_ref())
    }
    fn holder_id(&mut self, name: &str) -> u32 {
        if let Some(&i) = self.holder_idx.get(name) {
            return i;
        }
        let i = self.holder_names.len() as u32;
        self.holder_names.push(name.to_string());
        self.holder_idx.insert(name.to_string(), i);
        i
    }
    fn holder_name(&self, hid: u32) -> &str {
        self.holder_names.get(hid as usize).map(|s| s.as_str()).unwrap_or("")
    }
    fn display_count(&self, oid: u32) -> usize {
        self.displays.iter().filter(|d| **d == Some(oid)).count()
    }
    fn tracked(&self, oid: u32) -> bool {
        self.display_count(oid) > 0
    }
    fn any_tracked(&self) -> bool {
        self.displays.iter().any(|d| d.is_some())
    }
    fn team(&self, tid: u32) -> Option<&Team> {
        self.teams.get(tid as usize).and_then(|t| t.as_ref())
    }
    fn team_by_name(&self, name: &str) -> Option<u32> {
        self.team_idx.get(name).copied()
    }
    fn team_of_holder(&self, holder: &str) -> Option<&Team> {
        self.member_team.get(holder).and_then(|t| self.team(*t))
    }
    fn objectives(&self) -> impl Iterator<Item = (u32, &Objective)> {
        self.objs.iter().enumerate().filter_map(|(i, o)| o.as_ref().map(|o| (i as u32, o)))
    }
    fn teams(&self) -> impl Iterator<Item = &Team> {
        self.teams.iter().flatten()
    }
    fn insert_objective(&mut self, o: Objective) -> u32 {
        let id = self.objs.len() as u32;
        self.obj_idx.insert(o.name.clone(), id);
        self.objs.push(Some(o));
        id
    }
    /// Plain insert used by `load` (no packets, no clients yet).
    fn insert_team(&mut self, t: Team) -> u32 {
        let id = self.teams.len() as u32;
        self.team_idx.insert(t.name.clone(), id);
        for m in &t.members {
            self.member_team.insert(m.clone(), id);
        }
        self.teams.push(Some(t));
        id
    }
}

/// `Scoreboard.getTrackedPlayers()` (wildcard `*`): holders with scores.
pub fn tracked_holders(g: &Game) -> Vec<Holder> {
    let sb = &g.cs.scoreboard;
    let mut v: Vec<(u32, &String)> = sb.holder_objs.iter().filter(|(_, o)| !o.is_empty()).map(|(h, _)| (*h, &sb.holder_names[*h as usize])).collect();
    v.sort_by_key(|(h, _)| *h);
    v.into_iter().map(|(_, n)| Holder::named(n.clone())).collect()
}

/// Set a score, creating it (`getOrCreatePlayerScore(...).set(v)`).
pub fn set_score(g: &mut Game, holder: &Holder, objective: &str, value: i32) {
    let Some(oid) = g.cs.scoreboard.obj_id(objective) else { return };
    let mut a = acc(g, holder, oid);
    acc_set(g, holder, &mut a, value);
}

/// Objective exists (`ObjectiveArgument.getObjective`: else
/// `arguments.objective.notFound`).
pub fn objective_exists(g: &Game, objective: &str) -> bool {
    g.cs.scoreboard.obj_id(objective).is_some()
}

/// `ObjectiveArgument.getWritableObjective`: read-only criteria error.
pub fn objective_writable(g: &Game, objective: &str) -> bool {
    g.cs.scoreboard.obj_id(objective).and_then(|o| g.cs.scoreboard.obj(o)).is_some_and(|o| !o.criteria.read_only)
}

/// `PlayerTeam.formatNameForTeam(team of holder, name)`.
pub fn team_format(g: &Game, holder: &str, name: Text) -> Text {
    match g.cs.scoreboard.team_of_holder(holder) {
        None => name,
        Some(t) => {
            let mut out = Text::empty().push(t.prefix.clone()).push(name).push(t.suffix.clone());
            if let Some(c) = t.color {
                out = out.color(COLOR_NAMES[c as usize & 15]);
            }
            out
        }
    }
}

/// Team name of a score holder.
pub fn team_of(g: &Game, holder: &str) -> Option<String> {
    g.cs.scoreboard.team_of_holder(holder).map(|t| t.name.clone())
}

/// Score of a holder in an objective.
pub fn score(g: &Game, holder: &str, objective: &str) -> Option<i32> {
    let sb = &g.cs.scoreboard;
    let oid = sb.obj_id(objective)?;
    let hid = *sb.holder_idx.get(holder)?;
    sb.scores.get(&(hid, oid)).map(|s| s.value)
}

/// `score` text component value (`ScoreContents.getScore`): formatted
/// score or empty.
pub fn format_score(g: &Game, holder: &str, objective: &str) -> Text {
    let sb = &g.cs.scoreboard;
    let Some(oid) = sb.obj_id(objective) else { return Text::empty() };
    let Some(hid) = sb.holder_idx.get(holder).copied() else { return Text::empty() };
    let Some(s) = sb.scores.get(&(hid, oid)) else { return Text::empty() };
    let fmt = s.format.as_ref().or_else(|| sb.obj(oid).and_then(|o| o.format.as_ref()));
    match fmt {
        Some(f) => f.format(s.value),
        None => Text::plain(s.value.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Packets
// ---------------------------------------------------------------------------

fn frame(p: BytesMut) -> Bytes {
    out::frame(p)
}

fn bcast(g: &mut Game, p: BytesMut) {
    g.broadcast.push(frame(p));
}

fn to_slot(g: &mut Game, slot: usize, p: BytesMut) {
    if let Some(Some(pl)) = g.players.get_mut(slot) {
        pl.send(p);
    }
}

fn write_fmt(b: &mut BytesMut, f: Option<&NumFmt>) {
    match f {
        None => b.w_bool(false),
        Some(f) => {
            b.w_bool(true);
            match f {
                NumFmt::Blank => b.w_varint(0),
                NumFmt::Styled(t) => {
                    b.w_varint(1);
                    t.write_network(b);
                }
                NumFmt::Fixed(t) => {
                    b.w_varint(2);
                    out::write_text(b, t);
                }
            }
        }
    }
}

fn set_score_pkt(owner: &str, obj: &str, s: &Score) -> BytesMut {
    packet(C_SET_SCORE, |b| {
        b.w_str(owner);
        b.w_str(obj);
        b.w_varint(s.value);
        match &s.display {
            Some(t) => {
                b.w_bool(true);
                out::write_text(b, t);
            }
            None => b.w_bool(false),
        }
        write_fmt(b, s.format.as_ref());
    })
}

fn reset_score_pkt(owner: &str, obj: Option<&str>) -> BytesMut {
    packet(C_RESET_SCORE, |b| {
        b.w_str(owner);
        match obj {
            Some(o) => {
                b.w_bool(true);
                b.w_str(o);
            }
            None => b.w_bool(false),
        }
    })
}

fn display_pkt(slot: usize, name: &str) -> BytesMut {
    packet(C_SET_DISPLAY_OBJECTIVE, |b| {
        b.w_varint(slot as i32);
        b.w_str(name);
    })
}

fn objective_pkt(o: &Objective, method: u8) -> BytesMut {
    packet(C_SET_OBJECTIVE, |b| {
        b.w_str(&o.name);
        b.w_u8(method);
        if method == OBJ_ADD || method == OBJ_CHANGE {
            out::write_text(b, &o.display);
            b.w_varint(o.render as i32);
            write_fmt(b, o.format.as_ref());
        }
    })
}

fn team_pkt(t: &Team, method: u8, players: &[String]) -> BytesMut {
    packet(C_SET_PLAYER_TEAM, |b| {
        b.w_str(&t.name);
        b.w_u8(method);
        if method == TEAM_ADD || method == TEAM_CHANGE {
            out::write_text(b, &t.display);
            out::write_text(b, &t.prefix);
            out::write_text(b, &t.suffix);
            b.w_varint(t.name_tag as i32);
            b.w_varint(t.collision as i32);
            match t.color {
                Some(c) => {
                    b.w_bool(true);
                    b.w_varint(c as i32);
                }
                None => b.w_bool(false),
            }
            let options = (t.friendly_fire as u8) | ((t.see_invisibles as u8) << 1);
            b.w_u8(options);
        }
        if method == TEAM_ADD || method == TEAM_JOIN || method == TEAM_LEAVE {
            b.w_varint(players.len() as i32);
            for p in players {
                b.w_str(p);
            }
        }
    })
}

/// Packets of `ServerScoreboard.getStartTrackingPackets`.
fn start_packets(sb: &State, oid: u32) -> Vec<BytesMut> {
    let Some(o) = sb.obj(oid) else { return Vec::new() };
    let mut v = vec![objective_pkt(o, OBJ_ADD)];
    for (slot, d) in sb.displays.iter().enumerate() {
        if *d == Some(oid) {
            v.push(display_pkt(slot, &o.name));
        }
    }
    let mut holders: Vec<u32> = sb.holder_objs.iter().filter(|(_, l)| l.contains(&oid)).map(|(h, _)| *h).collect();
    holders.sort_unstable();
    for h in holders {
        if let Some(s) = sb.scores.get(&(h, oid)) {
            v.push(set_score_pkt(sb.holder_name(h), &o.name, s));
        }
    }
    v
}

// ---------------------------------------------------------------------------
// Tracking and display slots (`ServerScoreboard`)
// ---------------------------------------------------------------------------

fn start_tracking(g: &mut Game, oid: u32) {
    let pkts = start_packets(&g.cs.scoreboard, oid);
    for p in pkts {
        bcast(g, p);
    }
}

fn stop_tracking(g: &mut Game, oid: u32) {
    let pkt = match g.cs.scoreboard.obj(oid) {
        Some(o) => objective_pkt(o, OBJ_REMOVE),
        None => return,
    };
    bcast(g, pkt);
}

/// `ServerScoreboard.setDisplayObjective`.
fn set_display(g: &mut Game, slot: usize, new: Option<u32>) {
    let old = g.cs.scoreboard.displays[slot];
    if old == new {
        // Same objective again: vanilla only re-sends the slot packet.
        if let Some(name) = new.and_then(|n| g.cs.scoreboard.obj(n)).map(|o| o.name.clone()) {
            bcast(g, display_pkt(slot, &name));
        }
        return;
    }
    let was_tracked_new = new.is_some_and(|n| g.cs.scoreboard.displays.iter().enumerate().any(|(i, d)| i != slot && *d == Some(n)));
    g.cs.scoreboard.displays[slot] = new;
    let new_name = new.and_then(|n| g.cs.scoreboard.obj(n)).map(|o| o.name.clone()).unwrap_or_default();
    if let Some(o) = old {
        if g.cs.scoreboard.display_count(o) > 0 {
            bcast(g, display_pkt(slot, &new_name));
        } else {
            stop_tracking(g, o);
        }
    }
    if let Some(n) = new {
        if was_tracked_new {
            bcast(g, display_pkt(slot, &new_name));
        } else {
            start_tracking(g, n);
        }
    }
}

/// `onObjectiveChanged`: objective packet only when tracked.
fn objective_changed(g: &mut Game, oid: u32) {
    let pkt = {
        let sb = &g.cs.scoreboard;
        if !sb.tracked(oid) {
            return;
        }
        match sb.obj(oid) {
            Some(o) => objective_pkt(o, OBJ_CHANGE),
            None => return,
        }
    };
    bcast(g, pkt);
}

/// Remove an objective (`Scoreboard.removeObjective`).
fn remove_objective(g: &mut Game, oid: u32) {
    for slot in 0..19 {
        if g.cs.scoreboard.displays[slot] == Some(oid) {
            set_display(g, slot, None);
        }
    }
    let sb = &mut g.cs.scoreboard;
    let Some(name) = sb.obj(oid).map(|o| o.name.clone()) else { return };
    sb.obj_idx.remove(&name);
    sb.objs[oid as usize] = None;
    sb.scores.retain(|(_, o), _| *o != oid);
    for l in sb.holder_objs.values_mut() {
        l.retain(|o| *o != oid);
    }
}

// ---------------------------------------------------------------------------
// Scores (`Scoreboard.getOrCreatePlayerScore` and `ScoreAccess`)
// ---------------------------------------------------------------------------

/// Auto display name of a holder (`displayAutoUpdate`).
fn auto_display(g: &Game, h: &Holder, oid: u32) -> Option<Text> {
    let auto = g.cs.scoreboard.obj(oid).is_some_and(|o| o.auto_update);
    if !auto {
        return None;
    }
    h.entity.map(|e| feedback::display_name(g, e))
}

/// A score access: `requiresSync` is `created` until the first send.
struct Acc {
    hid: u32,
    oid: u32,
    created: bool,
}

fn acc(g: &mut Game, h: &Holder, oid: u32) -> Acc {
    let sb = &mut g.cs.scoreboard;
    let hid = sb.holder_id(&h.name);
    let created = !sb.scores.contains_key(&(hid, oid));
    if created {
        sb.scores.insert((hid, oid), Score::default());
        sb.holder_objs.entry(hid).or_default().push(oid);
    }
    Acc { hid, oid, created }
}

fn acc_value(g: &Game, a: &Acc) -> i32 {
    g.cs.scoreboard.scores.get(&(a.hid, a.oid)).map(|s| s.value).unwrap_or(0)
}

/// `onScoreChanged`: score packet when the objective is tracked.
fn send_score(g: &mut Game, hid: u32, oid: u32) {
    let pkt = {
        let sb = &g.cs.scoreboard;
        if !sb.tracked(oid) {
            return;
        }
        match (sb.obj(oid), sb.scores.get(&(hid, oid))) {
            (Some(o), Some(s)) => set_score_pkt(sb.holder_name(hid), &o.name, s),
            _ => return,
        }
    };
    bcast(g, pkt);
}

/// `ScoreAccess.set` with auto display.
fn acc_set(g: &mut Game, h: &Holder, a: &mut Acc, value: i32) {
    let auto = auto_display(g, h, a.oid);
    let mut changed = a.created;
    if let Some(s) = g.cs.scoreboard.scores.get_mut(&(a.hid, a.oid)) {
        if let Some(nd) = auto {
            if s.display.as_ref() != Some(&nd) {
                s.display = Some(nd);
                changed = true;
            }
        }
        if value != s.value {
            s.value = value;
            changed = true;
        }
    }
    if changed {
        send_score(g, a.hid, a.oid);
    }
    a.created = false;
}

/// `ScoreAccess.display(display)`.
fn acc_display(g: &mut Game, a: &mut Acc, display: Option<Text>) {
    let mut send = a.created;
    if let Some(s) = g.cs.scoreboard.scores.get_mut(&(a.hid, a.oid)) {
        if s.display != display {
            send = true;
        }
        s.display = display;
    }
    if send {
        send_score(g, a.hid, a.oid);
    }
    a.created = false;
}

/// `ScoreAccess.numberFormatOverride`: always sends.
fn acc_format(g: &mut Game, a: &mut Acc, fmt: Option<NumFmt>) {
    if let Some(s) = g.cs.scoreboard.scores.get_mut(&(a.hid, a.oid)) {
        s.format = fmt;
    }
    send_score(g, a.hid, a.oid);
    a.created = false;
}

/// `ScoreAccess.locked()` / `unlock()` / `lock()`.
fn acc_lock(g: &mut Game, a: &mut Acc, locked: bool) {
    if let Some(s) = g.cs.scoreboard.scores.get_mut(&(a.hid, a.oid)) {
        s.locked = locked;
    }
    if a.created {
        send_score(g, a.hid, a.oid);
    }
    a.created = false;
}

fn acc_locked(g: &Game, a: &Acc) -> bool {
    g.cs.scoreboard.scores.get(&(a.hid, a.oid)).map(|s| s.locked).unwrap_or(true)
}

/// `ScoreAccess.set` on a holder with auto display (no access state kept).
fn put_value(g: &mut Game, h: &Holder, oid: u32, value: i32) {
    let mut a = acc(g, h, oid);
    acc_set(g, h, &mut a, value);
}

/// `Scoreboard.resetSinglePlayerScore`.
fn reset_single(g: &mut Game, hname: &str, oid: u32) {
    let Some(hid) = g.cs.scoreboard.holder_idx.get(hname).copied() else { return };
    if g.cs.scoreboard.scores.remove(&(hid, oid)).is_none() {
        return;
    }
    let empty = {
        let l = g.cs.scoreboard.holder_objs.entry(hid).or_default();
        l.retain(|o| *o != oid);
        l.is_empty()
    };
    if empty {
        g.cs.scoreboard.holder_objs.remove(&hid);
        if g.cs.scoreboard.any_tracked() {
            bcast(g, reset_score_pkt(hname, None));
        }
    } else if g.cs.scoreboard.tracked(oid) {
        let oname = g.cs.scoreboard.obj(oid).map(|o| o.name.clone()).unwrap_or_default();
        bcast(g, reset_score_pkt(hname, Some(&oname)));
    }
}

/// `Scoreboard.resetAllPlayerScores`.
fn reset_all(g: &mut Game, hname: &str) {
    let Some(hid) = g.cs.scoreboard.holder_idx.get(hname).copied() else { return };
    let Some(objs) = g.cs.scoreboard.holder_objs.remove(&hid) else { return };
    for o in objs {
        g.cs.scoreboard.scores.remove(&(hid, o));
    }
    if g.cs.scoreboard.any_tracked() {
        bcast(g, reset_score_pkt(hname, None));
    }
}

/// Scores of a holder as (objective id, value), in objective order.
fn holder_scores(g: &Game, hname: &str) -> Vec<(u32, i32)> {
    let sb = &g.cs.scoreboard;
    let Some(hid) = sb.holder_idx.get(hname).copied() else { return Vec::new() };
    let mut v: Vec<(u32, i32)> = sb.holder_objs.get(&hid).map(|l| l.iter().filter_map(|o| sb.scores.get(&(hid, *o)).map(|s| (*o, s.value))).collect()).unwrap_or_default();
    v.sort_by_key(|(o, _)| *o);
    v
}

// ---------------------------------------------------------------------------
// Teams (`ServerScoreboard` team hooks)
// ---------------------------------------------------------------------------

fn add_team(g: &mut Game, name: &str) -> u32 {
    let t = Team::new(name);
    let pkt = team_pkt(&t, TEAM_ADD, &[]);
    let tid = g.cs.scoreboard.insert_team(t);
    bcast(g, pkt);
    tid
}

fn team_changed(g: &mut Game, tid: u32) {
    let pkt = match g.cs.scoreboard.team(tid) {
        Some(t) => team_pkt(t, TEAM_CHANGE, &[]),
        None => return,
    };
    bcast(g, pkt);
}

fn remove_team(g: &mut Game, tid: u32) {
    let Some(t) = g.cs.scoreboard.teams.get_mut(tid as usize).and_then(|t| t.take()) else { return };
    g.cs.scoreboard.team_idx.remove(&t.name);
    for m in &t.members {
        g.cs.scoreboard.member_team.remove(m);
    }
    bcast(g, team_pkt(&t, TEAM_REMOVE, &[]));
}

/// `Scoreboard.addPlayerToTeam` (leaves the old team first).
fn join_team(g: &mut Game, hname: &str, tid: u32) -> bool {
    if g.cs.scoreboard.member_team.contains_key(hname) {
        leave_team(g, hname);
    }
    let pkt = {
        let sb = &mut g.cs.scoreboard;
        let Some(t) = sb.teams.get_mut(tid as usize).and_then(|t| t.as_mut()) else { return false };
        t.members.push(hname.to_string());
        let pkt = team_pkt(t, TEAM_JOIN, &[hname.to_string()]);
        sb.member_team.insert(hname.to_string(), tid);
        pkt
    };
    bcast(g, pkt);
    true
}

/// `Scoreboard.removePlayerFromTeam(player)`.
fn leave_team(g: &mut Game, hname: &str) -> bool {
    let Some(tid) = g.cs.scoreboard.member_team.remove(hname) else { return false };
    let pkt = {
        let Some(t) = g.cs.scoreboard.teams.get_mut(tid as usize).and_then(|t| t.as_mut()) else { return false };
        t.members.retain(|m| m != hname);
        team_pkt(t, TEAM_LEAVE, &[hname.to_string()])
    };
    bcast(g, pkt);
    true
}

// ---------------------------------------------------------------------------
// Join and save/load
// ---------------------------------------------------------------------------

/// `PlayerList.placeNewPlayer` board part: every team, then the objectives
/// on display slots, sent to one player.
pub fn on_join(g: &mut Game, slot: usize) {
    let pkts = {
        let sb = &g.cs.scoreboard;
        let mut v: Vec<BytesMut> = sb.teams().map(|t| team_pkt(t, TEAM_ADD, &t.members)).collect();
        let mut seen: Vec<u32> = Vec::new();
        for d in sb.displays.iter().flatten() {
            if !seen.contains(d) {
                seen.push(*d);
                v.extend(start_packets(sb, *d));
            }
        }
        v
    };
    for p in pkts {
        to_slot(g, slot, p);
    }
}

/// Write `data/scoreboard.dat` (gzip NBT, vanilla `ScoreboardSaveData`).
pub fn save(g: &Game, world_dir: &str) {
    let sb = &g.cs.scoreboard;
    let objectives: Vec<Tag> = sb.objectives().map(|(_, o)| objective_tag(o)).collect();
    let mut scores: Vec<Tag> = Vec::new();
    for ((hid, oid), s) in &sb.scores {
        let (Some(o), Some(h)) = (sb.obj(*oid), sb.holder_names.get(*hid as usize)) else { continue };
        let mut t = Tag::compound()
            .with("Name", Tag::String(h.clone()))
            .with("Objective", Tag::String(o.name.clone()))
            .with("Score", Tag::Int(s.value))
            .with("Locked", Tag::Byte(s.locked as i8));
        if let Some(d) = &s.display {
            t = t.with("display", d.to_nbt());
        }
        if let Some(f) = &s.format {
            t = t.with("format", fmt_tag(f));
        }
        scores.push(t);
    }
    let mut slots: Vec<(String, Tag)> = Vec::new();
    for (i, d) in sb.displays.iter().enumerate() {
        if let Some(oid) = d {
            if let Some(o) = sb.obj(*oid) {
                slots.push((SLOT_NAMES[i].to_string(), Tag::String(o.name.clone())));
            }
        }
    }
    let teams: Vec<Tag> = sb.teams().map(team_tag).collect();
    let data = Tag::compound()
        .with("Objectives", Tag::List(objectives))
        .with("PlayerScores", Tag::List(scores))
        .with("DisplaySlots", Tag::Compound(slots))
        .with("Teams", Tag::List(teams));
    let raw = nbt_root(&[("data", data), ("DataVersion", Tag::Int(DATA_VERSION))]);
    write_gzip(&Path::new(world_dir).join("data").join("scoreboard.dat"), &raw);
}

/// Read `data/scoreboard.dat` if present, replacing the board.
pub fn load(g: &mut Game, world_dir: &str) {
    let path = Path::new(world_dir).join("data").join("scoreboard.dat");
    let Some(root) = read_root_file(&path) else { return };
    let Some((_, data)) = root.iter().find(|(k, _)| k == "data") else { return };
    g.cs.scoreboard = State::default();
    let sb = &mut g.cs.scoreboard;
    for o in list_of(data, "Objectives") {
        let name = str_of(o, "Name").unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let crit = criteria_of(&str_of(o, "CriteriaName").unwrap_or_else(|| "dummy".into()));
        let render = if str_of(o, "RenderType").as_deref() == Some("hearts") { 1 } else { 0 };
        let display = o.get("DisplayName").map(select::text_from_tag).unwrap_or_else(|| Text::plain(name.clone()));
        sb.insert_objective(Objective {
            name,
            criteria: crit,
            display,
            render,
            auto_update: byte_of(o, "display_auto_update") != 0,
            format: o.get("format").and_then(fmt_from_tag),
        });
    }
    for s in list_of(data, "PlayerScores") {
        let (Some(h), Some(on)) = (str_of(s, "Name"), str_of(s, "Objective")) else { continue };
        let Some(oid) = sb.obj_id(&on) else { continue };
        let hid = sb.holder_id(&h);
        let score = Score {
            value: match s.get("Score") {
                Some(Tag::Int(v)) => *v,
                _ => 0,
            },
            locked: byte_of(s, "Locked") != 0,
            display: s.get("display").map(select::text_from_tag),
            format: s.get("format").and_then(fmt_from_tag),
        };
        sb.scores.insert((hid, oid), score);
        sb.holder_objs.entry(hid).or_default().push(oid);
    }
    if let Some(Tag::Compound(slots)) = data.get("DisplaySlots") {
        for (k, v) in slots {
            let (Some(i), Tag::String(on)) = (SLOT_NAMES.iter().position(|n| n == k), v) else { continue };
            if let Some(oid) = sb.obj_id(on) {
                sb.displays[i] = Some(oid);
            }
        }
    }
    for t in list_of(data, "Teams") {
        let Some(name) = str_of(t, "Name") else { continue };
        let mut team = Team::new(&name);
        if let Some(d) = t.get("DisplayName") {
            team.display = select::text_from_tag(d);
        }
        team.color = str_of(t, "TeamColor").and_then(|c| COLOR_NAMES.iter().position(|n| *n == c).map(|i| i as u8));
        team.friendly_fire = byte_default(t, "AllowFriendlyFire", 1) != 0;
        team.see_invisibles = byte_default(t, "SeeFriendlyInvisibles", 1) != 0;
        team.prefix = t.get("MemberNamePrefix").map(select::text_from_tag).unwrap_or_default();
        team.suffix = t.get("MemberNameSuffix").map(select::text_from_tag).unwrap_or_default();
        team.name_tag = str_of(t, "NameTagVisibility").and_then(|v| VIS_NAMES.iter().position(|n| *n == v)).unwrap_or(0) as u8;
        team.death_msg = str_of(t, "DeathMessageVisibility").and_then(|v| VIS_NAMES.iter().position(|n| *n == v)).unwrap_or(0) as u8;
        team.collision = str_of(t, "CollisionRule").and_then(|v| COLL_NAMES.iter().position(|n| *n == v)).unwrap_or(0) as u8;
        for p in list_of_strings(t, "Players") {
            team.members.push(p);
        }
        sb.insert_team(team);
    }
}

fn objective_tag(o: &Objective) -> Tag {
    let mut t = Tag::compound()
        .with("Name", Tag::String(o.name.clone()))
        .with("CriteriaName", Tag::String(o.criteria.name.clone()))
        .with("DisplayName", o.display.to_nbt())
        .with("RenderType", Tag::String(RENDER_NAMES[o.render as usize & 1].to_string()))
        .with("display_auto_update", Tag::Byte(o.auto_update as i8));
    if let Some(f) = &o.format {
        t = t.with("format", fmt_tag(f));
    }
    t
}

fn team_tag(t: &Team) -> Tag {
    let mut c = Tag::compound()
        .with("Name", Tag::String(t.name.clone()))
        .with("DisplayName", t.display.to_nbt())
        .with("AllowFriendlyFire", Tag::Byte(t.friendly_fire as i8))
        .with("SeeFriendlyInvisibles", Tag::Byte(t.see_invisibles as i8))
        .with("MemberNamePrefix", t.prefix.to_nbt())
        .with("MemberNameSuffix", t.suffix.to_nbt())
        .with("NameTagVisibility", Tag::String(VIS_NAMES[t.name_tag as usize & 3].to_string()))
        .with("DeathMessageVisibility", Tag::String(VIS_NAMES[t.death_msg as usize & 3].to_string()))
        .with("CollisionRule", Tag::String(COLL_NAMES[t.collision as usize & 3].to_string()))
        .with("Players", Tag::List(t.members.iter().map(|m| Tag::String(m.clone())).collect()));
    if let Some(col) = t.color {
        c = c.with("TeamColor", Tag::String(COLOR_NAMES[col as usize & 15].to_string()));
    }
    c
}

/// `NumberFormatTypes` map codec: `type` plus the type's fields.
fn fmt_tag(f: &NumFmt) -> Tag {
    match f {
        NumFmt::Blank => Tag::compound().with("type", Tag::String("minecraft:blank".into())),
        NumFmt::Fixed(t) => Tag::compound().with("type", Tag::String("minecraft:fixed".into())).with("value", t.to_nbt()),
        NumFmt::Styled(Tag::Compound(fields)) => {
            let mut c = vec![("type".to_string(), Tag::String("minecraft:styled".into()))];
            c.extend(fields.iter().cloned());
            Tag::Compound(c)
        }
        NumFmt::Styled(_) => Tag::compound().with("type", Tag::String("minecraft:styled".into())),
    }
}

fn fmt_from_tag(t: &Tag) -> Option<NumFmt> {
    match str_of(t, "type")?.as_str() {
        "minecraft:blank" => Some(NumFmt::Blank),
        "minecraft:fixed" => Some(NumFmt::Fixed(t.get("value").map(select::text_from_tag).unwrap_or_default())),
        "minecraft:styled" => {
            let fields: Vec<(String, Tag)> = match t {
                Tag::Compound(f) => f.iter().filter(|(k, _)| k != "type").cloned().collect(),
                _ => Vec::new(),
            };
            Some(NumFmt::Styled(Tag::Compound(fields)))
        }
        _ => None,
    }
}

fn list_of<'a>(t: &'a Tag, key: &str) -> Vec<&'a Tag> {
    match t.get(key) {
        Some(Tag::List(v)) => v.iter().collect(),
        _ => Vec::new(),
    }
}

fn list_of_strings(t: &Tag, key: &str) -> Vec<String> {
    match t.get(key) {
        Some(Tag::List(v)) => v.iter().filter_map(|x| if let Tag::String(s) = x { Some(s.clone()) } else { None }).collect(),
        _ => Vec::new(),
    }
}

fn str_of(t: &Tag, key: &str) -> Option<String> {
    match t.get(key) {
        Some(Tag::String(s)) => Some(s.clone()),
        _ => None,
    }
}

fn byte_of(t: &Tag, key: &str) -> i8 {
    match t.get(key) {
        Some(Tag::Byte(b)) => *b,
        _ => 0,
    }
}

fn byte_default(t: &Tag, key: &str, d: i8) -> i8 {
    match t.get(key) {
        Some(Tag::Byte(b)) => *b,
        _ => d,
    }
}

/// Unnamed root compound (`TAG_Compound`, empty name) with the given fields,
/// as in vanilla `data/*.dat` files.
pub(crate) fn nbt_root(children: &[(&str, Tag)]) -> Vec<u8> {
    let mut raw = vec![10u8, 0, 0];
    for (k, v) in children {
        v.write_named(&mut raw, k);
    }
    raw.push(0);
    raw
}

/// Fields of the root compound of a (gzip or plain) NBT file.
pub(crate) fn read_root_file(path: &Path) -> Option<Vec<(String, Tag)>> {
    let bytes = std::fs::read(path).ok()?;
    let raw = if bytes.starts_with(&[0x1f, 0x8b]) {
        gunzip(&bytes).ok()?
    } else {
        bytes
    };
    let mut r = Reader::new(&raw);
    match rine_protocol::nbt::read::read_named(&mut r).ok()??.1 {
        Tag::Compound(fields) => Some(fields),
        _ => None,
    }
}

pub(crate) fn write_gzip(path: &Path, raw: &[u8]) {
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = path.with_extension("dat.tmp");
    if std::fs::write(&tmp, gzip_stored(raw)).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

fn crc32(d: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in d {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

/// gzip container with stored (uncompressed) deflate blocks.
pub(crate) fn gzip_stored(raw: &[u8]) -> Vec<u8> {
    let mut o = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
    let empty: &[u8] = &[];
    let chunks: Vec<&[u8]> = if raw.is_empty() { vec![empty] } else { raw.chunks(65535).collect() };
    let n = chunks.len();
    for (i, c) in chunks.iter().enumerate() {
        o.push(if i + 1 == n { 1 } else { 0 });
        let l = c.len() as u16;
        o.extend_from_slice(&l.to_le_bytes());
        o.extend_from_slice(&(!l).to_le_bytes());
        o.extend_from_slice(c);
    }
    o.extend_from_slice(&crc32(raw).to_le_bytes());
    o.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    o
}

/// gzip member decoder (RFC 1952 header, RFC 1951 inflate).
pub(crate) fn gunzip(d: &[u8]) -> Result<Vec<u8>, &'static str> {
    if d.len() < 18 || d[0] != 0x1f || d[1] != 0x8b || d[2] != 8 {
        return Err("not gzip");
    }
    let flg = d[3];
    let mut p = 10usize;
    if flg & 4 != 0 {
        let xlen = *d.get(p).ok_or("eof")? as usize | (*d.get(p + 1).ok_or("eof")? as usize) << 8;
        p += 2 + xlen;
    }
    if flg & 8 != 0 {
        while *d.get(p).ok_or("eof")? != 0 {
            p += 1;
        }
        p += 1;
    }
    if flg & 16 != 0 {
        while *d.get(p).ok_or("eof")? != 0 {
            p += 1;
        }
        p += 1;
    }
    if flg & 2 != 0 {
        p += 2;
    }
    inflate(d.get(p..).ok_or("eof")?)
}

struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
    buf: u32,
    cnt: u32,
}

impl Bits<'_> {
    fn bit(&mut self) -> Result<u32, &'static str> {
        if self.cnt == 0 {
            self.buf = *self.d.get(self.pos).ok_or("eof")? as u32;
            self.pos += 1;
            self.cnt = 8;
        }
        let v = self.buf & 1;
        self.buf >>= 1;
        self.cnt -= 1;
        Ok(v)
    }
    fn bits(&mut self, n: u32) -> Result<u32, &'static str> {
        let mut v = 0;
        for i in 0..n {
            v |= self.bit()? << i;
        }
        Ok(v)
    }
}

struct Huff {
    count: [u16; 16],
    sym: Vec<u16>,
}

fn build_huff(lengths: &[u8]) -> Huff {
    let mut count = [0u16; 16];
    for &l in lengths {
        count[l as usize & 15] += 1;
    }
    count[0] = 0;
    let mut offs = [0usize; 16];
    for len in 1..15 {
        offs[len + 1] = offs[len] + count[len] as usize;
    }
    let mut sym = vec![0u16; lengths.len()];
    for (s, &l) in lengths.iter().enumerate() {
        if l != 0 {
            let o = offs[l as usize & 15];
            if let Some(slot) = sym.get_mut(o) {
                *slot = s as u16;
            }
            offs[l as usize & 15] += 1;
        }
    }
    Huff { count, sym }
}

fn decode(b: &mut Bits, h: &Huff) -> Result<u16, &'static str> {
    let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
    for len in 1..16 {
        code |= b.bit()? as i32;
        let count = h.count[len] as i32;
        if code - count < first {
            return h.sym.get((index + (code - first)) as usize).copied().ok_or("bad code");
        }
        index += count;
        first += count;
        first <<= 1;
        code <<= 1;
    }
    Err("bad code")
}

const LBASE: [usize; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEXT: [u32; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DBASE: [usize; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DEXT: [u32; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

fn codes(b: &mut Bits, out: &mut Vec<u8>, lit: &Huff, dist: &Huff) -> Result<(), &'static str> {
    loop {
        let sym = decode(b, lit)? as usize;
        if sym < 256 {
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let s = sym - 257;
            let len = LBASE.get(s).ok_or("bad len")? + b.bits(*LEXT.get(s).ok_or("bad len")?)? as usize;
            let ds = decode(b, dist)? as usize;
            let d = DBASE.get(ds).ok_or("bad dist")? + b.bits(*DEXT.get(ds).ok_or("bad dist")?)? as usize;
            let start = out.len().checked_sub(d).ok_or("dist too far")?;
            for i in 0..len {
                let c = out[start + i];
                out.push(c);
            }
        }
    }
}

fn inflate(d: &[u8]) -> Result<Vec<u8>, &'static str> {
    let mut b = Bits { d, pos: 0, buf: 0, cnt: 0 };
    let mut out = Vec::new();
    loop {
        let last = b.bits(1)?;
        let ty = b.bits(2)?;
        match ty {
            0 => {
                b.buf = 0;
                b.cnt = 0;
                let p = b.pos;
                let len = *d.get(p).ok_or("eof")? as usize | (*d.get(p + 1).ok_or("eof")? as usize) << 8;
                let body = d.get(p + 4..p + 4 + len).ok_or("eof")?;
                out.extend_from_slice(body);
                b.pos = p + 4 + len;
            }
            1 => {
                let mut l = [0u8; 288];
                for (i, v) in l.iter_mut().enumerate() {
                    *v = match i {
                        0..=143 => 8,
                        144..=255 => 9,
                        256..=279 => 7,
                        _ => 8,
                    };
                }
                let lit = build_huff(&l);
                let dist = build_huff(&[5u8; 30]);
                codes(&mut b, &mut out, &lit, &dist)?;
            }
            2 => {
                let nlen = b.bits(5)? as usize + 257;
                let ndist = b.bits(5)? as usize + 1;
                let ncode = b.bits(4)? as usize + 4;
                const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
                let mut cl = [0u8; 19];
                for &o in ORDER.iter().take(ncode) {
                    cl[o] = b.bits(3)? as u8;
                }
                let clh = build_huff(&cl);
                let total = nlen + ndist;
                let mut lens = vec![0u8; total];
                let mut i = 0usize;
                while i < total {
                    let sym = decode(&mut b, &clh)?;
                    let (val, rep) = match sym {
                        0..=15 => (sym as u8, 1usize),
                        16 => {
                            if i == 0 {
                                return Err("repeat without previous");
                            }
                            (lens[i - 1], 3 + b.bits(2)? as usize)
                        }
                        17 => (0, 3 + b.bits(3)? as usize),
                        _ => (0, 11 + b.bits(7)? as usize),
                    };
                    if i + rep > total {
                        return Err("too many lengths");
                    }
                    for _ in 0..rep {
                        lens[i] = val;
                        i += 1;
                    }
                }
                let lit = build_huff(&lens[..nlen]);
                let dist = build_huff(&lens[nlen..]);
                codes(&mut b, &mut out, &lit, &dist)?;
            }
            _ => return Err("bad block type"),
        }
        if last == 1 {
            break;
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Command helpers
// ---------------------------------------------------------------------------

/// `ObjectiveCriteria` display: `[name]` with hover, as `getFormattedDisplayName`.
fn fmt_obj(o: &Objective) -> Text {
    Text::tr("chat.square_brackets", vec![o.display.clone().on_hover(hover_text(&Text::plain(o.name.clone())))])
}

/// `PlayerTeam.getFormattedDisplayName`.
fn fmt_team(t: &Team) -> Text {
    let inner = t.display.clone().insertion(t.name.clone()).on_hover(hover_text(&Text::plain(t.name.clone())));
    let mut out = Text::tr("chat.square_brackets", vec![inner]);
    if let Some(c) = t.color {
        out = out.color(COLOR_NAMES[c as usize & 15]);
    }
    out
}

/// `ComponentUtils.formatList`: items joined with gray ", ".
fn join(items: Vec<Text>) -> Text {
    crate::game::cmd::feedback::format_list(items, &crate::game::cmd::feedback::list_sep())
}

fn oid_of(g: &Game, name: &str) -> CmdResult<u32> {
    g.cs.scoreboard.obj_id(name).ok_or_else(|| CmdError::tr("arguments.objective.notFound", vec![lit(name)]))
}

fn obj_of<'a>(g: &'a Game, name: &str) -> CmdResult<&'a Objective> {
    let oid = oid_of(g, name)?;
    g.cs.scoreboard.obj(oid).ok_or_else(|| CmdError::tr("arguments.objective.notFound", vec![lit(name)]))
}

/// `CommandResponseTracker`: totals and the only element (or none) for
/// both element types (ANY and NON_ZERO).
#[derive(Default)]
struct Track {
    total: i32,
    n: usize,
    only: Option<usize>,
    nz: usize,
    nz_only: Option<usize>,
}

impl Track {
    fn add(&mut self, i: usize, v: i32) {
        self.total = self.total.wrapping_add(v);
        self.n += 1;
        self.only = if self.n == 1 { Some(i) } else { None };
        if v != 0 {
            self.nz += 1;
            self.nz_only = if self.nz == 1 { Some(i) } else { None };
        }
    }
    fn pick(&self, any: bool) -> (usize, Option<usize>) {
        if any {
            (self.n, self.only)
        } else {
            (self.nz, self.nz_only)
        }
    }
}

/// `CommandResponseTracker.sendFeedback`: single or multiple message.
fn respond(
    x: &mut Ctx,
    t: &Track,
    any: bool,
    zero_key: Option<&'static str>,
    bc: bool,
    single: impl FnOnce(&Game, usize, i32) -> Text,
    multi: impl FnOnce(usize, i32) -> Text,
) -> CmdResult<i32> {
    let (n, only) = t.pick(any);
    if n == 0 {
        if let Some(k) = zero_key {
            return Err(CmdError::key(k));
        }
    }
    let msg = match only {
        Some(i) => single(&*x.g, i, t.total),
        None => multi(n, t.total),
    };
    x.success(msg, bc);
    Ok(t.total)
}

// ---------------------------------------------------------------------------
// /scoreboard objectives
// ---------------------------------------------------------------------------

fn objectives_list(x: &mut Ctx) -> CmdResult<i32> {
    let (n, msg) = {
        let g = &*x.g;
        let items: Vec<Text> = g.cs.scoreboard.objectives().map(|(_, o)| fmt_obj(o)).collect();
        let n = items.len();
        let msg = if n == 0 {
            Text::key("commands.scoreboard.objectives.list.empty")
        } else {
            Text::tr("commands.scoreboard.objectives.list.success", vec![lit(n), join(items)])
        };
        (n, msg)
    };
    x.success(msg, false);
    Ok(n as i32)
}

fn objectives_add(x: &mut Ctx) -> CmdResult<i32> {
    let name = x.str("objective").to_string();
    let crit = x.str("criteria").to_string();
    add_objective(x, name, crit, None)
}

fn objectives_add_named(x: &mut Ctx) -> CmdResult<i32> {
    let name = x.str("objective").to_string();
    let crit = x.str("criteria").to_string();
    let display = x.component("displayName")?;
    add_objective(x, name, crit, Some(display))
}

fn add_objective(x: &mut Ctx, name: String, crit: String, display: Option<Text>) -> CmdResult<i32> {
    if x.g.cs.scoreboard.obj_id(&name).is_some() {
        return Err(CmdError::key("commands.scoreboard.objectives.add.duplicate"));
    }
    let criteria = criteria_of(&crit);
    let render = criteria.hearts as u8;
    let o = Objective { display: display.unwrap_or_else(|| Text::plain(name.clone())), name, criteria, render, auto_update: false, format: None };
    let msg = Text::tr("commands.scoreboard.objectives.add.success", vec![fmt_obj(&o)]);
    x.g.cs.scoreboard.insert_objective(o);
    let n = x.g.cs.scoreboard.objectives().count();
    x.success(msg, true);
    Ok(n as i32)
}

fn objectives_remove(x: &mut Ctx) -> CmdResult<i32> {
    let name = x.objective("objective")?;
    let oid = oid_of(x.g, &name)?;
    let msg = Text::tr("commands.scoreboard.objectives.remove.success", vec![fmt_obj(obj_of(x.g, &name)?)]);
    remove_objective(x.g, oid);
    let n = x.g.cs.scoreboard.objectives().count();
    x.success(msg, true);
    Ok(n as i32)
}

fn modify_displayname(x: &mut Ctx) -> CmdResult<i32> {
    let name = x.objective("objective")?;
    let display = x.component("displayName")?;
    let oid = oid_of(x.g, &name)?;
    let (changed, msg) = {
        let o = obj_of(x.g, &name)?;
        if o.display == display {
            (false, None)
        } else {
            (true, Some(Text::tr("commands.scoreboard.objectives.modify.displayname", vec![lit(&o.name), fmt_obj(o)])))
        }
    };
    if changed {
        if let Some(o) = x.g.cs.scoreboard.objs.get_mut(oid as usize).and_then(|o| o.as_mut()) {
            o.display = display;
        }
        objective_changed(x.g, oid);
        if let Some(m) = msg {
            x.success(m, true);
        }
    }
    Ok(0)
}

fn modify_rendertype(x: &mut Ctx, render: u8) -> CmdResult<i32> {
    let name = x.objective("objective")?;
    let oid = oid_of(x.g, &name)?;
    let msg = {
        let o = obj_of(x.g, &name)?;
        if o.render == render {
            None
        } else {
            Some(Text::tr("commands.scoreboard.objectives.modify.rendertype", vec![fmt_obj(o)]))
        }
    };
    if let Some(m) = msg {
        if let Some(o) = x.g.cs.scoreboard.objs.get_mut(oid as usize).and_then(|o| o.as_mut()) {
            o.render = render;
        }
        objective_changed(x.g, oid);
        x.success(m, true);
    }
    Ok(0)
}

fn modify_autoupdate(x: &mut Ctx) -> CmdResult<i32> {
    let name = x.objective("objective")?;
    let value = x.bool("value");
    let oid = oid_of(x.g, &name)?;
    let msg = {
        let o = obj_of(x.g, &name)?;
        if o.auto_update == value {
            None
        } else {
            let key = if value { "commands.scoreboard.objectives.modify.displayAutoUpdate.enable" } else { "commands.scoreboard.objectives.modify.displayAutoUpdate.disable" };
            Some(Text::tr(key, vec![lit(&o.name), fmt_obj(o)]))
        }
    };
    if let Some(m) = msg {
        if let Some(o) = x.g.cs.scoreboard.objs.get_mut(oid as usize).and_then(|o| o.as_mut()) {
            o.auto_update = value;
        }
        objective_changed(x.g, oid);
        x.success(m, true);
    }
    Ok(0)
}

fn set_objective_format(x: &mut Ctx, name: &str, fmt: Option<NumFmt>) -> CmdResult<i32> {
    let oid = oid_of(x.g, name)?;
    let set = fmt.is_some();
    if let Some(o) = x.g.cs.scoreboard.objs.get_mut(oid as usize).and_then(|o| o.as_mut()) {
        o.format = fmt;
    }
    objective_changed(x.g, oid);
    let key = if set { "commands.scoreboard.objectives.modify.objectiveFormat.set" } else { "commands.scoreboard.objectives.modify.objectiveFormat.clear" };
    x.success(Text::tr(key, vec![lit(name)]), true);
    Ok(0)
}

fn objective_format(x: &mut Ctx, fmt: Option<NumFmt>) -> CmdResult<i32> {
    let name = x.objective("objective")?;
    set_objective_format(x, &name, fmt)
}

fn objective_format_fixed(x: &mut Ctx) -> CmdResult<i32> {
    let name = x.objective("objective")?;
    let t = x.component("contents")?;
    set_objective_format(x, &name, Some(NumFmt::Fixed(t)))
}

fn objective_format_styled(x: &mut Ctx) -> CmdResult<i32> {
    let name = x.objective("objective")?;
    let t = x.nbt("style").clone();
    set_objective_format(x, &name, Some(NumFmt::Styled(t)))
}

fn slot_of(x: &Ctx) -> usize {
    match x.arg("slot") {
        Arg::ScoreboardSlot(i) => (*i).clamp(0, 18) as usize,
        _ => 0,
    }
}

fn display_clear(x: &mut Ctx) -> CmdResult<i32> {
    let slot = slot_of(x);
    if x.g.cs.scoreboard.displays[slot].is_none() {
        return Err(CmdError::key("commands.scoreboard.objectives.display.alreadyEmpty"));
    }
    set_display(x.g, slot, None);
    x.success(Text::tr("commands.scoreboard.objectives.display.cleared", vec![lit(SLOT_NAMES[slot])]), true);
    Ok(0)
}

fn display_set(x: &mut Ctx) -> CmdResult<i32> {
    let slot = slot_of(x);
    let name = x.objective("objective")?;
    let oid = oid_of(x.g, &name)?;
    if x.g.cs.scoreboard.displays[slot] == Some(oid) {
        return Err(CmdError::key("commands.scoreboard.objectives.display.alreadySet"));
    }
    let disp = obj_of(x.g, &name)?.display.clone();
    set_display(x.g, slot, Some(oid));
    x.success(Text::tr("commands.scoreboard.objectives.display.set", vec![lit(SLOT_NAMES[slot]), disp]), true);
    Ok(0)
}

// ---------------------------------------------------------------------------
// /scoreboard players
// ---------------------------------------------------------------------------

fn players_list(x: &mut Ctx) -> CmdResult<i32> {
    let (n, msg) = {
        let g = &*x.g;
        let holders = tracked_holders(g);
        let n = holders.len();
        let msg = if n == 0 {
            Text::key("commands.scoreboard.players.list.empty")
        } else {
            Text::tr("commands.scoreboard.players.list.success", vec![lit(n), join(holders.iter().map(|h| h.display(g)).collect())])
        };
        (n, msg)
    };
    x.success(msg, false);
    Ok(n as i32)
}

fn players_list_target(x: &mut Ctx) -> CmdResult<i32> {
    let h = x.score_holder("target")?;
    let msgs: Vec<Text> = {
        let g = &*x.g;
        let disp = h.display(g);
        let scores = holder_scores(g, &h.name);
        if scores.is_empty() {
            vec![Text::tr("commands.scoreboard.players.list.entity.empty", vec![disp])]
        } else {
            let mut v = vec![Text::tr("commands.scoreboard.players.list.entity.success", vec![disp, lit(scores.len())])];
            for (oid, val) in &scores {
                if let Some(o) = g.cs.scoreboard.obj(*oid) {
                    v.push(Text::tr("commands.scoreboard.players.list.entity.entry", vec![fmt_obj(o), lit(val)]));
                }
            }
            v
        }
    };
    let n = holder_scores(x.g, &h.name).len();
    for m in msgs {
        x.success(m, false);
    }
    Ok(n as i32)
}

/// `ScoreboardCommand.performOperation` operator semantics.
fn apply_op(op: Operation, a: i32, b: i32) -> CmdResult<(i32, Option<i32>)> {
    let div0 = || rine_command::CmdError::key("arguments.operation.div0");
    Ok(match op {
        Operation::Assign => (b, None),
        Operation::Add => (a.wrapping_add(b), None),
        Operation::Sub => (a.wrapping_sub(b), None),
        Operation::Mul => (a.wrapping_mul(b), None),
        Operation::Div => {
            if b == 0 {
                return Err(div0());
            }
            (floor_div(a, b), None)
        }
        Operation::Mod => {
            if b == 0 {
                return Err(div0());
            }
            // `Mth.positiveModulo` = `Math.floorMod` (sign of the divisor)
            ((a as i64).rem_euclid(b as i64) as i32 + if b < 0 && (a as i64).rem_euclid(b as i64) != 0 { b } else { 0 }, None)
        }
        Operation::Min => (a.min(b), None),
        Operation::Max => (a.max(b), None),
        Operation::Swap => (b, Some(a)),
    })
}

fn floor_div(a: i32, b: i32) -> i32 {
    let q = a.wrapping_div(b);
    if a.wrapping_rem(b) != 0 && ((a < 0) != (b < 0)) {
        q.wrapping_sub(1)
    } else {
        q
    }
}

fn players_set(x: &mut Ctx) -> CmdResult<i32> {
    let hs = x.score_holders_wild("targets")?;
    let name = x.writable_objective("objective")?;
    let value = x.int("score");
    let oid = oid_of(x.g, &name)?;
    let mut t = Track::default();
    for (i, h) in hs.iter().enumerate() {
        put_value(x.g, h, oid, value);
        t.add(i, value);
    }
    let o_disp = fmt_obj(obj_of(x.g, &name)?);
    respond(
        x,
        &t,
        false,
        None,
        true,
        |g, i, _| Text::tr("commands.scoreboard.players.set.success.single", vec![o_disp.clone(), hs[i].display(g), lit(value)]),
        |n, _| Text::tr("commands.scoreboard.players.set.success.multiple", vec![o_disp.clone(), lit(n), lit(value)]),
    )
}

/// Add or remove a constant from each holder's score (`addScore` / `removeScore`).
fn change_score(x: &mut Ctx, add: bool) -> CmdResult<i32> {
    let hs = x.score_holders_wild("targets")?;
    let name = x.writable_objective("objective")?;
    let value = x.int("score");
    let oid = oid_of(x.g, &name)?;
    let mut t = Track::default();
    for (i, h) in hs.iter().enumerate() {
        let mut a = acc(x.g, h, oid);
        let cur = acc_value(x.g, &a);
        let new = if add { cur.wrapping_add(value) } else { cur.wrapping_sub(value) };
        acc_set(x.g, h, &mut a, new);
        t.add(i, new);
    }
    let o_disp = fmt_obj(obj_of(x.g, &name)?);
    let key_single = if add { "commands.scoreboard.players.add.success.single" } else { "commands.scoreboard.players.remove.success.single" };
    let key_multi = if add { "commands.scoreboard.players.add.success.multiple" } else { "commands.scoreboard.players.remove.success.multiple" };
    respond(
        x,
        &t,
        true,
        None,
        true,
        |g, i, total| Text::tr(key_single, vec![lit(value), o_disp.clone(), hs[i].display(g), lit(total)]),
        |n, _| Text::tr(key_multi, vec![lit(value), o_disp.clone(), lit(n)]),
    )
}

fn players_add(x: &mut Ctx) -> CmdResult<i32> {
    change_score(x, true)
}

fn players_remove(x: &mut Ctx) -> CmdResult<i32> {
    change_score(x, false)
}

fn players_get(x: &mut Ctx) -> CmdResult<i32> {
    let h = x.score_holder("target")?;
    let name = x.objective("objective")?;
    let (value, msg) = {
        let g = &*x.g;
        let o = obj_of(g, &name)?;
        match score(g, &h.name, &name) {
            None => return Err(CmdError::tr("commands.scoreboard.players.get.null", vec![lit(&name), h.display(g)])),
            Some(v) => (v, Text::tr("commands.scoreboard.players.get.success", vec![h.display(g), lit(v), fmt_obj(o)])),
        }
    };
    x.success(msg, false);
    Ok(value)
}

fn players_reset_all(x: &mut Ctx) -> CmdResult<i32> {
    let hs = x.score_holders_wild("targets")?;
    let mut t = Track::default();
    for (i, h) in hs.iter().enumerate() {
        reset_all(x.g, &h.name);
        t.add(i, 1);
    }
    respond(
        x,
        &t,
        false,
        None,
        true,
        |g, i, _| Text::tr("commands.scoreboard.players.reset.all.single", vec![hs[i].display(g)]),
        |n, _| Text::tr("commands.scoreboard.players.reset.all.multiple", vec![lit(n)]),
    )
}

fn players_reset(x: &mut Ctx) -> CmdResult<i32> {
    let hs = x.score_holders_wild("targets")?;
    let name = x.objective("objective")?;
    let oid = oid_of(x.g, &name)?;
    let mut t = Track::default();
    for (i, h) in hs.iter().enumerate() {
        reset_single(x.g, &h.name, oid);
        t.add(i, 1);
    }
    let o_disp = fmt_obj(obj_of(x.g, &name)?);
    respond(
        x,
        &t,
        false,
        None,
        true,
        |g, i, _| Text::tr("commands.scoreboard.players.reset.specific.single", vec![o_disp.clone(), hs[i].display(g)]),
        |n, _| Text::tr("commands.scoreboard.players.reset.specific.multiple", vec![o_disp.clone(), lit(n)]),
    )
}

fn players_enable(x: &mut Ctx) -> CmdResult<i32> {
    let hs = x.score_holders_wild("targets")?;
    let name = x.objective("objective")?;
    let oid = oid_of(x.g, &name)?;
    if !obj_of(x.g, &name)?.criteria.trigger {
        return Err(CmdError::key("commands.scoreboard.players.enable.invalid"));
    }
    let mut t = Track::default();
    for (i, h) in hs.iter().enumerate() {
        let mut a = acc(x.g, h, oid);
        if acc_locked(x.g, &a) {
            acc_lock(x.g, &mut a, false);
            t.add(i, 1);
        }
    }
    let o_disp = fmt_obj(obj_of(x.g, &name)?);
    respond(
        x,
        &t,
        false,
        Some("commands.scoreboard.players.enable.failed"),
        true,
        |g, i, _| Text::tr("commands.scoreboard.players.enable.success.single", vec![o_disp.clone(), hs[i].display(g)]),
        |n, _| Text::tr("commands.scoreboard.players.enable.success.multiple", vec![o_disp.clone(), lit(n)]),
    )
}

/// `setScoreDisplay` with an optional display name.
fn set_score_display(x: &mut Ctx, display: Option<Text>) -> CmdResult<i32> {
    let hs = x.score_holders_wild("targets")?;
    let name = x.objective("objective")?;
    let oid = oid_of(x.g, &name)?;
    let mut t = Track::default();
    for (i, h) in hs.iter().enumerate() {
        let mut a = acc(x.g, h, oid);
        acc_display(x.g, &mut a, display.clone());
        t.add(i, 1);
    }
    let o_disp = fmt_obj(obj_of(x.g, &name)?);
    match display {
        None => respond(
            x,
            &t,
            false,
            None,
            true,
            |g, i, _| Text::tr("commands.scoreboard.players.display.name.clear.success.single", vec![hs[i].display(g), o_disp.clone()]),
            |n, _| Text::tr("commands.scoreboard.players.display.name.clear.success.multiple", vec![lit(n), o_disp.clone()]),
        ),
        Some(d) => respond(
            x,
            &t,
            false,
            None,
            true,
            |g, i, _| Text::tr("commands.scoreboard.players.display.name.set.success.single", vec![d.clone(), hs[i].display(g), o_disp.clone()]),
            |n, _| Text::tr("commands.scoreboard.players.display.name.set.success.multiple", vec![d.clone(), lit(n), o_disp.clone()]),
        ),
    }
}

fn display_name_clear(x: &mut Ctx) -> CmdResult<i32> {
    set_score_display(x, None)
}

fn display_name_set(x: &mut Ctx) -> CmdResult<i32> {
    let d = x.component("name")?;
    set_score_display(x, Some(d))
}

/// `setScoreNumberFormat`.
fn set_score_format(x: &mut Ctx, fmt: Option<NumFmt>) -> CmdResult<i32> {
    let hs = x.score_holders_wild("targets")?;
    let name = x.objective("objective")?;
    let oid = oid_of(x.g, &name)?;
    let mut t = Track::default();
    for (i, h) in hs.iter().enumerate() {
        let mut a = acc(x.g, h, oid);
        acc_format(x.g, &mut a, fmt.clone());
        t.add(i, 1);
    }
    let o_disp = fmt_obj(obj_of(x.g, &name)?);
    match fmt {
        None => respond(
            x,
            &t,
            false,
            None,
            true,
            |g, i, _| Text::tr("commands.scoreboard.players.display.numberFormat.clear.success.single", vec![hs[i].display(g), o_disp.clone()]),
            |n, _| Text::tr("commands.scoreboard.players.display.numberFormat.clear.success.multiple", vec![lit(n), o_disp.clone()]),
        ),
        Some(_) => respond(
            x,
            &t,
            false,
            None,
            true,
            |g, i, _| Text::tr("commands.scoreboard.players.display.numberFormat.set.success.single", vec![hs[i].display(g), o_disp.clone()]),
            |n, _| Text::tr("commands.scoreboard.players.display.numberFormat.set.success.multiple", vec![lit(n), o_disp.clone()]),
        ),
    }
}

fn score_format(x: &mut Ctx, fmt: Option<NumFmt>) -> CmdResult<i32> {
    set_score_format(x, fmt)
}

fn score_format_fixed(x: &mut Ctx) -> CmdResult<i32> {
    let t = x.component("contents")?;
    set_score_format(x, Some(NumFmt::Fixed(t)))
}

fn score_format_styled(x: &mut Ctx) -> CmdResult<i32> {
    let t = x.nbt("style").clone();
    set_score_format(x, Some(NumFmt::Styled(t)))
}

fn players_operation(x: &mut Ctx) -> CmdResult<i32> {
    let targets = x.score_holders_wild("targets")?;
    let tname = x.writable_objective("targetObjective")?;
    let op = match x.arg("operation") {
        Arg::Operation(o) => *o,
        _ => Operation::Assign,
    };
    let sources = x.score_holders_wild("source")?;
    let sname = x.objective("sourceObjective")?;
    let toid = oid_of(x.g, &tname)?;
    let soid = oid_of(x.g, &sname)?;
    let mut t = Track::default();
    for (i, th) in targets.iter().enumerate() {
        let mut ta = acc(x.g, th, toid);
        for sh in &sources {
            let mut sa = acc(x.g, sh, soid);
            let a = acc_value(x.g, &ta);
            let b = acc_value(x.g, &sa);
            let (na, swap) = apply_op(op, a, b)?;
            acc_set(x.g, th, &mut ta, na);
            if let Some(sv) = swap {
                acc_set(x.g, sh, &mut sa, sv);
            }
        }
        t.add(i, acc_value(x.g, &ta));
    }
    let o_disp = fmt_obj(obj_of(x.g, &tname)?);
    respond(
        x,
        &t,
        true,
        None,
        true,
        |g, i, total| Text::tr("commands.scoreboard.players.operation.success.single", vec![o_disp.clone(), targets[i].display(g), lit(total)]),
        |n, _| Text::tr("commands.scoreboard.players.operation.success.multiple", vec![o_disp.clone(), lit(n)]),
    )
}

// ---------------------------------------------------------------------------
// /team
// ---------------------------------------------------------------------------

fn team_arg(x: &Ctx, n: &str) -> CmdResult<u32> {
    let name = x.str(n);
    x.g.cs.scoreboard.team_by_name(name).ok_or_else(|| CmdError::tr("team.notFound", vec![lit(name)]))
}

fn team_at(x: &Ctx, tid: u32) -> CmdResult<Team> {
    x.g.cs.scoreboard.team(tid).cloned().ok_or_else(|| CmdError::key("team.notFound"))
}

fn team_list(x: &mut Ctx) -> CmdResult<i32> {
    let (n, msg) = {
        let items: Vec<Text> = x.g.cs.scoreboard.teams().map(fmt_team).collect();
        let n = items.len();
        let msg = if n == 0 {
            Text::key("commands.team.list.teams.empty")
        } else {
            Text::tr("commands.team.list.teams.success", vec![lit(n), join(items)])
        };
        (n, msg)
    };
    x.success(msg, false);
    Ok(n as i32)
}

fn team_list_members(x: &mut Ctx) -> CmdResult<i32> {
    let tid = team_arg(x, "team")?;
    let t = team_at(x, tid)?;
    let n = t.members.len();
    let msg = if n == 0 {
        Text::tr("commands.team.list.members.empty", vec![fmt_team(&t)])
    } else {
        Text::tr(
            "commands.team.list.members.success",
            vec![fmt_team(&t), lit(n), join(t.members.iter().map(|m| Text::plain(m.clone())).collect())],
        )
    };
    x.success(msg, false);
    Ok(n as i32)
}

fn create_team(x: &mut Ctx, name: String, display: Option<Text>) -> CmdResult<i32> {
    if x.g.cs.scoreboard.team_by_name(&name).is_some() {
        return Err(CmdError::key("commands.team.add.duplicate"));
    }
    let tid = add_team(x.g, &name);
    if let Some(d) = display {
        if let Some(t) = x.g.cs.scoreboard.teams.get_mut(tid as usize).and_then(|t| t.as_mut()) {
            t.display = d;
        }
        team_changed(x.g, tid);
    }
    let (msg, n) = {
        let t = team_at(x, tid)?;
        (Text::tr("commands.team.add.success", vec![fmt_team(&t)]), x.g.cs.scoreboard.teams().count())
    };
    x.success(msg, true);
    Ok(n as i32)
}

fn team_add(x: &mut Ctx) -> CmdResult<i32> {
    let name = x.str("team").to_string();
    create_team(x, name, None)
}

fn team_add_named(x: &mut Ctx) -> CmdResult<i32> {
    let name = x.str("team").to_string();
    let d = x.component("displayName")?;
    create_team(x, name, Some(d))
}

fn team_remove(x: &mut Ctx) -> CmdResult<i32> {
    let tid = team_arg(x, "team")?;
    let t = team_at(x, tid)?;
    let msg = Text::tr("commands.team.remove.success", vec![fmt_team(&t)]);
    remove_team(x.g, tid);
    let n = x.g.cs.scoreboard.teams().count();
    x.success(msg, true);
    Ok(n as i32)
}

fn team_empty(x: &mut Ctx) -> CmdResult<i32> {
    let tid = team_arg(x, "team")?;
    let t = team_at(x, tid)?;
    if t.members.is_empty() {
        return Err(CmdError::key("commands.team.empty.unchanged"));
    }
    for m in &t.members {
        leave_team(x.g, m);
    }
    x.success(Text::tr("commands.team.empty.success", vec![lit(t.members.len()), fmt_team(&t)]), true);
    Ok(t.members.len() as i32)
}

fn join_members(x: &mut Ctx, tid: u32, members: Vec<Holder>) -> CmdResult<i32> {
    let team = team_at(x, tid)?;
    let mut t = Track::default();
    for (i, m) in members.iter().enumerate() {
        let ok = join_team(x.g, &m.name, tid);
        t.add(i, ok as i32);
    }
    let ft = fmt_team(&team);
    respond(
        x,
        &t,
        false,
        None,
        true,
        |g, i, _| Text::tr("commands.team.join.success.single", vec![members[i].display(g), ft.clone()]),
        |n, _| Text::tr("commands.team.join.success.multiple", vec![lit(n), ft.clone()]),
    )
}

fn team_join_self(x: &mut Ctx) -> CmdResult<i32> {
    let tid = team_arg(x, "team")?;
    let eid = x.source_entity()?;
    let name = select::scoreboard_name(x.g, eid);
    join_members(x, tid, vec![Holder { name, entity: Some(eid) }])
}

fn team_join(x: &mut Ctx) -> CmdResult<i32> {
    let tid = team_arg(x, "team")?;
    let members = x.score_holders_wild("members")?;
    join_members(x, tid, members)
}

fn team_leave(x: &mut Ctx) -> CmdResult<i32> {
    let members = x.score_holders_wild("members")?;
    let mut t = Track::default();
    for (i, m) in members.iter().enumerate() {
        t.add(i, leave_team(x.g, &m.name) as i32);
    }
    respond(
        x,
        &t,
        false,
        None,
        true,
        |g, i, _| Text::tr("commands.team.leave.success.single", vec![members[i].display(g)]),
        |n, _| Text::tr("commands.team.leave.success.multiple", vec![lit(n)]),
    )
}

/// Apply a change to a team and send its CHANGE packet.
fn modify_team(x: &mut Ctx, tid: u32, f: impl FnOnce(&mut Team)) {
    if let Some(t) = x.g.cs.scoreboard.teams.get_mut(tid as usize).and_then(|t| t.as_mut()) {
        f(t);
    }
    team_changed(x.g, tid);
}

fn team_set_display(x: &mut Ctx) -> CmdResult<i32> {
    let tid = team_arg(x, "team")?;
    let d = x.component("displayName")?;
    let t = team_at(x, tid)?;
    if t.display == d {
        return Err(CmdError::key("commands.team.option.name.unchanged"));
    }
    modify_team(x, tid, |t| t.display = d);
    let t = team_at(x, tid)?;
    x.success(Text::tr("commands.team.option.name.success", vec![fmt_team(&t)]), true);
    Ok(0)
}

fn team_clear_color(x: &mut Ctx) -> CmdResult<i32> {
    let tid = team_arg(x, "team")?;
    let t = team_at(x, tid)?;
    if t.color.is_none() {
        return Err(CmdError::key("commands.team.option.color.unchanged"));
    }
    modify_team(x, tid, |t| t.color = None);
    x.success(Text::tr("commands.team.option.color.clear.success", vec![fmt_team(&t)]), true);
    Ok(0)
}

fn team_set_color(x: &mut Ctx) -> CmdResult<i32> {
    let tid = team_arg(x, "team")?;
    let color = match x.arg("value") {
        Arg::Color(c) => *c & 15,
        _ => 15,
    };
    let t = team_at(x, tid)?;
    if t.color == Some(color) {
        return Err(CmdError::key("commands.team.option.color.unchanged"));
    }
    modify_team(x, tid, |t| t.color = Some(color));
    x.success(Text::tr("commands.team.option.color.success", vec![fmt_team(&t), lit(COLOR_NAMES[color as usize])]), true);
    Ok(0)
}

fn team_friendly_fire(x: &mut Ctx) -> CmdResult<i32> {
    let tid = team_arg(x, "team")?;
    let allowed = x.bool("allowed");
    let t = team_at(x, tid)?;
    if t.friendly_fire == allowed {
        let key = if allowed { "commands.team.option.friendlyfire.alreadyEnabled" } else { "commands.team.option.friendlyfire.alreadyDisabled" };
        return Err(CmdError::key(key));
    }
    modify_team(x, tid, |t| t.friendly_fire = allowed);
    let key = if allowed { "commands.team.option.friendlyfire.enabled" } else { "commands.team.option.friendlyfire.disabled" };
    x.success(Text::tr(key, vec![fmt_team(&t)]), true);
    Ok(0)
}

fn team_see_invisibles(x: &mut Ctx) -> CmdResult<i32> {
    let tid = team_arg(x, "team")?;
    let allowed = x.bool("allowed");
    let t = team_at(x, tid)?;
    if t.see_invisibles == allowed {
        let key = if allowed { "commands.team.option.seeFriendlyInvisibles.alreadyEnabled" } else { "commands.team.option.seeFriendlyInvisibles.alreadyDisabled" };
        return Err(CmdError::key(key));
    }
    modify_team(x, tid, |t| t.see_invisibles = allowed);
    let key = if allowed { "commands.team.option.seeFriendlyInvisibles.enabled" } else { "commands.team.option.seeFriendlyInvisibles.disabled" };
    x.success(Text::tr(key, vec![fmt_team(&t)]), true);
    Ok(0)
}

/// `which` 0 nametag, 1 death message.
fn team_set_visibility(x: &mut Ctx, vis: u8, which: u8) -> CmdResult<i32> {
    let tid = team_arg(x, "team")?;
    let t = team_at(x, tid)?;
    let cur = if which == 0 { t.name_tag } else { t.death_msg };
    if cur == vis {
        let key = if which == 0 { "commands.team.option.nametagVisibility.unchanged" } else { "commands.team.option.deathMessageVisibility.unchanged" };
        return Err(CmdError::key(key));
    }
    modify_team(x, tid, |t| {
        if which == 0 {
            t.name_tag = vis
        } else {
            t.death_msg = vis
        }
    });
    let key = if which == 0 { "commands.team.option.nametagVisibility.success" } else { "commands.team.option.deathMessageVisibility.success" };
    let vis_key = format!("team.visibility.{}", VIS_NAMES[vis as usize]);
    x.success(Text::tr(key, vec![fmt_team(&t), Text::key(vis_key)]), true);
    Ok(0)
}

fn team_set_collision(x: &mut Ctx, rule: u8) -> CmdResult<i32> {
    let tid = team_arg(x, "team")?;
    let t = team_at(x, tid)?;
    if t.collision == rule {
        return Err(CmdError::key("commands.team.option.collisionRule.unchanged"));
    }
    modify_team(x, tid, |t| t.collision = rule);
    let rule_key = format!("team.collision.{}", COLL_NAMES[rule as usize]);
    x.success(Text::tr("commands.team.option.collisionRule.success", vec![fmt_team(&t), Text::key(rule_key)]), true);
    Ok(0)
}

fn team_prefix(x: &mut Ctx) -> CmdResult<i32> {
    let tid = team_arg(x, "team")?;
    let p = x.component("prefix")?;
    modify_team(x, tid, |t| t.prefix = p.clone());
    x.success(Text::tr("commands.team.option.prefix.success", vec![p]), false);
    Ok(1)
}

fn team_suffix(x: &mut Ctx) -> CmdResult<i32> {
    let tid = team_arg(x, "team")?;
    let p = x.component("suffix")?;
    modify_team(x, tid, |t| t.suffix = p.clone());
    x.success(Text::tr("commands.team.option.suffix.success", vec![p]), false);
    Ok(1)
}

// ---------------------------------------------------------------------------
// /trigger
// ---------------------------------------------------------------------------

/// `TriggerCommand.getScore`: the trigger must be primed (unlocked), then locked.
fn trigger_score(x: &mut Ctx, name: &str) -> CmdResult<(Holder, u32, i32)> {
    let slot = x.source_player()?;
    let (eid, pname) = match x.g.players.get(slot).and_then(|p| p.as_ref()) {
        Some(p) => (p.eid, p.name.clone()),
        None => return Err(CmdError::key("permissions.requires.player")),
    };
    let h = Holder { name: pname, entity: Some(eid) };
    let oid = oid_of(x.g, name)?;
    if !obj_of(x.g, name)?.criteria.trigger {
        return Err(CmdError::key("commands.trigger.failed.invalid"));
    }
    let primed = {
        let sb = &x.g.cs.scoreboard;
        sb.holder_idx.get(&h.name).and_then(|hid| sb.scores.get(&(*hid, oid))).is_some_and(|s| !s.locked)
    };
    if !primed {
        return Err(CmdError::key("commands.trigger.failed.unprimed"));
    }
    let mut a = acc(x.g, &h, oid);
    acc_lock(x.g, &mut a, true);
    Ok((h, oid, acc_value(x.g, &a)))
}

fn trigger_simple(x: &mut Ctx) -> CmdResult<i32> {
    let name = x.objective("objective")?;
    let (h, oid, cur) = trigger_score(x, &name)?;
    let new = cur.wrapping_add(1);
    put_value(x.g, &h, oid, new);
    let msg = Text::tr("commands.trigger.simple.success", vec![fmt_obj(obj_of(x.g, &name)?)]);
    x.success(msg, true);
    Ok(new)
}

fn trigger_add(x: &mut Ctx) -> CmdResult<i32> {
    let name = x.objective("objective")?;
    let amount = x.int("value");
    let (h, oid, cur) = trigger_score(x, &name)?;
    let new = cur.wrapping_add(amount);
    put_value(x.g, &h, oid, new);
    let msg = Text::tr("commands.trigger.add.success", vec![fmt_obj(obj_of(x.g, &name)?), lit(amount)]);
    x.success(msg, true);
    Ok(new)
}

fn trigger_set(x: &mut Ctx) -> CmdResult<i32> {
    let name = x.objective("objective")?;
    let amount = x.int("value");
    let (h, oid, _) = trigger_score(x, &name)?;
    put_value(x.g, &h, oid, amount);
    let msg = Text::tr("commands.trigger.set.success", vec![fmt_obj(obj_of(x.g, &name)?), lit(amount)]);
    x.success(msg, true);
    Ok(amount)
}

// ---------------------------------------------------------------------------
// /teammsg (and /tm)
// ---------------------------------------------------------------------------

fn teammsg(x: &mut Ctx) -> CmdResult<i32> {
    let eid = x.source_entity()?;
    let name = select::scoreboard_name(x.g, eid);
    let Some(tid) = x.g.cs.scoreboard.member_team.get(&name).copied() else {
        return Err(CmdError::key("commands.teammsg.failed.noteam"));
    };
    let team_name = match x.g.cs.scoreboard.team(tid) {
        Some(t) => t.name.clone(),
        None => return Err(CmdError::key("commands.teammsg.failed.noteam")),
    };
    let receivers: Vec<usize> = {
        let g = &*x.g;
        g.players
            .iter()
            .enumerate()
            .filter_map(|(s, p)| p.as_ref().filter(|p| p.eid == eid || team_of(g, &p.name).as_deref() == Some(team_name.as_str())).map(|_| s))
            .collect()
    };
    if receivers.is_empty() {
        return Ok(0);
    }
    let msg = x.message("message")?;
    let (sender, team_text, in_id, out_id) = {
        let g = &*x.g;
        let team = g.cs.scoreboard.team(tid).map(fmt_team).unwrap_or_else(|| Text::plain(team_name.clone()));
        let team_text = team.on_hover(hover_text(&Text::key("chat.type.team.hover"))).on_click(click_suggest("/teammsg "));
        let regs = &rine_data::get().registries;
        let reg = regs.synced("minecraft:chat_type");
        (
            (*x.src.display).clone(),
            team_text,
            reg.id("minecraft:team_msg_command_incoming"),
            reg.id("minecraft:team_msg_command_outgoing"),
        )
    };
    for slot in receivers {
        let outgoing = x.g.players.get(slot).and_then(|p| p.as_ref()).is_some_and(|p| p.eid == eid);
        let type_id = if outgoing { out_id } else { in_id };
        let Some(type_id) = type_id else { continue };
        let p = packet(C_DISGUISED_CHAT, |b| {
            out::write_text(b, &msg);
            b.w_varint(type_id as i32 + 1);
            out::write_text(b, &sender);
            b.w_bool(true);
            out::write_text(b, &team_text);
        });
        to_slot(x.g, slot, p);
    }
    let n = x.g.players.iter().flatten().filter(|p| p.eid == eid || team_of(x.g, &p.name).as_deref() == Some(team_name.as_str())).count();
    Ok(n as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vanilla-written file must inflate and parse to a `data` compound.
    #[test]
    fn vanilla_scoreboard_dat_parses() {
        let p = Path::new("E:/Projects/Rine-server/vanilla-data/run/world/data/minecraft/scoreboard.dat");
        if !p.exists() {
            return;
        }
        let root = read_root_file(p).expect("vanilla scoreboard.dat");
        assert!(root.iter().any(|(k, _)| k == "data"));
    }

    #[test]
    fn stored_gzip_round_trip() {
        let raw: Vec<u8> = (0..200_000u32).map(|i| (i * 31 % 251) as u8).collect();
        assert_eq!(gunzip(&gzip_stored(&raw)).unwrap(), raw);
        assert_eq!(gunzip(&gzip_stored(&[])).unwrap(), Vec::<u8>::new());
    }
}
