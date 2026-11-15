//! Brigadier `CommandDispatcher.getSmartUsage` (brigadier 1.3.11, checked
//! against its bytecode) over the Rine command tree.

use rine_command::tree::{Kind, NodeId, Tree, ROOT};

/// `CommandNode.getUsageText()`.
pub fn usage_text(t: &Tree, id: NodeId) -> String {
    let n = t.node(id);
    match n.kind {
        Kind::Root => String::new(),
        Kind::Literal => n.name.to_string(),
        Kind::Argument => format!("<{}>", n.name),
    }
}

/// `CommandNode.canUse`: the node's minimum op level is reached.
fn can_use(t: &Tree, id: NodeId, level: u8) -> bool {
    t.node(id).level <= level
}

fn smart(t: &Tree, id: NodeId, level: u8, optional: bool, deep: bool) -> Option<String> {
    if !can_use(t, id, level) {
        return None;
    }
    let node = t.node(id);
    let usage = usage_text(t, id);
    let self_ = if optional { format!("[{usage}]") } else { usage };
    let child_opt = node.executable;
    let (open, close) = if child_opt { ("[", "]") } else { ("(", ")") };
    if deep {
        return Some(self_);
    }
    if let Some(r) = node.redirect {
        let rt = if r == ROOT { "...".to_string() } else { format!("-> {}", usage_text(t, r)) };
        return Some(format!("{self_} {rt}"));
    }
    let children: Vec<NodeId> = node.children.iter().copied().filter(|&c| can_use(t, c, level)).collect();
    if children.len() == 1 {
        if let Some(u) = smart(t, children[0], level, child_opt, child_opt) {
            return Some(format!("{self_} {u}"));
        }
    } else if children.len() > 1 {
        // LinkedHashSet of the deep usages
        let mut set: Vec<String> = Vec::new();
        for &c in &children {
            if let Some(u) = smart(t, c, level, child_opt, true) {
                if !set.contains(&u) {
                    set.push(u);
                }
            }
        }
        if set.len() == 1 {
            let u = &set[0];
            let shown = if child_opt { format!("[{u}]") } else { u.clone() };
            return Some(format!("{self_} {shown}"));
        }
        if set.len() > 1 {
            let mut b = String::from(open);
            for (i, &c) in children.iter().enumerate() {
                if i > 0 {
                    b.push('|');
                }
                b.push_str(&usage_text(t, c));
            }
            b.push_str(close);
            return Some(format!("{self_} {b}"));
        }
    }
    Some(self_)
}

/// `getSmartUsage(node, source)`: one usage line per usable child.
pub fn smart_children(t: &Tree, node: NodeId, level: u8) -> Vec<String> {
    let optional = t.node(node).executable;
    t.node(node).children.iter().filter_map(|&c| smart(t, c, level, optional, false)).collect()
}
