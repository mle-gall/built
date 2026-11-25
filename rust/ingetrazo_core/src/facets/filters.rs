// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::types::{PyBool, PyFloat, PyInt, PyString};
enum Expr<'p> {
    True,
    Group(bool, Vec<Self>),
    Not(Box<Self>),
    Text(Obj<'p>),
    Leaf(String, String, Obj<'p>),
}
fn invalid(s: &str) -> PyErr {
    PyValueError::new_err(s.to_owned())
}
fn scalar(v: &Obj) -> bool {
    !v.is_instance_of::<PyDict>() && !v.is_instance_of::<PyList>()
}
fn number(v: &Obj) -> bool {
    v.is_exact_instance_of::<PyInt>() || v.is_exact_instance_of::<PyFloat>()
}
fn parse<'p>(value: Option<&Obj<'p>>) -> PyResult<Expr<'p>> {
    let Some(v) = value.filter(|v| !v.is_none()) else {
        return Ok(Expr::True);
    };
    let d = v
        .cast::<PyDict>()
        .map_err(|_| invalid("filter must be an object or null"))?;
    if d.len() == 1 {
        for k in ["all", "any"] {
            if let Some(arr) = d.get_item(k)? {
                let rows = arr
                    .cast::<PyList>()
                    .map_err(|_| invalid("all/any require a list"))?;
                return Ok(Expr::Group(
                    k == "all",
                    rows.iter()
                        .map(|v| parse(Some(&v)))
                        .collect::<PyResult<_>>()?,
                ));
            }
        }
        if let Some(child) = d.get_item("not")? {
            return Ok(Expr::Not(Box::new(parse(Some(&child))?)));
        }
        if let Some(s) = d.get_item("text")? {
            if s.is_instance_of::<PyString>() {
                return Ok(Expr::Text(s.call_method0("casefold")?));
            }
        }
    }
    let facet = d
        .get_item("facet")?
        .and_then(|v| v.extract::<String>().ok())
        .filter(|_| d.len() == 2)
        .ok_or_else(|| invalid("leaf requires facet and one operator"))?;
    let (k, mut operand) = d
        .iter()
        .find(|(k, _)| !k.eq("facet").unwrap_or(false))
        .unwrap();
    let op = k
        .extract::<String>()
        .map_err(|_| invalid("unknown filter operator"))?;
    let valid = match op.as_str() {
        "eq" => scalar(&operand),
        "exists" => operand.is_instance_of::<PyBool>(),
        "gt" | "lt" => number(&operand),
        "glob" => operand.is_instance_of::<PyString>(),
        "in" => operand
            .cast::<PyList>()
            .is_ok_and(|l| l.iter().all(|v| scalar(&v))),
        _ => return Err(invalid("unknown filter operator")),
    };
    if !valid {
        return Err(invalid("invalid filter operand"));
    }
    if op == "glob" {
        operand = operand.call_method0("casefold")?;
    }
    Ok(Expr::Leaf(facet, op, operand))
}
impl Expr<'_> {
    fn prepare(&self, idx: &Obj) -> PyResult<()> {
        match self {
            Self::Leaf(f, _, _) => super::prepare(idx, f)?,
            Self::Group(_, cs) => {
                for c in cs {
                    c.prepare(idx)?;
                }
            }
            Self::Not(c) => c.prepare(idx)?,
            _ => (),
        }
        Ok(())
    }
    fn matches(&self, row: &Obj, glob: &Obj) -> PyResult<bool> {
        match self {
            Self::True => Ok(true),
            Self::Not(c) => Ok(!c.matches(row, glob)?),
            Self::Group(all, cs) => {
                for c in cs {
                    if c.matches(row, glob)? != *all {
                        return Ok(!all);
                    }
                }
                Ok(*all)
            }
            Self::Text(pat) => {
                for k in ["name", "path"] {
                    if glob
                        .call1((row.get_item(k)?.str()?.call_method0("casefold")?, pat))?
                        .is_truthy()?
                    {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Leaf(f, op, operand) => {
                let v = get(row, f)?;
                if op == "exists" {
                    let exists = !v.is_none() && !v.eq(PyList::empty(v.py()))? && !v.eq("")?;
                    return Ok(exists == operand.is_truthy()?);
                }
                let check = |v: &Obj| -> PyResult<bool> {
                    match op.as_str() {
                        "eq" => v.eq(operand),
                        "in" => operand.contains(v),
                        "gt" => Ok(number(v) && v.gt(operand)?),
                        "lt" => Ok(number(v) && v.lt(operand)?),
                        "glob" => Ok(v.is_instance_of::<PyString>()
                            && glob
                                .call1((v.call_method0("casefold")?, operand))?
                                .is_truthy()?),
                        _ => unreachable!(),
                    }
                };
                if let Ok(l) = v.cast::<PyList>() {
                    for i in l.iter() {
                        if check(&i)? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                } else {
                    check(&v)
                }
            }
        }
    }
}
#[pyfunction]
fn validate_filter(value: &Obj) -> PyResult<()> {
    parse(Some(value)).map(drop)
}
#[pyfunction]
#[pyo3(signature=(index,filter=None))]
pub(super) fn evaluate<'p>(
    index: &Obj<'p>,
    filter: Option<&Obj<'p>>,
) -> PyResult<Bound<'p, PyList>> {
    let e = parse(filter)?;
    e.prepare(index)?;
    let glob = index.py().import("fnmatch")?.getattr("fnmatchcase")?;
    let out = PyList::empty(index.py());
    for (k, row) in index.cast::<PyDict>()?.iter() {
        if e.matches(&row, &glob)? {
            out.append(k)?;
        }
    }
    Ok(out)
}
#[pyfunction]
#[pyo3(signature=(index,facet,uids=None))]
fn values<'p>(index: &Obj<'p>, facet: &str, uids: Option<&Obj<'p>>) -> PyResult<Bound<'p, PyDict>> {
    super::prepare(index, facet)?;
    let counts = PyDict::new(index.py());
    for uid in uids.unwrap_or(index).try_iter()? {
        let v = get(&index.get_item(uid?)?, facet)?;
        let count = |v: &Obj| -> PyResult<()> {
            if !v.is_none() {
                let n = counts
                    .get_item(v)?
                    .map(|v| v.extract::<usize>())
                    .transpose()?
                    .unwrap_or(0);
                counts.set_item(v, n + 1)?;
            }
            Ok(())
        };
        if let Ok(l) = v.cast::<PyList>() {
            let seen = PySet::empty(index.py())?;
            for v in l.iter() {
                if !seen.contains(&v)? {
                    seen.add(&v)?;
                    count(&v)?;
                }
            }
        } else {
            count(&v)?;
        }
    }
    Ok(counts)
}
#[pyfunction]
fn palette<'p>(value: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = value.py();
    let dump = py.import("json")?.getattr("dumps")?;
    let kw = PyDict::new(py);
    kw.set_item("sort_keys", true)?;
    let encode = |v: &Obj| dump.call((v,), Some(&kw))?.extract::<String>();
    let mut v = value.clone();
    if let Ok(l) = v.cast::<PyList>() {
        let mut members = std::collections::BTreeMap::new();
        for m in l.iter() {
            members.insert(encode(&m)?, m);
        }
        v = if members.len() == 1 {
            members.into_values().next().unwrap()
        } else {
            PyList::new(py, members.into_values())?.into_any()
        };
    }
    let hash: Vec<u8> = py
        .import("hashlib")?
        .call_method1(
            "sha256",
            (pyo3::types::PyBytes::new(py, encode(&v)?.as_bytes()),),
        )?
        .call_method0("digest")?
        .extract()?;
    let hue = f64::from(u32::from_be_bytes(hash[..4].try_into().unwrap())) / 4294967296.;
    py.import("colorsys")?
        .call_method1("hsv_to_rgb", (hue, 0.55, 0.85))
}
pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    for f in [
        wrap_pyfunction!(validate_filter, m)?,
        wrap_pyfunction!(evaluate, m)?,
        wrap_pyfunction!(values, m)?,
        wrap_pyfunction!(palette, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
