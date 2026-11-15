//! English translations (`en_us.json` of 26.3) for what the server itself
//! renders: console output, `Component.getString()` (selector `name=`,
//! score holder names, sign text checks).

use ahash::AHashMap;
use rine_protocol::text::{Content, Text};
use std::sync::OnceLock;

static LANG: OnceLock<AHashMap<String, String>> = OnceLock::new();

fn table() -> &'static AHashMap<String, String> {
    LANG.get_or_init(|| {
        let m: std::collections::HashMap<String, String> = serde_json::from_str(include_str!("../assets/en_us.json")).unwrap_or_default();
        m.into_iter().collect()
    })
}

pub fn translate(key: &str) -> Option<&'static str> {
    table().get(key).map(|s| s.as_str())
}

pub fn has(key: &str) -> bool {
    table().contains_key(key)
}

/// `Component.getString()` with English translations.
pub fn plain(t: &Text) -> String {
    let mut s = String::new();
    render(t, &mut s);
    s
}

fn render(t: &Text, out: &mut String) {
    match &t.content {
        Content::Text => out.push_str(&t.text),
        Content::Translate { key, fallback, with } => {
            let fmt = translate(key).or(fallback.as_deref()).unwrap_or(key);
            format_into(fmt, with, out);
        }
        Content::Keybind(k) => out.push_str(k),
        Content::Raw(_) => {}
    }
    for e in &t.extra {
        render(e, out);
    }
}

/// Java `TranslatableContents` formatting: `%s`, `%1$s`, `%%`.
fn format_into(fmt: &str, args: &[Text], out: &mut String) {
    let b = fmt.as_bytes();
    let mut i = 0;
    let mut next = 0usize;
    while i < b.len() {
        if b[i] == b'%' && i + 1 < b.len() {
            if b[i + 1] == b'%' {
                out.push('%');
                i += 2;
                continue;
            }
            if b[i + 1] == b's' {
                if let Some(a) = args.get(next) {
                    render(a, out);
                }
                next += 1;
                i += 2;
                continue;
            }
            // %N$s
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j + 1 < b.len() && j > i + 1 && b[j] == b'$' && b[j + 1] == b's' {
                let n: usize = fmt[i + 1..j].parse().unwrap_or(1);
                if let Some(a) = args.get(n.saturating_sub(1)) {
                    render(a, out);
                }
                i = j + 2;
                continue;
            }
        }
        let c = fmt[i..].chars().next().unwrap();
        out.push(c);
        i += c.len_utf8();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formats() {
        let t = Text::tr("commands.gamemode.success.self", vec![Text::key("gameMode.creative")]);
        assert_eq!(plain(&t), "Set own game mode to Creative Mode");
    }
}
