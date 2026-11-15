//! `item_stack` and `item_predicate` (`ItemParser`, `ItemPredicateArgument`,
//! `ComponentPredicateParser`).
//!
//! The predicate grammar is a packrat grammar in Java. It is ported as a
//! hand-written parser with the same alternatives, cuts and error stores
//! in the same order, so the reported error is the one vanilla reports
//! (the error stored at the furthest cursor, first one wins).
//!
//! Component values and predicate values are parsed as SNBT only; their
//! contents are not checked against the codecs.

use super::block::read_identifier;
use crate::error::{lit, CmdError, CmdResult};
use crate::reader::Reader;
use rine_protocol::nbt::Tag;
use rine_protocol::text::Text;

/// `count` is a pseudo component and predicate (not in the registries).
const COUNT: &str = "minecraft:count";

/// `ItemInput`: item id and component patch (`None` = removed with `!`).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemInput {
    pub item: String,
    pub components: Vec<(String, Option<Tag>)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemTest {
    /// `component_id` presence or `component_id=value` exact match.
    Component { id: String, value: Option<Tag>, negated: bool },
    /// `predicate_id~value` sub-predicate.
    Predicate { id: String, value: Tag, negated: bool },
    /// `a|b|c` alternatives.
    Any(Vec<ItemTest>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ItemPredicate {
    /// `*`, `#tag`, or an item id.
    pub item: ItemMatch,
    pub tests: Vec<ItemTest>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemMatch {
    Any,
    Tag(String),
    Item(String),
}

fn has_entry(registry: &str, id: &str) -> bool {
    rine_data::get().registries.id(registry, id).is_some()
}

fn has_tag(registry: &str, id: &str) -> bool {
    rine_data::get().registries.tag(registry, id).is_some()
}

fn repeated(id: &str) -> CmdError {
    // Java creates this one without reader context.
    CmdError::new(Text::tr("arguments.item.component.repeated", vec![lit(id)]))
}

pub fn parse_item_stack(r: &mut Reader) -> CmdResult<ItemInput> {
    let start = r.cursor;
    let res = item_stack(r);
    if res.is_err() {
        r.cursor = start;
    }
    res
}

fn item_stack(r: &mut Reader) -> CmdResult<ItemInput> {
    let start = r.cursor;
    let item = read_identifier(r, false)?;
    if !has_entry("minecraft:item", &item) {
        r.cursor = start;
        return Err(CmdError::at(Text::tr("argument.item.id.invalid", vec![lit(&item)]), r));
    }
    let mut components = Vec::new();
    if r.can_read() && r.peek() == '[' {
        read_components(r, &mut components)?;
    }
    Ok(ItemInput { item, components })
}

/// `ItemParser.State.readComponents`.
fn read_components(r: &mut Reader, out: &mut Vec<(String, Option<Tag>)>) -> CmdResult<()> {
    r.expect('[')?;
    let mut seen: Vec<String> = Vec::new();
    while r.can_read() && r.peek() != ']' {
        r.skip_whitespace();
        if r.can_read() && r.peek() == '!' {
            r.skip();
            let id = read_component_type(r)?;
            if seen.contains(&id) {
                return Err(repeated(&id));
            }
            seen.push(id.clone());
            out.push((id, None));
            r.skip_whitespace();
        } else {
            let id = read_component_type(r)?;
            if seen.contains(&id) {
                return Err(repeated(&id));
            }
            seen.push(id.clone());
            r.skip_whitespace();
            r.expect('=')?;
            r.skip_whitespace();
            let value = super::snbt::parse_tag(r)?;
            out.push((id, Some(value)));
            r.skip_whitespace();
        }
        if !r.can_read() || r.peek() != ',' {
            break;
        }
        r.skip();
        r.skip_whitespace();
        if !r.can_read() {
            return Err(CmdError::at(Text::key("arguments.item.component.expected"), r));
        }
    }
    r.expect(']')
}

/// `ItemParser.State.readComponentType`.
/// Deviation: Java also rejects transient component types. The data tables
/// do not record that flag, so every registered type is accepted.
fn read_component_type(r: &mut Reader) -> CmdResult<String> {
    if !r.can_read() {
        return Err(CmdError::at(Text::key("arguments.item.component.expected"), r));
    }
    let start = r.cursor;
    let id = read_identifier(r, false)?;
    if !has_entry("minecraft:data_component_type", &id) {
        r.cursor = start;
        return Err(CmdError::at(Text::tr("arguments.item.component.unknown", vec![lit(&id)]), r));
    }
    Ok(id)
}

// ---------------------------------------------------------------------------
// item_predicate: packrat grammar with the LongestOnly error collector.
// ---------------------------------------------------------------------------

/// `DelayedException` reasons: created at the furthest cursor when the
/// grammar fails, so they only carry the kind of failure.
#[derive(Clone, Copy)]
enum Delayed {
    IdInvalid,
    Literal(char),
}

enum Reason {
    Delayed(Delayed),
    Exact(CmdError),
}

struct Grammar {
    r: Reader,
    /// Furthest cursor with an error, -1 if none.
    last: i64,
    reasons: Vec<Reason>,
}

impl Grammar {
    fn store(&mut self, cursor: usize, reason: Reason) {
        let c = cursor as i64;
        if c > self.last {
            self.last = c;
            self.reasons.clear();
        }
        if c == self.last {
            self.reasons.push(reason);
        }
    }

    fn mark(&self) -> usize {
        self.r.cursor
    }

    fn restore(&mut self, m: usize) {
        self.r.cursor = m;
    }

    /// `TerminalCharacters`: one character after whitespace.
    fn lit(&mut self, c: char) -> bool {
        self.r.skip_whitespace();
        let cursor = self.r.cursor;
        if self.r.can_read() && self.r.peek() == c {
            self.r.skip();
            return true;
        }
        self.store(cursor, Reason::Delayed(Delayed::Literal(c)));
        false
    }

    /// `ResourceLookupRule`: an identifier that passes `check`.
    fn lookup(&mut self, check: fn(&Reader, &str) -> CmdResult<()>) -> Option<String> {
        self.r.skip_whitespace();
        let mark = self.r.cursor;
        self.r.skip_whitespace();
        match read_identifier(&mut self.r, true) {
            Ok(id) => match check(&self.r, &id) {
                Ok(()) => Some(id),
                Err(e) => {
                    self.store(mark, Reason::Exact(e));
                    None
                }
            },
            Err(_) => {
                self.store(mark, Reason::Delayed(Delayed::IdInvalid));
                None
            }
        }
    }

    /// `TagParseRule`: one SNBT value, failures are stored at its start.
    fn tag(&mut self) -> Option<Tag> {
        self.r.skip_whitespace();
        let mark = self.r.cursor;
        match super::snbt::parse_tag(&mut self.r) {
            Ok(t) => Some(t),
            Err(e) => {
                self.store(mark, Reason::Exact(e));
                None
            }
        }
    }

    /// `top`: `type [ conditions ]` (cut after `[`), or a bare `type`.
    fn top(&mut self) -> Option<ItemPredicate> {
        let m = self.mark();
        if let Some(ty) = self.type_rule() {
            if self.lit('[') {
                // Cut: a failure here does not fall back to the bare type.
                let conds = self.conditions().unwrap_or_default();
                if self.lit(']') {
                    return Some(ItemPredicate { item: ty, tests: conds });
                }
                self.restore(m);
                return None;
            }
        }
        self.restore(m);
        if let Some(ty) = self.type_rule() {
            return Some(ItemPredicate { item: ty, tests: Vec::new() });
        }
        self.restore(m);
        None
    }

    /// `type`: element id, `#tag` (cut), or `*`.
    fn type_rule(&mut self) -> Option<ItemMatch> {
        let m = self.mark();
        if let Some(id) = self.lookup(check_item) {
            return Some(ItemMatch::Item(id));
        }
        self.restore(m);
        if self.lit('#') {
            // Cut: a missing tag does not try `*`.
            if let Some(id) = self.lookup(check_item_tag) {
                return Some(ItemMatch::Tag(id));
            }
            self.restore(m);
            return None;
        }
        self.restore(m);
        if self.lit('*') {
            return Some(ItemMatch::Any);
        }
        self.restore(m);
        None
    }

    /// `conditions`: `alternatives (, conditions)?`.
    fn conditions(&mut self) -> Option<Vec<ItemTest>> {
        let m = self.mark();
        let alts = match self.alternatives() {
            Some(a) => a,
            None => {
                self.restore(m);
                return None;
            }
        };
        let mut out = vec![any_of(alts)];
        let m2 = self.mark();
        if self.lit(',') {
            match self.conditions() {
                Some(rest) => out.extend(rest),
                None => self.restore(m2),
            }
        } else {
            self.restore(m2);
        }
        Some(out)
    }

    /// `alternatives`: `term (| alternatives)?`.
    fn alternatives(&mut self) -> Option<Vec<ItemTest>> {
        let m = self.mark();
        let first = match self.term() {
            Some(t) => t,
            None => {
                self.restore(m);
                return None;
            }
        };
        let mut out = vec![first];
        let m2 = self.mark();
        if self.lit('|') {
            match self.alternatives() {
                Some(rest) => out.extend(rest),
                None => self.restore(m2),
            }
        } else {
            self.restore(m2);
        }
        Some(out)
    }

    /// `term`: `test` or `! test`.
    fn term(&mut self) -> Option<ItemTest> {
        let m = self.mark();
        if let Some(t) = self.test_rule() {
            return Some(t);
        }
        self.restore(m);
        if self.lit('!') {
            if let Some(t) = self.test_rule() {
                return Some(negate(t));
            }
        }
        self.restore(m);
        None
    }

    /// `test`: `component=value` (cut), `predicate~value` (cut), or `component`.
    fn test_rule(&mut self) -> Option<ItemTest> {
        let m = self.mark();
        if let Some(id) = self.lookup(check_component) {
            if self.lit('=') {
                // Cut: a bad value fails the whole test.
                return match self.tag() {
                    Some(v) => Some(ItemTest::Component { id, value: Some(v), negated: false }),
                    None => {
                        self.restore(m);
                        None
                    }
                };
            }
        }
        self.restore(m);
        if let Some(id) = self.lookup(check_predicate) {
            if self.lit('~') {
                return match self.tag() {
                    Some(v) => Some(ItemTest::Predicate { id, value: v, negated: false }),
                    None => {
                        self.restore(m);
                        None
                    }
                };
            }
        }
        self.restore(m);
        if let Some(id) = self.lookup(check_component) {
            return Some(ItemTest::Component { id, value: None, negated: false });
        }
        self.restore(m);
        None
    }

    /// Error vanilla throws: the first stored reason at the furthest cursor.
    fn error(&self) -> CmdError {
        match self.reasons.first() {
            Some(Reason::Exact(e)) => e.clone(),
            Some(Reason::Delayed(d)) => {
                let msg = match d {
                    Delayed::IdInvalid => Text::key("argument.id.invalid"),
                    Delayed::Literal(c) => Text::tr("argument.literal.incorrect", vec![lit(c)]),
                };
                CmdError::at_cursor(msg, &self.r, self.last.max(0) as usize)
            }
            None => CmdError::at(Text::key("command.failed"), &self.r),
        }
    }
}

fn any_of(mut alts: Vec<ItemTest>) -> ItemTest {
    if alts.len() == 1 {
        alts.pop().expect("one alternative")
    } else {
        ItemTest::Any(alts)
    }
}

fn negate(t: ItemTest) -> ItemTest {
    match t {
        ItemTest::Component { id, value, negated } => ItemTest::Component { id, value, negated: !negated },
        ItemTest::Predicate { id, value, negated } => ItemTest::Predicate { id, value, negated: !negated },
        other => other,
    }
}

fn check_item(r: &Reader, id: &str) -> CmdResult<()> {
    if has_entry("minecraft:item", id) {
        Ok(())
    } else {
        Err(CmdError::at(Text::tr("argument.item.id.invalid", vec![lit(id)]), r))
    }
}

fn check_item_tag(r: &Reader, id: &str) -> CmdResult<()> {
    if has_tag("minecraft:item", id) {
        Ok(())
    } else {
        Err(CmdError::at(Text::tr("arguments.item.tag.unknown", vec![lit(id)]), r))
    }
}

fn check_component(r: &Reader, id: &str) -> CmdResult<()> {
    if id == COUNT || has_entry("minecraft:data_component_type", id) {
        Ok(())
    } else {
        Err(CmdError::at(Text::tr("arguments.item.component.unknown", vec![lit(id)]), r))
    }
}

fn check_predicate(r: &Reader, id: &str) -> CmdResult<()> {
    if id == COUNT || has_entry("minecraft:data_component_predicate_type", id) || has_entry("minecraft:data_component_type", id) {
        Ok(())
    } else {
        Err(CmdError::at(Text::tr("arguments.item.predicate.unknown", vec![lit(id)]), r))
    }
}

/// `item_predicate`. On failure the reader is reset to the start.
pub fn parse_item_predicate(r: &mut Reader) -> CmdResult<ItemPredicate> {
    let start = r.cursor;
    let mut g = Grammar { r: r.clone(), last: -1, reasons: Vec::new() };
    match g.top() {
        Some(p) => {
            r.cursor = g.r.cursor;
            Ok(p)
        }
        None => {
            r.cursor = start;
            Err(g.error())
        }
    }
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

    fn stack(s: &str) -> CmdResult<(ItemInput, usize)> {
        rine_data::init();
        let mut r = Reader::new(s);
        parse_item_stack(&mut r).map(|v| (v, r.cursor))
    }

    fn pred(s: &str) -> CmdResult<(ItemPredicate, usize)> {
        rine_data::init();
        let mut r = Reader::new(s);
        parse_item_predicate(&mut r).map(|v| (v, r.cursor))
    }

    fn stack_err(s: &str) -> (String, Vec<String>, usize) {
        let e = stack(s).expect_err(s);
        let (k, a) = parts(&e);
        (k, a, e.cursor)
    }

    fn pred_err(s: &str) -> (String, Vec<String>, usize) {
        let e = pred(s).expect_err(s);
        let (k, a) = parts(&e);
        (k, a, e.cursor)
    }

    fn comp(id: &str, negated: bool) -> ItemTest {
        ItemTest::Component { id: id.to_string(), value: None, negated }
    }

    #[test]
    fn stack_plain_id() {
        let (v, cur) = stack("stone").unwrap();
        assert_eq!(v, ItemInput { item: "minecraft:stone".into(), components: vec![] });
        assert_eq!(cur, 5);
        assert_eq!(stack("minecraft:stick x").unwrap().1, 15);
    }

    #[test]
    fn stack_id_errors() {
        // Identifier.read on an empty read gives `minecraft:`, which is not an item
        assert_eq!(stack_err("Stone"), ("argument.item.id.invalid".into(), vec!["minecraft:".into()], 0));
        assert_eq!(stack_err("minecraft:nope"), ("argument.item.id.invalid".into(), vec!["minecraft:nope".into()], 0));
        // a second colon makes the identifier itself invalid
        assert_eq!(stack_err("a:b:c"), ("argument.id.invalid".into(), vec![], 0));
    }

    #[test]
    fn stack_component_removal() {
        let (v, cur) = stack("stone[!minecraft:damage]").unwrap();
        assert_eq!(v.components, vec![("minecraft:damage".to_string(), None)]);
        assert_eq!(cur, 24);
    }

    #[test]
    fn stack_component_errors() {
        assert_eq!(stack_err("stone["), ("parsing.expected".into(), vec!["]".into()], 6));
        assert_eq!(stack_err("stone[!"), ("arguments.item.component.expected".into(), vec![], 7));
        assert_eq!(stack_err("stone[!minecraft:nope]"), ("arguments.item.component.unknown".into(), vec!["minecraft:nope".into()], 7));
        assert_eq!(stack_err("stone[ ]"), ("arguments.item.component.unknown".into(), vec!["minecraft:".into()], 7));
        let e = stack("stone[!minecraft:damage,!minecraft:damage]").unwrap_err();
        assert_eq!(parts(&e), ("arguments.item.component.repeated".into(), vec!["minecraft:damage".into()]));
        assert!(e.input.is_none());
    }

    #[test]
    fn predicate_forms() {
        let (p, cur) = pred("*").unwrap();
        assert_eq!(p, ItemPredicate { item: ItemMatch::Any, tests: vec![] });
        assert_eq!(cur, 1);

        let (p, cur) = pred("stone").unwrap();
        assert_eq!(p, ItemPredicate { item: ItemMatch::Item("minecraft:stone".into()), tests: vec![] });
        assert_eq!(cur, 5);

        let (p, _) = pred("#minecraft:acacia_logs").unwrap();
        assert_eq!(p.item, ItemMatch::Tag("minecraft:acacia_logs".into()));
    }

    #[test]
    fn predicate_presence_tests() {
        let (p, cur) = pred("stone[minecraft:damage]").unwrap();
        assert_eq!(p.tests, vec![comp("minecraft:damage", false)]);
        assert_eq!(cur, 23);

        let (p, _) = pred("stone[!minecraft:damage]").unwrap();
        assert_eq!(p.tests, vec![comp("minecraft:damage", true)]);

        // pseudo component `count` is always known
        let (p, _) = pred("stone[count]").unwrap();
        assert_eq!(p.tests, vec![comp("minecraft:count", false)]);
    }

    #[test]
    fn predicate_alternatives_and_conditions() {
        let (p, _) = pred("stone[minecraft:damage|minecraft:max_damage]").unwrap();
        assert_eq!(p.tests, vec![ItemTest::Any(vec![comp("minecraft:damage", false), comp("minecraft:max_damage", false)])]);

        let (p, _) = pred("stone[minecraft:damage,!minecraft:max_damage]").unwrap();
        assert_eq!(p.tests, vec![comp("minecraft:damage", false), comp("minecraft:max_damage", true)]);
    }

    #[test]
    fn predicate_errors() {
        // unknown item: the element lookup error, cursor after the id
        assert_eq!(pred_err("nope"), ("argument.item.id.invalid".into(), vec!["minecraft:nope".into()], 4));
        // no identifier after `[`: the first stored error is "Invalid ID" at the cursor
        assert_eq!(pred_err("stone["), ("argument.id.invalid".into(), vec![], 6));
        // unknown component: the first stored error, cursor after the id
        assert_eq!(pred_err("stone[minecraft:nope]"), ("arguments.item.component.unknown".into(), vec!["minecraft:nope".into()], 20));
        assert_eq!(pred_err("#minecraft:nope"), ("arguments.item.tag.unknown".into(), vec!["minecraft:nope".into()], 15));
    }
}
