//! Winit's per-window controller. CPython owns the live document and plugin objects.
use pyo3::exceptions::{PyException, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PySet, PyTuple};
use std::time::Instant;

mod diagnostics;

type Obj<'py> = Bound<'py, PyAny>;
const FILES: &str = "built_render.shell_files";
const DIALOGS: &str = "built_render.chrome_dialogs";

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
fn optional<'p>(obj: &Obj<'p>, name: &str) -> PyResult<Obj<'p>> {
    Ok(obj.getattr_opt(name)?.unwrap_or_else(|| none(obj.py())))
}
#[pyfunction(signature=(directory=None, *, temporary=false))]
pub(crate) fn start_profile<'p>(
    py: Python<'p>,
    directory: Option<Obj<'p>>,
    temporary: bool,
) -> PyResult<Obj<'p>> {
    let env = py.import("os")?.getattr("environ")?;
    let mut profile = directory.unwrap_or_else(|| none(py));
    if !profile.is_truthy()? {
        profile = env.call_method1("get", ("BUILT_SETTINGS_DIR",))?;
    }
    if !profile.is_truthy()? && temporary {
        let options = PyDict::new(py);
        options.set_item("prefix", "built-shell-settings-")?;
        profile = py
            .import("tempfile")?
            .call_method("mkdtemp", (), Some(&options))?;
    }
    if profile.is_truthy()? {
        let data = profile.extract::<std::path::PathBuf>()?.join("data");
        std::fs::create_dir_all(&data)?;
        env.set_item(
            "XDG_DATA_HOME",
            py.import("os")?.call_method1("fsdecode", (data,))?,
        )?;
    } else if py.import("sys")?.getattr("modules")?.contains("pytest")?
        || env.contains("PYTEST_CURRENT_TEST")?
    {
        return Err(pyo3::exceptions::PyRuntimeError::new_err(
            "pytest must supply an isolated shell profile",
        ));
    }
    let settings = py.import("built_core.settings")?;
    settings.call_method1("use_native", (&profile,))?;
    let language = settings
        .call_method0("settings")?
        .call_method1("value", ("language",))?;
    let i18n = py.import("built_core.i18n")?;
    i18n.call_method1(
        "set_language",
        (i18n.call_method1("resolve_language", (language,))?,),
    )?;
    Ok(profile)
}
fn front(host: &Obj<'_>) -> PyResult<()> {
    crate::shell_runtime::windows::activate(host.py(), Some(host))
}
fn modal(host: &Obj<'_>) -> PyResult<bool> {
    host.py()
        .import(DIALOGS)?
        .call_method1("modal", (host.getattr("window")?,))?
        .is_truthy()
}
fn report(host: &Obj<'_>, place: &str, exc: &Obj<'_>) -> PyResult<()> {
    let lines: Vec<String> = host
        .py()
        .import("traceback")?
        .call_method1("format_exception", (exc,))?
        .extract()?;
    let text = lines.concat();
    let seen = host.getattr("_seen_errors")?;
    if !seen.contains(&text)? {
        seen.call_method1("add", (&text,))?;
        let errors = host.getattr("errors")?;
        if errors.len()? < 50 {
            errors.call_method1("append", ((place, &text),))?;
        }
        eprintln!("[built] error in {place}:\n{text}");
    }
    Ok(())
}
fn guarded<'p>(host: &Obj<'p>, place: &str, result: PyResult<Obj<'p>>) -> PyResult<Obj<'p>> {
    match result {
        Err(e) if e.is_instance_of::<PyException>(host.py()) => {
            report(host, place, e.value(host.py()).as_any())?;
            Ok(none(host.py()))
        }
        other => other,
    }
}
fn drain_timers(h: &Obj<'_>) -> PyResult<()> {
    guarded(
        h,
        "window timers",
        h.getattr("timers")?.call_method0("run_due"),
    )?;
    guarded(
        h,
        "hostless timers",
        h.py().import("built_core.timers")?.call_method0("run_due"),
    )?;
    Ok(())
}
fn pane<'p>(h: &Obj<'p>, kind: &str, method: &str, ev: &Obj<'p>, held: i64) -> PyResult<Obj<'p>> {
    let doc = h.getattr("window")?.getattr("_doc")?;
    doc.getattr("model_views")?
        .call_method1("route", (&doc, kind, method, ev, held))
}
fn pixels(n: f64) -> i64 {
    (n.round_ties_even() as i64).max(1)
}

