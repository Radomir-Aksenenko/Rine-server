//! Text components. On the network (play/config) components are NBT; in the
//! status response and login disconnect they are JSON.
//!
//! The model mirrors vanilla `Component`: one content (literal text,
//! translatable key with arguments, keybind, or any other content passed
//! through as raw NBT fields), a style, and `extra` siblings.

use crate::nbt::Tag;
use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Text,
    Translate { key: Cow<'static, str>, fallback: Option<String>, with: Vec<Text> },
    Keybind(String),
    /// Content fields copied verbatim (object/sprite, unresolved nbt...).
    Raw(Vec<(String, Tag)>),
}

impl Default for Content {
    fn default() -> Self {
        Content::Text
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Text {
    pub text: String,
    pub content: Content,
    pub color: Option<Cow<'static, str>>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underlined: Option<bool>,
    pub strikethrough: Option<bool>,
    pub obfuscated: Option<bool>,
    pub font: Option<String>,
    pub insertion: Option<String>,
    pub shadow_color: Option<i32>,
    /// `click_event` compound (see `click_*` helpers).
    pub click: Option<Box<Tag>>,
    /// `hover_event` compound (see `hover_*` helpers).
    pub hover: Option<Box<Tag>>,
    pub extra: Vec<Text>,
}

impl Text {
    pub fn plain(s: impl Into<String>) -> Self {
        Self { text: s.into(), ..Default::default() }
    }
    pub fn empty() -> Self {
        Self::default()
    }
    /// Translatable component; arguments are components.
    pub fn tr(key: impl Into<Cow<'static, str>>, with: Vec<Text>) -> Self {
        Self { content: Content::Translate { key: key.into(), fallback: None, with }, ..Default::default() }
    }
    /// Translatable component without arguments.
    pub fn key(key: impl Into<Cow<'static, str>>) -> Self {
        Self::tr(key, Vec::new())
    }
    pub fn color(mut self, c: impl Into<Cow<'static, str>>) -> Self {
        self.color = Some(c.into());
        self
    }
    pub fn bold(mut self) -> Self {
        self.bold = Some(true);
        self
    }
    pub fn italic(mut self) -> Self {
        self.italic = Some(true);
        self
    }
    pub fn underlined(mut self) -> Self {
        self.underlined = Some(true);
        self
    }
    pub fn insertion(mut self, s: impl Into<String>) -> Self {
        self.insertion = Some(s.into());
        self
    }
    pub fn push(mut self, t: Text) -> Self {
        self.extra.push(t);
        self
    }
    pub fn on_click(mut self, c: Tag) -> Self {
        self.click = Some(Box::new(c));
        self
    }
    pub fn on_hover(mut self, h: Tag) -> Self {
        self.hover = Some(Box::new(h));
        self
    }
    /// Only fills style fields that are unset (vanilla `Style.applyTo`
    /// order: `self` wins).
    pub fn with_style_from(mut self, s: &Text) -> Self {
        if self.color.is_none() {
            self.color = s.color.clone();
        }
        macro_rules! inherit {
            ($($f:ident),*) => {$( if self.$f.is_none() { self.$f = s.$f.clone(); } )*};
        }
        inherit!(bold, italic, underlined, strikethrough, obfuscated, font, insertion, shadow_color, click, hover);
        self
    }

    /// Plain string of the whole component (literals only, translation keys
    /// are kept as keys). Used for logs and `getString()`-like checks.
    pub fn plain_string(&self) -> String {
        let mut s = String::new();
        self.collect_plain(&mut s);
        s
    }
    fn collect_plain(&self, s: &mut String) {
        match &self.content {
            Content::Text => s.push_str(&self.text),
            Content::Translate { key, with, .. } => {
                s.push_str(key);
                if !with.is_empty() {
                    s.push('[');
                    for (i, w) in with.iter().enumerate() {
                        if i > 0 {
                            s.push_str(", ");
                        }
                        w.collect_plain(s);
                    }
                    s.push(']');
                }
            }
            Content::Keybind(k) => s.push_str(k),
            Content::Raw(_) => {}
        }
        for e in &self.extra {
            e.collect_plain(s);
        }
    }

    fn has_style(&self) -> bool {
        self.color.is_some()
            || self.bold.is_some()
            || self.italic.is_some()
            || self.underlined.is_some()
            || self.strikethrough.is_some()
            || self.obfuscated.is_some()
            || self.font.is_some()
            || self.insertion.is_some()
            || self.shadow_color.is_some()
            || self.click.is_some()
            || self.hover.is_some()
    }

    pub fn to_nbt(&self) -> Tag {
        if matches!(self.content, Content::Text) && !self.has_style() && self.extra.is_empty() {
            return Tag::String(self.text.clone());
        }
        let mut c: Vec<(String, Tag)> = Vec::new();
        match &self.content {
            Content::Text => c.push(("text".into(), Tag::String(self.text.clone()))),
            Content::Translate { key, fallback, with } => {
                c.push(("translate".into(), Tag::String(key.to_string())));
                if let Some(f) = fallback {
                    c.push(("fallback".into(), Tag::String(f.clone())));
                }
                if !with.is_empty() {
                    c.push(("with".into(), Tag::List(with.iter().map(Text::to_nbt_arg).collect())));
                }
            }
            Content::Keybind(k) => c.push(("keybind".into(), Tag::String(k.clone()))),
            Content::Raw(fields) => c.extend(fields.iter().cloned()),
        }
        if let Some(col) = &self.color {
            c.push(("color".into(), Tag::String(col.to_string())));
        }
        let flags = [
            ("bold", self.bold),
            ("italic", self.italic),
            ("underlined", self.underlined),
            ("strikethrough", self.strikethrough),
            ("obfuscated", self.obfuscated),
        ];
        for (k, v) in flags {
            if let Some(v) = v {
                c.push((k.into(), Tag::Byte(v as i8)));
            }
        }
        if let Some(f) = &self.font {
            c.push(("font".into(), Tag::String(f.clone())));
        }
        if let Some(i) = &self.insertion {
            c.push(("insertion".into(), Tag::String(i.clone())));
        }
        if let Some(s) = self.shadow_color {
            c.push(("shadow_color".into(), Tag::Int(s)));
        }
        if let Some(cl) = &self.click {
            c.push(("click_event".into(), (**cl).clone()));
        }
        if let Some(h) = &self.hover {
            c.push(("hover_event".into(), (**h).clone()));
        }
        if !self.extra.is_empty() {
            c.push(("extra".into(), Tag::List(self.extra.iter().map(|e| e.to_nbt_compound()).collect())));
        }
        Tag::Compound(c)
    }

    /// Translation arguments: vanilla writes plain literal args as bare
    /// strings only when every element is one; mixed lists must be
    /// compounds, so args always go out as compounds unless plain.
    fn to_nbt_arg(&self) -> Tag {
        self.to_nbt_compound()
    }

    fn to_nbt_compound(&self) -> Tag {
        match self.to_nbt() {
            Tag::String(s) => Tag::compound().with("text", Tag::String(s)),
            t => t,
        }
    }

    pub fn to_json(&self) -> String {
        let mut s = String::from("{");
        match &self.content {
            Content::Translate { key, with, .. } => {
                s.push_str("\"translate\":");
                s.push_str(&json_str(key));
                if !with.is_empty() {
                    s.push_str(",\"with\":[");
                    for (i, w) in with.iter().enumerate() {
                        if i > 0 {
                            s.push(',');
                        }
                        s.push_str(&w.to_json());
                    }
                    s.push(']');
                }
            }
            Content::Keybind(k) => {
                s.push_str("\"keybind\":");
                s.push_str(&json_str(k));
            }
            _ => {
                s.push_str("\"text\":");
                s.push_str(&json_str(&self.text));
            }
        }
        if let Some(c) = &self.color {
            s.push_str(",\"color\":");
            s.push_str(&json_str(c));
        }
        if let Some(b) = self.bold {
            s.push_str(if b { ",\"bold\":true" } else { ",\"bold\":false" });
        }
        if let Some(b) = self.italic {
            s.push_str(if b { ",\"italic\":true" } else { ",\"italic\":false" });
        }
        if !self.extra.is_empty() {
            s.push_str(",\"extra\":[");
            for (i, e) in self.extra.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                s.push_str(&e.to_json());
            }
            s.push(']');
        }
        s.push('}');
        s
    }
}

impl From<&str> for Text {
    fn from(s: &str) -> Self {
        Text::plain(s)
    }
}
impl From<String> for Text {
    fn from(s: String) -> Self {
        Text::plain(s)
    }
}

/// `click_event` builders (26.3 NBT shape).
pub fn click_suggest(cmd: impl Into<String>) -> Tag {
    Tag::compound().with("action", Tag::String("suggest_command".into())).with("command", Tag::String(cmd.into()))
}
pub fn click_run(cmd: impl Into<String>) -> Tag {
    Tag::compound().with("action", Tag::String("run_command".into())).with("command", Tag::String(cmd.into()))
}
pub fn click_copy(v: impl Into<String>) -> Tag {
    Tag::compound().with("action", Tag::String("copy_to_clipboard".into())).with("value", Tag::String(v.into()))
}
pub fn click_url(u: impl Into<String>) -> Tag {
    Tag::compound().with("action", Tag::String("open_url".into())).with("url", Tag::String(u.into()))
}

/// `hover_event` builders (26.3 NBT shape).
pub fn hover_text(t: &Text) -> Tag {
    Tag::compound().with("action", Tag::String("show_text".into())).with("value", t.to_nbt())
}
pub fn hover_entity(type_id: &str, uuid: [i32; 4], name: Option<&Text>) -> Tag {
    let mut c = Tag::compound()
        .with("action", Tag::String("show_entity".into()))
        .with("id", Tag::String(type_id.into()))
        .with("uuid", Tag::IntArray(uuid.to_vec()));
    if let Some(n) = name {
        c = c.with("name", n.to_nbt());
    }
    c
}
pub fn hover_item(id: &str, count: i32, components: Option<Tag>) -> Tag {
    let mut c = Tag::compound().with("action", Tag::String("show_item".into())).with("id", Tag::String(id.into()));
    if count != 1 {
        c = c.with("count", Tag::Int(count));
    }
    if let Some(comp) = components {
        c = c.with("components", comp);
    }
    c
}

pub fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}
