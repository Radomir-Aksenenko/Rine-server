//! `component` and `style` arguments: SNBT checked against the text
//! component / style codecs (`ComponentArgument`, `StyleArgument`,
//! `ComponentSerialization`, `Style.Serializer`). Resolution (selectors,
//! scores, nbt) is done by the server.
//!
//! The checks follow the codecs field by field: required fields, field
//! types, named or hex colours, click and hover action shapes. Deviations
//! are listed next to the code: selector, nbt path and item template
//! syntax are only checked as strings, and the error texts are not the
//! vanilla codec messages.

use super::block::parse_id;
use crate::error::{lit, CmdError, CmdResult};
use crate::reader::Reader;
use rine_protocol::nbt::Tag;
use rine_protocol::text::Text;

type Fields = [(String, Tag)];

const NAMED_COLORS: [&str; 16] = [
    "black", "dark_blue", "dark_green", "dark_aqua", "dark_red", "dark_purple", "gold", "gray", "dark_gray", "blue", "green", "aqua", "red",
    "light_purple", "yellow", "white",
];

/// `component`: one SNBT tag that is a well-formed text component.
pub fn parse_component(r: &mut Reader) -> CmdResult<Tag> {
    let start = r.cursor;
    let tag = super::snbt::parse_tag(r)?;
    if let Err(msg) = validate_component(&tag) {
        return Err(CmdError::at_cursor(Text::tr("argument.component.invalid", vec![lit(msg)]), r, start));
    }
    Ok(tag)
}

/// `style`: one SNBT tag that is a valid `Style` compound.
pub fn parse_style(r: &mut Reader) -> CmdResult<Tag> {
    let start = r.cursor;
    let tag = super::snbt::parse_tag(r)?;
    let res = match &tag {
        Tag::Compound(f) => validate_style(f),
        _ => Err("Not a map".to_string()),
    };
    if let Err(msg) = res {
        return Err(CmdError::at_cursor(Text::tr("argument.style.invalid", vec![lit(msg)]), r, start));
    }
    Ok(tag)
}

/// `ComponentSerialization.CODEC`: a string, a non-empty list of
/// components, or a compound (contents, `extra`, style).
pub fn validate_component(t: &Tag) -> Result<(), String> {
    match t {
        Tag::String(_) => Ok(()),
        Tag::List(items) => {
            if items.is_empty() {
                return Err("Empty component list".into());
            }
            items.iter().try_for_each(validate_component)
        }
        Tag::Compound(f) => {
            validate_contents(f)?;
            validate_extra(f)?;
            validate_style(f)
        }
        _ => Err("Not a valid text component".into()),
    }
}

fn get<'a>(f: &'a Fields, key: &str) -> Option<&'a Tag> {
    f.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn as_str(t: &Tag) -> Option<&str> {
    match t {
        Tag::String(s) => Some(s),
        _ => None,
    }
}

fn is_numeric(t: &Tag) -> bool {
    matches!(t, Tag::Byte(_) | Tag::Short(_) | Tag::Int(_) | Tag::Long(_) | Tag::Float(_) | Tag::Double(_))
}

/// Integer value of an integral tag (`POSITIVE_INT` style checks).
fn as_integer(t: &Tag) -> Option<i64> {
    match t {
        Tag::Byte(v) => Some(*v as i64),
        Tag::Short(v) => Some(*v as i64),
        Tag::Int(v) => Some(*v as i64),
        Tag::Long(v) => Some(*v),
        _ => None,
    }
}

/// A required string field.
fn req_str(f: &Fields, key: &str) -> Result<(), String> {
    match get(f, key).and_then(as_str) {
        Some(_) => Ok(()),
        None => Err(format!("'{key}' is missing or not a string")),
    }
}

/// The content of a component: `type` picks one, otherwise the first
/// content codec that accepts the compound wins (`FuzzyCodec`).
fn validate_contents(f: &Fields) -> Result<(), String> {
    if let Some(ty) = get(f, "type") {
        let ty = as_str(ty).ok_or_else(|| "'type' is not a string".to_string())?;
        return match ty {
            "text" => content_text(f),
            "translatable" => content_translatable(f),
            "keybind" => content_keybind(f),
            "score" => content_score(f),
            "selector" => content_selector(f),
            "nbt" => content_nbt(f),
            "object" => content_object(f),
            other => Err(format!("Unknown component type '{other}'")),
        };
    }
    let checks: [fn(&Fields) -> Result<(), String>; 7] =
        [content_text, content_translatable, content_keybind, content_score, content_selector, content_nbt, content_object];
    if checks.iter().any(|check| check(f).is_ok()) {
        Ok(())
    } else {
        Err("No matching codec found".into())
    }
}

