//! Brigadier `StringReader`, same semantics and errors. The cursor is a
//! byte offset; it only ever stops on char boundaries.

use crate::error::{builtin, CmdResult};
use std::sync::Arc;

pub const SYNTAX_ESCAPE: char = '\\';
pub const SYNTAX_DOUBLE_QUOTE: char = '"';
pub const SYNTAX_SINGLE_QUOTE: char = '\'';

#[derive(Clone, Debug)]
pub struct Reader {
    input: Arc<str>,
    pub cursor: usize,
}

impl Reader {
    pub fn new(s: &str) -> Self {
        Self { input: Arc::from(s), cursor: 0 }
    }
    pub fn from_arc(s: Arc<str>) -> Self {
        Self { input: s, cursor: 0 }
    }
    pub fn input(&self) -> &str {
        &self.input
    }
    pub fn input_arc(&self) -> Arc<str> {
        self.input.clone()
    }
    pub fn remaining(&self) -> &str {
        &self.input[self.cursor..]
    }
    pub fn remaining_len(&self) -> usize {
        self.input.len() - self.cursor
    }
    pub fn total_len(&self) -> usize {
        self.input.len()
    }
    pub fn read_so_far(&self) -> &str {
        &self.input[..self.cursor]
    }
    pub fn slice(&self, from: usize, to: usize) -> &str {
        &self.input[from..to]
    }
    /// At least `n` more chars (Brigadier `canRead(int)`).
    pub fn can_read_n(&self, n: usize) -> bool {
        self.remaining().chars().take(n).count() >= n
    }
    #[inline]
    pub fn can_read(&self) -> bool {
        self.cursor < self.input.len()
    }
    #[inline]
    pub fn peek(&self) -> char {
        self.remaining().chars().next().unwrap_or('\0')
    }
    /// Char `offset` chars ahead of the cursor.
    pub fn peek_at(&self, offset: usize) -> char {
        self.remaining().chars().nth(offset).unwrap_or('\0')
    }
    #[inline]
    pub fn read(&mut self) -> char {
        let c = self.peek();
        self.cursor += c.len_utf8();
        c
    }
    #[inline]
    pub fn skip(&mut self) {
        let c = self.peek();
        self.cursor += c.len_utf8().max(1).min(self.remaining_len());
    }
    pub fn skip_whitespace(&mut self) {
        while self.can_read() && is_java_whitespace(self.peek()) {
            self.skip();
        }
    }

    pub fn is_allowed_number(c: char) -> bool {
        c.is_ascii_digit() || c == '.' || c == '-'
    }
    pub fn is_quoted_string_start(c: char) -> bool {
        c == SYNTAX_DOUBLE_QUOTE || c == SYNTAX_SINGLE_QUOTE
    }
    pub fn is_allowed_in_unquoted_string(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '+'
    }

    fn read_number_str(&mut self) -> &str {
        let start = self.cursor;
        while self.can_read() && Self::is_allowed_number(self.peek()) {
            self.skip();
        }
        &self.input[start..self.cursor]
    }

    pub fn read_int(&mut self) -> CmdResult<i32> {
        let start = self.cursor;
        let s = self.read_number_str().to_string();
        if s.is_empty() {
            return Err(builtin::expected_int(self));
        }
        match s.parse::<i32>() {
            Ok(v) => Ok(v),
            Err(_) => {
                self.cursor = start;
                Err(builtin::invalid_int(self, &s))
            }
        }
    }
    pub fn read_long(&mut self) -> CmdResult<i64> {
        let start = self.cursor;
        let s = self.read_number_str().to_string();
        if s.is_empty() {
            return Err(builtin::expected_long(self));
        }
        match s.parse::<i64>() {
            Ok(v) => Ok(v),
            Err(_) => {
                self.cursor = start;
                Err(builtin::invalid_long(self, &s))
            }
        }
    }
    pub fn read_double(&mut self) -> CmdResult<f64> {
        let start = self.cursor;
        let s = self.read_number_str().to_string();
        if s.is_empty() {
            return Err(builtin::expected_double(self));
        }
        match parse_java_float(&s) {
            Some(v) => Ok(v),
            None => {
                self.cursor = start;
                Err(builtin::invalid_double(self, &s))
            }
        }
    }
    pub fn read_float(&mut self) -> CmdResult<f32> {
        let start = self.cursor;
        let s = self.read_number_str().to_string();
        if s.is_empty() {
            return Err(builtin::expected_float(self));
        }
        match parse_java_float(&s).and_then(|_| s.parse::<f32>().ok()) {
            Some(v) => Ok(v),
            None => {
                self.cursor = start;
                Err(builtin::invalid_float(self, &s))
            }
        }
    }

