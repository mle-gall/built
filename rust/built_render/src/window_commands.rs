// SPDX-License-Identifier: GPL-3.0-or-later
//! Document verbs bound directly to ShellWindow. Repeatable verbs remember
//! their arguments for Repeat (Shift+R).
use crate::settings_rows::{func, none, obj, opt, s, tr};
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};

fn native(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("built_render")
}

/// Positional and keyword arguments of a call, read by position or name.
struct Args<'a, 'py> {
    pos: &'a Bound<'py, PyTuple>,
    kw: Option<&'a Bound<'py, PyDict>>,
}

impl<'py> Args<'_, 'py> {
    fn at(&self, i: usize, name: &str) -> PyResult<Option<Bound<'py, PyAny>>> {
        if i < self.pos.len() {
            return Ok(Some(self.pos.get_item(i)?));
        }
        match self.kw {
            Some(kw) => kw.get_item(name),
            None => Ok(None),
        }
    }

    fn req(&self, i: usize, name: &str) -> PyResult<Bound<'py, PyAny>> {
        self.at(i, name)?.ok_or_else(|| pyo3::exceptions::PyTypeError::new_err(format!("missing argument '{name}'")))
    }

    fn or_none(&self, i: usize, name: &str) -> PyResult<Bound<'py, PyAny>> {
        let py = self.pos.py();
        Ok(self.at(i, name)?.unwrap_or_else(|| py.None().into_bound(py)))
    }

    fn or<T: IntoPyObject<'py>>(&self, i: usize, name: &str, default: T) -> PyResult<Bound<'py, PyAny>> {
        let py = self.pos.py();
        match self.at(i, name)? {
            Some(v) => Ok(v),
            None => Ok(obj(py, default)?.into_bound(py)),
        }
    }

    /// A keyword-only argument.
    fn kw_or<T: IntoPyObject<'py>>(&self, name: &str, default: T) -> PyResult<Bound<'py, PyAny>> {
        let py = self.pos.py();
        match self.kw.map(|k| k.get_item(name)).transpose()?.flatten() {
            Some(v) => Ok(v),
            None => Ok(obj(py, default)?.into_bound(py)),
        }
    }
}

fn kwargs<'py>(py: Python<'py>, items: &[(&str, Bound<'py, PyAny>)]) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    for (k, v) in items {
        d.set_item(*k, v)?;
    }
    Ok(d)
}

/// One verb: its name, and its Repeat label with the arguments it keeps.
#[pyclass(module = "built_render", frozen, weakref)]
pub struct Verb {
    name: &'static str,
    repeat: Option<(&'static str, usize)>,
}

#[pymethods]
impl Verb {
    fn __get__<'py>(slf: &Bound<'py, Self>, instance: Option<&Bound<'py, PyAny>>, _owner: Option<&Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyAny>> {
        match instance.filter(|i| !i.is_none()) {
            None => Ok(slf.clone().into_any()),
            Some(win) => Ok(Bound::new(slf.py(), BoundVerb { verb: slf.clone().unbind(), win: win.clone().unbind() })?.into_any()),
        }
    }

    #[pyo3(signature = (win, *args, **kwargs))]
    fn __call__<'py>(&self, win: &Bound<'py, PyAny>, args: &Bound<'py, PyTuple>, kwargs: Option<&Bound<'py, PyDict>>) -> PyResult<Bound<'py, PyAny>> {
        let py = win.py();
        if let Some((label, takes)) = self.repeat {
            let kept = args.get_slice(0, takes.min(args.len()));
            let (name, w, a) = (self.name, win.clone().unbind(), kept.clone().unbind());
            let again = func(py, move |py, _| Ok(run(py, name, w.bind(py), &Args { pos: a.bind(py), kw: None })?.unbind()))?;
            win.call_method1("_remember_command", (label, again))?;
            return run(py, self.name, win, &Args { pos: &kept, kw: None });
        }
        run(py, self.name, win, &Args { pos: args, kw: kwargs })
    }

    #[getter]
    fn __name__(&self) -> &'static str {
        self.name
    }

    fn __repr__(&self) -> String {
        format!("<window verb {}>", self.name)
    }
}

/// A verb bound to a window, as a method is: `__func__`, `__self__`, equal
/// to the same verb on the same window (Qt's `disconnect`, `WeakMethod`).
/// Not a `types.MethodType`: PySide reads a method's function as Python
/// code when it connects a signal to it.
#[pyclass(module = "built_render", frozen, weakref)]
pub struct BoundVerb {
    verb: Py<Verb>,
    win: Py<PyAny>,
}

#[pymethods]
impl BoundVerb {
    #[new]
    fn new(verb: Py<Verb>, win: Py<PyAny>) -> Self {
        BoundVerb { verb, win }
    }

    #[pyo3(signature = (*args, **kwargs))]
    fn __call__<'py>(&self, py: Python<'py>, args: &Bound<'py, PyTuple>, kwargs: Option<&Bound<'py, PyDict>>) -> PyResult<Bound<'py, PyAny>> {
        self.verb.get().__call__(self.win.bind(py), args, kwargs)
    }

    #[getter]
    fn __func__(&self, py: Python<'_>) -> Py<Verb> {
        self.verb.clone_ref(py)
    }

    #[getter]
    fn __self__(&self, py: Python<'_>) -> Py<PyAny> {
        self.win.clone_ref(py)
    }

    #[getter]
    fn __name__(&self) -> &'static str {
        self.verb.get().name
    }

    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> bool {
        other.cast::<BoundVerb>().is_ok_and(|o| o.get().verb.is(&self.verb) && o.get().win.bind(py).is(self.win.bind(py)))
    }

    fn __hash__(&self) -> isize {
        (self.verb.as_ptr() as isize) ^ (self.win.as_ptr() as isize).rotate_left(17)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!("<bound window verb {} of {}>", self.verb.get().name, self.win.bind(py).repr()?))
    }
}