fn content_text(f: &Fields) -> Result<(), String> {
    req_str(f, "text")
}

fn content_translatable(f: &Fields) -> Result<(), String> {
    req_str(f, "translate")?;
    // `fallback` is lenient in vanilla: a wrong type is ignored.
    match get(f, "with") {
        None => Ok(()),
        Some(Tag::List(args)) => args.iter().try_for_each(|a| match a {
            Tag::String(_) => Ok(()),
            t if is_numeric(t) => Ok(()),
            other => validate_component(other),
        }),
        Some(_) => Err("'with' is not a list".into()),
    }
}

fn content_keybind(f: &Fields) -> Result<(), String> {
    req_str(f, "keybind")
}

fn content_score(f: &Fields) -> Result<(), String> {
    match get(f, "score") {
        Some(Tag::Compound(s)) => {
            req_str(s, "name")?;
            req_str(s, "objective")
        }
        _ => Err("'score' is missing or not a compound".into()),
    }
}

/// Deviation: the selector string is not parsed.
fn content_selector(f: &Fields) -> Result<(), String> {
    req_str(f, "selector")?;
    validate_optional_component(f, "separator")
}

/// `nbt` contents. Deviation: the nbt path string is not parsed.
fn content_nbt(f: &Fields) -> Result<(), String> {
    req_str(f, "nbt")?;
    // interpret/plain/separator are lenient in vanilla: bad values are dropped.
    let flag = |k: &str| get(f, k).is_some_and(|t| matches!(t, Tag::Byte(v) if *v != 0) || matches!(t, Tag::Int(v) if *v != 0));
    if flag("interpret") && flag("plain") {
        return Err("'interpret' and 'plain' flags can't be both on".into());
    }
    match get(f, "source").and_then(as_str) {
        Some("block") => req_str(f, "block"),
        Some("entity") => req_str(f, "entity"),
        Some("storage") => req_str(f, "storage"),
        Some(other) => Err(format!("Unknown nbt source '{other}'")),
        None => {
            if ["block", "entity", "storage"].iter().any(|k| req_str(f, k).is_ok()) {
                Ok(())
            } else {
                Err("No nbt source (block, entity or storage)".into())
            }
        }
    }
}

/// `object` contents: `atlas` (sprite id) or `player`, plus `fallback`.
fn content_object(f: &Fields) -> Result<(), String> {
    match get(f, "object").and_then(as_str) {
        Some("atlas") => req_str(f, "sprite")?,
        Some("player") => {
            if get(f, "player").is_none() {
                return Err("'player' is missing".into());
            }
        }
        Some(other) => return Err(format!("Unknown object type '{other}'")),
        None => {
            if req_str(f, "sprite").is_err() && get(f, "player").is_none() {
                return Err("No object type".into());
            }
        }
    }
    validate_optional_component(f, "fallback")
}

/// A component field that is parsed strictly (`optionalFieldOf`).
fn validate_optional_component(f: &Fields, key: &str) -> Result<(), String> {
    match get(f, key) {
        None => Ok(()),
        Some(t) => validate_component(t),
    }
}

fn validate_extra(f: &Fields) -> Result<(), String> {
    match get(f, "extra") {
        None => Ok(()),
        Some(Tag::List(items)) if !items.is_empty() => items.iter().try_for_each(validate_component),
        Some(Tag::List(_)) => Err("'extra' is an empty list".into()),
        Some(_) => Err("'extra' is not a list".into()),
    }
}

