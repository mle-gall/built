// SPDX-License-Identifier: GPL-3.0-or-later
//! Construct just the requested plugin snapshot column from the captured slots.
use super::*;
use crate::store::keys::NONE;
use pyo3::types::PySet;

type Obj<'py> = Bound<'py, PyAny>;

pub(super) fn field<'py>(
    store: &Store,
    name: &str,
    mesh: &Obj<'py>,
    own: &Bound<'py, PyDict>,
    table: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let views = py.import("ingetrazo_core.store_mesh")?;
    let view = |kind, slot| {
        let class = match kind {
            Kind::Vertex => "VertexView",
            Kind::Edge => "EdgeView",
            Kind::Face => "FaceView",
        };
        views.call_method1(class, (mesh, store.key(kind, slot)))
    };
    let kind = match name {
        "vertices" | "registry" | "vedges" => Kind::Vertex,
        "edges" | "efaces" | "esoft" | "ehidden" | "eflat" | "ecurve" | "elayer" => Kind::Edge,
        "faces" | "floop" | "fholes" | "fattrs" | "finterior" => Kind::Face,
        _ => return Err(PyKeyError::new_err(name.to_owned())),
    };
    if matches!(name, "vertices" | "edges" | "faces") {
        let items = PyList::empty(py);
        for slot in store.live_slots(kind) {
            items.append(view(kind, slot)?)?;
        }
        return Ok(items.into_any());
    }
    let result = PyDict::new(py);
    let state = &store.state;
    if name == "registry" {
        let mut cells: Vec<_> = state
            .registry
            .iter()
            .map(|(c, &v)| ((c.0, c.1, c.2), v))
            .collect();
        cells.sort_unstable();
        for (cell, slot) in cells {
            result.set_item(cell, view(Kind::Vertex, slot)?)?;
        }
        return Ok(result.into_any());
    }
    for slot in store.live_slots(kind) {
        let key = store.key(kind, slot);
        let i = slot as usize;
        match name {
            "vedges" => {
                let edges = PySet::empty(py)?;
                for e in store.edges_at(slot) {
                    edges.add(view(Kind::Edge, e)?)?;
                }
                result.set_item(key, edges)?;
            }
            "efaces" => {
                let faces = PyList::empty(py);
                for &f in store.edge_faces(slot) {
                    faces.append(view(Kind::Face, f)?)?;
                }
                result.set_item(key, faces)?;
            }
            "esoft" | "ehidden" | "eflat" => {
                let mask = match name {
                    "esoft" => 1,
                    "ehidden" => 2,
                    _ => 4,
                };
                result.set_item(key, *state.eflags.get(i) & mask != 0)?;
            }
            "ecurve" => {
                let c = *state.ecurve.get(i);
                result.set_item(key, (c >= 0).then_some(c))?;
            }
            "elayer" => {
                let l = *state.elayer.get(i);
                result.set_item(key, (l != NONE).then(|| &state.layers[l as usize]))?;
            }
            "floop" | "fholes" => {
                let rings = PyList::empty(py);
                for (index, ring) in store.face_rings(slot).into_iter().enumerate() {
                    if (index == 0) != (name == "floop") {
                        continue;
                    }
                    let vertices = PyList::empty(py);
                    for v in ring {
                        vertices.append(view(Kind::Vertex, v)?)?;
                    }
                    rings.append(vertices)?;
                }
                if name == "floop" {
                    result.set_item(key, rings.get_item(0)?)?;
                } else {
                    result.set_item(key, rings)?;
                }
            }
            "fattrs" => {
                let row = *state.fattr.get(i);
                let attrs = match own.get_item(key)? {
                    Some(value) if !value.is_none() => value,
                    _ if row > 0 => table.get_item(row - 1)?,
                    _ => PyDict::new(py).into_any(),
                };
                result.set_item(key, py.get_type::<PyDict>().call1((attrs,))?)?;
            }
            "finterior" => result.set_item(key, *state.fflags.get(i) & 1 != 0)?,
            _ => unreachable!(),
        }
    }
    Ok(result.into_any())
}
