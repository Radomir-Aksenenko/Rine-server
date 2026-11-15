//! SNBT parsing and printing (`SnbtGrammar`, `TagParser`, `StringTagVisitor`
//! and `TextComponentTagVisitor` of 26.3).
//!
//! The grammar is a hand-written recursive descent. It keeps the packrat rules
//! of the Java parser where they show up in errors: every failed terminal
//! stores an error at its cursor (after skipping whitespace), the furthest
//! cursor wins and the first error stored there is reported. A cut stops the
//! other alternatives of its enclosing choice. Rule actions that reject a
//! value store their error at the end of the matched text.
//!
//! Deviations from vanilla:
//! - `\N{name}` always fails: there is no Unicode name table in std.
//! - A lone surrogate from `\uD800`-style escapes cannot live in a Rust
//!   string, so it becomes U+FFFD.
//! - Compound keys keep insertion order; vanilla iterates a hash map.
//! - The 512 nesting limit ("Too deep") is Paper's; the message is not
//!   translated.
//! - `NumberFormatException` texts follow the JDK wording, approximated.

use crate::error::{java_double, java_float, lit, CmdError, CmdResult};
use crate::reader::{is_java_whitespace, Reader};
use rine_protocol::nbt::Tag;
use rine_protocol::text::Text;

