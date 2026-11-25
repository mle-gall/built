// SPDX-License-Identifier: GPL-3.0-or-later
//! Lazy snapshot and bulk-edge mappings used by the mesh plugin API.
use super::Obj;
use pyo3::{
    exceptions::{PyIndexError, PyKeyError, PyOverflowError, PyTypeError, PyValueError},
    prelude::*,
    types::{PyDict, PyInt, PyIterator, PyTuple},
};
#[pyclass(subclass, module = "built_core")]
pub(super) struct SnapshotPositions;
#[pymethods]
impl SnapshotPositions {
    #[new]
    fn new(_snap: &Obj<'_>) -> Self {
        Self
    }
    fn __contains__(slf: &Bound<'_, Self>, key: &Obj<'_>) -> PyResult<bool> {
        if !key.is_instance_of::<PyInt>()
            || !slf
                .getattr("snap")?
                .call_method1("has", (key,))?
                .is_truthy()?
        {
            return Ok(false);
        }
        Ok((key.extract::<u128>()? >> 38) & 3 == 0)
    }
    fn __getitem__<'py>(slf: &Bound<'py, Self>, key: &Obj<'py>) -> PyResult<Obj<'py>> {
        let py = slf.py();
        match slf.getattr("snap")?.call_method1("vertex_position", (key,)) {
            Ok(v) => py
                .get_type::<crate::math::Vec3>()
                .call1(v.cast_into::<PyTuple>()?),
            Err(e)
                if e.is_instance_of::<PyKeyError>(py)
                    || e.is_instance_of::<PyOverflowError>(py)
                    || e.is_instance_of::<PyTypeError>(py) =>
            {
                Err(PyKeyError::new_err(key.clone().unbind()))
            }
            Err(e) => Err(e),
        }
    }
    fn __iter__<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyIterator>> {
        slf.getattr("snap")?
            .call_method1("keys", ("vertex",))?
            .try_iter()
    }
    fn __len__(slf: &Bound<'_, Self>) -> PyResult<usize> {
        slf.getattr("snap")?
            .call_method0("counts")?
            .get_item(0)?
            .extract()
    }
}
#[pyclass(subclass, module = "built_core")]
pub(super) struct BulkEdgeMap;
fn index_edges<'py>(map: &Bound<'py, BulkEdgeMap>) -> PyResult<Obj<'py>> {
    let full = map.getattr("_full")?;
    if !full.is_none() {
        return Ok(full);
    }
    let py = map.py();
    let verts = map.getattr("vobjs")?;
    let u = verts.len()?;
    let ids = PyDict::new(py);
    let out = PyDict::new(py);
    for (i, v) in verts.try_iter()?.enumerate() {
        ids.set_item(v?.getattr("key")?, i)?
    }
    for e in map.getattr("m")?.getattr("edges")?.try_iter()? {
        let e = e?;
        let rank = |end: &str| -> PyResult<Option<u128>> {
            ids.get_item(e.getattr(end)?.getattr("key")?)?
                .map(|v| v.extract())
                .transpose()
        };
        if let (Some(a), Some(b)) = (rank("v0")?, rank("v1")?) {
            let key = a.min(b) * u as u128 + a.max(b);
            if !out.contains(key)? {
                out.set_item(key, e)?
            }
        }
    }
    map.setattr("_full", &out)?;
    Ok(out.into_any())
}
#[pymethods]
impl BulkEdgeMap {
    #[new]
    fn new(_m: &Obj<'_>, _vobjs: &Obj<'_>) -> Self {
        Self
    }
    fn __iter__<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyIterator>> {
        index_edges(slf)?.try_iter()
    }
    fn __len__(slf: &Bound<'_, Self>) -> PyResult<usize> {
        index_edges(slf)?.len()
    }
    fn __getitem__<'py>(slf: &Bound<'py, Self>, key: &Obj<'py>) -> PyResult<Obj<'py>> {
        let py = slf.py();
        let find = || -> PyResult<Obj<'py>> {
            let verts = slf.getattr("vobjs")?;
            let pair = py
                .get_type::<PyInt>()
                .call1((key,))?
                .call_method1("__divmod__", (verts.len()?,))?;
            slf.getattr("m")?.call_method1(
                "find_edge",
                (
                    verts.get_item(pair.get_item(0)?)?,
                    verts.get_item(pair.get_item(1)?)?,
                ),
            )
        };
        match find() {
            Ok(e) if !e.is_none() => Ok(e),
            Err(e)
                if !(e.is_instance_of::<PyIndexError>(py)
                    || e.is_instance_of::<PyValueError>(py)
                    || e.is_instance_of::<PyTypeError>(py)) =>
            {
                Err(e)
            }
            _ => Err(PyKeyError::new_err(key.clone().unbind())),
        }
    }
}
