//! Component placements share a definition inside one document and reset instance overrides.
use super::model_edit::translated;
use pyo3::{prelude::*, types::PyDict};
use std::path::PathBuf;

fn notice(
    win: &Bound<'_, PyAny>,
    title: &str,
    description: &str,
    field: &str,
    value: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = win.py();
    let vars = PyDict::new(py);
    vars.set_item(field, value)?;
    py.import("views.chrome_dialogs")?.call_method1(
        "warning",
        (
            win,
            translated(py, title, None)?,
            translated(py, description, Some(&vars))?,
        ),
    )?;
    Ok(())
}

fn group_from_obj<'py>(
    py: Python<'py>,
    scene: &Bound<'py, PyAny>,
    name: &str,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    let groups = scene.getattr("groups")?;
    let opts = PyDict::new(py);
    opts.set_item("cos_threshold", 0.55)?;
    let soften = py.import("formats.fuse")?.getattr("soften_smooth_edges")?;
    let mut mesh = scene.getattr("mesh")?;
    if groups.is_truthy()? {
        let first = groups.get_item(0)?;
        let children = first.getattr("children")?;
        if children.is_truthy()? {
            for child in children.try_iter()? {
                soften.call((child?.getattr("mesh")?,), Some(&opts))?;
            }
            if !name.is_empty() {
                first.setattr("name", name)?;
            }
            return Ok(Some(first));
        }
        if !mesh.getattr("faces")?.is_truthy()? {
            mesh = first.getattr("mesh")?;
        }
    }
    if !mesh.getattr("faces")?.is_truthy()? {
        return Ok(None);
    }
    soften.call((&mesh,), Some(&opts))?;
    new_group(py, &mesh, name).map(Some)
}

fn new_group<'py>(
    py: Python<'py>,
    mesh: &Bound<'py, PyAny>,
    name: &str,
) -> PyResult<Bound<'py, PyAny>> {
    let kw = PyDict::new(py);
    kw.set_item("name", name)?;
    py.import("core.group")?
        .getattr("Group")?
        .call((mesh,), Some(&kw))
}

#[pyfunction]
#[pyo3(signature=(win,key,name=None))]
fn bundled_component<'py>(
    win: &Bound<'py, PyAny>,
    key: &str,
    name: Option<String>,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    let py = win.py();
    let base = py
        .import("core.paths")?
        .call_method0("app_root")?
        .extract::<PathBuf>()?
        .join("resources")
        .join("components");
    let scene = py.import("core.scene")?.call_method0("Scene")?;
    let default_name = py
        .import("builtins")?
        .call_method1("str", (key,))?
        .call_method0("capitalize")?
        .extract::<String>()?;
    let fallback = translated(py, &default_name, None)?.extract::<String>()?;
    let name = name.filter(|n| !n.is_empty());
    for kind in ["igz", "glb", "obj"] {
        let file = base.join(format!("{key}.{kind}"));
        if !file.is_file() {
            continue;
        }
        let path = py.import("pathlib")?.call_method1("Path", (file,))?;
        let (module, load) = match kind {
            "igz" => ("formats.igz", "load_into"),
            "glb" => ("formats.glb", "load_glb"),
            _ => ("formats.obj", "load_obj"),
        };
        py.import(module)?.call_method1(load, (&scene, &path))?;
        if kind == "obj" {
            return group_from_obj(py, &scene, name.as_deref().unwrap_or(&fallback));
        }
        let groups = scene.getattr("groups")?;
        if !groups.is_truthy()? {
            notice(
                win,
                "Insert component",
                "Component file missing: {p}",
                "p",
                &path,
            )?;
            return Ok(None);
        }
        let first = groups.get_item(0)?;
        let source_name = first
            .getattr("name")?
            .extract::<Option<String>>()?
            .filter(|s| !s.is_empty());
        let chosen = name
            .as_deref()
            .or(if kind == "glb" {
                source_name.as_deref()
            } else {
                None
            })
            .unwrap_or(&fallback);
        return new_group(py, &first.getattr("mesh")?, chosen).map(Some);
    }
    let path = py
        .import("pathlib")?
        .call_method1("Path", (base.join(format!("{key}.glb")),))?;
    notice(
        win,
        "Insert component",
        "Component file missing: {p}",
        "p",
        &path,
    )?;
    Ok(None)
}

