//! `block_state` and `block_predicate` (`BlockStateParser`,
//! `BlockStateArgument`, `BlockPredicateArgument`).
//!
//! Tags (`#tag`) are only accepted by predicates. For tags Java never
//! replaces the default identifier, so the property errors of a tag name
//! the block as `minecraft:`; that quirk is kept.

use crate::error::{lit, CmdError, CmdResult};
use crate::reader::Reader;
use rine_data::blocks::{BlockInfo, Property};
use rine_protocol::nbt::Tag;
use rine_protocol::text::Text;

/// `BlockInput`: the state (defaults for unset properties), the
/// properties written explicitly, and block entity NBT.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockInput {
    pub state: u16,
    pub props: Vec<(String, String)>,
    pub nbt: Option<Tag>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BlockPredicate {
    /// Exact block with only the written properties checked.
    Block { block: String, props: Vec<(String, String)>, nbt: Option<Tag> },
    /// `#tag[props]{nbt}`: string-valued property checks.
    Tag { tag: String, props: Vec<(String, String)>, nbt: Option<Tag> },
}

/// Identifier characters accepted by `Identifier.read`.
fn is_identifier_char(c: char) -> bool {
    c.is_ascii_digit() || c.is_ascii_lowercase() || matches!(c, '_' | ':' | '/' | '.' | '-')
}

fn valid_namespace(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_digit() || b.is_ascii_lowercase() || matches!(b, b'_' | b'-' | b'.'))
}

fn valid_path(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_digit() || b.is_ascii_lowercase() || matches!(b, b'_' | b'-' | b'.' | b'/'))
}

/// `Identifier.parse`: the full `namespace:path` form, or None if invalid.
pub(crate) fn parse_id(raw: &str) -> Option<String> {
    match raw.find(':') {
        Some(0) => {
            let path = &raw[1..];
            valid_path(path).then(|| format!("minecraft:{path}"))
        }
        Some(i) => {
            let (ns, path) = (&raw[..i], &raw[i + 1..]);
            (valid_namespace(ns) && valid_path(path)).then(|| raw.to_string())
        }
        None => valid_path(raw).then(|| format!("minecraft:{raw}")),
    }
}

fn invalid_id(r: &Reader) -> CmdError {
    CmdError::at(Text::key("argument.id.invalid"), r)
}

/// `Identifier.read`, or `Identifier.readNonEmpty` when `non_empty`.
/// Reads identifier characters greedily. Without `non_empty`, an empty
/// read is accepted and yields `minecraft:` (as in Java).
pub(crate) fn read_identifier(r: &mut Reader, non_empty: bool) -> CmdResult<String> {
    let start = r.cursor;
    let rest = r.remaining();
    let len = rest.find(|c: char| !is_identifier_char(c)).unwrap_or(rest.len());
    let raw = rest[..len].to_string();
    r.cursor = start + len;
    if non_empty && raw.is_empty() {
        r.cursor = start;
        return Err(invalid_id(r));
    }
    match parse_id(&raw) {
        Some(id) => Ok(id),
        None => {
            r.cursor = start;
            Err(invalid_id(r))
        }
    }
}

/// Parsed result of `BlockStateParser`, before it is split for the
/// `block_state` and `block_predicate` arguments.
enum Parsed {
    Block { id: String, state: u16, props: Vec<(String, String)> },
    Tag { tag: String, props: Vec<(String, String)> },
}

/// `BlockStateParser.parseForBlock` / `parseForTesting`: on error the
/// cursor goes back to the start, the error keeps its own cursor.
fn parse_inner(r: &mut Reader, for_testing: bool) -> CmdResult<(Parsed, Option<Tag>)> {
    let start = r.cursor;
    let res = parse_body(r, for_testing);
    if res.is_err() {
        r.cursor = start;
    }
    res
}

fn parse_body(r: &mut Reader, for_testing: bool) -> CmdResult<(Parsed, Option<Tag>)> {
    let parsed = if r.can_read() && r.peek() == '#' {
        let tag = read_tag(r, for_testing)?;
        let mut props = Vec::new();
        if r.can_read() && r.peek() == '[' {
            read_vague_properties(r, &mut props)?;
        }
        Parsed::Tag { tag, props }
    } else {
        let (id, info, mut state) = read_block(r)?;
        let mut props = Vec::new();
        if r.can_read() && r.peek() == '[' {
            read_properties(r, &id, info, &mut state, &mut props)?;
        }
        Parsed::Block { id, state, props }
    };
    let nbt = if r.can_read() && r.peek() == '{' { Some(super::snbt::parse_compound(r)?) } else { None };
    Ok((parsed, nbt))
}

