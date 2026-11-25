//! The value box: which keys it
//! takes and what Enter reads, for a modelling tool and for Zoom's lens.
//!
//! Before this slice it was `DocumentInteraction._handle_value_key` and
//! `_handle_zoom_value_key` in Python. The box's text stays with the view
//! that shows it; these functions take it and say what it becomes. What a
//! value then does is the tool's (`on_value`, `on_array_value`…), so the
//! Python adapter only hands the reading to the tool.

use crate::units::{self, parse, parse_value_buffer, plain_marks, Ctx, Typed, VULGAR};
use pyo3::prelude::*;
use pyo3::types::PyTuple;
use regex::Regex;
use std::sync::LazyLock;

/// What the active tool takes, beyond lengths.
#[derive(Clone, Copy, Debug, Default)]
pub struct Accepts {
    /// `3x`, `x3`, `*3`, `/3` (Move, Rotate…).
    pub arrays: bool,
    /// A copy was just made: a bare `x` opens `x5` (#542).
    pub array_pending: bool,
    /// `0.3p`, a bevel's profile.
    pub profile: bool,
    /// `200,100` is two fields (the Rectangle, #152).
    pub comma_lists: bool,
}

/// The key, as the box sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key<'a> {
    Backspace,
    Escape,
    /// A printable key's text (Qt's `event.text()`); empty for others.
    Text(&'a str),
}

/// The part of the box after the last `;` or space: the field being typed.
pub fn token_tail(buffer: &str) -> &str {
    match buffer.rfind([';', ' ']) {
        Some(i) => &buffer[i + 1..],
        None => buffer,
    }
}

pub fn is_expression(text: &str) -> bool { text.trim_start().starts_with('=') }

fn printable(text: &str) -> bool { !text.is_empty() && !text.chars().any(char::is_control) }

/// Python's `str.isdigit` for one character: a decimal or digit number
/// (`7`, `²`, `①`), not a fraction or a Roman numeral (`½`, `Ⅻ`), which
/// `char::is_numeric` also takes.
fn digit(c: char) -> bool {
    c.is_numeric() && !matches!(c, '\u{BC}'..='\u{BE}' | '\u{2150}'..='\u{218F}')
}

fn digits(text: &str) -> bool { !text.is_empty() && text.chars().all(digit) }

const UNIT_LETTERS: &[&str] = &["m", "c", "r", "i", "n", "f", "t", "s"];
const VALUE_MARKS: &[&str] = &[".", ",", ";", " ", "-", ":", "\"", "'", "/"];

/// The box after a key with a tool on: `Some(text)` when the box takes the
/// key (the text may be unchanged: a mark with nothing before it is eaten,
/// not passed on as a shortcut), `None` when the key is not the box's.
/// `chord`: Ctrl, Alt or Cmd held, a shortcut (#185).
pub fn tool_key(buffer: &str, key: Key, chord: bool, accepts: Accepts) -> Option<String> {
    let text = match key {
        Key::Backspace => return pop(buffer),
        Key::Escape => return None,
        Key::Text(t) => t,
    };
    if chord {
        return None;
    }
    let mut chars = text.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if VULGAR.iter().any(|(v, _)| *v == c) {
            // ⅜, ½…: the fraction as digits, after a space when it follows
            // a whole number (1⅜ → 1 3/8) (#189).
            return Some(plain_marks(&format!("{buffer}{text}")));
        }
    }
    let text = plain_marks(text);
    let tail = token_tail(buffer);
    let lower = text.to_lowercase();
    let push = |t: &str| Some(format!("{buffer}{t}"));
    if text == "=" && buffer.is_empty() {
        return push("=");                     // a calculation (#630)
    }
    if is_expression(buffer) {
        // Inside a calculation every printable key is part of it.
        return printable(&text).then(|| format!("{buffer}{text}"));
    }
    if accepts.arrays && text == "*" {
        return push(&text);
    }
    if accepts.arrays && lower == "x" && (!tail.is_empty() || (buffer.is_empty() && accepts.array_pending)) {
        return push(&text);                   // 3x, or x5 right after a copy
    }
    if lower == "p" && !tail.is_empty() && accepts.profile {
        return push(&text);                   // 0.3p; bare P is Push/Pull
    }
    if accepts.arrays && text == "/" && buffer.is_empty() {
        return push(&text);                   // /3, the internal array
    }
    let letter = UNIT_LETTERS.contains(&lower.as_str());
    if !(digits(&text) || VALUE_MARKS.contains(&text.as_str()) || letter) {
        return None;
    }
    // A separator with nothing typed is Space, the Select shortcut.
    if (text == ";" || text == " ") && buffer.is_empty() {
        return None;
    }
    // A unit letter opens nothing: M, C, R… are tool shortcuts. After a
    // digit it is a unit, a radius (2r) or a segment count (16s).
    if letter && tail.is_empty() {
        return None;
    }
    // Imperial marks and the fraction bar follow a digit: 2", 1'6", 3/4".
    if (text == "\"" || text == "'" || text == "/") && tail.is_empty() {
        return Some(buffer.to_string());
    }
    // Minus only opens a field (a sign, not an operator).
    if text == "-" && !tail.is_empty() {
        return Some(buffer.to_string());
    }
    // The slope colon (3:12): inside a field, once.
    if text == ":" && (tail.is_empty() || buffer.contains(':')) {
        return Some(buffer.to_string());
    }
    let mut text = text;
    if text == "." || text == "," {
        // A second decimal separator in the field: a "." is dropped, a ","
        // starts the next value (#598).
        let comma_fields = accepts.comma_lists && !buffer.contains(';') && !buffer.contains(' ');
        let field = if comma_fields { tail.rsplit(',').next().unwrap_or(tail) } else { tail };
        if field.contains('.') || field.contains(',') {
            if text == "." {
                return Some(buffer.to_string());
            }
            if !comma_fields {
                text = ";".to_string();
            }
        }
    }
    push(&text)
}

