// SPDX-License-Identifier: GPL-3.0-or-later
//! Standard PAT interchange. Imported dimensions are stored in millimetres.
use super::*;
use regex::Regex;
use std::sync::LazyLock;

const MAX_FAMILIES: usize = 64;
const MAX_DASHES: usize = 32;
const MIN_SPACING: f64 = 0.05;
static INCHES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i);\s*%\s*UNITS\s*=\s*INCH").unwrap());
static UNSAFE_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new("[^A-Za-z0-9_-]+").unwrap());

fn family<'p>(py: Python<'p>, params: [f64; 5], dashes: Vec<f64>) -> PyResult<Bound<'p, PyList>> {
    let row = PyList::new(py, params)?;
    row.append(dashes)?;
    Ok(row)
}

#[pyfunction]
pub(super) fn parse_pat<'p>(py: Python<'p>, text: &str) -> PyResult<Bound<'p, PyDict>> {
    let factor = if INCHES.is_match(text) { 25.4 } else { 1. };
    let out = PyDict::new(py);
    let mut current = None;
    // Match str.splitlines() on files from classic Mac and Unicode text sources.
    for raw in text.split([
        '\n', '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}', '\u{2029}',
    ]) {
        let line = raw.split(';').next().unwrap().trim();
        if line.is_empty() {
            continue;
        }
        if let Some(header) = line.strip_prefix('*') {
            let (name, label) = header.split_once(',').unwrap_or((header, ""));
            let name: String = name.trim().chars().take(64).collect();
            if name.is_empty() {
                current = None;
                continue;
            }
            let entry = if let Some(entry) = out.get_item(&name)? {
                entry
            } else {
                let entry = PyDict::new(py);
                let label: String = label.trim().chars().take(120).collect();
                entry.set_item("label", if label.is_empty() { &name } else { &label })?;
                entry.set_item("families", PyList::empty(py))?;
                out.set_item(&name, &entry)?;
                entry.into_any()
            };
            current = Some(entry.get_item("families")?.cast_into::<PyList>()?);
        } else if let Some(rows) = &current {
            if rows.len() >= MAX_FAMILIES {
                continue;
            }
            let tokens = line.split(',').map(str::trim).filter(|s| !s.is_empty());
            let nums = tokens
                .map(|s| py.get_type::<PyFloat>().call1((s,))?.extract::<f64>())
                .collect::<PyResult<Vec<_>>>();
            let Ok(nums) = nums else { continue };
            if nums.len() < 5
                || !nums.iter().all(|v| v.is_finite())
                || (nums[4] * factor).abs() < MIN_SPACING
            {
                continue;
            }
            rows.append(family(
                py,
                [
                    nums[0].rem_euclid(360.),
                    nums[1] * factor,
                    nums[2] * factor,
                    nums[3] * factor,
                    nums[4] * factor,
                ],
                nums[5..]
                    .iter()
                    .take(MAX_DASHES)
                    .map(|v| v * factor)
                    .collect(),
            )?)?;
        }
    }
    for key in out.keys() {
        if out.get_item(&key)?.unwrap().get_item("families")?.len()? == 0 {
            out.del_item(key)?;
        }
    }
    Ok(out)
}

#[pyfunction]
pub(super) fn clean_families<'p>(
    py: Python<'p>,
    raw: Option<&Obj<'p>>,
) -> PyResult<Bound<'p, PyList>> {
    let out = PyList::empty(py);
    let Some(rows) = raw.and_then(|r| r.cast::<PyList>().ok()) else {
        return Ok(out);
    };
    for r in rows {
        let read = || -> PyResult<_> {
            let mut values = [0.; 5];
            for (i, v) in values.iter_mut().enumerate() {
                *v = number(&r.get_item(i)?)?;
            }
            let mut ds = Vec::new();
            if r.len()? > 5 {
                for d in r.get_item(5)?.try_iter()? {
                    let v = number(&d?)?;
                    if ds.len() < MAX_DASHES {
                        ds.push(v);
                    }
                }
            }
            Ok((values, ds))
        };
        let Ok((v, d)) = read() else { continue };
        if v.iter().chain(&d).all(|x| x.is_finite()) && v[4].abs() >= MIN_SPACING {
            out.append(family(py, v, d)?)?;
            if out.len() == MAX_FAMILIES {
                break;
            }
        }
    }
    Ok(out)
}

#[pyfunction(pass_module)]
pub(super) fn line_families<'p>(
    m: &Module<'p>,
    key: Option<&str>,
) -> PyResult<Option<Bound<'p, PyList>>> {
    let Some(k) = key else { return Ok(None) };
    let Some(rows) = value(m, "_FAMILIES", k)? else {
        return Ok(None);
    };
    if value(m, "_SCATTERED", k)?.is_some() || value(m, "_WAVES", k)?.is_some() {
        return Ok(None);
    }
    let turn = value(m, "_TURN", k)?
        .map(|v| v.extract::<f64>())
        .transpose()?
        .unwrap_or(0.);
    let (sin, cos) = turn.to_radians().sin_cos();
    let out = PyList::empty(m.py());
    for r in rows.try_iter()? {
        let r = r?;
        let get = |i| r.get_item(i)?.extract::<f64>();
        let (angle, mut x, mut y) = (get(0)?, get(1)?, get(2)?);
        if r.len()? > 6 {
            let offset = get(6)?;
            x -= angle.to_radians().sin() * offset;
            y += angle.to_radians().cos() * offset;
        }
        if turn != 0. {
            (x, y) = (x * cos - y * sin, x * sin + y * cos);
        }
        out.append(family(
            m.py(),
            [(angle + turn).rem_euclid(360.), x, y, get(3)?, get(4)?],
            r.get_item(5)?.extract()?,
        )?)?;
    }
    Ok(Some(out))
}

#[pyfunction]
pub(super) fn to_pat(patterns: &Obj<'_>) -> PyResult<String> {
    let mut out = String::from(";; Hatch patterns exported from Built\n;%UNITS=MM\n\n");
    let format_number = |v: f64| {
        let s = format!("{v:.6}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s == "-0" {
            "0".into()
        } else {
            s.to_owned()
        }
    };
    for pattern in patterns.try_iter()? {
        let pattern = pattern?;
        let fields = pattern.try_iter()?.take(4).collect::<PyResult<Vec<_>>>()?;
        let [name, label, rows] = fields.as_slice() else {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "expected three pattern fields (name, label, families)",
            ));
        };
        if !rows.is_truthy()? {
            continue;
        }
        let name: String = name.extract()?;
        let label: String = label.extract()?;
        let safe = UNSAFE_NAME.replace_all(&name, "-");
        let safe = safe.trim_matches('-');
        out.push_str(&format!(
            "*{}, {}\n",
            if safe.is_empty() { "PATTERN" } else { safe },
            label.replace(',', " ")
        ));
        for row in rows.try_iter()? {
            let row = row?;
            let mut values = (0..5)
                .map(|i| row.get_item(i)?.extract::<f64>())
                .collect::<PyResult<Vec<_>>>()?;
            values.extend(row.get_item(5)?.extract::<Vec<f64>>()?);
            out.push_str(
                &values
                    .into_iter()
                    .map(format_number)
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            out.push('\n');
        }
    }
    Ok(out)
}
