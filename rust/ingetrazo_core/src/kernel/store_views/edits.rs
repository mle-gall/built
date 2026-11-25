// SPDX-License-Identifier: GPL-3.0-or-later
//! Store edits that also invalidate mesh caches and construct plugin element views.
use super::Obj;
use crate::store::py::StoreFallback;
use pyo3::{
    prelude::*,
    types::{PyInt, PyList},
};
fn api(py: Python<'_>) -> PyResult<Obj<'_>> {
    crate::model::module(py, "core.store_mesh")
}
fn refusal(py: Python<'_>, op: &str) -> PyResult<PyErr> {
    Ok(PyErr::from_value(api(py)?.getattr("StoreRefusal")?.call1(
        (format!(
            "{op} requires live, representable incidence in this mesh"
        ),),
    )?))
}
pub(super) fn checked<T>(v: PyResult<T>, py: Python<'_>, op: &str) -> PyResult<T> {
    match v {
        Err(e) if e.is_instance_of::<StoreFallback>(py) => Err(refusal(py, op)?),
        v => v,
    }
}
pub(super) fn view<'py>(mesh: &Obj<'py>, kind: usize, key: &Obj<'py>) -> PyResult<Obj<'py>> {
    api(mesh.py())?.call_method1(["VertexView", "EdgeView", "FaceView"][kind], (mesh, key))
}
fn clear_bulk(mesh: &Obj<'_>) -> PyResult<()> {
    mesh.getattr("__dict__")?
        .call_method1("pop", ("_bulk", mesh.py().None()))?;
    Ok(())
}
#[pyfunction]
#[pyo3(signature=(mesh,n=1))]
pub(super) fn dirty(mesh: &Obj<'_>, n: i64) -> PyResult<()> {
    clear_bulk(mesh)?;
    if n != 0 {
        let d = mesh.getattr("__dict__")?;
        d.set_item("_chunk_dirty_value", true)?;
        for key in ["_revision", "_mut_serial"] {
            d.set_item(key, d.call_method1("get", (key, 0))?.add(n)?)?;
        }
    }
    Ok(())
}
#[pyfunction]
#[pyo3(signature=(mesh,element,kind,live=false))]
pub(super) fn owned<'py>(
    mesh: &Obj<'py>,
    element: &Obj<'py>,
    kind: u8,
    live: bool,
) -> PyResult<Option<Obj<'py>>> {
    let Some(k) = element.getattr_opt("key")? else {
        return Ok(None);
    };
    if !k.is_exact_instance_of::<PyInt>() {
        return Ok(None);
    }
    let Ok(value) = k.extract::<u128>() else {
        return Ok(None);
    };
    if ((value >> 38) & 3) as u8 != kind {
        return Ok(None);
    }
    let st = mesh.getattr("_store")?;
    Ok(
        if st
            .call_method1(if live { "is_live" } else { "knows" }, (&k,))?
            .is_truthy()?
        {
            Some(k)
        } else {
            None
        },
    )
}
pub(super) fn key<'py>(
    mesh: &Obj<'py>,
    element: &Obj<'py>,
    kind: u8,
    live: bool,
    op: &str,
) -> PyResult<Obj<'py>> {
    match owned(mesh, element, kind, live)? {
        Some(k) => Ok(k),
        None => Err(refusal(mesh.py(), op)?),
    }
}
#[pyfunction]
fn add_face<'py>(
    mesh: &Obj<'py>,
    loop_positions: &Obj<'py>,
    hole_loops: Option<&Obj<'py>>,
) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    dirty(mesh, 1)?;
    let list = py.get_type::<PyList>();
    let outer = list.call1((loop_positions,))?;
    let holes = PyList::empty(py);
    if let Some(h) = hole_loops {
        for ring in h.try_iter()? {
            holes.append(list.call1((ring?,))?)?;
        }
    }
    let rings = PyList::empty(py);
    let input = mesh.getattr("_input_point")?;
    let convert = |r: &Obj<'py>| -> PyResult<Bound<'py, PyList>> {
        let seq = PyList::empty(py);
        for p in r.try_iter()? {
            seq.append(input.call1((p?,))?)?;
        }
        Ok(seq)
    };
    let mut keep = vec![true; holes.len()];
    if keep.len() > 1 {
        match api(py)?.call_method1("_hole_filter", (&holes,)) {
            Ok(mask) => {
                keep = mask
                    .try_iter()?
                    .map(|v| v?.is_truthy())
                    .collect::<PyResult<_>>()?
            }
            Err(e) => {
                for p in outer.try_iter()? {
                    mesh.call_method1("vertex", (p?,))?;
                }
                return Err(e);
            }
        }
    }
    let outer = convert(&outer)?;
    let mut corners = outer.len();
    rings.append(outer)?;
    for (r, keep) in holes.iter().zip(keep) {
        if keep {
            let r = convert(&r)?;
            corners += r.len();
            rings.append(r)?;
        }
    }
    let made = mesh.getattr("_store")?.call_method1("add_face", (rings,))?;
    dirty(mesh, corners as i64)?;
    view(mesh, 2, &made.get_item(0)?)
}
#[pyfunction]
fn remove_element(mesh: &Obj<'_>, element: &Obj<'_>, kind: u8) -> PyResult<()> {
    let op = if kind == 1 {
        "remove_edge"
    } else {
        "remove_face"
    };
    let k = key(mesh, element, kind, false, op)?;
    let st = mesh.getattr("_store")?;
    if kind == 1 {
        dirty(mesh, 1)?;
        st.call_method1(op, (k,))?;
    } else {
        checked(st.call_method1(op, (k,)), mesh.py(), op)?;
        dirty(mesh, 1)?;
    }
    Ok(())
}
#[pyfunction]
fn relink_element(mesh: &Obj<'_>, element: &Obj<'_>, kind: u8) -> PyResult<()> {
    let op = if kind == 1 {
        "relink_edge"
    } else {
        "relink_face"
    };
    let k = key(mesh, element, kind, false, op)?;
    let st = mesh.getattr("_store")?;
    let done = checked(st.call_method1(op, (&k,)), mesh.py(), op)?
        .get_item(0)?
        .is_truthy()?;
    clear_bulk(mesh)?;
    if done {
        if kind == 2 {
            let mut count = 0;
            for ring in st.call_method1("face_rings", (k,))?.try_iter()? {
                count += ring?.len()?;
            }
            dirty(mesh, count as i64)?;
        }
        mesh.call_method0("mark_modified")?;
    }
    Ok(())
}
#[pyfunction]
fn add_hole<'py>(
    mesh: &Obj<'py>,
    face: &Obj<'py>,
    loop_positions: &Obj<'py>,
) -> PyResult<Bound<'py, PyList>> {
    let py = mesh.py();
    let loop_positions = py.get_type::<PyList>().call1((loop_positions,))?;
    let k = key(mesh, face, 2, true, "add_hole")?;
    let points = PyList::empty(py);
    for p in loop_positions.try_iter()? {
        points.append(mesh.call_method1("_input_point", (p?,))?)?;
    }
    let made = checked(
        mesh.getattr("_store")?
            .call_method1("add_hole", (k, &points)),
        py,
        "add_hole",
    )?;
    dirty(mesh, points.len() as i64)?;
    let out = PyList::empty(py);
    for k in made.get_item(0)?.try_iter()? {
        out.append(view(mesh, 0, &k?)?)?;
    }
    Ok(out)
}
#[pyfunction]
fn remove_hole(mesh: &Obj<'_>, face: &Obj<'_>, ring: &Obj<'_>) -> PyResult<()> {
    let k = key(mesh, face, 2, false, "remove_hole")?;
    let keys = PyList::empty(mesh.py());
    for v in ring.try_iter()? {
        keys.append(key(mesh, &v?, 0, false, "remove_hole")?)?;
    }
    clear_bulk(mesh)?;
    mesh.getattr("_store")?
        .call_method1("remove_hole", (k, keys))?;
    Ok(())
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(dirty, m)?)?;
    m.add_function(wrap_pyfunction!(owned, m)?)?;
    m.add_function(wrap_pyfunction!(add_face, m)?)?;
    m.add_function(wrap_pyfunction!(remove_element, m)?)?;
    m.add_function(wrap_pyfunction!(relink_element, m)?)?;
    m.add_function(wrap_pyfunction!(add_hole, m)?)?;
    m.add_function(wrap_pyfunction!(remove_hole, m)?)?;
    Ok(())
}
