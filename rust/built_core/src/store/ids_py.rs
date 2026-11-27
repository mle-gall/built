// SPDX-License-Identifier: GPL-3.0-or-later
//! Python side of the id sources a block is built from: `ids` is a
//! 3-tuple (vertex, edge, face) of `None` (fresh), an int (implicit:
//! `first + index`) or a uint64 array (one packed id per element), all in
//! process space (`formats/built` remaps a file's table on read).
use super::ids::IdSource;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

pub(crate) enum Owned {
    Fresh,
    Legacy,
    First(u64),
    Array(Vec<u64>),
}

pub(crate) fn source(v: &Bound<'_, PyAny>) -> PyResult<Owned> {
    if v.is_none() {
        return Ok(Owned::Fresh);
    }
    if v.extract::<String>().is_ok_and(|s| s == "legacy") {
        return Ok(Owned::Legacy);
    }
    if let Ok(first) = v.extract::<u64>() {
        return Ok(Owned::First(first));
    }
    let np = v.py().import("numpy")?;
    let a = np.call_method1("ascontiguousarray", (v, np.getattr("uint64")?))?.call_method1("reshape", (-1,))?;
    let a: numpy::PyReadonlyArray1<u64> = a.extract()?;
    Ok(Owned::Array(a.as_slice()?.to_vec()))
}

pub(crate) fn sources(ids: Option<&Bound<'_, PyAny>>) -> PyResult<[Owned; 3]> {
    let Some(ids) = ids.filter(|v| !v.is_none()) else {
        return Ok([Owned::Fresh, Owned::Fresh, Owned::Fresh]);
    };
    if ids.extract::<String>().is_ok_and(|s| s == "legacy") {
        return Ok([Owned::Legacy, Owned::Legacy, Owned::Legacy]);
    }
    if ids.len()? != 3 {
        return Err(PyValueError::new_err("ids: (vertex, edge, face)"));
    }
    Ok([source(&ids.get_item(0)?)?, source(&ids.get_item(1)?)?, source(&ids.get_item(2)?)?])
}

fn one(o: &Owned) -> IdSource<'_> {
    match o {
        Owned::Fresh => IdSource::Fresh,
        Owned::Legacy => IdSource::Legacy,
        Owned::First(f) => IdSource::First(*f),
        Owned::Array(a) => IdSource::Array(a.as_slice()),
    }
}

pub(crate) fn borrow(owned: &[Owned; 3]) -> [IdSource<'_>; 3] {
    [one(&owned[0]), one(&owned[1]), one(&owned[2])]
}

/// After a source was built element by element (small JSON meshes, the
/// staged `_build_geometry` path), give the store the ids the source
/// names: the plan's source rows map them, as `Store::from_block` does.
/// A legacy source is numbered in list order.
#[pyfunction]
pub(crate) fn apply_source_ids(mesh: &Bound<'_, PyAny>, src: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = mesh.py();
    let Some(store) = mesh.getattr_opt("_store")?.filter(|s| !s.is_none()) else {
        return Ok(());
    };
    let Ok(mut store) = store.extract::<PyRefMut<'_, super::py::PyMeshStore>>() else {
        return Ok(());
    };
    let ids = src.call_method1("get", ("ids",))?;
    let owned = sources(Some(&ids))?;
    if owned.iter().all(|o| matches!(o, Owned::Legacy)) || ids.is_none() {
        if !ids.is_none() {
            store.inner.number_legacy();
        }
        return Ok(());
    }
    let plan = py.import("built_core")?.getattr("deferred_plan")?.call1((
        src.get_item("pos")?,
        src.get_item("edges")?,
        src.call_method1("get", ("flags",))?,
        src.call_method1("get", ("curves",))?,
        src.call_method1("get", ("layers",))?,
        pyo3::types::PyList::empty(py),
        src.get_item("rings")?,
        src.get_item("sizes")?,
        src.get_item("counts")?,
        src.call_method1("get", ("sequential", false))?.is_truthy()?,
    ))?;
    let extra = plan.getattr("extra")?;
    let np = py.import("numpy")?;
    let rows = |k: &str| -> PyResult<usize> { np.call_method1("shape", (src.get_item(k)?,))?.get_item(0)?.extract() };
    let planned = crate::file_ids::plan_ids(
        py,
        &ids,
        extra.get_item("vertex_src")?.extract()?,
        extra.get_item("edge_src")?.extract()?,
        rows("pos")?,
        rows("edges")?,
        rows("counts")?,
    )?;
    let st = &mut store.inner;
    for kind in super::keys::Kind::ALL {
        let k = kind as usize;
        let ids: numpy::PyReadonlyArray1<u64> = planned.get_item(k)?.extract()?;
        let ids = ids.as_slice()?;
        let order = st.state.order[k].live(&st.state.slots[k]);
        if order.len() != ids.len() {
            // Not the plan's elements: keep what the build minted.
            continue;
        }
        let n = st.state.slots[k].len();
        let mut col = vec![0u64; n];
        for (i, &s) in order.iter().enumerate() {
            col[s as usize] = ids[i];
        }
        st.state.ids[k] = super::ids::IdColumn::Dense(super::cow::CowVec::from_vec(col));
    }
    Ok(())
}
