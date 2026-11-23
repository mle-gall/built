// SPDX-License-Identifier: GPL-3.0-or-later
//! Build document placements from their file records. The host supplies
//! mesh handles (including shared linked definitions) and legacy identities.
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyFloat, PyInt, PyList, PyTuple};
use std::sync::atomic::{AtomicUsize, Ordering};

#[pyclass(frozen)]
pub struct FileGroups {
    done: AtomicUsize,
    count: usize,
}

struct Builder<'py> {
    group: Bound<'py, PyAny>,
    meshes: Bound<'py, PyAny>,
    read_mesh: Bound<'py, PyAny>,
    built: bool,
}
impl<'py> Builder<'py> {
    fn group(&self, raw: &Bound<'py, PyDict>, depth: usize) -> PyResult<Bound<'py, PyAny>> {
        let py = raw.py();
        let kwargs = PyDict::new(py);
        if let Some(v) = raw.get_item("name")? {
            if v.is_truthy()? {
                kwargs.set_item("name", v)?;
            }
        }
        if let Some(v) = raw.get_item("uid")? {
            if v.is_truthy()? {
                kwargs.set_item("uid", v.str()?)?;
            }
        }
        let xform = raw.get_item("xform")?.filter(|v| !v.is_none());
        let proto = raw.get_item("proto")?;
        let group = if let (Some(xf), Some(code)) = (xform, proto) {
            let index = py.get_type::<PyInt>().call1((code,))?;
            let group = self
                .group
                .call((self.meshes.get_item(index)?,), Some(&kwargs))?;
            if xf.is_instance_of::<crate::file_json::FileMatrix>() {
                group.call_method1("_load_file_xform", (xf, self.built))?;
            } else {
                let values = PyList::empty(py);
                for v in xf.try_iter()? {
                    values.append(py.get_type::<PyFloat>().call1((v?,))?)?;
                }
                if !self.built && values.len() != 16 {
                    // Older IGZ matrices use just their first sixteen values.
                    let matrix = matrix(py, values.as_any())?;
                    group.setattr("xform", matrix)?;
                } else if self.built && values.len() != 16 {
                    return Err(pyo3::exceptions::PyValueError::new_err(
                        "Invalid Built placement transform",
                    ));
                } else {
                    group.call_method1("_load_file_xform", (values, self.built))?;
                }
            }
            group
        } else {
            let group = self.group.call((), Some(&kwargs))?;
            let mesh = self.read_mesh.call1((
                group.getattr("mesh")?,
                raw,
                ("group", group.getattr("uid")?),
            ))?;
            group.setattr("mesh", mesh)?;
            group
        };
        if let Some(v) = raw
            .get_item("axes")?
            .and_then(|v| v.cast_into::<PyList>().ok())
        {
            if v.len() == 16 {
                group.setattr("axes", matrix(py, v.as_any())?)?;
            }
        }
        if let Some(v) = raw.get_item("ifc")? {
            if v.is_truthy()? {
                group.setattr("ifc", py.get_type::<PyDict>().call1((v,))?)?;
            }
        }
        group.call_method1("_load_file_fields", (raw,))?;
        if let Some(v) = raw
            .get_item("explode_offset")?
            .and_then(|v| v.cast_into::<PyList>().ok())
        {
            if v.len() == 3 {
                let items = v
                    .iter()
                    .map(|n| py.get_type::<PyFloat>().call1((n,)))
                    .collect::<PyResult<Vec<_>>>()?;
                group.setattr("explode_offset", PyTuple::new(py, items)?)?;
            }
        }
        let mut classic_child = false;
        if let Some(children) = raw.get_item("children")? {
            if children.is_truthy()? {
                let parts = PyList::empty(py);
                for child in children.try_iter()? {
                    let child = child?;
                    if let Ok(child) = child.cast::<PyDict>() {
                        classic_child |= !child.contains("xform")?;
                    }
                    if depth < 32 {
                        parts.append(self.group(child.cast::<PyDict>()?, depth + 1)?)?;
                    }
                }
                if !parts.is_empty() {
                    group.call_method1("adopt", (parts,))?;
                }
            }
        }
        if let Some(v) = raw.get_item("component")? {
            group.setattr("component", v.is_truthy()?)?;
        } else if classic_child {
            group.setattr("component", false)?;
        }
        Ok(group)
    }
}

fn matrix<'py>(py: Python<'py>, values: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let mut row_major = Vec::with_capacity(16);
    for r in 0..4 {
        for c in 0..4 {
            row_major.push(
                py.get_type::<PyFloat>()
                    .call1((values.get_item(c * 4 + r)?,))?,
            );
        }
    }
    py.import("ingetrazo_core")?
        .getattr("Matrix4")?
        .call1(PyTuple::new(py, row_major)?)
}

#[pymethods]
impl FileGroups {
    #[new]
    fn new(count: usize) -> Self {
        Self {
            done: 0.into(),
            count,
        }
    }
    fn fraction(&self) -> f64 {
        self.done.load(Ordering::Relaxed) as f64 / self.count.max(1) as f64
    }
    fn build<'py>(
        &self,
        py: Python<'py>,
        raw: &Bound<'py, PyAny>,
        kept: &Bound<'py, PyAny>,
        meshes: Bound<'py, PyAny>,
        read_mesh: Bound<'py, PyAny>,
        built: bool,
    ) -> PyResult<(Bound<'py, PyList>, Bound<'py, PyList>)> {
        let builder = Builder {
            group: py.import("core.group")?.getattr("Group")?,
            meshes,
            read_mesh,
            built,
        };
        let roots = PyList::empty(py);
        let library = PyList::empty(py);
        for (table, is_kept) in [(raw, false), (kept, true)] {
            for record in table.try_iter()? {
                let record = record?;
                if !is_kept || record.is_instance_of::<PyDict>() {
                    let g = builder.group(record.cast::<PyDict>()?, 0)?;
                    if !is_kept {
                        roots.append(g)?;
                    } else if g.call_method0("is_component")?.is_truthy()? {
                        library.append(g)?;
                    }
                }
                self.done.fetch_add(1, Ordering::Relaxed);
            }
        }
        Ok((roots, library))
    }
}
