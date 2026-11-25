// SPDX-License-Identifier: GPL-3.0-or-later
//! Journalled extension values and record patches retain their public Python
//! constructors and copy values at the command boundary.
use super::steps::{version, Obj};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};

fn json_copy<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let json = v.py().import("json")?;
    json.call_method1("loads", (json.call_method1("dumps", (v,))?,))
}
fn plugin_apply(step: &Obj<'_>, scene: &Obj<'_>, undo: bool) -> PyResult<()> {
    let py = scene.py();
    let key = step.getattr("key")?;
    let data = scene.getattr("plugin_data")?;
    let value = if undo {
        if step.getattr("_had")?.is_truthy()? {
            Some(step.getattr("_before")?)
        } else {
            None
        }
    } else {
        let had = data.contains(&key)?;
        step.setattr("_had", had)?;
        step.setattr(
            "_before",
            if had {
                Some(json_copy(&data.get_item(&key)?)?)
            } else {
                None
            },
        )?;
        Some(step.getattr("value")?).filter(|v| !v.is_none())
    };
    if let Some(value) = value {
        data.set_item(key, json_copy(&value)?)?;
    } else {
        data.call_method1("pop", (key, py.None()))?;
    }
    version(scene)
}
#[pyclass(subclass, module = "ingetrazo_core")]
struct PluginDataStep;
#[pymethods]
impl PluginDataStep {
    #[new]
    #[pyo3(signature=(*_args,**_kw))]
    fn new(_args: &Bound<'_, PyTuple>, _kw: Option<&Bound<'_, PyDict>>) -> Self {
        Self
    }
    fn r#do(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<()> {
        plugin_apply(slf.as_any(), scene, false)
    }
    fn undo(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<()> {
        plugin_apply(slf.as_any(), scene, true)
    }
}
#[pyfunction]
pub(crate) fn registry_apply(step: &Obj<'_>, scene: &Obj<'_>, undo: bool) -> PyResult<()> {
    let py = scene.py();
    let project = py.import("ingetrazo_core.project")?;
    let value = step.getattr("value")?;
    let shared = if undo {
        let kw = PyDict::new(py);
        kw.set_item("base", &value)?;
        project.call_method(
            "write_registry",
            (scene, step.getattr("_project_before")?),
            Some(&kw),
        )?
    } else {
        let stamp = scene.getattr_opt("_project_mtime_ns")?;
        let base = scene.getattr_opt("_project_registry_base")?;
        let current = py
            .import("core.bim_registry")?
            .call_method1("get", (scene,))?;
        let keys = |v: &Obj<'_>| -> PyResult<Py<PyAny>> {
            Ok(v.call_method1("get", ("structure", PyDict::new(py)))?
                .call_method1("get", ("nodes", PyDict::new(py)))?
                .call_method0("keys")?
                .unbind())
        };
        let removed = keys(&current)?.bind(py).sub(keys(&value)?.bind(py))?;
        if removed.is_truthy()? {
            let structure = py.import("core.structure")?;
            let refiled = step.getattr("refiled_uids")?;
            for row in structure
                .call_method1("_placements", (scene,))?
                .try_iter()?
            {
                let row = row?;
                if row.get_item(1)?.is_truthy()? {
                    continue;
                }
                let g = row.get_item(0)?;
                let node = structure.call_method1("node_of", (&g,))?;
                if removed.contains(&node)? && !refiled.contains(g.getattr("uid")?)? {
                    return Err(PyErr::from_value(py.import("ingetrazo_core.history")?.call_method1(
                        "CommandRejected",
                        (format!("Cannot delete occupied node: {}", node.str()?),),
                    )?));
                }
            }
        }
        if !step.hasattr("_project_before")? {
            step.setattr("_project_before", current)?;
        }
        if let Some(v) = stamp.filter(|v| !v.is_none()) {
            scene.setattr("_project_mtime_ns", v)?;
        }
        if let Some(v) = base.filter(|v| !v.is_none()) {
            scene.setattr("_project_registry_base", v)?;
        }
        project.call_method1("write_registry", (scene, value))?
    };
    if shared.is_truthy()? {
        version(scene)?;
    } else {
        plugin_apply(step, scene, undo)?;
    }
    scene.getattr("mesh")?.call_method0("bump_revision")?;
    Ok(())
}

