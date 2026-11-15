//! The command graph. Loaded once from `assets/tree.json`, which is decoded
//! from the vanilla 26.3 server's `ClientboundCommandsPacket` at every op
//! level (tools/cmdtree/decode.py), so client and server see the same
//! grammar. Executors and redirect modifiers are attached by path.

use crate::args::ArgType;
use ahash::AHashMap;
use serde::Deserialize;

pub type NodeId = u32;
pub const ROOT: NodeId = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Root,
    Literal,
    Argument,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: Kind,
    /// Literal text or argument name.
    pub name: &'static str,
    /// Space-joined path from the root ("gamemode gamemode target").
    pub path: &'static str,
    pub children: Vec<NodeId>,
    /// Literal children by exact text (Brigadier `literals`).
    pub literals: AHashMap<&'static str, NodeId>,
    /// Argument children in declaration order (Brigadier `arguments`).
    pub arguments: Vec<NodeId>,
    pub redirect: Option<NodeId>,
    pub executable: bool,
    pub arg: Option<ArgType>,
    /// Suggestion provider id (minecraft:ask_server, ...).
    pub suggest: Option<&'static str>,
    /// Minimum op level that can see and use the node.
    pub level: u8,
    pub restricted: bool,
    /// Raw parser id and properties, for re-serialising the packet.
    pub parser: Option<&'static str>,
    pub props: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct RawNode {
    kind: String,
    name: String,
    exec: bool,
    children: Vec<u32>,
    redirect: Option<u32>,
    parser: Option<String>,
    props: Option<serde_json::Value>,
    suggest: Option<String>,
    level: u8,
    restricted: bool,
    path: String,
}

pub struct Tree {
    pub nodes: Vec<Node>,
    by_path: AHashMap<&'static str, NodeId>,
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

impl Tree {
    /// The vanilla 26.3 tree.
    pub fn vanilla() -> Self {
        let raw: Vec<RawNode> = serde_json::from_str(include_str!("../assets/tree.json")).expect("tree.json");
        let mut nodes = Vec::with_capacity(raw.len());
        for r in raw {
            let kind = match r.kind.as_str() {
                "root" => Kind::Root,
                "literal" => Kind::Literal,
                _ => Kind::Argument,
            };
            let arg = r.parser.as_deref().map(|p| ArgType::from_parser(p, r.props.as_ref()));
            nodes.push(Node {
                kind,
                name: leak(r.name),
                path: leak(r.path),
                children: r.children,
                literals: AHashMap::new(),
                arguments: Vec::new(),
                redirect: r.redirect,
                executable: r.exec,
                arg,
                suggest: r.suggest.map(leak),
                level: r.level,
                restricted: r.restricted,
                parser: r.parser.map(leak),
                props: r.props,
            });
        }
        let mut t = Tree { nodes, by_path: AHashMap::new() };
        t.reindex();
        t
    }

    fn reindex(&mut self) {
        self.by_path.clear();
        for i in 0..self.nodes.len() {
            let kids = self.nodes[i].children.clone();
            let mut lits = AHashMap::new();
            let mut args = Vec::new();
            for c in kids {
                let n = &self.nodes[c as usize];
                match n.kind {
                    Kind::Literal => {
                        lits.insert(n.name, c);
                    }
                    _ => args.push(c),
                }
            }
            self.nodes[i].literals = lits;
            self.nodes[i].arguments = args;
            // first path wins (redirect targets keep their own path)
            let p = self.nodes[i].path;
            self.by_path.entry(p).or_insert(i as NodeId);
        }
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    pub fn find(&self, path: &str) -> Option<NodeId> {
        self.by_path.get(path).copied()
    }

    /// Adds a top-level literal with a greedy string argument (plugin
    /// commands). No-op when the literal exists.
    pub fn add_plugin_command(&mut self, name: &str) {
        if self.nodes[0].literals.contains_key(name) {
            return;
        }
        let lit = self.nodes.len() as NodeId;
        let arg = lit + 1;
        let name_s = leak(name.to_string());
        let arg_path = leak(format!("{name} args"));
        self.nodes.push(Node {
            kind: Kind::Literal,
            name: name_s,
            path: name_s,
            children: vec![arg],
            literals: AHashMap::new(),
            arguments: Vec::new(),
            redirect: None,
            executable: true,
            arg: None,
            suggest: None,
            level: 0,
            restricted: false,
            parser: None,
            props: None,
        });
        self.nodes.push(Node {
            kind: Kind::Argument,
            name: "args",
            path: arg_path,
            children: vec![],
            literals: AHashMap::new(),
            arguments: Vec::new(),
            redirect: None,
            executable: true,
            arg: Some(ArgType::String(crate::args::StringKind::Greedy)),
            suggest: Some("minecraft:ask_server"),
            level: 0,
            restricted: false,
            parser: Some("brigadier:string"),
            props: Some(serde_json::json!({"type": "greedy"})),
        });
        self.nodes[0].children.push(lit);
        self.reindex();
    }

    /// All executable paths (for coverage checks).
    pub fn executable_paths(&self) -> Vec<&'static str> {
        self.nodes.iter().filter(|n| n.executable).map(|n| n.path).collect()
    }
}