/// Paper's nesting limit for lists and compounds.
const MAX_DEPTH: u32 = 512;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sign {
    Plus,
    Minus,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Base {
    Binary,
    Decimal,
    Hex,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum IntTy {
    Byte,
    Short,
    Int,
    Long,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FloatTy {
    Float,
    Double,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ArrTy {
    Byte,
    Int,
    Long,
}

impl ArrTy {
    fn default_ty(self) -> IntTy {
        match self {
            ArrTy::Byte => IntTy::Byte,
            ArrTy::Int => IntTy::Int,
            ArrTy::Long => IntTy::Long,
        }
    }

    fn allows(self, t: IntTy) -> bool {
        match self {
            ArrTy::Byte => t == IntTy::Byte,
            ArrTy::Int => matches!(t, IntTy::Int | IntTy::Byte | IntTy::Short),
            ArrTy::Long => matches!(t, IntTy::Long | IntTy::Byte | IntTy::Short | IntTy::Int),
        }
    }
}

/// `IntegerSuffix`: `signed` is `Some(true)` for `s` prefix, `Some(false)` for `u`.
#[derive(Clone, Copy, Debug)]
struct IntSuffix {
    signed: Option<bool>,
    ty: Option<IntTy>,
}

#[derive(Clone, Copy, Debug)]
struct IntLit<'a> {
    sign: Sign,
    base: Base,
    digits: &'a str,
    suffix: IntSuffix,
}

/// Error message as Java builds it: a translation key, optionally with one
/// argument, or a literal "Expected literal X" from a terminal.
#[derive(Clone, Debug)]
enum Msg {
    Key(&'static str),
    Literal(&'static str),
    Arg(&'static str, String),
    Plain(&'static str),
}

impl Msg {
    fn text(&self) -> Text {
        match self {
            Msg::Key(k) => Text::key(*k),
            Msg::Literal(joined) => Text::tr("argument.literal.incorrect", vec![lit(joined)]),
            Msg::Arg(k, a) => Text::tr(*k, vec![lit(a)]),
            Msg::Plain(s) => Text::plain(*s),
        }
    }
}

/// Recursive-descent state. Every method that returns `None` leaves `pos`
/// where it was when it was called.
struct P<'a> {
    s: &'a str,
    pos: usize,
    /// Furthest error cursor so far (`LongestOnly` of the Java collector).
    last: isize,
    /// Errors stored at `last`, in store order.
    entries: Vec<(usize, Msg)>,
    /// Java `Scope.depth`: incremented on entry, not restored on failure.
    depth: u32,
}

impl<'a> P<'a> {
    fn new(s: &'a str, pos: usize) -> Self {
        P { s, pos, last: -1, entries: Vec::new(), depth: 0 }
    }

    fn store(&mut self, at: usize, msg: Msg) {
        let c = at as isize;
        if c > self.last {
            self.last = c;
            self.entries.clear();
        }
        if c == self.last {
            self.entries.push((at, msg));
        }
    }

    /// The error Java's `Grammar.parseForCommands` throws: the first one
    /// stored at the furthest cursor.
    fn error(&self, r: &Reader) -> CmdError {
        match self.entries.first() {
            Some((at, msg)) => CmdError::at_cursor(msg.text(), r, *at),
            None => CmdError::at(Text::key("command.failed"), r),
        }
    }

    fn peek(&self) -> Option<char> {
        self.s[self.pos..].chars().next()
    }

    fn peek_after_ws(&self) -> Option<char> {
        self.s[self.pos..].chars().find(|c| !is_java_whitespace(*c))
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if !is_java_whitespace(c) {
                break;
            }
            self.pos += c.len_utf8();
        }
    }

    /// `TerminalCharacters`: skip whitespace, take one char from `accept`.
    fn term(&mut self, accept: &[char], joined: &'static str) -> bool {
        let m = self.pos;
        self.skip_ws();
        let at = self.pos;
        if let Some(c) = self.peek() {
            if accept.contains(&c) {
                self.pos += c.len_utf8();
                return true;
            }
        }
        self.store(at, Msg::Literal(joined));
        self.pos = m;
        false
    }

    fn ch(&mut self, c: char) -> bool {
        self.term(&[c], one_str(c))
    }

    /// `opt`-style repetition of a separated list (`repeatedWithTrailingSeparator`,
    /// minimum 0). Always succeeds.
    fn rep_sep<T>(&mut self, mut elem: impl FnMut(&mut P<'a>) -> Option<T>) -> Vec<T> {
        let mut out = Vec::new();
        let mut first = true;
        loop {
            let before = self.pos;
            if !first && !self.ch(',') {
                self.pos = before;
                break;
            }
            let after = self.pos;
            match elem(&mut *self) {
                Some(v) => out.push(v),
                None => {
                    self.pos = after;
                    break;
                }
            }
            first = false;
        }
        out
    }

    /// `sign` rule: `+` or `-`, default plus when absent.
    fn opt_sign(&mut self) -> Sign {
        if self.term(&['+'], "+") {
            return Sign::Plus;
        }
        if self.term(&['-'], "-") {
            return Sign::Minus;
        }
        Sign::Plus
    }

    /// `NumberRunParseRule`: whitespace, then a run of `base` digits and
    /// underscores that may not start or end with `_`.
    fn number_run(&mut self, base: Base) -> Option<&'a str> {
        let m = self.pos;
        self.skip_ws();
        let s = self.s;
        let bytes = s.as_bytes();
        let start = self.pos;
        let mut end = start;
        while end < bytes.len() && base_accepts(base, bytes[end]) {
            end += 1;
        }
        if end == start {
            self.store(start, no_value_msg(base));
            self.pos = m;
            return None;
        }
        if bytes[start] != b'_' && bytes[end - 1] != b'_' {
            self.pos = end;
            return Some(&s[start..end]);
        }
        self.store(start, Msg::Key("snbt.parser.underscore_not_allowed"));
        self.pos = m;
        None
    }

    /// `UnquotedStringParseRule(1)`: `[0-9A-Za-z_.+-]+` after whitespace.
    fn unquoted_raw(&mut self) -> Option<&'a str> {
        let m = self.pos;
        self.skip_ws();
        let s = self.s;
        let bytes = s.as_bytes();
        let start = self.pos;
        let mut end = start;
        while end < bytes.len() && is_unquoted_byte(bytes[end]) {
            end += 1;
        }
        if end == start {
            self.store(start, Msg::Key("snbt.parser.expected_unquoted_string"));
            self.pos = m;
            return None;
        }
        self.pos = end;
        Some(&s[start..end])
    }

    fn enter_depth(&mut self) -> bool {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            let at = self.pos;
            self.store(at, Msg::Plain("Too deep"));
            return false;
        }
        true
    }

    // ----- integers -------------------------------------------------------

    /// `integer_literal` rule: sign, digits in one of three bases, suffix.
    fn integer_literal(&mut self) -> Option<IntLit<'a>> {
        let m = self.pos;
        let sign = self.opt_sign();
        match self.integer_body() {
            Some((base, digits)) => {
                let suffix = self.int_suffix().unwrap_or(IntSuffix { signed: None, ty: None });
                Some(IntLit { sign, base, digits, suffix })
            }
            None => {
                self.pos = m;
                None
            }
        }
    }

    fn integer_body(&mut self) -> Option<(Base, &'a str)> {
        let m = self.pos;
        if self.ch('0') {
            // Cut: once '0' matched, failures below do not try the decimal branch.
            let after0 = self.pos;
            if self.term(&['x', 'X'], "x|X") {
                return self.number_run(Base::Hex).map(|d| (Base::Hex, d));
            }
            self.pos = after0;
            if self.term(&['b', 'B'], "b|B") {
                if let Some(d) = self.number_run(Base::Binary) {
                    return Some((Base::Binary, d));
                }
            }
            self.pos = after0;
            if self.number_run(Base::Decimal).is_some() {
                let at = self.pos;
                self.store(at, Msg::Key("snbt.parser.leading_zero_not_allowed"));
                self.pos = m;
                return None;
            }
            self.pos = after0;
            return Some((Base::Decimal, "0"));
        }
        self.pos = m;
        self.number_run(Base::Decimal).map(|d| (Base::Decimal, d))
    }

    /// `integerSuffix`: `u`/`s` prefix (unsigned/signed) then a type letter,
    /// or a bare type letter.
    fn int_suffix(&mut self) -> Option<IntSuffix> {
        let m = self.pos;
        if self.term(&['u', 'U'], "u|U") {
            if let Some(ty) = self.suffix_type() {
                return Some(IntSuffix { signed: Some(false), ty: Some(ty) });
            }
        }
        self.pos = m;
        if self.term(&['s', 'S'], "s|S") {
            if let Some(ty) = self.suffix_type() {
                return Some(IntSuffix { signed: Some(true), ty: Some(ty) });
            }
        }
        self.pos = m;
        if let Some(ty) = self.suffix_type() {
            return Some(IntSuffix { signed: None, ty: Some(ty) });
        }
        self.pos = m;
        None
    }

    fn suffix_type(&mut self) -> Option<IntTy> {
        let m = self.pos;
        if self.term(&['b', 'B'], "b|B") {
            return Some(IntTy::Byte);
        }
        self.pos = m;
        if self.term(&['s', 'S'], "s|S") {
            return Some(IntTy::Short);
        }
        self.pos = m;
        if self.term(&['i', 'I'], "i|I") {
            return Some(IntTy::Int);
        }
        self.pos = m;
        if self.term(&['l', 'L'], "l|L") {
            return Some(IntTy::Long);
        }
        self.pos = m;
        None
    }

    /// Rule action of `integer_literal` in a plain literal: parse the digits
    /// as the suffix type, reporting range errors at the end of the literal.
    fn create_int(&mut self, il: &IntLit<'a>, ty: IntTy) -> Option<Tag> {
        let signed = il.suffix.signed.unwrap_or(il.base == Base::Decimal);
        if !signed && il.sign == Sign::Minus {
            let at = self.pos;
            self.store(at, Msg::Key("snbt.parser.expected_non_negative_number"));
            return None;
        }
        let mut digits = String::new();
        if il.sign == Sign::Minus {
            digits.push('-');
        }
        push_clean(&mut digits, il.digits);
        let radix = match il.base {
            Base::Binary => 2,
            Base::Decimal => 10,
            Base::Hex => 16,
        };
        let res = if signed { parse_signed(&digits, radix, ty) } else { parse_unsigned(&digits, radix, ty) };
        match res {
            Ok(t) => Some(t),
            Err(msg) => {
                let at = self.pos;
                self.store(at, Msg::Arg("snbt.parser.number_parse_failure", msg));
                None
            }
        }
    }

    // ----- floats ---------------------------------------------------------

    fn float_literal(&mut self) -> Option<Tag> {
        let m = self.pos;
        let r = self.float_inner();
        if r.is_none() {
            self.pos = m;
        }
        r
    }

    fn float_inner(&mut self) -> Option<Tag> {
        let m = self.pos;
        let sign = self.opt_sign();
        let after_sign = self.pos;

        // alt 1: whole '.' [fraction] [exponent] [suffix]
        if let Some(whole) = self.number_run(Base::Decimal) {
            if self.ch('.') {
                let frac = self.number_run(Base::Decimal);
                let exp = self.exponent();
                let suf = self.float_suffix();
                return self.float_value(sign, Some(whole), frac, exp, suf);
            }
        }
        self.pos = after_sign;

        // alt 2: '.' fraction [exponent] [suffix]   (cut after '.')
        if self.ch('.') {
            let frac = self.number_run(Base::Decimal)?;
            let exp = self.exponent();
            let suf = self.float_suffix();
            return self.float_value(sign, None, Some(frac), exp, suf);
        }
        self.pos = after_sign;

        // alt 3: whole exponent [suffix]   (cut after the exponent)
        if let Some(whole) = self.number_run(Base::Decimal) {
            if let Some(exp) = self.exponent() {
                let suf = self.float_suffix();
                return self.float_value(sign, Some(whole), None, Some(exp), suf);
            }
        }
        self.pos = after_sign;

        // alt 4: whole [exponent] suffix   (suffix required)
        if let Some(whole) = self.number_run(Base::Decimal) {
            let exp = self.exponent();
            if let Some(suf) = self.float_suffix() {
                return self.float_value(sign, Some(whole), None, exp, Some(suf));
            }
        }
        self.pos = m;
        None
    }

    fn exponent(&mut self) -> Option<(Sign, &'a str)> {
        let m = self.pos;
        if !self.term(&['e', 'E'], "e|E") {
            return None;
        }
        let sign = self.opt_sign();
        match self.number_run(Base::Decimal) {
            Some(d) => Some((sign, d)),
            None => {
                self.pos = m;
                None
            }
        }
    }

    fn float_suffix(&mut self) -> Option<FloatTy> {
        let m = self.pos;
        if self.term(&['f', 'F'], "f|F") {
            return Some(FloatTy::Float);
        }
        self.pos = m;
        if self.term(&['d', 'D'], "d|D") {
            return Some(FloatTy::Double);
        }
        self.pos = m;
        None
    }

    /// `createFloat`: rebuild the text without underscores and parse it.
    fn float_value(
        &mut self,
        sign: Sign,
        whole: Option<&str>,
        frac: Option<&str>,
        exp: Option<(Sign, &str)>,
        suf: Option<FloatTy>,
    ) -> Option<Tag> {
        let mut text = String::new();
        if sign == Sign::Minus {
            text.push('-');
        }
        if let Some(w) = whole {
            push_clean(&mut text, w);
        }
        if let Some(f) = frac {
            text.push('.');
            push_clean(&mut text, f);
        }
        if let Some((es, e)) = exp {
            text.push('e');
            if es == Sign::Minus {
                text.push('-');
            }
            push_clean(&mut text, e);
        }
        let at = self.pos;
        match suf {
            Some(FloatTy::Float) => match text.parse::<f32>() {
                Ok(v) if v.is_finite() => Some(Tag::Float(v)),
                Ok(_) => {
                    self.store(at, Msg::Key("snbt.parser.infinity_not_allowed"));
                    None
                }
                Err(_) => {
                    self.store(at, number_failure(&text));
                    None
                }
            },
            _ => match text.parse::<f64>() {
                Ok(v) if v.is_finite() => Some(Tag::Double(v)),
                Ok(_) => {
                    self.store(at, Msg::Key("snbt.parser.infinity_not_allowed"));
                    None
                }
                Err(_) => {
                    self.store(at, number_failure(&text));
                    None
                }
            },
        }
    }

    // ----- strings --------------------------------------------------------

    fn quoted_string(&mut self) -> Option<String> {
        let m = self.pos;
        if self.ch('"') {
            // Cut: a missing closing quote fails the literal.
            let mut units = Vec::new();
            self.quoted_contents('"', &mut units);
            if self.ch('"') {
                return Some(String::from_utf16_lossy(&units));
            }
            self.pos = m;
            return None;
        }
        if self.ch('\'') {
            let mut units = Vec::new();
            self.quoted_contents('\'', &mut units);
            if self.ch('\'') {
                return Some(String::from_utf16_lossy(&units));
            }
        }
        self.pos = m;
        None
    }

    fn quoted_contents(&mut self, q: char, units: &mut Vec<u16>) {
        loop {
            let m = self.pos;
            if !self.chunk(q, units) {
                self.pos = m;
                break;
            }
        }
    }

    /// One piece of a quoted string: plain text, an escape, or the other quote.
    fn chunk(&mut self, q: char, units: &mut Vec<u16>) -> bool {
        let m = self.pos;
        let s = self.s;
        let rest = &s[m..];
        let n = rest.find(|c: char| c == '"' || c == '\'' || c == '\\').unwrap_or(rest.len());
        if n > 0 {
            for c in rest[..n].chars() {
                push_char(units, c);
            }
            self.pos = m + n;
            return true;
        }
        self.store(m, Msg::Key("snbt.parser.invalid_string_contents"));
        if self.ch('\\') && self.escape(units) {
            return true;
        }
        self.pos = m;
        let other = if q == '"' { '\'' } else { '"' };
        if self.ch(other) {
            push_char(units, other);
            return true;
        }
        self.pos = m;
        false
    }

    /// Escape sequence after the backslash.
    fn escape(&mut self, units: &mut Vec<u16>) -> bool {
        let m = self.pos;
        const SIMPLE: [(char, char); 9] = [
            ('b', '\u{8}'),
            ('s', ' '),
            ('t', '\t'),
            ('n', '\n'),
            ('f', '\u{c}'),
            ('r', '\r'),
            ('\\', '\\'),
            ('\'', '\''),
            ('"', '"'),
        ];
        for (c, v) in SIMPLE {
            if self.ch(c) {
                push_char(units, v);
                return true;
            }
        }
        self.pos = m;

        if self.ch('x') {
            if let Some(cp) = self.hex_digits(2) {
                if self.push_cp(cp, units) {
                    return true;
                }
            }
        }
        self.pos = m;

        if self.ch('u') {
            if let Some(cp) = self.hex_digits(4) {
                if self.push_cp(cp, units) {
                    return true;
                }
            }
        }
        self.pos = m;

        if self.ch('U') {
            if let Some(cp) = self.hex_digits(8) {
                if self.push_cp(cp, units) {
                    return true;
                }
            }
        }
        self.pos = m;

        if self.ch('N') && self.ch('{') {
            let s = self.s;
            let name_start = self.pos;
            let n = s.as_bytes()[name_start..].iter().take_while(|b| is_name_byte(**b)).count();
            if n == 0 {
                self.store(name_start, Msg::Key("snbt.parser.invalid_character_name"));
            } else {
                self.pos = name_start + n;
                if self.ch('}') {
                    // Vanilla resolves the name with the Unicode database; this
                    // port has no name table, so every name is rejected here.
                    let at = self.pos;
                    self.store(at, Msg::Key("snbt.parser.invalid_character_name"));
                }
            }
        }
        self.pos = m;
        false
    }

    /// Exactly `n` hex digits (`GreedyPredicateParseRule(n, n)`, no whitespace skip).
    fn hex_digits(&mut self, n: usize) -> Option<u32> {
        let start = self.pos;
        let bytes = self.s.as_bytes();
        let mut k = 0;
        while k < n && start + k < bytes.len() && bytes[start + k].is_ascii_hexdigit() {
            k += 1;
        }
        if k < n {
            self.store(start, Msg::Arg("snbt.parser.expected_hex_escape", n.to_string()));
            return None;
        }
        let mut v: u32 = 0;
        for &b in &bytes[start..start + n] {
            v = v * 16 + (b as char).to_digit(16).unwrap_or(0);
        }
        self.pos = start + n;
        Some(v)
    }

    fn push_cp(&mut self, cp: u32, units: &mut Vec<u16>) -> bool {
        if cp > 0x10FFFF {
            let at = self.pos;
            self.store(at, Msg::Arg("snbt.parser.invalid_codepoint", format!("U+{cp:08X}")));
            return false;
        }
        match char::from_u32(cp) {
            Some(c) => push_char(units, c),
            // Surrogate code point: keep the code unit as Java does.
            None => units.push(cp as u16),
        }
        true
    }

    // ----- containers -----------------------------------------------------

    fn map_entry(&mut self) -> Option<(String, Tag)> {
        let m = self.pos;
        let key = match self.map_key() {
            Some(k) => k,
            None => {
                self.pos = m;
                return None;
            }
        };
        if !self.ch(':') {
            self.pos = m;
            return None;
        }
        let value = match self.literal() {
            Some(v) => v,
            None => {
                self.pos = m;
                return None;
            }
        };
        if key.is_empty() {
            let at = self.pos;
            self.store(at, Msg::Key("snbt.parser.empty_key"));
            self.pos = m;
            return None;
        }
        Some((key, value))
    }

    fn map_key(&mut self) -> Option<String> {
        let m = self.pos;
        if let Some(q) = self.quoted_string() {
            return Some(q);
        }
        self.pos = m;
        self.unquoted_raw().map(|s| s.to_string())
    }

    fn map_literal(&mut self) -> Option<Tag> {
        let m = self.pos;
        let r = self.map_inner();
        if r.is_none() {
            self.pos = m;
        }
        r
    }

    fn map_inner(&mut self) -> Option<Tag> {
        if !self.ch('{') || !self.enter_depth() {
            return None;
        }
        let entries = self.rep_sep(|p| p.map_entry());
        self.depth = self.depth.saturating_sub(1);
        if !self.ch('}') {
            return None;
        }
        Some(build_compound(entries))
    }

    fn list_literal(&mut self) -> Option<Tag> {
        let m = self.pos;
        let r = self.list_inner();
        if r.is_none() {
            self.pos = m;
        }
        r
    }

    fn list_inner(&mut self) -> Option<Tag> {
        if !self.ch('[') || !self.enter_depth() {
            return None;
        }
        let m2 = self.pos;
        let mut arr: Option<(ArrTy, Vec<IntLit<'a>>)> = None;
        if let Some(prefix) = self.array_prefix() {
            if self.ch(';') {
                let entries = self.rep_sep(|p| p.integer_literal());
                arr = Some((prefix, entries));
            }
        }
        let (arr, list) = match arr {
            Some(a) => (Some(a), Vec::new()),
            None => {
                self.pos = m2;
                (None, self.rep_sep(|p| p.literal()))
            }
        };
        self.depth = self.depth.saturating_sub(1);
        if !self.ch(']') {
            return None;
        }
        match arr {
            Some((prefix, entries)) => self.array_value(prefix, entries),
            None => Some(Tag::List(list)),
        }
    }

    fn array_prefix(&mut self) -> Option<ArrTy> {
        if self.ch('B') {
            return Some(ArrTy::Byte);
        }
        if self.ch('L') {
            return Some(ArrTy::Long);
        }
        if self.ch('I') {
            return Some(ArrTy::Int);
        }
        None
    }

    /// `ArrayPrefix.create`: element types are checked against the array kind.
    fn array_value(&mut self, prefix: ArrTy, entries: Vec<IntLit<'a>>) -> Option<Tag> {
        if entries.is_empty() {
            return Some(match prefix {
                ArrTy::Byte => Tag::ByteArray(Vec::new()),
                ArrTy::Int => Tag::IntArray(Vec::new()),
                ArrTy::Long => Tag::LongArray(Vec::new()),
            });
        }
        let mut bytes: Vec<i8> = Vec::new();
        let mut ints: Vec<i32> = Vec::new();
        let mut longs: Vec<i64> = Vec::new();
        for e in &entries {
            let ty = match e.suffix.ty {
                None => prefix.default_ty(),
                Some(t) if prefix.allows(t) => t,
                Some(_) => {
                    let at = self.pos;
                    self.store(at, Msg::Key("snbt.parser.invalid_array_element_type"));
                    return None;
                }
            };
            let n = self.create_int(e, ty)?;
            let v = int_value(&n);
            match prefix {
                ArrTy::Byte => bytes.push(v as i8),
                ArrTy::Int => ints.push(v as i32),
                ArrTy::Long => longs.push(v),
            }
        }
        Some(match prefix {
            ArrTy::Byte => Tag::ByteArray(bytes),
            ArrTy::Int => Tag::IntArray(ints),
            ArrTy::Long => Tag::LongArray(longs),
        })
    }

    // ----- unquoted strings and built-ins ----------------------------------

    fn unquoted_or_builtin(&mut self) -> Option<Tag> {
        let m = self.pos;
        let contents = self.unquoted_raw()?;

        // `name(args)`: the argument list is kept even when ')' is missing
        // (Java's scope keeps the value), but the cursor goes back to the '('.
        let m_args = self.pos;
        let mut args: Option<Vec<Tag>> = None;
        if self.ch('(') {
            args = Some(self.rep_sep(|p| p.literal()));
            if !self.ch(')') {
                self.pos = m_args;
            }
        }

        if contents.chars().next().is_some_and(can_start_number) {
            let at = self.pos;
            self.store(at, Msg::Key("snbt.parser.invalid_unquoted_start"));
            self.pos = m;
            return None;
        }

        match args {
            Some(list) => {
                let r = match (contents, list.len()) {
                    ("bool", 1) => self.builtin_bool(&list[0]),
                    ("uuid", 1) => self.builtin_uuid(&list[0]),
                    _ => {
                        let at = self.pos;
                        let key = format!("{contents}/{}", list.len());
                        self.store(at, Msg::Arg("snbt.parser.no_such_operation", key));
                        None
                    }
                };
                if r.is_none() {
                    self.pos = m;
                }
                r
            }
            None => Some(if contents.eq_ignore_ascii_case("true") {
                Tag::Byte(1)
            } else if contents.eq_ignore_ascii_case("false") {
                Tag::Byte(0)
            } else {
                Tag::String(contents.to_string())
            }),
        }
    }

    fn builtin_bool(&mut self, arg: &Tag) -> Option<Tag> {
        match numeric_f64(arg) {
            Some(v) => Some(Tag::Byte(if v != 0.0 { 1 } else { 0 })),
            None => {
                let at = self.pos;
                self.store(at, Msg::Key("snbt.parser.expected_number_or_boolean"));
                None
            }
        }
    }

    fn builtin_uuid(&mut self, arg: &Tag) -> Option<Tag> {
        let parsed = match arg {
            Tag::String(s) => parse_uuid(s),
            _ => None,
        };
        match parsed {
            Some(v) => Some(Tag::IntArray(v.to_vec())),
            None => {
                let at = self.pos;
                self.store(at, Msg::Key("snbt.parser.expected_string_uuid"));
                None
            }
        }
    }

    // ----- top rule -------------------------------------------------------

    /// `literal` rule: the top of the grammar.
    fn literal(&mut self) -> Option<Tag> {
        let m = self.pos;
        let r = self.literal_inner();
        if r.is_none() {
            self.pos = m;
        }
        r
    }

    fn literal_inner(&mut self) -> Option<Tag> {
        if self.peek_after_ws().is_some_and(can_start_number) {
            if let Some(v) = self.float_literal() {
                return Some(v);
            }
            let m = self.pos;
            if let Some(il) = self.integer_literal() {
                let ty = il.suffix.ty.unwrap_or(IntTy::Int);
                return self.create_int(&il, ty);
            }
            self.pos = m;
        }
        match self.peek_after_ws() {
            Some('"') | Some('\'') => {
                let s = self.quoted_string()?;
                Some(Tag::String(s))
            }
            Some('{') => self.map_literal(),
            Some('[') => self.list_literal(),
            _ => self.unquoted_or_builtin(),
        }
    }
}

// ----- helpers ----------------------------------------------------------------

fn one_str(c: char) -> &'static str {
    match c {
        '"' => "\"",
        '\'' => "'",
        '\\' => "\\",
        '(' => "(",
        ')' => ")",
        '[' => "[",
        ']' => "]",
        '{' => "{",
        '}' => "}",
        ',' => ",",
        ':' => ":",
        ';' => ";",
        '.' => ".",
        '+' => "+",
        '-' => "-",
        '0' => "0",
        'B' => "B",
        'I' => "I",
        'L' => "L",
        'N' => "N",
        'U' => "U",
        'b' => "b",
        'f' => "f",
        'n' => "n",
        'r' => "r",
        's' => "s",
        't' => "t",
        'u' => "u",
        'x' => "x",
        _ => "?",
    }
}

fn base_accepts(base: Base, b: u8) -> bool {
    match base {
        Base::Binary => matches!(b, b'0' | b'1' | b'_'),
        Base::Decimal => b.is_ascii_digit() || b == b'_',
        Base::Hex => b.is_ascii_hexdigit() || b == b'_',
    }
}

fn no_value_msg(base: Base) -> Msg {
    Msg::Key(match base {
        Base::Binary => "snbt.parser.expected_binary_numeral",
        Base::Decimal => "snbt.parser.expected_decimal_numeral",
        Base::Hex => "snbt.parser.expected_hex_numeral",
    })
}

fn is_unquoted_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.' || b == b'+'
}

fn is_name_byte(b: u8) -> bool {
    b == b'-' || b == b' ' || b.is_ascii_alphanumeric()
}

/// `canStartNumber`: `+ - . 0-9`.
fn can_start_number(c: char) -> bool {
    matches!(c, '+' | '-' | '.' | '0'..='9')
}

fn push_char(units: &mut Vec<u16>, c: char) {
    let mut buf = [0u16; 2];
    units.extend_from_slice(c.encode_utf16(&mut buf));
}

fn push_clean(out: &mut String, s: &str) {
    out.extend(s.chars().filter(|c| *c != '_'));
}

fn number_failure(text: &str) -> Msg {
    Msg::Arg("snbt.parser.number_parse_failure", format!("For input string: \"{text}\""))
}

fn int_value(t: &Tag) -> i64 {
    match t {
        Tag::Byte(v) => *v as i64,
        Tag::Short(v) => *v as i64,
        Tag::Int(v) => *v as i64,
        Tag::Long(v) => *v,
        _ => 0,
    }
}

/// `build` of a map literal: keys keep the first position, the last value wins.
fn build_compound(entries: Vec<(String, Tag)>) -> Tag {
    let mut out: Vec<(String, Tag)> = Vec::with_capacity(entries.len());
    for (k, v) in entries {
        if let Some(p) = out.iter().position(|e| e.0 == k) {
            out[p].1 = v;
        } else {
            out.push((k, v));
        }
    }
    Tag::Compound(out)
}

fn numeric_f64(t: &Tag) -> Option<f64> {
    match t {
        Tag::Byte(v) => Some(*v as f64),
        Tag::Short(v) => Some(*v as f64),
        Tag::Int(v) => Some(*v as f64),
        Tag::Long(v) => Some(*v as f64),
        Tag::Float(v) => Some(*v as f64),
        Tag::Double(v) => Some(*v),
        _ => None,
    }
}

fn for_input(d: &str, radix: u32) -> String {
    let suffix = if radix == 10 { String::new() } else { format!(" under radix {radix}") };
    format!("For input string: \"{d}\"{suffix}")
}

fn out_of_range(d: &str, radix: u32) -> String {
    format!("Value out of range. Value:\"{d}\" Radix:{radix}")
}

/// Digits with an optional leading `-`, accumulated in `i128`.
fn parse_big(d: &str, radix: u32) -> Option<(bool, i128)> {
    let (neg, body) = match d.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, d),
    };
    if body.is_empty() {
        return None;
    }
    let mut v: i128 = 0;
    for c in body.chars() {
        let digit = c.to_digit(radix)? as i128;
        v = v.saturating_mul(radix as i128).saturating_add(digit);
    }
    Some((neg, v))
}

