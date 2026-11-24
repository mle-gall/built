//! Portable shortcut spellings used in saved profiles, and platform menu text.
//! Built keeps key names in English across UI languages. Mac menu glyphs and
//! printable Unicode come from the logical key, independent of keyboard layout.
use std::{collections::HashMap, sync::OnceLock};

use pyo3::prelude::*;

#[derive(Clone, Copy)]
struct NamedKey {
    code: u32,
    portable: &'static str,
    mac: &'static str,
    name: &'static str,
}

fn names() -> &'static HashMap<String, NamedKey> {
    static NAMES: OnceLock<HashMap<String, NamedKey>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut keys = HashMap::new();
        for line in include_str!("../assets/shortcut_keys.tsv").lines().skip(1) {
            let mut fields = line.split('\t');
            let code = u32::from_str_radix(fields.next().unwrap(), 16).unwrap();
            let name = fields.next().unwrap();
            let portable = fields.next().unwrap();
            let mac = fields.next().unwrap_or(portable);
            let key = NamedKey {
                code,
                portable,
                mac,
                name,
            };
            for alias in [portable, mac, name] {
                keys.entry(alias.to_lowercase()).or_insert(key);
            }
        }
        keys
    })
}

const SHIFT: u32 = 0x0200_0000;
const CTRL: u32 = 0x0400_0000;
const ALT: u32 = 0x0800_0000;
const META: u32 = 0x1000_0000;
const NUM: u32 = 0x2000_0000;

fn modifier(s: &str) -> Option<u32> {
    match s.to_ascii_lowercase().as_str() {
        "ctrl" | "control" | "cmd" | "command" | "⌘" => Some(CTRL),
        "alt" | "option" | "⌥" => Some(ALT),
        "shift" | "⇧" => Some(SHIFT),
        "meta" | "super" | "⌃" => Some(META),
        "num" => Some(NUM),
        _ => None,
    }
}

fn stroke(mut text: &str, mac: bool) -> Option<(String, String, u32, Option<&'static str>)> {
    let mut mods = 0;
    loop {
        let first = text.chars().next()?;
        let rest = &text[first.len_utf8()..];
        if !rest.is_empty() && matches!(first, '⌘' | '⇧' | '⌥' | '⌃') {
            mods |= modifier(&text[..first.len_utf8()])?;
            text = rest;
        } else if let Some((prefix, tail)) = text.split_once('+').filter(|(p, _)| !p.is_empty()) {
            mods |= modifier(prefix.trim_ascii())?;
            text = tail.trim_ascii();
        } else {
            break;
        }
    }

    // Cocoa displays Num+1 as Num1, but NumLock is a distinct named key.
    if let Some(rest) = text.strip_prefix("Num").filter(|s| s.chars().count() == 1) {
        mods |= NUM;
        text = rest;
    }

    let (code, portable_key, mac_key, name) = if let Some(key) = names().get(&text.to_lowercase()) {
        (
            key.code,
            key.portable.to_owned(),
            key.mac.to_owned(),
            key.name.to_owned(),
        )
    } else {
        let mut chars = text.chars();
        let c = chars.next()?;
        if chars.next().is_some() && !text.eq_ignore_ascii_case("space") {
            return None;
        }
        let c = if text.eq_ignore_ascii_case("space") {
            ' '
        } else {
            c
        };
        if c.is_control() {
            return None;
        }
        let mut upper = c.to_uppercase();
        let first = upper.next()?;
        let c = if upper.next().is_none() { first } else { c };
        let symbol = if c == ' ' {
            "Space".to_string()
        } else {
            c.to_string()
        };
        let name = match c {
            '.' => "Period",
            ',' => "Comma",
            ';' => "Semicolon",
            '-' => "Minus",
            '/' => "Slash",
            '*' => "Asterisk",
            '\'' => "Apostrophe",
            '"' => "QuoteDbl",
            _ => &symbol,
        }
        .to_owned();
        (c as u32, symbol.clone(), symbol, name)
    };
    let mut portable = String::new();
    for (mask, token) in [
        (META, "Meta+"),
        (CTRL, "Ctrl+"),
        (ALT, "Alt+"),
        (SHIFT, "Shift+"),
        (NUM, "Num+"),
    ] {
        if mods & mask != 0 {
            portable.push_str(token);
        }
    }
    portable.push_str(&portable_key);
    let display = if mac {
        let mut value = String::new();
        for (mask, token) in [
            (META, "⌃"),
            (ALT, "⌥"),
            (SHIFT, "⇧"),
            (CTRL, "⌘"),
            (NUM, "Num"),
        ] {
            if mods & mask != 0 {
                value.push_str(token);
            }
        }
        value.push_str(&mac_key);
        value
    } else {
        portable.clone()
    };
    let why =
        crate::commands::shortcut_reserved(&format!("Key_{name}"), mods & !(NUM | SHIFT) == 0);
    Some((portable, display, code | mods, why))
}

