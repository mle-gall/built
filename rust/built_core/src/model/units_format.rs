//! How a stored value is shown: lengths, areas, volumes and angles in the
//! document's units.
//!
//! `built_core.units` was the formatter; it is now a facade over these
//! functions, which read the same unit table as the parser (`units.rs`).
//! Each function answers exactly what the Python one did
//! (`tests/test_units_format_parity.py` holds its answers):
//!
//! - `fixed` rounds half up on the number *as written* to 12 significant
//!   digits (Python's `Decimal(f"{v:.12g}").quantize(…, ROUND_HALF_UP)`):
//!   1.2345 m is «123.5 cm» at one decimal, never «-0.00».
//! - `{:.N}` and Python's `{:.Nf}` / `round(x, N)` both round the exact
//!   binary value half to even, so they are used as they are.
//! - Feet are split with Python's float floor division (`fmod` based), so
//!   a value a hair under 12" stays in the same foot.
//!
//! Settings are the document's `units` dictionary (`model_units_of`):
//! `length`, `precision`, and optionally `area`, `area_precision`,
//! `volume`, `volume_precision`, `angle`, `angle_precision`, `decimal`.

use crate::units::{FT_M, IN_M, TABLE, Unit};
use pyo3::exceptions::{PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString};
use regex::Regex;
use std::sync::LazyLock;

/// The finest fraction of an inch per precision step (fractional forms).
fn frac_denom(decimals: i64) -> i64 {
    match decimals.clamp(0, 4) {
        0 => 1,
        1 => 4,
        2 => 16,
        3 => 32,
        _ => 64,
    }
}

fn row(code: &str) -> Option<&'static Unit> {
    TABLE.iter().find(|u| u.code == code)
}

/// The length forms a document may pick (and that a bare number is read in).
const LENGTH_FORMS: [&str; 8] = ["m", "cm", "mm", "in", "ft", "ft-in", "in-frac", "ft-in-frac"];

fn is_length_form(u: &str) -> bool {
    LENGTH_FORMS.contains(&u)
}

fn metric_factor(u: &str) -> Option<f64> {
    match u {
        "m" => Some(1.0),
        "cm" => Some(100.0),
        "mm" => Some(1000.0),
        _ => None,
    }
}

/// Python's `repr` of a non-finite float, as an f-string shows it.
fn non_finite(v: f64) -> String {
    if v.is_nan() {
        "nan".into()
    } else if v > 0.0 {
        "inf".into()
    } else {
        "-inf".into()
    }
}

/// `value` to `n` decimals, rounded half up on its 12-significant-digit
/// decimal form; a value that rounds to zero has no sign.
pub fn fixed(value: f64, n: usize) -> String {
    if !value.is_finite() {
        return non_finite(value);
    }
    // 12 significant digits, correctly rounded (what `%.12g` gives).
    let sci = format!("{:.11e}", value.abs());
    let (mant, exp) = sci.split_once('e').expect("scientific form");
    let exp: i64 = exp.parse().expect("exponent");
    let digits: Vec<u8> = mant.bytes().filter(u8::is_ascii_digit).map(|b| b - b'0').collect();
    // Digit i has the place value 10^(exp - i); keep those down to 10^-n.
    let keep = exp + n as i64 + 1;
    let mut scaled: Vec<u8> = Vec::new(); // value * 10^n, most significant first
    let mut round_up = false;
    if keep >= 0 {
        let k = keep as usize;
        for i in 0..k {
            scaled.push(*digits.get(i).unwrap_or(&0));
        }
        round_up = digits.get(k).is_some_and(|d| *d >= 5);
    }
    if round_up {
        let mut i = scaled.len();
        loop {
            if i == 0 {
                scaled.insert(0, 1);
                break;
            }
            i -= 1;
            if scaled[i] == 9 {
                scaled[i] = 0;
            } else {
                scaled[i] += 1;
                break;
            }
        }
    }
    while scaled.len() < n + 1 {
        scaled.insert(0, 0);
    }
    // Leading zeros of the integer part go, one stays.
    let int_len = scaled.len() - n;
    let first = scaled[..int_len - 1].iter().position(|d| *d != 0).unwrap_or(int_len - 1);
    let zero = scaled.iter().all(|d| *d == 0);
    let mut out = String::with_capacity(scaled.len() + 2);
    if value.is_sign_negative() && !zero {
        out.push('-');
    }
    for d in &scaled[first..int_len] {
        out.push((b'0' + d) as char);
    }
    if n > 0 {
        out.push('.');
        for d in &scaled[int_len..] {
            out.push((b'0' + d) as char);
        }
    }
    out
}