/// `Integer.parseInt(s, radix)`.
fn parse_i32(d: &str, radix: u32) -> Result<i32, String> {
    let (neg, v) = parse_big(d, radix).ok_or_else(|| for_input(d, radix))?;
    let v = if neg { -v } else { v };
    i32::try_from(v).map_err(|_| for_input(d, radix))
}

/// `Long.parseLong(s, radix)`.
fn parse_i64(d: &str, radix: u32) -> Result<i64, String> {
    let (neg, v) = parse_big(d, radix).ok_or_else(|| for_input(d, radix))?;
    let v = if neg { -v } else { v };
    i64::try_from(v).map_err(|_| for_input(d, radix))
}

/// Unsigned magnitude; a minus sign is rejected earlier for unsigned literals.
fn parse_unsigned_u64(d: &str, radix: u32, max: u64, what: &str) -> Result<u64, String> {
    if d.starts_with('-') {
        return Err(format!("Illegal leading minus sign on unsigned string {d}."));
    }
    let (_, v) = parse_big(d, radix).ok_or_else(|| for_input(d, radix))?;
    if v > max as i128 {
        return Err(format!("String value {d} exceeds range of unsigned {what}."));
    }
    Ok(v as u64)
}

/// `Byte.parseByte` / `Short.parseShort` / `Integer.parseInt` / `Long.parseLong`.
fn parse_signed(d: &str, radix: u32, ty: IntTy) -> Result<Tag, String> {
    match ty {
        IntTy::Byte => {
            let v = parse_i32(d, radix)?;
            if (-128..=127).contains(&v) {
                Ok(Tag::Byte(v as i8))
            } else {
                Err(out_of_range(d, radix))
            }
        }
        IntTy::Short => {
            let v = parse_i32(d, radix)?;
            if (-32768..=32767).contains(&v) {
                Ok(Tag::Short(v as i16))
            } else {
                Err(out_of_range(d, radix))
            }
        }
        IntTy::Int => Ok(Tag::Int(parse_i32(d, radix)?)),
        IntTy::Long => Ok(Tag::Long(parse_i64(d, radix)?)),
    }
}

