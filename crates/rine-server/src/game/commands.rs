//! Commands. `run` gets the command line without the leading slash.

use super::entity;
use super::player::Stack;
use super::Game;
use crate::out;
use rine_protocol::text::Text;

pub fn run(g: &mut Game, slot: usize, line: &str) {
    let name = g.player(slot).map(|p| p.name.clone()).unwrap_or_default();
    tracing::info!("{name} issued /{line}");
    let mut it = line.split_whitespace();
    let cmd = it.next().unwrap_or("").to_lowercase();
    let args: Vec<&str> = it.collect();
    let reply = |g: &mut Game, t: Text| {
        if let Some(p) = g.player(slot) {
            p.msg(&t);
        }
    };
    let op = g.player(slot).map(|p| p.op).unwrap_or(false);
    if !op && OP_ONLY.contains(&cmd.as_str()) {
        return reply(g, Text::plain("Нет прав на эту команду").color("red"));
    }
    match cmd.as_str() {
        "op" | "deop" => {
            let Some(target) = args.first().copied() else {
                return reply(g, Text::plain(format!("Использование: /{cmd} <игрок>")).color("red"));
            };
            let give = cmd == "op";
            {
                let ops = &mut g.ops;
                ops.retain(|n| !n.eq_ignore_ascii_case(target));
                if give {
                    ops.push(target.to_string());
                }
            }
            g.save_ops();
            if let Some(ts) = g.find_player(target) {
                set_op(g, ts, give);
            }
            reply(g, Text::plain(format!("{target}: {}", if give { "теперь оператор" } else { "больше не оператор" })).color("green"));
        }
        "help" => reply(
            g,
            Text::plain("Команды: /gamemode <s|c|a|sp>, /tp <x y z|игрок>, /give <предмет> [кол-во], /time <day|night|число>, /tps, /spawn, /tnt <кол-во>, /list, /seed").color("gray"),
        ),
        "gamemode" | "gm" => {
            let gm = match args.first().copied().unwrap_or("") {
                "0" | "s" | "survival" => 0,
                "1" | "c" | "creative" => 1,
                "2" | "a" | "adventure" => 2,
                "3" | "sp" | "spectator" => 3,
                _ => return reply(g, Text::plain("РСЃРїРѕР»СЊР·РѕРІР°РЅРёРµ: /gamemode <survival|creative|adventure|spectator>").color("red")),
            };
            set_gamemode(g, slot, gm);
        }
        "tp" | "teleport" => {
            let target = match args.len() {
                1 => g.find_player(args[0]).and_then(|s| g.players[s].as_ref()).map(|p| p.pos),
                3 => {
                    let cur = g.player(slot).map(|p| p.pos).unwrap_or_default();
                    let parse = |s: &str, c: f64| -> Option<f64> {
                        if let Some(rest) = s.strip_prefix('~') {
                            Some(c + if rest.is_empty() { 0.0 } else { rest.parse().ok()? })
                        } else {
                            s.parse().ok()
                        }
                    };
                    match (parse(args[0], cur[0]), parse(args[1], cur[1]), parse(args[2], cur[2])) {
                        (Some(x), Some(y), Some(z)) => Some([x, y, z]),
                        _ => None,
                    }
                }
                _ => None,
            };
            match target {
                Some([x, y, z]) => {
                    let p = g.player(slot).unwrap();
                    let (yaw, pitch) = (p.yaw, p.pitch);
                    p.teleport(x, y, z, yaw, pitch);
                }
                None => reply(g, Text::plain("РСЃРїРѕР»СЊР·РѕРІР°РЅРёРµ: /tp <x> <y> <z> РёР»Рё /tp <РёРіСЂРѕРє>").color("red")),
            }
        }
        "spawn" => {
            let s = g.spawn;
            let p = g.player(slot).unwrap();
            p.teleport(s[0], s[1], s[2], 0.0, 0.0);
        }
        "give" => {
            let Some(item_name) = args.first() else {
                return reply(g, Text::plain("РСЃРїРѕР»СЊР·РѕРІР°РЅРёРµ: /give <РїСЂРµРґРјРµС‚> [РєРѕР»-РІРѕ]").color("red"));
            };
            let count: i32 = args.get(1).and_then(|c| c.parse().ok()).unwrap_or(1).clamp(1, 64 * 36);
            match super::items::id(item_name) {
                Some(id) => {
                    let p = g.player(slot).unwrap();
                    let left = p.inv.add(super::player::Stack::new(id, count));
                    let inv = p.inventory_packet();
                    p.send(inv);
                    if left > 0 {
                        entity::drop_from_player(g, slot, Stack::new(id, left.min(64)), false);
                    }
                    super::track::equipment_changed(g, slot);
                    reply(g, Text::plain(format!("Выдано: {item_name} x{count}")).color("green"));
                }
                None => reply(g, Text::plain(format!("Нет такого предмета: {item_name}")).color("red")),
            }
        }
        "time" => {
            let v = match args.first().copied() {
                Some("day") => Some(1000),
                Some("noon") => Some(6000),
                Some("night") => Some(13000),
                Some("midnight") => Some(18000),
                Some(n) => n.parse().ok(),
                None => None,
            };
            match v {
                Some(t) => {
                    g.day_time = (g.day_time / 24000) * 24000 + t;
                    let ids = g.clock_ids.clone();
                    let f = out::frame(out::set_time(g.tick as i64, g.day_time, &ids, 1.0));
                    g.broadcast.push(f);
                }
                None => reply(g, Text::plain("РСЃРїРѕР»СЊР·РѕРІР°РЅРёРµ: /time <day|night|С‡РёСЃР»Рѕ>").color("red")),
            }
        }
        "tps" => {
            let ents = g.entities.list.len();
            let chunks = g.world.chunks.len();
            let t = Text::plain(format!("TPS {:.2}, MSPT {:.2}, сущностей {ents}, чанков {chunks}", g.tps, g.mspt)).color("aqua");
            reply(g, t);
        }
        "list" => {
            let names: Vec<String> = g.players.iter().flatten().map(|p| p.name.clone()).collect();
            reply(g, Text::plain(format!("Онлайн {}: {}", names.len(), names.join(", "))).color("gray"));
        }
        "seed" => {
            let s = g.shared.config.seed;
            reply(g, Text::plain(format!("Сид: {s}")).color("gray"));
        }
        "tnt" => {
            // spawn N primed TNT in a cube above the player: stress test
            let n: usize = args.first().and_then(|c| c.parse().ok()).unwrap_or(100).min(2_000_000);
            let pos = g.player(slot).unwrap().pos;
            let side = (n as f64).cbrt().ceil() as i64;
            let mut k = 0;
            'o: for dy in 0..side {
                for dx in 0..side {
                    for dz in 0..side {
                        if k >= n {
                            break 'o;
                        }
                        entity::spawn_tnt(g, [pos[0] + (dx - side / 2) as f64, pos[1] + 5.0 + dy as f64, pos[2] + (dz - side / 2) as f64], 80);
                        k += 1;
                    }
                }
            }
            reply(g, Text::plain(format!("Заспавнено TNT: {k}")).color("gold"));
        }
        "stop" => {
            g.running.store(false, std::sync::atomic::Ordering::Relaxed);
        }
        _ if super::plugins::has_command(g, &cmd) => {
            super::plugins::check(g, rine_plugin::SyncKind::Command, rine_plugin::Event::Command { name: name.clone(), line: line.to_string() });
        }
        _ => reply(g, Text::plain(format!("Неизвестная команда: /{cmd}. /help")).color("red")),
    }
}