/// The window's front view (the one being worked in).
fn viewport<'py>(win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    win.getattr("viewport")
}

fn scene<'py>(win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    viewport(win)?.getattr("scene")
}

fn status(win: &Bound<'_, PyAny>, text: &str, ms: i64) -> PyResult<()> {
    win.call_method0("statusBar")?.call_method1("showMessage", (text, ms))?;
    Ok(())
}

/// The verb `name` on `win`.
fn run<'py>(py: Python<'py>, name: &str, win: &Bound<'py, PyAny>, a: &Args<'_, 'py>) -> PyResult<Bound<'py, PyAny>> {
    let n = native(py)?;
    let nothing = || Ok(py.None().into_bound(py));
    macro_rules! call {
        ($f:literal, $args:expr) => {
            n.call_method1($f, $args)
        };
    }
    macro_rules! done {
        ($e:expr) => {{
            $e?;
            nothing()
        }};
    }
    match name {
        // ---- groups, components, paths, images, text -----------------------------------
        "_on_make_unique" => call!("unique_components", (win,)),
        "_on_merge_groups" => call!("merge_groups", (win,)),
        "_on_explode_curve" => call!("explode_curves", (win,)),
        "_on_reverse_section" => call!("section_action", (win, "reverse")),
        "_on_toggle_active_section" => call!("section_action", (win, "toggle")),
        "_on_align_view_to_section" => call!("section_action", (win, "align")),
        "_on_align_view_to_face" => call!("align_face", (viewport(win)?, a.req(0, "face")?)),
        "_on_set_surface" => done!(call!("set_path_surface", (win, a.req(0, "mode")?))),
        "_on_toggle_path_closed" => call!("toggle_paths", (win,)),
        "_assign_layer" => call!("assign_tag", (win, a.req(0, "name")?)),
        "_unlock_group" => call!("unlock_group", (viewport(win)?, a.req(0, "group")?)),
        "_delete_image" => call!("image_delete", (viewport(win)?, a.req(0, "image")?)),
        "_unlock_image" => call!("image_unlock", (win, a.req(0, "image")?)),
        "_on_toggle_image_lock" => call!("image_lock", (win,)),
        "_on_image_size" => call!("image_measure", (win, true)),
        "_on_image_opacity" => call!("image_measure", (win, false)),
        "_single_textured_face" => call!("textured_face", (viewport(win)?,)),
        "_on_texture_position" => call!("edit_texture", (win, false)),
        "_on_texture_reset" => call!("edit_texture", (win, true)),
        "_selected_text3d" => call!("selected_text", (scene(win)?,)),
        "_on_edit_3d_text" => call!("edit_text", (win, a.or_none(0, "group")?)),
        "_assign_new_layer" => {
            let sc = scene(win)?;
            let (w, at) = (win.clone().unbind(), sc.clone().unbind());
            let assign = func(py, move |py, args| {
                let (win, answer) = (w.bind(py), args.get_item(0)?);
                if scene(win)?.is(at.bind(py)) && answer.is_truthy()? {
                    let name = answer.call_method0("strip")?;
                    if name.is_truthy()? {
                        native(py)?.call_method1("assign_tag", (win, name))?;
                    }
                }
                Ok(none(py))
            })?;
            let text = n.call_method1("next_tag", (&sc,))?;
            let kw = kwargs(py, &[("text", text), ("done", assign.into_bound(py))])?;
            done!(py.import("built_render.chrome_forms")?.getattr("ask_text")?.call((win, tr(py, "New layer")?, tr(py, "Layer name:")?), Some(&kw)))
        }
        // Right-click ▸ Change Axes: three clicks give the group or component new local axes.
        "_on_change_axes" => {
            let tool = win.getattr("_tools")?.get_item("change_axes")?;
            tool.setattr("target", a.req(0, "group")?)?;
            win.call_method1("_activate_tool", ("change_axes",))?;
            done!(viewport(win)?.call_method1("flash_status", (tr(py, "Click the new origin")?, 4000)))
        }
        // Right-click ▸ Flip Along ▸ Red / Green / Blue.
        "_on_flip_along" => done!(py.import("tools.flip")?.getattr("FlipTool")?.call0()?.call_method1("flip", (viewport(win)?, a.req(0, "axis")?))),
        // A link was broken or brought back: watch the sources still linked, update the rows.
        "_unlinked" => {
            viewport(win)?.call_method0("notify_scene_changed")?;
            done!(win.call_method0("_link_watcher")?.call_method0("refresh"))
        }
        "_refresh_scene_tabs" => {
            if let Some(view) = opt(&win.getattr("_doc")?, "stand_in") {
                view.setattr("_ui_tabs", py.None())?;
            }
            nothing()
        }

        // ---- 3D mouse, projection, tools, sheets ------------------------------------------
        "_on_ndof_motion" => {
            // Only the window being worked in moves (New Window makes several).
            if !win.call_method0("isActiveWindow")?.is_truthy()? {
                return nothing();
            }
            let vp = viewport(win)?;
            let settings = py.import("views.ndof_input")?.call_method0("current_settings")?;
            let moved = py.import("built_render.ndof")?.call_method1("apply_ndof", (vp.getattr("camera")?, a.req(0, "sample")?, a.req(1, "dt")?, vp.call_method0("height")?, settings))?;
            if moved.is_truthy()? {
                vp.call_method0("update")?;
            }
            nothing()
        }
        "_on_ndof_button" => {
            // Both buttons fit the model: the usual default for the right one.
            let number: i64 = a.req(0, "number")?.extract()?;
            if a.req(1, "down")?.is_truthy()? && (number == 0 || number == 1) && win.call_method0("isActiveWindow")?.is_truthy()? {
                win.call_method0("_on_zoom_extents")?;
            }
            nothing()
        }
        // Shift+P, in the active view only.
        "_toggle_projection" => {
            let vp = win.call_method0("_active_view")?;
            vp.call_method0("toggle_projection")?;
            nothing()
        }
        // Back to Select once a picture is placed, so the next click doesn't stamp a copy.
        "activate_select_tool" => done!(win.call_method1("_activate_tool", ("select",))),
        "_ensure_composer" => py.import("built_render.sheet_workspace")?.call_method1("composer_for", (win,)),
        "_on_open_composer" => done!(win.call_method0("_ensure_composer")?.call_method0("show")),
        "_show_sheet" => {
            let index = a.req(0, "index")?;
            win.setattr("_last_sheet", &index)?;
            win.call_method0("_ensure_composer")?.call_method1("show_sheet", (index,))?;
            nothing()
        }

        // ---- camera ---------------------------------------------------------------------
        "set_field_of_view" => call!("lens_angle", (win, win.call_method0("_active_view")?, a.req(0, "degrees")?)),
        "_ask_field_of_view" => {
            let view = win.call_method0("_active_view")?;
            let cam = view.getattr("camera")?;
            if !cam.getattr("perspective")?.is_truthy()? {
                return done!(n.call_method1("lens_angle", (win, &view, cam.getattr("fov_deg")?)));
            }
            let at = view.getattr("scene")?;
            let (w, v, sc) = (win.clone().unbind(), view.clone().unbind(), at.unbind());
            let apply = func(py, move |py, args| {
                let (win, view, value) = (w.bind(py), v.bind(py), args.get_item(0)?);
                if !value.is_none() && win.call_method0("_active_view")?.is(view) && view.getattr("scene")?.is(sc.bind(py)) {
                    native(py)?.call_method1("lens_angle", (win, view, value))?;
                }
                Ok(none(py))
            })?;
            let range = py.import("core.camera")?.getattr("FOV_RANGE")?;
            let kw = kwargs(py, &[("done", apply.into_bound(py)), ("suffix", s(py, "°"))])?;
            done!(py.import("built_render.chrome_forms")?.getattr("ask_number")?.call(
                (win, tr(py, "Field of view")?, tr(py, "Field of view in degrees (1–120):")?, cam.getattr("fov_deg")?, range.get_item(0)?, range.get_item(1)?, 1),
                Some(&kw),
            ))
        }
        "_camera_pose" => {
            let cam = viewport(win)?.getattr("camera")?;
            let t = cam.getattr("target")?;
            Ok(PyTuple::new(py, [t.call_method0("x")?, t.call_method0("y")?, t.call_method0("z")?, cam.getattr("distance")?, cam.getattr("yaw")?, cam.getattr("pitch")?])?.into_any())
        }
        "edit_scene_camera" => {
            let view = a.req(0, "view")?;
            if view.is_none() {
                return nothing();
            }
            win.getattr("tray")?.getattr("scenes")?.call_method1("show_view", (&view,))?;
            call!("open_camera", (win, view))
        }
        "_camera_dict" => {
            let vp = a.or_none(0, "vp")?;
            call!("camera_record", (if vp.is_truthy()? { vp } else { viewport(win)? },))
        }
        "_apply_camera_dict" => done!(call!("apply_camera_record", (viewport(win)?, a.req(0, "home")?))),
        "_apply_camera_home" => {
            let home = scene(win)?.getattr("camera_home").unwrap_or_else(|_| py.None().into_bound(py));
            done!(win.call_method1("_apply_camera_dict", (home,)))
        }
        "_reset_camera_home" => done!(call!("reset_home", (viewport(win)?,))),

        // ---- components and sections ------------------------------------------------------
        "keep_component" => done!(call!("retain_components", (win, a.or(1, "keep", true)?, a.req(0, "definition")?))),
        "keep_selected_components" => done!(call!("retain_components", (win, a.or(0, "keep", true)?))),
        "rename_component" => {
            let definition = a.req(0, "definition")?;
            let name = definition.getattr("name")?;
            let text = if name.is_truthy()? { name } else { s(py, "") };
            let done = py.import("functools")?.getattr("partial")?.call1((n.getattr("name_definition")?, win, scene(win)?, &definition))?;
            let kw = kwargs(py, &[("text", text), ("done", done)])?;
            done!(py.import("built_render.chrome_forms")?.getattr("ask_text")?.call((win, tr(py, "Rename Component")?, tr(py, "Component name:")?), Some(&kw)))
        }
        "prompt_section_name" => {
            let plane = a.req(0, "plane")?;
            let automatic = a.kw_or("automatic", true)?.is_truthy()?;
            let asked = py.import("built_core.settings")?.call_method0("settings")?.call_method1("value", ("section/ask_name", "1"))?;
            if automatic && asked.str()?.to_string() == "0" {
                return nothing();
            }
            let forms = py.import("built_render.chrome_forms")?;
            let model = n.getattr("OperationForm")?.call_method1("section", (plane.getattr("name")?, plane.getattr("symbol")?, automatic))?;
            let done = py.import("functools")?.getattr("partial")?.call1((n.getattr("name_section")?, win, scene(win)?, &plane))?;
            let form = forms.call_method1("OperationForm", (tr(py, "Name Section Plane")?, model, done))?;
            done!(forms.call_method1("show", (win, form)))
        }
        "_on_soften_edges" => {
            let view = viewport(win)?;
            let sc = view.getattr("scene")?;
            if !sc.getattr("selection")?.is_truthy()? {
                return done!(view.call_method1("flash_status", (tr(py, "Select faces, edges or groups to soften")?, 4000)));
            }
            let saved = py.import("built_core.settings")?.call_method0("settings")?.call_method1("value", ("tools/soften_angle", 30.0))?;
            let (w, v, at) = (win.clone().unbind(), view.clone().unbind(), sc.unbind());
            let apply = func(py, move |py, args| {
                let (win, view) = (w.bind(py), v.bind(py));
                if win.getattr("viewport")?.is(view) && view.getattr("scene")?.is(at.bind(py)) {
                    return Ok(native(py)?.call_method1("soften_selection", (view, args.get_item(0)?))?.unbind());
                }
                Ok(none(py))
            })?;
            let forms = py.import("built_render.chrome_forms")?;
            let model = n.getattr("OperationForm")?.call_method1("soften", (saved.str()?,))?;
            let note = tr(py, "Edges between faces at this angle or less become smooth; sharper edges stay hard. Selected groups include their nested geometry, and component copies share the change.")?;
            let kw = kwargs(py, &[("note", s(py, &note))])?;
            let form = forms.getattr("OperationForm")?.call((tr(py, "Soften / Smooth Edges")?, model, apply), Some(&kw))?;
            done!(forms.call_method1("show", (win, form)))
        }
        // Open the terrain chart in the chrome's bottom bar.
        "_on_terrain_profile" => {
            let view = opt(&win.getattr("_doc")?, "stand_in");
            match view.filter(|v| opt(v, "renderer").is_some()) {
                Some(view) => done!(call!("inspector_action", (win, view, "[\"terrain.open\",null]"))),
                None => {
                    win.call_method0("statusBar")?.call_method1("showMessage", (tr(py, "The terrain profile needs an active renderer.")?,))?;
                    nothing()
                }
            }
        }

        // ---- help -----------------------------------------------------------------------
        "_on_documentation" => done!(py.import("built_core.desktop_open")?.call_method1("open_url", ("https://ingetrazo-docs.pages.dev/",))),
        "_on_get_models" => {
            let text = format!(
                "{}<br><br><b>Poly Haven</b> — <a href='https://polyhaven.com'>polyhaven.com</a> {}<br><b>ambientCG</b> — <a href='https://ambientcg.com'>ambientcg.com</a> {}<br><b>Sketchfab</b> — <a href='https://sketchfab.com'>sketchfab.com</a> {}",
                tr(py, "Free sources that open directly in Built:")?,
                tr(py, "(CC0: models OBJ and PBR textures)")?,
                tr(py, "(CC0 textures — drop the PNG into resources/textures)")?,
                tr(py, "(filter by CC licence, download OBJ)")?,
            );
            done!(py.import("built_render.chrome_dialogs")?.call_method1("information", (win, tr(py, "Get more models and textures")?, text)))
        }
        "_on_check_updates" | "check_updates_quietly" => {
            let kw = kwargs(py, &[("quiet", obj(py, name == "check_updates_quietly")?.into_bound(py))])?;
            done!(py.import("built_render")?.getattr("start_check")?.call((win,), Some(&kw)))
        }
        "open_preferences" => {
            let kw = kwargs(py, &[("page", a.or_none(1, "page")?), ("shortcut_of", a.or_none(0, "shortcut_of")?)])?;
            done!(n.getattr("settings_sheet")?.getattr("show")?.call((win,), Some(&kw)))
        }
        "_on_units" => {
            let kw = kwargs(py, &[("page", s(py, "units"))])?;
            done!(win.getattr("open_preferences")?.call((), Some(&kw)))
        }

        // ---- documents, history, recovery -------------------------------------------------
        "_doc_is_blank" => {
            let doc = a.or_none(0, "doc")?;
            call!("document_blank", (if doc.is_truthy()? { doc } else { win.getattr("_doc")? },))
        }
        "_free_untitled_no" => call!("untitled_slot", (win.getattr("_documents")?,)),
        "_is_dirty" => call!("document_dirty", (win,)),
        "_on_undo" => done!(call!("history_step", (win, false))),
        "_on_redo" => done!(call!("history_step", (win, true))),
        "_figure_bounds" => call!("figure_bounds", (scene(win)?,)),
        "_image_bounds" => call!("image_bounds", (scene(win)?,)),
        "_extents_bounds" => call!("extents_bounds", (win,)),
        "_parse_skp_threaded" => {
            let kw = kwargs(py, &[("insert", obj(py, true)?.into_bound(py))])?;
            let opened = py.import("built_core")?.getattr("DocumentOpen")?.call((a.req(0, "skp")?,), Some(&kw)).and_then(|o| o.call_method0("wait"));
            Ok(match opened {
                Ok(doc) => PyTuple::new(py, [doc, py.None().into_bound(py)])?.into_any(),
                Err(e) if e.is_instance_of::<pyo3::exceptions::PyException>(py) => PyTuple::new(py, [py.None().into_bound(py), e.value(py).clone().into_any()])?.into_any(),
                Err(e) => return Err(e),
            })
        }
        // A new or opened document: the UTM readout of the one before goes.
        "_forget_coordinate" => {
            let vp = viewport(win)?;
            vp.setattr("_last_coordinate", "")?;
            vp.setattr("_last_coordinate_full", "")?;
            nothing()
        }
        "_configure_recovery" => done!(call!("configure_recovery", (win, a.req(0, "timer")?))),
        "_on_autosave_tick" => done!(call!("request_recovery", (win,))),
        "_on_toggle_eyedropper" => done!(call!("toggle_eyedropper", (win, a.req(0, "on")?))),
        "_remember_recent" => done!(call!("remember_document", (win, a.req(0, "path")?, a.or_none(1, "doc")?))),
        "_tool_key" => call!("tool_key", (win.getattr("_tools")?, a.req(0, "tool")?)),
        "_link_watcher" => call!("link_watcher", (win,)),
        "_after_links_loaded" => done!(call!("links_loaded", (win,))),
        "_apply_new_document_units" => {
            let units = py.import("core.units")?;
            done!(units.call_method1("apply_units", (scene(win)?, units.call_method0("new_document_units")?)))
        }
        "_on_new" => done!(call!("new_document_command", (win,))),
        "_on_new_tab" => done!(call!("new_document_command", (win, true))),
        "settle_heap" => done!(call!("settle_document_heap", (win.getattr("GC_THRESHOLD0")?,))),
        "_start_blank_document" => done!(call!("start_blank_document", (win,))),
        "_offer_untitled_recovery" => done!(call!("offer_untitled_recovery", (win,))),
        "_recover_untitled" => done!(call!("recover_untitled", (win, a.req(0, "n")?, a.req(1, "slot")?))),
        "_on_recover_discarded" => done!(call!("recover_discarded", (win,))),
        "_on_open_cancelled" => {
            if win.getattr("_open_in_progress")?.is_truthy()? {
                win.setattr("_open_cancelled", true)?;
            }
            nothing()
        }
        "_finish_cancelled_open" => done!(call!("reset_cancelled_open", (win,))),
        "_end_group_edits" => {
            viewport(win)?.call_method0("end_group_edit")?;
            done!(win.getattr("_doc")?.getattr("viewport")?.call_method0("end_group_edit"))
        }
        "_confirm_discard" => call!("confirm_discard", (win, a.req(0, "prompt")?)),
        "_confirm_quit" => call!("confirm_quit", (win,)),
        "_save_all_or_stay" => call!("save_selected", (win, a.req(0, "dirty")?, a.req(1, "answer")?)),

        // ---- selection, visibility, edits -------------------------------------------------
        "_on_select_all" => done!(call!("edit_selection", (win, "all"))),
        "_on_invert_selection" => done!(call!("edit_selection", (win, "invert"))),
        "_on_select_none" => done!(call!("edit_selection", (win, "none"))),
        "_on_delete_selection" => done!(call!("delete_entities", (viewport(win)?,))),
        "_on_paste_in_place" => done!(call!("paste_model", (win, true))),
        "_on_paste" => done!(call!("paste_model", (win, false))),
        "_hideable" => call!("hideable", (a.req(0, "entities")?,)),
        "_bridgeable" => call!("bridgeable", (scene(win)?.getattr("mesh")?, a.req(0, "sel")?)),
        "_insert_scale_figure" => done!(call!("insert_scale_figure", (win,))),
        "_make_billboard_person" => call!("billboard_person", (a.or(0, "image", "person_billboard.png")?, a.or(1, "height", 1.75)?, a.or_none(2, "name")?, a.or_none(3, "position")?)),
        "_shown_scene_index" => call!("marked_scene_index", (win,)),
        "play_scenes" => done!(call!("play_scenes", (win, a.or(0, "on", true)?))),
        "step_scene" => done!(call!("step_scene", (win, a.req(0, "step")?))),
        "set_scene_transitions" => done!(call!("scene_transitions", (win, a.req(0, "on")?))),
        "_no_scene_selected" => {
            status(win, &tr(py, "Select a scene in the Scenes panel first")?, 3000)?;
            nothing()
        }
        "_can_make_here" => call!("can_make_here", (viewport(win)?,)),
        "_execute_make" => done!(call!("execute_make", (viewport(win)?, a.req(0, "cmd")?))),
        "_on_make_group" => done!(call!("make_group", (win,))),
        "_on_make_component" => done!(call!("make_component", (win,))),
        "_on_explode_group" => done!(call!("explode_group", (win,))),
        "_on_explode_image" => done!(call!("explode_images", (viewport(win)?,))),
        "_on_hide" => done!(call!("change_visibility", (win, "hide"))),
        "_on_unhide_selected" => done!(call!("change_visibility", (win, "selected"))),
        "_on_unhide_last" => done!(call!("change_visibility", (win, "last"))),
        "_on_unhide_all" => done!(call!("change_visibility", (win, "all"))),
        "_hidden_everywhere" => call!("hidden_everywhere", (scene(win)?,)),
        "_on_toggle_hide_rest" => done!(call!("toggle_edit_visibility", (win, false))),
        "_on_toggle_hide_similar" => done!(call!("toggle_edit_visibility", (win, true))),
        "repeat_last_command" => call!("repeat_last_command", (win,)),
        "_on_reverse_faces" => done!(call!("face_winding", (win, false))),
        "_on_orient_faces" => done!(call!("face_winding", (win, true))),
        "_on_heal_overlaps" => done!(call!("heal_overlaps", (win,))),
        "_on_delete_guides" => done!(call!("delete_guides", (viewport(win)?,))),
        "_on_intersect_faces" => done!(call!("intersect_faces", (win, a.req(0, "mode")?))),
        "_on_convert_geopath" => done!(call!("convert_geopath", (win,))),
        "_on_bridge" => done!(call!("create_profile_faces", (win, true))),
        "_on_make_faces" => done!(call!("create_profile_faces", (win, false))),
        "_on_rebuild_planar" => done!(call!("rebuild_faces", (win,))),
        "_on_simplify_mesh" => done!(call!("ask_simplify_mesh", (win,))),
        "_on_divide" => done!(call!("divide_selected", (win,))),
        "_on_split_into_pieces" => done!(call!("split_pieces", (win,))),
        "_on_join_segments" => done!(call!("join_segments", (win,))),
        "_replacement_choices" => call!("replacement_choices", (scene(win)?, a.req(0, "selection")?)),
        "_on_replace_component" => done!(call!("choose_replacement", (win,))),
        "replace_selection_with" => call!("replace_selection", (win, a.req(0, "source")?)),
        "_ask_keep_orphans" => call!("ask_keep_orphans", (win, a.req(0, "orphans")?)),

        // ---- export, import, links -----------------------------------------------------------
        "export_video" => done!(crate::shell_window::video::run_export(win)),
        "ask_animation" => done!(crate::shell_window::playback::edit_animation(win)),
        "_on_export_view_svg" => py.import("built_render.view_export")?.call_method1("open_view_export", (win,)),
        "_view_line_groups" => py.import("built_render.view_export")?.call_method1("view_line_groups", (win.call_method0("_active_view")?, a.or(0, "fast", true)?)),
        "view_svg_builder" => py.import("built_render.view_export")?.call_method1("active_view_builder", (win,)),
        "_run_import" => call!("run_import", (win, a.req(0, "load")?, a.req(1, "dlg")?, a.req(2, "title")?, a.or_none(3, "describe")?, a.or_none(4, "command_type")?)),
        "_prepare_import_display" => done!(call!("prepare_import", (viewport(win)?, a.req(0, "cmd")?, a.req(1, "cb")?))),
        "_report_import" => done!(call!("report_import", (win, a.req(0, "name")?, a.or_none(1, "offset")?))),
        "_import_rfa_path" => call!("import_file", (win, a.req(0, "path")?, "rfa")),
        "_import_glb_path" => call!("import_file", (win, a.req(0, "path")?, "glb")),
        "_import_dae_path" => call!("import_file", (win, a.req(0, "path")?, "dae")),
        "_dxf_options" => call!("dxf_options", (win, a.or_none(1, "header_scale")?)),
        "_import_dxf_path" => {
            let kw = kwargs(py, &[("label", a.or_none(1, "label")?)])?;
            n.getattr("import_file")?.call((win, a.req(0, "path")?, "dxf"), Some(&kw))
        }
        "_apply_obj_import" => call!("import_file", (win, a.req(0, "path")?, "obj", a.req(1, "scale")?)),
        "_apply_stl_import" => call!("import_file", (win, a.req(0, "path")?, "stl", a.req(1, "scale")?, a.req(2, "simplify_mode")?)),
        "_start_place" => done!(call!("start_placement", (win, a.req(0, "group")?, a.or(1, "align_to_face", false)?, a.or_none(2, "anchor")?))),
        "import_igz_path" => call!("insert_document", (win, a.req(0, "path")?, a.or_none(1, "at")?)),
        "_on_insert_3d_text" => done!(py.import("built_render.font_picker")?.call_method1("text3d", (win, py.None(), win.getattr("_place_text3d")?))),
        "_place_text3d" => done!(call!("place_text", (win, a.req(0, "params")?))),
        "_on_import_image" => done!(call!("import_picture", (win,))),
        "_on_import_obj" => done!(call!("pick_mesh_file", (win, false))),
        "_on_import_stl" => done!(call!("pick_mesh_file", (win, true))),
        "_mesh_import_options" => call!("ask_mesh_units", (win, a.req(0, "path")?, a.kw_or("open_tab", py.None())?)),
        "_on_export_image" => done!(call!("choose_image_export", (win,))),
        "_export_image_to" => done!(call!("image_export_form", (win, a.req(0, "path")?))),
        "_save_export_image" => done!(call!("save_export_image", (win, a.req(0, "view")?, a.req(1, "path")?, a.req(2, "width")?, a.req(3, "transparent")?))),
        "import_skp_path" => call!("import_skp", (win, a.req(0, "path")?)),
        "_import_dwg_path" => call!("import_dwg", (win, a.req(0, "path")?)),
        "_on_clear_texture_cache" => done!(call!("clear_texture_cache", (win,))),
        "link_igz_path" => call!("link_document", (win, a.req(0, "path")?, a.or_none(1, "at")?, a.or_none(2, "select")?, a.kw_or("include_nested", true)?)),
        "_relink_done" => done!(call!("link_changed", (win, a.req(0, "note")?))),
        "_reload_link" => done!(call!("reload_links", (win, a.req(0, "path")?))),
        "_reload_all_links" => done!(call!("reload_links", (win,))),
        "_set_link_nested" => call!("set_link_nested", (win, a.req(0, "group")?, a.req(1, "enabled")?)),
        "_reset_link_tags" => done!(call!("reset_link_tags", (win, a.req(0, "path")?))),
        "_pack_link" => done!(call!("pack_link", (win, a.req(0, "path")?, a.req(1, "on")?))),
        "_choose_link_contents" => done!(call!("choose_link_contents", (win, a.req(0, "path")?, a.or_none(1, "select")?))),

        // ---- the Library's components and figures ------------------------------------------
        "_on_insert_component" => done!(call!("place_bundled", (win, a.req(0, "key")?, a.or_none(1, "name")?))),
        "_build_bundled_component" => call!("bundled_component", (win, a.req(0, "key")?, a.req(1, "name")?)),
        "insert_library_component" => done!(call!("place_catalogue", (win, a.req(0, "entry")?, a.or_none(1, "obj")?))),
        "_build_library_component" => call!("catalogue_component", (win, a.req(0, "entry")?, a.or_none(1, "obj")?)),
        "_on_insert_person_2d" => done!(call!("place_figure", (win, a.or(0, "image", "person_billboard.png")?, a.or(1, "height", 1.75)?, a.or_none(2, "name")?))),
        "_on_insert_faceme_image" => done!(call!("choose_billboard", (win,))),
        _ => Err(pyo3::exceptions::PyAttributeError::new_err(format!("no window verb {name}"))),
    }
}

