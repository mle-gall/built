//! Line types: the built-in catalogue, the `.lin` line type files, and how
//! a pattern is drawn — dashes cut along a segment, the words a line type
//! writes along it.
//!
//! `built_core.layers` was the implementation; it keeps the same names as a
//! facade over these functions. A pattern is kept as a line type file has
//! it (dash > 0, gap < 0, dot 0, millimetres on paper) and drawn as
//! `drawn_dashes` turns it (dash, gap, dash… all ≥ 0). Answers match the
//! Python ones exactly (`tests/test_linetype_parity.py`).

use numpy::{PyArray3, PyArrayMethods};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};

/// The built-in line types and their dash pattern on paper, mm (dash, gap,
/// …; empty = continuous), with the name every CAD program knows them by.
pub const LINETYPES: [(&str, &[f64], &str); 10] = [
    ("continuous", &[], "CONTINUOUS"),
    ("dashed", &[3.0, 1.5], "DASHED"),
    ("dashed_short", &[1.5, 0.75], "HIDDEN"),
    ("dashed_long", &[8.0, 2.0], "DASHEDX2"),
    ("dotted", &[0.5, 1.0], "DOT"),
    ("dashdot", &[6.0, 1.5, 0.5, 1.5], "DASHDOT"),
    ("dashdotdot", &[6.0, 1.5, 0.5, 1.5, 0.5, 1.5], "DIVIDE"),
    ("center", &[12.0, 1.5, 2.0, 1.5], "CENTER"),
    ("phantom", &[12.0, 1.5, 2.0, 1.5, 2.0, 1.5], "PHANTOM"),
    ("border", &[6.0, 1.5, 6.0, 1.5, 0.5, 1.5], "BORDER"),
];

/// The most pieces a pattern keeps, the longest text and its height (mm).
const MAX_PIECES: usize = 32;
const MAX_TEXT: usize = 24;
const TEXT_SIZES: (f64, f64) = (0.5, 20.0);

pub fn builtin(key: &str) -> Option<&'static [f64]> {
    LINETYPES.iter().find(|l| l.0 == key).map(|l| l.1)
}

/// Python's `float(text)` for a string; `None` where it raises.
fn py_float_str(text: &str) -> Option<f64> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let t = if t.contains('_') {
        let c: Vec<char> = t.chars().collect();
        let ok = c.iter().enumerate().all(|(i, ch)| {
            *ch != '_' || (i > 0 && i + 1 < c.len() && c[i - 1].is_ascii_digit() && c[i + 1].is_ascii_digit())
        });
        if !ok {
            return None;
        }
        t.replace('_', "")
    } else {
        t.to_string()
    };
    t.parse::<f64>().ok()
}