fn bump_records(step: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let mut seen = std::collections::HashSet::new();
    let mesh = scene.getattr("mesh")?;
    seen.insert(mesh.as_ptr() as usize);
    mesh.call_method0("bump_revision")?;
    for g in step.getattr("targets")?.try_iter()? {
        let m = g?.getattr("mesh")?;
        if seen.insert(m.as_ptr() as usize) {
            m.call_method0("bump_revision")?;
        }
    }
    version(scene)
}
fn write_records(step: &Obj<'_>, scene: &Obj<'_>, values: &Obj<'_>) -> PyResult<()> {
    let copy = scene.py().import("copy")?.getattr("deepcopy")?;
    for (g, v) in step.getattr("targets")?.try_iter()?.zip(values.try_iter()?) {
        g?.setattr("ifc", copy.call1((v?,))?)?;
    }
    bump_records(step, scene)
}
#[pyclass(subclass, module = "ingetrazo_core")]
struct RecordPatchStep;
#[pymethods]
impl RecordPatchStep {
    #[new]
    #[pyo3(signature=(*_args,**_kw))]
    fn new(_args: &Bound<'_, PyTuple>, _kw: Option<&Bound<'_, PyDict>>) -> Self {
        Self
    }
    fn doc_after(slf: &Bound<'_, Self>, scene: &Obj<'_>, phase: &str) -> PyResult<()> {
        let _ = phase;
        bump_records(slf.as_any(), scene)
    }
    fn r#do(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<()> {
        let py = scene.py();
        let s = slf.as_any();
        if s.getattr("_before")?.is_none() {
            let copy = py.import("copy")?.getattr("deepcopy")?;
            let before = PyList::empty(py);
            for g in s.getattr("targets")?.try_iter()? {
                before.append(copy.call1((g?.getattr("ifc")?,))?)?;
            }
            s.setattr("_before", &before)?;
            let after = PyList::empty(py);
            s.setattr("_after", &after)?;
            let patch = s.getattr("patch")?;
            let normalize = py.import("core.record")?.getattr("normalize")?;
            let ensure = py.import("core.ifc_identity")?.getattr("ensure_guid")?;
            for old in before.iter() {
                if patch.is_none() {
                    after.append(py.None())?;
                    continue;
                }
                let new = PyDict::new(py);
                if old.is_truthy()? {
                    new.call_method1("update", (copy.call1((&old,))?,))?;
                }
                new.call_method1("update", (copy.call1((&patch,))?,))?;
                let guid = if old.is_truthy()? {
                    Some(old.call_method1("get", ("guid",))?)
                } else {
                    None
                };
                if let Some(guid) = guid.filter(|g| g.is_truthy().unwrap_or(false)) {
                    new.set_item("guid", guid)?;
                } else {
                    new.call_method1("pop", ("guid", py.None()))?;
                    ensure.call1((&new,))?;
                }
                if s.getattr("normalize_values")?.is_truthy()? {
                    after.append(normalize.call1((new,))?)?;
                } else {
                    after.append(new)?;
                }
            }
        }
        write_records(s, scene, &s.getattr("_after")?)
    }
    fn undo(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<()> {
        write_records(slf.as_any(), scene, &slf.getattr("_before")?)
    }
}
fn face_tags(step: &Obj<'_>, scene: &Obj<'_>, undo: bool) -> PyResult<()> {
    let py = scene.py();
    let copy = py.import("copy")?.getattr("deepcopy")?;
    let bim = py.import("core.bim")?;
    let faces = step.getattr("faces")?;
    if !undo && step.getattr("_before")?.is_none() {
        let before = PyList::empty(py);
        for face in faces.try_iter()? {
            before
                .append(copy.call1((face?.getattr("attrs")?.call_method1("get", ("ifc",))?,))?)?;
        }
        step.setattr("_before", before)?;
        let ifc = step.getattr("ifc_class")?;
        let tag = if ifc.is_truthy()? {
            let tag = PyDict::new(py);
            tag.set_item("id", bim.call_method1("next_object_id", (scene,))?)?;
            tag.set_item("class", ifc)?;
            tag.set_item("name", step.getattr("name")?)?;
            py.import("core.ifc_identity")?
                .call_method1("ensure_guid", (&tag,))?;
            tag.into_any()
        } else {
            py.None().into_bound(py)
        };
        let after = PyList::empty(py);
        for _ in faces.try_iter()? {
            after.append(copy.call1((&tag,))?)?;
        }
        step.setattr("_after", after)?;
    }
    for (f, v) in faces.try_iter()?.zip(
        step.getattr(if undo { "_before" } else { "_after" })?
            .try_iter()?,
    ) {
        let attrs = f?.getattr("attrs")?;
        let v = v?;
        if v.is_none() {
            attrs.call_method1("pop", ("ifc", py.None()))?;
        } else {
            attrs.set_item("ifc", copy.call1((v,))?)?;
        }
    }
    bim.setattr("_TAG_EPOCH", bim.getattr("_TAG_EPOCH")?.add(1)?)?;
    scene.getattr("mesh")?.call_method0("bump_revision")?;
    version(scene)
}
#[pyclass(subclass, module = "ingetrazo_core")]
struct FaceTagsStep;
#[pymethods]
impl FaceTagsStep {
    #[new]
    #[pyo3(signature=(*_args,**_kw))]
    fn new(_args: &Bound<'_, PyTuple>, _kw: Option<&Bound<'_, PyDict>>) -> Self {
        Self
    }
    fn r#do(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<()> {
        face_tags(slf.as_any(), scene, false)
    }
    fn undo(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<()> {
        face_tags(slf.as_any(), scene, true)
    }
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<RecordPatchStep>()?;
    m.add_class::<FaceTagsStep>()?;
    m.add_class::<PluginDataStep>()?;
    m.add_function(wrap_pyfunction!(registry_apply, m)?)?;
    Ok(())
}