/// The verbs, with the Repeat label of those Shift+R replays and how many
/// arguments they keep.
const VERBS: &[(&str, Option<(&str, usize)>)] = &[
    ("_on_make_unique", Some(("Make Unique", 0))),
    ("_on_merge_groups", Some(("Merge Groups", 0))),
    ("_on_explode_curve", Some(("Explode Curve", 0))),
    ("_on_make_group", Some(("Make Group", 0))),
    ("_on_make_component", Some(("Make Component", 0))),
    ("_on_explode_group", Some(("Explode Group", 0))),
    ("_on_hide", Some(("Hide", 0))),
    ("_on_reverse_faces", Some(("Reverse Faces", 0))),
    ("_on_orient_faces", Some(("Orient Faces", 0))),
    ("_on_heal_overlaps", Some(("Heal Overlapping Faces", 0))),
    ("_on_delete_guides", Some(("Delete Guides", 0))),
    ("_on_intersect_faces", Some(("Intersect Faces", 1))),
    ("_on_convert_geopath", Some(("Convert Path to Geometry", 0))),
    ("_on_bridge", Some(("Bridge", 0))),
    ("_on_make_faces", Some(("Make Faces", 0))),
    ("_on_rebuild_planar", Some(("Rebuild Faces (Planar)", 0))),
    ("_on_divide", Some(("Divide", 0))),
    ("_on_split_into_pieces", Some(("Split into Pieces", 0))),
    ("_on_join_segments", Some(("Join Segments", 0))),
    ("_on_reverse_section", None),
    ("_on_toggle_active_section", None),
    ("_on_align_view_to_section", None),
    ("_on_align_view_to_face", None),
    ("_on_set_surface", None),
    ("_on_toggle_path_closed", None),
    ("_assign_layer", None),
    ("_unlock_group", None),
    ("_delete_image", None),
    ("_unlock_image", None),
    ("_on_toggle_image_lock", None),
    ("_on_image_size", None),
    ("_on_image_opacity", None),
    ("_single_textured_face", None),
    ("_on_texture_position", None),
    ("_on_texture_reset", None),
    ("_selected_text3d", None),
    ("_on_edit_3d_text", None),
    ("_assign_new_layer", None),
    ("_on_change_axes", None),
    ("_on_flip_along", None),
    ("_unlinked", None),
    ("_refresh_scene_tabs", None),
    ("_on_ndof_motion", None),
    ("_on_ndof_button", None),
    ("_toggle_projection", None),
    ("activate_select_tool", None),
    ("_ensure_composer", None),
    ("_on_open_composer", None),
    ("_show_sheet", None),
    ("set_field_of_view", None),
    ("_ask_field_of_view", None),
    ("_camera_pose", None),
    ("edit_scene_camera", None),
    ("_camera_dict", None),
    ("_apply_camera_dict", None),
    ("_apply_camera_home", None),
    ("_reset_camera_home", None),
    ("keep_component", None),
    ("keep_selected_components", None),
    ("rename_component", None),
    ("prompt_section_name", None),
    ("_on_soften_edges", None),
    ("_on_terrain_profile", None),
    ("_on_documentation", None),
    ("_on_get_models", None),
    ("_on_check_updates", None),
    ("check_updates_quietly", None),
    ("open_preferences", None),
    ("_on_units", None),
    ("_doc_is_blank", None),
    ("_free_untitled_no", None),
    ("_is_dirty", None),
    ("_on_undo", None),
    ("_on_redo", None),
    ("_figure_bounds", None),
    ("_image_bounds", None),
    ("_extents_bounds", None),
    ("_parse_skp_threaded", None),
    ("_forget_coordinate", None),
    ("_configure_recovery", None),
    ("_on_autosave_tick", None),
    ("_on_toggle_eyedropper", None),
    ("_remember_recent", None),
    ("_tool_key", None),
    ("_link_watcher", None),
    ("_after_links_loaded", None),
    ("_apply_new_document_units", None),
    ("_on_new", None),
    ("_on_new_tab", None),
    ("settle_heap", None),
    ("_start_blank_document", None),
    ("_offer_untitled_recovery", None),
    ("_recover_untitled", None),
    ("_on_recover_discarded", None),
    ("_on_open_cancelled", None),
    ("_finish_cancelled_open", None),
    ("_end_group_edits", None),
    ("_confirm_discard", None),
    ("_confirm_quit", None),
    ("_save_all_or_stay", None),
    ("_on_select_all", None),
    ("_on_invert_selection", None),
    ("_on_select_none", None),
    ("_on_delete_selection", None),
    ("_on_paste_in_place", None),
    ("_on_paste", None),
    ("_hideable", None),
    ("_bridgeable", None),
    ("_insert_scale_figure", None),
    ("_make_billboard_person", None),
    ("_shown_scene_index", None),
    ("play_scenes", None),
    ("step_scene", None),
    ("set_scene_transitions", None),
    ("_no_scene_selected", None),
    ("_can_make_here", None),
    ("_execute_make", None),
    ("_on_explode_image", None),
    ("_on_unhide_selected", None),
    ("_on_unhide_last", None),
    ("_on_unhide_all", None),
    ("_hidden_everywhere", None),
    ("_on_toggle_hide_rest", None),
    ("_on_toggle_hide_similar", None),
    ("repeat_last_command", None),
    ("_on_simplify_mesh", None),
    ("_replacement_choices", None),
    ("_on_replace_component", None),
    ("replace_selection_with", None),
    ("_ask_keep_orphans", None),
    ("export_video", None),
    ("ask_animation", None),
    ("_on_export_view_svg", None),
    ("_view_line_groups", None),
    ("view_svg_builder", None),
    ("_run_import", None),
    ("_prepare_import_display", None),
    ("_report_import", None),
    ("_import_rfa_path", None),
    ("_import_glb_path", None),
    ("_import_dae_path", None),
    ("_dxf_options", None),
    ("_import_dxf_path", None),
    ("_apply_obj_import", None),
    ("_apply_stl_import", None),
    ("_start_place", None),
    ("import_igz_path", None),
    ("_on_insert_3d_text", None),
    ("_place_text3d", None),
    ("_on_import_image", None),
    ("_on_import_obj", None),
    ("_on_import_stl", None),
    ("_mesh_import_options", None),
    ("_on_export_image", None),
    ("_export_image_to", None),
    ("_save_export_image", None),
    ("import_skp_path", None),
    ("_import_dwg_path", None),
    ("_on_clear_texture_cache", None),
    ("link_igz_path", None),
    ("_relink_done", None),
    ("_reload_link", None),
    ("_reload_all_links", None),
    ("_set_link_nested", None),
    ("_reset_link_tags", None),
    ("_pack_link", None),
    ("_choose_link_contents", None),
    ("_on_insert_component", None),
    ("_build_bundled_component", None),
    ("insert_library_component", None),
    ("_build_library_component", None),
    ("_on_insert_person_2d", None),
    ("_on_insert_faceme_image", None),
];

