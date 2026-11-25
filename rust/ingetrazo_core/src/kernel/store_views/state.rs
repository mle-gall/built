// SPDX-License-Identifier: GPL-3.0-or-later
//! Compare shared store snapshots, including Python-owned face attribute overrides.
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};
use std::collections::BTreeSet;
type Obj<'py> = Bound<'py, PyAny>;
pub(crate) fn snapshot_attr<'py>(
    state: &Obj<'py>,
    key: u128,
    row: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let own = state.getattr("own")?.call_method1("get", (key,))?;
    if !own.is_none() {
        return Ok(own);
    }
    if row.ge(0)? {
        state.getattr("table")?.get_item(row)
    } else {
        Ok(PyDict::new(state.py()).into_any())
    }
}
fn overridden(state: &Obj<'_>, other: &Obj<'_>) -> PyResult<BTreeSet<u128>> {
    let mut keys = BTreeSet::new();
    for s in [state, other] {
        for key in s.getattr("own")?.try_iter()? {
            keys.insert(key?.extract()?);
        }
    }
    Ok(keys)
}
fn mapping_equal(state: &Obj<'_>, other: &Obj<'_>) -> PyResult<bool> {
    let py = state.py();
    let mapping = crate::model::module(py, "collections.abc")?.getattr("Mapping")?;
    for key in state.getattr("_KEYS")?.try_iter()? {
        let key = key?;
        if !other.contains(&key)? {
            return Ok(false);
        }
        let mine = state.get_item(&key)?;
        let theirs = other.get_item(&key)?;
        if key.eq("vpos")? {
            if mine.len()? != theirs.len()? {
                return Ok(false);
            }
            for pair in theirs.call_method0("items")?.try_iter()? {
                let pair = pair?;
                let k = pair.get_item(0)?;
                if !mine.contains(&k)? || !mine.get_item(&k)?.eq(pair.get_item(1)?)? {
                    return Ok(false);
                }
            }
        } else {
            let same = if mine.is_instance(&mapping)? {
                py.get_type::<PyDict>().call1((mine,))?.eq(&theirs)?
            } else {
                mine.eq(&theirs)?
            };
            if !same {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
#[pyclass(subclass, module = "ingetrazo_core")]
pub(super) struct MeshSnapshotRecord;
#[pymethods]
impl MeshSnapshotRecord {
    #[new]
    #[pyo3(signature=(*_args,**_kw))]
    fn new(_args: &Bound<'_, PyTuple>, _kw: Option<&Bound<'_, PyDict>>) -> Self {
        Self
    }
    fn changes_since<'py>(slf: &Bound<'py, Self>, before: &Obj<'py>) -> PyResult<Obj<'py>> {
        let now = slf.getattr("snap")?;
        let old = before.getattr("snap")?;
        let mut changes = vec![];
        for k in overridden(slf.as_any(), before)? {
            if now.call_method1("has", (k,))?.is_truthy()?
                && old.call_method1("has", (k,))?.is_truthy()?
            {
                let a = snapshot_attr(
                    slf.as_any(),
                    k,
                    &now.call_method1("face_info", (k,))?.get_item(0)?,
                )?;
                let b = snapshot_attr(
                    before,
                    k,
                    &old.call_method1("face_info", (k,))?.get_item(0)?,
                )?;
                if !a.eq(&b)? {
                    changes.push(k)
                }
            }
        }
        now.call_method1("changes_since", (old, changes))
    }
    fn __eq__<'py>(slf: &Bound<'py, Self>, other: &Obj<'py>) -> PyResult<Obj<'py>> {
        let py = slf.py();
        let this = slf.as_any();
        let convert = |v: bool| -> Obj<'py> { v.into_pyobject(py).unwrap().to_owned().into_any() };
        if this.is(other) {
            return Ok(convert(true));
        }
        if other.is_instance_of::<PyDict>() {
            return Ok(convert(mapping_equal(this, other)?));
        }
        if !other.is_instance_of::<MeshSnapshotRecord>() {
            return Ok(py.NotImplemented().into_bound(py));
        }
        let now = slf.getattr("snap")?;
        if !other
            .getattr("snap")?
            .call_method1("same_as", (&now,))?
            .is_truthy()?
        {
            return Ok(convert(false));
        }
        for k in overridden(this, other)? {
            if now.call_method1("has", (k,))?.is_truthy()? {
                let row = now.call_method1("face_info", (k,))?.get_item(0)?;
                if !snapshot_attr(this, k, &row)?.eq(snapshot_attr(other, k, &row)?)? {
                    return Ok(convert(false));
                }
            }
        }
        Ok(convert(true))
    }
    fn __len__(slf: &Bound<'_, Self>) -> PyResult<usize> {
        slf.getattr("_KEYS")?.len()
    }
    fn __iter__<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, pyo3::types::PyIterator>> {
        slf.getattr("_KEYS")?.try_iter()
    }
    fn __getitem__<'py>(slf: &Bound<'py, Self>, name: &Obj<'py>) -> PyResult<Obj<'py>> {
        let cache = slf.getattr("_cache")?.cast_into::<PyDict>()?;
        if let Some(hit) = cache.get_item(name)? {
            return Ok(hit);
        }
        if !slf.getattr("_KEYS")?.contains(name)? {
            return Err(pyo3::exceptions::PyKeyError::new_err(name.clone().unbind()));
        }
        let snap = slf.getattr("snap")?;
        let hit = if name.eq("vpos")? {
            crate::model::module(slf.py(), "ingetrazo_core.store_mesh")?
                .call_method1("_SnapPositions", (snap,))?
        } else {
            snap.call_method1(
                "field",
                (
                    name,
                    slf.getattr("mesh")?,
                    slf.getattr("own")?,
                    slf.getattr("table")?,
                ),
            )?
        };
        cache.set_item(name, &hit)?;
        Ok(hit)
    }
}