fn read_block(r: &mut Reader) -> CmdResult<(String, &'static BlockInfo, u16)> {
    let start = r.cursor;
    let id = read_identifier(r, false)?;
    match rine_data::get().blocks.block(&id) {
        Some(info) => Ok((id, info, info.default_state as u16)),
        None => {
            r.cursor = start;
            Err(CmdError::at(Text::tr("argument.block.id.invalid", vec![lit(&id)]), r))
        }
    }
}

fn read_tag(r: &mut Reader, for_testing: bool) -> CmdResult<String> {
    if !for_testing {
        return Err(CmdError::at(Text::key("argument.block.tag.disallowed"), r));
    }
    let start = r.cursor;
    r.expect('#')?;
    let id = read_identifier(r, false)?;
    if rine_data::get().registries.tag("minecraft:block", &id).is_none() {
        r.cursor = start;
        return Err(CmdError::at(Text::tr("arguments.block.tag.unknown", vec![lit(&id)]), r));
    }
    Ok(id)
}

/// Java `Property.getValue(String)`: exact value name, or an integer
/// spelled differently (`07`, `+7`) whose canonical name is a value.
fn canonical_value(p: &Property, raw: &str) -> Option<String> {
    if p.values.iter().any(|v| v == raw) {
        return Some(raw.to_string());
    }
    let n: i64 = raw.parse().ok()?;
    let s = n.to_string();
    p.values.iter().any(|v| *v == s).then_some(s)
}

fn unclosed(r: &Reader) -> CmdError {
    CmdError::at(Text::key("argument.block.property.unclosed"), r)
}

fn invalid_value(r: &mut Reader, start: usize, id: &str, name: &str, raw: &str) -> CmdError {
    r.cursor = start;
    CmdError::at(Text::tr("argument.block.property.invalid", vec![lit(id), lit(raw), lit(name)]), r)
}

/// `readProperties`: `[name=value,...]` with the block's own properties.
fn read_properties(
    r: &mut Reader,
    id: &str,
    info: &BlockInfo,
    state: &mut u16,
    props: &mut Vec<(String, String)>,
) -> CmdResult<()> {
    let blocks = &rine_data::get().blocks;
    r.skip();
    r.skip_whitespace();
    while r.can_read() && r.peek() != ']' {
        r.skip_whitespace();
        let key_start = r.cursor;
        let key = r.read_string()?;
        let Some(prop) = info.props.iter().find(|p| p.name == key) else {
            r.cursor = key_start;
            return Err(CmdError::at(Text::tr("argument.block.property.unknown", vec![lit(id), lit(&key)]), r));
        };
        if props.iter().any(|(k, _)| *k == key) {
            r.cursor = key_start;
            return Err(CmdError::at(Text::tr("argument.block.property.duplicate", vec![lit(&key), lit(id)]), r));
        }
        r.skip_whitespace();
        if !r.can_read() || r.peek() != '=' {
            return Err(CmdError::at(Text::tr("argument.block.property.novalue", vec![lit(&key), lit(id)]), r));
        }
        r.skip();
        r.skip_whitespace();
        let value_start = r.cursor;
        let raw = r.read_string()?;
        let Some(value) = canonical_value(prop, &raw) else {
            return Err(invalid_value(r, value_start, id, &prop.name, &raw));
        };
        let Some(next) = blocks.with_prop(*state, &prop.name, &value) else {
            return Err(invalid_value(r, value_start, id, &prop.name, &raw));
        };
        *state = next;
        props.push((prop.name.clone(), value));
        r.skip_whitespace();
        if r.can_read() {
            if r.peek() != ',' {
                if r.peek() != ']' {
                    return Err(unclosed(r));
                }
                break;
            }
            r.skip();
        }
    }
    if r.can_read() {
        r.skip();
        Ok(())
    } else {
        Err(unclosed(r))
    }
}

