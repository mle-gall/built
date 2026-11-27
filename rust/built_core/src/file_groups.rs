// SPDX-License-Identifier: GPL-3.0-or-later
//! Build document placements from their file records. The host supplies
//! mesh handles (including shared linked definitions) and legacy identities.
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyFloat, PyInt, PyIterator, PyList, PyTuple};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[pyclass(frozen)]
pub struct FileGroups {
    cancelled: AtomicBool,
    done: AtomicUsize,
    count: usize,
}

struct Builder<'py> {
    cancelled: &'py AtomicBool,
    group: Bound<'py, PyAny>,
    meshes: Bound<'py, PyAny>,
    read_mesh: Bound<'py, PyAny>,
    built: bool,
}
impl<'py> Builder<'py> {
    fn record(&self, raw: &Bound<'py, PyDict>) -> PyResult<Bound<'py, PyAny>> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(pyo3::exceptions::PyInterruptedError::new_err("Opening cancelled"));
        }
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
        Ok(group)
    }

    fn group(&self, raw: &Bound<'py, PyDict>) -> PyResult<Bound<'py, PyAny>> {
        let root = self.record(raw)?;
        // Create records in file order (legacy names are allocated on creation)
        // and adopt only after each child's full hierarchy has been restored.
        enum Task<'py> {
            Visit(Bound<'py, PyDict>, Bound<'py, PyAny>),
            Child(Bound<'py, PyDict>, Bound<'py, PyList>),
            Finish(
                Bound<'py, PyDict>,
                Bound<'py, PyAny>,
                Bound<'py, PyList>,
                bool,
            ),
        }
        let mut stack = vec![Task::Visit(raw.clone(), root.clone())];
        while let Some(task) = stack.pop() {
            match task {
                Task::Child(raw, parts) => {
                    let group = self.record(&raw)?;
                    parts.append(&group)?;
                    stack.push(Task::Visit(raw, group));
                }
                Task::Visit(raw, group) => {
                    let mut children = Vec::new();
                    let mut classic = false;
                    if let Some(rows) = raw.get_item("children")?.filter(|v| !v.is_none()) {
                        for row in rows.try_iter()? {
                            let row = row?.cast_into::<PyDict>()?;
                            classic |= !row.contains("xform")?;
                            children.push(row);
                        }
                    }
                    let parts = PyList::empty(raw.py());
                    stack.push(Task::Finish(raw, group, parts.clone(), classic));
                    stack.extend(
                        children
                            .into_iter()
                            .rev()
                            .map(|row| Task::Child(row, parts.clone())),
                    );
                }
                Task::Finish(raw, group, parts, classic) => {
                    if !parts.is_empty() {
                        group.call_method1("adopt", (parts,))?;
                    }
                    if let Some(v) = raw.get_item("component")? {
                        group.setattr("component", v.is_truthy()?)?;
                    } else if classic {
                        group.setattr("component", false)?;
                    }
                }
            }
        }
        Ok(root)
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
    py.import("built_core")?
        .getattr("Matrix4")?
        .call1(PyTuple::new(py, row_major)?)
}

#[pymethods]
impl FileGroups {
    #[new]
    fn new(count: usize) -> Self {
        Self {
            done: 0.into(),
            cancelled: false.into(),
            count,
        }
    }
    #[staticmethod]
    fn records<'py>(payload: &Bound<'py, PyDict>) -> PyResult<Bound<'py, PyList>> {
        let records = PyList::empty(payload.py());
        let mut pending = Vec::new();
        for key in ["kept_components", "groups"] {
            if let Some(rows) = payload.get_item(key)?.filter(|rows| !rows.is_none()) {
                pending.extend(
                    rows.try_iter()?
                        .collect::<PyResult<Vec<_>>>()?
                        .into_iter()
                        .rev(),
                );
            }
        }
        while let Some(item) = pending.pop() {
            if let Ok(row) = item.cast::<PyDict>() {
                records.append(row)?;
                if let Some(rows) = row.get_item("children")?.filter(|rows| !rows.is_none()) {
                    pending.extend(
                        rows.try_iter()?
                            .collect::<PyResult<Vec<_>>>()?
                            .into_iter()
                            .rev(),
                    );
                }
            }
        }
        Ok(records)
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    fn fraction(&self) -> f64 {
        self.done.load(Ordering::Relaxed) as f64 / self.count.max(1) as f64
    }
    fn build<'py>(
        &'py self,
        py: Python<'py>,
        raw: &Bound<'py, PyAny>,
        kept: &Bound<'py, PyAny>,
        meshes: Bound<'py, PyAny>,
        read_mesh: Bound<'py, PyAny>,
        built: bool,
    ) -> PyResult<(Bound<'py, PyList>, Bound<'py, PyList>)> {
        let builder = Builder {
            cancelled: &self.cancelled,
            group: py.import("built_core.group")?.getattr("Group")?,
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
                    let g = builder.group(record.cast::<PyDict>()?)?;
                    if !is_kept {
                        roots.append(g)?;
                    } else if g.call_method0("is_component")?.is_truthy()? || g.getattr("ext")?.contains("built.cut_source").unwrap_or(false) {
                        library.append(g)?;
                    }
                }
                self.done.fetch_add(1, Ordering::Relaxed);
            }
        }
        Ok((roots, library))
    }
}

