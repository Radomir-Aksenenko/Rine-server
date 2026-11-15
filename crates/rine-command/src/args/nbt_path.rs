//! NBT paths (`NbtPathArgument` of 26.3): parsing, and get / set / insert /
//! remove over `rine_protocol::nbt::Tag`.
//!
//! Java passes `Tag` references around. Here a matched tag is a location (the
//! child indices from the root). Elements of primitive arrays are values in
//! Java (`ByteTag` and friends are immutable), so a match on one is an
//! (array location, index) pair and writes to it are dropped.
//!
//! Deviations from vanilla:
//! - `set`, `insert`, `remove` and `modify_each` on an empty path do nothing
//!   (Java indexes `nodes[-1]` and throws).
//! - `insert` takes any `Tag` as the root, not only a compound.

use super::snbt;
use crate::error::{lit, CmdError, CmdResult};
use crate::reader::Reader;
use rine_protocol::nbt::Tag;
use rine_protocol::text::Text;

/// Nesting limit used by `NbtPath.isTooDeep` (`MAX_DEPTH`).
const TOO_DEEP: usize = 512;

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// `{...}` at the root: root must match the compound.
    MatchRoot(Tag),
    /// `name` / `"name"`.
    Key(String),
    /// `name{...}`.
    MatchKey(String, Tag),
    /// `[]`.
    AllElements,
    /// `[n]` (negative from the end).
    Index(i32),
    /// `[{...}]`.
    MatchElement(Tag),
}

