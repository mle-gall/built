// SPDX-License-Identifier: GPL-3.0-or-later
//! Document-owned delivery records and a scoped scene for the selected writer.
mod package;

mod write;
use super::Obj;
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList, PySet},
};
use std::collections::HashMap;
#[pyfunction]
fn records<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let raw = scene
        .getattr("plugin_data")?
        .call_method1("get", ("deliveries", PyDict::new(scene.py())))?;
    let records = raw.cast::<PyDict>()?.copy()?;
    for (key, value) in raw.cast::<PyDict>()?.iter() {
        if !value.contains("outputs")? {
            let record = value.cast::<PyDict>()?.copy()?;
            migrate(&record)?;
            records.set_item(key, record)?;
        }
    }
    Ok(records.into_any())
}

fn migrate(record: &Bound<'_, PyDict>) -> PyResult<()> {
    if !record.contains("scope")? {
        record.set_item("scope", "whole")?;
    }
    if !record.contains("options")? {
        record.set_item("options", PyDict::new(record.py()))?;
    }
    if record.get_item("format")?.is_none() && record.contains("outputs")? {
        record.set_item("format", "package")?;
    }
    if !record
        .get_item("format")?
        .is_some_and(|v| v.eq("package").unwrap_or(false))
    {
        let out = PyDict::new(record.py());
        for field in ["format", "scope", "options", "node", "destination"] {
            if let Some(v) = record.get_item(field)? {
                out.set_item(field, v)?;
            }
        }
        record.set_item("outputs", PyList::new(record.py(), [out])?)?;
    }
    Ok(())
}
#[pyfunction(name="validate")]
fn delivery_validate(record: &Obj<'_>) -> PyResult<()> {
    let py = record.py();
    let kw = PyDict::new(py);
    kw.set_item("default", py.get_type::<pyo3::types::PyString>())?;
    let json = py
        .import("json")?
        .call_method("dumps", (record,), Some(&kw))?
        .extract::<String>()?;
    let err = crate::delivery_form::delivery_error(&json)?;
    if !err.is_empty() {
        return Err(PyValueError::new_err(err));
    }
    Ok(())
}
fn store<'p>(history: &Obj<'p>, key: &Obj<'p>, values: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = history.py();
    let before = history.getattr("revision")?;
    let cmd = py
        .import("built_core.history")?
        .call_method1("SetPluginDataCommand", ("deliveries", values))?;
    history.call_method1("execute", (cmd,))?;
    let out = py.get_type::<PyDict>().call1((py
        .import("built_core.command_protocol")?
        .call_method1("answer", (history, before))?,))?;
    out.set_item("key", key)?;
    Ok(out)
}
fn entries<'p>(h: &Obj<'p>) -> PyResult<Obj<'p>> {
    h.py()
        .get_type::<PyDict>()
        .call1((records(&h.getattr("scene")?)?,))
}
#[pyfunction]
fn create_delivery<'p>(history: &Obj<'p>, delivery: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = history.py();
    let key = py.import("uuid")?.call_method0("uuid4")?.getattr("hex")?;
    let record = PyDict::new(py);
    record.set_item("scope", "whole")?;
    record.set_item("options", PyDict::new(py))?;
    record.call_method1("update", (delivery,))?;
    record.set_item("last_run", py.None())?;
    migrate(&record)?;
    delivery_validate(record.as_any())?;
    let vals = entries(history)?;
    vals.set_item(&key, record)?;
    store(history, &key, &vals)
}
#[pyfunction]
fn edit_delivery<'p>(history: &Obj<'p>, key: &Obj<'p>, changes: &Obj<'p>) -> PyResult<Obj<'p>> {
    let vals = entries(history)?;
    let record = history
        .py()
        .get_type::<PyDict>()
        .call1((vals.get_item(key)?,))?;
    record.call_method1("update", (changes,))?;
    if changes.contains("outputs")? && !changes.contains("format")? {
        record.set_item("format", "package")?;
    }
    migrate(record.cast::<PyDict>()?)?;
    delivery_validate(&record)?;
    vals.set_item(key, record)?;
    store(history, key, &vals)
}
#[pyfunction]
fn delete_delivery<'p>(history: &Obj<'p>, key: &Obj<'p>) -> PyResult<Obj<'p>> {
    let vals = entries(history)?;
    vals.del_item(key)?;
    store(history, key, &vals)
}
#[pyfunction(name="finish", signature=(history,key,folder="",result="Delivered",details=None))]
fn delivery_finish<'p>(
    history: &Obj<'p>,
    key: &Obj<'p>,
    folder: &str,
    result: &str,
    details: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let py = history.py();
    let date = py.import("datetime")?;
    let now = date
        .getattr("datetime")?
        .call_method1("now", (date.getattr("timezone")?.getattr("utc")?,))?;
    let kw = PyDict::new(py);
    kw.set_item("timespec", "seconds")?;
    let changes = PyDict::new(py);
    let time = now.call_method("isoformat", (), Some(&kw))?;
    let record = records(&history.getattr("scene")?)?.get_item(key)?;
    let runs = py.get_type::<PyList>().call1((record.call_method1("get", ("runs", PyList::empty(py)))?,))?;
    let run = PyDict::new(py);
    run.set_item("time", &time)?;
    run.set_item("folder", folder)?;
    run.set_item("result", result)?;
    run.set_item("revision", history.getattr("revision")?)?;
    if let Some(details) = details {
        run.call_method1("update", (details,))?;
    }
    runs.call_method1("append", (run,))?;
    changes.set_item("runs", runs)?;
    if result == "Delivered" { changes.set_item("last_run", time)?; }
    edit_delivery(history, key, changes.as_any())
}
#[pyfunction(signature=(history,key,**context))]
fn run_delivery<'p>(
    history: &Obj<'p>,
    key: &Obj<'p>,
    context: Option<&Bound<'p, PyDict>>,
) -> PyResult<Obj<'p>> {
    let py = history.py();
    let s = history.getattr("scene")?;
    let record = records(&s)?.get_item(key)?;
    if let Some(runner) = history
        .getattr_opt("delivery_runner")?
        .filter(|r| !r.is_none() && context.is_none_or(|d| d.is_empty()))
    {
        let d = PyDict::new(py);
        d.set_item("ok", true)?;
        for k in ["step", "error", "rejected"] {
            d.set_item(k, py.None())?
        }
        d.set_item("job", runner.call1((key, &record))?)?;
        return Ok(d.into_any());
    }
    let directory: Option<std::path::PathBuf> = match context {
        Some(ctx) => ctx.get_item("project_directory")?.map(|p| p.extract()).transpose()?.flatten(),
        None => None,
    };
    let (target, _) = write::destination_path(py, &s, &record, directory.as_deref())?;
    let kwargs = match context {
        Some(c) => c.copy()?,
        None => PyDict::new(py),
    };
    if record.get_item("format")?.eq("package")? {
        kwargs.set_item("revision", history.getattr("revision")?)?;
    }
    let steps = py.import("built_core.deliveries")?.call_method(
        "write_steps",
        (&s, record),
        Some(&kwargs),
    )?;
    for step in steps.try_iter()? {
        step?;
    }
    if let Some(details) = steps.getattr_opt("summary")?.filter(|s| !s.is_none()) {
        return delivery_finish(
            history,
            key,
            &details.get_item("folder")?.extract::<String>()?,
            &details.get_item("result")?.extract::<String>()?,
            Some(&details),
        );
    }
    let folder = target.parent().unwrap_or(&target).canonicalize()?;
    delivery_finish(history, key, &folder.to_string_lossy(), "Delivered", None)
}
fn mesh_part<'p>(mesh: &Obj<'p>, keep: Option<&Obj<'p>>,step:bool) -> PyResult<Obj<'p>> {
    if mesh.is_none() {return mesh.py().import("built_core.mesh")?.call_method0("Mesh")}
    let Some(keep) = keep else {
        return Ok(mesh.clone());
    };
    if step && mesh.getattr("_store")?.getattr("has_exact_bindings")?.is_truthy()? {
        return crate::step::scope::subset(mesh,keep);
    }
    let out = mesh.py().import("built_core.mesh")?.call_method0("Mesh")?;
    for f in mesh.getattr("faces")?.try_iter()? {
        let f = f?;
        if keep.call1((&f,))?.is_truthy()? {
            out.call_method1("add_face", (f.getattr("vertices")?, f.getattr("holes")?))?
                .getattr("attrs")?
                .call_method1("update", (f.getattr("attrs")?,))?;
        }
    }
    for e in mesh.getattr("edges")?.try_iter()? {
        let e = e?;
        if keep.call1((&e,))?.is_truthy()? {
            let edge = out.call_method1(
                "add_edge",
                (
                    e.getattr("v0")?.getattr("position")?,
                    e.getattr("v1")?.getattr("position")?,
                ),
            )?;
            for k in ["soft", "flat", "layer", "hidden", "curve"] {
                edge.setattr(k, e.getattr(k)?)?
            }
        }
    }
    Ok(out)
}
struct Scope<'p> {
    step:bool,
    scene: Obj<'p>,
    wanted: Option<Obj<'p>>,
    visible: Option<Obj<'p>>,
    member: Option<Obj<'p>>,
    selection: bool,
    meshes: HashMap<usize, Obj<'p>>,
}
impl<'p> Scope<'p> {
    fn branch(&mut self, g: &Obj<'p>, inside: bool) -> PyResult<Option<Obj<'p>>> {
        let py = g.py();
        if let Some(visible) = &self.visible {
            if !visible.call1((g,))?.is_truthy()? {
                return Ok(None);
            }
        }
        let inside = inside
            || match &self.wanted {
                Some(w) => w.contains(g)?,
                None => true,
            };
        let kids = PyList::empty(py);
        let children = g.getattr("children")?;
        if !children.is_none() {
            for child in children.try_iter()? {
                let child = child?;
                let inherit = inside
                    && match &self.member {
                        Some(t) => !t.contains(child.as_ptr() as usize)?,
                        None => true,
                    };
                if let Some(item) = self.branch(&child, inherit)? {
                    kids.append(item)?
                }
            }
        }
        let editing = self.selection && g.is(&self.scene.getattr("edit_group")?);
        if !inside && kids.is_empty() && !editing {
            return Ok(None);
        }
        let item = crate::model::links::clone_shell_of(g)?;
        let mesh = g.getattr("mesh")?;
        if editing {
            let test = self.wanted.as_ref().unwrap().getattr("__contains__")?;
            item.setattr("mesh", mesh_part(&mesh, Some(&test),self.step)?)?
        } else if !inside {
            item.setattr("mesh", py.import("built_core.mesh")?.call_method0("Mesh")?)?
        } else if let Some(visible) = &self.visible {
            let part = match self.meshes.entry(mesh.as_ptr() as usize) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(mesh_part(&mesh, Some(visible),self.step)?)
                }
            };
            item.setattr("mesh", &*part)?
        }
        item.setattr("children", kids)?;
        if !inside {
            item.setattr("ifc", py.None())?
        }
        Ok(Some(item))
    }
}
#[pyfunction]
fn delivery_visible(_entity: &Obj<'_>) -> bool {
    true
}
#[pyfunction(name="scoped")]
fn delivery_scoped<'p>(py: Python<'p>, scene: &Obj<'p>, record: &Obj<'p>) -> PyResult<Obj<'p>> {
    let scope = record.get_item("scope")?.extract::<String>()?;
    let mut ctx = Scope {
        step:record.call_method1("get",("format",""))?.eq("step")?,
        scene: scene.clone(),
        wanted: None,
        visible: None,
        member: None,
        selection: scope == "selection",
        meshes: HashMap::new(),
    };
    match scope.as_str() {
        "node" => {
            let tree = py.import("built_core.link_structure")?.call_method1(
                "Tree",
                (
                    scene,
                    py.import("core.bim_registry")?
                        .call_method1("get", (scene,))?,
                ),
            )?;
            let node = record.get_item("node")?;
            if !tree.getattr("nodes")?.contains(&node)? {
                return Err(PyValueError::new_err(
                    "The delivery's project node is missing",
                ));
            }
            ctx.wanted = Some(
                py.get_type::<PySet>()
                    .call1((tree.call_method1("members", (node,))?,))?,
            );
            ctx.member = Some(tree.getattr("member")?)
        }
        "selection" => {
            let w = py
                .get_type::<PySet>()
                .call1((scene.getattr("selection")?,))?;
            if !w.is_truthy()? {
                return Err(PyValueError::new_err(
                    "Select geometry before running this delivery",
                ));
            }
            ctx.wanted = Some(w)
        }
        "visible" => ctx.visible = Some(scene.getattr("entity_visible")?),
        _ => (),
    }
    let copy = py.import("copy")?;
    let out = copy.call_method1("copy", (scene,))?;
    let tables = PyDict::new(py);
    for field in py
        .import("built_core.scene")?
        .getattr("ROOT_TABLES")?
        .try_iter()?
    {
        let k = field?.extract::<String>()?;
        tables.set_item(
            &k,
            copy.call_method1(
                if k == "plugin_data" {
                    "deepcopy"
                } else {
                    "copy"
                },
                (scene.getattr(&k)?,),
            )?,
        )?
    }
    let materials = scene.getattr("materials")?.call_method0("copy")?;
    let kept = py
        .get_type::<PyList>()
        .call1((scene.getattr("kept_components")?,))?;
    out.setattr(
        "_root",
        py.import("built_core.doc_model")?
            .call_method0("new_scene_root")?,
    )?;
    for (k, v) in tables.iter() {
        out.setattr(k.cast::<pyo3::types::PyString>()?, v)?
    }
    out.setattr("materials", materials)?;
    if kept.is_truthy()? {
        let memo = PyDict::new(py);
        for g in py
            .import("built_core")?
            .call_method1("purge_iter_groups", (&kept,))?
            .try_iter()?
        {
            let m = g?.getattr("mesh")?;
            memo.set_item(m.as_ptr() as usize, m)?
        }
        out.setattr(
            "kept_components",
            copy.call_method1("deepcopy", (kept, memo))?,
        )?
    }
    if !record.contains("format")? || record.get_item("format")?.eq("ifc")? {
        out.setattr(
            "_repeated_source_uids",
            scene.call_method0("repeated_uids")?,
        )?
    }
    out.setattr("edit_group", py.None())?;
    out.getattr("__dict__")?
        .call_method1("pop", ("_groups_owner", py.None()))?;
    let layers = PyList::empty(py);
    for l in scene.getattr("layers")?.try_iter()? {
        layers.append(copy.call_method1("copy", (l?,))?)?
    }
    out.setattr("layers", layers)?;
    let groups = PyList::empty(py);
    for g in scene.getattr("groups")?.try_iter()? {
        if let Some(g) = ctx.branch(&g?, false)? {
            groups.append(g)?
        }
    }
    out.setattr("groups", groups)?;
    let pred = ctx
        .wanted
        .as_ref()
        .map(|v| v.getattr("__contains__"))
        .transpose()?
        .or(ctx.visible);
    out.setattr(
        "mesh",
        mesh_part(&scene.getattr("loose_mesh")?, pred.as_ref(),ctx.step)?,
    )?;
    if scope != "scenes" {
        out.setattr(
            "entity_visible",
            wrap_pyfunction!(delivery_visible,py)?,
        )?
    }
    Ok(out)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py=m.py();
    let sub=&PyModule::new(py,"built_core.deliveries")?;
    functions!(sub;records,delivery_validate,create_delivery,edit_delivery,delete_delivery,delivery_finish,run_delivery,delivery_scoped);
    write::register(sub)?;
    sub.add("PACKAGE_OUTPUTS", crate::delivery_form::PACKAGE_OUTPUTS.to_vec())?;
    sub.add("FORMATS",pyo3::types::PyTuple::new(py,crate::delivery_form::delivery_formats())?)?;
    py.import("sys")?.getattr("modules")?.set_item("built_core.deliveries",sub)?;
    m.add("deliveries",sub)
}