/// `str.splitlines()`.
fn splitlines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut it = text.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        let brk = matches!(c, '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e'
            | '\u{85}' | '\u{2028}' | '\u{2029}');
        if brk {
            out.push(&text[start..i]);
            let mut end = i + c.len_utf8();
            if c == '\r' {
                if let Some((j, '\n')) = it.peek().copied() {
                    it.next();
                    end = j + 1;
                }
            }
            start = end;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

fn first_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// About how wide `text` runs at `size` mm high, in mm.
pub fn text_width(text: &str, size: f64) -> f64 {
    0.68 * size * text.chars().count() as f64
}

/// A pattern kept sane: finite numbers in mm (rounded to 0.1 µm), at most
/// 32 pieces, something drawn and something left blank.
pub fn clean_values(values: &[f64]) -> Option<Vec<f64>> {
    let mut out = Vec::with_capacity(values.len().min(MAX_PIECES));
    for &v in values.iter().take(MAX_PIECES) {
        if !v.is_finite() || v.abs() > 1000.0 {
            return None;
        }
        out.push(crate::model::pyfloat::round(v, 4));
    }
    if out.is_empty() || crate::model::pyfloat::sum(out.iter().map(|v| v.abs())) < 0.05 {
        return None;
    }
    if !out.iter().any(|v| *v < 0.0) || !out.iter().any(|v| *v >= 0.0) {
        return None;
    }
    Some(out)
}

/// Pieces as a file has them (dash > 0, gap < 0, dot 0) as a pen draws
/// them: dash, gap, … all ≥ 0, starting with a dash, two pieces of a kind
/// joined, a leading gap moved to the end.
pub fn drawn_dashes(pieces: &[f64]) -> Vec<f64> {
    let mut runs: Vec<(bool, f64)> = Vec::new();
    for &v in pieces {
        let drawn = v >= 0.0;
        match runs.last_mut() {
            Some(last) if last.0 == drawn && !(drawn && v == 0.0) => last.1 += v.abs(),
            _ => runs.push((drawn, v.abs())),
        }
    }
    if runs.first().is_some_and(|r| !r.0) {
        runs.rotate_left(1);
    }
    let mut out: Vec<f64> = Vec::new();
    for (drawn, length) in runs {
        if !out.is_empty() && (out.len() % 2 == 0) != drawn {
            let last = out.last_mut().expect("non-empty");
            *last += if drawn { 0.0 } else { length };
            continue;
        }
        out.push(length);
    }
    if out.len() % 2 == 1 {
        out.push(0.0);
    }
    out
}

pub fn file_dashes(drawn: &[f64]) -> Vec<f64> {
    drawn.iter().enumerate().map(|(i, v)| if i % 2 == 0 { *v } else { -*v }).collect()
}

/// Where along one drawn period the middle of piece `at` falls, in mm.
pub fn text_offset(pattern: &[f64], at: usize) -> f64 {
    let period = crate::model::pyfloat::sum(pattern.iter().map(|v| v.abs()));
    let raw = crate::model::pyfloat::sum(pattern[..at].iter().map(|v| v.abs())) + pattern[at].abs() / 2.0;
    let mut lead = 0.0;
    for v in pattern {
        if *v >= 0.0 {
            break;
        }
        lead += v.abs();
    }
    if period > 0.0 { crate::model::pyfloat::modulo(raw - lead, period) } else { 0.0 }
}

/// The text of a line type: its words (printable, stripped, at most 24),
/// height (clamped, paper mm) and the gap of `pattern` it sits in.
#[derive(Clone, Debug, PartialEq)]
pub struct TextSpec {
    pub text: String,
    pub size: f64,
    pub at: usize,
}

fn clean_spec(text: String, size: f64, at: i64, pattern: &[f64]) -> Option<TextSpec> {
    let text = first_chars(text.trim(), MAX_TEXT);
    if text.is_empty() || !size.is_finite() {
        return None;
    }
    let size = TEXT_SIZES.1.min(TEXT_SIZES.0.max(size));
    if at < 0 || at as usize >= pattern.len() || pattern[at as usize] >= 0.0 {
        return None;
    }
    Some(TextSpec { text, size: crate::model::pyfloat::round(size, 3), at: at as usize })
}

/// `text` with only the characters Python counts printable.
fn printable(py: Python<'_>, text: &str) -> PyResult<String> {
    let s = PyString::new(py, text);
    if s.call_method0("isprintable")?.is_truthy()? {
        return Ok(text.to_string());
    }
    let mut out = String::with_capacity(text.len());
    let mut buf = [0u8; 4];
    for c in text.chars() {
        if PyString::new(py, c.encode_utf8(&mut buf)).call_method0("isprintable")?.is_truthy()? {
            out.push(c);
        }
    }
    Ok(out)
}

/// `pattern` with room for `text` (its width and a margin each side) added
/// to its last gap, or as a gap of its own.
pub fn with_text_room(pattern: &[f64], text: &str, size: f64) -> Vec<f64> {
    let room = text_width(text, size) + 2.0 * 0.6f64.max(0.4 * size);
    let mut out = pattern.to_vec();
    match out.last_mut() {
        Some(last) if *last < 0.0 => *last = crate::model::pyfloat::round(*last - room, 3),
        _ => out.push(-crate::model::pyfloat::round(room, 3)),
    }
    out
}

/// `pattern` with `text` in it (`with_text_room`) and the text's spec;
/// the pattern as it is when there is no text.
pub fn with_text(py: Python<'_>, pattern: &[f64], text: &str, size: f64) -> PyResult<(Vec<f64>, Option<TextSpec>)> {
    let text = text.trim();
    if text.is_empty() {
        return Ok((pattern.to_vec(), None));
    }
    let out = with_text_room(pattern, text, size);
    let at = out.len() as i64 - 1;
    let spec = clean_spec(printable(py, text)?, size, at, &out);
    Ok((out, spec))
}

/// Where a line type's words go along `p0`→`p1` (paper mm, the pattern
/// restarting at `p0`): `(x, y, degrees)`, the middle of each word that
/// fits whole, kept reading upright.
pub fn text_marks(p0: (f64, f64), p1: (f64, f64), text: &str, size: f64, offset: f64, period: f64) -> Vec<(f64, f64, f64)> {
    let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
    let length = crate::model::pyfloat::hypot(&[dx, dy]);
    if length <= 1e-9 || period <= 1e-9 {
        return Vec::new();
    }
    let half = text_width(text, size) / 2.0;
    let mut angle = dy.atan2(dx).to_degrees();
    if angle > 90.0 {
        angle -= 180.0;
    } else if angle <= -90.0 {
        angle += 180.0;
    }
    let mut out = Vec::new();
    let mut t = offset;
    let mut n = 0;
    while t + half <= length && n < 2000 {
        if t - half >= 0.0 {
            let f = t / length;
            out.push((p0.0 + dx * f, p0.1 + dy * f, angle));
        }
        t += period;
        n += 1;
    }
    out
}

/// Segments cut as a dash pattern (paper mm) at `unit` metres per mm,
/// restarting on each segment: the dashes and the gaps. A dot is a short
/// dash; a pattern too fine for an edge (over 400 periods) leaves it whole.
pub fn dash_segments(segs: &[[[f64; 3]; 2]], pattern: &[f64], unit: f64) -> (Vec<[[f64; 3]; 2]>, Vec<[[f64; 3]; 2]>) {
    let mut lengths: Vec<f64> = pattern
        .iter()
        .enumerate()
        .map(|(i, v)| if i % 2 == 0 { v.max(0.25) * unit } else { v.max(0.0) * unit })
        .collect();
    if lengths.len() % 2 == 1 {
        lengths.push(0.0);
    }
    let period = crate::model::pyfloat::sum(lengths.iter().copied());
    let (mut dashes, mut gaps) = (Vec::new(), Vec::new());
    for [a, b] in segs {
        let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let total = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if total <= 1e-12 {
            continue;
        }
        if period <= 1e-12 || total / period > 400.0 {
            dashes.push([*a, *b]);
            continue;
        }
        let u = [d[0] / total, d[1] / total, d[2] / total];
        let at = |t: f64| [a[0] + u[0] * t, a[1] + u[1] * t, a[2] + u[2] * t];
        let mut t = 0.0;
        let mut k = 0usize;
        while t < total {
            let step = lengths[k % lengths.len()];
            let end = total.min(t + step);
            if end > t {
                if k % 2 == 0 { &mut dashes } else { &mut gaps }.push([at(t), at(end)]);
            }
            t = end;
            k += 1;
        }
    }
    (dashes, gaps)
}

/// One line type read from a file.
pub struct LinEntry {
    pub name: String,
    pub label: String,
    pub pattern: Vec<f64>,
    pub text: Option<TextSpec>,
}

/// The part of a `.lin` line between commas outside `[…]` (Python's
/// `re.split(r",(?![^\[]*\])", body)`).
fn split_pieces(body: &str) -> Vec<&str> {
    let bytes = body.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    for (i, b) in bytes.iter().enumerate() {
        if *b != b',' {
            continue;
        }
        let inside = bytes[i + 1..].iter().find(|c| **c == b'[' || **c == b']') == Some(&b']');
        if !inside {
            out.push(&body[start..i]);
            start = i + 1;
        }
    }
    out.push(&body[start..]);
    out
}

/// The line types of a line type file (`.lin`): `*NAME,description` then
/// `A,dash,-gap,0,…`, with a text piece (`["GAS",STANDARD,S=2.5,…]`) when
/// one writes words; those that draw a shape are skipped by name. A
/// pattern whose every piece is within one unit is taken as inches.
pub fn parse_lin(py: Python<'_>, text: &str) -> PyResult<(Vec<LinEntry>, Vec<String>)> {
    let mut found: Vec<LinEntry> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut name: Option<String> = None;
    let mut label = String::new();
    for raw in splitlines(text) {
        let line = raw.split_once(";;").map_or(raw, |p| p.0).trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('*') {
            let (head, tail) = rest.split_once(',').unwrap_or((rest, ""));
            let n = first_chars(head.trim(), 64);
            name = if n.is_empty() { None } else { Some(n) };
            label = first_chars(tail.trim(), 120);
            continue;
        }
        let Some(current) = name.take() else { continue };
        let mut chars = line.chars();
        let body = match chars.next() {
            Some('A') | Some('a') => chars.as_str(),
            _ => line,
        };
        let body = body.trim_start_matches(',');
        let mut words: Option<(String, f64)> = None;
        let mut at: Option<i64> = None; // the gap the words sit in; -1: merged
        let mut nums: Vec<f64> = Vec::new();
        let mut bad = false;
        for part in split_pieces(body) {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if part.starts_with('[') {
                let inner: Vec<&str> = part.trim_matches(|c| c == '[' || c == ']').split(',').map(str::trim).collect();
                let first = inner.first().copied().unwrap_or("");
                if !(first.starts_with('"') && first.ends_with('"') && first.chars().count() > 2) || words.is_some() {
                    bad = true;
                    break;
                }
                let mut size = 0.1;
                for item in &inner[1..] {
                    let (k, v) = item.split_once('=').unwrap_or((item, ""));
                    if k.trim().to_uppercase() == "S" {
                        if let Some(s) = py_float_str(v) {
                            size = s;
                        }
                    }
                }
                let quoted = &first[1..first.len() - 1];
                words = Some((quoted.to_string(), size));
                at = match nums.last() {
                    Some(v) if *v < 0.0 => Some(nums.len() as i64 - 1),
                    _ => None,
                };
                continue;
            }
            let Some(v) = py_float_str(part) else {
                bad = true;
                break;
            };
            if let Some(a) = at {
                if nums.len() as i64 - 1 == a && v < 0.0 {
                    nums[a as usize] += v;
                    at = Some(-1); // merged: kept where it is
                    continue;
                }
            }
            if words.is_some() && at.is_none() && v < 0.0 {
                at = Some(nums.len() as i64); // no gap before: the one after
            }
            nums.push(v);
            if at == Some(-1) {
                at = None;
            }
        }
        if bad {
            skipped.push(current);
            continue;
        }
        let inches = !nums.is_empty() && nums.iter().all(|v| v.abs() <= 1.0);
        if inches {
            for v in nums.iter_mut() {
                *v *= 25.4;
            }
        }
        let Some(pattern) = clean_values(&nums) else { continue };
        let mut spec = None;
        if let Some((text, size)) = words {
            // The widest gap holds the text when its place was lost.
            let mut widest: Option<usize> = None;
            for (i, v) in pattern.iter().enumerate() {
                if *v < 0.0 && widest.is_none_or(|w| -*v > -pattern[w]) {
                    widest = Some(i);
                }
            }
            if let Some(w) = widest {
                let size = size * if inches { 25.4 } else { 1.0 };
                spec = clean_spec(printable(py, &text)?, size, w as i64, &pattern);
            }
        }
        let entry = LinEntry {
            name: current.clone(),
            label: if label.is_empty() { current } else { label.clone() },
            pattern,
            text: spec,
        };
        match found.iter_mut().find(|e| e.name == entry.name) {
            Some(slot) => *slot = entry,
            None => found.push(entry),
        }
    }
    Ok((found, skipped))
}

/// A line type file (`.lin`, mm) of `(name, label, pattern, text)` rows;
/// a text goes in the middle of its gap, as the format writes one.
pub fn to_lin(entries: &[(String, String, Vec<f64>, Option<TextSpec>)]) -> String {
    let mut lines = vec![";; Line types exported from Built (millimetres)".to_string(), String::new()];
    for (name, label, pattern, spec) in entries {
        if pattern.is_empty() {
            continue;
        }
        let mut safe = String::new();
        let mut in_bad = false;
        for c in name.chars() {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                safe.push(c);
                in_bad = false;
            } else if !in_bad {
                safe.push('_');
                in_bad = true;
            }
        }
        let safe = safe.trim_matches('_');
        let safe = if safe.is_empty() { "LINE" } else { safe };
        lines.push(format!("*{safe},{}", label.replace(',', " ")));
        let mut pieces: Vec<String> = pattern.iter().map(|v| crate::model::pyfloat::g(*v)).collect();
        if let Some(s) = spec {
            let half = pattern[s.at] / 2.0;
            let words = s.text.replace('"', "'");
            let mark = format!(
                "[\"{words}\",STANDARD,S={},R=0.0,X={},Y={}]",
                crate::model::pyfloat::g(s.size),
                crate::model::pyfloat::g(-text_width(&words, s.size) / 2.0),
                crate::model::pyfloat::g(-s.size / 2.0)
            );
            pieces.splice(s.at..s.at + 1, [crate::model::pyfloat::g(half), mark, crate::model::pyfloat::g(half)]);
        }
        lines.push(format!("A,{}", pieces.join(",")));
    }
    lines.join("\n") + "\n"
}

