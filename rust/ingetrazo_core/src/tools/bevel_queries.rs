// SPDX-License-Identifier: GPL-3.0-or-later
//! Resolve Fillet picks against live store keys. Preserve selection order
//! because the bevel planner uses it to orient a strip.
use super::view_queries::classify;
use crate::{
    math::xyz,
    store::{keys::Kind, py::PyMeshStore},
};
use pyo3::prelude::*;
use std::collections::HashSet;

#[pyfunction]
pub fn bevel_nearest_end<'py>(
    edge: &Bound<'py, PyAny>,
    cursor: Option<&Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    let a = edge.getattr("v0")?;
    if let Some(cursor) = cursor {
        let p = xyz(cursor)?;
        let b = edge.getattr("v1")?;
        if xyz(&b.getattr("position")?)?.distance_squared(p)
            < xyz(&a.getattr("position")?)?.distance_squared(p)
        {
            return Ok(b);
        }
    }
    Ok(a)
}

#[pyfunction]
pub fn bevel_selection<'py>(
    items: &Bound<'py, PyAny>,
    mesh: &Bound<'py, PyAny>,
    corners: bool,
) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let store = mesh
        .getattr_opt("_store")?
        .filter(|s| s.is_instance_of::<PyMeshStore>());
    let py = items.py();
    for item in items.try_iter()? {
        let item = item?;
        let kind = match classify(&item)? {
            "edge" => Kind::Edge,
            "face" if corners => Kind::Face,
            _ => continue,
        };
        if let Some(s) = &store {
            let st = PyMeshStore::read(s)?;
            let Some(slot) = st.inner.slot(kind, item.getattr("key")?.extract()?) else {
                continue;
            };
            if !corners {
                out.push(item);
                continue;
            }
            let ids = match kind {
                Kind::Edge => st.inner.state.ends.get(slot as usize).to_vec(),
                _ => st.inner.outer_ring(slot),
            };
            let ctor = py.import("core.store_mesh")?.getattr("VertexView")?;
            for v in ids {
                let key = st.inner.key(Kind::Vertex, v);
                if seen.insert(key) {
                    out.push(ctor.call1((mesh, key))?);
                }
            }
        } else {
            let vertices = if kind == Kind::Edge {
                let a = item.getattr("v0")?;
                if mesh.is_none() || !a.getattr("edges")?.contains(&item)? {
                    continue;
                }
                if !corners {
                    out.push(item);
                    continue;
                }
                vec![a, item.getattr("v1")?]
            } else {
                item.getattr("loop")?
                    .try_iter()?
                    .collect::<PyResult<Vec<_>>>()?
            };
            for v in vertices {
                if seen.insert(v.getattr("key")?.extract()?) {
                    out.push(v);
                }
            }
        }
    }
    Ok(out)
}

#[pyfunction]
pub fn bevel_targets<'py>(
    view: &Bound<'py, PyAny>,
    corners: bool,
    cursor: Option<&Bound<'py, PyAny>>,
) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let Some(edge) = view.getattr_opt("_hover_edge")? else {
        return Ok(vec![]);
    };
    if classify(&edge)? != "edge" {
        return Ok(vec![]);
    }
    if let Some(scene) = view.getattr_opt("scene")? {
        if let Some(selection) = scene.getattr_opt("selection")?.filter(|s| !s.is_none()) {
            if selection.contains(&edge)? {
                if corners {
                    return bevel_selection(&selection, &scene.getattr("mesh")?, true);
                }
                let mut result = Vec::new();
                for e in selection.try_iter()? {
                    let e = e?;
                    if classify(&e)? == "edge" {
                        result.push(e);
                    }
                }
                return Ok(result);
            }
        }
    }
    Ok(vec![if corners {
        bevel_nearest_end(&edge, cursor)?
    } else {
        edge
    }])
}