/// `readVagueProperties`: `#tag[name=value,...]`, values are not checked.
fn read_vague_properties(r: &mut Reader, props: &mut Vec<(String, String)>) -> CmdResult<()> {
    // Java keeps the default identifier here (`minecraft:`).
    const ID: &str = "minecraft:";
    r.skip();
    let mut value_start: Option<usize> = None;
    r.skip_whitespace();
    while r.can_read() && r.peek() != ']' {
        r.skip_whitespace();
        let key_start = r.cursor;
        let key = r.read_string()?;
        if props.iter().any(|(k, _)| *k == key) {
            r.cursor = key_start;
            return Err(CmdError::at(Text::tr("argument.block.property.duplicate", vec![lit(&key), lit(ID)]), r));
        }
        r.skip_whitespace();
        if !r.can_read() || r.peek() != '=' {
            r.cursor = key_start;
            return Err(CmdError::at(Text::tr("argument.block.property.novalue", vec![lit(&key), lit(ID)]), r));
        }
        r.skip();
        r.skip_whitespace();
        value_start = Some(r.cursor);
        let value = r.read_string()?;
        props.push((key, value));
        r.skip_whitespace();
        if r.can_read() {
            value_start = None;
            if r.peek() != ',' {
                if r.peek() != ']' {
                    return Err(unclosed(r));
                }
                break;
            }
            r.skip();
        }
    }
    if r.can_read() {
        r.skip();
        Ok(())
    } else {
        if let Some(vs) = value_start {
            r.cursor = vs;
        }
        Err(unclosed(r))
    }
}

/// `block_state`: a concrete block, no tags.
pub fn parse_block_state(r: &mut Reader) -> CmdResult<BlockInput> {
    match parse_inner(r, false)? {
        (Parsed::Block { state, props, .. }, nbt) => Ok(BlockInput { state, props, nbt }),
        // Tags fail earlier in `read_tag` for this argument.
        (Parsed::Tag { .. }, _) => Err(CmdError::at(Text::key("argument.block.tag.disallowed"), r)),
    }
}