/// Unsigned variants as Guava's `UnsignedBytes` and the port's `parseUnsignedShort`.
fn parse_unsigned(d: &str, radix: u32, ty: IntTy) -> Result<Tag, String> {
    match ty {
        IntTy::Byte => {
            let v = parse_i32(d, radix)?;
            if v >> 8 == 0 {
                Ok(Tag::Byte(v as u8 as i8))
            } else {
                Err(format!("out of range: {v}"))
            }
        }
        IntTy::Short => {
            let v = parse_i32(d, radix)?;
            if v >> 16 == 0 {
                Ok(Tag::Short(v as u16 as i16))
            } else {
                Err(format!("out of range: {v}"))
            }
        }
        IntTy::Int => {
            let v = parse_unsigned_u64(d, radix, u32::MAX as u64, "int")?;
            Ok(Tag::Int(v as u32 as i32))
        }
        IntTy::Long => {
            let v = parse_unsigned_u64(d, radix, u64::MAX, "long")?;
            Ok(Tag::Long(v as i64))
        }
    }
}

/// `UUID.fromString`: exactly five dash-separated hex groups, each group may be
/// short; the result is `UUIDUtil.uuidToIntArray`.
fn parse_uuid(s: &str) -> Option<[i32; 4]> {
    if s.encode_utf16().count() > 36 {
        return None;
    }
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 5 {
        return None;
    }
    let p1 = uuid_group(parts[0])?;
    let p2 = uuid_group(parts[1])?;
    let p3 = uuid_group(parts[2])?;
    let p4 = uuid_group(parts[3])?;
    let p5 = uuid_group(parts[4])?;

    let mut most: i64 = p1 & 0xffff_ffff;
    most <<= 16;
    most |= p2 & 0xffff;
    most <<= 16;
    most |= p3 & 0xffff;
    let mut least: i64 = p4 & 0xffff;
    least <<= 48;
    least |= p5 & 0xffff_ffff_ffff;
    Some([(most >> 32) as i32, most as i32, (least >> 32) as i32, least as i32])
}

