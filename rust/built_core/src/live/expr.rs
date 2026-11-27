// SPDX-License-Identifier: GPL-3.0-or-later
//! Detail expressions contain only arithmetic, named dimensions and pure functions.
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
enum Expr {
    Number(f64),
    Name(String),
    List(Vec<Expr>),
    Op(char, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}
struct Parser<'a>(&'a str);
impl Parser<'_> {
    fn peek(&mut self) -> Option<char> {
        self.0 = self.0.trim_start();
        self.0.chars().next()
    }
    fn take(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.0 = &self.0[c.len_utf8()..];
            true
        } else {
            false
        }
    }
    fn list(&mut self, end: char) -> Result<Vec<Expr>, String> {
        let mut items = vec![];
        if self.take(end) {
            return Ok(items);
        }
        loop {
            items.push(self.expr(0)?);
            if self.take(end) {
                return Ok(items);
            }
            if !self.take(',') {
                return Err(format!("Expected ',' or '{end}'"));
            }
        }
    }
    fn expr(&mut self, prec: u8) -> Result<Expr, String> {
        let c = self.peek().ok_or("Expected a value")?;
        let mut lhs = if self.take('-') || self.take('−') {
            Expr::Op('-', Box::new(Expr::Number(0.)), Box::new(self.expr(3)?))
        } else if self.take('(') {
            let e = self.expr(0)?;
            if !self.take(')') {
                return Err("Missing ')'".into());
            }
            e
        } else if self.take('[') {
            Expr::List(self.list(']')?)
        } else if c.is_ascii_alphabetic() || c == '_' {
            let n = self
                .0
                .find(|x: char| !(x.is_ascii_alphanumeric() || x == '_' || x == '.'))
                .unwrap_or(self.0.len());
            let name = self.0[..n].to_owned();
            self.0 = &self.0[n..];
            if self.take('(') {
                Expr::Call(name, self.list(')')?)
            } else {
                Expr::Name(name)
            }
        } else {
            let n = self
                .0
                .find(|x: char| !x.is_ascii_digit() && x != '.')
                .unwrap_or(self.0.len());
            let value = self.0[..n].parse().map_err(|_| "Expected a number")?;
            self.0 = &self.0[n..];
            Expr::Number(value)
        };
        while let Some(op) = self.peek() {
            let power = match op {
                '+' | '-' | '−' => 1,
                '*' | '/' | '×' | '÷' => 2,
                _ => break,
            };
            if power < prec {
                break;
            }
            self.take(op);
            lhs = Expr::Op(op, Box::new(lhs), Box::new(self.expr(power + 1)?));
        }
        Ok(lhs)
    }
}
fn parse(v: &Value) -> Result<Expr, String> {
    if let Some(x) = v.as_f64() {
        return Ok(Expr::Number(x));
    }
    let s = v.as_str().ok_or("Expected a numeric expression")?;
    if s.len() > 512 {
        return Err("Expression too long".into());
    }
    let mut p = Parser(s);
    let e = p.expr(0)?;
    if p.peek().is_some() {
        return Err("Unexpected text after expression".into());
    }
    Ok(e)
}
impl Expr {
    fn eval(
        &self,
        names: &mut impl FnMut(&str) -> Result<f64, String>,
        dims: &BTreeMap<String, f64>,
    ) -> Result<f64, String> {
        let value = match self {
            Self::Number(n) => *n,
            Self::Name(n) => names(n)?,
            Self::List(_) => return Err("Expected a number".into()),
            Self::Op(op, l, r) => {
                let a = l.eval(names, dims)?;
                let b = r.eval(names, dims)?;
                match op {
                    '+' => a + b,
                    '-' | '−' => a - b,
                    '*' | '×' => a * b,
                    _ => a / b,
                }
            }
            Self::Call(f, args) if f == "sum" => {
                let [Self::Name(n)] = args.as_slice() else {
                    return Err("Expected ply dimension".into());
                };
                let dim = n.strip_prefix("ply.").ok_or("Expected ply dimension")?;
                let mut values = dims
                    .iter()
                    .filter(|(k, _)| {
                        k.split_once('.')
                            .is_some_and(|(r, d)| r.starts_with("ply_") && d == dim)
                    })
                    .map(|(_, v)| *v);
                values.next().ok_or("Missing plies")? + values.sum::<f64>()
            }
            Self::Call(f, args) if f == "pick" => {
                let [Self::List(list), val] = args.as_slice() else {
                    return Err("Expected pick(list, value)".into());
                };
                let x = val.eval(names, dims)?;
                list.iter()
                    .map(|e| e.eval(names, dims))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .filter(|v| *v >= x)
                    .min_by(f64::total_cmp)
                    .ok_or("No suitable list value")?
            }
            Self::Call(f, args) => {
                let args = args
                    .iter()
                    .map(|e| e.eval(names, dims))
                    .collect::<Result<Vec<_>, _>>()?;
                match (f.as_str(), args.as_slice()) {
                    ("abs", [x]) => x.abs(),
                    ("min", [x, y]) => x.min(*y),
                    ("max", [x, y]) => x.max(*y),
                    ("clamp", [x, a, b]) if a <= b => x.clamp(*a, *b),
                    ("floor", [x, s]) if *s > 0. => (x / s).floor() * s,
                    ("ceil", [x, s]) if *s > 0. => (x / s).ceil() * s,
                    ("round", [x, s]) if *s > 0. => (x / s).round() * s,
                    _ => return Err(format!("Invalid arguments or unknown function {f}")),
                }
            }
        };
        if value.is_finite() {
            Ok(value)
        } else {
            Err("Non-finite expression".into())
        }
    }
}
pub(super) type Values = BTreeMap<String, f64>;
pub(super) fn field(v: &Value, params: &Values, dims: &Values) -> Result<f64, String> {
    parse(v)?.eval(
        &mut |k| {
            params
                .get(k)
                .or_else(|| dims.get(k))
                .copied()
                .ok_or_else(|| format!("Unknown name {k}"))
        },
        dims,
    )
}
pub(super) fn resolve(
    defs: &[Value],
    overrides: &Value,
    dims: &Values,
) -> Result<(Values, bool), String> {
    let mut inputs = BTreeMap::new();
    for d in defs {
        let k = d["key"].as_str().ok_or("Expected parameter key")?;
        if inputs
            .insert(k, (parse(overrides.get(k).unwrap_or(&d["expr"]))?, d))
            .is_some()
        {
            return Err(format!("Duplicate parameter {k}"));
        }
    }
    fn get(
        k: &str,
        inputs: &BTreeMap<&str, (Expr, &Value)>,
        out: &mut Values,
        active: &mut BTreeSet<String>,
        dims: &Values,
        limited: &mut bool,
    ) -> Result<f64, String> {
        if let Some(n) = out.get(k).or_else(|| dims.get(k)) {
            return Ok(*n);
        }
        let (e, d) = inputs
            .get(k)
            .ok_or_else(|| format!("Unknown parameter {k}"))?;
        if !active.insert(k.into()) {
            return Err(format!("Parameter cycle through {k}"));
        }
        if active.len() > 128 {
            return Err("Too many dependent parameters".into());
        }
        let mut n = e.eval(
            &mut |key| get(key, inputs, out, active, dims, limited),
            dims,
        )?;
        if d["unit"] == "count" {
            n = n.round().max(0.)
        }
        let raw = n;
        if let Some(arr) = d["limits"]["range"].as_array() {
            let [a, b] = arr.as_slice() else {
                return Err("Invalid parameter range".into());
            };
            let (a, b) = (
                a.as_f64().ok_or("Invalid minimum")?,
                b.as_f64().ok_or("Invalid maximum")?,
            );
            if a > b {
                return Err("Inverted parameter range".into());
            }
            n = n.clamp(a, b);
        }
        if let Some(arr) = d["limits"]["list"].as_array() {
            n = arr
                .iter()
                .map(|v| v.as_f64().ok_or("Invalid list value"))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .min_by(|a, b| (a - n).abs().total_cmp(&(b - n).abs()))
                .ok_or("Empty parameter list")?;
        }
        *limited |= raw != n;
        active.remove(k);
        out.insert(k.into(), n);
        Ok(n)
    }
    let mut out = Values::new();
    let mut limited = false;
    for k in inputs.keys() {
        get(
            k,
            &inputs,
            &mut out,
            &mut BTreeSet::new(),
            dims,
            &mut limited,
        )?;
    }
    Ok((out, limited))
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn sizes_and_dependencies() {
        let defs = json!([{"key":"bolt","expr":"pick([60,40,50], sum(ply.thickness) + gap)"},{"key":"gap","expr":"max(4, ceil(main.width ÷ 10, 5))"}]);
        let dims = BTreeMap::from([
            ("ply_1.thickness".into(), 15.),
            ("ply_2.thickness".into(), 25.),
            ("main.width".into(), 99.),
        ]);
        assert_eq!(
            resolve(defs.as_array().unwrap(), &Value::Null, &dims)
                .unwrap()
                .0["bolt"],
            50.
        );
        assert!(
            resolve(defs.as_array().unwrap(), &json!({"gap":"bolt"}), &dims)
                .unwrap_err()
                .contains("cycle")
        );
    }
}

