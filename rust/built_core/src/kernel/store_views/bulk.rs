// SPDX-License-Identifier: GPL-3.0-or-later
//! Ordered bulk mesh construction. Reuse the staged store columns only when
//! callers return the vertex sequence from the preceding weld.
use super::{edits::view, properties::copy_attrs, Obj};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyInt, PyList, PyString, PyTuple},
};
use std::collections::{HashMap, HashSet};
fn api(py: Python<'_>) -> PyResult<Obj<'_>> {
    crate::model::module(py, "built_core.store_mesh")
}
fn bump(m: &Obj<'_>) -> PyResult<()> {
    m.setattr("_chunk_dirty", true)?;
    m.setattr("_mut_serial", m.getattr("_mut_serial")?.add(1)?)
}
fn refuse(m: &Obj<'_>, msg: &str) -> PyResult<PyErr> {
    Ok(PyErr::from_value(
        api(m.py())?.getattr("StoreRefusal")?.call1((msg,))?,
    ))
}
fn views<'py>(mesh: &Obj<'py>, keys: &Obj<'py>, kind: usize) -> PyResult<Bound<'py, PyList>> {
    let out = PyList::empty(mesh.py());
    for k in keys.try_iter()? {
        out.append(view(mesh, kind, &k?)?)?;
    }
    Ok(out)
}
fn column<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let np = crate::model::module(v.py(), "numpy")?;
    np.call_method1("ascontiguousarray", (v, np.getattr("int64")?))?
        .call_method1("reshape", (-1,))
}
#[pyfunction]
pub(crate) fn table_index(table: &Obj<'_>, index: &Obj<'_>, a: &Obj<'_>) -> PyResult<isize> {
    if !a.is_truthy()? {
        return Ok(-1);
    }
    let key = a.repr()?;
    let hit = index.call_method1("get", (&key,))?;
    if !hit.is_none() {
        return hit.extract();
    }
    let i = table.len()?;
    index.set_item(key, i)?;
    table.call_method1("append", (copy_attrs(a)?,))?;
    Ok(i as isize)
}
#[pyfunction]
fn bulk_weld<'py>(mesh: &Obj<'py>, pos: &Obj<'py>) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let py = mesh.py();
    let np = crate::model::module(py, "numpy")?;
    let d = mesh.getattr("__dict__")?;
    let store = mesh.getattr("_store")?;
    let fresh = store.getattr("revision")?.eq(0)?
        && store
            .call_method0("counts")?
            .extract::<(usize, usize, usize)>()?
            == (0, 0, 0)
        && store.getattr("_model_frame")?.is_truthy()?;
    let mut dtype = np.getattr("float64")?;
    if fresh
        && np
            .call_method1("asarray", (pos,))?
            .getattr("dtype")?
            .eq(np.getattr("float32")?)?
    {
        dtype = np.getattr("float32")?;
    }
    let mut pts = np.call_method1("ascontiguousarray", (pos, &dtype))?;
    if fresh {
        pts = pts.call_method1("reshape", (-1, 3))?;
    }
    if pts.is_empty()? {
        return Ok((
            PyList::empty(py).into_any(),
            np.call_method1("empty", (0, np.getattr("int64")?))?,
        ));
    }
    let (keys, ids) = if fresh {
        let ns = crate::model::module(py, "built_core.mesh")?
            .getattr("_KEY_NAMESPACES")?
            .call_method0("__next__")?;
        let kw = api(py)?.call_method0("_weld_args")?.cast_into::<PyDict>()?;
        let result = store
            .get_type()
            .call_method("_bulk_points", (ns, &pts), Some(&kw))?;
        store.call_method1("_extend_bulk", (result.get_item(0)?,))?;
        (
            store.call_method1("keys", ("vertex",))?,
            result.get_item(1)?,
        )
    } else {
        let plan = d.call_method1("get", ("_planned_points",))?;
        let supplied = if plan.is_truthy()? {
            let p = PyList::empty(py);
            for point in pts.try_iter()? {
                let point = point?;
                p.append(
                    plan.call_method1("get", (py.get_type::<PyTuple>().call1((&point,))?, &point))?,
                )?;
            }
            Some(np.call_method1("asarray", (p, dtype))?)
        } else {
            None
        };
        let result = store.call_method1("weld_existing", (pts, supplied))?;
        (result.get_item(0)?, result.get_item(1)?)
    };
    let vs = views(mesh, &keys, 0)?;
    if fresh {
        let b = PyDict::new(py);
        b.set_item("stage", "points")?;
        b.set_item("vobjs", &vs)?;
        d.set_item("_bulk", b)?;
    } else {
        d.call_method1("pop", ("_bulk", py.None()))?;
    }
    Ok((vs.into_any(), ids))
}
#[pyfunction]
#[pyo3(signature=(mesh,vobjs,ia,ib,flags=None))]
fn add_edges_welded<'py>(
    mesh: &Obj<'py>,
    vobjs: &Obj<'py>,
    ia: &Obj<'py>,
    ib: &Obj<'py>,
    flags: Option<&Obj<'py>>,
) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let store = mesh.getattr("_store")?;
    let b = mesh.getattr("__dict__")?.call_method1("get", ("_bulk",))?;
    if !b.is_none() && b.get_item("vobjs")?.is(vobjs) && b.get_item("stage")?.eq("points")? {
        if !store
            .call_method1("_bulk_edges", (column(ia)?, column(ib)?, flags))?
            .is_truthy()?
        {
            return Err(refuse(mesh, "unsupported bulk edge flags or incidence")?);
        }
        bump(mesh)?;
        b.set_item("stage", "edges")?;
        return api(py)?.call_method1("_EdgeMap", (mesh, vobjs));
    }
    let fail = "add_edges_welded requires live, representable incidence in this mesh";
    let mut pairs = vec![];
    for (i, a) in ia.try_iter()?.enumerate() {
        let a = py.get_type::<PyInt>().call1((a?,))?.extract::<isize>()?;
        let b = py
            .get_type::<PyInt>()
            .call1((ib.get_item(i)?,))?
            .extract::<isize>()?;
        pairs.push((a, b));
    }
    for v in vobjs.try_iter()? {
        if super::edits::owned(mesh, &v?, 0, true)?.is_none() {
            return Err(refuse(mesh, fail)?);
        }
    }
    for &(a, b) in &pairs {
        let va = vobjs.get_item(a)?;
        let vb = vobjs.get_item(b)?;
        if !va.getattr("key")?.eq(vb.getattr("key")?)?
            && !mesh.call_method1("find_edge", (va, vb))?.is_none()
        {
            return Err(refuse(mesh, fail)?);
        }
    }
    if let Some(flags) = flags {
        for row in flags.try_iter()? {
            let row = row?;
            let curve = row.get_item(1)?;
            let tag = row.get_item(2)?;
            if (!curve.is_none() && !curve.is_instance_of::<PyInt>())
                || (!tag.is_none() && !tag.is_instance_of::<PyString>())
            {
                return Err(refuse(mesh, fail)?);
            }
        }
    }
    mesh.call_method0("_bump")?;
    let out = PyDict::new(py);
    let n = vobjs.len()? as i128;
    for (i, (a, b)) in pairs.into_iter().enumerate() {
        let ka = vobjs.get_item(a)?.getattr("key")?;
        let kb = vobjs.get_item(b)?.getattr("key")?;
        if ka.eq(&kb)? {
            continue;
        }
        let k = a.min(b) as i128 * n + a.max(b) as i128;
        let edge = match out.get_item(k)? {
            Some(e) => e,
            None => {
                let made = store.call_method1("link_edge", (ka, kb))?;
                let e = view(mesh, 1, &made.get_item(0)?)?;
                out.set_item(k, &e)?;
                e
            }
        };
        if let Some(flags) = flags {
            for (name, v) in ["soft", "curve", "layer", "hidden", "flat"]
                .into_iter()
                .zip(flags.get_item(i)?.try_iter()?)
            {
                let v = v?;
                if v.is_none() {
                    continue;
                }
                if matches!(name, "curve" | "layer") {
                    edge.setattr(name, v)?;
                } else if v.is_truthy()? {
                    edge.setattr(name, true)?;
                }
            }
        }
    }
    Ok(out.into_any())
}
#[pyfunction]
#[pyo3(signature=(mesh,vobjs,inverse,ring_sizes,face_ring_counts,attrs=None,edge_map=None))]
fn add_faces_welded<'py>(
    mesh: &Obj<'py>,
    vobjs: &Obj<'py>,
    inverse: &Obj<'py>,
    ring_sizes: &Obj<'py>,
    face_ring_counts: &Obj<'py>,
    attrs: Option<&Obj<'py>>,
    edge_map: Option<&Obj<'py>>,
) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let model = api(py)?;
    let st = mesh.getattr("_store")?;
    let d = mesh.getattr("__dict__")?;
    let b = d.call_method1("get", ("_bulk",))?;
    let edge_type = model.getattr("_EdgeMap")?;
    let staged = !b.is_none()
        && b.get_item("vobjs")?.is(vobjs)
        && matches!(
            b.get_item("stage")?.extract::<String>()?.as_str(),
            "points" | "edges"
        )
        && match edge_map {
            None => true,
            Some(m) => m.is_instance(&edge_type)?,
        };
    let sizes = column(ring_sizes)?;
    let counts = column(face_ring_counts)?;
    let inv = column(inverse)?;
    let lens = sizes.extract::<numpy::PyReadonlyArray1<i64>>()?;
    let nr = counts.extract::<numpy::PyReadonlyArray1<i64>>()?;
    let lens = lens.as_slice()?;
    let nr = nr.as_slice()?;
    let invalid = lens.iter().chain(nr).any(|n| *n < 1);
    if staged {
        if nr.is_empty() {
            bump(mesh)?;
            return Ok(PyList::empty(py).into_any());
        }
        if invalid {
            return Err(PyValueError::new_err(
                "bulk face ring sizes and counts must be positive",
            ));
        }
        let fa = if let Some(attrs) = attrs {
            let table = d.get_item("_attr_table")?;
            let index = PyDict::new(py);
            let mut by_id = HashMap::new();
            let mut values = vec![-1i64; nr.len()];
            for (i, a) in attrs.try_iter()?.enumerate() {
                let a = a?;
                if !a.is_truthy()? {
                    continue;
                }
                let id = a.as_ptr();
                let row = match by_id.get(&id) {
                    Some(&v) => v,
                    None => {
                        let v = table_index(&table, index.as_any(), &a)? as i64;
                        by_id.insert(id, v);
                        v
                    }
                };
                *values.get_mut(i).ok_or_else(|| {
                    pyo3::exceptions::PyIndexError::new_err("face attribute index out of range")
                })? = row;
            }
            Some(numpy::IntoPyArray::into_pyarray(values, py))
        } else {
            None
        };
        bump(mesh)?;
        st.call_method1("_bulk_faces", (inv, &sizes, &counts, fa))?;
        d.call_method1("pop", ("_bulk", py.None()))?;
        return Ok(views(mesh, &st.call_method1("keys", ("face",))?, 2)?.into_any());
    }
    let msg = "bulk faces require vertices and edges from this store";
    if invalid {
        return Err(refuse(mesh, msg)?);
    }
    for v in vobjs.try_iter()? {
        if super::edits::owned(mesh, &v?, 0, true)?.is_none() {
            return Err(refuse(mesh, msg)?);
        }
    }
    if let Some(map) = edge_map {
        if !(map.is_instance_of::<PyDict>() || map.is_instance(&edge_type)?) {
            return Err(refuse(mesh, msg)?);
        }
    }
    let indices = inv.extract::<numpy::PyReadonlyArray1<i64>>()?;
    let indices = indices.as_slice()?;
    let mut at: usize = 0;
    let mut rings = vec![];
    for &n in lens {
        let end = at.saturating_add(n as usize).min(indices.len());
        rings.push(&indices[at..end]);
        at = end;
    }
    let mut faces = vec![];
    at = 0;
    for &n in nr {
        let end = at.saturating_add(n as usize).min(rings.len());
        faces.push(&rings[at..end]);
        at = end;
    }
    let mut pairs = Vec::new();
    if let Some(map) = edge_map.filter(|m| m.is_instance_of::<PyDict>()) {
        let n = vobjs.len()? as i128;
        let mut seen = HashSet::new();
        for face in &faces {
            for ring in *face {
                for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                    let key = i128::from(a.min(b)) * n + i128::from(a.max(b));
                    if seen.insert(key) {
                        pairs.push((key, a, b));
                    }
                }
            }
        }
        for &(key, a, b) in &pairs {
            if !map
                .call_method1("get", (key,))?
                .eq(mesh.call_method1("find_edge", (vobjs.get_item(a)?, vobjs.get_item(b)?))?)?
            {
                return Err(refuse(mesh, msg)?);
            }
        }
    }
    mesh.call_method0("_bump")?;
    let out = PyList::empty(py);
    for (i, face) in faces.into_iter().enumerate() {
        let mut r = vec![];
        for ring in face {
            let mut keys = Vec::with_capacity(ring.len());
            for &v in *ring {
                keys.push(vobjs.get_item(v)?.getattr("key")?);
            }
            r.push(keys);
        }
        let f = view(
            mesh,
            2,
            &st.call_method1("add_welded_face", (r,))?.get_item(0)?,
        )?;
        if let Some(attrs) = attrs {
            let a = attrs.get_item(i)?;
            if a.is_truthy()? {
                f.getattr("attrs")?.call_method1("update", (a,))?;
            }
        }
        out.append(f)?;
    }
    if let Some(map) = edge_map {
        for (key, a, b) in pairs {
            map.set_item(
                key,
                mesh.call_method1("find_edge", (vobjs.get_item(a)?, vobjs.get_item(b)?))?,
            )?;
        }
    }
    Ok(out.into_any())
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(bulk_weld, m)?)?;
    m.add_function(wrap_pyfunction!(table_index, m)?)?;
    m.add_function(wrap_pyfunction!(add_faces_welded, m)?)?;
    m.add_function(wrap_pyfunction!(add_edges_welded, m)?)?;
    Ok(())
}