/// `6.5` → `6 1/2`; `0.75` → `3/4`; `6.0` → `6`.
pub fn frac_inches(inches: f64, decimals: i64) -> String {
    let denom = frac_denom(decimals) as i128;
    let total = (inches * denom as f64).round_ties_even() as i128;
    let (whole, rest) = (total / denom, total % denom);
    if rest == 0 {
        return whole.to_string();
    }
    let g = gcd(rest.abs(), denom);
    let (num, den) = (rest / g, denom / g);
    if whole == 0 {
        format!("{num}/{den}")
    } else {
        format!("{whole} {num}/{den}")
    }
}

fn gcd(mut a: i128, mut b: i128) -> i128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Python's `a // b` for floats.
fn py_floordiv(vx: f64, wx: f64) -> f64 {
    let m = vx % wx;
    let mut div = (vx - m) / wx;
    if m != 0.0 && ((wx < 0.0) != (m < 0.0)) {
        div -= 1.0;
    }
    if div != 0.0 {
        let f = div.floor();
        if div - f > 0.5 { f + 1.0 } else { f }
    } else {
        0.0f64.copysign(vx / wx)
    }
}

/// `metres` in `units` to `decimals` (clamped to 0–6). `show_unit` false
/// leaves the bare number; feet and inches together keep their marks.
pub fn format_length(metres: f64, units: &str, decimals: i64, show_unit: bool) -> String {
    let n = decimals.clamp(0, 6) as usize;
    let u = if units.is_empty() { "m" } else { units };
    if !metres.is_finite() {
        return "—".into();
    }
    let mark = |m: &'static str| if show_unit { m } else { "" };
    match u {
        "in" => return format!("{}{}", fixed(metres / IN_M, n), mark("\"")),
        "ft" => return format!("{}{}", fixed(metres / FT_M, n), mark("'")),
        "ft-in" | "ft-in-frac" | "in-frac" => {
            let sign = if metres < 0.0 { "-" } else { "" };
            let total_in = metres.abs() / IN_M;
            if u == "in-frac" {
                let txt = frac_inches(total_in, n as i64);
                let sign = if txt != "0" { sign } else { "" };
                return format!("{sign}{txt}{}", mark("\""));
            }
            let mut feet = py_floordiv(total_in, 12.0) as i128;
            let mut inches = total_in - (feet as f64) * 12.0;
            if u == "ft-in" {
                if crate::model::pyfloat::round(inches, n) >= 12.0 {
                    feet += 1;
                    inches = 0.0;
                }
                let txt = format!("{inches:.n$}");
                let zero = feet == 0 && txt.parse::<f64>().unwrap_or(1.0) == 0.0;
                return format!("{}{feet}'{txt}\"", if zero { "" } else { sign });
            }
            let mut txt = frac_inches(inches, n as i64);
            if txt == "12" {
                feet += 1;
                txt = "0".into();
            }
            let zero = feet == 0 && txt == "0";
            return format!("{}{feet}'{txt}\"", if zero { "" } else { sign });
        }
        _ => {}
    }
    if let Some(r) = row(u).filter(|r| r.kind == "length") {
        let number = fixed(metres / r.factor, n);
        return if show_unit { format!("{number} {}", r.symbol) } else { number };
    }
    let number = fixed(metres, n);
    if show_unit { format!("{number} m") } else { number }
}