/// `block_predicate`: a concrete block or a `#tag`.
pub fn parse_block_predicate(r: &mut Reader) -> CmdResult<BlockPredicate> {
    Ok(match parse_inner(r, true)? {
        (Parsed::Block { id, props, .. }, nbt) => BlockPredicate::Block { block: id, props, nbt },
        (Parsed::Tag { tag, props }, nbt) => BlockPredicate::Tag { tag, props, nbt },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rine_protocol::text::Content;

    /// Translation key and its arguments (literal args are plain texts).
    fn parts(e: &CmdError) -> (String, Vec<String>) {
        match &e.msg.content {
            Content::Translate { key, with, .. } => (key.to_string(), with.iter().map(|t| t.text.clone()).collect()),
            other => panic!("not a translation: {other:?}"),
        }
    }

    fn pred(s: &str) -> CmdResult<BlockPredicate> {
        rine_data::init();
        let mut r = Reader::new(s);
        parse_block_predicate(&mut r)
    }

    fn ok_block(s: &str) -> (BlockInput, usize) {
        rine_data::init();
        let mut r = Reader::new(s);
        let v = parse_block_state(&mut r).unwrap_or_else(|e| panic!("{s}: {:?}", parts(&e)));
        (v, r.cursor)
    }

    fn err_block(s: &str) -> (String, Vec<String>, usize) {
        rine_data::init();
        let mut r = Reader::new(s);
        let e = parse_block_state(&mut r).expect_err(s);
        let (k, a) = parts(&e);
        (k, a, e.cursor)
    }

    #[test]
    fn plain_and_namespaced_ids() {
        rine_data::init();
        let d = rine_data::get();
        let (v, cur) = ok_block("stone");
        assert_eq!(v.state, d.blocks.default_state("minecraft:stone"));
        assert!(v.props.is_empty() && v.nbt.is_none());
        assert_eq!(cur, 5);
        assert_eq!(ok_block("minecraft:stone").0, v);
        // trailing text is left for the caller
        assert_eq!(ok_block("stone extra").1, 5);
    }

    #[test]
    fn properties_set_state() {
        rine_data::init();
        let want = rine_data::get().blocks.parse_state("minecraft:oak_log[axis=x]").unwrap();
        let (v, cur) = ok_block("oak_log[axis=x]");
        assert_eq!(v.state, want);
        assert_eq!(v.props, vec![("axis".to_string(), "x".to_string())]);
        assert_eq!(cur, 15);
        // whitespace and quotes are accepted around the value
        assert_eq!(ok_block("oak_log[ axis = \"x\" ]").0.state, want);
    }

    #[test]
    fn integer_property_spellings() {
        rine_data::init();
        let want = rine_data::get().blocks.parse_state("minecraft:wheat[age=7]").unwrap();
        let (v, _) = ok_block("wheat[age=07]");
        assert_eq!(v.state, want);
        assert_eq!(v.props, vec![("age".to_string(), "7".to_string())]);
    }

    #[test]
    fn property_errors() {
        // unknown property: [block, key] at the key
        assert_eq!(err_block("oak_log[foo=x]"), ("argument.block.property.unknown".into(), vec!["minecraft:oak_log".into(), "foo".into()], 8));
        // duplicate: [key, block] at the second key
        assert_eq!(err_block("oak_log[axis=x,axis=y]"), ("argument.block.property.duplicate".into(), vec!["axis".into(), "minecraft:oak_log".into()], 15));
        // invalid value: [block, value, property] at the value
        assert_eq!(
            err_block("oak_log[axis=w]"),
            ("argument.block.property.invalid".into(), vec!["minecraft:oak_log".into(), "w".into(), "axis".into()], 13)
        );
        // missing '=': [key, block] at the closing bracket
        assert_eq!(err_block("oak_log[axis]"), ("argument.block.property.novalue".into(), vec!["axis".into(), "minecraft:oak_log".into()], 12));
        // unterminated: cursor at end
        assert_eq!(err_block("oak_log[axis=x"), ("argument.block.property.unclosed".into(), vec![], 14));
        assert_eq!(err_block("stone["), ("argument.block.property.unclosed".into(), vec![], 6));
    }

    #[test]
    fn id_errors() {
        assert_eq!(err_block("foo"), ("argument.block.id.invalid".into(), vec!["minecraft:foo".into()], 0));
        // uppercase is not an identifier character: the read is empty
        assert_eq!(err_block("Stone"), ("argument.block.id.invalid".into(), vec!["minecraft:".into()], 0));
    }

    #[test]
    fn tags_only_for_predicates() {
        assert_eq!(err_block("#minecraft:acacia_logs"), ("argument.block.tag.disallowed".into(), vec![], 0));
        match pred("#minecraft:acacia_logs").unwrap() {
            BlockPredicate::Tag { tag, props, nbt } => {
                assert_eq!(tag, "minecraft:acacia_logs");
                assert!(props.is_empty() && nbt.is_none());
            }
            other => panic!("{other:?}"),
        }
        // vague properties are not checked against the blocks
        match pred("#minecraft:acacia_logs[axis=x,foo=bar]").unwrap() {
            BlockPredicate::Tag { props, .. } => assert_eq!(props.len(), 2),
            other => panic!("{other:?}"),
        }
        let mut r = Reader::new("#minecraft:nope");
        let e = parse_block_predicate(&mut r).unwrap_err();
        assert_eq!(parts(&e), ("arguments.block.tag.unknown".into(), vec!["minecraft:nope".into()]));
        assert_eq!(e.cursor, 0);
        // vague property errors name the block as `minecraft:`
        let mut r = Reader::new("#minecraft:acacia_logs[axis]");
        let e = parse_block_predicate(&mut r).unwrap_err();
        assert_eq!(parts(&e), ("argument.block.property.novalue".into(), vec!["axis".into(), "minecraft:".into()]));
        let mut r = Reader::new("#minecraft:acacia_logs[axis=x");
        let e = parse_block_predicate(&mut r).unwrap_err();
        assert_eq!(parts(&e).0, "argument.block.property.unclosed");
        // Java moves the cursor back to the start of the last value
        assert_eq!(e.cursor, 28);
    }

    #[test]
    fn predicate_block_form() {
        match pred("minecraft:oak_log[axis=x]").unwrap() {
            BlockPredicate::Block { block, props, nbt } => {
                assert_eq!(block, "minecraft:oak_log");
                assert_eq!(props, vec![("axis".to_string(), "x".to_string())]);
                assert!(nbt.is_none());
            }
            other => panic!("{other:?}"),
        }
    }
}
