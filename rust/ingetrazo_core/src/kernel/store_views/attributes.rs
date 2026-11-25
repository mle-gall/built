// SPDX-License-Identifier: GPL-3.0-or-later
//! Read only the face keys and attribute column for draw, export and material scans.
use super::{Obj, PyMeshStore};
use crate::store::keys::Kind;
use numpy::IntoPyArray;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::HashSet;

fn rows(mesh: &Obj<'_>) -> PyResult<Vec<(u128, i64)>> {
    let store = mesh.getattr("_store")?;
    let st = PyMeshStore::read(&store)?;
    let data = &st.inner;
    Ok(data
        .live_slots(Kind::Face)
        .into_iter()
        .map(|f| {
            (
                data.key(Kind::Face, f),
                *data.state.fattr.get(f as usize) as i64 - 1,
            )
        })
        .collect())
}

#[pyclass(module = "ingetrazo_core")]
struct FaceAttributes {
    entries: std::vec::IntoIter<(u128, i64)>,
    seen: HashSet<i64>,
    own: Py<PyDict>,
    table: Py<PyAny>,
}
#[pymethods]
impl FaceAttributes {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__<'py>(&mut self, py: Python<'py>) -> PyResult<Option<Obj<'py>>> {
        for (key, index) in self.entries.by_ref() {
            if let Some(d) = self.own.bind(py).get_item(key)?.filter(|v| !v.is_none()) {
                return Ok(Some(d));
            }
            if self.seen.insert(index) {
                let attrs = if index < 0 {
                    PyDict::new(py)
                } else {
                    super::properties::copy_attrs(&self.table.bind(py).get_item(index)?)?
                };
                return Ok(Some(attrs.into_any()));
            }
        }
        Ok(None)
    }
}
#[pyfunction]
fn attribute_rows(mesh: &Obj<'_>) -> PyResult<FaceAttributes> {
    let owner = super::properties::attribute_owner(mesh)?;
    Ok(FaceAttributes {
        entries: rows(mesh)?.into_iter(),
        seen: HashSet::new(),
        own: owner.getattr("_attr_own")?.cast_into()?.unbind(),
        table: owner.getattr("_attr_table")?.unbind(),
    })
}

#[pyfunction]
#[pyo3(signature=(mesh,overrides=None,*,face_attr=None))]
pub(crate) fn attr_columns<'py>(
    mesh: &Obj<'py>,
    overrides: Option<&Obj<'py>>,
    face_attr: Option<&Obj<'py>>,
) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let py = mesh.py();
    let owner = super::properties::attribute_owner(mesh)?;
    let own = owner.getattr("_attr_own")?.cast_into::<PyDict>()?;
    let table = py
        .get_type::<PyList>()
        .call1((owner.getattr("_attr_table")?,))?
        .cast_into::<PyList>()?;
    let overrides = match overrides {
        Some(v) if v.is_truthy()? => Some(v),
        _ => None,
    };
    if let Some(fa) = face_attr {
        if own.is_empty() && overrides.is_none() {
            return Ok((table.into_any(), fa.call_method0("copy")?));
        }
    }
    let entries = rows(mesh)?;
    let fa = if let Some(v) = face_attr {
        v.call_method0("copy")?
    } else {
        entries
            .iter()
            .map(|(_, i)| *i)
            .collect::<Vec<_>>()
            .into_pyarray(py)
            .into_any()
    };
    if !own.is_empty() || overrides.is_some() {
        for (i, (key, _)) in entries.into_iter().enumerate() {
            let over = overrides
                .map(|o| o.call_method1("get", (key,)))
                .transpose()?
                .filter(|v| !v.is_none());
            if let Some(attr) = over.or(own.get_item(key)?).filter(|v| !v.is_none()) {
                fa.set_item(i, table.len())?;
                table.append(attr)?;
            }
        }
    }
    Ok((table.into_any(), fa))
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(attribute_rows, m)?)?;
    m.add_function(wrap_pyfunction!(attr_columns, m)?)?;
    Ok(())
}