fn pop(buffer: &str) -> Option<String> {
    let mut b = buffer.to_string();
    b.pop()?;
    Some(b)
}

/// The box after a key with Zoom on (no tool): a focal length (`35mm`) or a
/// field of view (`60deg`, `60°`, `60`). Letters follow a digit only, so
/// the neighbouring tools' shortcuts still work.
pub fn zoom_key(buffer: &str, key: Key, chord: bool) -> Option<String> {
    let text = match key {
        Key::Backspace => return pop(buffer),
        Key::Escape => return (!buffer.is_empty()).then(String::new),
        Key::Text(t) => t,
    };
    if chord || text.chars().count() != 1 {
        return None;
    }
    let lower = text.to_lowercase();
    if (text == "=" && buffer.is_empty())
        || (is_expression(buffer) && printable(text))
        || digits(text)
        || text == "."
        || text == ","
    {
        return Some(format!("{buffer}{text}"));
    }
    if buffer.chars().any(digit) && (["m", "d", "e", "g", " "].contains(&lower.as_str()) || text == "°") {
        return Some(format!("{buffer}{text}"));
    }
    None
}

/// Zoom's Enter: what the lens reads (a calculation resolved to `…deg` or
/// `…mm`), or the parse error and its word.
pub fn zoom_enter(buffer: &str) -> Result<String, (&'static str, String)> {
    if !is_expression(buffer) {
        return Ok(buffer.to_string());
    }
    let deg = Ctx { bare: std::f64::consts::PI / 180.0, hyphen_feet: false };
    let angle = parse(buffer, "angle", deg);
    if let Some(v) = angle.value {
        return Ok(format!("{:.12}deg", v.to_degrees()));
    }
    let mm = Ctx { bare: 1e-3, hyphen_feet: false };
    let length = parse(buffer, "length", mm);
    match length.value {
        Some(v) => Ok(format!("{:.12}mm", v * 1000.0)),
        None => Err((length.error, length.word)),
    }
}

/// What Enter read with a tool on.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    /// A value for the tool. `absolute`: a unit was typed (`2m`), for a
    /// tool that reads a bare number as a factor (Scale).
    Value { typed: Typed, absolute: bool },
    /// A calculation that doesn't add up: it stays in the box, with why.
    Error { error: &'static str, word: String },
    /// Nothing readable: the box empties.
    Clear,
}

static EXPR_FIELDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r";|,\s+").expect("pattern"));
static HAS_UNIT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"[\d"']\s*(mm|cm|m|in|ft)\b|\d\s*["']"#).expect("pattern"));

/// Enter with a tool on. `ctx` reads the value (unitless for an angle, a
/// factor, a count, #176); `length` reads the error of a calculation.
/// `absolute_length`: the tool tells `2` from `2m`.
pub fn tool_enter(buffer: &str, comma_lists: bool, ctx: Ctx, length: Ctx, absolute_length: bool) -> Option<Entry> {
    if buffer.is_empty() {
        return None;
    }
    let Some(typed) = parse_value_buffer(buffer, comma_lists, ctx) else {
        if is_expression(buffer) {
            return Some(expression_error(buffer, length));
        }
        return Some(Entry::Clear);
    };
    let absolute = absolute_length
        && matches!(typed, Typed::One(_) | Typed::Two(..) | Typed::Three(..))
        && if is_expression(buffer) {
            parse(buffer, "length", length).explicit_measurement
        } else {
            HAS_UNIT.is_match(&buffer.to_lowercase())
        };
    Some(Entry::Value { typed, absolute })
}