/// A level mark's number: metres as «+0.15» / «-0.30» / «±0.00»; any other
/// unit is that sign in front of `format_length`.
pub fn format_signed_level(metres: f64, units: &str, decimals: i64) -> String {
    let u = if units.is_empty() { "m" } else { units };
    let d = decimals.max(0) as usize;
    let v = metres;
    if u == "m" {
        if crate::model::pyfloat::round(v, d) == 0.0 {
            return format!("±{:.d$}", 0.0);
        }
        if !v.is_finite() {
            return format!("{}{}", if v.is_sign_negative() && !v.is_nan() { "" } else { "+" }, non_finite(v));
        }
        return format!("{v:+.d$}");
    }
    let shown = format_length(v.abs(), u, d as i64, true);
    if shown == format_length(0.0, u, d as i64, true) {
        return format!("±{shown}");
    }
    if v < 0.0 {
        return format_length(v, u, d as i64, true);
    }
    format!("+{shown}")
}

/// `value` (in the base unit of its kind) in `unit`; `None` for a code the
/// table does not have.
pub fn format_value(value: f64, unit: &str, decimals: i64, symbol: bool) -> Option<String> {
    if is_length_form(unit) && metric_factor(unit).is_none() {
        return Some(format_length(value, unit, decimals, true)); // imperial forms
    }
    let r = row(unit)?;
    if !value.is_finite() {
        return Some("—".into());
    }
    let txt = fixed(value / r.factor, decimals.clamp(0, 6) as usize);
    if !symbol {
        return Some(txt);
    }
    let glue = if matches!(r.symbol, "°" | "\"" | "'") { "" } else { " " };
    Some(format!("{txt}{glue}{}", r.symbol))
}

pub fn area_unit_for(length: &str) -> &'static str {
    match length {
        "m" | "km" => "m2",
        "cm" => "cm2",
        "mm" => "mm2",
        "in" | "in-frac" => "in2",
        "ft" | "ft-in" | "ft-in-frac" => "ft2",
        "yd" => "yd2",
        "mi" => "acre",
        _ => "m2",
    }
}

pub fn volume_unit_for(length: &str) -> &'static str {
    match length {
        "m" | "km" => "m3",
        "cm" | "mm" => "cm3",
        "in" | "in-frac" => "in3",
        "ft" | "ft-in" | "ft-in-frac" => "ft3",
        "yd" | "mi" => "yd3",
        _ => "m3",
    }
}

/// The fewest decimals that still resolve a millimetre (or 1/16").
fn fine_decimals(u: &str) -> i64 {
    match u {
        "m" | "ft" => 3,
        "cm" => 1,
        "mm" => 0,
        "in" | "ft-in" | "in-frac" | "ft-in-frac" => 2,
        _ => 3,
    }
}

// ---- Settings ---------------------------------------------------------------

/// The document's units as the formatters read them (`unit_setting`).
pub struct Settings {
    pub length: String,
    pub precision: i64,
    pub area: String,
    pub area_precision: i64,
    pub volume: String,
    pub volume_precision: i64,
    pub angle: String,
    pub angle_precision: i64,
    pub comma: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            length: "m".into(),
            precision: 2,
            area: "auto".into(),
            area_precision: 2,
            volume: "auto".into(),
            volume_precision: 3,
            angle: "deg".into(),
            angle_precision: 1,
            comma: false,
        }
    }
}

impl Settings {
    fn localized(&self, text: String) -> String {
        if self.comma { text.replace('.', ",") } else { text }
    }
}

/// Python's `int(value)`.
fn py_int(value: &Bound<'_, PyAny>) -> PyResult<i64> {
    if let Ok(v) = value.extract::<i64>() {
        return Ok(v);
    }
    let int = value.py().get_type::<pyo3::types::PyInt>();
    int.call1((value,))?.extract::<i64>()
}

fn get<'py>(map: &Bound<'py, PyAny>, key: &str) -> PyResult<Option<Bound<'py, PyAny>>> {
    if let Ok(d) = map.cast::<PyDict>() {
        return d.get_item(key);
    }
    if map.contains(key)? { Ok(Some(map.get_item(key)?)) } else { Ok(None) }
}

