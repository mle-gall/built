// SPDX-License-Identifier: GPL-3.0-or-later
//! Tag and material tallies of a store-backed mesh (moved from
//! `core/store_parts.py` for the purge survey).
//!
//! The face attribute rows and the edge tags come from the store. The mesh
//! facade still owns the attribute table (`_attr_table`) and the faces that
//! carry a dict of their own (`_attr_own`); both are read once per call.
//! Counts are added in the order the Python version added them: own dicts,
//! then attribute rows, then edge tags, each ascending.
use super::keys::{Kind, NONE};
use super::py::PyMeshStore;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PySet};
use std::collections::{BTreeMap, HashSet};

type Obj<'py> = Bound<'py, PyAny>;

/// `(store facade, attribute table, own dicts)` of a live store-backed mesh.
fn parts<'py>(mesh: &Obj<'py>) -> PyResult<Option<(Obj<'py>, Bound<'py, PyList>, Bound<'py, PyDict>)>> {
    let py = mesh.py();
    let store_mesh = py.import("built_core.store_mesh")?.getattr("StoreMesh")?;
    if !mesh.get_type().is(&store_mesh) {
        return Ok(None);
    }
    let d = mesh.getattr("__dict__")?;
    let table = d.get_item("_attr_table")?.cast_into::<PyList>()?;
    let own = d.get_item("_attr_own")?.cast_into::<PyDict>()?;
    Ok(Some((mesh.getattr("_store")?, table, own)))
}

/// Per attribute row (−1 for none), how many live faces without a dict of
/// their own use it; and the live own dicts in their order.
fn face_rows<'py>(store: &Obj<'py>, own: &Bound<'py, PyDict>) -> PyResult<(BTreeMap<i64, i64>, Vec<Obj<'py>>)> {
    let cell = store.cast::<PyMeshStore>()?;
    let st = cell.borrow();
    let inner = &st.inner;
    let mut own_keys = HashSet::new();
    let mut live_own = Vec::new();
    for (k, attrs) in own.iter() {
        let key: u128 = k.extract()?;
        own_keys.insert(key);
        if inner.slot(Kind::Face, key).is_some() {
            live_own.push(attrs);
        }
    }
    let mut rows = BTreeMap::new();
    for f in inner.live_slots(Kind::Face) {
        if !own_keys.is_empty() && own_keys.contains(&inner.key(Kind::Face, f)) {
            continue;
        }
        let row = *inner.state.fattr.get(f as usize) as i64 - 1;
        *rows.entry(row).or_insert(0) += 1;
    }
    Ok((rows, live_own))
}

fn field<'py>(attrs: &Obj<'py>, name: &str) -> PyResult<Obj<'py>> {
    let py = attrs.py();
    if attrs.is_none() || !attrs.is_truthy()? {
        return Ok(py.None().into_bound(py));
    }
    attrs.call_method1("get", (name,))
}

fn add(tally: &Obj<'_>, name: &Obj<'_>, n: i64) -> PyResult<()> {
    let py = tally.py();
    let name = if name.is_truthy()? { name.clone() } else { py.None().into_bound(py) };
    let current: i64 = tally.get_item(&name)?.extract()?;
    tally.set_item(name, current + n)
}

/// Add every face's and edge's tag to `tally` (a `Counter`; None counts the
/// default tag). False when `mesh` is not store-backed.
#[pyfunction]
pub fn store_layer_tally(mesh: &Obj<'_>, tally: &Obj<'_>) -> PyResult<bool> {
    let Some((store, table, own)) = parts(mesh)? else {
        return Ok(false);
    };
    let (rows, live_own) = face_rows(&store, &own)?;
    for attrs in &live_own {
        add(tally, &field(attrs, "layer")?, 1)?;
    }
    for (row, n) in rows {
        let name = if row >= 0 { field(&table.get_item(row as usize)?, "layer")? } else { mesh.py().None().into_bound(mesh.py()) };
        add(tally, &name, n)?;
    }
    let (edge_layers, names) = {
        let cell = store.cast::<PyMeshStore>()?;
        let st = cell.borrow();
        let inner = &st.inner;
        let mut counts = BTreeMap::new();
        for e in inner.live_slots(Kind::Edge) {
            let l = *inner.state.elayer.get(e as usize);
            *counts.entry(if l == NONE { -1i64 } else { l as i64 }).or_insert(0i64) += 1;
        }
        (counts, inner.state.layers.as_ref().clone())
    };
    let py = mesh.py();
    for (i, n) in edge_layers {
        let name = match usize::try_from(i).ok().and_then(|i| names.get(i)) {
            Some(name) => pyo3::types::PyString::new(py, name).into_any(),
            None => py.None().into_bound(py),
        };
        add(tally, &name, n)?;
    }
    Ok(true)
}

/// The registry material names the mesh's faces wear (`attrs["mat"]`),
/// empties left out; None when `mesh` is not store-backed.
#[pyfunction]
pub fn store_material_names<'py>(mesh: &Obj<'py>) -> PyResult<Option<Bound<'py, PySet>>> {
    let Some((store, table, own)) = parts(mesh)? else {
        return Ok(None);
    };
    let py = mesh.py();
    let (rows, live_own) = face_rows(&store, &own)?;
    let names = PySet::empty(py)?;
    for attrs in &live_own {
        names.add(field(attrs, "mat")?)?;
    }
    for row in rows.keys().filter(|&&r| r >= 0) {
        names.add(field(&table.get_item(*row as usize)?, "mat")?)?;
    }
    names.discard(py.None())?;
    names.discard("")?;
    Ok(Some(names))
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(store_layer_tally, m)?)?;
    m.add_function(wrap_pyfunction!(store_material_names, m)?)?;
    Ok(())
}