impl Node {
    /// `createPreferredParentTag`: what a missing intermediate tag becomes.
    fn preferred_parent(&self) -> Tag {
        match self {
            Node::Key(_) | Node::MatchKey(..) | Node::MatchRoot(_) => Tag::Compound(Vec::new()),
            Node::AllElements | Node::Index(_) | Node::MatchElement(_) => Tag::List(Vec::new()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NbtPath {
    /// The source text (for messages: `arguments.nbtpath.nothing_found`...).
    pub original: String,
    pub nodes: Vec<Node>,
    /// End of each node within `original` (Java `nodeToOriginalPosition`).
    positions: Vec<usize>,
}

/// `NbtPathArgument.parse`: reads nodes until a space or the end of input.
pub fn parse(r: &mut Reader) -> CmdResult<NbtPath> {
    let start = r.cursor;
    let mut nodes = Vec::new();
    let mut positions = Vec::new();
    let mut first = true;
    while r.can_read() && r.peek() != ' ' {
        let node = parse_node(r, first)?;
        nodes.push(node);
        positions.push(r.cursor - start);
        first = false;
        if r.can_read() {
            let next = r.peek();
            if next != ' ' && next != '[' && next != '{' {
                r.expect('.')?;
            }
        }
    }
    Ok(NbtPath { original: r.slice(start, r.cursor).to_string(), nodes, positions })
}

fn invalid_node(r: &Reader) -> CmdError {
    CmdError::at(Text::key("arguments.nbtpath.node.invalid"), r)
}

fn parse_node(r: &mut Reader, first: bool) -> CmdResult<Node> {
    match r.peek() {
        '"' | '\'' => {
            let name = r.read_string()?;
            read_object_node(r, name)
        }
        '[' => {
            r.skip();
            let next = r.peek();
            if next == '{' {
                let pattern = snbt::parse_compound(r)?;
                r.expect(']')?;
                Ok(Node::MatchElement(pattern))
            } else if next == ']' {
                r.skip();
                Ok(Node::AllElements)
            } else {
                let index = r.read_int()?;
                r.expect(']')?;
                Ok(Node::Index(index))
            }
        }
        '{' => {
            if !first {
                return Err(invalid_node(r));
            }
            let pattern = snbt::parse_compound(r)?;
            Ok(Node::MatchRoot(pattern))
        }
        _ => {
            let name = read_unquoted_name(r)?;
            read_object_node(r, name)
        }
    }
}

fn read_object_node(r: &mut Reader, name: String) -> CmdResult<Node> {
    if name.is_empty() {
        return Err(invalid_node(r));
    }
    if r.can_read() && r.peek() == '{' {
        let pattern = snbt::parse_compound(r)?;
        return Ok(Node::MatchKey(name, pattern));
    }
    Ok(Node::Key(name))
}

fn read_unquoted_name(r: &mut Reader) -> CmdResult<String> {
    let start = r.cursor;
    while r.can_read() && is_allowed_in_unquoted_name(r.peek()) {
        r.skip();
    }
    if r.cursor == start {
        return Err(invalid_node(r));
    }
    Ok(r.slice(start, r.cursor).to_string())
}

fn is_allowed_in_unquoted_name(c: char) -> bool {
    !matches!(c, ' ' | '"' | '\'' | '[' | ']' | '.' | '{' | '}')
}

// ----- matching helpers ---------------------------------------------------------

fn is_collection(t: &Tag) -> bool {
    matches!(t, Tag::List(_) | Tag::ByteArray(_) | Tag::IntArray(_) | Tag::LongArray(_))
}

fn is_numeric(t: &Tag) -> bool {
    matches!(t, Tag::Byte(_) | Tag::Short(_) | Tag::Int(_) | Tag::Long(_) | Tag::Float(_) | Tag::Double(_))
}

/// `Mth.floor(double)`: saturating cast, then round toward negative infinity.
fn mth_floor(v: f64) -> i32 {
    let i = v as i32;
    if v < i as f64 {
        i.wrapping_sub(1)
    } else {
        i
    }
}

/// `NumericTag.byteValue()`.
fn num_byte(t: &Tag) -> i8 {
    match t {
        Tag::Byte(v) => *v,
        Tag::Short(v) => *v as i8,
        Tag::Int(v) => *v as i8,
        Tag::Long(v) => *v as i8,
        Tag::Float(v) => (mth_floor(*v as f64) & 0xFF) as i8,
        Tag::Double(v) => (mth_floor(*v) & 0xFF) as i8,
        _ => 0,
    }
}

/// `NumericTag.intValue()`.
fn num_int(t: &Tag) -> i32 {
    match t {
        Tag::Byte(v) => *v as i32,
        Tag::Short(v) => *v as i32,
        Tag::Int(v) => *v,
        Tag::Long(v) => *v as i32,
        Tag::Float(v) => mth_floor(*v as f64),
        Tag::Double(v) => mth_floor(*v),
        _ => 0,
    }
}

/// `NumericTag.longValue()` (float truncates, double floors, as in vanilla).
fn num_long(t: &Tag) -> i64 {
    match t {
        Tag::Byte(v) => *v as i64,
        Tag::Short(v) => *v as i64,
        Tag::Int(v) => *v as i64,
        Tag::Long(v) => *v,
        Tag::Float(v) => *v as i64,
        Tag::Double(v) => v.floor() as i64,
        _ => 0,
    }
}

/// Element `i` of a collection as Java hands it out (arrays yield new tags).
fn coll_view(t: &Tag, i: usize) -> Option<Tag> {
    match t {
        Tag::List(l) => l.get(i).cloned(),
        Tag::ByteArray(a) => a.get(i).map(|v| Tag::Byte(*v)),
        Tag::IntArray(a) => a.get(i).map(|v| Tag::Int(*v)),
        Tag::LongArray(a) => a.get(i).map(|v| Tag::Long(*v)),
        _ => None,
    }
}

fn coll_len(t: &Tag) -> usize {
    match t {
        Tag::List(l) => l.len(),
        Tag::ByteArray(a) => a.len(),
        Tag::IntArray(a) => a.len(),
        Tag::LongArray(a) => a.len(),
        _ => 0,
    }
}

/// `CollectionTag.setTag`: arrays accept numeric tags only.
fn coll_set(t: &mut Tag, i: usize, v: &Tag) -> bool {
    match t {
        Tag::List(l) => match l.get_mut(i) {
            Some(slot) => {
                *slot = v.clone();
                true
            }
            None => false,
        },
        Tag::ByteArray(a) if is_numeric(v) => match a.get_mut(i) {
            Some(slot) => {
                *slot = num_byte(v);
                true
            }
            None => false,
        },
        Tag::IntArray(a) if is_numeric(v) => match a.get_mut(i) {
            Some(slot) => {
                *slot = num_int(v);
                true
            }
            None => false,
        },
        Tag::LongArray(a) if is_numeric(v) => match a.get_mut(i) {
            Some(slot) => {
                *slot = num_long(v);
                true
            }
            None => false,
        },
        _ => false,
    }
}

/// `CollectionTag.addTag`: `Ok(false)` when an array refuses the type; `Err`
/// when the index is out of bounds (Java's `IndexOutOfBoundsException`).
fn coll_add(t: &mut Tag, index: i64, v: &Tag) -> Result<bool, ()> {
    let len = coll_len(t) as i64;
    if matches!(t, Tag::List(_)) {
        if index < 0 || index > len {
            return Err(());
        }
        if let Tag::List(l) = t {
            l.insert(index as usize, v.clone());
        }
        return Ok(true);
    }
    if !is_numeric(v) {
        return Ok(false);
    }
    if index < 0 || index > len {
        return Err(());
    }
    let i = index as usize;
    match t {
        Tag::ByteArray(a) => a.insert(i, num_byte(v)),
        Tag::IntArray(a) => a.insert(i, num_int(v)),
        Tag::LongArray(a) => a.insert(i, num_long(v)),
        _ => return Ok(false),
    }
    Ok(true)
}

fn coll_remove(t: &mut Tag, i: usize) {
    match t {
        Tag::List(l) => {
            l.remove(i);
        }
        Tag::ByteArray(a) => {
            a.remove(i);
        }
        Tag::IntArray(a) => {
            a.remove(i);
        }
        Tag::LongArray(a) => {
            a.remove(i);
        }
        _ => {}
    }
}

fn coll_clear(t: &mut Tag) {
    match t {
        Tag::List(l) => l.clear(),
        Tag::ByteArray(a) => a.clear(),
        Tag::IntArray(a) => a.clear(),
        Tag::LongArray(a) => a.clear(),
        _ => {}
    }
}

/// `Object.equals` on tags: compounds compare as maps (order-insensitive),
/// float bits as `Float.compare` (NaN equals NaN, -0.0 differs from 0.0).
fn java_eq(a: &Tag, b: &Tag) -> bool {
    match (a, b) {
        (Tag::Byte(x), Tag::Byte(y)) => x == y,
        (Tag::Short(x), Tag::Short(y)) => x == y,
        (Tag::Int(x), Tag::Int(y)) => x == y,
        (Tag::Long(x), Tag::Long(y)) => x == y,
        (Tag::Float(x), Tag::Float(y)) => (x.is_nan() && y.is_nan()) || x.to_bits() == y.to_bits(),
        (Tag::Double(x), Tag::Double(y)) => (x.is_nan() && y.is_nan()) || x.to_bits() == y.to_bits(),
        (Tag::String(x), Tag::String(y)) => x == y,
        (Tag::ByteArray(x), Tag::ByteArray(y)) => x == y,
        (Tag::IntArray(x), Tag::IntArray(y)) => x == y,
        (Tag::LongArray(x), Tag::LongArray(y)) => x == y,
        (Tag::List(x), Tag::List(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| java_eq(p, q)),
        (Tag::Compound(x), Tag::Compound(y)) => {
            x.len() == y.len()
                && x.iter().all(|(k, v)| y.iter().any(|(yk, yv)| yk == k && java_eq(v, yv)))
        }
        _ => false,
    }
}

/// `NbtUtils.compareNbt(expected, actual, true)`: a pattern matches when every
/// key (and every list element) of the pattern is found in the actual tag.
fn compare(expected: &Tag, actual: &Tag) -> bool {
    match (expected, actual) {
        (Tag::Compound(e), Tag::Compound(a)) => {
            if a.len() < e.len() {
                return false;
            }
            e.iter().all(|(k, ev)| match a.iter().find(|(ak, _)| ak == k) {
                Some((_, av)) => compare(ev, av),
                None => false,
            })
        }
        (Tag::List(e), Tag::List(a)) => {
            if e.is_empty() {
                return a.is_empty();
            }
            if a.len() < e.len() {
                return false;
            }
            e.iter().all(|ev| a.iter().any(|av| compare(ev, av)))
        }
        _ => java_eq(expected, actual),
    }
}

/// `isTooDeep`: compounds and lists only, arrays are leaves.
fn is_too_deep(tag: &Tag, depth: usize) -> bool {
    if depth >= TOO_DEEP {
        return true;
    }
    match tag {
        Tag::Compound(c) => c.iter().any(|(_, v)| is_too_deep(v, depth + 1)),
        Tag::List(l) => l.iter().any(|v| is_too_deep(v, depth + 1)),
        _ => false,
    }
}

fn position_key(c: &[(String, Tag)], name: &str) -> Option<usize> {
    c.iter().position(|(k, _)| k == name)
}

/// What a node selects from one parent.
#[derive(Clone, Copy, Debug)]
enum Sel {
    /// Child of a compound (by position) or element of a list.
    Idx(usize),
    /// Element of a primitive array (a value, not addressable).
    Elem(usize),
    /// The parent itself (`MatchRoot`).
    SelfRef,
}

fn sel_for(parent: &Tag, i: usize) -> Sel {
    if matches!(parent, Tag::List(_)) {
        Sel::Idx(i)
    } else {
        Sel::Elem(i)
    }
}

/// `getTag` of each node type.
fn sels(node: &Node, parent: &Tag) -> Vec<Sel> {
    match node {
        Node::Key(name) => match parent {
            Tag::Compound(c) => position_key(c, name).map(Sel::Idx).into_iter().collect(),
            _ => Vec::new(),
        },
        Node::MatchKey(name, pattern) => match parent {
            Tag::Compound(c) => match position_key(c, name) {
                Some(p) if compare(pattern, &c[p].1) => vec![Sel::Idx(p)],
                _ => Vec::new(),
            },
            _ => Vec::new(),
        },
        Node::MatchRoot(pattern) => {
            if matches!(parent, Tag::Compound(_)) && compare(pattern, parent) {
                vec![Sel::SelfRef]
            } else {
                Vec::new()
            }
        }
        Node::Index(index) => {
            if !is_collection(parent) {
                return Vec::new();
            }
            let size = coll_len(parent) as i64;
            let actual = if *index < 0 { size + *index as i64 } else { *index as i64 };
            if actual >= 0 && actual < size {
                vec![sel_for(parent, actual as usize)]
            } else {
                Vec::new()
            }
        }
        Node::AllElements => {
            if is_collection(parent) {
                (0..coll_len(parent)).map(|i| sel_for(parent, i)).collect()
            } else {
                Vec::new()
            }
        }
        Node::MatchElement(pattern) => match parent {
            Tag::List(l) => l
                .iter()
                .enumerate()
                .filter(|(_, e)| compare(pattern, e))
                .map(|(i, _)| Sel::Idx(i))
                .collect(),
            _ => Vec::new(),
        },
    }
}

/// `getOrCreateTag` creation step: when the node has nothing to select, it
/// adds one child (a copy of the pattern, or `fill` for untyped children).
/// Returns the new selection, or `None` when nothing was created.
fn ensure(node: &Node, parent: &mut Tag, fill: &Tag) -> Option<Sel> {
    match (node, parent) {
        (Node::Key(name), Tag::Compound(c)) => {
            if c.iter().any(|(k, _)| k == name) {
                return None;
            }
            c.push((name.clone(), fill.clone()));
            Some(Sel::Idx(c.len() - 1))
        }
        (Node::MatchKey(name, pattern), Tag::Compound(c)) => {
            if c.iter().any(|(k, _)| k == name) {
                return None;
            }
            c.push((name.clone(), pattern.clone()));
            Some(Sel::Idx(c.len() - 1))
        }
        (Node::AllElements, Tag::List(l)) => {
            if !l.is_empty() {
                return None;
            }
            l.push(fill.clone());
            Some(Sel::Idx(0))
        }
        (Node::MatchElement(pattern), Tag::List(l)) => {
            if l.iter().any(|e| compare(pattern, e)) {
                return None;
            }
            l.push(pattern.clone());
            Some(Sel::Idx(l.len() - 1))
        }
        _ => None,
    }
}

/// `setTag` of each node type, applied to the parent. Returns the change count.
fn set_tag(node: &Node, parent: &mut Tag, value: &Tag) -> i32 {
    match node {
        Node::Key(name) => match parent {
            Tag::Compound(c) => match position_key(c, name) {
                Some(p) => {
                    let changed = !java_eq(value, &c[p].1);
                    c[p].1 = value.clone();
                    changed as i32
                }
                None => {
                    c.push((name.clone(), value.clone()));
                    1
                }
            },
            _ => 0,
        },
        Node::MatchKey(name, pattern) => match parent {
            Tag::Compound(c) => match position_key(c, name) {
                Some(p) if compare(pattern, &c[p].1) && !java_eq(value, &c[p].1) => {
                    c[p].1 = value.clone();
                    1
                }
                _ => 0,
            },
            _ => 0,
        },
        Node::Index(index) => {
            if !is_collection(parent) {
                return 0;
            }
            let size = coll_len(parent) as i64;
            let actual = if *index < 0 { size + *index as i64 } else { *index as i64 };
            if actual < 0 || actual >= size {
                return 0;
            }
            let idx = actual as usize;
            match coll_view(parent, idx) {
                Some(prev) if !java_eq(value, &prev) => {
                    if coll_set(parent, idx, value) {
                        1
                    } else {
                        0
                    }
                }
                _ => 0,
            }
        }
        Node::AllElements => {
            if !is_collection(parent) {
                return 0;
            }
            let size = coll_len(parent);
            if size == 0 {
                // Vanilla ignores the add result here and reports a change.
                let _ = coll_add(parent, 0, value);
                return 1;
            }
            let view: &Tag = parent;
            let same = (0..size).filter(|&i| coll_view(view, i).is_some_and(|e| java_eq(value, &e))).count();
            let changed = size - same;
            if changed == 0 {
                return 0;
            }
            coll_clear(parent);
            if !matches!(coll_add(parent, 0, value), Ok(true)) {
                return 0;
            }
            for i in 1..size {
                let _ = coll_add(parent, i as i64, value);
            }
            changed as i32
        }
        Node::MatchElement(pattern) => match parent {
            Tag::List(l) => {
                if l.is_empty() {
                    l.push(value.clone());
                    return 1;
                }
                let mut changed = 0;
                for slot in l.iter_mut() {
                    if compare(pattern, slot) && !java_eq(value, slot) {
                        *slot = value.clone();
                        changed += 1;
                    }
                }
                changed
            }
            _ => 0,
        },
        Node::MatchRoot(_) => 0,
    }
}

/// `removeTag` of each node type, applied to the parent.
fn remove_tag(node: &Node, parent: &mut Tag) -> i32 {
    match node {
        Node::Key(name) => match parent {
            Tag::Compound(c) => match position_key(c, name) {
                Some(p) => {
                    c.remove(p);
                    1
                }
                None => 0,
            },
            _ => 0,
        },
        Node::MatchKey(name, pattern) => match parent {
            Tag::Compound(c) => match position_key(c, name) {
                Some(p) if compare(pattern, &c[p].1) => {
                    c.remove(p);
                    1
                }
                _ => 0,
            },
            _ => 0,
        },
        Node::Index(index) => {
            if !is_collection(parent) {
                return 0;
            }
            let size = coll_len(parent) as i64;
            let actual = if *index < 0 { size + *index as i64 } else { *index as i64 };
            if actual >= 0 && actual < size {
                coll_remove(parent, actual as usize);
                1
            } else {
                0
            }
        }
        Node::AllElements => {
            let size = if is_collection(parent) { coll_len(parent) } else { 0 };
            if size > 0 {
                coll_clear(parent);
                size as i32
            } else {
                0
            }
        }
        Node::MatchElement(pattern) => {
            let mut changed = 0;
            if let Tag::List(l) = parent {
                for i in (0..l.len()).rev() {
                    if compare(pattern, &l[i]) {
                        l.remove(i);
                        changed += 1;
                    }
                }
            }
            changed
        }
        Node::MatchRoot(_) => 0,
    }
}

// ----- walking ----------------------------------------------------------------

/// Read-only match: a tag by reference, or an array element by value.
enum RHit<'a> {
    Ref(&'a Tag),
    Val(Tag),
}

impl RHit<'_> {
    fn into_tag(self) -> Tag {
        match self {
            RHit::Ref(t) => t.clone(),
            RHit::Val(v) => v,
        }
    }
}

fn resolve_ref(parent: &Tag, sel: Sel) -> Option<RHit<'_>> {
    match sel {
        Sel::Idx(i) => match parent {
            Tag::Compound(c) => c.get(i).map(|e| RHit::Ref(&e.1)),
            Tag::List(l) => l.get(i).map(RHit::Ref),
            _ => None,
        },
        Sel::Elem(i) => coll_view(parent, i).map(RHit::Val),
        Sel::SelfRef => Some(RHit::Ref(parent)),
    }
}

/// Mutable match: a location, or an element of a primitive array.
enum Hit {
    At(Vec<usize>),
    Elem(Vec<usize>, usize),
}

fn hit_of(sel: Sel, loc: &[usize]) -> Hit {
    match sel {
        Sel::Idx(i) => {
            let mut l = loc.to_vec();
            l.push(i);
            Hit::At(l)
        }
        Sel::Elem(i) => Hit::Elem(loc.to_vec(), i),
        Sel::SelfRef => Hit::At(loc.to_vec()),
    }
}

fn nav_mut<'a>(root: &'a mut Tag, loc: &[usize]) -> Option<&'a mut Tag> {
    let mut cur = root;
    for &i in loc {
        cur = match cur {
            Tag::Compound(c) => &mut c.get_mut(i)?.1,
            Tag::List(l) => l.get_mut(i)?,
            _ => return None,
        };
    }
    Some(cur)
}

impl NbtPath {
    /// `NbtPath.get`: every match; `nothing_found` when a level is empty.
    pub fn get(&self, root: &Tag) -> CmdResult<Vec<Tag>> {
        match self.read_walk(root) {
            Ok(hits) => Ok(hits.into_iter().map(RHit::into_tag).collect()),
            Err(node) => Err(self.not_found(node)),
        }
    }

