//! Sheet editing, page synchronization and input for one document.
use crate::document_registration::view::calls;
use super::tools::{mode, ANNOT_FIELDS, CONTENT_FIELDS, RAIL, SNAP_TOOLS, WINDOW_KEYS};
use super::Obj;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet, PyTuple},
};

fn export_spec(kind: &str) -> Option<(&'static str, &'static str, &'static str, &'static str)> {
    Some(match kind {
        "pdf" => ("Export PDF…", ".pdf", "PDF (*.pdf)", "Exporting PDF"),
        "pdf_all" => (
            "Export all sheets (PDF)…",
            ".pdf",
            "PDF (*.pdf)",
            "Exporting the sheets",
        ),
        "png" => (
            "Export image…",
            ".png",
            "PNG (*.png);;JPEG (*.jpg *.jpeg)",
            "Exporting image",
        ),
        "dxf" => (
            "Export sheet as DXF…",
            ".dxf",
            "DXF (*.dxf)",
            "Exporting DXF",
        ),
        "svg" => (
            "Export sheet as SVG…",
            ".svg",
            "SVG (*.svg)",
            "Exporting SVG",
        ),
        _ => return None,
    })
}
fn enum_int(v: &Obj<'_>) -> PyResult<i64> {
    v.getattr("value").unwrap_or_else(|_| v.clone()).extract()
}
fn key_mods(event: &Obj<'_>) -> PyResult<(i64, i64)> {
    Ok((
        enum_int(&event.call_method0("key")?)?,
        enum_int(&event.call_method0("modifiers")?)?,
    ))
}
fn host_window<'py>(s: &Bound<'py, Workspace>) -> PyResult<Obj<'py>> {
    let w = s.getattr("composer")?.getattr("_window")?;
    Ok(w.getattr("_host_window").unwrap_or(w))
}
/// Sheet gestures arrive as JSON arrays; native renderer queries return tuples.
fn point_xy(value: &Obj<'_>) -> PyResult<(f64, f64)> {
    let [x, y] = value.extract::<[f64; 2]>()?;
    Ok((x, y))
}
fn num(o: &Obj<'_>, k: &str) -> PyResult<f64> {
    o.getattr(k)?.extract()
}
fn select_item(s: &Bound<'_, Workspace>, item: &Obj<'_>) -> PyResult<()> {
    s.setattr("_page", s.py().None())?;
    s.getattr("view")?
        .getattr("renderer")?
        .call_method1("page_select", (vec![item.getattr("_doc_key")?],))?;
    Ok(())
}
fn add_item(s: &Bound<'_, Workspace>, comp: &Obj<'_>, item: &Obj<'_>) -> PyResult<()> {
    Workspace::_run(
        s,
        &s.py()
            .import("built_core.composition")?
            .call_method1("AddItemCommand", (comp, item))?,
    )?;
    s.setattr("_page", s.py().None())
}
fn partial<'py, T: IntoPyObject<'py>>(
    py: Python<'py>,
    f: &Obj<'py>,
    args: (T,),
) -> PyResult<Obj<'py>> {
    py.import("functools")?.call_method1("partial", (f, args.0))
}
fn ground<'py>(ws: &Bound<'py, Workspace>) -> PyResult<Obj<'py>> {
    let g = ws.getattr("_ground")?;
    if !g.is_none() {
        return Ok(g);
    }
    let g = ws
        .py()
        .import("built_render.sheet_frames")?
        .call_method1("Ground", (ws.getattr("_touch")?, ws.getattr("view")?))?;
    ws.setattr("_ground", &g)?;
    Ok(g)
}
fn field_signature<'py>(frame: &Obj<'py>, fields: &[&str]) -> PyResult<Bound<'py, PyTuple>> {
    let mut values = Vec::new();
    for k in fields {
        values.push(
            frame
                .getattr(*k)
                .unwrap_or_else(|_| frame.py().None().into_bound(frame.py()))
                .repr()?,
        );
    }
    PyTuple::new(frame.py(), values)
}
fn alive(view: &Obj<'_>) -> PyResult<bool> {
    Ok(!view.is_none()
        && !view
            .getattr("_closed")
            .map_or(Ok(false), |x| x.is_truthy())?)
}
fn renderer<'py>(view: &Obj<'py>) -> Option<Obj<'py>> {
    view.getattr("renderer").ok().filter(|r| !r.is_none())
}
#[pyclass(dict, weakref, subclass, module = "built_render.sheet_workspace")]
#[derive(Default)]
pub struct Workspace {
    #[pyo3(get, set)]
    shown: bool,
    more_frames: bool,
    stale: bool,
    in_edit: bool,
    message: Option<(String, std::time::Instant)>,
}

