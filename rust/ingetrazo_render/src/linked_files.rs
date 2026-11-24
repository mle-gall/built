//! Place and refresh external document links in the receiving viewport.
use super::{
    model_edit::{busy, translated},
    window_document::transient_tool_class,
};
use pyo3::{prelude::*, types::PyDict};
use std::path::PathBuf;

fn named<'py>(
    win: &Bound<'py, PyAny>,
    text: &str,
    key: &str,
    path: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let vars = PyDict::new(win.py());
    vars.set_item(key, path.getattr("name")?)?;
    translated(win.py(), text, Some(&vars))
}
#[pyfunction]
#[pyo3(signature=(win,path,at=None,select=None,include_nested=true))]
fn link_document<'py>(
    win: &Bound<'py, PyAny>,
    path: PathBuf,
    at: Option<&Bound<'py, PyAny>>,
    select: Option<&Bound<'py, PyAny>>,
    include_nested: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let py = win.py();
    let p = py.import("pathlib")?.call_method1("Path", (path,))?;
    let vp = win.getattr("viewport")?;
    vp.call_method0("end_group_edit")?;
    let kw = PyDict::new(py);
    kw.set_item("host_path", win.getattr("_current_path")?)?;
    kw.set_item("select", select)?;
    kw.set_item("include_nested", include_nested)?;
    let title = named(win, "Linking {name}…", "name", &p)?;
    let comp = busy(win, &title, || {
        py.import("core.links")?
            .getattr("make_link")?
            .call((vp.getattr("scene")?, &p), Some(&kw))
    })?;
    win.call_method0("statusBar")?.call_method1(
        "showMessage",
        (named(win, "Click to place {name}", "name", &p)?, 5000),
    )?;
    let vec = py.import("ingetrazo_core")?.getattr("Vec3")?;
    let kw = PyDict::new(py);
    kw.set_item("anchor", vec.call0()?)?;
    if let Some(at) = at {
        transient_tool_class(py, "place_group")?
            .call((&comp,), Some(&kw))?
            .call_method1("place_at", (&vp, vec.call1((at,))?))?;
    } else {
        win.call_method("_start_place", (&comp,), Some(&kw))?;
    }
    win.call_method0("_link_watcher")?.call_method0("refresh")?;
    Ok(comp)
}

#[pyfunction]
fn link_changed(win: &Bound<'_, PyAny>, note: &str) -> PyResult<()> {
    let vp = win.getattr("viewport")?;
    vp.call_method0("notify_scene_changed")?;
    vp.call_method0("update")?;
    win.call_method0("_link_watcher")?.call_method0("refresh")?;
    if !note.is_empty() {
        win.call_method0("statusBar")?
            .call_method1("showMessage", (note, 4000))?;
    }
    Ok(())
}

fn warn(win: &Bound<'_, PyAny>, detail: &Bound<'_, pyo3::types::PyString>) -> PyResult<()> {
    let py = win.py();
    py.import("views.chrome_dialogs")?.call_method1(
        "warning",
        (win, translated(py, "Linked files", None)?, detail),
    )?;
    Ok(())
}

