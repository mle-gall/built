// SPDX-License-Identifier: GPL-3.0-or-later
//! Store pieces as lazy key sequences. Row interning follows source row order
//! before face-specific overrides, preserving the archive's table order.
use super::provenance::{self, CopiedKeys, PieceElements};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::{BTreeMap, HashMap};
type Obj<'py> = Bound<'py, PyAny>;
fn module<'py>(py: Python<'py>, name: &str) -> PyResult<Obj<'py>> {
    crate::model::module(py, name)
}
pub(crate) fn mesh_parts<'py>(mesh: &Obj<'py>, solid: bool) -> PyResult<Option<Bound<'py, PyList>>> {
    let py = mesh.py();
    let facade = module(py, "ingetrazo_core.store_mesh")?;
    if !mesh.get_type().is(facade.getattr("StoreMesh")?) {
        return Ok(None);
    }
    let snap = mesh.getattr("_store")?.call_method0("snapshot")?;
    let groups = if solid {
        snap.call_method1(
            "solid_parts",
            (module(py, "core._native")?.call_method0("qt_hypot")?,),
        )?
    } else {
        snap.call_method0("connected_parts")?
    };
    let out = PyList::empty(py);
    let faces = facade.getattr("FaceView")?;
    let edges = facade.getattr("EdgeView")?;
    for pair in groups.try_iter()? {
        let pair = pair?;
        out.append((
            Py::new(py, PieceElements::new(mesh, pair.get_item(0)?, &faces))?,
            Py::new(py, PieceElements::new(mesh, pair.get_item(1)?, &edges))?,
        ))?;
    }
    Ok(Some(out))
}
struct Column<'py> {
    mesh: Option<Obj<'py>>,
    keys: Vec<u128>,
}
fn column<'py>(elements: &Obj<'py>, class: &Obj<'py>) -> PyResult<Option<Column<'py>>> {
    if elements.is_empty()? {
        return Ok(Some(Column {
            mesh: None,
            keys: vec![],
        }));
    }
    if let Ok(views) = elements.cast::<PieceElements>() {
        let view = views.borrow();
        if view.class.bind(elements.py()).is(class) {
            return Ok(Some(Column {
                mesh: Some(view.mesh.bind(elements.py()).clone()),
                keys: view.keys.bind(elements.py()).extract()?,
            }));
        }
    }
    let mut result = Column {
        mesh: None,
        keys: Vec::with_capacity(elements.len()?),
    };
    for e in elements.try_iter()? {
        let e = e?;
        if !e.get_type().is(class) {
            return Ok(None);
        }
        let m = e.getattr("_m")?;
        if result.mesh.as_ref().is_some_and(|v| !v.is(&m)) {
            return Ok(None);
        }
        result.mesh = Some(m);
        result.keys.push(e.getattr("key")?.extract()?);
    }
    Ok(Some(result))
}
pub(crate) fn copy_mesh_part<'py>(faces: &Obj<'py>, edges: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let py = faces.py();
    let model = module(py, "ingetrazo_core.store_mesh")?;
    let Some(fs) = column(faces, &model.getattr("FaceView")?)? else {
        return Ok(None);
    };
    let Some(es) = column(edges, &model.getattr("EdgeView")?)? else {
        return Ok(None);
    };
    if let (Some(a), Some(b)) = (&fs.mesh, &es.mesh) {
        if !a.is(b) {
            return Ok(None);
        }
    }
    let Some(mesh) = fs.mesh.or(es.mesh) else {
        return Ok(None);
    };
    if !mesh.get_type().is(model.getattr("StoreMesh")?) {
        return Ok(None);
    }
    let snap = mesh.getattr("_store")?.call_method0("snapshot")?;
    let source = mesh.getattr("__dict__")?;
    let src_table = source.get_item("_attr_table")?;
    let own = source.get_item("_attr_own")?;
    let rows: Vec<i64> = snap
        .call_method1("face_rows", (&fs.keys,))?
        .call_method0("tolist")?
        .extract()?;
    let table = PyList::empty(py);
    let index = PyDict::new(py);
    let mut map: BTreeMap<i64, i64> = rows.iter().map(|r| (*r, -1)).collect();
    for (r, v) in &mut map {
        if *r >= 0 {
            *v = super::bulk::table_index(table.as_any(), index.as_any(), &src_table.get_item(*r)?)?
                as i64;
        }
    }
    let mut attrs: Vec<i64> = rows.iter().map(|r| map[r]).collect();
    if !own.is_empty()? {
        let positions: HashMap<_, _> = fs.keys.iter().enumerate().map(|(i, &k)| (k, i)).collect();
        let mut overrides = BTreeMap::new();
        for (k, v) in own.cast::<PyDict>()?.iter() {
            if let Some(&i) = positions.get(&k.extract::<u128>()?) {
                overrides.insert(i, v);
            }
        }
        for (i, v) in overrides {
            attrs[i] = super::bulk::table_index(table.as_any(), index.as_any(), &v)? as i64;
        }
    }
    let public = module(py, "ingetrazo_core.mesh")?;
    let ns = public
        .getattr("_KEY_NAMESPACES")?
        .call_method0("__next__")?;
    let kw = model.call_method0("_weld_args")?.cast_into::<PyDict>()?;
    let store = snap.call_method(
        "extract",
        (
            ns,
            &fs.keys,
            &es.keys,
            attrs,
            model.getattr("_hole_filter")?,
        ),
        Some(&kw),
    )?;
    let new = public.call_method0("Mesh")?;
    super::lifecycle::become_store(&new, &store, table.as_any(), None)?;
    new.call_method0("resplit_curves")?;
    let cols = PyDict::new(py);
    cols.set_item("face", (fs.keys, store.call_method1("keys", ("face",))?))?;
    provenance::remember(&new, &mesh, CopiedKeys::new(cols))?;
    Ok(Some(new))
}