    /// `NbtPath.countMatching`: number of matches, 0 when nothing matches.
    pub fn count_matching(&self, root: &Tag) -> i32 {
        self.read_walk(root).map_or(0, |hits| hits.len() as i32)
    }

    /// `NbtPath.set`: writes `value` to every match of the last node, creating
    /// missing intermediate tags. Returns the number of changed tags.
    pub fn set(&self, root: &mut Tag, value: &Tag) -> CmdResult<i32> {
        let n = self.nodes.len();
        if is_too_deep(value, n) {
            return Err(CmdError::key("arguments.nbtpath.too_deep"));
        }
        if n == 0 {
            return Ok(0);
        }
        let parents = self.walk_mut(root, true, n - 1, n - 1, &Tag::List(Vec::new()))?;
        let last = &self.nodes[n - 1];
        let mut total = 0;
        for hit in parents {
            if let Hit::At(loc) = hit {
                if let Some(parent) = nav_mut(root, &loc) {
                    total += set_tag(last, parent, value);
                }
            }
        }
        Ok(total)
    }

    /// `NbtPath.insert` (`getOrCreate` with a list leaf): inserts `values` at
    /// `index` of every matched list. A negative index counts from the end
    /// (`size + index + 1`).
    pub fn insert(&self, index: i32, root: &mut Tag, values: &[Tag]) -> CmdResult<i32> {
        let n = self.nodes.len();
        if values.iter().any(|v| is_too_deep(v, n)) {
            return Err(CmdError::key("arguments.nbtpath.too_deep"));
        }
        if n == 0 {
            return Ok(0);
        }
        let targets = self.walk_mut(root, true, n, n - 1, &Tag::List(Vec::new()))?;
        let mut modified_count = 0;
        for hit in targets {
            let loc = match hit {
                Hit::At(loc) => loc,
                Hit::Elem(loc, idx) => {
                    let desc = nav_mut(root, &loc).and_then(|arr| coll_view(arr, idx)).map(|v| snbt::to_snbt(&v));
                    return Err(expected_list(desc.unwrap_or_default()));
                }
            };
            let target = match nav_mut(root, &loc) {
                Some(t) => t,
                None => continue,
            };
            if !is_collection(target) {
                return Err(expected_list(snbt::to_snbt(target)));
            }
            let size = coll_len(target) as i64;
            let mut actual = if index < 0 { size + index as i64 + 1 } else { index as i64 };
            let mut modified = false;
            for v in values {
                match coll_add(target, actual, v) {
                    Ok(true) => {
                        actual += 1;
                        modified = true;
                    }
                    Ok(false) => {}
                    Err(()) => return Err(CmdError::tr("commands.data.modify.invalid_index", vec![lit(actual)])),
                }
            }
            if modified {
                modified_count += 1;
            }
        }
        Ok(modified_count)
    }

