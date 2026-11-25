// SPDX-License-Identifier: GPL-3.0-or-later
//! Partition a mixed face/edge selection into exactly two bridge profiles.
use super::{bridge_error, xyz, PointList, EPS};
use pyo3::{
    prelude::*,
    types::{PyList, PySet},
};
use std::collections::{HashMap, HashSet};
type Obj<'py> = PointList<'py>;
const COUNT: &str = "Select two faces, a face and a line, or two lines to bridge";
fn ids(items: &Obj<'_>) -> PyResult<HashSet<i128>> {
    items
        .try_iter()?
        .map(|v| v?.getattr("key")?.extract())
        .collect()
}
fn chains<'py>(edges: &[Obj<'py>], limit: usize, py: Python<'py>) -> PyResult<Vec<Vec<Obj<'py>>>> {
    let left = PySet::new(py, edges)?;
    let mut at: HashMap<i128, Vec<usize>> = HashMap::new();
    for (i, e) in edges.iter().enumerate() {
        for name in ["v0", "v1"] {
            at.entry(e.getattr(name)?.getattr("key")?.extract()?)
                .or_default()
                .push(i);
        }
    }
    let mut pieces = vec![];
    while !left.is_empty() {
        if pieces.len() >= limit {
            return Err(bridge_error(py, COUNT)?);
        }
        let seed = left.pop().unwrap();
        let mut piece = vec![seed.clone()];
        let mut stack = vec![seed];
        while let Some(e) = stack.pop() {
            for name in ["v0", "v1"] {
                let k = e.getattr(name)?.getattr("key")?.extract::<i128>()?;
                for &j in &at[&k] {
                    if left.discard(&edges[j])? {
                        piece.push(edges[j].clone());
                        stack.push(edges[j].clone());
                    }
                }
            }
        }
        pieces.push(piece);
    }
    if pieces.len() > 1 {
        let key = crate::model::module(py, "core.topology")?.getattr("_key")?;
        let mut ordered = vec![];
        for piece in pieces {
            let mut first: Option<Obj> = None;
            for e in &piece {
                for name in ["v0", "v1"] {
                    let k = key.call1((e.getattr(name)?.getattr("position")?,))?;
                    if match &first {
                        None => true,
                        Some(v) => k.lt(v)?,
                    } {
                        first = Some(k)
                    }
                }
            }
            ordered.push((first.unwrap(), piece));
        }
        if ordered[1].0.lt(&ordered[0].0)? {
            ordered.swap(0, 1)
        }
        pieces = ordered.into_iter().map(|(_, v)| v).collect();
    }
    Ok(pieces)
}
#[pyfunction]
pub(super) fn profiles_from_selection<'py>(
    mesh: &Obj<'py>,
    selection: &Obj<'py>,
) -> PyResult<Bound<'py, PyList>> {
    let py = mesh.py();
    let model = crate::model::module(py, "ingetrazo_core.mesh")?;
    let face = model.getattr("Face")?;
    let edge = model.getattr("Edge")?;
    let faces = ids(&mesh.getattr("faces")?)?;
    let edges = ids(&mesh.getattr("edges")?)?;
    let mut selected = vec![];
    let mut selected_ids = HashSet::new();
    for v in selection.try_iter()? {
        let v = v?;
        if v.is_instance(&face)? {
            let k = v.getattr("key")?.extract::<i128>()?;
            if faces.contains(&k) {
                selected_ids.insert(k);
                selected.push(v);
                if selected.len() > 2 {
                    return Err(bridge_error(py, COUNT)?);
                }
            }
        }
    }
    let mut loose = vec![];
    for v in selection.try_iter()? {
        let v = v?;
        if !v.is_instance(&edge)? || !edges.contains(&v.getattr("key")?.extract::<i128>()?) {
            continue;
        }
        let mut attached = false;
        for f in v.getattr("faces")?.try_iter()? {
            if selected_ids.contains(&f?.getattr("key")?.extract::<i128>()?) {
                attached = true;
                break;
            }
        }
        if !attached {
            loose.push(v)
        }
    }
    let profile = crate::model::module(py, "core.bridge")?.getattr("Profile")?;
    let out = PyList::empty(py);
    for f in selected {
        let loops = PyList::empty(py);
        loops.append(f.getattr("vertices")?)?;
        loops.call_method1("extend", (f.getattr("holes")?,))?;
        out.append(profile.call1((loops, true, f))?)?;
    }
    let order = crate::model::module(py, "core.sweep")?.getattr("order_path_edges")?;
    for chain in chains(&loose, 2 - out.len(), py)? {
        let result = order.call1((chain,))?;
        if result.is_none() {
            return Err(bridge_error(
                py,
                "Each line to bridge must be one chain of edges, without branches",
            )?);
        }
        let mut pts = result.get_item(0)?;
        let closed = result.get_item(1)?.is_truthy()?;
        let n = pts.len()?;
        if closed && n > 1 && xyz(&pts.get_item(0)?)?.distance(xyz(&pts.get_item(n - 1)?)?) < EPS {
            pts = pts.get_item(pyo3::types::PySlice::new(py, 0, (n - 1) as isize, 1))?;
        }
        out.append(profile.call1((vec![pts], closed))?)?;
    }
    if out.len() != 2 {
        return Err(bridge_error(py, COUNT)?);
    }
    Ok(out)
}