// ---- Python -----------------------------------------------------------------

/// Python's `float(v)`; `None` where it raises TypeError / ValueError.
fn py_float(v: &Bound<'_, PyAny>) -> PyResult<Option<f64>> {
    if let Ok(f) = v.extract::<f64>() {
        return Ok(Some(f));
    }
    match v.py().get_type::<PyFloat>().call1((v,)) {
        Ok(f) => Ok(Some(f.extract()?)),
        Err(e) if e.is_instance_of::<PyTypeError>(v.py()) || e.is_instance_of::<PyValueError>(v.py()) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Python's `int(v)`; `None` where it raises TypeError / ValueError.
fn py_int(v: &Bound<'_, PyAny>) -> PyResult<Option<i64>> {
    if let Ok(i) = v.extract::<i64>() {
        return Ok(Some(i));
    }
    match v.py().get_type::<PyInt>().call1((v,)) {
        Ok(i) => Ok(Some(i.extract()?)),
        Err(e) if e.is_instance_of::<PyTypeError>(v.py()) || e.is_instance_of::<PyValueError>(v.py()) => Ok(None),
        Err(e) => Err(e),
    }
}

pub(crate) fn spec_dict<'py>(py: Python<'py>, s: &TextSpec) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("text", &s.text)?;
    d.set_item("text_size", s.size)?;
    d.set_item("text_at", s.at)?;
    Ok(d)
}