fn get_str(map: &Bound<'_, PyAny>, key: &str, default: &str) -> PyResult<String> {
    match get(map, key)? {
        None => Ok(default.into()),
        Some(v) if v.is_none() => Ok(default.into()),
        Some(v) => v.extract::<String>(),
    }
}

fn get_int(map: &Bound<'_, PyAny>, key: &str, default: i64) -> PyResult<i64> {
    match get(map, key)? {
        None => Ok(default),
        Some(v) => py_int(&v),
    }
}

impl Settings {
    /// From the document's `units` dictionary, as `unit_setting` reads it.
    pub fn read(map: &Bound<'_, PyAny>) -> PyResult<Self> {
        let precision = get_int(map, "precision", 2)?;
        Ok(Settings {
            length: get_str(map, "length", "m")?,
            precision,
            area: get_str(map, "area", "auto")?,
            area_precision: get_int(map, "area_precision", precision)?,
            volume: get_str(map, "volume", "auto")?,
            volume_precision: get_int(map, "volume_precision", 3)?,
            angle: get_str(map, "angle", "deg")?,
            angle_precision: get_int(map, "angle_precision", 1)?,
            comma: get_str(map, "decimal", ".")? == ",",
        })
    }
}

pub fn fmt_len_with(metres: f64, s: &Settings) -> String {
    s.localized(format_length(metres, &s.length, s.precision, true))
}

pub fn fmt_area_with(square_metres: f64, s: &Settings) -> Option<String> {
    let n = s.area_precision.max(0) as usize;
    if s.area != "auto" {
        return Some(s.localized(format_value(square_metres, &s.area, s.area_precision, true)?));
    }
    let u = s.length.as_str();
    let txt = if let Some(f) = metric_factor(u) {
        let f = f * f;
        format!("{} {u}²", fixed(square_metres * f, n))
    } else if u.starts_with("ft") {
        format!("{} ft²", fixed(square_metres / (FT_M * FT_M), n))
    } else {
        format!("{} in²", fixed(square_metres / (IN_M * IN_M), n))
    };
    Some(s.localized(txt))
}

pub fn fmt_volume_with(cubic_metres: f64, s: &Settings) -> Option<String> {
    let unit = if s.volume == "auto" { volume_unit_for(&s.length) } else { s.volume.as_str() };
    Some(s.localized(format_value(cubic_metres, unit, s.volume_precision, true)?))
}

pub fn fmt_angle_with(radians: f64, s: &Settings, signed: bool) -> Option<String> {
    let mut txt = format_value(radians, &s.angle, s.angle_precision, true)?;
    if signed && !txt.starts_with('-') && !txt.starts_with('—') {
        txt.insert(0, '+');
    }
    Some(s.localized(txt))
}

pub fn fmt_num_with(metres: f64, s: &Settings) -> String {
    match metric_factor(&s.length) {
        Some(f) => s.localized(fixed(metres * f, s.precision.max(0) as usize)),
        None => fmt_len_with(metres, s),
    }
}

pub fn fmt_pair_with(a: f64, b: f64, s: &Settings) -> String {
    if metric_factor(&s.length).is_some() {
        format!("{} × {}", fmt_num_with(a, s), fmt_len_with(b, s))
    } else {
        format!("{} × {}", fmt_len_with(a, s), fmt_len_with(b, s))
    }
}

pub fn fine_precision(s: &Settings) -> i64 {
    s.precision.max(fine_decimals(&s.length))
}

pub fn fmt_len_fine_with(metres: f64, s: &Settings) -> String {
    s.localized(format_length(metres, &s.length, fine_precision(s), true))
}

pub fn fmt_triple_with(a: f64, b: f64, c: f64, s: &Settings, fine: bool) -> String {
    let n = if fine { fine_precision(s) } else { s.precision };
    let u = s.length.as_str();
    if let Some(f) = metric_factor(u) {
        let nn = n.max(0) as usize;
        return s.localized(format!("{} × {} × {}", fixed(a * f, nn), fixed(b * f, nn),
            format_length(c, u, n, true)));
    }
    s.localized([a, b, c].map(|v| format_length(v, u, n, true)).join(" × "))
}

