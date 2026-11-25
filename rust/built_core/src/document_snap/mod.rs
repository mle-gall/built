// SPDX-License-Identifier: GPL-3.0-or-later
//! Assemble document snap candidates from placement arrays and record handles.
use crate::math::{xyz, Vec3};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
type Obj<'py> = Bound<'py, PyAny>;
fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
fn opt<'py>(o: &Obj<'py>, n: &str) -> PyResult<Obj<'py>> {
    Ok(o.getattr_opt(n)?.unwrap_or_else(|| none(o.py())))
}
fn host(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("views.document_interaction")
}
fn id(o: &Obj) -> usize {
    o.as_ptr() as usize
}
fn optional_call<'py>(o: &Obj<'py>, n: &str) -> PyResult<Obj<'py>> {
    match o.getattr_opt(n)? {
        Some(f) if f.is_callable() => f.call0(),
        _ => Ok(none(o.py())),
    }
}
fn kw<'py>(py: Python<'py>, name: &str, value: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item(name, value)?;
    Ok(d)
}
fn collection<'py>(o: &Obj<'py>, name: &str) -> PyResult<Obj<'py>> {
    Ok(o.getattr_opt(name)?
        .filter(|v| !v.is_none())
        .unwrap_or_else(|| PyTuple::empty(o.py()).into_any()))
}
fn cache<'py>(v: &Obj<'py>, name: &str, key: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    if let Some(c) = v.getattr_opt(name)?.filter(|c| !c.is_none()) {
        if c.get_item(0)?.eq(key)? {
            return Ok(Some(c.get_item(1)?));
        }
    }
    Ok(None)
}
fn dict_slot<'py>(v: &Obj<'py>, name: &str) -> PyResult<Obj<'py>> {
    if let Some(d) = v.getattr_opt(name)?.filter(|x| !x.is_none()) {
        return Ok(d);
    }
    let d = PyDict::new(v.py());
    v.setattr(name, &d)?;
    Ok(d.into_any())
}
fn context<'py>(v: &Obj<'py>, g: &Obj<'py>) -> PyResult<Obj<'py>> {
    host(v.py())?.call_method1("_snap_context_of", (v, g))
}
fn snap_edge<'py>(
    py: Python<'py>,
    a: &Obj<'py>,
    b: &Obj<'py>,
    flag: Option<&str>,
    context: Option<&Obj<'py>>,
    group: Option<&Obj<'py>>,
) -> PyResult<Obj<'py>> {
    let opts = PyDict::new(py);
    if let Some(f) = flag {
        opts.set_item(f, true)?
    }
    if let Some(c) = context {
        opts.set_item("context", c)?
    }
    if let Some(g) = group {
        opts.set_item("group", g)?
    }
    host(py)?.getattr("_SnapEdge")?.call((a, b), Some(&opts))
}
fn point<'py>(v: &Obj<'py>, flag: Option<&str>, context: Option<&Obj<'py>>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let p = xyz(v)?;
    let a = Bound::new(py, Vec3::from(p))?.into_any();
    let b = Bound::new(py, Vec3::from(p))?.into_any();
    snap_edge(py, &a, &b, flag, context, None)
}
fn index<'py>(v: &Obj<'py>, near: Obj<'py>) -> PyResult<Obj<'py>> {
    v.call_method("_pick_index", (), Some(&kw(v.py(), "near", &near)?))
}
fn skip_index<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    index(v, "skip".into_pyobject(v.py())?.into_any())
}
mod candidates;
mod bounds;
mod centers;
mod nearby;
mod projection;
mod records;
mod scene;
pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    candidates::register(m)?;
    bounds::register(m)?;
    nearby::register(m)?;
    projection::register(m)?;
    centers::register(m)?;
    scene::register(m)?;
    records::register(m)
}