/// `Long.parseLong(part, 16)`: optional sign, at least one hex digit.
fn uuid_group(part: &str) -> Option<i64> {
    let (neg, body) = match part.as_bytes().first() {
        Some(b'-') => (true, &part[1..]),
        Some(b'+') => (false, &part[1..]),
        _ => (false, part),
    };
    if body.is_empty() {
        return None;
    }
    let mut v: i128 = 0;
    for c in body.chars() {
        v = v * 16 + c.to_digit(16)? as i128;
        if v > i64::MAX as i128 + 1 {
            return None;
        }
    }
    let v = if neg { -v } else { v };
    i64::try_from(v).ok()
}

// ----- public API ---------------------------------------------------------------

/// Any tag (`NbtTagArgument`): parses one tag at the cursor and leaves the
/// cursor right after it (`TagParser.parseAsArgument`).
pub fn parse_tag(r: &mut Reader) -> CmdResult<Tag> {
    let input = r.input_arc();
    let s: &str = &input;
    let mut p = P::new(s, r.cursor);
    match p.literal() {
        Some(t) => {
            r.cursor = p.pos;
            Ok(t)
        }
        None => Err(p.error(r)),
    }
}

/// A compound (`CompoundTagArgument`); errors `argument.nbt.expected.compound`
/// with the cursor right after the parsed tag.
pub fn parse_compound(r: &mut Reader) -> CmdResult<Tag> {
    let t = parse_tag(r)?;
    if matches!(t, Tag::Compound(_)) {
        Ok(t)
    } else {
        Err(CmdError::at(Text::key("argument.nbt.expected.compound"), r))
    }
}