#[pymethods]
impl Workspace {
    #[new]
    fn new(composer: &Obj<'_>) -> Self {
        let _ = composer;
        Self::default()
    }
    fn __init__(s: &Bound<'_, Self>, composer: &Obj<'_>) -> PyResult<()> {
        let py = s.py();
        let core = py.import("built_core")?;
        core.call_method0("sheet_forget_looks")?;
        core.call_method0("sheet_restore_looks")?;
        s.setattr("composer", composer)?;
        for key in [
            "view",
            "_page",
            "_ground",
            "_edit_after",
            "_followed",
            "_model_version",
            "_selection",
            "_pending",
            "_prompt",
        ] {
            s.setattr(key, py.None())?;
        }
        for key in [
            "_sent",
            "_content",
            "_snapped",
            "_annotated",
            "_anchor_memo",
            "_holds",
            "_tried",
        ] {
            s.setattr(key, PyDict::new(py))?;
        }
        *s.borrow_mut() = Self::default();
        s.setattr("_own_versions", PySet::empty(py)?)?;
        s.setattr("_actions", PyList::empty(py))?;
        s.setattr("_shell", !composer.getattr("_window")?.is_none())
    }
    fn _front_view(s: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let w = s.getattr("composer")?.getattr("_window")?;
        let doc = w
            .getattr("_host_doc")
            .ok()
            .filter(|d| d.is_truthy().unwrap_or(false))
            .or_else(|| w.getattr("_doc").ok());
        Ok(doc
            .and_then(|d| d.getattr("stand_in").ok())
            .map_or_else(|| s.py().None(), |v| v.unbind()))
    }
    fn enter(s: &Bound<'_, Self>) -> PyResult<()> {
        let py = s.py();
        let view = s.call_method0("_front_view")?;
        if s.borrow().shown {
            if view.is(s.getattr("view")?) {
                return Self::_touch(s);
            }
            Self::leave(s)?;
        }
        if !alive(&view)? {
            return Ok(());
        }
        s.borrow_mut().shown = true;
        s.setattr("view", &view)?;
        view.getattr("__dict__")?.set_item("_sheets_ws", s)?;
        if let Ok(tools) = view.getattr("tools") {
            let _ = tools.call_method0("cancel");
        }
        s.getattr("composer")?.call_method0("_adopt_document")?;
        s.call_method0("_auto_render_stale")?;
        for key in ["_sent", "_content", "_snapped", "_annotated", "_tried"] {
            s.setattr(key, PyDict::new(py))?;
        }
        s.setattr("_page", py.None())?;
        s.setattr("_selection", py.None())?;
        if let Some(r) = renderer(&view).filter(|r| r.hasattr("ui_show_sheets").unwrap_or(false)) {
            r.call_method1("ui_show_sheets", (true,))?;
        }
        Self::_reset_chrome(&view)?;
        Self::_touch(s)
    }
    fn leave(s: &Bound<'_, Self>) -> PyResult<()> {
        if !s.borrow().shown {
            return Ok(());
        }
        s.borrow_mut().shown = false;
        let view = s.getattr("view")?;
        s.setattr("view", s.py().None())?;
        if alive(&view)? {
            view.getattr("__dict__")?
                .call_method1("pop", ("_sheets_ws", s.py().None()))?;
            if let Some(r) =
                renderer(&view).filter(|r| r.hasattr("ui_show_sheets").unwrap_or(false))
            {
                r.call_method1("ui_show_sheets", (false,))?;
                r.call_method0("page_clear")?;
            }
            Self::_reset_chrome(&view)?;
            if let Ok(tools) = view.getattr("tools") {
                let _ = tools.call_method0("_apply_tool_cursor");
            }
            view.call_method0("update")?;
        }
        Ok(())
    }
    fn detach(s: &Bound<'_, Self>) -> PyResult<()> {
        Self::leave(s)
    }
    #[staticmethod]
    fn _reset_chrome(view: &Obj<'_>) -> PyResult<()> {
        let vars = view.getattr("__dict__")?;
        for k in ["_ui_tabs", "_ui_cluster_sig", "_ui_opts", "_ui_jobs_sig"] {
            vars.set_item(k, view.py().None())?;
        }
        vars.set_item("_ui_sheet_sig", false)
    }
    fn _touch(s: &Bound<'_, Self>) -> PyResult<()> {
        let view = s.getattr("view")?;
        if alive(&view)? {
            view.call_method0("update")?;
        }
        Ok(())
    }
    #[pyo3(signature=(text,msec=4000))]
    fn flash(s: &Bound<'_, Self>, text: &str, msec: i64) -> PyResult<()> {
        let until = std::time::Instant::now()
            .checked_add(std::time::Duration::from_millis(msec.max(0) as u64));
        s.borrow_mut().message = until.map(|expiry| (text.to_owned(), expiry));
        Self::_touch(s)
    }
    fn _passing(s: &Bound<'_, Self>) -> String {
        s.borrow()
            .message
            .as_ref()
            .filter(|(_, expiry)| std::time::Instant::now() < *expiry)
            .map(|(text, _)| text.clone())
            .unwrap_or_default()
    }
    fn timers_pending(s: &Bound<'_, Self>) -> PyResult<bool> {
        Ok(s.borrow().more_frames || !Self::_passing(s).is_empty())
    }
    fn _models(s: &Bound<'_, Self>) -> PyResult<Py<PyDict>> {
        let models = PyDict::new(s.py());
        let comp = s.getattr("composer")?.getattr("comp")?;
        for m in comp.call_method0("all_items")?.try_iter()? {
            let m = m?;
            models.set_item(m.getattr("_doc_key")?, m)?;
        }
        models.set_item(comp.getattr("_doc_key")?, comp)?;
        Ok(models.unbind())
    }
    fn _selected_models(s: &Bound<'_, Self>) -> PyResult<Py<PyList>> {
        let py = s.py();
        let out = PyList::empty(py);
        if let Some(r) = renderer(&s.getattr("view")?) {
            let models = Self::_models(s)?;
            for k in r.call_method0("page_selection")?.try_iter()? {
                if let Some(v) = models.bind(py).get_item(k?)? {
                    out.append(v)?;
                }
            }
        }
        Ok(out.unbind())
    }
    fn _run(s: &Bound<'_, Self>, inner: &Obj<'_>) -> PyResult<()> {
        s.call_method1(
            "_execute",
            (s.py()
                .import("built_core.history")?
                .call_method1("SheetCommand", (inner,))?,),
        )?;
        Ok(())
    }
    fn _forget_render(s: &Bound<'_, Self>, model: &Obj<'_>) -> PyResult<()> {
        if model.is_instance(&s.py().import("built_core.composition")?.getattr("MarcoVista")?)? {
            let key = model.getattr("_doc_key")?;
            for name in ["_content", "_tried", "_annotated"] {
                s.getattr(name)?
                    .call_method1("pop", (&key, s.py().None()))?;
            }
            if let Some(r) = renderer(&s.getattr("view")?) {
                r.call_method1("page_frame_forget", (key,))?;
            }
        }
        Ok(())
    }
    fn _refresh_all(s: &Bound<'_, Self>) -> PyResult<()> {
        s.borrow_mut().stale = false;
        for f in s
            .getattr("composer")?
            .getattr("comp")?
            .getattr("frames")?
            .try_iter()?
        {
            Self::_forget_render(s, &f?)?;
        }
        Self::_touch(s)
    }
    #[pyo3(signature=(redo=false))]
    fn _undo(s: &Bound<'_, Self>, redo: bool) -> PyResult<()> {
        let win = s.getattr("composer")?.getattr("_window")?;
        win.getattr("_host_window")
            .unwrap_or(win)
            .call_method0(if redo { "_on_redo" } else { "_on_undo" })?;
        Ok(())
    }
    #[pyo3(signature=(width_only=false))]
    fn fit(s: &Bound<'_, Self>, width_only: bool) -> PyResult<()> {
        let view = s.getattr("view")?;
        if !view.is_none() {
            view.getattr("renderer")?
                .call_method1("page_fit", (width_only,))?;
            Self::_touch(s)?;
        }
        Ok(())
    }
    fn _sheets(s: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        Ok(s.getattr("composer")?
            .call_method0("_scene")?
            .getattr("compositions")?
            .unbind())
    }
    fn _history(s: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        Ok(s.getattr("composer")?
            .getattr("_window")?
            .getattr("viewport")?
            .getattr("history")?
            .unbind())
    }
    fn sheet_changed(s: &Bound<'_, Self>) -> PyResult<()> {
        s.setattr("_page", s.py().None())?;
        s.getattr("_sent")?
            .call_method1("pop", ("list", s.py().None()))?;
        Self::_touch(s)
    }
    fn _show(s: &Bound<'_, Self>, index: isize) -> PyResult<()> {
        let last = Self::_sheets(s)?.bind(s.py()).len()? as isize - 1;
        s.getattr("composer")?
            .call_method1("_on_comp_switched", (index.min(last).max(0),))?;
        Self::sheet_changed(s)
    }
    fn _keep_a_sheet(s: &Bound<'_, Self>) -> PyResult<()> {
        let sheets = Self::_sheets(s)?.into_bound(s.py());
        if sheets.len()? > 0 && !sheets.contains(s.getattr("composer")?.getattr("comp")?)? {
            Self::_show(s, 0)?;
        }
        Ok(())
    }
    fn _execute(s: &Bound<'_, Self>, command: &Obj<'_>) -> PyResult<()> {
        let py = s.py();
        s.borrow_mut().in_edit = true;
        let result = Self::_history(s).and_then(|h| h.call_method1(py, "execute", (command,)));
        s.borrow_mut().in_edit = false;
        let versions = s.getattr("_own_versions")?;
        versions.call_method1(
            "add",
            (s.getattr("composer")?
                .call_method0("_scene")?
                .getattr("version")
                .ok(),),
        )?;
        if versions.len()? > 256 {
            let mut recent = Vec::new();
            for v in versions.try_iter()? {
                let v = v?;
                if !v.is_none() {
                    recent.push(v.extract::<u64>()?);
                }
            }
            recent.sort_unstable();
            s.setattr(
                "_own_versions",
                PySet::new(py, recent.iter().rev().take(128))?,
            )?;
        }
        result.map(|_| ())
    }
    fn model_changed(s: &Bound<'_, Self>, version: &Obj<'_>) -> PyResult<()> {
        let own = s.getattr("_own_versions")?;
        if s.borrow().in_edit {
            own.call_method1("add", (version,))?;
            return Ok(());
        }
        if own.contains(version)? || version.eq(s.getattr("_model_version")?)? {
            return Ok(());
        }
        s.setattr("_model_version", version)?;
        s.borrow_mut().stale = true;
        if s.borrow().shown {
            Self::_auto_render_stale(s)?;
        }
        Ok(())
    }
    fn _auto_render_stale(s: &Bound<'_, Self>) -> PyResult<()> {
        let comp = s.getattr("composer")?;
        if s.borrow().stale && comp.getattr("_auto_render")?.is_truthy()? {
            s.borrow_mut().stale = false;
            for f in comp.getattr("comp")?.getattr("frames")?.try_iter()? {
                let f = f?;
                if !f.getattr("style")?.eq("vectorial")?
                    || comp
                        .call_method1("frame_is_perspective", (&f,))?
                        .is_truthy()?
                {
                    Self::_forget_render(s, &f)?;
                }
            }
            Self::_touch(s)?;
        }
        Ok(())
    }
    fn set_auto_render(s: &Bound<'_, Self>, on: bool) -> PyResult<()> {
        s.getattr("composer")?.setattr("_auto_render", on)?;
        s.py()
            .import("built_core.settings")?
            .call_method0("settings")?
            .call_method1(
                "setValue",
                ("composer/auto_render", if on { "1" } else { "0" }),
            )?;
        Self::_auto_render_stale(s)
    }
    // Synchronize the page, rendering at most one uncached frame per turn.
    fn _send_page(s: &Bound<'_, Self>, renderer: &Obj<'_>) -> PyResult<()> {
        let py = s.py();
        let composer = s.getattr("composer")?;
        let ground = ground(s)?;
        let sheet = composer.getattr("comp")?;
        let profiles =
            ground.call_method1("profiles", (&sheet, composer.call_method0("_scene")?))?;
        let page = py.import("built_core")?.call_method1(
            "sheet_page_json",
            (sheet, profiles, s.call_method0("_choices")?),
        )?;
        if !page.eq(s.getattr("_page")?)? {
            s.setattr("_page", &page)?;
            renderer.call_method1("page_set", (page,))?;
        }
        let edit = s.getattr("_edit_after")?;
        s.setattr("_edit_after", py.None())?;
        if !edit.is_none() {
            renderer.call_method1("page_edit_text", (edit,))?;
        }
        Ok(())
    }
    fn _choices(s: &Bound<'_, Self>) -> PyResult<Py<PyDict>> {
        let py = s.py();
        let i18n = py.import("built_core.i18n")?;
        let tr = |label: &str| i18n.call_method1("tr", (label,))?.extract::<String>();
        let mut views = vec![vec!["__current__".to_string(), tr("Current view")?]];
        for v in py
            .import("built_core.composition")?
            .getattr("STD_VIEWS")?
            .try_iter()?
        {
            let (label, key): (String, String) = v?.extract()?;
            views.push(vec![format!("std:{key}"), tr(&label)?]);
        }
        for v in s
            .getattr("composer")?
            .call_method0("_scene")?
            .getattr("saved_views")?
            .try_iter()?
        {
            let name: String = v?.getattr("name")?.extract()?;
            let kw = PyDict::new(py);
            kw.set_item("name", &name)?;
            views.push(vec![
                format!("scene:{name}"),
                i18n.getattr("tr")?
                    .call(("Scene: {name}",), Some(&kw))?
                    .extract()?,
            ]);
        }
        let mut styles = vec![vec!["sombreado".to_string(), tr("Model style")?]];
        let style = py.import("built_core.style")?;
        for (items, translate) in [
            (style.getattr("BUILTIN_STYLES")?, true),
            (style.call_method0("user_styles")?, false),
        ] {
            for p in items.try_iter()? {
                let name: String = p?.getattr("name")?.extract()?;
                styles.push(vec![
                    format!("style:{name}"),
                    if translate { tr(&name)? } else { name },
                ]);
            }
        }
        styles.push(vec![
            "vectorial".to_string(),
            tr("Vector (hidden lines removed)")?,
        ]);
        let out = PyDict::new(py);
        out.set_item("views", views)?;
        out.set_item("styles", styles)?;
        Ok(out.unbind())
    }
    fn _send_contents(s: &Bound<'_, Self>, renderer: &Obj<'_>) -> PyResult<()> {
        let py = s.py();
        let comp = s.getattr("composer")?;
        let frames = py.import("built_render.sheet_frames")?;
        let content = s.getattr("_content")?;
        let tried = s.getattr("_tried")?;
        let page = s.getattr("_page")?;
        let view = s.getattr("view")?;
        s.borrow_mut().more_frames = false;
        let mut rendered = false;
        for f in comp.getattr("comp")?.getattr("frames")?.try_iter()? {
            let f = f?;
            let key = f.getattr("_doc_key")?;
            let sig = field_signature(&f, CONTENT_FIELDS)?;
            if !content.call_method1("get", (&key, &sig))?.eq(&sig)? {
                renderer.call_method1("page_frame_forget", (&key,))?;
                tried.call_method1("pop", (&key, py.None()))?;
                s.getattr("_annotated")?
                    .call_method1("pop", (&key, py.None()))?;
            }
            content.set_item(&key, sig)?;
            if renderer
                .call_method1("page_has_content", (&key,))?
                .is_truthy()?
                || tried.call_method1("get", (&key,))?.eq(&page)?
            {
                continue;
            }
            if rendered {
                s.borrow_mut().more_frames = true;
                continue;
            }
            tried.set_item(&key, &page)?;
            rendered = true;
            let result = (|| -> PyResult<()> {
                if f.getattr("style")?.eq("vectorial")?
                    && !comp
                        .call_method1("frame_is_perspective", (&f,))?
                        .is_truthy()?
                {
                    let drawing =
                        frames.call_method1("drawing", (view.getattr("source_viewport")?, &f))?;
                    if !drawing.is_none() {
                        let ops = py
                            .import("built_core")?
                            .call_method1("sheet_frame_display", (&f, drawing, true))?;
                        renderer.call_method1("page_frame_ops", (&key, ops))?;
                    }
                } else {
                    frames.call_method1("render", (&view, &f, &key))?;
                }
                Ok(())
            })();
            if let Err(err) = result {
                err.print(py);
            }
        }
        Ok(())
    }
    fn _follow_model(s: &Bound<'_, Self>) -> PyResult<()> {
        let py = s.py();
        let composer = s.getattr("composer")?;
        let comp = composer.getattr("comp")?;
        let scene = composer.call_method0("_scene")?;
        let mut frames = Vec::new();
        for f in comp.getattr("frames")?.try_iter()? {
            let f = f?;
            let mut fields = Vec::new();
            for k in [
                "uid",
                "x_mm",
                "y_mm",
                "w_mm",
                "h_mm",
                "scale_n",
                "view_key",
                "cam_yaw",
                "cam_pitch",
                "cam_target",
                "rot_deg",
                "perspective",
            ] {
                let v = f.getattr(k)?;
                fields.push(if k == "cam_target" {
                    if v.is_truthy()? {
                        py.get_type::<PyTuple>().call1((v,))?
                    } else {
                        PyTuple::empty(py).into_any()
                    }
                } else {
                    v
                });
            }
            frames.push(PyTuple::new(py, fields)?);
        }
        let sig = (
            comp.getattr("_doc_key")?,
            scene.getattr("version").ok(),
            PyTuple::new(py, frames)?,
        )
            .into_pyobject(py)?;
        if sig.eq(s.getattr("_followed")?)? {
            return Ok(());
        }
        s.setattr("_followed", sig)?;
        if let Err(e) = py.import("built_render.sheet_frames")?.call_method1(
            "follow_anchors",
            (
                s.getattr("view")?.getattr("source_viewport")?,
                comp,
                s.getattr("_anchor_memo")?,
                s.getattr("_holds")?,
            ),
        ) {
            e.print(py);
        }
        Ok(())
    }
    fn _send_annots(s: &Bound<'_, Self>, renderer: &Obj<'_>) -> PyResult<()> {
        let py = s.py();
        let comp = s.getattr("composer")?;
        let version = comp.call_method0("_scene")?.getattr("version").ok();
        let providers = comp
            .getattr("_window")?
            .getattr("_ext_sheet_annots")
            .ok()
            .filter(|v| !v.is_none());
        let n = providers
            .as_ref()
            .map(|v| v.len())
            .transpose()?
            .unwrap_or(0);
        let annotated = s.getattr("_annotated")?;
        for f in comp.getattr("comp")?.getattr("frames")?.try_iter()? {
            let f = f?;
            let key = f.getattr("_doc_key")?;
            let sig = (
                &version,
                s.getattr("_content")?.call_method1("get", (&key,))?,
                n,
                field_signature(&f, ANNOT_FIELDS)?,
            )
                .into_pyobject(py)?;
            if annotated.call_method1("get", (&key,))?.eq(&sig)? {
                continue;
            }
            annotated.set_item(&key, sig)?;
            let result = (|| -> PyResult<()> {
                let data = py.import("built_render.sheet_frames")?.call_method1(
                    "annotations",
                    (
                        s.getattr("view")?.getattr("source_viewport")?,
                        &f,
                        &providers,
                    ),
                )?;
                let annots = py
                    .import("built_core")?
                    .call_method1("sheet_frame_annots", (&f, data))?;
                renderer.call_method1("page_frame_annots", (key, annots))?;
                Ok(())
            })();
            if let Err(e) = result {
                e.print(py);
            }
            s.borrow_mut().more_frames = true;
            break;
        }
        Ok(())
    }
    fn _send_snaps(s: &Bound<'_, Self>, renderer: &Obj<'_>) -> PyResult<()> {
        let py = s.py();
        let comp = s.getattr("composer")?;
        let scene = comp.call_method0("_scene")?;
        let snapped = s.getattr("_snapped")?;
        for f in comp.getattr("comp")?.getattr("frames")?.try_iter()? {
            let f = f?;
            let key = f.getattr("_doc_key")?;
            let sig = (
                scene.getattr("version").ok(),
                s.getattr("_content")?.call_method1("get", (&key,))?,
                f.getattr("x_mm")?,
                f.getattr("y_mm")?,
            )
                .into_pyobject(py)?;
            if snapped.call_method1("get", (&key,))?.eq(&sig)? {
                continue;
            }
            snapped.set_item(&key, sig)?;
            let got = py.import("built_render.sheet_frames")?.call_method1(
                "snaps",
                (s.getattr("view")?.getattr("source_viewport")?, &f),
            );
            let mut args = vec![key];
            match got {
                Ok(data) if !data.is_none() => {
                    let np = py.import("numpy")?;
                    let dtype = py.import("builtins")?.getattr("float")?;
                    for a in data.try_iter()? {
                        args.push(
                            np.call_method1("ascontiguousarray", (a?, &dtype))?
                                .call_method1("reshape", (-1,))?
                                .call_method0("tolist")?,
                        );
                    }
                }
                other => {
                    if let Err(e) = other {
                        e.print(py);
                    }
                    for _ in 0..5 {
                        args.push(PyList::empty(py).into_any());
                    }
                }
            }
            renderer.call_method1("page_frame_snaps", PyTuple::new(py, args)?)?;
            s.borrow_mut().more_frames = true;
            break;
        }
        Ok(())
    }
    fn before_frame(s: &Bound<'_, Self>, view: &Obj<'_>) -> PyResult<()> {
        let r = view.getattr("renderer")?;
        Self::_keep_a_sheet(s)?;
        Self::_follow_model(s)?;
        Self::_send_page(s, &r)?;
        Self::_send_contents(s, &r)?;
        if !s.borrow().more_frames {
            Self::_send_annots(s, &r)?;
        }
        let comp = s.getattr("composer")?;
        let mode: String = comp.getattr("tool_mode")?.extract()?;
        if SNAP_TOOLS.contains(&mode.as_str()) && !s.borrow().more_frames {
            Self::_send_snaps(s, &r)?;
            let step = comp
                .call_method0("_scene")?
                .getattr("dimension_style")
                .ok()
                .filter(|v| !v.is_none())
                .and_then(|v| v.call_method1("get", ("base_step_mm", 8.0)).ok())
                .and_then(|v| {
                    v.call_method0("__float__")
                        .or_else(|_| s.py().import("builtins")?.call_method1("float", (v,)))
                        .ok()
                })
                .and_then(|v| v.extract::<f64>().ok())
                .filter(|v| *v > 0.1)
                .unwrap_or(8.);
            s.call_method1("_once", ("run_step", step, r.getattr("page_set_run_step")?))?;
        }
        s.call_method1("_send", (r, view))?;
        Ok(())
    }
    // All changes made by a gesture share one document history step.
    fn _edit(s: &Bound<'_, Self>, edits: &Obj<'_>) -> PyResult<()> {
        let py = s.py();
        let commands = PyList::empty(py);
        let composition = py.import("built_core.composition")?;
        for pair in edits.try_iter()? {
            let (model, input): (Obj, Bound<PyDict>) = pair?.extract()?;
            let fields = model.getattr("_record_fields")?;
            let changes = PyDict::new(py);
            for (k, v) in input.iter() {
                if fields.contains(&k)? {
                    if k.eq("cam_target")? && !v.is_none() {
                        changes.set_item(k, py.get_type::<PyTuple>().call1((v,))?)?;
                    } else {
                        changes.set_item(k, v)?;
                    }
                }
            }
            if !changes.is_empty() {
                commands.append(composition.call_method1("EditItemCommand", (&model, changes))?)?;
                Self::_forget_render(s, &model)?;
            }
        }
        if !commands.is_empty() {
            let command = if commands.len() == 1 {
                commands.get_item(0)?
            } else {
                composition.call_method1("CompoundCommand", (&commands,))?
            };
            Self::_run(s, &command)?;
            let core = py.import("built_core")?;
            for c in commands.iter() {
                core.call_method1(
                    "sheet_remember_look",
                    (
                        c.getattr("item")?,
                        py.get_type::<PyList>().call1((c.getattr("after")?,))?,
                    ),
                )?;
            }
        }
        Ok(())
    }
    fn _move(s: &Bound<'_, Self>, arg: &str) -> PyResult<()> {
        let py = s.py();
        let models = Self::_models(s)?.into_bound(py);
        let edits = PyList::empty(py);
        for part in arg.split(';').filter(|v| !v.is_empty()) {
            let fields: Vec<_> = part.split(':').collect();
            let [key, x, y, w, h] = fields.as_slice() else {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "invalid sheet rectangle",
                ));
            };
            let Some(model) = models.get_item(key.parse::<i64>()?)? else {
                continue;
            };
            if model
                .getattr("locked")
                .is_ok_and(|v| v.is_truthy().unwrap_or(false))
            {
                continue;
            }
            let changes = PyDict::new(py);
            let supported = model.getattr("_record_fields")?;
            for (name, raw) in [("x_mm", x), ("y_mm", y), ("w_mm", w), ("h_mm", h)] {
                let value = raw.parse::<f64>()?;
                if supported.contains(name)?
                    && (model.getattr(name)?.extract::<f64>()? - value).abs() > 1e-9
                {
                    changes.set_item(name, value)?;
                }
            }
            edits.append((model, changes))?;
        }
        Self::_edit(s, edits.as_any())
    }
    fn _lock_items(s: &Bound<'_, Self>) -> PyResult<()> {
        let py = s.py();
        let comp = s.getattr("composer")?.getattr("comp")?;
        let mut items = vec![];
        let mut lock = false;
        for item in Self::_selected_models(s)?.bind(py).iter() {
            if !item.is(&comp) {
                let locked = item.getattr("locked")?.is_truthy()?;
                lock |= !locked;
                items.push((item, locked));
            }
        }
        let edits = PyList::empty(py);
        for (item, locked) in items {
            if lock != locked {
                let d = PyDict::new(py);
                d.set_item("locked", lock)?;
                edits.append((item, d))?;
            }
        }
        Self::_edit(s, edits.as_any())
    }
    fn _delete(s: &Bound<'_, Self>) -> PyResult<()> {
        let py = s.py();
        let comp = s.getattr("composer")?.getattr("comp")?;
        let cmds = PyList::empty(py);
        let model = py.import("built_core.composition")?;
        for item in Self::_selected_models(s)?.bind(py).iter() {
            if !item.is(&comp)
                && !item
                    .getattr("locked")
                    .is_ok_and(|v| v.is_truthy().unwrap_or(false))
            {
                cmds.append(model.call_method1("RemoveItemCommand", (&comp, item))?)?;
            }
        }
        if !cmds.is_empty() {
            Self::_run(s, &model.call_method1("CompoundCommand", (cmds,))?)?;
        }
        Ok(())
    }
    fn _duplicate(s: &Bound<'_, Self>) -> PyResult<()> {
        let py = s.py();
        let comp = s.getattr("composer")?.getattr("comp")?;
        let sel = Self::_selected_models(s)?.into_bound(py);
        let items: Vec<_> = sel.iter().filter(|i| !i.is(&comp)).collect();
        let copies = py
            .import("built_core")?
            .call_method1("sheet_duplicates", (&comp, items))?;
        if copies.is_truthy()? {
            let cmds = PyList::empty(py);
            let keys = PyList::empty(py);
            let model = py.import("built_core.composition")?;
            for item in copies.try_iter()? {
                let item = item?;
                keys.append(item.getattr("_doc_key")?)?;
                cmds.append(model.call_method1("AddItemCommand", (&comp, item))?)?;
            }
            Self::_run(s, &model.call_method1("CompoundCommand", (cmds,))?)?;
            s.setattr("_page", py.None())?;
            s.getattr("view")?
                .getattr("renderer")?
                .call_method1("page_select", (keys,))?;
        }
        Ok(())
    }
    fn _label_at(s: &Bound<'_, Self>, x: f64, y: f64) -> PyResult<Option<Py<PyAny>>> {
        let labels = s
            .getattr("composer")?
            .getattr("comp")?
            .getattr("etiquetas")
            .ok()
            .filter(|v| !v.is_none());
        let core = s.py().import("built_core")?;
        let mut best = None;
        let mut best_z = f64::NEG_INFINITY;
        if let Some(labels) = labels {
            for item in labels.try_iter()? {
                let item = item?;
                if item
                    .getattr("locked")
                    .is_ok_and(|v| v.is_truthy().unwrap_or(false))
                {
                    continue;
                }
                let (w, h): (f64, f64) =
                    core.call_method1("sheet_item_size", (&item,))?.extract()?;
                let (x0, y0) = (num(&item, "x_mm")?, num(&item, "y_mm")?);
                if (x0 - 0.5..=x0 + w + 0.5).contains(&x) && (y0 - 0.5..=y0 + h + 0.5).contains(&y)
                {
                    let z = num(&item, "z")?;
                    if z >= best_z {
                        best_z = z;
                        best = Some(item.unbind());
                    }
                }
            }
        }
        Ok(best)
    }
    fn _frame_at(s: &Bound<'_, Self>, x: f64, y: f64) -> PyResult<Option<Py<PyAny>>> {
        let frames = s.getattr("composer")?.getattr("comp")?.getattr("frames")?;
        for i in (0..frames.len()?).rev() {
            let f = frames.get_item(i)?;
            let (x0, y0) = (num(&f, "x_mm")?, num(&f, "y_mm")?);
            if (x0..=x0 + num(&f, "w_mm")?).contains(&x)
                && (y0..=y0 + num(&f, "h_mm")?).contains(&y)
            {
                return Ok(Some(f.unbind()));
            }
        }
        Ok(None)
    }
    #[pyo3(signature=(x0,y0,x1,y1,tool="vista",hit=None))]
    fn _place(
        s: &Bound<'_, Self>,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        tool: &str,
        hit: Option<&Obj<'_>>,
    ) -> PyResult<()> {
        let py = s.py();
        let composer = s.getattr("composer")?;
        let comp = composer.getattr("comp")?;
        let scene = composer.call_method0("_scene")?;
        let frames = py.import("built_render.sheet_frames")?;
        let binding = match hit {
            Some(h) if h.call_method1("get", ("world",))?.is_truthy()? => Self::_models(s)?
                .bind(py)
                .get_item(h.call_method1("get", ("frame",))?)?
                .map(|f| (f, h)),
            _ => None,
        };
        if tool == "etiqueta" {
            if let Some(target) = Self::_label_at(s, x1, y1)? {
                let target = target.bind(py);
                let leader = PyDict::new(py);
                leader.set_item("ax_mm", x0 - num(target, "x_mm")?)?;
                leader.set_item("ay_mm", y0 - num(target, "y_mm")?)?;
                leader.set_item("anchor_uid", "")?;
                leader.set_item("a_world", py.None())?;
                if let Some((ref frame, h)) = binding {
                    leader.set_item("anchor_uid", frames.call_method1("frame_uid", (frame,))?)?;
                    leader.set_item(
                        "a_world",
                        py.get_type::<PyList>().call1((h.get_item("world")?,))?,
                    )?;
                }
                let old = PyList::empty(py);
                let leaders = target.getattr("leaders")?;
                if leaders.is_truthy()? {
                    for entry in leaders.try_iter()? {
                        old.append(py.get_type::<PyDict>().call1((entry?,))?)?;
                    }
                }
                let before = PyDict::new(py);
                before.set_item("leaders", &old)?;
                let after = PyDict::new(py);
                let updated = old.call_method0("copy")?;
                updated.call_method1("append", (leader,))?;
                after.set_item("leaders", updated)?;
                Self::_run(
                    s,
                    &py.import("built_core.composition")?
                        .call_method1("EditItemCommand", (target, after, before))?,
                )?;
                select_item(s, target)?;
                Self::_tool(s, "select")?;
                return Ok(());
            }
        }
        let template = if tool == "cajetin" {
            super::_cajetin_template(py)?
        } else {
            (String::new(), None)
        };
        let folder = (tool == "cajetin").then_some(template.0);
        let item = py.import("built_core")?.call_method1(
            "sheet_new_item",
            (&comp, tool, x0, y0, x1, y1, &scene, folder, template.1),
        )?;
        if item.is_none() {
            return Ok(());
        }
        if tool == "nivel" {
            if let Some(host) = Self::_frame_at(s, x0, y0)? {
                let z = frames.call_method1(
                    "level_at",
                    (s.getattr("view")?.getattr("source_viewport")?, host, x0, y0),
                )?;
                if !z.is_none() {
                    item.setattr("z_m", z)?;
                }
            }
        }
        if matches!(tool, "etiqueta" | "nivel") {
            if let Some((f, h)) = binding {
                frames.call_method1("anchor", (&item, &f, &scene, vec![("a_world", h)]))?;
                if tool == "nivel" {
                    let (x, y) = point_xy(&h.get_item("p")?)?;
                    item.setattr("x_mm", x)?;
                    item.setattr("y_mm", y)?;
                    if f.getattr("view_key")?.eq("std:top")? {
                        item.setattr("symbol", "circle")?;
                    }
                }
            }
        }
        add_item(s, &comp, &item)?;
        select_item(s, &item)?;
        if tool == "texto" {
            s.setattr("_edit_after", item.getattr("_doc_key")?)?;
        }
        Self::_tool(s, "select")
    }
    #[pyo3(signature=(body,total=false))]
    fn _cota(s: &Bound<'_, Self>, body: &Obj<'_>, total: bool) -> PyResult<()> {
        let py = s.py();
        let core = py.import("built_core")?;
        let composer = s.getattr("composer")?;
        let comp = composer.getattr("comp")?;
        let scene = composer.call_method0("_scene")?;
        let a = body.get_item("a")?;
        let b = body.get_item("b")?;
        let point =
            |o: &Obj<'_>, name: &str| -> PyResult<(f64, f64)> { point_xy(&o.get_item(name)?) };
        let axis = body.call_method1("get", ("axis", ""))?;
        let item = if total {
            core.call_method1(
                "sheet_new_chain_total",
                (
                    &comp,
                    point(&a, "p")?,
                    point(&b, "p")?,
                    body.get_item("sep")?,
                    axis,
                    point(body, "p1")?,
                    point(body, "p2")?,
                    &scene,
                ),
            )?
        } else {
            core.call_method1(
                "sheet_new_cota",
                (
                    &comp,
                    point(&a, "p")?,
                    point(&b, "p")?,
                    body.call_method1("get", ("sep", 0.0))?,
                    axis,
                    &scene,
                ),
            )?
        };
        let key = a.call_method1("get", ("frame",))?;
        if let Some(f) = Self::_models(s)?.bind(py).get_item(&key)? {
            if key.eq(b.call_method1("get", ("frame",))?)?
                && a.call_method1("get", ("world",))?.is_truthy()?
                && b.call_method1("get", ("world",))?.is_truthy()?
            {
                py.import("built_render.sheet_frames")?.call_method1(
                    "anchor",
                    (&item, &f, scene, vec![("a_world", a), ("b_world", b)]),
                )?;
                item.setattr("scale_n", f.getattr("scale_n")?)?;
            }
        }
        add_item(s, &comp, &item)
    }
    fn _dimension(s: &Bound<'_, Self>, kind: &str, body: &Obj<'_>) -> PyResult<()> {
        let composer = s.getattr("composer")?;
        let comp = composer.getattr("comp")?;
        let core = s.py().import("built_core")?;
        let point = |k: &str| -> PyResult<(f64, f64)> { point_xy(&body.get_item(k)?) };
        let item = if kind == "radial" {
            core.call_method1(
                "sheet_new_radial",
                (
                    &comp,
                    point("c")?,
                    point("p")?,
                    body.call_method1("get", ("kind", "radius"))?,
                    composer.call_method0("_scene")?,
                ),
            )?
        } else {
            core.call_method1(
                "sheet_new_angular",
                (
                    &comp,
                    point("v")?,
                    point("a")?,
                    point("b")?,
                    body.get_item("r")?,
                ),
            )?
        };
        if !item.is_none() {
            add_item(s, &comp, &item)?;
        }
        Ok(())
    }
    fn _tool(s: &Bound<'_, Self>, name: &str) -> PyResult<()> {
        s.getattr("composer")?.setattr(
            "tool_mode",
            if mode(name).is_some() { name } else { "select" },
        )?;
        Self::_touch(s)
    }
    fn _add_image(s: &Bound<'_, Self>) -> PyResult<()> {
        let py = s.py();
        let tr = py.import("built_core.i18n")?;
        let dialog = py
            .import("built_render.shell_files")?
            .getattr("ShellFileDialog")?;
        let path = dialog
            .call_method1(
                "getOpenFileName",
                (
                    py.None(),
                    tr.call_method1("tr", ("Choose image…",))?,
                    "",
                    tr.call_method1("tr", ("Images (*.png *.jpg *.jpeg)",))?,
                ),
            )?
            .get_item(0)?;
        Self::_tool(s, "select")?;
        if !path.is_truthy()? {
            return Ok(());
        }
        let comp = s.getattr("composer")?.getattr("comp")?;
        let item = py
            .import("built_core")?
            .call_method1("sheet_new_image", (&comp, path))?;
        if item.is_none() {
            Self::flash(s, "That file is not a picture", 4000)
        } else {
            add_item(s, &comp, &item)?;
            select_item(s, &item)
        }
    }
    fn _font(s: &Bound<'_, Self>, model: &Obj<'_>) -> PyResult<()> {
        if model.is_none() {
            return Ok(());
        }
        let py = s.py();
        let apply = partial(py, &s.getattr("_apply_font")?, (model,))?;
        py.import("built_render.font_picker")?.call_method1(
            "show",
            (
                s.getattr("view")?.call_method0("window")?,
                "Sheet text",
                model.getattr("text")?,
                model.getattr("family")?,
                model.getattr("bold")?,
                model.getattr("italic")?,
                model.getattr("size_pt")?,
                " pt",
                apply,
            ),
        )?;
        Ok(())
    }
    fn _apply_font(s: &Bound<'_, Self>, model: &Obj<'_>, params: &Obj<'_>) -> PyResult<()> {
        let d = PyDict::new(s.py());
        for (k, v) in [
            ("family", "font"),
            ("bold", "bold"),
            ("italic", "italic"),
            ("size_pt", "height"),
        ] {
            d.set_item(k, params.get_item(v)?)?;
        }
        let edits = PyList::new(s.py(), [(model, d)])?;
        Self::_edit(s, edits.as_any())?;
        Self::_touch(s)
    }
    // Dispatch page input after chrome and window shortcuts have had their turn.
    fn pointer(s: &Bound<'_, Self>, kind: &str, event: &Obj<'_>) -> PyResult<()> {
        let py = s.py();
        let view = s.getattr("view")?;
        if view.is_none() {
            return Ok(());
        }
        let r = view.getattr("renderer")?;
        if kind == "leave" || event.is_none() {
            r.call_method1("page_pointer", ("leave", -1., -1., 0, 0))?;
            return Ok(());
        }
        let codes = py.import("built_core")?;
        let double = codes.getattr("Type")?.getattr("MouseButtonDblClick")?;
        let kind =
            if kind == "press" && enum_int(&event.call_method0("type")?)? == enum_int(&double)? {
                "double"
            } else {
                kind
            };
        let p = event.call_method0("position")?;
        let shape = r.call_method1(
            "page_pointer",
            (
                kind,
                p.call_method0("x")?,
                p.call_method0("y")?,
                calls::pointer_button(&event.call_method0("button")?)?,
                calls::mods(event)?,
            ),
        )?;
        let name = match shape.extract::<String>()?.as_str() {
            "move" => "SizeAllCursor",
            "grab" => "OpenHandCursor",
            "grabbing" => "ClosedHandCursor",
            "crosshair" | "zoom" => "CrossCursor",
            "resize_h" => "SizeHorCursor",
            "resize_v" => "SizeVerCursor",
            "resize_nwse" => "SizeFDiagCursor",
            "resize_nesw" => "SizeBDiagCursor",
            _ => "ArrowCursor",
        };
        let cursor = codes.getattr("Input")?.getattr(name)?;
        if !view
            .getattr("__dict__")?
            .call_method1("get", ("_sheets_cursor",))?
            .eq(&cursor)?
        {
            view.setattr("_sheets_cursor", &cursor)?;
            view.call_method1("setCursor", (cursor,))?;
        }
        Self::_touch(s)
    }
    fn chrome_took(s: &Bound<'_, Self>) -> PyResult<()> {
        let view = s.getattr("view")?;
        if !view.is_none() {
            view.getattr("renderer")?
                .call_method1("page_pointer", ("leave", -1., -1., 0, 0))?;
            view.getattr("__dict__")?
                .call_method1("pop", ("_sheets_cursor", s.py().None()))?;
        }
        Ok(())
    }
    fn wheel(s: &Bound<'_, Self>, event: &Obj<'_>) -> PyResult<()> {
        let p = event.call_method0("position")?;
        let delta = event.call_method0("pixelDelta")?;
        let (d, k) = if delta.call_method0("isNull")?.is_truthy()? {
            (event.call_method0("angleDelta")?, 3. / 8.)
        } else {
            (delta, 1.)
        };
        let control = s
            .py()
            .import("built_core")?
            .getattr("Input")?
            .getattr("ControlModifier")?;
        let zoom = enum_int(&event.call_method0("modifiers")?)? & enum_int(&control)? != 0;
        s.getattr("view")?.getattr("renderer")?.call_method1(
            "page_wheel",
            (
                p.call_method0("x")?,
                p.call_method0("y")?,
                k * d.call_method0("x")?.extract::<f64>()?,
                k * d.call_method0("y")?.extract::<f64>()?,
                zoom,
            ),
        )?;
        Self::_touch(s)
    }
    fn gesture(s: &Bound<'_, Self>, event: &Obj<'_>) -> PyResult<bool> {
        let qt = s.py().import("built_core")?.getattr("Input")?;
        let kind = event.call_method0("gestureType")?;
        if kind.eq(qt.getattr("ZoomNativeGesture")?)? {
            let p = event.call_method0("position")?;
            s.getattr("view")?.getattr("renderer")?.call_method1(
                "page_zoom_by",
                (
                    p.call_method0("x")?,
                    p.call_method0("y")?,
                    1. + event.call_method0("value")?.extract::<f64>()?,
                ),
            )?;
        } else if kind.eq(qt.getattr("SmartZoomNativeGesture")?)? {
            Self::fit(s, false)?;
        }
        Self::_touch(s)?;
        Ok(true)
    }
    fn context_menu(s: &Bound<'_, Self>, event: &Obj<'_>) -> PyResult<()> {
        let p = event.call_method0("pos")?;
        s.getattr("view")?.getattr("renderer")?.call_method1(
            "page_pointer",
            ("context", p.call_method0("x")?, p.call_method0("y")?, 2, 0),
        )?;
        Self::_touch(s)
    }
    fn _command_callbacks(s: &Bound<'_, Self>) -> PyResult<Py<PyDict>> {
        let py = s.py();
        let callbacks = PyDict::new(py);
        for (key, method) in [
            ("undo", "_undo"),
            ("delete", "_delete"),
            ("duplicate", "_duplicate"),
            ("lock", "_lock_items"),
            ("model", "_back_to_model"),
            ("rename", "_rename_current"),
        ] {
            callbacks.set_item(key, s.getattr(method)?)?;
        }
        callbacks.set_item("redo", partial(py, &s.getattr("_undo")?, (true,))?)?;
        for key in ["group", "ungroup", "copy_style", "paste_style"] {
            callbacks.set_item(key, partial(py, &s.getattr("_page_command")?, (key,))?)?;
        }
        let step = s.getattr("composer")?.getattr("step_sheet")?;
        for (key, n) in [("previous", -1), ("next", 1)] {
            callbacks.set_item(key, partial(py, &step, (n,))?)?;
        }
        Ok(callbacks.unbind())
    }
    fn _page_command(s: &Bound<'_, Self>, key: &str) -> PyResult<()> {
        s.getattr("view")?
            .getattr("renderer")?
            .call_method1("page_command", (key,))?;
        Ok(())
    }
    fn _back_to_model(s: &Bound<'_, Self>) -> PyResult<()> {
        super::hooks::to_model(&s.getattr("composer")?.getattr("_window")?)
    }
    fn _rename_current(s: &Bound<'_, Self>) -> PyResult<()> {
        let sheets = Self::_sheets(s)?.into_bound(s.py());
        let cur = s.getattr("composer")?.getattr("comp")?;
        if sheets.contains(&cur)? {
            if let Some(r) = renderer(&s.getattr("view")?) {
                r.call_method1("ui_rename_sheet", (sheets.call_method1("index", (cur,))?,))?;
            }
        }
        Ok(())
    }
    #[staticmethod]
    fn _combo(event: &Obj<'_>) -> Option<i64> {
        let (key, mods) = key_mods(event).ok()?;
        Some(key | mods)
    }
    fn window_key(s: &Bound<'_, Self>, event: &Obj<'_>) -> PyResult<bool> {
        let py = s.py();
        let win = host_window(s)?;
        let render = py.import("built_render")?;
        if win.getattr("__dict__")?.contains("_registry_actions")? {
            let args = key_mods(event)?;
            let slot = win
                .getattr("_command_store")?
                .call_method1("resolve", args)?;
            if !slot.is_none() {
                return render
                    .call_method1(
                        "shortcut_domains",
                        (win.getattr("_command_actions")?
                            .get_item(slot)?
                            .call_method0("objectName")?,),
                    )?
                    .eq(3);
            }
            if win.getattr("_menu_items")?.is_truthy()? {
                return Ok(false);
            }
            let scope = render.call_method1("sheet_command_scope", (&win,))?;
            if !scope
                .getattr("_command_store")?
                .call_method1("resolve", args)?
                .is_none()
            {
                return Ok(false);
            }
        }
        if let Some(combo) = Self::_combo(event) {
            for key in WINDOW_KEYS {
                if render
                    .call_method1("shortcut_text", (*key,))?
                    .get_item(2)?
                    .eq(combo)?
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    fn key(s: &Bound<'_, Self>, event: &Obj<'_>, pressed: bool) -> PyResult<()> {
        if !pressed {
            return Ok(());
        }
        let py = s.py();
        if !event
            .call_method0("isAutoRepeat")
            .is_ok_and(|v| v.is_truthy().unwrap_or(false))
        {
            let scope = py
                .import("built_render")?
                .call_method1("sheet_command_scope", (host_window(s)?,))?;
            let slot = scope
                .getattr("_command_store")?
                .call_method1("resolve", key_mods(event)?)?;
            if !slot.is_none() {
                scope
                    .getattr("_command_actions")?
                    .get_item(slot)?
                    .call_method0("trigger")?;
                return Self::_touch(s);
            }
        }
        let name = calls::key_name(event)?;
        if name.is_truthy()?
            && s.getattr("view")?
                .getattr("renderer")?
                .call_method1("page_key", (name, calls::mods(event)?))?
                .is_truthy()?
        {
            Self::_touch(s)?;
        }
        Ok(())
    }
    fn zoom_to(s: &Bound<'_, Self>, text: &str) -> PyResult<()> {
        let view = s.getattr("view")?;
        if view.is_none() {
            return Ok(());
        }
        let m = s
            .py()
            .import("re")?
            .call_method1("search", (r"[-+]?\d+(?:[.,]\d+)?", text))?;
        if !m.is_none() {
            let value = m
                .call_method1("group", (0,))?
                .extract::<String>()?
                .replace(',', ".")
                .parse::<f64>()?;
            view.getattr("renderer")?
                .call_method1("page_zoom_to", (value,))?;
            Self::_touch(s)?;
        }
        Ok(())
    }
    fn _once(s: &Bound<'_, Self>, key: &str, value: &Obj<'_>, send: &Obj<'_>) -> PyResult<()> {
        let cache = s.getattr("_sent")?;
        if !cache.contains(key)? || !cache.get_item(key)?.eq(value)? {
            cache.set_item(key, value)?;
            send.call1((value,))?;
        }
        Ok(())
    }
    // Send chrome data only when it changes.
    fn _send(s: &Bound<'_, Self>, renderer: &Obj<'_>, view: &Obj<'_>) -> PyResult<()> {
        let cache = s.getattr("_sent")?;
        if !cache.contains("tools")? {
            cache.set_item("tools", true)?;
            renderer.call_method1("ui_set_sheet_tools", (Self::_tool_groups(s)?,))?;
        }
        let armed = s
            .getattr("composer")?
            .getattr("tool_mode")?
            .extract::<String>()?;
        renderer.call_method1(
            "ui_set_sheet_active",
            (mode(&armed).map_or("select", |t| t.0),),
        )?;
        for (k, method, sender) in [
            ("list", "sheet_rows", "ui_set_sheet_list"),
            ("exports", "exports", "ui_set_sheet_exports"),
        ] {
            Self::_once(s, k, &s.call_method0(method)?, &renderer.getattr(sender)?)?;
        }
        let status = s.call_method0("status")?;
        if !cache.contains("status")? || !cache.get_item("status")?.eq(&status)? {
            cache.set_item("status", &status)?;
            renderer.call_method1("ui_set_sheet_status", status.cast::<PyTuple>()?.clone())?;
        }
        let prompt = s.getattr("_prompt")?;
        s.setattr("_prompt", s.py().None())?;
        if !prompt.is_none() {
            renderer.call_method1("ui_set_sheet_prompt", prompt.cast::<PyTuple>()?.clone())?;
        }
        let (cluster, menu): (Obj, Obj) = s.call_method0("cluster")?.extract()?;
        let sig = ("sheets", &cluster, &menu).into_pyobject(s.py())?;
        if !view
            .getattr("__dict__")?
            .call_method1("get", ("_ui_cluster_sig",))?
            .eq(&sig)?
        {
            view.setattr("_ui_cluster_sig", sig)?;
            renderer.call_method1("ui_set_cluster", (cluster, menu))?;
        }
        Ok(())
    }
    fn _tool_groups(s: &Bound<'_, Self>) -> PyResult<Py<PyList>> {
        let py = s.py();
        let ui = py.import("views.wgpu_ui")?;
        let groups = PyList::empty(py);
        for row in RAIL {
            let tools = PyList::empty(py);
            for (key, name, icon) in *row {
                let (w, h, data): (usize, usize, Obj) =
                    ui.call_method1("_icon_rgba", (*icon,))?.extract()?;
                tools.append((*key, *name, "", w, h, data))?;
            }
            groups.append(tools)?;
        }
        Ok(groups.unbind())
    }
    fn sheet_rows(s: &Bound<'_, Self>) -> PyResult<Py<PyTuple>> {
        let composer = s.getattr("composer")?;
        let cur = composer.getattr("comp")?;
        let mut rows = vec![];
        for c in composer
            .call_method0("_scene")?
            .getattr("compositions")?
            .try_iter()?
        {
            let c = c?;
            let mut n = 0;
            for k in [
                "frames",
                "texts",
                "images",
                "scalebars",
                "nortes",
                "leyendas",
                "shapes",
                "cotas",
                "cotas_ang",
                "cotas_rad",
                "etiquetas",
                "perfiles",
                "niveles",
                "llamadas",
            ] {
                if let Some(items) = c.getattr(k).ok().filter(|i| !i.is_none()) {
                    n += items.len()?;
                }
            }
            n += usize::from(c.getattr("cajetin").is_ok_and(|v| !v.is_none()));
            let shape = if c.getattr("landscape")?.is_truthy()? {
                "▭"
            } else {
                "▯"
            };
            rows.push((
                c.getattr("name")?,
                format!("{} {shape} · {n} {}", c.getattr("paper")?.str()?, if n == 1 { "item" } else { "items" }),
                c.is(&cur),
            ));
        }
        Ok(PyTuple::new(s.py(), rows)?.unbind())
    }
    fn _frame(s: &Bound<'_, Self>) -> PyResult<Option<Py<PyAny>>> {
        let sel = Self::_selected_models(s)?.into_bound(s.py());
        if sel.len() == 1 {
            let item = sel.get_item(0)?;
            if item.is_instance(&s.py().import("built_core.composition")?.getattr("MarcoVista")?)? {
                return Ok(Some(item.unbind()));
            }
        }
        Ok(None)
    }
    fn exports(s: &Bound<'_, Self>) -> PyResult<Py<PyTuple>> {
        Ok(PyTuple::new(
            s.py(),
            [
                ("pdf", "PDF…", true),
                ("pdf_all", "All sheets…", true),
                ("png", "Image…", true),
                ("dxf", "DXF…", true),
                ("svg", "SVG…", true),
            ],
        )?
        .unbind())
    }
    fn status(s: &Bound<'_, Self>) -> PyResult<(String, String, String, String, String, String)> {
        let py = s.py();
        let composer = s.getattr("composer")?;
        let c = composer.getattr("comp")?;
        let paper = format!(
            "{} {}",
            c.getattr("paper")?.str()?,
            if c.getattr("landscape")?.is_truthy()? {
                "landscape"
            } else {
                "portrait"
            }
        );
        let frame = match Self::_frame(s)? {
            Some(f) => Some(f.into_bound(py)),
            None => c.getattr("frames")?.get_item(0).ok(),
        };
        let scale = if let Some(f) = frame {
            if f.getattr("perspective")
                .is_ok_and(|v| v.is_truthy().unwrap_or(false))
            {
                "No scale".into()
            } else {
                py.import("built_core.composition")?
                    .call_method1("format_scale", (f.getattr("scale_n")?,))?
                    .extract()?
            }
        } else {
            String::new()
        };
        let tool = mode(&composer.getattr("tool_mode")?.extract::<String>()?)
            .map_or("Select", |t| t.1)
            .to_owned();
        let (mut cursor, mut zoom) = (String::new(), String::new());
        if let Some(r) = renderer(&s.getattr("view")?) {
            let at = r.call_method0("page_pointer_mm")?;
            if at.is_truthy()? {
                let (x, y) = point_xy(&at)?;
                cursor = format!("x: {x:.1} mm  y: {y:.1} mm");
            }
            zoom = format!("{:.0}%", r.call_method0("page_zoom")?.extract::<f64>()?);
        }
        Ok((paper, scale, tool, Self::_passing(s), cursor, zoom))
    }
    fn cluster(s: &Bound<'_, Self>) -> PyResult<(Py<PyList>, Py<PyList>)> {
        let py = s.py();
        let acts = PyList::empty(py);
        let item = |label: &str,
                    f: Obj<'_>,
                    tip: &str,
                    icon: &str,
                    checked: i32|
         -> PyResult<Py<PyTuple>> {
            let i = acts.len();
            let action = PyDict::new(py);
            action.set_item("trigger", f)?;
            acts.append(
                py.import("types")?
                    .getattr("SimpleNamespace")?
                    .call((), Some(&action))?,
            )?;
            Ok((i.to_string(), label, tip, icon, checked, true)
                .into_pyobject(py)?
                .unbind())
        };
        let fit = s.getattr("fit")?;
        let cluster = PyList::new(
            py,
            [PyList::new(
                py,
                [item(
                    "Fit page",
                    fit.clone(),
                    "Fit the page in the window",
                    "zoom_extents",
                    -1,
                )?],
            )?],
        )?;
        let zoom = PyList::new(
            py,
            [
                item("Fit page", fit.clone(), "", "", -1)?,
                item("Fit width", partial(py, &fit, (true,))?, "", "", -1)?,
            ],
        )?;
        for n in [50, 100, 200, 400] {
            zoom.append(item(
                &format!("{n}%"),
                partial(py, &s.getattr("zoom_to")?, (n.to_string(),))?,
                "",
                "",
                -1,
            )?)?;
        }
        let auto = s
            .getattr("composer")?
            .getattr("_auto_render")?
            .is_truthy()?;
        let sheet = PyList::new(
            py,
            [
                item("Update all views", s.getattr("_refresh_all")?, "", "", -1)?,
                item(
                    "Auto-render views",
                    s.getattr("_toggle_auto")?,
                    "",
                    "",
                    i32::from(auto),
                )?,
                item(
                    "Back to the model",
                    s.getattr("_back_to_model")?,
                    "",
                    "",
                    -1,
                )?,
            ],
        )?;
        s.setattr("_actions", acts)?;
        Ok((
            cluster.unbind(),
            PyList::new(py, [("Zoom", zoom), ("Sheet", sheet)])?.unbind(),
        ))
    }
    fn _toggle_auto(s: &Bound<'_, Self>) -> PyResult<()> {
        Self::set_auto_render(
            s,
            !s.getattr("composer")?
                .getattr("_auto_render")?
                .is_truthy()?,
        )
    }
    // Actions emitted by the native page and its inspector.
    fn action(s: &Bound<'_, Self>, arg: &str) -> PyResult<()> {
        let (verb, rest) = arg.split_once('\x1f').unwrap_or((arg, ""));
        let py = s.py();
        let comp = s.getattr("composer")?;
        let sheets = Self::_sheets(s)?.into_bound(py);
        let json = |v: &str| py.import("json")?.call_method1("loads", (v,));
        match verb {
            "tool" if rest == "imagen" => Self::_add_image(s)?,
            "tool" if mode(rest).is_some() => Self::_tool(s, rest)?,
            "lock_items" => Self::_lock_items(s)?,
            "delete_items" => Self::_delete(s)?,
            "duplicate_items" => Self::_duplicate(s)?,
            "rects" => Self::_move(s, rest)?,
            "place" => {
                let parts: Vec<_> = rest.split('\x1f').collect();
                if parts.len() < 4 {
                    return Err(pyo3::exceptions::PyValueError::new_err(
                        "missing sheet placement coordinates",
                    ));
                }
                let mode = comp.getattr("tool_mode")?.extract::<String>()?;
                let hit = parts.get(5).map(|v| json(v)).transpose()?;
                Self::_place(
                    s,
                    parts[0].parse()?,
                    parts[1].parse()?,
                    parts[2].parse()?,
                    parts[3].parse()?,
                    parts.get(4).copied().unwrap_or(&mode),
                    hit.as_ref().filter(|h| !h.is_none()),
                )?;
            }
            "sheet" | "set" | "set_many" => {
                let edits = PyList::empty(py);
                if verb == "sheet" {
                    edits.append((comp.getattr("comp")?, json(rest)?))?;
                } else {
                    let models = Self::_models(s)?.into_bound(py);
                    let parts: Vec<_> = rest.split('\x1f').collect();
                    for pair in parts.chunks_exact(2) {
                        if let Some(m) = models.get_item(pair[0].parse::<i64>()?)? {
                            edits.append((m, json(pair[1])?))?;
                        }
                    }
                }
                Self::_edit(s, edits.as_any())?;
            }
            "font" | "refresh" => {
                if let Some(m) = Self::_models(s)?.bind(py).get_item(rest.parse::<i64>()?)? {
                    if verb == "font" {
                        Self::_font(s, &m)?;
                    } else {
                        Self::_forget_render(s, &m)?;
                    }
                }
            }
            "radial" | "angular" => Self::_dimension(s, verb, &json(rest)?)?,
            "cota" | "cota_total" => Self::_cota(s, &json(rest)?, verb == "cota_total")?,
            "open" => {
                comp.call_method1("show_sheet", (rest.parse::<isize>()?,))?;
            }
            "add" => Self::_add_sheet(s)?,
            "dup" => Self::_duplicate_sheet(s, rest.parse()?)?,
            "rename" => {
                let (i, name) = rest.split_once('\x1f').unwrap_or((rest, ""));
                let i = i.parse::<isize>()?;
                let name = name.trim();
                if (0..sheets.len()? as isize).contains(&i) && !name.is_empty() {
                    let sheet = sheets.get_item(i)?;
                    if !sheet.getattr("name")?.eq(name)? {
                        let change = PyDict::new(py);
                        change.set_item("name", name)?;
                        Self::_edit(s, PyList::new(py, [(sheet, change)])?.as_any())?;
                        comp.call_method0("_reload_comp_combo")?;
                    }
                }
            }
            "up" | "down" => {
                let i = rest.parse::<isize>()?;
                let j = i.saturating_add(if verb == "up" { -1 } else { 1 });
                let valid = 0..sheets.len()? as isize;
                if valid.contains(&i) && valid.contains(&j) {
                    let step = py
                        .import("built_core.history")?
                        .call_method1("MoveSheetCommand", (sheets.get_item(i)?, j))?;
                    Self::_execute(s, &step)?;
                    comp.call_method0("_reload_comp_combo")?;
                }
            }
            "del" => {
                let i = rest.parse::<isize>()?;
                let n = sheets.len()? as isize;
                if n > 1 && (0..n).contains(&i) {
                    s.setattr("_pending", ("del", i))?;
                    s.setattr(
                        "_prompt",
                        (
                            format!("Delete “{}”?", sheets.get_item(i)?.getattr("name")?.str()?),
                            "Its views, texts and dimensions go with it. Undo brings it back.",
                            py.None(),
                            vec![("del", "Delete"), ("cancel", "Cancel")],
                        ),
                    )?;
                }
            }
            "answer" => {
                let (bid, text) = rest.split_once('\x1f').unwrap_or((rest, ""));
                Self::_answer(s, bid, text)?;
            }
            "export" => Self::export(s, rest)?,
            "zoom" => Self::zoom_to(s, rest)?,
            "fit" => Self::fit(s, false)?,
            _ => {}
        }
        Self::_touch(s)
    }
    fn _answer(s: &Bound<'_, Self>, bid: &str, text: &str) -> PyResult<()> {
        let _ = text;
        let pending = s.getattr("_pending")?;
        s.setattr("_pending", s.py().None())?;
        if bid == "del" && !pending.is_none() && pending.get_item(0)?.eq("del")? {
            Self::_delete_sheet(s, pending.get_item(1)?.extract()?)?;
        }
        Ok(())
    }
    fn _add_sheet(s: &Bound<'_, Self>) -> PyResult<()> {
        let py = s.py();
        let sheets = Self::_sheets(s)?.into_bound(py);
        let template = super::sheet_template(py)?;
        let mut sheet = py.None().into_bound(py);
        if let Some(path) = template {
            sheet = py
                .import("built_core")?
                .call_method1("sheet_from_template", (path.to_string_lossy().as_ref(),))?;
        }
        if sheet.is_none() {
            sheet = py.import("built_core.composition")?.call_method0("Composicion")?;
            sheet
                .getattr("frames")?
                .call_method1("append", (sheet.call_method0("default_frame")?,))?;
        }
        let args = PyDict::new(py);
        args.set_item("n", sheets.len()? + 1)?;
        sheet.setattr(
            "name",
            py.import("built_core.i18n")?
                .getattr("tr")?
                .call(("Sheet {n}",), Some(&args))?,
        )?;
        Self::_execute(
            s,
            &py.import("built_core.history")?
                .call_method1("InsertSheetCommand", (sheet,))?,
        )?;
        Self::_show(s, sheets.len()? as isize - 1)
    }
    fn _duplicate_sheet(s: &Bound<'_, Self>, index: isize) -> PyResult<()> {
        let py = s.py();
        let sheets = Self::_sheets(s)?.into_bound(py);
        if !(0..sheets.len()? as isize).contains(&index) {
            return Ok(());
        }
        let src = sheets.get_item(index)?;
        let dup = py
            .import("built_core.composition")?
            .getattr("Composicion")?
            .call_method1("from_dict", (src.call_method0("to_dict")?,))?;
        dup.setattr(
            "name",
            format!(
                "{}{}",
                src.getattr("name")?.str()?,
                py.import("built_core.i18n")?
                    .call_method1("tr", (" (copy)",))?
                    .str()?
            ),
        )?;
        Self::_execute(
            s,
            &py.import("built_core.history")?
                .call_method1("InsertSheetCommand", (dup, index + 1))?,
        )?;
        Self::_show(s, index + 1)
    }
    fn _delete_sheet(s: &Bound<'_, Self>, index: isize) -> PyResult<()> {
        let py = s.py();
        let sheets = Self::_sheets(s)?.into_bound(py);
        let n = sheets.len()? as isize;
        if n <= 1 || !(0..n).contains(&index) {
            return Ok(());
        }
        let sheet = sheets.get_item(index)?;
        let comp = s.getattr("composer")?;
        let current = comp.getattr("comp")?.is(&sheet);
        Self::_execute(
            s,
            &py.import("built_core.history")?
                .call_method1("RemoveSheetCommand", (sheet,))?,
        )?;
        if current {
            Self::_show(s, index)?;
        } else {
            comp.call_method0("_reload_comp_combo")?;
        }
        Ok(())
    }
    fn _default_path(s: &Bound<'_, Self>, kind: &str) -> PyResult<String> {
        use std::path::Path;
        let (_, ext, _, _) = export_spec(kind)
            .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(kind.to_string()))?;
        let c = s.getattr("composer")?;
        let mut name = if kind == "pdf_all" {
            "Sheets".to_string()
        } else {
            c.getattr("comp")?.getattr("name")?.extract()?
        };
        let doc = c
            .getattr("_window")?
            .getattr("_current_path")
            .ok()
            .filter(|d| d.is_truthy().unwrap_or(false));
        let folder = if let Some(doc) = doc {
            let path = doc.str()?.to_string();
            let path = Path::new(&path);
            name = format!(
                "{} — {name}",
                path.file_stem().unwrap_or_default().to_string_lossy()
            );
            path.parent().unwrap_or(Path::new("")).to_path_buf()
        } else {
            s.py()
                .import("pathlib")?
                .getattr("Path")?
                .call_method0("home")?
                .extract::<std::path::PathBuf>()?
                .join("Desktop")
        };
        let mut clean = String::new();
        let mut bad = false;
        for ch in name.chars() {
            if r#"\/:*?"<>|"#.contains(ch) {
                if !bad {
                    clean.push('-');
                }
                bad = true;
            } else {
                clean.push(ch);
                bad = false;
            }
        }
        let name = clean.trim();
        Ok(folder
            .join(format!(
                "{}{ext}",
                if name.is_empty() { "sheet" } else { name }
            ))
            .to_string_lossy()
            .into_owned())
    }
    fn export(s: &Bound<'_, Self>, kind: &str) -> PyResult<()> {
        let Some((title, ext, filters, _)) = export_spec(kind) else {
            return Ok(());
        };
        let py = s.py();
        let panel = py
            .import("built_render.shell_files")?
            .getattr("ShellFileDialog")?;
        let mut path: String = panel
            .call_method1(
                "getSaveFileName",
                (py.None(), title, Self::_default_path(s, kind)?, filters),
            )?
            .get_item(0)?
            .extract()?;
        if !path.is_empty() {
            if std::path::Path::new(&path)
                .extension()
                .is_none_or(|s| s.is_empty())
            {
                path.push_str(ext);
            }
            Self::_export_to(s, kind, &path)?;
        }
        Ok(())
    }
    fn _export_to(s: &Bound<'_, Self>, kind: &str, path: &str) -> PyResult<()> {
        let py = s.py();
        let comp = s.getattr("composer")?;
        let parent = comp.getattr("_window")?;
        let name = std::path::Path::new(path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let result = (|| -> PyResult<Vec<String>> {
            let (_, _, _, label) = export_spec(kind)
                .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(kind.to_string()))?;
            let kw = PyDict::new(py);
            kw.set_item("output_path", path)?;
            let busy = py
                .import("built_render.chrome_dialogs")?
                .getattr("busy")?
                .call((&parent, format!("{label} · {name}")), Some(&kw))?;
            busy.call_method0("__enter__")?;
            let result = (|| -> PyResult<Vec<String>> {
                let frames = py.import("built_render.sheet_frames")?;
                let view = s.getattr("view")?;
                let sheet = comp.getattr("comp")?;
                let current = parent.getattr("_current_path").ok();
                let annots = parent.getattr("_ext_sheet_annots").ok();
                match kind {
                    "pdf" | "pdf_all" => {
                        let sheets = if kind == "pdf" {
                            PyList::new(py, [sheet])?.into_any()
                        } else {
                            py.get_type::<PyList>().call1((Self::_sheets(s)?,))?
                        };
                        frames
                            .call_method1(
                                "export_pdf",
                                (view, sheets, path, ground(s)?, current, annots),
                            )?
                            .extract()
                    }
                    "png" => frames
                        .call_method1(
                            "export_image",
                            (view, sheet, path, 200., ground(s)?, current, annots),
                        )?
                        .extract(),
                    _ => frames.call_method1("export_sheet_vector", (kind, view, sheet, path, ground(s)?, current, annots))?.extract(),
                }
            })();
            match result {
                Ok(value) => {
                    busy.call_method1("__exit__", (py.None(), py.None(), py.None()))?;
                    Ok(value)
                }
                Err(e) => {
                    if busy
                        .call_method1("__exit__", (e.get_type(py), e.value(py), e.traceback(py)))?
                        .is_truthy()?
                    {
                        Ok(vec![])
                    } else {
                        Err(e)
                    }
                }
            }
        })();
        match result {
            Err(e) => {
                let msg = e.value(py).str()?.to_string();
                e.print(py);
                Self::flash(s, &format!("Export failed: {msg}"), 8000)
            }
            Ok(troubled) => {
                s.setattr("last_export", path)?;
                if troubled.is_empty() {
                    Self::flash(s, &format!("Exported {name}"), 5000)
                } else {
                    Self::flash(
                        s,
                        &format!(
                            "Exported {name} — a view would not render on {}",
                            troubled.join(", ")
                        ),
                        10000,
                    )
                }
            }
        }
    }
}