    /// `NbtPath.remove`: removes every match of the last node. Returns the
    /// number of removed tags.
    pub fn remove(&self, root: &mut Tag) -> i32 {
        let n = self.nodes.len();
        if n == 0 {
            return 0;
        }
        let parents = match self.walk_mut(root, false, n - 1, 0, &Tag::List(Vec::new())) {
            Ok(p) => p,
            Err(_) => return 0,
        };
        let last = &self.nodes[n - 1];
        let mut total = 0;
        for hit in parents {
            if let Hit::At(loc) = hit {
                if let Some(parent) = nav_mut(root, &loc) {
                    total += remove_tag(last, parent);
                }
            }
        }
        total
    }

    /// Calls `f` on every match, creating missing tags when `create` is set
    /// (`getOrCreate`; a missing last node becomes an empty list). Without
    /// `create` this is `get`: `nothing_found` when nothing matches.
    /// Returns the sum of `f`.
    pub fn modify_each(&self, root: &mut Tag, create: bool, f: &mut dyn FnMut(&mut Tag) -> i32) -> CmdResult<i32> {
        self.modify_each_with(root, create, &Tag::List(Vec::new()), f)
    }

    /// `modify_each` with the tag created for a missing last node given by the
    /// caller (`getOrCreate(tag, supplier)`).
    pub fn modify_each_with(
        &self,
        root: &mut Tag,
        create: bool,
        leaf: &Tag,
        f: &mut dyn FnMut(&mut Tag) -> i32,
    ) -> CmdResult<i32> {
        let n = self.nodes.len();
        if n == 0 {
            return Ok(f(root));
        }
        let hits = if create {
            self.walk_mut(root, true, n, n - 1, leaf)?
        } else {
            self.walk_mut(root, false, n, n, leaf)?
        };
        let mut total = 0;
        for hit in hits {
            match hit {
                Hit::At(loc) => {
                    if let Some(t) = nav_mut(root, &loc) {
                        total += f(t);
                    }
                }
                Hit::Elem(loc, idx) => {
                    if let Some(arr) = nav_mut(root, &loc) {
                        if let Some(mut v) = coll_view(arr, idx) {
                            total += f(&mut v);
                        }
                    }
                }
            }
        }
        Ok(total)
    }

