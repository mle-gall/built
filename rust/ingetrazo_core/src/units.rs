//! Units and typed values (R2.5 slice 3, `docs/rust-tools.md` §16).
//!
//! The one table of units (Python's `core.units` reads it from here), the
//! reading of one typed measurement (`parse`), calculations after `=`
//! (`parse_expression`) and the value box's grammar (`parse_value_buffer`:
//! several fields, arrays, radius, profile, segments, slopes). What a bare
//! number means comes from the caller's settings as `Ctx`.
//!
//! Positions in errors count characters, as Python's did.

use pyo3::prelude::*;
use pyo3::types::PyTuple;
use regex::Regex;
use std::sync::LazyLock;

pub const IN_M: f64 = 0.0254;
pub const FT_M: f64 = 0.3048;
/// The largest number a field takes (#185).
pub const MAX_TYPED: f64 = 1e9;

pub struct Unit {
    pub code: &'static str,
    pub kind: &'static str,
    pub factor: f64,
    pub symbol: &'static str,
    pub aliases: &'static [&'static str],
}

const PI: f64 = std::f64::consts::PI;

pub static TABLE: &[Unit] = &[
    Unit { code: "mm", kind: "length", factor: 1e-3, symbol: "mm", aliases: &["mm", "millimetre", "millimetres", "millimeter", "millimeters", "milímetro", "milímetros", "milimetro", "milimetros"] },
    Unit { code: "cm", kind: "length", factor: 1e-2, symbol: "cm", aliases: &["cm", "centimetre", "centimetres", "centimeter", "centimeters", "centímetro", "centímetros", "centimetro", "centimetros"] },
    Unit { code: "m", kind: "length", factor: 1.0, symbol: "m", aliases: &["m", "metre", "metres", "meter", "meters", "metro", "metros"] },
    Unit { code: "km", kind: "length", factor: 1e3, symbol: "km", aliases: &["km", "kilometre", "kilometres", "kilometer", "kilometers", "kilómetro", "kilómetros", "kilometro", "kilometros"] },
    Unit { code: "in", kind: "length", factor: IN_M, symbol: "\"", aliases: &["\"", "in", "inch", "inches", "pulgada", "pulgadas"] },
    Unit { code: "ft", kind: "length", factor: FT_M, symbol: "'", aliases: &["'", "ft", "foot", "feet", "pie", "pies"] },
    Unit { code: "yd", kind: "length", factor: 0.9144, symbol: "yd", aliases: &["yd", "yard", "yards", "yarda", "yardas"] },
    Unit { code: "mi", kind: "length", factor: 1609.344, symbol: "mi", aliases: &["mi", "mile", "miles", "milla", "millas"] },
    Unit { code: "us-ft", kind: "length", factor: 1200.0 / 3937.0, symbol: "ft (US)", aliases: &["us-ft", "usft", "survey-ft"] },
    Unit { code: "mm2", kind: "area", factor: 1e-6, symbol: "mm²", aliases: &["mm2", "mm²", "mm^2"] },
    Unit { code: "cm2", kind: "area", factor: 1e-4, symbol: "cm²", aliases: &["cm2", "cm²", "cm^2"] },
    Unit { code: "m2", kind: "area", factor: 1.0, symbol: "m²", aliases: &["m2", "m²", "m^2"] },
    Unit { code: "ha", kind: "area", factor: 1e4, symbol: "ha", aliases: &["ha", "hectare", "hectares", "hectárea", "hectáreas", "hectarea", "hectareas"] },
    Unit { code: "in2", kind: "area", factor: IN_M * IN_M, symbol: "in²", aliases: &["in2", "in²", "in^2"] },
    Unit { code: "ft2", kind: "area", factor: FT_M * FT_M, symbol: "ft²", aliases: &["ft2", "ft²", "ft^2"] },
    Unit { code: "yd2", kind: "area", factor: 0.9144 * 0.9144, symbol: "yd²", aliases: &["yd2", "yd²", "yd^2"] },
    Unit { code: "acre", kind: "area", factor: 4046.8564224, symbol: "acre", aliases: &["acre", "acres", "ac"] },
    Unit { code: "cm3", kind: "volume", factor: 1e-6, symbol: "cm³", aliases: &["cm3", "cm³", "cm^3", "cc"] },
    Unit { code: "l", kind: "volume", factor: 1e-3, symbol: "L", aliases: &["l", "litre", "litres", "liter", "liters", "litro", "litros"] },
    Unit { code: "m3", kind: "volume", factor: 1.0, symbol: "m³", aliases: &["m3", "m³", "m^3"] },
    Unit { code: "in3", kind: "volume", factor: IN_M * IN_M * IN_M, symbol: "in³", aliases: &["in3", "in³", "in^3"] },
    Unit { code: "ft3", kind: "volume", factor: FT_M * FT_M * FT_M, symbol: "ft³", aliases: &["ft3", "ft³", "ft^3"] },
    Unit { code: "yd3", kind: "volume", factor: 0.9144 * 0.9144 * 0.9144, symbol: "yd³", aliases: &["yd3", "yd³", "yd^3"] },
    Unit { code: "gal", kind: "volume", factor: 3.785411784e-3, symbol: "gal", aliases: &["gal", "gallon", "gallons", "galón", "galones", "galon"] },
    Unit { code: "deg", kind: "angle", factor: PI / 180.0, symbol: "°", aliases: &["°", "deg", "degree", "degrees", "grado", "grados"] },
    Unit { code: "rad", kind: "angle", factor: 1.0, symbol: "rad", aliases: &["rad", "radian", "radians", "radián", "radianes"] },
    Unit { code: "grad", kind: "angle", factor: PI / 200.0, symbol: "gon", aliases: &["gon", "grad", "gradian", "gradians"] },
];