fn floats(v: &Bound<'_, PyAny>) -> PyResult<Vec<f64>> {
    v.try_iter()?.map(|x| x?.extract::<f64>()).collect()
}

/// `{key: pattern}` and `{key: DXF name}` of the built-in line types.
#[pyfunction]
pub fn linetype_table() -> Vec<(&'static str, Vec<f64>, &'static str)> {
    LINETYPES.iter().map(|(k, p, d)| (*k, p.to_vec(), *d)).collect()
}

#[pyfunction]
#[pyo3(name = "linetype_clean_pattern")]
pub fn py_clean_pattern(raw: &Bound<'_, PyAny>) -> PyResult<Option<Vec<f64>>> {
    if !(raw.is_instance_of::<PyList>() || raw.is_instance_of::<PyTuple>()) {
        return Ok(None);
    }
    let mut values = Vec::new();
    for v in raw.try_iter()?.take(MAX_PIECES) {
        match py_float(&v?)? {
            Some(f) => values.push(f),
            None => return Ok(None),
        }
    }
    Ok(clean_values(&values))
}

#[pyfunction]
#[pyo3(name = "linetype_drawn_dashes")]
pub fn py_drawn_dashes(pieces: &Bound<'_, PyAny>) -> PyResult<Vec<f64>> {
    Ok(drawn_dashes(&floats(pieces)?))
}