#[pyfunction]
#[pyo3(signature=(win,path=None))]
fn reload_links(win: &Bound<'_, PyAny>, path: Option<PathBuf>) -> PyResult<()> {
    let py = win.py();
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let current = win.getattr("_current_path")?;
    let backend = py.import("core.links")?;
    if let Some(path) = path {
        let p = py.import("pathlib")?.call_method1("Path", (path,))?;
        let label = named(win, "Reloading {name}…", "name", &p)?;
        match busy(win, &label, || {
            backend.call_method1("reload", (&scene, &p, &current))
        }) {
            Ok(_) => win.call_method1(
                "_relink_done",
                (named(win, "Reloaded {names}", "names", &p)?,),
            )?,
            Err(e) if e.value(py).is_instance(&backend.getattr("LinkError")?)? => {
                return warn(win, &e.value(py).str()?)
            }
            Err(e) => return Err(e),
        };
    } else {
        let caption = translated(py, "Reloading linked documents…", None)?;
        let errors = busy(win, &caption, || {
            backend.call_method1("resolve", (scene, current))
        })?;
        win.call_method1(
            "_relink_done",
            (translated(py, "Linked documents reloaded", None)?,),
        )?;
        if errors.is_truthy()? {
            let details: Vec<String> = errors
                .call_method0("values")?
                .try_iter()?
                .map(|v| v?.extract())
                .collect::<PyResult<_>>()?;
            warn(win, &pyo3::types::PyString::new(py, &details.join("\n")))?;
        }
    }
    Ok(())
}
#[pyfunction]
fn set_link_nested(
    win: &Bound<'_, PyAny>,
    group: &Bound<'_, PyAny>,
    enabled: bool,
) -> PyResult<bool> {
    let py = win.py();
    let backend = py.import("core.links")?;
    if let Err(e) = backend.call_method1(
        "set_include_nested",
        (win.getattr("viewport")?.getattr("scene")?, group, enabled),
    ) {
        if !e.is_instance_of::<pyo3::exceptions::PyOSError>(py)
            && !e.value(py).is_instance(&backend.getattr("LinkError")?)?
        {
            return Err(e);
        }
        warn(win, &e.value(py).str()?)?;
        return Ok(false);
    }
    win.call_method1("_relink_done", ("",))?;
    Ok(true)
}
#[pyfunction]
fn reset_link_tags(win: &Bound<'_, PyAny>, path: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let backend = py.import("core.links")?;
    if let Err(e) = backend.call_method1(
        "clear_overrides",
        (
            win.getattr("viewport")?.getattr("scene")?,
            path,
            win.getattr("_current_path")?,
        ),
    ) {
        if !e.value(py).is_instance(&backend.getattr("LinkError")?)? {
            return Err(e);
        }
        return warn(win, &e.value(py).str()?);
    }
    win.call_method1("_relink_done", ("",))?;
    Ok(())
}
#[pyfunction]
fn pack_link(win: &Bound<'_, PyAny>, path: PathBuf, on: bool) -> PyResult<()> {
    let py = win.py();
    let scene = win.getattr("viewport")?.getattr("scene")?;
    py.import("core.links")?
        .call_method1("set_packed", (&scene, &path, on))?;
    scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
    if !on && path.is_file() {
        return reload_links(win, Some(path));
    }
    let p = py.import("pathlib")?.call_method1("Path", (path,))?;
    let note = if on {
        named(win, "{name} will be saved inside this document", "name", &p)?.extract()?
    } else {
        String::new()
    };
    win.call_method1("_relink_done", (note,))?;
    Ok(())
}

#[pyfunction]
#[pyo3(signature=(win,path,select=None))]
fn choose_link_contents(
    win: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
    select: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    let py = win.py();
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let p = py.import("pathlib")?.call_method1("Path", (path,))?;
    let options = [
        translated(py, "Include nested linked files", None)?,
        translated(py, "Source geometry only", None)?,
    ];
    let kw = PyDict::new(py);
    kw.set_item(
        "done",
        py.import("functools")?.call_method1(
            "partial",
            (
                wrap_pyfunction!(link_contents_answer, py)?,
                win,
                scene,
                path,
                select,
                &options,
            ),
        )?,
    )?;
    py.import("ingetrazo_render.chrome_forms")?
        .getattr("ask_choice")?
        .call(
            (
                win,
                translated(py, "Link IngeTrazo document", None)?,
                named(win, "Contents of {name}", "name", &p)?,
                options,
            ),
            Some(&kw),
        )?;
    Ok(())
}
#[pyfunction]
fn link_contents_answer(
    win: &Bound<'_, PyAny>,
    scene: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
    select: &Bound<'_, PyAny>,
    options: [String; 2],
    answer: Option<String>,
) -> PyResult<()> {
    if !win.getattr("viewport")?.getattr("scene")?.is(scene) {
        return Ok(());
    }
    let Some(index) = options
        .iter()
        .position(|item| Some(item) == answer.as_ref())
    else {
        return Ok(());
    };
    let kw = PyDict::new(win.py());
    kw.set_item("select", select)?;
    kw.set_item("include_nested", index == 0)?;
    win.call_method("link_igz_path", (path,), Some(&kw))?;
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(choose_link_contents, m)?)?;
    m.add_function(wrap_pyfunction!(link_document, m)?)?;
    m.add_function(wrap_pyfunction!(link_changed, m)?)?;
    m.add_function(wrap_pyfunction!(reload_links, m)?)?;
    m.add_function(wrap_pyfunction!(set_link_nested, m)?)?;
    m.add_function(wrap_pyfunction!(reset_link_tags, m)?)?;
    m.add_function(wrap_pyfunction!(pack_link, m)?)?;
    Ok(())
}