/// Parse a whole string as SNBT (`TagParser.parseFully`): trailing non-space
/// input is an error.
pub fn parse_str(s: &str) -> CmdResult<Tag> {
    let mut r = Reader::new(s);
    let t = parse_tag(&mut r)?;
    r.skip_whitespace();
    if r.can_read() {
        return Err(CmdError::at(Text::key("argument.nbt.trailing"), &r));
    }
    Ok(t)
}

/// `StringTagVisitor`: compact SNBT as `Tag.toString()` prints it. Compound
/// keys are sorted by UTF-16 order.
pub fn to_snbt(t: &Tag) -> String {
    let mut out = String::new();
    write_snbt(t, &mut out);
    out
}

fn write_snbt(t: &Tag, out: &mut String) {
    match t {
        Tag::Byte(v) => {
            out.push_str(&v.to_string());
            out.push('b');
        }
        Tag::Short(v) => {
            out.push_str(&v.to_string());
            out.push('s');
        }
        Tag::Int(v) => out.push_str(&v.to_string()),
        Tag::Long(v) => {
            out.push_str(&v.to_string());
            out.push('L');
        }
        Tag::Float(v) => {
            out.push_str(&java_float(*v));
            out.push('f');
        }
        Tag::Double(v) => {
            out.push_str(&java_double(*v));
            out.push('d');
        }
        Tag::String(s) => out.push_str(&quote_and_escape(s)),
        Tag::ByteArray(v) => {
            out.push_str("[B;");
            for (i, b) in v.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&b.to_string());
                out.push('B');
            }
            out.push(']');
        }
        Tag::IntArray(v) => {
            out.push_str("[I;");
            for (i, x) in v.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&x.to_string());
            }
            out.push(']');
        }
        Tag::LongArray(v) => {
            out.push_str("[L;");
            for (i, x) in v.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&x.to_string());
                out.push('L');
            }
            out.push(']');
        }
        Tag::List(l) => {
            out.push('[');
            for (i, e) in l.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_snbt(e, out);
            }
            out.push(']');
        }
        Tag::Compound(c) => {
            let mut entries: Vec<&(String, Tag)> = c.iter().collect();
            entries.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
            out.push('{');
            for (i, (k, v)) in entries.into_iter().map(|e| (&e.0, &e.1)).enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if is_unquoted_key(k) {
                    out.push_str(k);
                } else {
                    out.push_str(&quote_and_escape(k));
                }
                out.push(':');
                write_snbt(v, out);
            }
            out.push('}');
        }
    }
}

/// `StringTagVisitor.handleKeyEscape`: `[A-Za-z._][A-Za-z0-9._+-]*`, not true/false.
fn is_unquoted_key(k: &str) -> bool {
    if k.eq_ignore_ascii_case("true") || k.eq_ignore_ascii_case("false") {
        return false;
    }
    let mut chars = k.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '.' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-'))
}

/// `StringTag.quoteAndEscape`: the quote is the opposite of the first quote
/// char in the text (default `"`); only that quote char is escaped.
pub fn quote_and_escape(s: &str) -> String {
    let mut quote: Option<char> = None;
    let mut body = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        if c == '\\' {
            body.push_str("\\\\");
        } else if c != '"' && c != '\'' {
            match escape_control(c) {
                Some(e) => {
                    body.push('\\');
                    body.push_str(&e);
                }
                None => body.push(c),
            }
        } else {
            if quote.is_none() {
                quote = Some(if c == '"' { '\'' } else { '"' });
            }
            if quote == Some(c) {
                body.push('\\');
            }
            body.push(c);
        }
    }
    let q = quote.unwrap_or('"');
    let mut out = String::with_capacity(body.len() + 2);
    out.push(q);
    out.push_str(&body);
    out.push(q);
    out
}

/// `SnbtGrammar.escapeControlCharacters`.
fn escape_control(c: char) -> Option<String> {
    Some(match c {
        '\u{8}' => "b".to_string(),
        '\t' => "t".to_string(),
        '\n' => "n".to_string(),
        '\u{c}' => "f".to_string(),
        '\r' => "r".to_string(),
        c if (c as u32) < 0x20 => format!("x{:02X}", c as u32),
        _ => return None,
    })
}

/// `NbtUtils.toPrettyComponent` (`TextComponentTagVisitor` with no indentation
/// and rich styling). Keys stay in insertion order.
pub fn to_pretty_text(t: &Tag) -> Text {
    let mut out = Vec::new();
    pretty(t, 0, &mut out);
    let mut root = Text::plain("");
    root.extra = out;
    root
}

fn num(s: impl Into<String>) -> Text {
    Text::plain(s).color("gold")
}

fn suffix(s: &'static str) -> Text {
    Text::plain(s).color("red")
}

fn folded() -> Text {
    Text::plain("<...>").color("gray")
}

