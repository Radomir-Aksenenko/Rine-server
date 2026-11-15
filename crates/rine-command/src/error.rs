//! Command syntax errors (Brigadier `CommandSyntaxException`) with the
//! vanilla translation keys from `BrigadierExceptions.java`.

use crate::reader::Reader;
use rine_protocol::text::Text;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct CmdError {
    /// The raw message (vanilla `getRawMessage()`), shown red.
    pub msg: Text,
    /// Input and cursor (byte offset) when created with context.
    pub input: Option<Arc<str>>,
    pub cursor: usize,
}

pub type CmdResult<T> = Result<T, CmdError>;

impl CmdError {
    /// Error without input context (`create()`).
    pub fn new(msg: Text) -> Self {
        Self { msg, input: None, cursor: 0 }
    }
    /// Error with the reader's input and current cursor (`createWithContext`).
    pub fn at(msg: Text, r: &Reader) -> Self {
        Self { msg, input: Some(r.input_arc()), cursor: r.cursor }
    }
    pub fn at_cursor(msg: Text, r: &Reader, cursor: usize) -> Self {
        Self { msg, input: Some(r.input_arc()), cursor }
    }
    /// Translatable error without context.
    pub fn tr(key: &'static str, with: Vec<Text>) -> Self {
        Self::new(Text::tr(key, with))
    }
    pub fn key(key: &'static str) -> Self {
        Self::new(Text::key(key))
    }
}

/// `translatableEscape` argument: numbers, strings and booleans are shown
/// verbatim; this builds the literal component for them.
pub fn lit(v: impl ToString) -> Text {
    Text::plain(v.to_string())
}

/// Java `String.valueOf(float)` / `Double.toString` formatting: shortest
/// repr, always with a fractional part, exponent form outside 1e-3..1e7.
pub fn java_double(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    let a = v.abs();
    if (1e-3..1e7).contains(&a) {
        let s = format!("{v}");
        if s.contains('.') {
            s
        } else {
            s + ".0"
        }
    } else {
        // Java: d.dddE[-]n
        let s = format!("{v:e}"); // e.g. 1.5e7, -2e-5
        let (m, e) = s.split_once('e').unwrap();
        let m = if m.contains('.') { m.to_string() } else { format!("{m}.0") };
        format!("{m}E{e}")
    }
}

pub fn java_float(v: f32) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    let a = v.abs();
    if (1e-3..1e7).contains(&a) {
        let s = format!("{v}");
        if s.contains('.') {
            s
        } else {
            s + ".0"
        }
    } else {
        let s = format!("{v:e}");
        let (m, e) = s.split_once('e').unwrap();
        let m = if m.contains('.') { m.to_string() } else { format!("{m}.0") };
        format!("{m}E{e}")
    }
}

/// Brigadier built-in exceptions.
pub mod builtin {
    use super::*;