    /// Read-only walk over all nodes. `Err(node)` when node `node` selects
    /// nothing.
    fn read_walk<'a>(&self, root: &'a Tag) -> Result<Vec<RHit<'a>>, usize> {
        let mut hits = vec![RHit::Ref(root)];
        for (i, node) in self.nodes.iter().enumerate() {
            let mut next = Vec::new();
            for hit in &hits {
                if let RHit::Ref(parent) = *hit {
                    for sel in sels(node, parent) {
                        if let Some(h) = resolve_ref(parent, sel) {
                            next.push(h);
                        }
                    }
                }
            }
            if next.is_empty() {
                return Err(i);
            }
            hits = next;
        }
        Ok(hits)
    }

    /// Mutable walk over the first `count` nodes, creating missing tags when
    /// `create` is set. Levels before `throw_upto` fail with `nothing_found`
    /// when they select nothing (Java's `getOrCreateParents` check).
    fn walk_mut(
        &self,
        root: &mut Tag,
        create: bool,
        count: usize,
        throw_upto: usize,
        leaf: &Tag,
    ) -> CmdResult<Vec<Hit>> {
        let n = self.nodes.len();
        let mut hits = vec![Hit::At(Vec::new())];
        for i in 0..count {
            let node = &self.nodes[i];
            let fill = if i + 1 < n { self.nodes[i + 1].preferred_parent() } else { leaf.clone() };
            let mut next = Vec::new();
            for hit in hits {
                let loc = match hit {
                    Hit::At(loc) => loc,
                    // A primitive is never a container: the walk stops there.
                    Hit::Elem(..) => continue,
                };
                let parent = match nav_mut(root, &loc) {
                    Some(p) => p,
                    None => continue,
                };
                let made = if create { ensure(node, parent, &fill) } else { None };
                let selected = match made {
                    Some(sel) => vec![sel],
                    None => sels(node, parent),
                };
                for sel in selected {
                    next.push(hit_of(sel, &loc));
                }
            }
            if next.is_empty() && i < throw_upto {
                return Err(self.not_found(i));
            }
            hits = next;
        }
        Ok(hits)
    }

    /// Node `node` may be an `AllElements` singleton: Java keys
    /// `nodeToOriginalPosition` by identity, so every `[]` reports the end
    /// of the last `[]` in the path.
    fn not_found(&self, node: usize) -> CmdError {
        let idx = if matches!(self.nodes[node], Node::AllElements) {
            self.nodes.iter().rposition(|n| matches!(n, Node::AllElements)).unwrap_or(node)
        } else {
            node
        };
        let end = self.positions[idx];
        CmdError::tr("arguments.nbtpath.nothing_found", vec![lit(&self.original[..end])])
    }
}