fn pretty(t: &Tag, depth: u32, out: &mut Vec<Text>) {
    match t {
        Tag::Byte(v) => {
            out.push(num(v.to_string()));
            out.push(suffix("b"));
        }
        Tag::Short(v) => {
            out.push(num(v.to_string()));
            out.push(suffix("s"));
        }
        Tag::Int(v) => out.push(num(v.to_string())),
        Tag::Long(v) => {
            out.push(num(v.to_string()));
            out.push(suffix("L"));
        }
        Tag::Float(v) => {
            out.push(num(java_float(*v)));
            out.push(suffix("f"));
        }
        Tag::Double(v) => {
            out.push(num(java_double(*v)));
            out.push(suffix("d"));
        }
        Tag::String(s) => {
            let quoted = quote_and_escape(s);
            let q = quoted.chars().next().unwrap_or('"');
            let inner = &quoted[1..quoted.len() - 1];
            out.push(Text::plain(q.to_string()));
            out.push(Text::plain(inner).color("green"));
            out.push(Text::plain(q.to_string()));
        }
        Tag::ByteArray(v) => {
            array_open(out, "B");
            for (i, b) in v.iter().take(128).enumerate() {
                out.push(Text::plain(" "));
                out.push(num(b.to_string()));
                out.push(suffix("b"));
                if i + 1 != v.len() {
                    out.push(Text::plain(","));
                }
            }
            if v.len() > 128 {
                out.push(folded());
            }
            out.push(Text::plain("]"));
        }
        Tag::IntArray(v) => {
            array_open(out, "I");
            for (i, x) in v.iter().take(128).enumerate() {
                out.push(Text::plain(" "));
                out.push(num(x.to_string()));
                if i + 1 != v.len() {
                    out.push(Text::plain(","));
                }
            }
            if v.len() > 128 {
                out.push(folded());
            }
            out.push(Text::plain("]"));
        }
        Tag::LongArray(v) => {
            array_open(out, "L");
            for (i, x) in v.iter().take(128).enumerate() {
                out.push(Text::plain(" "));
                out.push(num(x.to_string()));
                out.push(suffix("L"));
                if i + 1 != v.len() {
                    out.push(Text::plain(","));
                }
            }
            if v.len() > 128 {
                out.push(folded());
            }
            out.push(Text::plain("]"));
        }
        Tag::List(l) => {
            if l.is_empty() {
                out.push(Text::plain("[]"));
            } else if depth >= 64 {
                out.push(Text::plain("["));
                out.push(folded());
                out.push(Text::plain("]"));
            } else {
                out.push(Text::plain("["));
                for (i, e) in l.iter().enumerate() {
                    if i > 0 {
                        out.push(Text::plain(", "));
                    }
                    pretty(e, depth + 1, out);
                }
                out.push(Text::plain("]"));
            }
        }
        Tag::Compound(c) => {
            if c.is_empty() {
                out.push(Text::plain("{}"));
            } else if depth >= 64 {
                out.push(Text::plain("{"));
                out.push(folded());
                out.push(Text::plain("}"));
            } else {
                out.push(Text::plain("{"));
                for (i, &j) in java_hash_order(c).iter().enumerate() {
                    let (k, v) = &c[j];
                    if i > 0 {
                        out.push(Text::plain(", "));
                    }
                    out.push(pretty_key(k));
                    out.push(Text::plain(":"));
                    out.push(Text::plain(" "));
                    pretty(v, depth + 1, out);
                }
                out.push(Text::plain("}"));
            }
        }
    }
}

/// Iteration order of a `CompoundTag` (a Java `HashMap` filled one key at a
/// time): by bucket of `String.hashCode` in the table the map grew to, keys
/// of one bucket in insertion order.
pub fn java_hash_order(c: &[(String, Tag)]) -> Vec<usize> {
    let mut cap: usize = 16;
    while c.len() * 4 > cap * 3 {
        cap *= 2;
    }
    let bucket = |k: &str| {
        let h = k.encode_utf16().fold(0i32, |h, u| h.wrapping_mul(31).wrapping_add(u as i32)) as u32;
        ((h ^ (h >> 16)) as usize) & (cap - 1)
    };
    let mut idx: Vec<usize> = (0..c.len()).collect();
    idx.sort_by_key(|&i| bucket(&c[i].0));
    idx
}

fn array_open(out: &mut Vec<Text>, prefix: &'static str) {
    out.push(Text::plain("["));
    out.push(suffix(prefix));
    out.push(Text::plain(";"));
}

