// SPDX-License-Identifier: GPL-3.0-or-later
//! Flags shared by detached elements, native views and plugin objects.
use super::Obj;
use pyo3::{prelude::*, types::PyTuple};
const FLAGS: [&str; 5] = ["soft", "curve", "layer", "hidden", "flat"];
fn defaults(py: Python<'_>) -> PyResult<Bound<'_, PyTuple>> {
    (false, py.None(), py.None(), false, false).into_pyobject(py)
}
#[pyfunction]
fn edge_flags<'py>(edge: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    let py = edge.py();
    let defaults = defaults(py)?;
    let mut vals = Vec::with_capacity(FLAGS.len());
    for (i, name) in FLAGS.iter().enumerate() {
        vals.push(edge.getattr_opt(*name)?.unwrap_or(defaults.get_item(i)?));
    }
    PyTuple::new(py, vals)
}
#[pyfunction]
fn edge_is_plain(edge: &Obj<'_>) -> PyResult<bool> {
    edge_flags(edge)?.eq(defaults(edge.py())?)
}
#[pyfunction]
fn stamp_edge_flags(edge: &Obj<'_>, flags: &Obj<'_>) -> PyResult<()> {
    for (name, value) in FLAGS.iter().zip(flags.try_iter()?) {
        edge.setattr(*name, value?)?;
    }
    Ok(())
}
#[pyfunction]
fn smooths(edge: &Obj<'_>) -> PyResult<bool> {
    let truth = |name: &str| -> PyResult<bool> {
        Ok(edge
            .getattr_opt(name)?
            .map(|v| v.is_truthy())
            .transpose()?
            .unwrap_or(false))
    };
    Ok(truth("soft")? && !truth("flat")?)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("EDGE_FLAG_NAMES", PyTuple::new(m.py(), FLAGS)?)?;
    m.add("EDGE_FLAG_DEFAULTS", defaults(m.py())?)?;
    m.add_function(wrap_pyfunction!(edge_flags, m)?)?;
    m.add_function(wrap_pyfunction!(edge_is_plain, m)?)?;
    m.add_function(wrap_pyfunction!(stamp_edge_flags, m)?)?;
    m.add_function(wrap_pyfunction!(smooths, m)?)?;
    Ok(())
}