/// `Style.Serializer.MAP_CODEC`: every field is strict when present.
fn validate_style(f: &Fields) -> Result<(), String> {
    if let Some(t) = get(f, "color") {
        let s = as_str(t).ok_or_else(|| "'color' is not a string".to_string())?;
        if !valid_color(s) {
            return Err(format!("Invalid color '{s}'"));
        }
    }
    if let Some(t) = get(f, "shadow_color") {
        let ok = is_numeric(t) || matches!(t, Tag::List(l) if l.len() == 4 && l.iter().all(is_numeric));
        if !ok {
            return Err("'shadow_color' is not a color".into());
        }
    }
    for key in ["bold", "italic", "underlined", "strikethrough", "obfuscated"] {
        if let Some(t) = get(f, key) {
            if !is_numeric(t) {
                return Err(format!("'{key}' is not a boolean"));
            }
        }
    }
    if let Some(t) = get(f, "click_event") {
        validate_click(t)?;
    }
    if let Some(t) = get(f, "hover_event") {
        validate_hover(t)?;
    }
    if let Some(t) = get(f, "insertion") {
        if as_str(t).is_none() {
            return Err("'insertion' is not a string".into());
        }
    }
    if let Some(t) = get(f, "font") {
        match t {
            // Deviation: the compound font form is not checked.
            Tag::Compound(_) => {}
            Tag::String(s) if parse_id(s).is_some() => {}
            _ => return Err("'font' is not a valid identifier".into()),
        }
    }
    Ok(())
}

/// `TextColor.parseColor`: a colour name or `#rrggbb`.
fn valid_color(s: &str) -> bool {
    match s.strip_prefix('#') {
        Some(hex) => u32::from_str_radix(hex, 16).is_ok_and(|v| v <= 0xFF_FFFF),
        None => NAMED_COLORS.contains(&s),
    }
}

/// `ClickEvent.CODEC`, dispatched on `action`.
fn validate_click(t: &Tag) -> Result<(), String> {
    let Tag::Compound(f) = t else {
        return Err("'click_event' is not a compound".into());
    };
    let action = get(f, "action").and_then(as_str).ok_or_else(|| "'click_event' has no action".to_string())?;
    match action {
        "open_url" => req_str(f, "url"),
        // Not allowed from the server (`allowFromServer` is false).
        "open_file" => Err("Action 'open_file' is not allowed".into()),
        "run_command" | "suggest_command" => req_str(f, "command"),
        "change_page" => match get(f, "page").and_then(as_integer) {
            Some(p) if p >= 1 => Ok(()),
            _ => Err("'page' is not a positive integer".into()),
        },
        "copy_to_clipboard" => req_str(f, "value"),
        // Deviation: the dialog reference is accepted without checks.
        "show_dialog" => get(f, "dialog").map(|_| ()).ok_or_else(|| "'dialog' is missing".to_string()),
        "custom" => req_str(f, "id"),
        other => Err(format!("Unknown click action '{other}'")),
    }
}