/// A document-wide prototype table maintained across incremental saves.
/// Snapshot generators can still yield between root placements.
#[pyclass]
pub struct FileGroupWriter {
    scene: Py<PyAny>,
    block: Py<PyAny>,
    ifc: Py<PyAny>,
    empty: Py<PyAny>,
    protos: Py<PyList>,
    indices: std::collections::HashMap<usize, usize>,
    precise: bool,
    destination: Option<Py<PyAny>>,
}

#[pymethods]
impl FileGroupWriter {
    #[new]
    #[pyo3(signature=(scene, block, precise, destination=None))]
    fn new(
        py: Python<'_>,
        scene: Py<PyAny>,
        block: Py<PyAny>,
        precise: bool,
        destination: Option<Py<PyAny>>,
    ) -> PyResult<Self> {
        let igz = py.import("built_core.formats.igz")?;
        Ok(Self {
            scene,
            block,
            precise,
            destination,
            indices: Default::default(),
            protos: PyList::empty(py).unbind(),
            ifc: igz.getattr("_group_ifc_record")?.unbind(),
            empty: igz.getattr("_empty_mesh")?.unbind(),
        })
    }
    #[getter]
    fn protos<'py>(&self, py: Python<'py>) -> Bound<'py, PyList> {
        self.protos.bind(py).clone()
    }

    fn entry<'py>(
        &mut self,
        py: Python<'py>,
        group: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let (entry, branch) = self.record(py, group)?;
        let mut pending = Vec::from_iter(branch);
        let mut ancestors = std::collections::HashSet::from([group.as_ptr() as usize]);
        while let Some((id, output, children)) = pending.last_mut() {
            let Some(child) = children.next() else {
                ancestors.remove(id);
                pending.pop();
                continue;
            };
            let child = child?;
            let id = child.as_ptr() as usize;
            if ancestors.contains(&id) {
                return Err(pyo3::exceptions::PyRecursionError::new_err(
                    "cyclic placement hierarchy",
                ));
            }
            let (row, branch) = self.record(py, &child)?;
            output.append(row)?;
            if let Some(branch) = branch {
                ancestors.insert(id);
                pending.push(branch);
            }
        }
        Ok(entry)
    }
}

impl FileGroupWriter {
    fn record<'py>(
        &mut self,
        py: Python<'py>,
        group: &Bound<'py, PyAny>,
    ) -> PyResult<(
        Bound<'py, PyDict>,
        Option<(usize, Bound<'py, PyList>, Bound<'py, PyIterator>)>,
    )> {
        let matrix = group.call_method1("_file_xform", (self.precise,))?;
        let placeholder = !matrix.is_none()
            && crate::model::links::save_as_placeholder(group, self.scene.bind(py))?;
        let mesh = if placeholder {
            self.empty.bind(py).call0()?
        } else {
            group.getattr("mesh")?
        };
        let entry = if matrix.is_none() {
            self.block.bind(py).call1((mesh,))?.cast_into::<PyDict>()?
        } else {
            let id = mesh.as_ptr() as usize;
            let index = match self.indices.get(&id) {
                Some(&i) => i,
                None => {
                    let protos = self.protos.bind(py);
                    let i = protos.len();
                    protos.append(self.block.bind(py).call1((mesh,))?)?;
                    self.indices.insert(id, i);
                    i
                }
            };
            let row = PyDict::new(py);
            row.set_item("proto", index)?;
            row.set_item("xform", matrix)?;
            row.set_item("component", group.getattr("component")?.is_truthy()?)?;
            row
        };
        let ifc = self.ifc.bind(py).call1((group,))?;
        let ifc = if ifc.is_truthy()? { Some(ifc) } else { None };
        entry.call_method1("update", (group.call_method1("_file_fields", (ifc,))?,))?;
        if let Some(dest) = &self.destination {
            if crate::model::links::link_of(group)?.is_some() {
                let ext = entry
                    .get_item("ext")?
                    .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("ext"))?;
                let record = ext.get_item(crate::model::links::LINK_KEY)?;
                record.call_method1("pop", ("rel", py.None()))?;
                let fields = crate::model::link_io::link_record(&record.get_item("path")?, Some(dest.bind(py)))?;
                record.call_method1("update", (fields,))?;
            }
        }
        if !placeholder {
            let children = group.getattr("children")?;
            if children.is_truthy()? {
                let rows = PyList::empty(py);
                entry.set_item("children", &rows)?;
                return Ok((
                    entry,
                    Some((group.as_ptr() as usize, rows, children.try_iter()?)),
                ));
            }
        }
        Ok((entry, None))
    }
}
