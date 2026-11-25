// SPDX-License-Identifier: GPL-3.0-or-later
//! Document pointers, registry snapshots and portable links to a shared project.
use super::{
    format::{project_create, project_load},
    manifest, project_convert, project_relative, project_sync_host, project_write_registry,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyString},
};
use std::path::{Path, PathBuf};
type Obj<'p> = Bound<'p, PyAny>;
fn plugin<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    scene.getattr("plugin_data")
}
fn copy<'p>(v: &Obj<'p>) -> PyResult<Obj<'p>> {
    v.py().import("copy")?.call_method1("deepcopy", (v,))
}
fn path_obj<'p>(py: Python<'p>, p: &Path) -> PyResult<Obj<'p>> {
    py.import("pathlib")?.call_method1("Path", (p,))
}
fn dumps(v: &Obj) -> PyResult<String> {
    v.py()
        .import("json")?
        .call_method1("dumps", (v,))?
        .extract()
}
fn manifest_for(scene: &Obj, host: Option<PathBuf>) -> PyResult<Option<PathBuf>> {
    let r = plugin(scene)?.call_method1("get", ("project",))?;
    let Ok(d) = r.cast::<PyDict>() else {
        return Ok(None);
    };
    let string = |key: &str| -> PyResult<Option<String>> {
        Ok(d.get_item(key)?
            .filter(|v| v.is_instance_of::<PyString>())
            .and_then(|v| v.extract().ok()))
    };
    let host = match host {
        Some(h) => Some(h),
        None => scene
            .getattr_opt("_project_host_path")?
            .and_then(|p| p.extract().ok()),
    };
    let relative = string("rel")?;
    let absolute = string("path")?;
    Ok(
        manifest::resolve_pointer(host.as_deref(), relative.as_deref(), absolute.as_deref())
            .or_else(|| absolute.filter(|p| !p.is_empty()).map(PathBuf::from)),
    )
}
#[pyfunction(signature=(scene,host_path=None))]
fn path_for<'p>(scene: &Obj<'p>, host_path: Option<PathBuf>) -> PyResult<Option<Obj<'p>>> {
    manifest_for(scene, host_path)?
        .map(|p| path_obj(scene.py(), &p))
        .transpose()
}
fn mtime(scene: &Obj, path: &Path) -> PyResult<()> {
    scene.setattr(
        "_project_mtime_ns",
        path_obj(scene.py(), path)?
            .call_method0("stat")?
            .getattr("st_mtime_ns")?,
    )
}
fn observe(scene: &Obj, path: &Path, value: &Obj) -> PyResult<()> {
    mtime(scene, path)?;
    scene.setattr(
        "_project_registry_base",
        copy(&value.get_item("bim_registry")?)?,
    )
}
#[pyfunction]
fn read<'p>(scene: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    let Some(p) = manifest_for(scene, None)?.filter(|p| p.is_file()) else {
        return Ok(None);
    };
    let p = project_convert(p)?.0;
    let value = project_load(scene.py(), p.clone())?;
    observe(scene, &p, &value)?;
    Ok(Some(value))
}
#[pyfunction(signature=(scene,path,host_path=None))]
fn attach<'p>(scene: &Obj<'p>, path: PathBuf, host_path: Option<PathBuf>) -> PyResult<Obj<'p>> {
    let py = scene.py();
    let p = project_convert(path)?.0.canonicalize()?;
    let value = project_load(py, p.clone())?;
    let d = PyDict::new(py);
    d.set_item("path", p.to_string_lossy().as_ref())?;
    let data = plugin(scene)?;
    data.set_item("project", d)?;
    refresh_reference(scene, host_path)?;
    observe(scene, &p, &value)?;
    data.set_item("ifc_identity", copy(&value.get_item("ifc_identity")?)?)?;
    scene.setattr("version", scene.getattr("version")?.add(1)?)?;
    Ok(value)
}
fn resolve(py: Python, p: &Path) -> PyResult<PathBuf> {
    path_obj(py, p)?.call_method0("resolve")?.extract()
}
#[pyfunction]
fn refresh_reference(scene: &Obj, host_path: Option<PathBuf>) -> PyResult<()> {
    let Some(host) = host_path else {
        return Ok(());
    };
    let Some(mut p) = manifest_for(scene, None)? else {
        return Ok(());
    };
    if p.is_file() {
        p = project_convert(p)?.0;
    }
    let py = scene.py();
    let host = resolve(py, &host)?;
    let d = PyDict::new(py);
    d.set_item("path", p.to_string_lossy().as_ref())?;
    d.set_item(
        "rel",
        project_relative(p, host.parent().unwrap_or(&host).into()),
    )?;
    plugin(scene)?.set_item("project", d)?;
    scene.setattr("_project_host_path", host.to_string_lossy().as_ref())
}
#[pyfunction(signature=(scene,path,host_path=None))]
fn create<'p>(scene: &Obj<'p>, mut path: PathBuf, host_path: Option<PathBuf>) -> PyResult<Obj<'p>> {
    let py = scene.py();
    if !path
        .extension()
        .and_then(|v| v.to_str())
        .is_some_and(|v| v.eq_ignore_ascii_case("builtproject"))
    {
        path.set_extension("builtproject");
    }
    let identity = copy(&plugin(scene)?.call_method1("get", ("ifc_identity", PyDict::new(py)))?)?;
    let namespace = identity.call_method1(
        "setdefault",
        (
            "namespace",
            py.import("uuid")?.call_method0("uuid4")?.getattr("hex")?,
        ),
    )?;
    let guid = py.import("core.ifc_identity")?.getattr("stable_guid")?;
    for key in ["project", "site", "building", "storey"] {
        identity.call_method1("setdefault", (key, guid.call1((&namespace, key))?))?;
    }
    let value = project_create(
        py,
        path.clone(),
        &py.import("core.bim_registry")?
            .call_method1("get", (scene,))?,
        &identity,
    )?;
    attach(scene, path, host_path.clone())?;
    sync_files(scene, host_path)?;
    Ok(value)
}
#[pyfunction(signature=(scene,registry,base=None))]
fn write_registry(scene: &Obj, registry: &Obj, base: Option<&Obj>) -> PyResult<bool> {
    let Some(p) = manifest_for(scene, None)? else {
        return Ok(false);
    };
    let p = project_convert(p)?.0;
    let py = scene.py();
    let base = match base
        .cloned()
        .or(scene.getattr_opt("_project_registry_base")?)
        .filter(|v| !v.is_none())
    {
        Some(b) => b,
        None => project_load(py, p.clone())?.get_item("bim_registry")?,
    };
    let conflicts = project_write_registry(p.clone(), &dumps(&base)?, &dumps(registry)?)?;
    if !conflicts.is_empty() {
        let msg = format!(
            "Project entries also changed elsewhere; yours kept: {} ({})",
            conflicts.join(", "),
            p.display()
        );
        scene.setattr("project_warning", &msg)?;
        py.import("warnings")?.call_method1(
            "warn",
            (msg, py.get_type::<pyo3::exceptions::PyRuntimeWarning>(), 2),
        )?;
    }
    let v = project_load(py, p.clone())?;
    observe(scene, &p, &v)?;
    plugin(scene)?.set_item("bim_registry", copy(&v.get_item("bim_registry")?)?)?;
    Ok(true)
}
#[pyfunction]
fn sync_files(scene: &Obj, host_path: Option<PathBuf>) -> PyResult<()> {
    let Some(host) = host_path else {
        return Ok(());
    };
    let Some(p) = manifest_for(scene, None)?.filter(|p| p.is_file()) else {
        return Ok(());
    };
    let p = project_convert(p)?.0;
    let py = scene.py();
    let relative = |s: &Path| -> PyResult<String> {
        Ok(project_relative(
            resolve(py, s)?,
            p.parent().unwrap_or(&p).into(),
        ))
    };
    let host = relative(&host)?;
    let file = |path: &str, role: &str| -> PyResult<_> {
        let d = PyDict::new(py);
        d.set_item("path", path)?;
        d.set_item("role", role)?;
        Ok(d)
    };
    let mut files = vec![file(&host, "host")?];
    let mut edges = vec![];
    let links = py.import("core.links")?;
    for g in links.call_method1("linked_groups", (scene,))?.try_iter()? {
        let g = g?;
        let r = links.call_method1("link_of", (&g,))?;
        let source = relative(&r.get_item("path")?.extract::<PathBuf>()?)?;
        files.push(file(&source, "linked")?);
        let e = PyDict::new(py);
        e.set_item("host", &host)?;
        e.set_item("source", source)?;
        e.set_item("placement_uid", g.getattr("uid")?)?;
        e.set_item("select", r.call_method1("get", ("select",))?)?;
        if !r
            .call_method1("get", ("include_nested", true))?
            .is_truthy()?
        {
            e.set_item("include_nested", false)?;
        }
        edges.push(e);
    }
    project_sync_host(
        p.clone(),
        &host,
        &dumps(&files.into_pyobject(py)?.into_any())?,
        &dumps(&edges.into_pyobject(py)?.into_any())?,
    )?;
    mtime(scene, &p)
}
#[pyfunction]
fn save_worksession<'p>(
    py: Python<'p>,
    path: PathBuf,
    files: &Obj,
    active_file: Option<PathBuf>,
) -> PyResult<Obj<'p>> {
    let s = py
        .get_type::<super::session::ProjectSession>()
        .call1((path,))?;
    s.call_method1("set_editable", (active_file,))?;
    let mut rows = vec![];
    for f in files.try_iter()? {
        let f = f?;
        let c = f.call_method1("get", ("camera",))?;
        rows.push((
            f.get_item("path")?,
            if c.is_none() { None } else { Some(dumps(&c)?) },
            f.call_method1("get", ("scene",))?,
        ));
    }
    let kw = PyDict::new(py);
    kw.set_item("keep_planned", false)?;
    s.call_method("record", (rows,), Some(&kw))?;
    py.import("json")?
        .call_method1("loads", (s.call_method0("query")?,))
}
#[pyfunction]
fn worksession(py: Python<'_>, path: PathBuf) -> PyResult<Bound<'_, PyDict>> {
    let s = py
        .get_type::<super::session::ProjectSession>()
        .call1((path,))?;
    let mut rows = vec![];
    let json = py.import("json")?;
    for r in s.call_method0("plan")?.try_iter()? {
        let r = r?;
        let d = PyDict::new(py);
        d.set_item("path", r.get_item(0)?)?;
        d.set_item("role", r.get_item(1)?)?;
        let c = r.get_item(2)?;
        d.set_item(
            "camera",
            if c.is_truthy()? {
                Some(json.call_method1("loads", (c,))?)
            } else {
                None
            },
        )?;
        d.set_item("scene", r.get_item(3)?)?;
        rows.push(d);
    }
    let out = PyDict::new(py);
    out.set_item("project", s.getattr("path")?.str()?)?;
    let editable = s.getattr("editable")?;
    out.set_item(
        "active_file",
        if editable.is_truthy()? {
            Some(editable)
        } else {
            rows.first()
                .map(|r| r.as_any().get_item("path"))
                .transpose()?
        },
    )?;
    out.set_item("files", rows)?;
    Ok(out)
}
pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    let name = "ingetrazo_core.project";
    let sub = PyModule::new(m.py(), name)?;
    for f in [
        wrap_pyfunction!(path_for, &sub)?,
        wrap_pyfunction!(read, &sub)?,
        wrap_pyfunction!(attach, &sub)?,
        wrap_pyfunction!(refresh_reference, &sub)?,
        wrap_pyfunction!(create, &sub)?,
        wrap_pyfunction!(write_registry, &sub)?,
        wrap_pyfunction!(sync_files, &sub)?,
        wrap_pyfunction!(save_worksession, &sub)?,
        wrap_pyfunction!(worksession, &sub)?,
    ] {
        sub.add_function(f)?;
    }
    sub.add(
        "session",
        m.py().get_type::<super::session::ProjectSession>(),
    )?;
    m.py()
        .import("sys")?
        .getattr("modules")?
        .set_item(name, &sub)?;
    m.add("project", sub)
}