/// The units panel's preview of 1.2345 m, m², m³ and rad.
pub fn preview_text(s: &Settings) -> Option<String> {
    let v = 1.2345;
    let given = s.localized("1.2345".into());
    Some([
        format!("{given} m → {}", fmt_len_with(v, s)),
        format!("{given} m² → {}", fmt_area_with(v, s)?),
        format!("{given} m³ → {}", fmt_volume_with(v, s)?),
        format!("{given} rad → {}", fmt_angle_with(v, s, false)?),
    ]
    .join("\n"))
}

/// What the value box shows while a bare number is typed: the number with
/// the mark of the unit it will be read in.
pub fn typed_value_text(buffer: &str, unit: &str) -> String {
    let text = buffer.trim();
    if !text.is_empty() && is_bare_list(text) {
        let mark = match unit {
            "m" => " m",
            "cm" => " cm",
            "mm" => " mm",
            "ft" => "'",
            "in" | "ft-in" | "in-frac" | "ft-in-frac" => "\"",
            _ => " m",
        };
        return format!("{text}{mark}");
    }
    text.to_string()
}

/// A bare number or a `,`/`;` list of them, the whole text (Unicode
/// digits and spaces, as Python's `re` reads `\d` and `\s`).
fn is_bare_list(text: &str) -> bool {
    static BARE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^[-+]?[\d.]+(\s*[,;]\s*[-+]?[\d.]+)*$").expect("bare list")
    });
    BARE.is_match(text)
}

/// One step of the shown precision, in metres (`None` for an unknown unit).
pub fn display_step(unit: &str, decimals: i64) -> Option<f64> {
    if unit == "in-frac" || unit == "ft-in-frac" {
        return Some(IN_M / frac_denom(decimals) as f64);
    }
    let bare = match unit {
        "m" => 1.0,
        "cm" => 0.01,
        "mm" => 0.001,
        "in" | "ft-in" | "in-frac" | "ft-in-frac" => IN_M,
        "ft" => FT_M,
        _ => row(unit)?.factor,
    };
    Some(10f64.powf(-(decimals.max(0) as f64)) * bare)
}

// ---- The document's units, validated ----------------------------------------

/// The units panel's optional settings, in order, with their defaults.
const OPTIONAL: [&str; 7] = ["area", "area_precision", "volume", "volume_precision",
                             "angle", "angle_precision", "decimal"];

fn of_kind(v: &Bound<'_, PyAny>, kind: &str) -> bool {
    v.extract::<String>().is_ok_and(|c| c == "auto" || row(&c).is_some_and(|r| r.kind == kind))
}

fn is_precision(v: &Bound<'_, PyAny>) -> bool {
    v.is_instance_of::<pyo3::types::PyInt>()
        && !v.is_instance_of::<pyo3::types::PyBool>()
        && v.extract::<i64>().is_ok_and(|n| (0..=6).contains(&n))
}

fn str_in(v: &Bound<'_, PyAny>, options: &[&str]) -> bool {
    v.cast::<PyString>().is_ok_and(|s| s.to_str().is_ok_and(|s| options.contains(&s)))
}

fn is_default(key: &str, v: &Bound<'_, PyAny>) -> PyResult<bool> {
    Ok(match key {
        "area" | "volume" => str_in(v, &["auto"]),
        "area_precision" => v.is_none(),
        "volume_precision" => v.eq(3)?,
        "angle" => str_in(v, &["deg"]),
        "angle_precision" => v.eq(1)?,
        _ => str_in(v, &["."]),
    })
}

fn valid(key: &str, v: &Bound<'_, PyAny>) -> bool {
    match key {
        "area" => of_kind(v, "area"),
        "volume" => of_kind(v, "volume"),
        "angle" => str_in(v, &["deg", "rad", "grad"]),
        "decimal" => str_in(v, &[".", ","]),
        _ => is_precision(v),
    }
}