/// Two names for one verb.
const ALIASES: &[(&str, &str)] = &[("_on_hide_edges", "_on_hide"), ("_on_unhide_all_edges", "_on_unhide_all")];

/// The class the window classes inherit: the verbs, the window's front
/// view, its saved version and the constants.
fn commands_class<'py>(m: &Bound<'py, PyModule>) -> PyResult<Bound<'py, PyAny>> {
    let py = m.py();
    let n = m;
    let ns = PyDict::new(py);
    for (name, repeat) in VERBS {
        ns.set_item(*name, Bound::new(py, Verb { name, repeat: *repeat })?)?;
    }
    for (alias, name) in ALIASES {
        ns.set_item(*alias, ns.get_item(*name)?)?;
    }
    // Keep full collections off the pointer-move path in large documents.
    ns.set_item("GC_THRESHOLD0", 50_000)?;
    ns.set_item("_RECENT_MAX", 10)?;
    ns.set_item("SAVE_PICTURE_PX", 560)?;
    ns.set_item("_CAMERA_FIELDS", n.getattr("DOCUMENT_CAMERA_FIELDS")?)?;
    let staticmethod = py.import("builtins")?.getattr("staticmethod")?;
    ns.set_item("_survey_note", staticmethod.call1((n.getattr("survey_note")?,))?)?;
    ns.set_item("_dxf_failure_text", staticmethod.call1((n.getattr("dxf_failure")?,))?)?;
    let recent = func(py, |py, _| Ok(py.import("built_core.recent")?.getattr("menu")?.call((), Some(&kwargs(py, &[("limit", obj(py, 10)?.into_bound(py))])?))?.unbind()))?;
    ns.set_item("_recent_paths", staticmethod.call1((recent,))?)?;
    let makes_faces = func(py, |py, a| Ok(native(py)?.call_method1("makes_faces", (a.get_item(0)?,))?.unbind()))?;
    ns.set_item("_makes_faces", staticmethod.call1((makes_faces,))?)?;
    let property = py.import("builtins")?.getattr("property")?;
    // The front view: the one being worked in, else the document's own.
    let get_view = func(py, |py, a| {
        let doc = a.get_item(0)?.getattr("__dict__")?.call_method1("get", ("_doc",))?;
        if doc.is_none() {
            return Ok(none(py));
        }
        let front = doc.getattr("front_view")?;
        Ok(if front.is_none() { doc.getattr("viewport")?.unbind() } else { front.unbind() })
    })?;
    let set_view = func(py, |py, a| Ok(native(py)?.call_method1("set_front_view", (a.get_item(0)?, a.get_item(1)?))?.unbind()))?;
    ns.set_item("viewport", property.call1((get_view, set_view))?)?;
    let scene_tabs = func(py, |py, a| {
        let win = a.get_item(0)?;
        let vp = win.getattr("viewport")?;
        if vp.is_none() {
            return Ok(none(py));
        }
        Ok(win.getattr("_doc")?.getattr("scene_mark")?.call_method1("for_scene", (vp.getattr("scene")?,))?.unbind())
    })?;
    ns.set_item("_scene_tabs", property.call1((scene_tabs,))?)?;
    let get_saved = func(py, |_py, a| Ok(a.get_item(0)?.getattr("_saved_content")?.unbind()))?;
    let set_saved = func(py, |py, a| {
        let win = a.get_item(0)?;
        let doc = win.getattr("__dict__")?.call_method1("get", ("_doc",))?;
        win.setattr("_saved_content", native(py)?.call_method1("saved_content", (doc, a.get_item(1)?))?)?;
        Ok(none(py))
    })?;
    ns.set_item("_saved_version", property.call1((get_saved, set_saved))?)?;
    let window = py.get_type::<crate::shell_window::ShellWindow>();
    for (name, value) in ns.iter() {
        window.setattr(name.cast::<pyo3::types::PyString>()?, value)?;
    }
    Ok(window.into_any())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<crate::shell_window::ShellScenes>()?;
    m.add("ShellWindow", commands_class(m)?)?;
    Ok(())
}
