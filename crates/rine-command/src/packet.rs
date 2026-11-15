//! `ClientboundCommandsPacket` body for one op level.

use crate::tree::{Kind, NodeId, Tree};
use ahash::AHashMap;
use rine_protocol::Writer;
use std::collections::VecDeque;

fn arg_type_id(parser: &str) -> i32 {
    let regs = &rine_data::get().registries;
    regs.id("minecraft:command_argument_type", parser).map(|v| v as i32).unwrap_or_else(|| panic!("unknown parser {parser}"))
}

fn write_props<W: Writer + ?Sized>(w: &mut W, parser: &str, props: Option<&serde_json::Value>) {
    let p = props.cloned().unwrap_or(serde_json::Value::Null);
    let num = |k: &str| p.get(k).cloned();
    match parser {
        "brigadier:float" | "brigadier:double" | "brigadier:integer" | "brigadier:long" => {
            let (mn, mx) = (num("min"), num("max"));
            w.w_u8((mn.is_some() as u8) | ((mx.is_some() as u8) << 1));
            for v in [mn, mx].into_iter().flatten() {
                match parser {
                    "brigadier:float" => w.w_f32(v.as_f64().unwrap() as f32),
                    "brigadier:double" => w.w_f64(v.as_f64().unwrap()),
                    "brigadier:integer" => w.w_i32(v.as_i64().unwrap() as i32),
                    _ => w.w_i64(v.as_i64().unwrap()),
                }
            }
        }
        "brigadier:string" => {
            let t = p.get("type").and_then(|v| v.as_str()).unwrap_or("word");
            w.w_varint(match t {
                "word" => 0,
                "phrase" => 1,
                _ => 2,
            });
        }
        "minecraft:entity" => {
            let s = p.get("single").and_then(|v| v.as_bool()).unwrap_or(false);
            let pl = p.get("players").and_then(|v| v.as_bool()).unwrap_or(false);
            w.w_u8((s as u8) | ((pl as u8) << 1));
        }
        "minecraft:score_holder" => {
            let m = p.get("multiple").and_then(|v| v.as_bool()).unwrap_or(false);
            w.w_u8(m as u8);
        }
        "minecraft:time" => w.w_i32(p.get("min").and_then(|v| v.as_i64()).unwrap_or(0) as i32),
        "minecraft:resource_or_tag" | "minecraft:resource_or_tag_key" | "minecraft:resource" | "minecraft:resource_key"
        | "minecraft:resource_selector" => {
            w.w_str(p.get("registry").and_then(|v| v.as_str()).unwrap_or(""));
        }
        _ => {}
    }
}

