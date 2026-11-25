// SPDX-License-Identifier: GPL-3.0-or-later
//! Placement traversal for the project tree. A shared placement contributes
//! its first occurrence in depth-first display order.
use pyo3::{
    prelude::*,
    types::{PyList, PyTuple},
};
use std::collections::HashSet;
type Obj<'py> = Bound<'py, PyAny>;
#[pyfunction]
pub(crate) fn link_placements<'py>(scene: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let py = scene.py();
    let out = PyList::empty(py);
    let mut stack = Vec::new();
    let mut seen = HashSet::new();
    let children = |obj: &Obj<'py>, field: &str| -> PyResult<Vec<Obj<'py>>> {
        let Some(v) = obj.getattr_opt(field)?.filter(|v| !v.is_none()) else {
            return Ok(vec![]);
        };
        v.try_iter()?.collect()
    };
    for root in children(scene, "groups")?.into_iter().rev() {
        stack.push((root, PyTuple::empty(py), true));
    }
    while let Some((group, chain, root)) = stack.pop() {
        if !seen.insert(group.as_ptr()) {
            continue;
        }
        let link = crate::model::links::link_of(&group)?;
        let linked = link.is_some();
        let inner = if let Some(link) = link {
            let path = link.get_item("path")?;
            let entry = (
                crate::model::links::stem(&path)?,
                crate::model::links::norm(&path)?,
            )
                .into_pyobject(py)?;
            PyTuple::new(
                py,
                chain
                    .iter()
                    .chain(std::iter::once(entry.into_any()))
                    .collect::<Vec<_>>(),
            )?
        } else {
            chain.clone()
        };
        out.append((&group, chain, &inner, root))?;
        for child in children(&group, "children")?.into_iter().rev() {
            stack.push((child, inner.clone(), linked));
        }
    }
    Ok(out)
}
#[pyfunction]
fn tree_order<'py>(children: &Obj<'py>) -> PyResult<Vec<(Obj<'py>, usize)>> {
    let mut stack = children
        .get_item(children.py().None())?
        .try_iter()?
        .collect::<PyResult<Vec<_>>>()?
        .into_iter()
        .rev()
        .map(|k| (k, 0))
        .collect::<Vec<_>>();
    let mut out = vec![];
    while let Some((key, depth)) = stack.pop() {
        if let Some(kids) = children.cast::<pyo3::types::PyDict>()?.get_item(&key)? {
            let kids = kids.try_iter()?.collect::<PyResult<Vec<_>>>()?;
            stack.extend(kids.into_iter().rev().map(|k| (k, depth + 1)));
        }
        out.push((key, depth));
    }
    Ok(out)
}
#[pyfunction]
fn tree_subtree<'py>(children: &Obj<'py>, key: Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let mut stack = vec![key];
    let mut out = vec![];
    while let Some(key) = stack.pop() {
        if let Some(kids) = children.cast::<pyo3::types::PyDict>()?.get_item(&key)? {
            for k in kids.try_iter()? {
                stack.push(k?);
            }
        }
        out.push(key);
    }
    Ok(out)
}
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(link_placements, m)?)?;
    m.add_function(wrap_pyfunction!(tree_order, m)?)?;
    m.add_function(wrap_pyfunction!(tree_subtree, m)?)?;
    Ok(())
}