/// `handleEscapePretty`: plain `[A-Za-z0-9._+-]+` keys are aqua, others quoted.
fn pretty_key(k: &str) -> Text {
    let simple = !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'));
    if simple {
        return Text::plain(k).color("aqua");
    }
    let quoted = quote_and_escape(k);
    let q = quoted.chars().next().unwrap_or('"').to_string();
    let inner = &quoted[1..quoted.len() - 1];
    Text::plain(q.clone()).push(Text::plain(inner).color("aqua")).push(Text::plain(q))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Tag {
        parse_str(s).unwrap_or_else(|e| panic!("{s:?}: {}", e.msg.plain_string()))
    }

    fn err(s: &str) -> CmdError {
        match parse_str(s) {
            Ok(t) => panic!("{s:?} parsed as {t:?}"),
            Err(e) => e,
        }
    }

    #[test]
    fn numbers_and_suffixes() {
        assert_eq!(p("1"), Tag::Int(1));
        assert_eq!(p("-7"), Tag::Int(-7));
        assert_eq!(p("1b"), Tag::Byte(1));
        assert_eq!(p("-1B"), Tag::Byte(-1));
        assert_eq!(p("2S"), Tag::Short(2));
        assert_eq!(p("3l"), Tag::Long(3));
        assert_eq!(p("1.5f"), Tag::Float(1.5));
        assert_eq!(p("2.0D"), Tag::Double(2.0));
        assert_eq!(p("1e3"), Tag::Double(1000.0));
        assert_eq!(p(".5"), Tag::Double(0.5));
        assert_eq!(p("1.f"), Tag::Float(1.0));
        assert_eq!(p("1e1f"), Tag::Float(10.0));
        assert_eq!(p("0x1F"), Tag::Int(31));
        assert_eq!(p("0xFFFFFFFF"), Tag::Int(-1));
        assert_eq!(p("0xFFs"), Tag::Short(255));
        assert_eq!(p("0b101"), Tag::Int(5));
        assert_eq!(p("0b"), Tag::Byte(0));
        assert_eq!(p("0"), Tag::Int(0));
        assert_eq!(p("1_000"), Tag::Int(1000));
        assert_eq!(p("10ub"), Tag::Byte(10));
        assert_eq!(p("200ub"), Tag::Byte(-56));
        assert_eq!(p("true"), Tag::Byte(1));
        assert_eq!(p("FALSE"), Tag::Byte(0));
    }

    #[test]
    fn strings_and_escapes() {
        assert_eq!(p("\"a\\nb\""), Tag::String("a\nb".into()));
        assert_eq!(p("'it\"s'"), Tag::String("it\"s".into()));
        assert_eq!(p("\"x\\\"y\""), Tag::String("x\"y".into()));
        assert_eq!(p("\"\\x41\\u0042\\U00000043\""), Tag::String("ABC".into()));
        assert_eq!(p("\"a\\sb\""), Tag::String("a b".into()));
        assert_eq!(p("'\\\\'"), Tag::String("\\".into()));
        assert_eq!(p("\"\\uD83D\\uDE00\""), Tag::String("\u{1F600}".into()));
        assert_eq!(p("''"), Tag::String(String::new()));
        assert_eq!(p("hello_world-1.x+"), Tag::String("hello_world-1.x+".into()));
    }

    #[test]
    fn lists_arrays_compounds() {
        assert_eq!(p("[1,2,3]"), Tag::List(vec![Tag::Int(1), Tag::Int(2), Tag::Int(3)]));
        assert_eq!(p("[1,]"), Tag::List(vec![Tag::Int(1)]));
        assert_eq!(p("[]"), Tag::List(vec![]));
        assert_eq!(p("[1,'a',{}]"), Tag::List(vec![Tag::Int(1), Tag::String("a".into()), Tag::Compound(vec![])]));
        assert_eq!(p("[B]"), Tag::List(vec![Tag::String("B".into())]));
        assert_eq!(p("[B;1b,2,0x3]"), Tag::ByteArray(vec![1, 2, 3]));
        assert_eq!(p("[B;-128b,255ub]"), Tag::ByteArray(vec![-128, -1]));
        assert_eq!(p("[B;]"), Tag::ByteArray(vec![]));
        assert_eq!(p("[I;1,2s,3b]"), Tag::IntArray(vec![1, 2, 3]));
        assert_eq!(p("[L;1,2b,3]"), Tag::LongArray(vec![1, 2, 3]));
        assert!(parse_str("[B;1s]").is_err());
        assert!(parse_str("[B;200b]").is_err());
        assert_eq!(p("{a:1,\"b c\":2}"), Tag::Compound(vec![("a".into(), Tag::Int(1)), ("b c".into(), Tag::Int(2))]));
        assert_eq!(p("{a:1,a:2}"), Tag::Compound(vec![("a".into(), Tag::Int(2))]));
        assert_eq!(p("{}"), Tag::Compound(vec![]));
        assert_eq!(p("{ 'k' : [ 1 , 2 ] }"), Tag::Compound(vec![("k".into(), Tag::List(vec![Tag::Int(1), Tag::Int(2)]))]));
    }

    #[test]
    fn builtins() {
        assert_eq!(p("bool(1)"), Tag::Byte(1));
        assert_eq!(p("bool(0.0)"), Tag::Byte(0));
        assert!(parse_str("bool('x')").is_err());
        assert!(parse_str("true()").is_err());
        assert_eq!(
            p("uuid(\"12345678-1234-5678-9abc-def012345678\")"),
            Tag::IntArray(vec![0x12345678, 0x12345678, 0x9abcdef0u32 as i32, 0x12345678])
        );
        // Java accepts short groups: most = 0x1_0002_0003, least = 0x4_0000_0000_0005.
        assert_eq!(p("uuid(\"1-2-3-4-5\")"), Tag::IntArray(vec![1, 0x20003, 0x40000, 5]));
    }

    #[test]
    fn trailing_and_cursor() {
        let mut r = Reader::new("1 x");
        assert_eq!(parse_tag(&mut r).unwrap(), Tag::Int(1));
        assert_eq!(r.cursor, 1);
        assert_eq!(err("1 x").cursor, 2);
        assert_eq!(err("1 x").msg.plain_string(), "argument.nbt.trailing");
        let mut r = Reader::new("{a:1} tail");
        assert!(parse_compound(&mut r).is_ok());
        assert_eq!(r.cursor, 5);
        assert_eq!(parse_compound(&mut Reader::new("[1]")).unwrap_err().msg.plain_string(), "argument.nbt.expected.compound");
    }

    #[test]
    fn error_cursors() {
        assert_eq!(err("1000b").cursor, 5);
        assert!(err("1000b").msg.plain_string().contains("number_parse_failure"));
        assert_eq!(err("01").cursor, 2);
        assert_eq!(err("0x").cursor, 2);
        assert_eq!(err("-1ub").cursor, 4);
        assert_eq!(err("\"abc").cursor, 4);
        assert_eq!(err("\"a\\q\"").cursor, 3);
        assert_eq!(err("[1 2]").cursor, 3);
        assert_eq!(err("foo(1)").cursor, 6);
        assert_eq!(err("uuid(\"nope\")").cursor, 12);
        assert!(err("uuid(\"nope\")").msg.plain_string().contains("expected_string_uuid"));
        assert!(err("{a:}").cursor >= 3);
        // The suffix probe after `1` stores at the same cursor first, so the
        // reported message is that probe's; the empty-key error is at 5 as well.
        assert_eq!(err("{\"\":1}").cursor, 5);
    }

    #[test]
    fn depth_limit() {
        std::thread::Builder::new()
            .stack_size(32 << 20)
            .spawn(|| {
                let ok = "[".repeat(512) + &"]".repeat(512);
                assert!(parse_str(&ok).is_ok());
                let bad = "[".repeat(513) + &"]".repeat(513);
                assert!(parse_str(&bad).is_err());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn snbt_printing() {
        assert_eq!(to_snbt(&p("{b:1,a:[1,2]}")), "{a:[1,2],b:1}");
        assert_eq!(to_snbt(&Tag::Byte(-1)), "-1b");
        assert_eq!(to_snbt(&Tag::Short(2)), "2s");
        assert_eq!(to_snbt(&Tag::Long(3)), "3L");
        assert_eq!(to_snbt(&Tag::Float(1.5)), "1.5f");
        assert_eq!(to_snbt(&Tag::Float(1.0)), "1.0f");
        assert_eq!(to_snbt(&Tag::Double(2.0)), "2.0d");
        assert_eq!(to_snbt(&Tag::ByteArray(vec![1, -2])), "[B;1B,-2B]");
        assert_eq!(to_snbt(&Tag::IntArray(vec![])), "[I;]");
        assert_eq!(to_snbt(&Tag::LongArray(vec![5])), "[L;5L]");
        assert_eq!(to_snbt(&Tag::List(vec![])), "[]");
        assert_eq!(to_snbt(&Tag::String("a\"b".into())), "'a\"b'");
        assert_eq!(to_snbt(&Tag::String("it's".into())), "\"it's\"");
        assert_eq!(to_snbt(&Tag::String("tab\there".into())), "\"tab\\there\"");
        assert_eq!(to_snbt(&Tag::String("\u{1}".into())), "\"\\x01\"");
        let c = Tag::Compound(vec![("a b".into(), Tag::Int(1)), ("true".into(), Tag::Int(2))]);
        assert_eq!(to_snbt(&c), "{\"a b\":1,\"true\":2}");
    }

    #[test]
    fn snbt_round_trip() {
        for s in ["{a:1,b:[1b,2s,3L,4.5f,6.0d,\"x y\"],c:[B;1B,2B],d:[I;3,4],e:[L;5L]}", "[]", "{}", "[{x:1},{y:2}]"] {
            let t = p(s);
            let printed = to_snbt(&t);
            assert_eq!(to_snbt(&p(&printed)), printed);
        }
    }

    #[test]
    fn java_hash_order_matches_vanilla() {
        // `/data get block` of a chest on a vanilla 26.3 server; keys in the
        // order `saveWithFullMetadata` puts them (data first, then id, x, y, z)
        let keys = ["components", "Items", "id", "x", "y", "z"];
        let c: Vec<(String, Tag)> = keys.iter().map(|k| (k.to_string(), Tag::Int(0))).collect();
        let order: Vec<&str> = java_hash_order(&c).into_iter().map(|i| keys[i]).collect();
        assert_eq!(order, ["components", "x", "y", "Items", "z", "id"]);
        let keys = ["count", "Slot", "id", "components"];
        let c: Vec<(String, Tag)> = keys.iter().map(|k| (k.to_string(), Tag::Int(0))).collect();
        let order: Vec<&str> = java_hash_order(&c).into_iter().map(|i| keys[i]).collect();
        assert_eq!(order, ["components", "count", "Slot", "id"]);
    }

    #[test]
    fn pretty_text() {
        assert_eq!(to_pretty_text(&Tag::Int(5)).plain_string(), "5");
        assert_eq!(to_pretty_text(&Tag::Byte(1)).plain_string(), "1b");
        assert_eq!(to_pretty_text(&Tag::Compound(vec![("a".into(), Tag::String("x".into()))])).plain_string(), "{a: \"x\"}");
        assert_eq!(to_pretty_text(&Tag::ByteArray(vec![1, 2])).plain_string(), "[B; 1b, 2b]");
        assert_eq!(to_pretty_text(&Tag::IntArray(vec![])).plain_string(), "[I;]");
        assert_eq!(to_pretty_text(&Tag::List(vec![Tag::Int(1), Tag::Int(2)])).plain_string(), "[1, 2]");
        assert_eq!(to_pretty_text(&Tag::Compound(vec![("a b".into(), Tag::Int(1))])).plain_string(), "{\"a b\": 1}");
    }
}