pub(crate) fn document_views<'p>(h: &Obj<'p>) -> PyResult<(Obj<'p>, Obj<'p>)> {
    let py = h.py();
    let vp = py
        .import("views.document_controller")?
        .call_method1("DocumentController", (h,))?;
    if let Ok(steps) = py
        .import("built_core.general_settings")?
        .call_method1("value", ("undo_steps", 200))
        .and_then(|v| py.import("builtins")?.call_method1("int", (v,)))
    {
        vp.getattr("history")?.setattr("max_steps", steps)?;
    }
    let view = py
        .import("built_render.shell_views")?
        .call_method1("ShellView", (h,))?;
    view.setattr("source_viewport", &vp)?;
    view.setattr("camera", vp.getattr("camera")?)?;
    vp.setattr("keys_stand_in", &view)?;
    Ok((vp, view))
}
#[pyclass(module = "built_render", dict, weakref)]
pub struct Host {
    #[pyo3(get, set)]
    ns_view: usize,
    #[pyo3(get, set)]
    width: i64,
    #[pyo3(get, set)]
    height: i64,
    #[pyo3(get, set)]
    scale: f64,
    #[pyo3(get, set)]
    _frame_wanted: bool,
    #[pyo3(get, set)]
    _picture_seen: bool,
    #[pyo3(get, set)]
    _recent_dirty: bool,
    #[pyo3(get, set)]
    _cursor: Option<String>,
    #[pyo3(get, set)]
    _title: Option<String>,
    #[pyo3(get, set)]
    _preedit: String,
    #[pyo3(get, set)]
    quit_now: bool,
    #[pyo3(get, set)]
    focus_requested: bool,
    #[pyo3(get, set)]
    window_drag_requested: bool,
}
#[pymethods]
impl Host {
    #[new]
    #[pyo3(signature=(ns_view,width,height,scale,settings=None,temporary=false,display=0,platform=""))]
    fn new(
        py: Python<'_>,
        ns_view: usize,
        width: f64,
        height: f64,
        scale: f64,
        settings: Option<Obj<'_>>,
        temporary: bool,
        display: usize,
        platform: &str,
    ) -> PyResult<Py<Self>> {
        let start = Instant::now();
        let profile = start_profile(py, settings, temporary)?;
        let host = Bound::new(
            py,
            Self {
                ns_view,
                width: pixels(width),
                height: pixels(height),
                scale,
                _frame_wanted: true,
                _picture_seen: false,
                _recent_dirty: true,
                _cursor: None,
                _title: None,
                _preedit: String::new(),
                quit_now: false,
                focus_requested: false,
                window_drag_requested: false,
            },
        )?;
        let h = host.as_any();
        h.setattr("settings_dir", profile)?;
        let handles = PyDict::new(py);
        if !platform.is_empty() {
            handles.set_item("display", display)?;
            handles.set_item("platform", platform)?;
        }
        h.setattr("surface_handles", handles)?;
        for attr in [
            "errors",
            "not_here_log",
            "_after_picture",
            "window_requests",
        ] {
            h.setattr(attr, PyList::empty(py))?;
        }
        h.setattr("_seen_errors", PySet::empty(py)?)?;
        h.setattr("_custom_cursors", PyDict::new(py))?;
        for attr in ["_ime_area", "_device"] {
            h.setattr(attr, py.None())?;
        }
        let render = py.import("built_render")?;
        let core = py.import("built_core")?;
        let timers = render.call_method0("WindowTimers")?;
        h.setattr("timers", &timers)?;
        h.setattr(
            "_wake_timer",
            timers.call_method1("timer", (h.getattr("request_frame")?,))?,
        )?;
        h.setattr("input_state", core.call_method0("WindowInput")?)?;
        let w = render.call_method1("ShellWindow", (h,))?;
        h.setattr("window", &w)?;
        let (vp, view) = document_views(h)?;
        w.call_method1("bind", (&vp, view))?;
        vp.call_method1("set_active_tool", (py.None(),))?;
        front(h)?;
        let ext = w
            .getattr("__dict__")?
            .call_method1("pop", ("_pending_extensions",))?;
        render.call_method1("setup_extension_menus", (&w, ext))?;
        w.getattr("_extension_menus")?
            .call_method1("refresh", (&w,))?;
        py.import("built_render.shell_windows")?
            .call_method1("register", (h,))?;
        let kw = PyDict::new(py);
        kw.set_item("repeat", true)?;
        w.setattr(
            "_autosave_timer",
            timers.call_method("timer", (w.getattr("_on_autosave_tick")?,), Some(&kw))?,
        )?;
        guarded(h, "autosave setup", w.call_method0("_setup_autosave"))?;
        // View ▸ Show Keystrokes is a preference (the overlay for recordings).
        let keys = crate::navigation::setting(py, crate::keycast::SETTING, "0")?;
        crate::keycast::set_shown(matches!(keys.to_ascii_lowercase().as_str(), "1" | "true"));
        h.setattr("startup_ms", start.elapsed().as_secs_f64() * 1000.)?;
        w.call_method0("_update_title")?;
        Ok(host.unbind())
    }