pub fn alias(word: &str) -> Option<&'static Unit> {
    TABLE.iter().find(|u| u.aliases.iter().any(|a| a.to_lowercase() == word))
}

fn unit(code: &str) -> &'static Unit { TABLE.iter().find(|u| u.code == code).expect("unit code") }

fn re(p: &str) -> Regex { Regex::new(p).expect("pattern") }

const QTY: &str = r"(?:\d+[ -]+\d+/\d+|\d+/\d+|\d+(?:\.\d*)?|\.\d+)";
static NUMBER: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:(\d+)[ -]+)?(?:(\d+)/(\d+)|(\d+(?:\.\d*)?|\.\d+))$"));
static QTY_AT: LazyLock<Regex> = LazyLock::new(|| re(&format!("^{QTY}")));
static LETTERS_AT: LazyLock<Regex> = LazyLock::new(|| re(r"^[\p{L}\p{Nl}\p{No}]+"));
static FEET_INCHES: LazyLock<Regex> = LazyLock::new(|| re(&format!(
    r#"^(?P<ft>{QTY})\s*(?:'|ft|feet|foot|pies?)\s*(?:-\s*)?(?:(?P<inch>{QTY})\s*(?:"|in|inch|inches|pulgadas?)?)?$"#)));
static HYPHEN_FEET: LazyLock<Regex> = LazyLock::new(|| re(r"^(?P<ft>\d+)-(?P<inch>\d+(?:\.\d+)?(?:[ -]+\d+/\d+)?)$"));
static EXPR_FRACTION: LazyLock<Regex> = LazyLock::new(|| re(r#"^\d+(?: +\d+)?/\d+ *(?:[\p{L}\p{Nl}\p{No}]+|["'°])"#));
static WS: LazyLock<Regex> = LazyLock::new(|| re(r"\s+"));

fn chars(s: &str) -> usize { s.chars().count() }

fn digits(s: &str) -> Option<f64> {
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// `2`, `2.5`, `.5`, `3/4`, `1 1/2`, `1-1/2` → the number; None for a
/// zero denominator, `1-2` (two numbers), or past [`MAX_TYPED`].
pub fn number(text: &str) -> Option<f64> {
    let c = NUMBER.captures(text.trim())?;
    let mut v = if let (Some(n), Some(d)) = (c.get(2), c.get(3)) {
        let d = digits(d.as_str())?;
        if d == 0.0 {
            return None;
        }
        digits(n.as_str())? / d
    } else {
        if c.get(1).is_some() {
            return None;
        }
        let t = c.get(4)?.as_str();
        // Python's float() reads "12." and ".5"; Rust's parse does too.
        digits(t)?
    };
    if let Some(w) = c.get(1) {
        v += digits(w.as_str())?;
    }
    (v.is_finite() && v.abs() <= MAX_TYPED).then_some(v)
}

const VULGAR: &[(char, &str)] = &[
    ('½', "1/2"), ('⅓', "1/3"), ('⅔', "2/3"), ('¼', "1/4"), ('¾', "3/4"), ('⅕', "1/5"), ('⅖', "2/5"),
    ('⅗', "3/5"), ('⅘', "4/5"), ('⅙', "1/6"), ('⅚', "5/6"), ('⅐', "1/7"), ('⅛', "1/8"), ('⅜', "3/8"),
    ('⅝', "5/8"), ('⅞', "7/8"), ('⅑', "1/9"), ('⅒', "1/10"),
];

fn typographic(c: char) -> Option<char> {
    match c {
        '″' | '”' | '“' => Some('"'),
        '′' | '’' | '‘' => Some('\''),
        _ => None,
    }
}

/// Typographic quotes as `"` / `'`, fraction characters as digits
/// (`1⅜”` → `1 3/8"`).
pub fn plain_marks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if let Some((_, frac)) = VULGAR.iter().find(|(c, _)| *c == ch) {
            if out.chars().last().is_some_and(|c| c.is_ascii_digit() || c.is_numeric()) {
                out.push(' ');
            }
            out.push_str(frac);
        } else {
            out.push(typographic(ch).unwrap_or(ch));
        }
    }
    out
}

/// The marks of a typed field read plainly: `,` as `.`, smart quotes.
fn translate_marks(s: &str) -> String {
    s.chars().map(|c| if c == ',' { '.' } else { typographic(c).unwrap_or(c) }).collect()
}

fn py_strip(s: &str) -> &str { s.trim_matches(|c: char| c.is_whitespace()) }
fn py_lstrip(s: &str) -> &str { s.trim_start_matches(|c: char| c.is_whitespace()) }

/// What a bare number means and which forms the field reads, from the
/// caller's settings.
#[derive(Clone, Copy, Debug)]
pub struct Ctx {
    /// Base units per bare number for this field.
    pub bare: f64,
    /// `5-3 1/2` reads as feet-inches (an imperial field).
    pub hyphen_feet: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parsed {
    pub value: Option<f64>,
    pub error: &'static str,
    pub position: usize,
    pub word: String,
    pub explicit_measurement: bool,
}

impl Parsed {
    fn ok(v: f64) -> Self { Parsed { value: Some(v), ..Default::default() } }
    fn err(error: &'static str, position: usize) -> Self { Parsed { error, position, ..Default::default() } }
}

fn expected(kind: &str) -> &'static str {
    match kind {
        "area" => "Expected an area",
        "volume" => "Expected a volume",
        "angle" => "Expected an angle",
        _ => "Expected a length",
    }
}

/// One typed measurement of `kind` (length, area, volume, angle) in the
/// base unit, or an error: `1,20` and `1.20` alike, free spaces, Spanish
/// or English units, `1m 20`, the imperial marks, `5-3 1/2` in an imperial
/// field, a unit of another kind refused.
pub fn parse(text: &str, kind: &str, ctx: Ctx) -> Parsed {
    let raw = plain_marks(text);
    if py_lstrip(&raw).starts_with('=') {
        return parse_expression(&raw, kind, ctx);
    }
    let mut offset = chars(&raw) - chars(py_lstrip(&raw));
    let t = WS.replace_all(&translate_marks(&py_strip(&raw).to_lowercase()), " ").into_owned();
    if t.is_empty() {
        return Parsed::err("Type a value", 0);
    }
    let mut sign = 1.0;
    let mut t = t.as_str();
    if t.starts_with('+') || t.starts_with('-') {
        sign = if t.starts_with('-') { -1.0 } else { 1.0 };
        t = py_lstrip(&t[1..]);
        offset += 1;
    }
    if t.is_empty() {
        return Parsed::err("Type a number", offset);
    }
    let done = |v: Option<f64>| match v {
        None => Parsed::err("Not a number", offset),
        Some(v) => Parsed::ok(sign * v),
    };
    if kind == "length" {
        if let Some(c) = FEET_INCHES.captures(t) {
            let ft = number(&c["ft"]);
            let inch = match c.name("inch") { Some(m) => number(m.as_str()), None => Some(0.0) };
            return done(ft.zip(inch).map(|(f, i)| f * FT_M + i * IN_M));
        }
        if ctx.hyphen_feet {
            if let Some(c) = HYPHEN_FEET.captures(t) {
                let (ft, inch) = (number(&c["ft"]), number(&c["inch"]));
                return done(ft.zip(inch).map(|(f, i)| f * FT_M + i * IN_M));
            }
        }
    }
    // (number text, unit word, char position of the word or the number)
    let mut terms: Vec<(&str, Option<&str>, usize)> = Vec::new();
    let mut pos = 0usize;
    while pos < t.len() {
        let rest = &t[pos..];
        let Some(q) = QTY_AT.find(rest) else {
            return Parsed::err("Not a number", offset + chars(&t[..pos]));
        };
        let num = q.as_str();
        let num_at = pos;
        let mut p = pos + q.end();
        p += t[p..].len() - py_lstrip(&t[p..]).len();
        let mut word = None;
        let mut word_at = num_at;
        if let Some(l) = LETTERS_AT.find(&t[p..]) {
            let mut end = p + l.end();
            // `m2`, `m^2`, `m3` (a power), unless more number follows.
            let tail = &t[end..];
            let power = if let Some(s) = tail.strip_prefix('^') { Some((1, s)) } else { Some((0, tail)) };
            if let Some((skip, s)) = power {
                let mut cs = s.chars();
                if let Some(d @ ('2' | '3')) = cs.next() {
                    let next = cs.next();
                    if !next.is_some_and(|n| n.is_ascii_digit() || n.is_numeric() || n == '.' || n == '/') {
                        end += skip + d.len_utf8();
                    }
                }
            }
            word = Some(&t[p..end]);
            word_at = p;
            p = end;
        } else if let Some(c) = t[p..].chars().next().filter(|c| matches!(c, '°' | '\'' | '"')) {
            word = Some(&t[p..p + c.len_utf8()]);
            word_at = p;
            p += c.len_utf8();
        }
        p += t[p..].len() - py_lstrip(&t[p..]).len();
        if p == pos {
            return Parsed::err("Not a number", offset + chars(&t[..pos]));
        }
        terms.push((num, word, offset + chars(&t[..word_at])));
        pos = p;
    }
    let mut total = 0.0;
    let mut previous: Option<&str> = None;
    let n_terms = terms.len();
    for (i, (num_txt, word, at)) in terms.into_iter().enumerate() {
        let Some(num) = number(num_txt) else { return Parsed::err("Not a number", at) };
        let Some(word) = word else {
            if n_terms == 1 {
                return done(Some(num * ctx.bare));
            }
            let down = match previous { Some("m") => Some("cm"), Some("ft") => Some("in"), _ => None };
            let (true, Some(down)) = (i == n_terms - 1, down) else { return Parsed::err("Type one value", at) };
            total += num * unit(down).factor;
            break;
        };
        let Some(row) = alias(word) else {
            return Parsed { error: "Unknown unit: {unit}", position: at, word: word.into(), ..Default::default() };
        };
        if row.kind != kind {
            return Parsed { error: expected(kind), position: at, word: word.into(), ..Default::default() };
        }
        total += num * row.factor;
        previous = Some(row.code);
    }
    Parsed::ok(sign * total)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dim { Bare, Unit, Scalar }

struct Expr<'a> {
    tokens: Vec<(char, String, usize)>,
    pos: usize,
    end: usize,
    kind: &'a str,
    ctx: Ctx,
    depth: usize,
}

type E = Result<(f64, Dim), Parsed>;

impl Expr<'_> {
    fn peek(&self) -> Option<&(char, String, usize)> { self.tokens.get(self.pos) }
    fn take(&mut self) -> (char, String, usize) {
        self.pos += 1;
        self.tokens[self.pos - 1].clone()
    }
    fn err(&self, msg: &'static str, at: usize) -> Parsed { Parsed::err(msg, at) }

    fn measurement(&self, txt: &str, at: usize) -> E {
        if !txt.chars().any(|c| !(c.is_numeric() || c.is_whitespace() || c == '.' || c == '/')) {
            return number(txt).map(|v| (v, Dim::Bare)).ok_or_else(|| self.err("Not a number", at));
        }
        if txt.contains('=') {
            return Err(self.err("Not a number", at));
        }
        let r = parse(txt, self.kind, self.ctx);
        match r.value {
            Some(v) => Ok((v, Dim::Unit)),
            None => Err(Parsed { position: at + r.position, ..r }),
        }
    }

    fn as_unit(&self, v: f64, d: Dim) -> f64 { if d == Dim::Bare { v * self.ctx.bare } else { v } }

    fn factor(&mut self) -> E {
        self.depth += 1;
        if self.depth > 200 {
            return Err(self.err("Not a number", 0));
        }
        let r = self.factor_inner();
        self.depth -= 1;
        r
    }

    fn factor_inner(&mut self) -> E {
        let Some(tok) = self.peek().cloned() else { return Err(self.err("A number is missing", self.end)) };
        match tok.0 {
            '+' | '-' => {
                self.take();
                let (v, d) = self.factor()?;
                Ok((if tok.0 == '-' { -v } else { v }, d))
            }
            '(' => {
                self.take();
                let v = self.expression()?;
                match self.peek() {
                    Some(c) if c.0 == ')' => {
                        self.take();
                        Ok(v)
                    }
                    _ => Err(self.err("A parenthesis is not closed", tok.2)),
                }
            }
            'q' => {
                self.take();
                self.measurement(&tok.1, tok.2)
            }
            _ => Err(self.err("A number is missing", tok.2)),
        }
    }

    fn term(&mut self) -> E {
        let (mut a, mut au) = self.factor()?;
        while let Some(op) = self.peek().filter(|t| t.0 == '*' || t.0 == '/').cloned() {
            self.take();
            let (b, bu) = self.factor()?;
            if op.0 == '*' {
                if au == Dim::Unit && bu == Dim::Unit {
                    return Err(self.err("Two measurements can't be multiplied: one must be a plain number", op.2));
                }
                a *= b;
                if bu != Dim::Bare && au != Dim::Unit {
                    au = bu;
                }
            } else {
                if b == 0.0 {
                    return Err(self.err("Division by zero", op.2));
                }
                if bu == Dim::Unit && au != Dim::Unit {
                    return Err(self.err("A plain number can't be divided by a measurement", op.2));
                }
                a /= b;
                if bu == Dim::Unit || (bu == Dim::Scalar && au == Dim::Bare) {
                    au = Dim::Scalar;
                }
            }
        }
        Ok((a, au))
    }

    fn expression(&mut self) -> E {
        let (mut a, mut au) = self.term()?;
        while let Some(op) = self.peek().filter(|t| t.0 == '+' || t.0 == '-').cloned() {
            self.take();
            let (mut b, bu) = self.term()?;
            if au != bu {
                if au == Dim::Unit || bu == Dim::Unit {
                    a = self.as_unit(a, au);
                    b = self.as_unit(b, bu);
                    au = Dim::Unit;
                } else {
                    au = Dim::Scalar;
                }
            }
            a = if op.0 == '+' { a + b } else { a - b };
        }
        Ok((a, au))
    }
}

fn expr_tokens(text: &str, base: usize) -> Vec<(char, String, usize)> {
    let mut tokens = Vec::new();
    let mut i = 0usize;
    while i < text.len() {
        let c = text[i..].chars().next().unwrap();
        let at = base + chars(&text[..i]);
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        if "+-*/()".contains(c) {
            tokens.push((c, c.to_string(), at));
            i += 1;
            continue;
        }
        if let Some(m) = EXPR_FRACTION.find(&text[i..]) {
            tokens.push(('q', m.as_str().to_string(), at));
            i += m.end();
            continue;
        }
        let mut j = i;
        while j < text.len() {
            let d = text[j..].chars().next().unwrap();
            if "+-*/()".contains(d) {
                break;
            }
            j += d.len_utf8();
        }
        tokens.push(('q', text[i..j].trim_end_matches(|c: char| c.is_whitespace()).to_string(), at));
        i = j;
    }
    tokens
}

/// A calculation typed after `=` (#630): `+ - * /` and parentheses with
/// the usual precedence; a bare number is in the field's unit unless it
/// multiplies or divides a measurement.
pub fn parse_expression(text: &str, kind: &str, ctx: Ctx) -> Parsed {
    let raw = plain_marks(text);
    let start = chars(&raw) - chars(py_lstrip(&raw));
    let stripped = py_strip(&raw);
    let body = translate_marks(stripped.char_indices().nth(1).map(|(i, _)| &stripped[i..]).unwrap_or(""));
    let end = start + chars(stripped);
    let tokens = expr_tokens(&body, start + 1);
    if tokens.is_empty() {
        return Parsed::err("Type a calculation after =", start + 1);
    }
    let mut e = Expr { tokens, pos: 0, end, kind, ctx, depth: 0 };
    let result = e.expression().and_then(|v| match e.peek() {
        Some(t) => Err(Parsed::err(if t.0 == ')' { "A parenthesis closes nothing" } else { "An operator is missing" }, t.2)),
        None => Ok(v),
    });
    match result {
        Err(p) => p,
        Ok((v, d)) => {
            let v = e.as_unit(v, d);
            if !v.is_finite() || v.abs() > MAX_TYPED {
                return Parsed::err("Not a number", start);
            }
            Parsed { value: Some(v), explicit_measurement: d == Dim::Unit, ..Default::default() }
        }
    }
}

/// What the value box read.
#[derive(Clone, Debug, PartialEq)]
pub enum Typed {
    One(f64),
    Two(f64, f64),
    Three(f64, f64, f64),
    /// `3x` / `x3` (external), `/3` (internal), `5x10m` (count and step).
    Array { count: i64, mode: &'static str, step: Option<f64> },
    Radius(f64),
    Profile(f64),
    Segments(i64),
    /// A slope `rise:run`, in degrees.
    Ratio(f64),
}

static MIXED_WHOLE: LazyLock<Regex> = LazyLock::new(|| re(r"^-?\d+(?:'-?\d*)?$"));
static MIXED_FRAC: LazyLock<Regex> = LazyLock::new(|| re(r#"^\d+/\d+(?:mm|cm|m|in|ft|"|')?$"#));
static FEET_FIELD: LazyLock<Regex> = LazyLock::new(|| re(r"^-?[\d./-]+'$"));
static INCH_FIELD: LazyLock<Regex> = LazyLock::new(|| re(r#"^[\d./-]+(?:"|in)$"#));
static ARRAY_COUNT: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:(\d+)\s*[x*]|[x*]\s*(\d+))$"));
static ARRAY_STEP: LazyLock<Regex> = LazyLock::new(|| re(r"^(\d+)\s*[x*]\s*(.+)$"));
static ARRAY_DIVIDE: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:/\s*(\d+)|(\d+)\s*/)$"));
static RADIUS: LazyLock<Regex> = LazyLock::new(|| re(r"^(\d+\.?\d*|\.\d+)r$"));
static PROFILE: LazyLock<Regex> = LazyLock::new(|| re(r"^(\d+\.?\d*|\.\d+)p$"));
static SEGMENTS: LazyLock<Regex> = LazyLock::new(|| re(r"^(\d+)s$"));
static RATIO: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*(-?(?:\d+\.?\d*|\.\d+)):(\d+\.?\d*|\.\d+)\s*$"));
static COMMA_SPACE: LazyLock<Regex> = LazyLock::new(|| re(r",\s+"));
static EXPR_SPLIT: LazyLock<Regex> = LazyLock::new(|| re(r";|,\s+"));

/// `1 1/2"` is one mixed number; `5' 3 1/2"` one length.
fn merge_mixed(fields: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < fields.len() {
        let f = fields[i];
        if let Some(nxt) = fields.get(i + 1) {
            if MIXED_WHOLE.is_match(f) && MIXED_FRAC.is_match(&nxt.to_lowercase()) {
                let joint = if f.ends_with('\'') || f.ends_with('-') { "" } else { "-" };
                out.push(format!("{f}{joint}{nxt}"));
                i += 2;
                continue;
            }
        }
        out.push(f.to_string());
        i += 1;
    }
    let mut joined: Vec<String> = Vec::new();
    for f in out {
        if joined.last().is_some_and(|l| FEET_FIELD.is_match(l)) && INCH_FIELD.is_match(&f.to_lowercase()) {
            joined.last_mut().unwrap().push_str(&f);
        } else {
            joined.push(f);
        }
    }
    joined
}

fn int(s: &str) -> Option<i64> { s.parse().ok() }

/// The value box's text (`views.document_interaction._parse_value_buffer`
/// before R2.5 slice 3): one to three lengths, an array, a radius, a
/// profile, a segment count or a slope; None on an error.
pub fn parse_value_buffer(buffer: &str, comma_lists: bool, ctx: Ctx) -> Option<Typed> {
    let buffer = plain_marks(buffer);
    if py_lstrip(&buffer).starts_with('=') {
        let mut nums = Vec::new();
        for field in EXPR_SPLIT.split(py_strip(&buffer)) {
            let f = py_strip(field);
            let f = if f.starts_with('=') { f.to_string() } else { format!("={f}") };
            nums.push(parse(&f, "length", ctx).value?);
        }
        return tuple(&nums);
    }
    let mut buffer = COMMA_SPACE.replace_all(py_strip(&buffer), ";").into_owned();
    if comma_lists && !buffer.contains(';') && !py_strip(&buffer).chars().any(char::is_whitespace) {
        buffer = buffer.replace(',', " ");
    }
    let normalized = buffer.replace(',', ".").replace(';', " ");
    let stripped = py_strip(&normalized);
    let lower = stripped.to_lowercase();
    if let Some(c) = ARRAY_COUNT.captures(&lower) {
        return Some(Typed::Array { count: int(c.get(1).or(c.get(2))?.as_str())?, mode: "x", step: None });
    }
    if let Some(c) = ARRAY_STEP.captures(&lower) {
        return match parse_value_buffer(&c[2], false, ctx)? {
            Typed::One(step) if step > 0.0 => Some(Typed::Array { count: int(&c[1])?, mode: "x", step: Some(step) }),
            _ => None,
        };
    }
    if let Some(c) = ARRAY_DIVIDE.captures(stripped) {
        return Some(Typed::Array { count: int(c.get(1).or(c.get(2))?.as_str())?, mode: "/", step: None });
    }
    if lower.ends_with('r') && !stripped.contains(':') {
        return RADIUS.captures(&lower).and_then(|c| digits(&c[1])).map(Typed::Radius);
    }
    if lower.ends_with('p') && !stripped.contains(':') {
        return PROFILE.captures(&lower).and_then(|c| digits(&c[1])).map(Typed::Profile);
    }
    if lower.ends_with('s') && !stripped.contains(':') {
        return SEGMENTS.captures(&lower).and_then(|c| int(&c[1])).map(Typed::Segments);
    }
    if normalized.contains(':') {
        let c = RATIO.captures(&normalized)?;
        let (rise, run) = (c[1].parse::<f64>().ok()?, c[2].parse::<f64>().ok()?);
        if run <= 0.0 || !rise.is_finite() || !run.is_finite() {
            return None;
        }
        return Some(Typed::Ratio(rise.atan2(run).to_degrees()));
    }
    let mut nums = Vec::new();
    let dotted = buffer.replace(',', ".");
    for group in dotted.split(';') {
        let words: Vec<&str> = group.split_whitespace().collect();
        for f in merge_mixed(&words) {
            nums.push(parse(&f, "length", ctx).value?);
        }
    }
    tuple(&nums)
}

fn tuple(nums: &[f64]) -> Option<Typed> {
    match *nums {
        [a] => Some(Typed::One(a)),
        [a, b] => Some(Typed::Two(a, b)),
        [a, b, c] => Some(Typed::Three(a, b, c)),
        _ => None,
    }
}

// ---- Python -------------------------------------------------------------

/// `(code, kind, factor, symbol, aliases)` per unit: `core.units` builds
/// `UNITS` and the alias table from it.
#[pyfunction]
pub fn units_table() -> Vec<(&'static str, &'static str, f64, &'static str, Vec<&'static str>)> {
    TABLE.iter().map(|u| (u.code, u.kind, u.factor, u.symbol, u.aliases.to_vec())).collect()
}

/// The fraction characters (`½`, `⅜`…) and the digits a field reads.
#[pyfunction]
pub fn vulgar_fractions() -> Vec<(String, &'static str)> {
    VULGAR.iter().map(|(c, f)| (c.to_string(), *f)).collect()
}

#[pyfunction]
#[pyo3(name = "typed_number")]
pub fn py_number(text: &str) -> Option<f64> { number(text) }

#[pyfunction]
#[pyo3(name = "plain_marks")]
pub fn py_plain_marks(text: &str) -> String { plain_marks(text) }

/// `(value, error, position, word, explicit_measurement)`.
#[pyfunction]
#[pyo3(name = "parse_measurement")]
pub fn py_parse(text: &str, kind: &str, bare: f64, hyphen_feet: bool) -> (Option<f64>, &'static str, usize, String, bool) {
    let p = parse(text, kind, Ctx { bare, hyphen_feet });
    (p.value, p.error, p.position, p.word, p.explicit_measurement)
}

/// The value box's reading: a float, a tuple of 2 or 3, `("array", n,
/// "x" | "/"[, step])`, `("radius", r)`, `("profile", p)`,
/// `("segments", n)`, `("ratio", degrees)`, or None.
#[pyfunction]
#[pyo3(name = "parse_value_buffer")]
pub fn py_parse_value_buffer<'py>(py: Python<'py>, buffer: &str, comma_lists: bool, bare: f64, hyphen_feet: bool)
    -> PyResult<Bound<'py, PyAny>> {
    let ctx = Ctx { bare, hyphen_feet };
    let any = |v: Bound<'py, PyTuple>| v.into_any();
    Ok(match parse_value_buffer(buffer, comma_lists, ctx) {
        None => py.None().into_bound(py),
        Some(Typed::One(v)) => v.into_pyobject(py)?.into_any(),
        Some(Typed::Two(a, b)) => any((a, b).into_pyobject(py)?),
        Some(Typed::Three(a, b, c)) => any((a, b, c).into_pyobject(py)?),
        Some(Typed::Array { count, mode, step: None }) => any(("array", count, mode).into_pyobject(py)?),
        Some(Typed::Array { count, mode, step: Some(s) }) => any(("array", count, mode, s).into_pyobject(py)?),
        Some(Typed::Radius(r)) => any(("radius", r).into_pyobject(py)?),
        Some(Typed::Profile(p)) => any(("profile", p).into_pyobject(py)?),
        Some(Typed::Segments(n)) => any(("segments", n).into_pyobject(py)?),
        Some(Typed::Ratio(d)) => any(("ratio", d).into_pyobject(py)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: Ctx = Ctx { bare: 1.0, hyphen_feet: false };

    #[test]
    fn measurements() {
        assert_eq!(parse("1,20", "length", M).value, Some(1.2));
        assert_eq!(parse(" 120 cm ", "length", M).value, Some(1.2));
        assert!((parse("1m 20", "length", M).value.unwrap() - 1.2).abs() < 1e-12);
        assert!((parse("5' 3 1/2\"", "length", M).value.unwrap() - (5.0 * FT_M + 3.5 * IN_M)).abs() < 1e-12);
        assert_eq!(parse("5 m²", "length", M).error, "Expected a length");
        assert_eq!(parse("5 m2", "area", M).value, Some(5.0));
        assert_eq!(parse("5 parsecs", "length", M).error, "Unknown unit: {unit}");
        assert_eq!(parse("-2", "length", M).value, Some(-2.0));
        assert_eq!(number("1-1/2"), Some(1.5));
        assert_eq!(number("1-2"), None);
        assert_eq!(number("3/0"), None);
        assert_eq!(number("2000000000"), None);
    }

    #[test]
    fn expressions() {
        assert!((parse("=1.25*2+2.45/2", "length", M).value.unwrap() - 3.725).abs() < 1e-12);
        assert!((parse("=2*45cm", "length", M).value.unwrap() - 0.9).abs() < 1e-12);
        assert_eq!(parse("=2cm*3cm", "length", M).error, "Two measurements can't be multiplied: one must be a plain number");
        assert_eq!(parse("=(1+2", "length", M).error, "A parenthesis is not closed");
        assert_eq!(parse("=1/0", "length", M).error, "Division by zero");
    }

    #[test]
    fn value_box() {
        let v = |s: &str| parse_value_buffer(s, false, M);
        assert_eq!(v("2;3"), Some(Typed::Two(2.0, 3.0)));
        assert_eq!(v("2,5"), Some(Typed::One(2.5)));
        assert_eq!(parse_value_buffer("200,100", true, M), Some(Typed::Two(200.0, 100.0)));
        assert_eq!(v("3x"), Some(Typed::Array { count: 3, mode: "x", step: None }));
        assert_eq!(v("/4"), Some(Typed::Array { count: 4, mode: "/", step: None }));
        assert_eq!(v("5x10"), Some(Typed::Array { count: 5, mode: "x", step: Some(10.0) }));
        assert_eq!(v("0.5r"), Some(Typed::Radius(0.5)));
        assert_eq!(v("12s"), Some(Typed::Segments(12)));
        assert!(matches!(v("1:1"), Some(Typed::Ratio(d)) if (d - 45.0).abs() < 1e-9));
        assert_eq!(v("1 1/2\""), Some(Typed::One(1.5 * IN_M)));
        assert_eq!(v("=3/2;1+0,5"), Some(Typed::Two(1.5, 1.5)));
        assert_eq!(v("1;2;3;4"), None);
    }
}