// Validate the parameter dependency graph before a preset enters a model.
// Role dimensions are symbolic at this point.
pub(super) fn validate(defs: &[Value]) -> Result<(), String> {
    fn names(e: &Expr, out: &mut BTreeSet<String>) {
        match e {
            Expr::Name(n) if !n.contains('.') => {
                out.insert(n.clone());
            }
            Expr::Op(_, a, b) => {
                names(a, out);
                names(b, out);
            }
            Expr::Call(_, args) | Expr::List(args) => {
                for a in args {
                    names(a, out)
                }
            }
            _ => (),
        }
    }
    let mut deps = BTreeMap::new();
    for p in defs {
        let key = p["key"].as_str().ok_or("Missing parameter key")?;
        let mut refs = BTreeSet::new();
        names(&parse(&p["expr"])?, &mut refs);
        if deps.insert(key.to_string(), refs).is_some() {
            return Err(format!("Duplicate parameter {key}"));
        }
    }
    let mut resolved = BTreeSet::new();
    while !deps.is_empty() {
        let Some(k) = deps
            .iter()
            .find_map(|(k, v)| v.is_subset(&resolved).then(|| k.clone()))
        else {
            for refs in deps.values() {
                for name in refs {
                    if !deps.contains_key(name) && !resolved.contains(name) {
                        return Err(format!("Unknown parameter {name}"));
                    }
                }
            }
            return Err(format!(
                "Parameter cycle through {}",
                deps.keys().next().unwrap()
            ));
        };
        deps.remove(&k);
        resolved.insert(k);
    }
    Ok(())
}