    #[getter]
    fn viewport<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.getattr("window")?.getattr("viewport")
    }
    #[getter]
    fn view<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.getattr("window")?.getattr("view")
    }
    #[getter]
    fn path<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.getattr("window")?.getattr("_current_path")
    }
    #[getter]
    fn renderer<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        Self::view(slf)?.getattr("renderer")
    }

    fn hand_renderer(slf: &Bound<'_, Self>, view: &Obj<'_>, renderer: &Obj<'_>) -> PyResult<()> {
        let py = slf.py();
        view.setattr("renderer", renderer)?;
        if !renderer.is_none() && renderer.hasattr("set_reference_layers")? {
            renderer.call_method1(
                "set_reference_layers",
                (PyList::empty(py), PyList::empty(py)),
            )?;
            renderer.call_method1("set_center_cue", (py.None(), 0., (0., 0.), 1.))?;
        }
        view.setattr("_reference_cache", py.None())?;
        view.setattr("_had_center_cue", false)?;
        if optional(view, "_ui_timer")?.is_none() {
            view.setattr(
                "_ui_timer",
                view.call_method1("_view_timer", (view.getattr("update")?,))?,
            )?;
        }
        let dict = view.getattr("__dict__")?.cast_into::<PyDict>()?;
        for k in dict.keys() {
            let key: String = k.extract()?;
            if key.starts_with("_ui_")
                && (["_sig", "_sent", "_cv"].iter().any(|s| key.ends_with(s))
                    || matches!(
                        key.as_str(),
                        "_ui_tabs"
                            | "_ui_opts"
                            | "_ui_palette"
                            | "_ui_issues"
                            | "_ui_cursor_shape"
                            | "_ui_was_over"
                    ))
            {
                dict.del_item(k)?;
            }
        }
        view.setattr("_ui_attached", true)?;
        for k in [
            "_ui_tools_sent",
            "_ui_sheet_sig",
            "_ui_jobs_sig",
            "_ui_units_sent",
        ] {
            view.setattr(k, false)?;
        }
        let vp = view.getattr("source_viewport")?;
        if !vp.is_none() {
            vp.setattr("_chrome_owns_labels", true)?;
        }
        view.getattr("camera")?.call_method1(
            "set_aspect",
            (view.call_method0("width")?, view.call_method0("height")?),
        )?;
        for k in [
            "_physical_size",
            "_selection_signature",
            "_tint_signature",
            "_last_camera",
        ] {
            view.setattr(k, py.None())?;
        }
        view.call_method0("_finish_resize")?;
        let scene = Self::viewport(slf)?.getattr("scene")?;
        if optional(view, "packed")?.is_none() {
            view.call_method1("set_scene", (&scene,))?;
        } else {
            let kw = PyDict::new(py);
            kw.set_item("force", true)?;
            view.call_method("_upload", (), Some(&kw))?;
        }
        view.setattr("_shell_resident_version", scene.getattr("version")?)?;
        slf.borrow_mut().request_frame();
        Ok(())
    }
    fn attach_surface<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        let v = Self::view(slf)?;
        v.call_method1("set_scene", (Self::viewport(slf)?.getattr("scene")?,))?;
        if slf.borrow().ns_view != 0 {
            let kw = PyDict::new(slf.py());
            kw.set_item("force", true)?;
            v.call_method("_upload", (), Some(&kw))?;
        }
        slf.getattr("window")?
            .call_method1("_activate_tool", ("select",))?;
        v.getattr("renderer")
    }
    fn request_frame(&mut self) {
        self._frame_wanted = true;
    }
    fn wake_after(slf: &Bound<'_, Self>, msec: i64) -> PyResult<()> {
        let timer = slf.getattr("_wake_timer")?;
        let left: i64 = timer.call_method0("remainingTime")?.extract()?;
        if left < 0 || left > msec {
            timer.call_method1("start", (msec,))?;
        }
        Ok(())
    }
    #[staticmethod]
    #[pyo3(signature=(key,plus=false))]
    fn tool_cursor<'p>(py: Python<'p>, key: Option<&str>, plus: bool) -> PyResult<Obj<'p>> {
        let style = py
            .import("built_core.general_settings")?
            .call_method1("value", ("cursor_style", "tool"))?
            .str()?;
        py.import("built_render")?
            .call_method1("tool_cursor_name", (key, plus, style))
    }
    fn cursor_changed(&mut self, cursor: &Obj<'_>) -> PyResult<()> {
        self._cursor = Some(if let Ok(s) = cursor.extract::<String>() {
            s
        } else {
            let val = (|| -> PyResult<i64> {
                let shape = if cursor.hasattr("shape")? {
                    cursor.call_method0("shape")?
                } else {
                    cursor.clone()
                };
                shape.getattr_opt("value")?.unwrap_or(shape).extract()
            })();
            match val {
                Ok(0 | 1 | 10) | Err(_) => "default",
                Ok(2) => "crosshair",
                Ok(3) => "wait",
                Ok(4) => "text",
                Ok(5) => "ns-resize",
                Ok(6) => "ew-resize",
                Ok(7) => "nesw-resize",
                Ok(8) => "nwse-resize",
                Ok(9) => "move",
                Ok(11) => "row-resize",
                Ok(12) => "col-resize",
                Ok(13) => "pointer",
                Ok(14) => "not-allowed",
                Ok(15) => "help",
                Ok(16) => "progress",
                Ok(17) => "grab",
                Ok(18) => "grabbing",
                Ok(20) => "copy",
                Ok(21) => "alias",
                _ => "crosshair",
            }
            .into()
        });
        Ok(())
    }
    fn custom_cursor<'p>(slf: &Bound<'p, Self>, key: &str) -> PyResult<Obj<'p>> {
        slf.getattr("_custom_cursors")?.call_method1("get", (key,))
    }
    fn set_title(&mut self, title: String) {
        self._title = Some(title);
    }
    fn _set_title(slf: &Bound<'_, Self>) -> PyResult<()> {
        slf.getattr("window")?.call_method0("_update_title")?;
        Ok(())
    }
    fn recent_changed(&mut self) {
        self._recent_dirty = true;
    }
    fn after_picture(slf: &Bound<'_, Self>, callback: &Obj<'_>) -> PyResult<()> {
        slf.getattr("_after_picture")?
            .call_method1("append", (callback,))?;
        slf.borrow_mut()._picture_seen = false;
        Ok(())
    }
    fn not_here(slf: &Bound<'_, Self>, feature: &str) -> PyResult<()> {
        slf.getattr("not_here_log")?
            .call_method1("append", (feature,))?;
        slf.getattr("window")?
            .call_method0("statusBar")?
            .call_method1(
                "showMessage",
                (format!("{feature} is unavailable in this window"), 6000),
            )?;
        Ok(())
    }
    fn idle<'p>(
        slf: &Bound<'p, Self>,
    ) -> PyResult<(bool, Option<f64>, Option<String>, Option<String>)> {
        let h = slf.as_any();
        let py = h.py();
        drain_timers(h)?;
        let win = h.getattr("window")?;
        win.getattr("_extension_menus")?
            .call_method1("refresh", (&win,))?;
        let pending = win
            .getattr("__dict__")?
            .call_method1("get", ("_pending_open",))?;
        let now = || {
            py.import("time")?
                .call_method0("monotonic")?
                .extract::<f64>()
        };
        if pending.is_truthy()? && now()? >= pending.getattr("next_poll")?.extract::<f64>()? {
            pending.setattr("next_poll", now()? + 0.032)?;
            pending.call_method0("poll")?;
            slf.borrow_mut().request_frame();
        }
        let v = Self::view(slf)?;
        let scene = Self::viewport(slf)?.getattr("scene")?;
        win.getattr("_doc")?
            .getattr("viewport")?
            .getattr("_document_watch")?
            .call_method0("observe")?;
        if !v.getattr("renderer")?.is_none() && scene_changed(&v, &scene)? {
            slf.borrow_mut().request_frame();
        }
        let mut delay = if pending.is_truthy()? {
            Some(((pending.getattr("next_poll")?.extract::<f64>()? - now()?) * 1000.).max(0.))
        } else {
            None
        };
        let mut sooner = |ms: Option<f64>| {
            if let Some(ms) = ms {
                delay = Some(delay.map_or(ms, |d| d.min(ms)));
            }
        };
        if !optional(&v, "_inference_future")?.is_none()
            || optional(&v, "_inference_queued")?.is_truthy()?
        {
            sooner(Some(33.));
        }
        let watch = win
            .getattr("__dict__")?
            .call_method1("get", ("_links_watch",))?;
        if !watch.is_none() {
            sooner(watch.call_method0("poll")?.extract()?);
        }
        let queue = h.getattr("_after_picture")?;
        if slf.borrow()._picture_seen && queue.len()? > 0 {
            let f = queue.call_method1("pop", (0,))?;
            guarded(h, "after picture", f.call0())?;
            sooner(Some(16.));
        }
        sooner(h.getattr("timers")?.call_method0("next_ms")?.extract()?);
        sooner(
            py.import("built_core.timers")?
                .call_method0("next_ms")?
                .extract()?,
        );
        let mut host = slf.borrow_mut();
        Ok((
            std::mem::take(&mut host._frame_wanted),
            delay,
            host._cursor.take(),
            host._title.take(),
        ))
    }
    fn extras<'p>(slf: &Bound<'p, Self>) -> PyResult<Bound<'p, PyDict>> {
        let py = slf.py();
        let d = PyDict::new(py);
        let dirty = std::mem::take(&mut slf.borrow_mut()._recent_dirty);
        let recent = if dirty {
            slf.getattr("window")?
                .call_method0("_recent_paths")
                .unwrap_or_else(|_| PyList::empty(py).into_any())
        } else {
            none(py)
        };
        d.set_item("recent", recent)?;
        d.set_item("exit", slf.borrow().quit_now)?;
        d.set_item("ime_area", slf.getattr("_ime_area")?)?;
        let q = slf.getattr("window_requests")?;
        d.set_item(
            "new_window",
            if q.len()? > 0 {
                q.call_method1("pop", (0,))?
            } else {
                none(py)
            },
        )?;
        d.set_item(
            "focus",
            std::mem::take(&mut slf.borrow_mut().focus_requested),
        )?;
        Ok(d)
    }
    fn adopt(slf: &Bound<'_, Self>, token: &Obj<'_>) -> PyResult<bool> {
        let h = slf.as_any();
        front(h)?;
        let windows = h.py().import("built_render.shell_windows")?;
        let request = windows.call_method1("take", (token,))?;
        if request.is_none() {
            return Ok(false);
        }
        let ok = if request.get_item(0)?.eq("move")? {
            let payload = request.get_item(1)?;
            guarded(
                h,
                "move project",
                windows.call_method1("transfer", (payload.get_item(0)?, payload.get_item(1)?, h)),
            )?
            .is_truthy()?
        } else {
            Self::cold_start(slf)?;
            true
        };
        slf.borrow_mut().request_frame();
        Ok(ok)
    }
    fn _front(slf: &Bound<'_, Self>) -> PyResult<()> {
        front(slf.as_any())
    }
    fn frame<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        let v = Self::view(slf)?;
        let scene = Self::viewport(slf)?.getattr("scene")?;
        guarded(
            slf.as_any(),
            "frame",
            (|| {
                if scene_changed(&v, &scene)? {
                    let version = scene.getattr("version")?;
                    v.call_method1("set_scene", (&scene,))?;
                    v.setattr("_shell_resident_version", version)?;
                }
                let doc = slf.getattr("window")?.getattr("_doc")?;
                let out = doc.getattr("model_views")?.call_method1("frame", (&doc,))?;
                if !out.is_none() {
                    slf.borrow_mut()._picture_seen = true;
                }
                Ok(out)
            })(),
        )
    }
    fn modal_frame(slf: &Bound<'_, Self>) -> PyResult<()> {
        drain_timers(slf.as_any())?;
        let draw = std::mem::take(&mut slf.borrow_mut()._frame_wanted);
        if draw {
            Self::frame(slf)?;
        }
        Ok(())
    }
    fn error(slf: &Bound<'_, Self>, place: &str, exc: &Obj<'_>) -> PyResult<()> {
        report(slf.as_any(), place, exc)
    }
    #[pyo3(signature=(place,callback,*args))]
    fn _guard<'p>(
        slf: &Bound<'p, Self>,
        place: &str,
        callback: &Obj<'p>,
        args: &Bound<'p, PyTuple>,
    ) -> PyResult<Obj<'p>> {
        guarded(slf.as_any(), place, callback.call1(args))
    }
    fn _modal(slf: &Bound<'_, Self>) -> PyResult<bool> {
        modal(slf.as_any())
    }
    fn open_path(slf: &Bound<'_, Self>, path: &Obj<'_>) -> PyResult<bool> {
        let py = slf.py();
        front(slf.as_any())?;
        let start = Instant::now();
        let w = slf.getattr("window")?;
        let p = py.import("pathlib")?.call_method1("Path", (path,))?;
        let ok = guarded(
            slf.as_any(),
            "open",
            py.import(FILES)?.call_method1("open_path", (&w, p)),
        )?
        .is_truthy()?;
        let key = if w
            .getattr("__dict__")?
            .call_method1("get", ("_pending_open",))?
            .is_truthy()?
        {
            "dispatch"
        } else {
            "total"
        };
        let d = PyDict::new(py);
        d.set_item(key, start.elapsed().as_secs_f64() * 1000.)?;
        slf.setattr("open_ms", d)?;
        slf.borrow_mut().request_frame();
        Ok(ok)
    }
    fn open(slf: &Bound<'_, Self>, path: &Obj<'_>) -> PyResult<bool> {
        Self::open_path(slf, path)
    }
    fn cold_start(slf: &Bound<'_, Self>) -> PyResult<()> {
        guarded(
            slf.as_any(),
            "cold start",
            slf.py()
                .import(FILES)?
                .call_method1("cold_start", (slf.getattr("window")?,)),
        )?;
        Ok(())
    }
    fn request_close(slf: &Bound<'_, Self>) -> PyResult<bool> {
        front(slf.as_any())?;
        guarded(
            slf.as_any(),
            "close",
            slf.py()
                .import(FILES)?
                .call_method1("request_close", (slf.getattr("window")?,)),
        )?
        .is_truthy()
    }
    fn confirm_window_close<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        front(slf.as_any())?;
        slf.py()
            .import("built_render")?
            .call_method1("confirm_window_close", (slf.getattr("window")?,))
    }
    fn finish_window_close(slf: &Bound<'_, Self>) -> PyResult<()> {
        slf.py()
            .import("built_render")?
            .call_method1("finish_window_close", (slf.getattr("window")?,))?;
        Ok(())
    }
    #[pyo3(signature=(kind,x,y,button=0,held=0,modifiers=0,timestamp=0))]
    fn pointer(
        slf: &Bound<'_, Self>,
        kind: &str,
        x: f64,
        y: f64,
        button: i64,
        held: i64,
        modifiers: i64,
        timestamp: u64,
    ) -> PyResult<()> {
        let h = slf.as_any();
        let py = h.py();
        let w = h.getattr("window")?;
        h.getattr("input_state")?
            .call_method1("pointer", (x, y, held, modifiers))?;
        if kind == "press" && crate::keycast::shown() {
            let renderer = Self::renderer(slf)?;
            let over_chrome = !renderer.is_none()
                && renderer.call_method1("ui_over_chrome", (x, y)).and_then(|v| v.is_truthy()).unwrap_or(false);
            if !renderer.is_none() && !over_chrome {
                renderer.call_method1("ui_keycast", ("button", format!("{button}:{modifiers}")))?;
            }
        }
        if matches!(kind, "press" | "double") {
            w.getattr("viewport")?.setattr("_clean_alt_press", false)?;
            w.getattr("scene_player")?.call_method0("stop")?;
            front(h)?;
        }
        if kind == "leave" {
            guarded(
                h,
                "leave",
                Self::view(slf)?.call_method1("leaveEvent", (py.None(),)),
            )?;
            return Ok(());
        }
        let method = match kind {
            "press" => "mousePressEvent",
            "release" => "mouseReleaseEvent",
            "move" => "mouseMoveEvent",
            "double" => "mouseDoubleClickEvent",
            _ => return Err(PyValueError::new_err("unknown pointer event")),
        };
        let native = py.import("built_render")?;
        let kw = PyDict::new(py);
        kw.set_item("timestamp", timestamp)?;
        let ev = native.call_method(
            "MouseEvent",
            (kind, x, y, button, held, modifiers),
            Some(&kw),
        )?;
        let v = pane(h, kind, method, &ev, held)?;
        if !v.is_none() {
            guarded(
                h,
                &format!("pointer {kind}"),
                v.call_method1(method, (&ev,)),
            )?;
            if kind == "press" && button == 2 && !modal(h)? {
                let ev = native.call_method1(
                    "ContextMenuEvent",
                    (ev.call_method0("x")?, ev.call_method0("y")?, modifiers),
                )?;
                guarded(h, "context menu", v.call_method1("contextMenuEvent", (ev,)))?;
            }
        }
        slf.borrow_mut().request_frame();
        Ok(())
    }
    #[pyo3(signature=(x,y,pixel_dx,pixel_dy,angle_dx,angle_dy,phase="none",modifiers=0,held=0,inverted=true,touch_screen=false))]
    fn wheel(
        slf: &Bound<'_, Self>,
        x: f64,
        y: f64,
        pixel_dx: f64,
        pixel_dy: f64,
        angle_dx: f64,
        angle_dy: f64,
        phase: &str,
        modifiers: i64,
        held: i64,
        inverted: bool,
        touch_screen: bool,
    ) -> PyResult<()> {
        let h = slf.as_any();
        h.getattr("input_state")?
            .call_method1("pointer", (x, y, held, modifiers))?;
        if modal(h)? {
            return Ok(());
        }
        h.getattr("window")?
            .getattr("scene_player")?
            .call_method0("stop")?;
        let ev = h.py().import("built_render")?.call_method1(
            "WheelEvent",
            (
                x,
                y,
                pixel_dx,
                pixel_dy,
                angle_dx,
                angle_dy,
                phase,
                modifiers,
                held,
                inverted,
                touch_screen,
            ),
        )?;
        let v = pane(h, "wheel", "wheelEvent", &ev, held)?;
        if !v.is_none() {
            guarded(h, "wheel", v.call_method1("wheelEvent", (ev,)))?;
        }
        slf.borrow_mut().request_frame();
        Ok(())
    }
    #[pyo3(signature=(kind,value,x,y,modifiers=0))]
    fn gesture(
        slf: &Bound<'_, Self>,
        kind: &str,
        value: f64,
        x: f64,
        y: f64,
        modifiers: i64,
    ) -> PyResult<()> {
        let h = slf.as_any();
        let i = h.getattr("input_state")?;
        let held = i.getattr("buttons")?.extract::<i64>()?;
        i.call_method1("pointer", (x, y, held, modifiers))?;
        if modal(h)? {
            return Ok(());
        }
        h.getattr("window")?
            .getattr("scene_player")?
            .call_method0("stop")?;
        let kw = PyDict::new(h.py());
        kw.set_item("modifiers", modifiers)?;
        let ev = h.py().import("built_render")?.call_method(
            "NativeGestureEvent",
            (kind, value, x, y),
            Some(&kw),
        )?;
        let v = pane(h, "gesture", "event", &ev, held)?;
        if !v.is_none() {
            guarded(h, "gesture", v.call_method1("event", (ev,)))?;
        }
        slf.borrow_mut().request_frame();
        Ok(())
    }
    #[pyo3(signature=(pressed,key,modifiers=0,text="",repeat=false))]
    fn key(
        slf: &Bound<'_, Self>,
        pressed: bool,
        key: i64,
        modifiers: i64,
        text: &str,
        repeat: bool,
    ) -> PyResult<bool> {
        let cast = pressed && !repeat && crate::keycast::shown();
        let before = if cast { Self::keycast_probe(slf).ok() } else { None };
        let out = Self::key_event(slf, pressed, key, modifiers, text, repeat)?;
        if let Some(before) = before {
            guarded(slf.as_any(), "keystrokes", Self::keycast_after(slf, key, modifiers, before).map(|_| none(slf.py())))?;
        }
        Ok(out)
    }
    /// The active tool and the value field's text, before a key.
    fn keycast_probe(slf: &Bound<'_, Self>) -> PyResult<(Option<String>, String)> {
        let vp = slf.getattr("window")?.getattr("viewport")?;
        let tool = vp.getattr("active_tool")?;
        let key = if tool.is_none() { None } else { slf.getattr("window")?.call_method1("_tool_key", (&tool,))?.extract()? };
        let buffer = vp.getattr_opt("_value_buffer")?.map(|b| b.extract::<String>()).transpose()?.unwrap_or_default();
        Ok((key, buffer))
    }
    /// What the key did, for the keystroke overlay: armed a tool, typed
    /// into the value field, entered the value, or just the key itself.
    fn keycast_after(slf: &Bound<'_, Self>, key: i64, modifiers: i64, before: (Option<String>, String)) -> PyResult<()> {
        let renderer = Self::renderer(slf)?;
        if renderer.is_none() { return Ok(()); }
        let Some(keys) = crate::keycast::chord(key, modifiers) else { return Ok(()) };
        let (tool, buffer) = Self::keycast_probe(slf)?;
        let entered = matches!(key, crate::keycast::RETURN | crate::keycast::ENTER) && !before.1.is_empty();
        if tool.is_some() && tool != before.0 {
            let active = slf.getattr("window")?.getattr("viewport")?.getattr("active_tool")?;
            let name: String = active.getattr("name")?.extract()?;
            let label = crate::model_edit::translated(slf.py(), &name, None)?;
            renderer.call_method1("ui_keycast", ("tool", keys, label))?;
        } else if buffer != before.1 && !buffer.is_empty() {
            renderer.call_method1("ui_keycast", ("typed", buffer))?;
        } else if entered {
            renderer.call_method1("ui_keycast", ("key", keys))?;
        } else if buffer == before.1 {
            renderer.call_method1("ui_keycast", ("key", keys))?;
        }
        Ok(())
    }
    fn key_event(
        slf: &Bound<'_, Self>,
        pressed: bool,
        key: i64,
        modifiers: i64,
        text: &str,
        repeat: bool,
    ) -> PyResult<bool> {
        const ESCAPE: i64 = 0x01000000;
        let h = slf.as_any();
        let py = h.py();
        let native = py.import("built_render")?;
        let cd = py.import(DIALOGS)?;
        let w = h.getattr("window")?;
        h.getattr("input_state")?.setattr("modifiers", modifiers)?;
        if pressed {
            front(h)?;
        }
        let pending = w
            .getattr("__dict__")?
            .call_method1("get", ("_pending_open",))?;
        let blocked = modal(h)?;
        if pressed && key == ESCAPE && !blocked && pending.is_truthy()? {
            pending.getattr("job")?.call_method0("cancel")?;
            return Ok(true);
        }
        let event = || native.call_method1("KeyEvent", (pressed, key, modifiers, text, repeat));
        if blocked {
            let sheet = cd.call_method1("current_sheet", (&w,))?;
            if sheet.is_none() && cd.call_method1("_form_up", (&w,))?.is_truthy()? {
                guarded(
                    h,
                    "form key",
                    py.import("views.wgpu_ui")?
                        .call_method1("key", (Self::view(slf)?, event()?, pressed)),
                )?;
            } else if pressed && !repeat {
                let (label, action) = if sheet.is_none() {
                    ("job key", "job_key")
                } else {
                    ("sheet key", "sheet_key")
                };
                guarded(h, label, cd.call_method1(action, (&w, event()?)))?;
            }
        } else {
            native.call_method1(
                "observe_alt_key",
                (w.getattr("viewport")?, pressed, key, repeat),
            )?;
            let doc = w.getattr("_doc")?;
            let views = doc.getattr("model_views")?;
            if pressed && key == ESCAPE {
                if views.call_method0("cancel_divider")?.is_truthy()? {
                    slf.borrow_mut().request_frame();
                    return Ok(true);
                }
                let player = w.getattr("scene_player")?;
                if player.call_method0("is_active")?.is_truthy()? {
                    player.call_method0("stop")?;
                    return Ok(true);
                }
            }
            if pressed {
                let ev = native.call_method1("ShortcutOverride", (key, modifiers, text, repeat))?;
                let target = views.call_method1("key_view", (&doc, "event", &ev))?;
                if !target.is_none() {
                    guarded(h, "shortcut override", target.call_method1("event", (&ev,)))?;
                }
                if !ev.call_method0("isAccepted")?.is_truthy()?
                    && !repeat
                    && Self::_shortcut(slf, key, modifiers)?
                {
                    slf.borrow_mut().request_frame();
                    return Ok(true);
                }
            }
            let method = if pressed {
                "keyPressEvent"
            } else {
                "keyReleaseEvent"
            };
            let ev = event()?;
            let target = views.call_method1("key_view", (&doc, method, &ev))?;
            if !target.is_none() {
                guarded(h, "key", target.call_method1(method, (ev,)))?;
            }
        }
        slf.borrow_mut().request_frame();
        Ok(true)
    }
    fn refresh_commands(slf: &Bound<'_, Self>) -> PyResult<()> {
        slf.getattr("window")?.call_method0("refresh_commands")?;
        Ok(())
    }
    fn _shortcut(slf: &Bound<'_, Self>, key: i64, modifiers: i64) -> PyResult<bool> {
        let w = slf.getattr("window")?;
        let store = w.getattr("_command_store")?;
        let slot = store.call_method1("resolve", (key, modifiers))?;
        let action = if slot.is_none() {
            store.call_method1("plugin_action", (key, modifiers))?
        } else {
            w.getattr("_command_actions")?.get_item(slot)?
        };
        if action.is_none() {
            return Ok(false);
        }
        action.call_method0("trigger")?;
        Ok(true)
    }
    fn text(slf: &Bound<'_, Self>, text: &str) -> PyResult<()> {
        let renderer = Self::renderer(slf)?;
        if !slf.borrow()._preedit.is_empty() {
            slf.borrow_mut()._preedit.clear();
            slf.getattr("window")?
                .call_method0("statusBar")?
                .call_method0("clearMessage")?;
        }
        if !renderer.is_none() && renderer.call_method1("ui_claims_key", ("",))?.is_truthy()? {
            if renderer.hasattr("ui_ime")? {
                renderer.call_method1("ui_ime", ("commit", text))?;
            } else {
                renderer.call_method1("ui_text", (text,))?;
            }
            slf.borrow_mut().request_frame();
            return Ok(());
        }
        for ch in text.chars() {
            let key = ch.to_uppercase().next().unwrap_or(ch) as i64;
            let text = ch.to_string();
            Self::key(slf, true, key, 0, &text, false)?;
            Self::key(slf, false, key, 0, &text, false)?;
        }
        Ok(())
    }
    #[pyo3(signature=(text,cursor=0))]
    fn preedit(slf: &Bound<'_, Self>, text: &str, cursor: i64) -> PyResult<()> {
        let _ = cursor;
        let was_empty = slf.borrow()._preedit.is_empty();
        slf.borrow_mut()._preedit = text.to_owned();
        let r = Self::renderer(slf)?;
        if !r.is_none()
            && r.call_method1("ui_claims_key", ("",))?.is_truthy()?
            && r.hasattr("ui_ime")?
        {
            if was_empty && !text.is_empty() {
                r.call_method1("ui_ime", ("enabled", ""))?;
            }
            r.call_method1("ui_ime", ("preedit", text))?;
            let rect = if r.hasattr("ui_ime_rect")? {
                r.call_method0("ui_ime_rect")?
            } else {
                none(slf.py())
            };
            slf.setattr(
                "_ime_area",
                if rect.is_truthy()? {
                    slf.py()
                        .import("builtins")?
                        .call_method1("tuple", (rect,))?
                } else {
                    none(slf.py())
                },
            )?;
        } else {
            let bar = slf.getattr("window")?.call_method0("statusBar")?;
            if text.is_empty() {
                bar.call_method0("clearMessage")?;
            } else {
                bar.call_method1("showMessage", (format!("Composing: {text}"), 4000))?;
            }
        }
        slf.borrow_mut().request_frame();
        Ok(())
    }
    fn drag_hover(slf: &Bound<'_, Self>, path: &Obj<'_>) -> PyResult<()> {
        let bar = slf.getattr("window")?.call_method0("statusBar")?;
        if path.is_truthy()? {
            let name = slf
                .py()
                .import("pathlib")?
                .call_method1("Path", (path,))?
                .getattr("name")?;
            bar.call_method1("showMessage", (format!("Drop to open {name}"), 3000))?;
        } else {
            bar.call_method0("clearMessage")?;
        }
        slf.borrow_mut().request_frame();
        Ok(())
    }
    fn resize(slf: &Bound<'_, Self>, width: f64, height: f64, scale: f64) -> PyResult<()> {
        {
            let mut h = slf.borrow_mut();
            h.width = pixels(width);
            h.height = pixels(height);
            h.scale = scale;
        }
        guarded(
            slf.as_any(),
            "resize",
            Self::view(slf)?.call_method1("resizeEvent", (slf.py().None(),)),
        )?;
        slf.borrow_mut().request_frame();
        Ok(())
    }
    fn focus(slf: &Bound<'_, Self>, on: bool) -> PyResult<()> {
        let h = slf.as_any();
        let py = h.py();
        let win = h.getattr("window")?;
        let current = h.getattr("_device")?;
        if on {
            front(h)?;
            if current.is_none() {
                let d = py
                    .import("views.ndof_input")?
                    .call_method1("shared_input", (&win,))?;
                if d.getattr("backend_name")?.is_truthy()? {
                    h.setattr("_device", &d)?;
                    for (sig, handler) in
                        [("motion", "_on_ndof_motion"), ("button", "_on_ndof_button")]
                    {
                        d.getattr(sig)?
                            .call_method1("connect", (win.getattr(handler)?,))?;
                    }
                }
            }
        } else {
            win.getattr("viewport")?
                .setattr("_clean_alt_press", false)?;
            if !current.is_none() {
                for (sig, handler) in [("motion", "_on_ndof_motion"), ("button", "_on_ndof_button")]
                {
                    current
                        .getattr(sig)?
                        .call_method1("disconnect", (win.getattr(handler)?,))?;
                }
                h.setattr("_device", py.None())?;
            }
            win.getattr("_doc")?
                .getattr("model_views")?
                .call_method0("cancel")?;
            h.getattr("input_state")?.call_method0("focus_lost")?;
            guarded(
                h,
                "focus out",
                Self::view(slf)?.call_method1("focusOutEvent", (py.None(),)),
            )?;
        }
        Ok(())
    }
    fn menu<'p>(slf: &Bound<'p, Self>, action: &str) -> PyResult<Obj<'p>> {
        let h = slf.as_any();
        let py = h.py();
        let w = h.getattr("window")?;
        front(h)?;
        if modal(h)? {
            return Ok(none(py));
        }
        let (kind, arg) = action.split_once(':').unwrap_or((action, ""));
        let mut out = none(py);
        match kind {
            "tool" => {
                guarded(h, "tool", w.call_method1("_activate_tool", (arg,)))?;
            }
            "panel" => {
                guarded(
                    h,
                    "open panel",
                    py.import("built_core.plugin_panels")?
                        .call_method1("open_panel", (&w, arg)),
                )?;
            }
            "undo" | "redo" => {
                guarded(h, kind, w.call_method0(format!("_on_{kind}")))?;
            }
            "zoom_extents" | "view" => {
                let cmd = if kind == "view" {
                    crate::commands::native_view_command(arg)
                } else {
                    action
                };
                guarded(
                    h,
                    "camera command",
                    py.import("views.view_commands")?
                        .call_method1("run", (&w, cmd)),
                )?;
            }
            _ => {
                let handler = match (kind, arg) {
                    ("verb", "divide") => Some("_on_divide"),
                    ("verb", "new_tag") => Some("_assign_new_layer"),
                    ("verb", "image_size") => Some("_on_image_size"),
                    ("verb", "image_opacity") => Some("_on_image_opacity"),
                    _ => None,
                };
                if let Some(handler) = handler {
                    guarded(h, &format!("verb {arg}"), w.call_method0(handler))?;
                } else {
                    out = guarded(
                        h,
                        &format!("menu {action}"),
                        w.call_method1("run_command", (action,)),
                    )?;
                }
            }
        }
        let v = Self::view(slf)?;
        if !v.is_none() && kind != "tool" {
            guarded(h, "tool sync", v.getattr("tools")?.call_method0("sync"))?;
        }
        slf.borrow_mut().request_frame();
        Ok(out)
    }
    fn close(slf: &Bound<'_, Self>) -> PyResult<()> {
        let h = slf.as_any();
        let py = h.py();
        let w = h.getattr("window")?;
        if !h.getattr("_device")?.is_none() {
            Self::focus(slf, false)?;
        }
        py.import("built_render.chrome_sheet")?
            .call_method1("close", (&w,))?;
        py.import("built_render.chrome_forms")?
            .call_method1("close_all", (&w,))?;
        w.setattr("_release_check", py.None())?;
        py.import("built_render.shell_windows")?
            .call_method1("unregister", (h,))?;
        w.getattr("scene_player")?.call_method0("cancel")?;
        w.getattr("_extension_menus")?.call_method0("clear")?;
        w.getattr("_extension_closed")?.call_method0("emit")?;
        h.getattr("timers")?.call_method0("cancel_all")?;
        for doc in py
            .import("builtins")?
            .call_method1("list", (w.getattr("_documents")?,))?
            .try_iter()?
        {
            let doc = doc?;
            doc.getattr("model_views")?
                .call_method1("suspend", (&doc,))?;
            crate::shell_window::release_watch(&doc)?;
            let v = doc.getattr("stand_in")?;
            if !v.is_none() {
                guarded(h, "close", v.call_method1("closeEvent", (py.None(),)))?;
            }
        }
        Ok(())
    }
    fn state<'p>(slf: &Bound<'p, Self>) -> PyResult<Bound<'p, PyDict>> {
        diagnostics::snapshot(slf)
    }
}
fn scene_changed(v: &Obj<'_>, scene: &Obj<'_>) -> PyResult<bool> {
    Ok(!scene.is(optional(v, "scene")?)
        || !scene
            .getattr("version")?
            .eq(optional(v, "_shell_resident_version")?)?)
}
