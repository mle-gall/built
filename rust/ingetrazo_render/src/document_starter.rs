//! Blank documents use the same units, camera and scale figure in both hosts.
use pyo3::{prelude::*, types::PyDict};
use std::path::PathBuf;

#[pyfunction]
fn reset_home(vp: &Bound<'_, PyAny>) -> PyResult<()> {
    let fresh = vp
        .py()
        .import("core.camera")?
        .getattr("OrbitCamera")?
        .call0()?;
    let cam = vp.getattr("camera")?;
    for field in std::iter::once("target").chain(crate::window_document::CAMERA_FIELDS) {
        cam.setattr(field, fresh.getattr(field)?)?;
    }
    cam.call_method0("reset_lens")?;
    vp.call_method0("update")?;
    Ok(())
}

#[pyfunction]
#[pyo3(signature=(image,height,name=None,position=None))]
fn billboard_person(
    py: Python<'_>,
    image: PathBuf,
    height: f64,
    name: Option<String>,
    position: Option<&Bound<'_, PyAny>>,
) -> PyResult<Py<PyAny>> {
    let path = if image.is_absolute() {
        image
    } else {
        py.import("core.paths")?
            .call_method0("app_root")?
            .extract::<PathBuf>()?
            .join("resources/components")
            .join(image)
    };
    let Some((w, h)) = crate::image_assets::raster_dimensions(py, path.clone())? else {
        return Ok(py.None());
    };
    if w == 0 || h == 0 {
        return Ok(py.None());
    }
    let name = match name.filter(|s| !s.is_empty()) {
        Some(s) => s,
        None => py
            .import("core.i18n")?
            .call_method1("tr", ("Person",))?
            .extract()?,
    };
    let kw = PyDict::new(py);
    kw.set_item("position", position)?;
    Ok(py
        .import("core.group")?
        .getattr("make_billboard_group")?
        .call(
            (
                path.to_string_lossy(),
                height,
                name,
                f64::from(w) / f64::from(h),
            ),
            Some(&kw),
        )?
        .unbind())
}

#[pyfunction]
fn insert_scale_figure(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let figures = py.import("core.scale_figure")?;
    let figure = figures.call_method0("chosen")?;
    let scene = win.getattr("viewport")?.getattr("scene")?;
    if !figure.is_none() {
        let kw = PyDict::new(py);
        kw.set_item(
            "position",
            py.import("ingetrazo_core")?
                .getattr("Vec3")?
                .call1((-0.65, -0.6, 0.0))?,
        )?;
        let make = win.getattr("_make_billboard_person")?;
        let mut group = make.call(
            (
                figure.getattr("image")?,
                figure.getattr("height")?,
                figure.getattr("name")?,
            ),
            Some(&kw),
        )?;
        if group.is_none() {
            group = make.call((), Some(&kw))?;
        }
        if group.is_none() {
            return Ok(());
        }
        figures.call_method1("mark", (&group,))?;
        scene.getattr("groups")?.call_method1("append", (group,))?;
        scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
    }
    win.setattr("_saved_version", scene.getattr("version")?)?;
    Ok(())
}

#[pyfunction]
fn start_blank_document(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let vp = win.getattr("viewport")?;
    vp.getattr("inference")?.call_method0("reset")?;
    for command in [
        "_forget_coordinate",
        "_reset_camera_home",
        "_apply_new_document_units",
        "_insert_scale_figure",
    ] {
        win.call_method0(command)?;
    }
    vp.call_method0("notify_scene_changed")?;
    for command in ["_sync_style_menu", "_sync_section_menu", "_update_title"] {
        win.call_method0(command)?;
    }
    if win.getattr("_session_path")?.is_truthy()? {
        win.setattr("_active_file", win.py().None())?;
        win.setattr("_session_unsaved_doc", win.getattr("_doc")?)?;
        win.call_method0("_refresh_worksession")?;
    }
    Ok(())
}
/// Start another tab, or release all document-owned resources and reuse an
/// untouched tab. Recovery slots follow the drawing rather than the tab.
#[pyfunction]
fn new_document_command(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let workspace = win.getattr("_workspace")?;
    if !workspace.is_none() && workspace.hasattr("new")? {
        workspace.call_method0("new")?;
        return Ok(());
    }
    win.call_method0("_leave_empty_area")?;
    let vp = win.getattr("viewport")?;
    vp.call_method0("end_group_edit")?;
    if !win.call_method0("_doc_is_blank")?.is_truthy()? {
        win.call_method0("_new_document")?;
        win.call_method0("_start_blank_document")?;
        return Ok(());
    }

    vp.getattr("inference")?.call_method0("reset")?;
    let args = PyDict::new(py);
    args.set_item("untitled", win.getattr("_doc")?.getattr("untitled_no")?)?;
    py.import("core.autosave")?
        .getattr("clear")?
        .call((win.getattr("_current_path")?,), Some(&args))?;
    win.setattr("_autosaved_version", py.None())?;
    let scene = vp.getattr("scene")?;
    scene.call_method0("clear")?;
    scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
    vp.getattr("history")?.call_method0("clear")?;
    vp.call_method0("reset_texture_cache")?;
    win.setattr("_current_path", py.None())?;
    win.setattr("_import_name", py.None())?;
    win.setattr("_saved_version", scene.getattr("version")?)?;
    win.call_method0("_start_blank_document")?;
    win.call_method0("settle_heap")?;
    Ok(())
}

#[pyfunction]
fn settle_document_heap(py: Python<'_>, threshold: usize) -> PyResult<()> {
    let gc = py.import("gc")?;
    gc.call_method0("unfreeze")?;
    // On CPython 3.13 a zero threshold for generation 2 collects the entire
    // loaded model at every generation-1 opportunity while drawing.
    gc.call_method1("set_threshold", (threshold, 10, 10))?;
    if py
        .import("sys")?
        .call_method0("getallocatedblocks")?
        .extract::<usize>()?
        <= 40_000_000
    {
        gc.call_method0("collect")?;
    }
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(new_document_command, m)?)?;
    m.add_function(wrap_pyfunction!(settle_document_heap, m)?)?;
    m.add_function(wrap_pyfunction!(start_blank_document, m)?)?;
    m.add_function(wrap_pyfunction!(reset_home, m)?)?;
    m.add_function(wrap_pyfunction!(insert_scale_figure, m)?)?;
    m.add_function(wrap_pyfunction!(billboard_person, m)?)?;
    Ok(())
}