#[pyfunction]
#[pyo3(signature=(win,entry,obj=None))]
fn catalogue_component<'py>(
    win: &Bound<'py, PyAny>,
    entry: &Bound<'py, PyDict>,
    obj: Option<&Bound<'py, PyAny>>,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    let py = win.py();
    let library = py.import("core.library")?;
    let path = match obj {
        Some(x) => x.clone(),
        None => library.call_method1("model_file", (entry,))?,
    };
    let name = entry.call_method1("get", ("nombre", ""))?;
    if path.is_none() {
        notice(
            win,
            "Component library",
            "Could not download “{name}”. Check your connection.",
            "name",
            &name,
        )?;
        return Ok(None);
    }
    let scene = py.import("core.scene")?.call_method0("Scene")?;
    let read = || -> PyResult<()> {
        let kw = PyDict::new(py);
        kw.set_item(
            "matrix",
            library.call_method1("model_matrix", (entry, &path))?,
        )?;
        py.import("formats.obj")?
            .getattr("load_obj")?
            .call((&scene, &path), Some(&kw))?;
        Ok(())
    };
    if let Err(err) = read() {
        py.import("views.chrome_dialogs")?.call_method1(
            "critical",
            (
                win,
                translated(py, "Component library", None)?,
                err.value(py).str()?,
            ),
        )?;
        return Ok(None);
    }
    let Some(group) = group_from_obj(py, &scene, &name.extract::<String>()?)? else {
        notice(
            win,
            "Component library",
            "“{name}” has no geometry.",
            "name",
            &name,
        )?;
        return Ok(None);
    };
    let ext = PyDict::new(py);
    let previous = group.getattr("ext")?;
    if !previous.is_none() {
        ext.call_method1("update", (previous,))?;
    }
    ext.set_item(
        "catalogue",
        py.import("ingetrazo_render")?
            .getattr("ComponentCatalogue")?
            .call_method1("attribution", (entry,))?,
    )?;
    group.setattr("ext", ext)?;
    Ok(Some(group))
}

fn place_shared(
    win: &Bound<'_, PyAny>,
    key: String,
    build: impl FnOnce() -> PyResult<Py<PyAny>>,
) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    vp.call_method0("end_group_edit")?;
    let scene = vp.getattr("scene")?;
    let cache = win
        .getattr("_doc")?
        .getattr("__dict__")?
        .call_method1("setdefault", ("_placement_definitions", PyDict::new(py)))?
        .cast_into::<PyDict>()?;
    let matrix = py.import("ingetrazo_core")?.getattr("Matrix4")?;
    if let Some(known) = cache.get_item(&key)? {
        let mesh = known.get_item(0)?;
        for g in scene.getattr("groups")?.try_iter()? {
            let g = g?;
            if !g.getattr("mesh")?.is(&mesh) || !g.call_method0("is_component")?.is_truthy()? {
                continue;
            }
            let group = py.import("core.group")?.call_method1("copy_group", (g,))?;
            group.setattr("xform", matrix.call1((known.get_item(1)?,))?)?;
            for flag in ["hidden", "locked"] {
                group.setattr(flag, false)?;
            }
            for attr in [
                "material",
                "layer",
                "ifc",
                "exploded",
                "explode_offset",
                "axes",
            ] {
                group.setattr(attr, py.None())?;
            }
            win.call_method1("_start_place", (group,))?;
            return Ok(());
        }
        cache.del_item(&key)?;
    }
    let group = build()?.into_bound(py);
    if group.is_none() || !win.getattr("viewport")?.getattr("scene")?.is(&scene) {
        return Ok(());
    }
    if group.getattr("xform")?.is_none() {
        group.setattr("xform", matrix.call0()?)?;
    }
    group.setattr("component", true)?;
    cache.set_item(
        key,
        (
            group.getattr("mesh")?,
            matrix.call1((group.getattr("xform")?,))?,
        ),
    )?;
    win.call_method1("_start_place", (group,))?;
    Ok(())
}

#[pyfunction]
#[pyo3(signature=(win,key,name=None))]
fn place_bundled(win: &Bound<'_, PyAny>, key: &str, name: Option<&str>) -> PyResult<()> {
    place_shared(win, format!("bundled:{key}"), || {
        Ok(win
            .call_method1("_build_bundled_component", (key, name))?
            .unbind())
    })
}
#[pyfunction]
#[pyo3(signature=(win,entry,obj=None))]
fn place_catalogue(
    win: &Bound<'_, PyAny>,
    entry: &Bound<'_, PyDict>,
    obj: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    let key = entry.call_method1("get", ("id", ""))?.str()?.to_string();
    place_shared(win, format!("library:{key}"), || {
        Ok(win
            .call_method1("_build_library_component", (entry, obj))?
            .unbind())
    })
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(bundled_component, m)?)?;
    m.add_function(wrap_pyfunction!(catalogue_component, m)?)?;
    m.add_function(wrap_pyfunction!(place_bundled, m)?)?;
    m.add_function(wrap_pyfunction!(place_catalogue, m)?)?;
    Ok(())
}