fn expression_error(buffer: &str, length: Ctx) -> Entry {
    let buffer = plain_marks(buffer);
    for field in EXPR_FIELDS.split(buffer.trim()) {
        let f = field.trim();
        let f = if f.starts_with('=') { f.to_string() } else { format!("={f}") };
        let p = parse(&f, "length", length);
        if p.value.is_none() {
            return Entry::Error { error: p.error, word: p.word };
        }
    }
    Entry::Error { error: "Type one, two or three values", word: String::new() }
}

/// Whether the box keeps a key from the window's shortcuts (Qt's
/// ShortcutOverride, before the key reaches the view): `2m` must finish
/// the length, not fire Move. Looser than `tool_key`: it reserves what
/// could continue the box. `text` is the key's text, lower case.
pub fn claims(buffer: &str, text: &str, tool_on: bool, zoom: bool, accepts: Accepts) -> bool {
    if text.is_empty() || (!tool_on && !zoom) {
        return false;
    }
    if (text == "=" && buffer.is_empty()) || (is_expression(buffer) && printable(text)) {
        return true;
    }
    let one_of = |set: &str| set.contains(text);   // Python's `text in set`
    if zoom {
        return digits(text) || one_of(".,") || (buffer.chars().any(digit) && one_of("mdeg °"));
    }
    if !buffer.is_empty() {
        return digits(text) || one_of(".,; -:\"'/mcrinftx*s") || (text == "p" && accepts.profile);
    }
    text == "x" && accepts.arrays && accepts.array_pending
}

// ---- Python -------------------------------------------------------------

fn key_of<'a>(key: &str, text: &'a str) -> Key<'a> {
    match key {
        "backspace" => Key::Backspace,
        "escape" => Key::Escape,
        _ => Key::Text(text),
    }
}

/// The box after a key: the new text, or None when the key is not the
/// box's. `key` is "backspace", "escape" or "" (a key with `text`);
/// `zoom`: Zoom's lens box (no tool on).
#[pyfunction]
#[pyo3(signature = (buffer, key, text, chord=false, arrays=false, array_pending=false,
                    profile=false, comma_lists=false, zoom=false))]
#[allow(clippy::too_many_arguments)]
pub fn value_key(buffer: &str, key: &str, text: &str, chord: bool, arrays: bool, array_pending: bool,
                 profile: bool, comma_lists: bool, zoom: bool) -> Option<String> {
    let key = key_of(key, text);
    if zoom {
        return zoom_key(buffer, key, chord);
    }
    tool_key(buffer, key, chord, Accepts { arrays, array_pending, profile, comma_lists })
}

/// Enter with a tool on: None (an empty box: not the box's key),
/// `("value", value, absolute)` with the value as `parse_value_buffer`
/// gives it, `("error", message, word)` or `("clear",)`.
#[pyfunction]
#[pyo3(signature = (buffer, comma_lists, bare, hyphen_feet, length_bare, absolute_length=false))]
pub fn value_enter<'py>(py: Python<'py>, buffer: &str, comma_lists: bool, bare: f64, hyphen_feet: bool,
                        length_bare: f64, absolute_length: bool) -> PyResult<Option<Bound<'py, PyTuple>>> {
    let ctx = Ctx { bare, hyphen_feet };
    let length = Ctx { bare: length_bare, hyphen_feet };
    Ok(match tool_enter(buffer, comma_lists, ctx, length, absolute_length) {
        None => None,
        Some(Entry::Clear) => Some(PyTuple::new(py, ["clear"])?),
        Some(Entry::Error { error, word }) => Some(("error", error, word).into_pyobject(py)?),
        Some(Entry::Value { typed, absolute }) => {
            Some(("value", units::typed_to_py(py, typed)?, absolute).into_pyobject(py)?)
        }
    })
}

/// Zoom's Enter: `(text, "", "")` for `apply_zoom_value`, or
/// `(None, error, word)`.
#[pyfunction]
#[pyo3(name = "zoom_value_enter")]
pub fn py_zoom_enter(buffer: &str) -> (Option<String>, &'static str, String) {
    match zoom_enter(buffer) {
        Ok(t) => (Some(t), "", String::new()),
        Err((e, w)) => (None, e, w),
    }
}