    pub fn double_too_low(r: &Reader, found: f64, min: f64) -> CmdError {
        CmdError::at(Text::tr("argument.double.low", vec![lit(java_double(min)), lit(java_double(found))]), r)
    }
    pub fn double_too_high(r: &Reader, found: f64, max: f64) -> CmdError {
        CmdError::at(Text::tr("argument.double.big", vec![lit(java_double(max)), lit(java_double(found))]), r)
    }
    pub fn float_too_low(r: &Reader, found: f32, min: f32) -> CmdError {
        CmdError::at(Text::tr("argument.float.low", vec![lit(java_float(min)), lit(java_float(found))]), r)
    }
    pub fn float_too_high(r: &Reader, found: f32, max: f32) -> CmdError {
        CmdError::at(Text::tr("argument.float.big", vec![lit(java_float(max)), lit(java_float(found))]), r)
    }
    pub fn integer_too_low(r: &Reader, found: i32, min: i32) -> CmdError {
        CmdError::at(Text::tr("argument.integer.low", vec![lit(min), lit(found)]), r)
    }
    pub fn integer_too_high(r: &Reader, found: i32, max: i32) -> CmdError {
        CmdError::at(Text::tr("argument.integer.big", vec![lit(max), lit(found)]), r)
    }
    pub fn long_too_low(r: &Reader, found: i64, min: i64) -> CmdError {
        CmdError::at(Text::tr("argument.long.low", vec![lit(min), lit(found)]), r)
    }
    pub fn long_too_high(r: &Reader, found: i64, max: i64) -> CmdError {
        CmdError::at(Text::tr("argument.long.big", vec![lit(max), lit(found)]), r)
    }
    pub fn literal_incorrect(r: &Reader, expected: &str) -> CmdError {
        CmdError::at(Text::tr("argument.literal.incorrect", vec![lit(expected)]), r)
    }
    pub fn expected_start_of_quote(r: &Reader) -> CmdError {
        CmdError::at(Text::key("parsing.quote.expected.start"), r)
    }
    pub fn expected_end_of_quote(r: &Reader) -> CmdError {
        CmdError::at(Text::key("parsing.quote.expected.end"), r)
    }
    pub fn invalid_escape(r: &Reader, c: char) -> CmdError {
        CmdError::at(Text::tr("parsing.quote.escape", vec![lit(c)]), r)
    }
    pub fn invalid_bool(r: &Reader, v: &str) -> CmdError {
        CmdError::at(Text::tr("parsing.bool.invalid", vec![lit(v)]), r)
    }
    pub fn invalid_int(r: &Reader, v: &str) -> CmdError {
        CmdError::at(Text::tr("parsing.int.invalid", vec![lit(v)]), r)
    }
    pub fn expected_int(r: &Reader) -> CmdError {
        CmdError::at(Text::key("parsing.int.expected"), r)
    }
    pub fn invalid_long(r: &Reader, v: &str) -> CmdError {
        CmdError::at(Text::tr("parsing.long.invalid", vec![lit(v)]), r)
    }
    pub fn expected_long(r: &Reader) -> CmdError {
        CmdError::at(Text::key("parsing.long.expected"), r)
    }
    pub fn invalid_double(r: &Reader, v: &str) -> CmdError {
        CmdError::at(Text::tr("parsing.double.invalid", vec![lit(v)]), r)
    }
    pub fn expected_double(r: &Reader) -> CmdError {
        CmdError::at(Text::key("parsing.double.expected"), r)
    }
    pub fn invalid_float(r: &Reader, v: &str) -> CmdError {
        CmdError::at(Text::tr("parsing.float.invalid", vec![lit(v)]), r)
    }
    pub fn expected_float(r: &Reader) -> CmdError {
        CmdError::at(Text::key("parsing.float.expected"), r)
    }
    pub fn expected_bool(r: &Reader) -> CmdError {
        CmdError::at(Text::key("parsing.bool.expected"), r)
    }
    pub fn expected_symbol(r: &Reader, sym: &str) -> CmdError {
        CmdError::at(Text::tr("parsing.expected", vec![lit(sym)]), r)
    }
    pub fn unknown_command(r: &Reader) -> CmdError {
        CmdError::at(Text::key("command.unknown.command"), r)
    }
    pub fn unknown_argument(r: &Reader) -> CmdError {
        CmdError::at(Text::key("command.unknown.argument"), r)
    }
    pub fn expected_separator(r: &Reader) -> CmdError {
        CmdError::at(Text::key("command.expected.separator"), r)
    }
    pub fn parse_exception(r: &Reader, msg: &str) -> CmdError {
        CmdError::at(Text::tr("command.exception", vec![lit(msg)]), r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn java_number_format() {
        assert_eq!(java_double(1.0), "1.0");
        assert_eq!(java_double(0.5), "0.5");
        assert_eq!(java_double(-3.25), "-3.25");
        assert_eq!(java_double(1e7), "1.0E7");
        assert_eq!(java_double(1.5e-5), "1.5E-5");
        assert_eq!(java_float(1.1), "1.1");
        assert_eq!(java_float(3.0e10), "3.0E10");
    }
}