#[pyfunction]
#[pyo3(name = "linetype_file_dashes")]
pub fn py_file_dashes(drawn: &Bound<'_, PyAny>) -> PyResult<Vec<f64>> {
    Ok(file_dashes(&floats(drawn)?))
}

#[pyfunction]
#[pyo3(name = "linetype_text_offset")]
pub fn py_text_offset(pattern: &Bound<'_, PyAny>, at: i64) -> PyResult<f64> {
    let p = floats(pattern)?;
    let i = if at < 0 { at + p.len() as i64 } else { at };
    if i < 0 || i as usize >= p.len() {
        return Err(pyo3::exceptions::PyIndexError::new_err("list index out of range"));
    }
    Ok(text_offset(&p, i as usize))
}

#[pyfunction]
#[pyo3(name = "linetype_text_width")]
pub fn py_text_width(text: &str, size: f64) -> f64 {
    text_width(text, size)
}

#[pyfunction]
#[pyo3(name = "linetype_clean_line_text")]
pub fn py_clean_line_text<'py>(entry: &Bound<'py, PyAny>, pattern: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyDict>>> {
    let py = entry.py();
    let Ok(entry) = entry.cast::<PyDict>() else { return Ok(None) };
    if !pattern.is_truthy()? {
        return Ok(None);
    }
    let Some(text) = entry.get_item("text")? else { return Ok(None) };
    let Ok(text) = text.cast::<PyString>() else { return Ok(None) };
    let text = printable(py, text.to_str()?)?;
    if text.trim().is_empty() {
        return Ok(None);
    }
    let size = match entry.get_item("text_size")? {
        None => Some(2.5),
        Some(v) => py_float(&v)?,
    };
    let pattern = floats(pattern)?;
    let at = match entry.get_item("text_at")? {
        None => Some(pattern.len() as i64 - 1),
        Some(v) => py_int(&v)?,
    };
    let (Some(size), Some(at)) = (size, at) else { return Ok(None) };
    clean_spec(text, size, at, &pattern).map(|s| spec_dict(py, &s)).transpose()
}