/// Serialise the nodes usable at `level` (vanilla `fillUsableCommands`).
pub fn commands_packet_body<W: Writer + ?Sized>(tree: &Tree, level: u8, w: &mut W) {
    // BFS over visible nodes; redirect targets are always visible when the
    // redirecting node is (root, execute, or a sibling command).
    let visible = |id: NodeId| tree.node(id).level <= level;
    let mut order: Vec<NodeId> = Vec::new();
    let mut index: AHashMap<NodeId, i32> = AHashMap::new();
    let mut q = VecDeque::from([0u32]);
    while let Some(id) = q.pop_front() {
        if index.contains_key(&id) {
            continue;
        }
        index.insert(id, order.len() as i32);
        order.push(id);
        let n = tree.node(id);
        for &c in &n.children {
            if visible(c) && !index.contains_key(&c) {
                q.push_back(c);
            }
        }
        if let Some(r) = n.redirect {
            if !index.contains_key(&r) {
                q.push_back(r);
            }
        }
    }
    w.w_varint(order.len() as i32);
    for &id in &order {
        let n = tree.node(id);
        let kids: Vec<i32> = n.children.iter().filter(|&&c| visible(c)).map(|c| index[c]).collect();
        let mut flags = match n.kind {
            Kind::Root => 0u8,
            Kind::Literal => 1,
            Kind::Argument => 2,
        };
        if n.executable {
            flags |= 0x04;
        }
        if n.redirect.is_some() {
            flags |= 0x08;
        }
        if n.suggest.is_some() {
            flags |= 0x10;
        }
        if n.restricted {
            flags |= 0x20;
        }
        w.w_u8(flags);
        w.w_varint(kids.len() as i32);
        for k in kids {
            w.w_varint(k);
        }
        if let Some(r) = n.redirect {
            w.w_varint(index[&r]);
        }
        match n.kind {
            Kind::Root => {}
            Kind::Literal => w.w_str(n.name),
            Kind::Argument => {
                w.w_str(n.name);
                let p = n.parser.unwrap();
                w.w_varint(arg_type_id(p));
                write_props(w, p, n.props.as_ref());
                if let Some(s) = n.suggest {
                    w.w_str(s);
                }
            }
        }
    }
    w.w_varint(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rine_protocol::buf::Reader as PR;
    use std::collections::BTreeMap;

    /// path -> (flags, redirect path, raw node stub bytes after the name)
    fn decode(body: &[u8]) -> BTreeMap<String, (u8, Option<String>, Vec<u8>)> {
        struct N {
            flags: u8,
            kids: Vec<usize>,
            red: Option<usize>,
            name: String,
            tail: Vec<u8>,
        }
        let mut r = PR::new(body);
        let n = r.varint().unwrap() as usize;
        let mut nodes = Vec::new();
        for _ in 0..n {
            let flags = r.u8().unwrap();
            let k = r.varint().unwrap() as usize;
            let kids = (0..k).map(|_| r.varint().unwrap() as usize).collect();
            let red = if flags & 8 != 0 { Some(r.varint().unwrap() as usize) } else { None };
            let mut name = String::new();
            let mut tail = Vec::new();
            match flags & 3 {
                1 => name = r.string(1000).unwrap().to_string(),
                2 => {
                    name = r.string(1000).unwrap().to_string();
                    let before = body.len() - r.remaining();
                    let id = r.varint().unwrap();
                    let p = rine_data::get().registries.reg("minecraft:command_argument_type").name(id as u32).unwrap().to_string();
                    let skip = match p.as_str() {
                        "brigadier:float" | "brigadier:integer" => {
                            let f = r.u8().unwrap();
                            1 + 4 * ((f & 1) + ((f >> 1) & 1)) as usize - 1
                        }
                        "brigadier:double" | "brigadier:long" => {
                            let f = r.u8().unwrap();
                            1 + 8 * ((f & 1) + ((f >> 1) & 1)) as usize - 1
                        }
                        "brigadier:string" => {
                            r.varint().unwrap();
                            0
                        }
                        "minecraft:entity" | "minecraft:score_holder" => 1,
                        "minecraft:time" => 4,
                        "minecraft:resource_or_tag" | "minecraft:resource_or_tag_key" | "minecraft:resource"
                        | "minecraft:resource_key" | "minecraft:resource_selector" => {
                            r.string(1000).unwrap();
                            0
                        }
                        _ => 0,
                    };
                    for _ in 0..skip {
                        r.u8().unwrap();
                    }
                    if flags & 0x10 != 0 {
                        r.string(1000).unwrap();
                    }
                    let now = body.len() - r.remaining();
                    tail = body[before..now].to_vec();
                }
                _ => {}
            }
            nodes.push(N { flags, kids, red, name, tail });
        }
        let root = r.varint().unwrap() as usize;
        assert_eq!(r.remaining(), 0);
        let mut paths = vec![None; n];
        let mut stack = vec![(root, String::new())];
        while let Some((i, p)) = stack.pop() {
            if paths[i].is_some() {
                continue;
            }
            paths[i] = Some(p.clone());
            for &c in &nodes[i].kids {
                let cp = if p.is_empty() { nodes[c].name.clone() } else { format!("{p} {}", nodes[c].name) };
                stack.push((c, cp));
            }
        }
        let mut out = BTreeMap::new();
        for (i, nd) in nodes.iter().enumerate() {
            let p = paths[i].clone().unwrap_or_default();
            out.insert(p, (nd.flags, nd.red.map(|r| paths[r].clone().unwrap_or_default()), nd.tail.clone()));
        }
        out
    }

    #[test]
    fn tree_matches_vanilla_at_every_level() {
        rine_data::init();
        let tree = Tree::vanilla();
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/cmdtree");
        for level in 0..=4u8 {
            let Ok(vanilla) = std::fs::read(format!("{dir}/bot{level}-commands.bin")) else { continue };
            let mut ours = bytes::BytesMut::new();
            commands_packet_body(&tree, level, &mut ours);
            let a = decode(&vanilla);
            let b = decode(&ours);
            assert_eq!(a.len(), b.len(), "level {level} node count");
            for (k, v) in &a {
                assert_eq!(Some(v), b.get(k), "level {level} node {k:?}");
            }
        }
    }
}