fn expected_list(desc: String) -> CmdError {
    CmdError::tr("commands.data.modify.expected_list", vec![lit(desc)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(s: &str) -> NbtPath {
        parse(&mut Reader::new(s)).unwrap_or_else(|e| panic!("{s:?}: {}", e.msg.plain_string()))
    }

    fn compound(v: Vec<(&str, Tag)>) -> Tag {
        Tag::Compound(v.into_iter().map(|(k, t)| (k.to_string(), t)).collect())
    }

    fn root() -> Tag {
        compound(vec![
            ("a", compound(vec![("b", Tag::List(vec![Tag::Int(5), Tag::Int(6)]))])),
            ("c", Tag::Int(1)),
        ])
    }

    #[test]
    fn parses_nodes() {
        let p = path("a.b[0]");
        assert_eq!(p.original, "a.b[0]");
        assert_eq!(p.nodes, vec![Node::Key("a".into()), Node::Key("b".into()), Node::Index(0)]);
        assert_eq!(path("a{x:1}").nodes, vec![Node::MatchKey("a".into(), compound(vec![("x", Tag::Int(1))]))]);
        assert_eq!(path("[{x:1}]").nodes, vec![Node::MatchElement(compound(vec![("x", Tag::Int(1))]))]);
        assert_eq!(path("{x:1}.a").nodes, vec![Node::MatchRoot(compound(vec![("x", Tag::Int(1))])), Node::Key("a".into())]);
        assert_eq!(path("\"a b\"").nodes, vec![Node::Key("a b".into())]);
        assert_eq!(path("[]").nodes, vec![Node::AllElements]);
        assert_eq!(path("[-1]").nodes, vec![Node::Index(-1)]);
        // A trailing dot is accepted, as in vanilla.
        assert_eq!(path("a.").nodes, vec![Node::Key("a".into())]);
        assert_eq!(path("a.").original, "a.");
        // The path stops at a space; the cursor stays there.
        let mut r = Reader::new("a.b c");
        let p = parse(&mut r).unwrap();
        assert_eq!(p.original, "a.b");
        assert_eq!(r.cursor, 3);
    }

    #[test]
    fn rejects_bad_nodes() {
        assert!(parse(&mut Reader::new("a..b")).is_err());
        assert!(parse(&mut Reader::new("a.{x:1}")).is_err());
        assert!(parse(&mut Reader::new("\"\"")).is_err());
        assert!(parse(&mut Reader::new("[x]")).is_err());
        assert!(parse(&mut Reader::new("[1.5]")).is_err());
    }

    #[test]
    fn gets_and_counts() {
        let r = root();
        assert_eq!(path("a.b[0]").get(&r).unwrap(), vec![Tag::Int(5)]);
        assert_eq!(path("a.b[-1]").get(&r).unwrap(), vec![Tag::Int(6)]);
        assert_eq!(path("a.b[]").get(&r).unwrap(), vec![Tag::Int(5), Tag::Int(6)]);
        assert_eq!(path("a.b[]").count_matching(&r), 2);
        assert_eq!(path("a.x").count_matching(&r), 0);
        assert_eq!(path("").get(&r).unwrap(), vec![r.clone()]);
        let err = path("a.x.y").get(&r).unwrap_err();
        assert_eq!(err.msg.plain_string(), "arguments.nbtpath.nothing_found[a.x]");
        let err = path("a.b[9]").get(&r).unwrap_err();
        assert_eq!(err.msg.plain_string(), "arguments.nbtpath.nothing_found[a.b[9]]");
    }

    #[test]
    fn matches_patterns() {
        let r = compound(vec![(
            "list",
            Tag::List(vec![
                compound(vec![("x", Tag::Int(1)), ("y", Tag::Int(2))]),
                compound(vec![("x", Tag::Int(3))]),
            ]),
        )]);
        assert_eq!(path("list[{x:3}]").get(&r).unwrap(), vec![compound(vec![("x", Tag::Int(3))])]);
        assert_eq!(path("list[{x:1}].y").get(&r).unwrap(), vec![Tag::Int(2)]);
        assert_eq!(path("list[{x:9}]").count_matching(&r), 0);
        assert_eq!(path("{list:[]}").count_matching(&r), 0);
        // A compound pattern never matches a list (class mismatch).
        assert_eq!(path("list{}").count_matching(&r), 0);
        assert_eq!(path("{list:[{x:1}]}").count_matching(&r), 1);
    }

    #[test]
    fn sets_creates_and_reports_changes() {
        let mut r = root();
        assert_eq!(path("a.b[1]").set(&mut r, &Tag::Int(9)).unwrap(), 1);
        assert_eq!(path("a.b[1]").get(&r).unwrap(), vec![Tag::Int(9)]);
        assert_eq!(path("a.b[1]").set(&mut r, &Tag::Int(9)).unwrap(), 0);
        assert_eq!(path("n.m.k").set(&mut r, &Tag::Int(4)).unwrap(), 1);
        assert_eq!(path("n.m.k").get(&r).unwrap(), vec![Tag::Int(4)]);
        assert_eq!(path("c").set(&mut r, &Tag::String("s".into())).unwrap(), 1);
        assert_eq!(path("zz[]").set(&mut r, &Tag::Int(1)).unwrap(), 1);
        assert_eq!(path("zz").get(&r).unwrap(), vec![Tag::List(vec![Tag::Int(1)])]);
    }

    #[test]
    fn inserts() {
        let mut r = root();
        assert_eq!(path("a.b").insert(0, &mut r, &[Tag::Int(7)]).unwrap(), 1);
        assert_eq!(path("a.b").get(&r).unwrap(), vec![Tag::List(vec![Tag::Int(7), Tag::Int(5), Tag::Int(6)])]);
        // Negative index counts from the end: size + index + 1.
        assert_eq!(path("a.b").insert(-1, &mut r, &[Tag::Int(8)]).unwrap(), 1);
        assert_eq!(
            path("a.b").get(&r).unwrap(),
            vec![Tag::List(vec![Tag::Int(7), Tag::Int(5), Tag::Int(6), Tag::Int(8)])]
        );
        // Out of bounds is an error with the computed index.
        assert!(path("a.b").insert(9, &mut r, &[Tag::Int(1)]).is_err());
        // A non-list target is an error.
        assert!(path("c").insert(0, &mut r, &[Tag::Int(1)]).is_err());
    }

    #[test]
    fn removes() {
        let mut r = root();
        assert_eq!(path("a.b[0]").remove(&mut r), 1);
        assert_eq!(path("a.b").get(&r).unwrap(), vec![Tag::List(vec![Tag::Int(6)])]);
        assert_eq!(path("a.b[]").remove(&mut r), 1);
        assert_eq!(path("c").remove(&mut r), 1);
        assert_eq!(path("c").remove(&mut r), 0);
        assert!(path("c").get(&r).is_err());
    }

    #[test]
    fn modifies_each_match() {
        let mut r = root();
        let n = path("a.b[]").modify_each(&mut r, false, &mut |t: &mut Tag| {
            *t = Tag::Int(42);
            1
        });
        assert_eq!(n.unwrap(), 2);
        assert_eq!(path("a.b[]").get(&r).unwrap(), vec![Tag::Int(42), Tag::Int(42)]);
        // Creation: missing `q` becomes a list, the leaf is created too.
        let n = path("q[]").modify_each(&mut r, true, &mut |t: &mut Tag| {
            *t = Tag::Int(3);
            1
        });
        assert_eq!(n.unwrap(), 1);
        assert_eq!(path("q").get(&r).unwrap(), vec![Tag::List(vec![Tag::Int(3)])]);
        assert!(path("nope").modify_each(&mut r, false, &mut |_| 1).is_err());
    }

    #[test]
    fn primitive_arrays() {
        let mut r = compound(vec![("arr", Tag::IntArray(vec![1]))]);
        assert_eq!(path("arr").insert(1, &mut r, &[Tag::Int(2), Tag::Long(3)]).unwrap(), 1);
        assert_eq!(path("arr").get(&r).unwrap(), vec![Tag::IntArray(vec![1, 2, 3])]);
        // Arrays refuse non-numeric tags: nothing is inserted, nothing counted.
        assert_eq!(path("arr").insert(0, &mut r, &[Tag::String("x".into())]).unwrap(), 0);
        assert_eq!(path("arr[0]").set(&mut r, &Tag::Long(5)).unwrap(), 1);
        assert_eq!(path("arr[]").get(&r).unwrap(), vec![Tag::Int(5), Tag::Int(2), Tag::Int(3)]);
        assert_eq!(path("arr[1]").remove(&mut r), 1);
        assert_eq!(path("arr").get(&r).unwrap(), vec![Tag::IntArray(vec![5, 3])]);
        let mut b = compound(vec![("b", Tag::ByteArray(vec![1, 2]))]);
        assert_eq!(path("b[]").get(&b).unwrap(), vec![Tag::Byte(1), Tag::Byte(2)]);
        // A byte element is a value: modify_each gets a temporary, the array is unchanged.
        assert_eq!(path("b[]").modify_each(&mut b, false, &mut |t: &mut Tag| {
            *t = Tag::Byte(9);
            1
        }).unwrap(), 2);
        assert_eq!(path("b").get(&b).unwrap(), vec![Tag::ByteArray(vec![1, 2])]);
    }

    #[test]
    fn too_deep_values_are_rejected() {
        let mut deep = Tag::Int(0);
        for _ in 0..600 {
            deep = Tag::List(vec![deep]);
        }
        let mut r = root();
        assert!(path("a").set(&mut r, &deep).is_err());
    }
}