#[pyfunction]
#[pyo3(name = "linetype_with_text", signature = (pattern, text, size=2.5))]
pub fn py_with_text<'py>(pattern: &Bound<'py, PyAny>, text: Option<&str>, size: f64) -> PyResult<(Vec<f64>, Option<Bound<'py, PyDict>>)> {
    let py = pattern.py();
    let (out, spec) = with_text(py, &floats(pattern)?, text.unwrap_or(""), size)?;
    Ok((out, spec.map(|s| spec_dict(py, &s)).transpose()?))
}

#[pyfunction]
#[pyo3(name = "linetype_text_marks")]
pub fn py_text_marks(p0: &Bound<'_, PyAny>, p1: &Bound<'_, PyAny>, spec: &Bound<'_, PyAny>) -> PyResult<Vec<(f64, f64, f64)>> {
    if spec.is_none() {
        return Ok(Vec::new());
    }
    let xy = |p: &Bound<'_, PyAny>| -> PyResult<(f64, f64)> { Ok((p.get_item(0)?.extract()?, p.get_item(1)?.extract()?)) };
    let text: String = spec.get_item(0)?.extract()?;
    Ok(text_marks(xy(p0)?, xy(p1)?, &text, spec.get_item(1)?.extract()?,
                  spec.get_item(2)?.extract()?, spec.get_item(3)?.extract()?))
}

#[pyfunction]
#[pyo3(name = "linetype_dash_segments")]
pub fn py_dash_segments<'py>(py: Python<'py>, segs: &Bound<'py, PyAny>, pattern: &Bound<'py, PyAny>, unit: f64)
    -> PyResult<(Bound<'py, PyArray3<f32>>, Bound<'py, PyArray3<f32>>)> {
    let np = py.import("numpy")?;
    let arr = np.call_method1("asarray", (segs, np.getattr("float64")?))?.call_method1("reshape", (-1, 2, 3))?;
    let arr = arr.cast_into::<numpy::PyArray3<f64>>()?;
    let ro = arr.readonly();
    let view = ro.as_array();
    let rows: Vec<[[f64; 3]; 2]> = (0..view.shape()[0])
        .map(|i| [[view[[i, 0, 0]], view[[i, 0, 1]], view[[i, 0, 2]]], [view[[i, 1, 0]], view[[i, 1, 1]], view[[i, 1, 2]]]])
        .collect();
    let pattern = floats(pattern)?;
    let to_py = |rows: Vec<[[f64; 3]; 2]>| -> PyResult<Bound<'py, PyArray3<f32>>> {
        let flat: Vec<f32> = rows.iter().flat_map(|s| s.iter().flat_map(|p| p.iter().map(|v| *v as f32))).collect();
        Ok(numpy::PyArray1::from_vec(py, flat).reshape([rows.len(), 2, 3])?)
    };
    if rows.is_empty() || pattern.is_empty() {
        return Ok((to_py(rows)?, to_py(Vec::new())?));
    }
    let (dashes, gaps) = dash_segments(&rows, &pattern, unit);
    Ok((to_py(dashes)?, to_py(gaps)?))
}