/// `max(0, min(6, int(value)))`, or `default` when `int` refuses it.
fn clean_precision(value: Option<Bound<'_, PyAny>>, default: i64) -> PyResult<i64> {
    let Some(value) = value else { return Ok(default) };
    match py_int(&value) {
        Ok(n) => Ok(n.clamp(0, 6)),
        Err(e) if e.is_instance_of::<PyTypeError>(value.py()) || e.is_instance_of::<PyValueError>(value.py()) => Ok(default),
        Err(e) => Err(e),
    }
}

/// A raw `units` value (from a scene or a file) as the document's settings:
/// its length form and precision, and the optional settings that are valid
/// and not at their default; metres to two decimals when it names no
/// length form.
#[pyfunction]
#[pyo3(name = "units_clean")]
pub fn py_clean_units<'py>(raw: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyDict>> {
    let py = raw.py();
    let out = PyDict::new(py);
    let length = raw
        .cast::<PyDict>()
        .ok()
        .and_then(|d| d.get_item("length").ok().flatten().map(|l| (d.clone(), l)))
        .filter(|(_, l)| l.cast::<PyString>().is_ok_and(|s| s.to_str().is_ok_and(is_length_form)));
    let Some((d, length)) = length else {
        out.set_item("length", "m")?;
        out.set_item("precision", 2)?;
        return Ok(out);
    };
    out.set_item("length", length)?;
    out.set_item("precision", clean_precision(d.get_item("precision")?, 2)?)?;
    for key in OPTIONAL {
        if let Some(v) = d.get_item(key)? {
            if valid(key, &v) && !is_default(key, &v)? {
                if key.ends_with("precision") {
                    out.set_item(key, py_int(&v)?)?;
                } else {
                    out.set_item(key, v)?;
                }
            }
        }
    }
    Ok(out)
}

/// The profiles of the units panel: name, length form and precision.
pub const PROFILES: [(&str, &str, i64); 4] = [
    ("metric", "m", 2),
    ("centimetric", "cm", 1),
    ("millimetric", "mm", 0),
    ("imperial", "ft-in-frac", 2),
];

#[pyfunction]
#[pyo3(name = "units_profiles")]
pub fn py_profiles() -> Vec<(&'static str, &'static str, i64)> {
    PROFILES.to_vec()
}

/// The profile `settings` match (decimal separator aside), or `"custom"`.
#[pyfunction]
#[pyo3(name = "units_profile_of")]
pub fn py_profile_of(settings: &Bound<'_, PyAny>) -> PyResult<&'static str> {
    let clean = py_clean_units(settings)?;
    if clean.len() - usize::from(clean.contains("decimal")?) != 2 {
        return Ok("custom");
    }
    let length: String = clean.get_item("length")?.map_or(Ok(String::new()), |v| v.extract())?;
    let precision: i64 = clean.get_item("precision")?.map_or(Ok(-1), |v| v.extract())?;
    Ok(PROFILES.iter().find(|p| p.1 == length && p.2 == precision).map_or("custom", |p| p.0))
}

/// `settings` switched to the profile `name`, its decimal separator kept.
#[pyfunction]
#[pyo3(name = "units_with_profile")]
pub fn py_with_profile<'py>(settings: &Bound<'py, PyAny>, name: &str) -> PyResult<Bound<'py, PyDict>> {
    let py = settings.py();
    let p = PROFILES.iter().find(|p| p.0 == name).ok_or_else(|| PyKeyError::new_err(name.to_string()))?;
    let out = PyDict::new(py);
    out.set_item("length", p.1)?;
    out.set_item("precision", p.2)?;
    let clean = py_clean_units(settings)?;
    if let Some(sep) = clean.get_item("decimal")? {
        out.set_item("decimal", sep)?;
    }
    Ok(out)
}

// ---- Python -----------------------------------------------------------------

fn unknown(unit: &str) -> PyErr {
    PyKeyError::new_err(unit.to_string())
}

#[pyfunction]
#[pyo3(name = "units_format_length", signature = (metres, units, decimals, show_unit=true))]
pub fn py_format_length(metres: f64, units: &str, decimals: i64, show_unit: bool) -> String {
    format_length(metres, units, decimals, show_unit)
}