    pub fn read_unquoted_string(&mut self) -> &str {
        let start = self.cursor;
        while self.can_read() && Self::is_allowed_in_unquoted_string(self.peek()) {
            self.skip();
        }
        &self.input[start..self.cursor]
    }

    pub fn read_quoted_string(&mut self) -> CmdResult<String> {
        if !self.can_read() {
            return Ok(String::new());
        }
        let next = self.peek();
        if !Self::is_quoted_string_start(next) {
            return Err(builtin::expected_start_of_quote(self));
        }
        self.skip();
        self.read_string_until(next)
    }

    pub fn read_string_until(&mut self, terminator: char) -> CmdResult<String> {
        let mut out = String::new();
        let mut escaped = false;
        while self.can_read() {
            let c = self.read();
            if escaped {
                if c == terminator || c == SYNTAX_ESCAPE {
                    out.push(c);
                    escaped = false;
                } else {
                    self.cursor -= c.len_utf8();
                    return Err(builtin::invalid_escape(self, c));
                }
            } else if c == SYNTAX_ESCAPE {
                escaped = true;
            } else if c == terminator {
                return Ok(out);
            } else {
                out.push(c);
            }
        }
        Err(builtin::expected_end_of_quote(self))
    }

    pub fn read_string(&mut self) -> CmdResult<String> {
        if !self.can_read() {
            return Ok(String::new());
        }
        let next = self.peek();
        if Self::is_quoted_string_start(next) {
            self.skip();
            return self.read_string_until(next);
        }
        Ok(self.read_unquoted_string().to_string())
    }

    pub fn read_boolean(&mut self) -> CmdResult<bool> {
        let start = self.cursor;
        let v = self.read_string()?;
        if v.is_empty() {
            return Err(builtin::expected_bool(self));
        }
        match v.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => {
                self.cursor = start;
                Err(builtin::invalid_bool(self, &v))
            }
        }
    }

    pub fn expect(&mut self, c: char) -> CmdResult<()> {
        if !self.can_read() || self.peek() != c {
            return Err(builtin::expected_symbol(self, &c.to_string()));
        }
        self.skip();
        Ok(())
    }
}

/// Java `Character.isWhitespace` for the chars that matter.
pub fn is_java_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | '\u{1C}'..='\u{1F}')
        || (c.is_whitespace() && c != '\u{A0}' && c != '\u{2007}' && c != '\u{202F}')
}

/// `Double.parseDouble` restricted to the chars `[0-9.-]` the reader
/// collects: at most one leading '-', digits with one optional '.', at
/// least one digit.
pub fn parse_java_float(s: &str) -> Option<f64> {
    let body = s.strip_prefix('-').unwrap_or(s);
    if body.is_empty() || body.contains('-') || body.matches('.').count() > 1 || !body.bytes().any(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbers() {
        assert_eq!(Reader::new("12 x").read_int().unwrap(), 12);
        assert!(Reader::new("1.5").read_int().is_err());
        assert_eq!(Reader::new("-.5").read_double().unwrap(), -0.5);
        assert_eq!(Reader::new("1.").read_double().unwrap(), 1.0);
        assert!(Reader::new("-").read_double().is_err());
        assert!(Reader::new("1-2").read_double().is_err());
        let mut r = Reader::new("abc");
        assert!(r.read_int().is_err());
        assert_eq!(r.cursor, 0);
    }
    #[test]
    fn strings() {
        assert_eq!(Reader::new("\"a\\\"b\" c").read_string().unwrap(), "a\"b");
        assert_eq!(Reader::new("'x y'").read_string().unwrap(), "x y");
        assert_eq!(Reader::new("foo.bar+ z").read_string().unwrap(), "foo.bar+");
        assert!(Reader::new("\"abc").read_string().is_err());
        assert!(Reader::new("yes").read_boolean().is_err());
    }
}
