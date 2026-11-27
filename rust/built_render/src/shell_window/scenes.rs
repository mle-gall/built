//! Saved-scene commands shared by the scene menu and browser.
use super::*;

#[pyclass(module = "built_render", dict)]
pub struct ShellScenes;

fn touch(w: &Obj<'_>) -> PyResult<()> {
    let vp = w.call_method0("_active_view")?;
    vp.call_method0("notify_scene_changed")?;
    vp.call_method0("update")?;
    let display = w.getattr("_doc")?.getattr("stand_in")?;
    if !display.is_none() {
        display.setattr("_ui_tabs", w.py().None())?;
    }
    Ok(())
}
fn edit<'p>(w: &Obj<'p>, op: &str, kw: &Bound<'p, PyDict>) -> PyResult<Obj<'p>> {
    let history = w.call_method0("_active_view")?.getattr("history")?;
    w.py().import("built_core.command_protocol")?.call_method(
        "edit_saved_scene",
        (history, op),
        Some(kw),
    )
}
fn with_view<'p>(view: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let kw = PyDict::new(view.py());
    kw.set_item("views", PyList::new(view.py(), [view])?)?;
    Ok(kw)
}
fn mark_result<'p>(w: &Obj<'p>, result: &Obj<'p>) -> PyResult<Obj<'p>> {
    if !result.get_item("ok")?.is_truthy()? {
        return Ok(none(w.py()));
    }
    let view = w
        .call_method0("_active_view")?
        .getattr("scene")?
        .getattr("saved_views")?
        .get_item(result.get_item("index")?)?;
    w.getattr("_scene_tabs")?
        .call_method1("mark_view", (&view,))?;
    touch(w)?;
    let text = crate::settings_rows::trf(w.py(), "Scene '{name}' added", &[("name", view.getattr("name")?)])?;
    w.call_method1("notify", ("success", text))?;
    Ok(view)
}
#[pymethods]
impl ShellScenes {
    #[new]
    fn new(py: Python<'_>, win: &Obj<'_>) -> PyResult<Py<Self>> {
        let scenes = Bound::new(py, Self)?;
        scenes.setattr("_win", win)?;
        Ok(scenes.unbind())
    }
    #[pyo3(signature=(view,animate=true))]
    fn show_view(slf: &Bound<'_, Self>, view: &Obj<'_>, animate: bool) -> PyResult<()> {
        let win = slf.getattr("_win")?;
        let player = win.getattr("scene_player")?;
        player.call_method0("cancel")?;
        if view.is_none() {
            return Ok(());
        }
        let secs = if animate {
            let transition: f64 = win.call_method0("_active_view")?
                .getattr("scene")?
                .getattr("animation")?
                .call_method0("glide_seconds")?
                .extract()?;
            // Scene Transitions off: the quarter-second camera move.
            if transition > 0.0 { transition } else { crate::native_view_actions::camera_motion(slf.py())? }
        } else {
            0.0
        };
        if secs > 0.0 && !win.getattr("view")?.getattr("renderer")?.is_none() {
            let kw = PyDict::new(slf.py());
            kw.set_item("camera", false)?;
            let arrive = slf.py().import("functools")?.call_method(
                "partial",
                (slf.getattr("finish_view")?,),
                Some(&kw),
            )?;
            if player
                .call_method1("glide_to", (view, secs, arrive))?
                .is_truthy()?
            {
                win.getattr("_scene_tabs")?
                    .call_method1("mark_view", (view,))?;
                return Ok(());
            }
        }
        Self::finish_view(slf, view, true)
    }
    #[pyo3(signature=(view,camera=true))]
    fn finish_view(slf: &Bound<'_, Self>, view: &Obj<'_>, camera: bool) -> PyResult<()> {
        if view.is_none() {
            return Ok(());
        }
        let win = slf.getattr("_win")?;
        let vp = win.call_method0("_active_view")?;
        let scene = vp.getattr("scene")?;
        let core = slf.py().import("built_core")?;
        let state = core.getattr("saved_view_shown_state")?;
        let before = state.call1((&scene,))?;
        let style = win.getattr("set_active_style")?;
        if camera {
            view.call_method1("apply", (&scene, vp.getattr("camera")?, style))?;
        } else {
            view.call_method1("apply_state", (&scene, style))?;
        }
        core.getattr("presentation")?
            .call_method1("recall_scene_display", (&vp, view))?;
        win.getattr("_scene_tabs")?
            .call_method1("mark_view", (view,))?;
        if state.call1((&scene,))?.ne(before)? {
            let version: i64 = scene.getattr("version")?.extract()?;
            scene.setattr("version", version + 1)?;
            touch(&win)?;
        } else {
            vp.call_method0("update")?;
            let display = win.getattr("_doc")?.getattr("stand_in")?;
            if !display.is_none() {
                display.setattr("_ui_tabs", slf.py().None())?;
                display.call_method0("update")?;
            }
        }
        Ok(())
    }
    fn add_scene<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        let mark = slf
            .getattr("_win")?
            .getattr("_scene_tabs")?
            .call_method0("marked_view")?;
        Self::add_scene_in_folder(slf, &optional(&mark, "folder")?)
    }
    fn add_scene_in_folder<'p>(slf: &Bound<'p, Self>, folder: &Obj<'p>) -> PyResult<Obj<'p>> {
        let py = slf.py();
        let win = slf.getattr("_win")?;
        let kw = PyDict::new(py);
        kw.set_item(
            "name",
            py.import("built_core.i18n")?.call_method1("tr", ("Scene",))?,
        )?;
        kw.set_item(
            "camera",
            win.call_method0("_active_view")?.getattr("camera")?,
        )?;
        kw.set_item("style", win.call_method0("active_style")?)?;
        kw.set_item(
            "folder",
            if folder.is_truthy()? {
                folder.clone()
            } else {
                none(py)
            },
        )?;
        mark_result(&win, &edit(&win, "add", &kw)?)
    }
    fn duplicate_view<'p>(slf: &Bound<'p, Self>, view: &Obj<'p>) -> PyResult<Obj<'p>> {
        let win = slf.getattr("_win")?;
        mark_result(&win, &edit(&win, "duplicate", &with_view(view)?)?)
    }
    fn update_view(slf: &Bound<'_, Self>, view: &Obj<'_>) -> PyResult<()> {
        let win = slf.getattr("_win")?;
        let kw = with_view(view)?;
        kw.set_item(
            "camera",
            win.call_method0("_active_view")?.getattr("camera")?,
        )?;
        kw.set_item("style", win.call_method0("active_style")?)?;
        if edit(&win, "update", &kw)?.get_item("ok")?.is_truthy()? {
            touch(&win)?;
        }
        Ok(())
    }
    fn rename_view(slf: &Bound<'_, Self>, view: &Obj<'_>, name: Option<&str>) -> PyResult<bool> {
        let win = slf.getattr("_win")?;
        let kw = with_view(view)?;
        let name = name.unwrap_or("");
        kw.set_item("name", name)?;
        let result = edit(&win, "rename", &kw)?;
        let ok = result.get_item("ok")?.is_truthy()?;
        if ok {
            touch(&win)?;
        } else if result.get_item("rejected")?.is_truthy()? {
            win.call_method0("statusBar")?.call_method1(
                "showMessage",
                (format!("Another scene is already called «{name}»"), 3500),
            )?;
        }
        Ok(ok)
    }
    fn delete_views(slf: &Bound<'_, Self>, views: &Obj<'_>) -> PyResult<()> {
        let win = slf.getattr("_win")?;
        let kw = PyDict::new(slf.py());
        kw.set_item("views", views)?;
        if edit(&win, "delete", &kw)?.get_item("ok")?.is_truthy()? {
            let tabs = win.getattr("_scene_tabs")?;
            if views.contains(tabs.call_method0("marked_view")?)? {
                tabs.call_method1("mark_view", (slf.py().None(),))?;
            }
            touch(&win)?;
        }
        Ok(())
    }
    fn move_view(slf: &Bound<'_, Self>, view: &Obj<'_>, index: i64) -> PyResult<()> {
        let win = slf.getattr("_win")?;
        let kw = with_view(view)?;
        kw.set_item("index", index)?;
        if edit(&win, "move", &kw)?.get_item("ok")?.is_truthy()? {
            touch(&win)?;
        }
        Ok(())
    }
}