#[pyfunction]
#[pyo3(name = "units_format_signed_level")]
pub fn py_format_signed_level(metres: f64, units: &str, decimals: i64) -> String {
    format_signed_level(metres, units, decimals)
}

#[pyfunction]
#[pyo3(name = "units_format_value", signature = (value, unit, decimals=2, symbol=true))]
pub fn py_format_value(value: f64, unit: &str, decimals: i64, symbol: bool) -> PyResult<String> {
    format_value(value, unit, decimals, symbol).ok_or_else(|| unknown(unit))
}

#[pyfunction]
#[pyo3(name = "units_fmt_len_with")]
pub fn py_fmt_len_with(metres: f64, settings: &Bound<'_, PyAny>) -> PyResult<String> {
    let settings = Settings::read(settings)?;
    Ok(fmt_len_with(metres, &settings))
}

#[pyfunction]
#[pyo3(name = "units_fmt_area_with")]
pub fn py_fmt_area_with(square_metres: f64, settings: &Bound<'_, PyAny>) -> PyResult<String> {
    let settings = Settings::read(settings)?;
    fmt_area_with(square_metres, &settings).ok_or_else(|| unknown(&settings.area))
}

#[pyfunction]
#[pyo3(name = "units_fmt_volume_with")]
pub fn py_fmt_volume_with(cubic_metres: f64, settings: &Bound<'_, PyAny>) -> PyResult<String> {
    let settings = Settings::read(settings)?;
    fmt_volume_with(cubic_metres, &settings).ok_or_else(|| unknown(&settings.volume))
}

#[pyfunction]
#[pyo3(name = "units_fmt_angle_with", signature = (radians, settings, signed=false))]
pub fn py_fmt_angle_with(radians: f64, settings: &Bound<'_, PyAny>, signed: bool) -> PyResult<String> {
    let settings = Settings::read(settings)?;
    fmt_angle_with(radians, &settings, signed).ok_or_else(|| unknown(&settings.angle))
}

#[pyfunction]
#[pyo3(name = "units_fmt_num_with")]
pub fn py_fmt_num_with(metres: f64, settings: &Bound<'_, PyAny>) -> PyResult<String> {
    let settings = Settings::read(settings)?;
    Ok(fmt_num_with(metres, &settings))
}

#[pyfunction]
#[pyo3(name = "units_fmt_pair_with")]
pub fn py_fmt_pair_with(a: f64, b: f64, settings: &Bound<'_, PyAny>) -> PyResult<String> {
    let settings = Settings::read(settings)?;
    Ok(fmt_pair_with(a, b, &settings))
}

#[pyfunction]
#[pyo3(name = "units_fmt_len_fine_with")]
pub fn py_fmt_len_fine_with(metres: f64, settings: &Bound<'_, PyAny>) -> PyResult<String> {
    let settings = Settings::read(settings)?;
    Ok(fmt_len_fine_with(metres, &settings))
}

#[pyfunction]
#[pyo3(name = "units_fmt_triple_with", signature = (a, b, c, settings, fine=false))]
pub fn py_fmt_triple_with(a: f64, b: f64, c: f64, settings: &Bound<'_, PyAny>, fine: bool) -> PyResult<String> {
    let settings = Settings::read(settings)?;
    Ok(fmt_triple_with(a, b, c, &settings, fine))
}

#[pyfunction]
#[pyo3(name = "units_fine_precision")]
pub fn py_fine_precision(settings: &Bound<'_, PyAny>) -> PyResult<i64> {
    let settings = Settings::read(settings)?;
    Ok(fine_precision(&settings))
}

#[pyfunction]
#[pyo3(name = "units_preview_text")]
pub fn py_preview_text(settings: &Bound<'_, PyAny>) -> PyResult<String> {
    let settings = Settings::read(settings)?;
    preview_text(&settings).ok_or_else(|| unknown(&settings.area))
}

#[pyfunction]
#[pyo3(name = "units_typed_value_text")]
pub fn py_typed_value_text(buffer: &str, unit: &str) -> String {
    typed_value_text(buffer, unit)
}