/// `HoverEvent.CODEC`, dispatched on `action`.
fn validate_hover(t: &Tag) -> Result<(), String> {
    let Tag::Compound(f) = t else {
        return Err("'hover_event' is not a compound".into());
    };
    let action = get(f, "action").and_then(as_str).ok_or_else(|| "'hover_event' has no action".to_string())?;
    match action {
        "show_text" => match get(f, "value") {
            Some(v) => validate_component(v),
            None => Err("'value' is missing".into()),
        },
        // Deviation: the item template is only checked for its id.
        "show_item" => req_str(f, "id"),
        "show_entity" => {
            req_str(f, "id")?;
            if get(f, "uuid").is_none() {
                return Err("'uuid' is missing".into());
            }
            validate_optional_component(f, "name")
        }
        other => Err(format!("Unknown hover action '{other}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(fields: Vec<(&str, Tag)>) -> Tag {
        Tag::Compound(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn s(v: &str) -> Tag {
        Tag::String(v.to_string())
    }

    fn ok(t: Tag) {
        if let Err(e) = validate_component(&t) {
            panic!("{t:?} should be valid: {e}");
        }
    }

    fn bad(t: Tag) {
        assert!(validate_component(&t).is_err(), "{t:?} should be invalid");
    }

    #[test]
    fn roots() {
        ok(s(""));
        ok(Tag::List(vec![s("a"), c(vec![("text", s("b"))])]));
        bad(Tag::List(vec![]));
        bad(Tag::Int(3));
        bad(c(vec![]));
        bad(c(vec![("bold", Tag::Byte(1))]));
    }

    #[test]
    fn contents() {
        ok(c(vec![("text", s("hi"))]));
        bad(c(vec![("text", Tag::Int(5))]));
        ok(c(vec![("translate", s("a.b")), ("with", Tag::List(vec![s("x"), Tag::Int(3), c(vec![("text", s("y"))])]))]));
        bad(c(vec![("translate", Tag::Int(1))]));
        bad(c(vec![("translate", s("a")), ("with", s("not a list"))]));
        ok(c(vec![("translate", s("a")), ("fallback", Tag::Int(1))]));
        ok(c(vec![("keybind", s("key.jump"))]));
        ok(c(vec![("score", c(vec![("name", s("a")), ("objective", s("b"))]))]));
        bad(c(vec![("score", c(vec![("name", s("a"))]))]));
        ok(c(vec![("selector", s("@a")), ("separator", s(", "))]));
        ok(c(vec![("nbt", s("x")), ("storage", s("minecraft:foo"))]));
        bad(c(vec![("nbt", s("x"))]));
        bad(c(vec![("nbt", s("x")), ("block", s("~ ~ ~")), ("interpret", Tag::Byte(1)), ("plain", Tag::Byte(1))]));
        ok(c(vec![("object", s("atlas")), ("sprite", s("minecraft:item/stone"))]));
        ok(c(vec![("sprite", s("minecraft:item/stone"))]));
        bad(c(vec![("object", s("nope"))]));
    }

    #[test]
    fn explicit_type() {
        ok(c(vec![("type", s("text")), ("text", s("a"))]));
        bad(c(vec![("type", s("text")), ("translate", s("a"))]));
        bad(c(vec![("type", s("bogus")), ("text", s("a"))]));
        bad(c(vec![("type", Tag::Int(1)), ("text", s("a"))]));
    }

    #[test]
    fn extra_and_style() {
        ok(c(vec![("text", s("a")), ("extra", Tag::List(vec![s("b")]))]));
        bad(c(vec![("text", s("a")), ("extra", Tag::List(vec![]))]));
        bad(c(vec![("text", s("a")), ("extra", s("b"))]));
        ok(c(vec![("text", s("a")), ("color", s("red")), ("bold", Tag::Byte(1)), ("insertion", s("x"))]));
        ok(c(vec![("text", s("a")), ("color", s("#ff00aa"))]));
        bad(c(vec![("text", s("a")), ("color", s("#ff00zz"))]));
        bad(c(vec![("text", s("a")), ("color", s("nope"))]));
        bad(c(vec![("text", s("a")), ("color", s("#1000000"))]));
        bad(c(vec![("text", s("a")), ("bold", s("yes"))]));
        ok(c(vec![("text", s("a")), ("font", s("minecraft:default"))]));
        bad(c(vec![("text", s("a")), ("font", s("Bad Font"))]));
        ok(c(vec![("text", s("a")), ("shadow_color", Tag::Int(-1))]));
    }

    #[test]
    fn events() {
        let click = |f: Vec<(&str, Tag)>| c(vec![("text", s("a")), ("click_event", c(f))]);
        ok(click(vec![("action", s("open_url")), ("url", s("https://example.com"))]));
        ok(click(vec![("action", s("run_command")), ("command", s("/list"))]));
        ok(click(vec![("action", s("change_page")), ("page", Tag::Int(2))]));
        bad(click(vec![("action", s("change_page")), ("page", Tag::Int(0))]));
        bad(click(vec![("action", s("open_file")), ("path", s("/tmp"))]));
        bad(click(vec![("action", s("explode")), ("command", s("/list"))]));
        bad(click(vec![("command", s("/list"))]));

        let hover = |f: Vec<(&str, Tag)>| c(vec![("text", s("a")), ("hover_event", c(f))]);
        ok(hover(vec![("action", s("show_text")), ("value", s("tip"))]));
        bad(hover(vec![("action", s("show_text"))]));
        ok(hover(vec![("action", s("show_entity")), ("id", s("minecraft:pig")), ("uuid", Tag::IntArray(vec![0, 0, 0, 0]))]));
        bad(hover(vec![("action", s("show_entity")), ("id", s("minecraft:pig"))]));
    }

    #[test]
    fn style_argument_shape() {
        assert!(validate_style(&[("color".to_string(), s("gold"))]).is_ok());
        assert!(validate_style(&[]).is_ok());
        assert!(validate_style(&[("color".to_string(), Tag::Int(1))]).is_err());
    }
}