type Sequence = (String, String, Option<u32>, Option<&'static str>);

fn parse(text: &str, mac: bool) -> Option<Sequence> {
    if text.contains('\0') {
        return None;
    }
    let mut portable = Vec::new();
    let mut display = Vec::new();
    let mut chord = None;
    let mut reason = None;
    let mut begin = 0;
    for (end, ch) in text
        .char_indices()
        .chain(std::iter::once((text.len(), '\0')))
    {
        if ch != '\0' && ch != ',' {
            continue;
        }
        let part = text[begin..end].trim_ascii();
        if ch == ','
            && (end + 1 == text.len()
                || part.is_empty()
                || (part.ends_with('+') && part != "+" && !part.ends_with("++"))
                || part.ends_with(['⌘', '⇧', '⌥', '⌃']) && text[end + 1..].starts_with(','))
        {
            continue;
        }
        let (key, label, code, why) = stroke(part, mac)?;
        if portable.is_empty() {
            chord = Some(code);
            reason = why;
        } else {
            chord = None;
        }
        portable.push(key);
        display.push(label);
        if portable.len() > 4 {
            return None;
        }
        begin = end + 1;
    }
    Some((portable.join(", "), display.join(", "), chord, reason))
}

/// Canonical portable spelling, platform display, single-stroke dispatch code
/// and the drawing tool's reservation, if any.
#[pyfunction]
pub fn shortcut_text(text: &str) -> Option<Sequence> {
    parse(text.trim_ascii(), cfg!(target_os = "macos"))
}

/// Profiles and exported files store alternatives separated by semicolons.
/// A semicolon after a modifier is itself a key (for example Ctrl+;).
#[pyfunction]
pub fn shortcut_alternatives(text: &str, assignable: bool) -> Vec<String> {
    let mut result = Vec::new();
    let mut begin = 0;
    for (end, ch) in text
        .char_indices()
        .chain(std::iter::once((text.len(), '\0')))
    {
        if ch != ';' && ch != '\0' {
            continue;
        }
        let part = text[begin..end].trim_ascii();
        if ch == ';'
            && (part.is_empty() || part.ends_with('+') && !part.ends_with("++") && part != "+")
        {
            continue;
        }
        if let Some((portable, _, _, why)) = shortcut_text(part) {
            if (!assignable || why.is_none()) && !result.contains(&portable) {
                result.push(portable);
            }
        }
        begin = end + 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_and_native_spellings_converge() {
        for text in ["⌘⇧K", "shift+Ctrl+k", "⇧⌘K"] {
            let (portable, display, chord, reason) = parse(text, true).unwrap();
            assert_eq!(
                (portable.as_str(), display.as_str()),
                ("Ctrl+Shift+K", "⇧⌘K")
            );
            assert_eq!(chord, Some(CTRL | SHIFT | 'K' as u32));
            assert_eq!(reason, None);
            assert_eq!(parse(text, false).unwrap().1, portable);
        }
        for text in [
            "Ctrl+,, Ctrl++",
            "Alt+F35",
            "Ctrl+É",
            "Num+1",
            "Space",
            "Ctrl+⇥",
        ] {
            let (portable, display, code, _) = parse(text, true).unwrap();
            assert_eq!(parse(&display, true), parse(&portable, true));
            assert_eq!(code.is_none(), text.contains(", "));
        }
        assert!(parse(";", true).unwrap().3.is_some());
        assert!(parse("Alt+;", true).unwrap().3.is_none());
        assert!(parse("Up", false).unwrap().3.is_some());
        assert!(parse("unknown key", true).is_none());
        assert!(parse("Ctrl+", true).is_none());
        assert_eq!(
            shortcut_alternatives("Ctrl+;; Alt+F9; Alt+F9; 2", true),
            ["Ctrl+;", "Alt+F9"]
        );
    }
}