#[pyfunction]
#[pyo3(name = "units_display_step")]
pub fn py_display_step(unit: &str, decimals: i64) -> PyResult<f64> {
    display_step(unit, decimals).ok_or_else(|| unknown(unit))
}

#[pyfunction]
#[pyo3(name = "units_area_unit_for")]
pub fn py_area_unit_for(length: &str) -> &'static str {
    area_unit_for(length)
}

#[pyfunction]
#[pyo3(name = "units_volume_unit_for")]
pub fn py_volume_unit_for(length: &str) -> &'static str {
    volume_unit_for(length)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(py_format_length, m)?)?;
    m.add_function(wrap_pyfunction!(py_format_signed_level, m)?)?;
    m.add_function(wrap_pyfunction!(py_format_value, m)?)?;
    m.add_function(wrap_pyfunction!(py_fmt_len_with, m)?)?;
    m.add_function(wrap_pyfunction!(py_fmt_area_with, m)?)?;
    m.add_function(wrap_pyfunction!(py_fmt_volume_with, m)?)?;
    m.add_function(wrap_pyfunction!(py_fmt_angle_with, m)?)?;
    m.add_function(wrap_pyfunction!(py_fmt_num_with, m)?)?;
    m.add_function(wrap_pyfunction!(py_fmt_pair_with, m)?)?;
    m.add_function(wrap_pyfunction!(py_fmt_len_fine_with, m)?)?;
    m.add_function(wrap_pyfunction!(py_fmt_triple_with, m)?)?;
    m.add_function(wrap_pyfunction!(py_fine_precision, m)?)?;
    m.add_function(wrap_pyfunction!(py_preview_text, m)?)?;
    m.add_function(wrap_pyfunction!(py_typed_value_text, m)?)?;
    m.add_function(wrap_pyfunction!(py_display_step, m)?)?;
    m.add_function(wrap_pyfunction!(py_area_unit_for, m)?)?;
    m.add_function(wrap_pyfunction!(py_volume_unit_for, m)?)?;
    m.add_function(wrap_pyfunction!(py_clean_units, m)?)?;
    m.add_function(wrap_pyfunction!(py_profiles, m)?)?;
    m.add_function(wrap_pyfunction!(py_profile_of, m)?)?;
    m.add_function(wrap_pyfunction!(py_with_profile, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_rounds_half_up_as_written() {
        assert_eq!(fixed(123.45, 1), "123.5");
        assert_eq!(fixed(2.675, 2), "2.68");
        assert_eq!(fixed(-0.004, 2), "0.00");
        assert_eq!(fixed(-0.005, 2), "-0.01");
        assert_eq!(fixed(0.0, 0), "0");
        assert_eq!(fixed(9.999, 2), "10.00");
        assert_eq!(fixed(1e-9, 2), "0.00");
        assert_eq!(fixed(1e21, 2), "1000000000000000000000.00");
        assert_eq!(fixed(f64::NAN, 2), "nan");
    }

    #[test]
    fn lengths() {
        assert_eq!(format_length(1.2345, "cm", 1, true), "123.5 cm");
        assert_eq!(format_length(0.0381, "in-frac", 2, true), "1 1/2\"");
        assert_eq!(format_length(0.4699, "ft-in-frac", 2, true), "1'6 1/2\"");
        assert_eq!(format_length(-0.3048, "ft-in", 0, true), "-1'0\"");
        assert_eq!(format_length(1.3, "m", 2, false), "1.30");
        assert_eq!(format_length(f64::NAN, "m", 2, true), "—");
        assert_eq!(format_signed_level(0.15, "m", 2), "+0.15");
        assert_eq!(format_signed_level(-0.001, "m", 2), "±0.00");
    }

    #[test]
    fn typed_lists() {
        assert_eq!(typed_value_text(" 500 ", "mm"), "500 mm");
        assert_eq!(typed_value_text("2 ; 3", "ft-in-frac"), "2 ; 3\"");
        assert_eq!(typed_value_text("5m", "m"), "5m");
    }
}