#[pyfunction]
#[pyo3(name = "linetype_parse_lin")]
pub fn py_parse_lin<'py>(py: Python<'py>, text: &str) -> PyResult<(Bound<'py, PyDict>, Vec<String>)> {
    let (found, skipped) = parse_lin(py, text)?;
    let out = PyDict::new(py);
    for e in found {
        let d = PyDict::new(py);
        d.set_item("label", &e.label)?;
        d.set_item("pattern", PyList::new(py, &e.pattern)?)?;
        if let Some(s) = &e.text {
            d.update(spec_dict(py, s)?.as_mapping())?;
        }
        out.set_item(&e.name, d)?;
    }
    Ok((out, skipped))
}

#[pyfunction]
#[pyo3(name = "linetype_to_lin")]
pub fn py_to_lin(entries: &Bound<'_, PyAny>) -> PyResult<String> {
    let mut rows = Vec::new();
    for row in entries.try_iter()? {
        let row = row?;
        let name: String = row.get_item(0)?.extract()?;
        let label: String = row.get_item(1)?.extract()?;
        let pattern = row.get_item(2)?;
        let pattern = if pattern.is_truthy()? { floats(&pattern)? } else { Vec::new() };
        let spec = if row.len()? > 3 { Some(row.get_item(3)?) } else { None };
        let spec = match spec {
            Some(s) if s.is_truthy()? => Some(TextSpec {
                text: s.get_item("text")?.extract()?,
                size: s.get_item("text_size")?.extract()?,
                at: s.get_item("text_at")?.extract()?,
            }),
            _ => None,
        };
        rows.push((name, label, pattern, spec));
    }
    Ok(to_lin(&rows))
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(linetype_table, m)?)?;
    m.add_function(wrap_pyfunction!(py_clean_pattern, m)?)?;
    m.add_function(wrap_pyfunction!(py_drawn_dashes, m)?)?;
    m.add_function(wrap_pyfunction!(py_file_dashes, m)?)?;
    m.add_function(wrap_pyfunction!(py_text_offset, m)?)?;
    m.add_function(wrap_pyfunction!(py_text_width, m)?)?;
    m.add_function(wrap_pyfunction!(py_clean_line_text, m)?)?;
    m.add_function(wrap_pyfunction!(py_with_text, m)?)?;
    m.add_function(wrap_pyfunction!(py_text_marks, m)?)?;
    m.add_function(wrap_pyfunction!(py_dash_segments, m)?)?;
    m.add_function(wrap_pyfunction!(py_parse_lin, m)?)?;
    m.add_function(wrap_pyfunction!(py_to_lin, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashes() {
        assert_eq!(drawn_dashes(&[-1.0, 3.0, -1.5, 0.0, -1.5]), vec![3.0, 1.5, 0.0, 2.5]);
        assert_eq!(drawn_dashes(&[5.0]), vec![5.0, 0.0]);
        assert_eq!(file_dashes(&[3.0, 1.5]), vec![3.0, -1.5]);
        assert_eq!(clean_values(&[1.0, 2.0]), None);
        assert_eq!(clean_values(&[0.01, -0.01]), None);
        assert_eq!(split_pieces("5,[\"A\",S=2],-1"), vec!["5", "[\"A\",S=2]", "-1"]);
        assert_eq!(splitlines("a\r\nb\x0cc\n"), vec!["a", "b", "c"]);
        let _ = text_offset(&[3.0, -1.0], 1);
    }
}
