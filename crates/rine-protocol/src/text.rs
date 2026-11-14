//! Text components. On the network (play/config) components are NBT; in the
//! status response and login disconnect they are JSON.

use crate::nbt::Tag;

#[derive(Debug, Clone, Default)]
pub struct Text {
    pub text: String,
    pub color: Option<&'static str>,
    pub bold: bool,
    pub italic: Option<bool>,
    pub extra: Vec<Text>,
}

impl Text {
    pub fn plain(s: impl Into<String>) -> Self {
        Self { text: s.into(), ..Default::default() }
    }
    pub fn color(mut self, c: &'static str) -> Self {
        self.color = Some(c);
        self
    }
    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }
    pub fn push(mut self, t: Text) -> Self {
        self.extra.push(t);
        self
    }

    pub fn to_nbt(&self) -> Tag {
        if self.color.is_none() && !self.bold && self.italic.is_none() && self.extra.is_empty() {
            return Tag::String(self.text.clone());
        }
        let mut c = Tag::compound().with("text", Tag::String(self.text.clone()));
        if let Some(col) = self.color {
            c = c.with("color", Tag::String(col.into()));
        }
        if self.bold {
            c = c.with("bold", Tag::Byte(1));
        }
        if let Some(i) = self.italic {
            c = c.with("italic", Tag::Byte(i as i8));
        }
        if !self.extra.is_empty() {
            c = c.with("extra", Tag::List(self.extra.iter().map(|e| e.to_nbt_compound()).collect()));
        }
        c
    }

    fn to_nbt_compound(&self) -> Tag {
        match self.to_nbt() {
            Tag::String(s) => Tag::compound().with("text", Tag::String(s)),
            t => t,
        }
    }

    pub fn to_json(&self) -> String {
        let mut s = String::from("{\"text\":");
        s.push_str(&json_str(&self.text));
        if let Some(c) = self.color {
            s.push_str(",\"color\":");
            s.push_str(&json_str(c));
        }
        if self.bold {
            s.push_str(",\"bold\":true");
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