/// Whether the box keeps the key from the window's shortcuts (`claims`).
#[pyfunction]
#[pyo3(signature = (buffer, text, tool_on, zoom=false, arrays=false, array_pending=false, profile=false))]
pub fn value_claims(buffer: &str, text: &str, tool_on: bool, zoom: bool, arrays: bool,
                    array_pending: bool, profile: bool) -> bool {
    claims(buffer, text, tool_on, zoom, Accepts { arrays, array_pending, profile, comma_lists: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(keys: &str, accepts: Accepts) -> String {
        let mut b = String::new();
        for c in keys.chars() {
            let s = c.to_string();
            if let Some(n) = tool_key(&b, Key::Text(&s), false, accepts) {
                b = n;
            }
        }
        b
    }

    const NONE: Accepts = Accepts { arrays: false, array_pending: false, profile: false, comma_lists: false };

    #[test]
    fn keys() {
        assert_eq!(typed("2.5.3", NONE), "2.53");
        assert_eq!(typed("0.9,2.1", NONE), "0.9;2.1");
        assert_eq!(typed("200,100", Accepts { comma_lists: true, ..NONE }), "200,100");
        assert_eq!(typed("2,5,3", Accepts { comma_lists: true, ..NONE }), "2,5,3");
        assert_eq!(typed("m2", NONE), "2");
        assert_eq!(typed("2m", NONE), "2m");
        assert_eq!(typed("\"2\"", NONE), "2\"");
        assert_eq!(typed("-2-", NONE), "-2");
        assert_eq!(typed("3:12:", NONE), "3:12");
        assert_eq!(typed("3x", NONE), "3");
        assert_eq!(typed("3x", Accepts { arrays: true, ..NONE }), "3x");
        assert_eq!(typed("x5", Accepts { arrays: true, ..NONE }), "5");
        assert_eq!(typed("x5", Accepts { arrays: true, array_pending: true, ..NONE }), "x5");
        assert_eq!(typed("/3", Accepts { arrays: true, ..NONE }), "/3");
        assert_eq!(typed("0.3p", Accepts { profile: true, ..NONE }), "0.3p");
        assert_eq!(typed("=1+2*(3)", NONE), "=1+2*(3)");
        assert_eq!(typed("1½", NONE), "1 1/2");
        assert_eq!(tool_key("", Key::Text(" "), false, NONE), None);
        assert_eq!(tool_key("", Key::Text("m"), false, NONE), None);
        assert_eq!(tool_key("2", Key::Text("1"), true, NONE), None);
        assert_eq!(tool_key("", Key::Backspace, false, NONE), None);
        assert_eq!(tool_key("25", Key::Backspace, false, NONE), Some("2".into()));
        assert_eq!(tool_key("2", Key::Text("\t"), false, NONE), None);
        assert!(claims("2", "½", true, false, NONE) == false && claims("2", "²", true, false, NONE));
    }

    #[test]
    fn zoom() {
        let z = |b: &str, t: &str| zoom_key(b, Key::Text(t), false);
        assert_eq!(z("", "m"), None);
        assert_eq!(z("35", "m"), Some("35m".into()));
        assert_eq!(z("60", "°"), Some("60°".into()));
        assert_eq!(zoom_key("6", Key::Escape, false), Some(String::new()));
        assert_eq!(zoom_key("", Key::Escape, false), None);
        assert_eq!(zoom_enter("=30*2").unwrap(), "60.000000000000deg");
        assert_eq!(zoom_enter("=2*17.5mm").unwrap(), "35.000000000000mm");
    }

    #[test]
    fn enter() {
        let m = Ctx { bare: 1.0, hyphen_feet: false };
        assert_eq!(tool_enter("", false, m, m, false), None);
        assert_eq!(tool_enter("2;3", false, m, m, false),
                   Some(Entry::Value { typed: Typed::Two(2.0, 3.0), absolute: false }));
        assert_eq!(tool_enter("2m", false, m, m, true),
                   Some(Entry::Value { typed: Typed::One(2.0), absolute: true }));
        assert_eq!(tool_enter("2", false, m, m, true),
                   Some(Entry::Value { typed: Typed::One(2.0), absolute: false }));
        assert_eq!(tool_enter("1;2;3;4", false, m, m, false), Some(Entry::Clear));
        assert_eq!(tool_enter("=(1+2", false, m, m, false),
                   Some(Entry::Error { error: "A parenthesis is not closed", word: String::new() }));
    }
}