/// Commands only operators may run.
pub const OP_ONLY: [&str; 10] = ["gamemode", "gm", "tp", "teleport", "give", "time", "tnt", "stop", "op", "deop"];

/// Apply operator status: entity_event 24+level makes the client offer
/// commands (and F3+F4 etc.), then resend the command tree for that level.
pub fn set_op(g: &mut Game, slot: usize, op: bool) {
    let Some(p) = g.player(slot) else { return };
    p.op = op;
    let eid = p.eid;
    p.send(out::entity_event(eid, if op { 28 } else { 24 }));
    p.send(commands_packet(op));
}

pub fn set_gamemode(g: &mut Game, slot: usize, gm: u8) {
    let Some(p) = g.player(slot) else { return };
    p.gamemode = gm;
    p.flying = false;
    p.send(out::game_event(3, gm as f32));
    let ab = p.abilities_packet();
    p.send(ab);
    let uuid = p.uuid;
    let names = ["выживание", "творческий", "приключение", "наблюдатель"];
    p.msg(&Text::plain(format!("Режим игры: {}", names[gm as usize])).color("green"));
    let f = out::frame(out::info_gamemode(uuid, gm as i32));
    g.broadcast.push(f);
}

/// Brigadier command tree for client-side completion (docs/spec/03 4.18).
pub fn commands_packet(op: bool) -> bytes::BytesMut {
    use rine_protocol::Writer;
    // nodes: 0 root; literals with a greedy string argument where useful
    let all = ["help", "gamemode", "gm", "tp", "teleport", "spawn", "give", "time", "tps", "list", "seed", "tnt", "stop", "op", "deop"];
    let lits: Vec<&str> = all.iter().copied().filter(|c| op || !OP_ONLY.contains(c)).collect();
    rine_protocol::frame::packet(rine_protocol::ids::play::C_COMMANDS, |b| {
        // root + per literal (literal node + greedy arg node)
        let n = 1 + lits.len() * 2;
        b.w_varint(n as i32);
        // root
        b.w_u8(0);
        b.w_varint(lits.len() as i32);
        for i in 0..lits.len() {
            b.w_varint(1 + i as i32 * 2);
        }
        for (i, l) in lits.iter().enumerate() {
            // literal: executable, child = arg
            b.w_u8(0x01 | 0x04);
            b.w_varint(1);
            b.w_varint(2 + i as i32 * 2);
            b.w_str(l);
            // argument "args": brigadier:string greedy, executable
            b.w_u8(0x02 | 0x04);
            b.w_varint(0);
            b.w_str("args");
            b.w_varint(5);
            b.w_varint(2);
        }
        b.w_varint(0);
    })
}
