//! Host adapters for the shared sheet. Files and preview decoding remain with
//! the existing recent-file worker. Export reuses the delivery form's model.
use crate::settings_rows::tr;
use pyo3::{exceptions::PyInterruptedError, prelude::*, types::PyDict};

#[pyclass]
pub struct ExportSheet {
    form: Py<PyAny>,
    svg: Py<crate::svg_options::SvgOptions>,
    one_off: bool,
    render_preview: Py<PyAny>,
}

/// The export backend can pump the window while working. Only the current
/// ticket may publish a texture into this window.
#[pyclass]
struct PreviewTask {
    win: Py<PyAny>,
    form: Py<PyAny>,
    svg: Py<crate::svg_options::SvgOptions>,
    ticket: u64,
    render: Py<PyAny>,
}

#[pymethods]
impl PreviewTask {
    fn __call__(&self, py: Python<'_>) -> PyResult<()> {
        let svg = self.svg.bind(py);
        let form = self.form.bind(py);
        if form.getattr("closed")?.is_truthy()? || !svg.borrow_mut().begin_preview(self.ticket) {
            return Ok(());
        }
        let win = self.win.bind(py);
        match self.render.call1(py, (self.ticket,)) {
            Ok(image) => {
                if svg.borrow_mut().finish_preview(self.ticket, String::new()) {
                    let view = py
                        .import("built_render.chrome_dialogs")?
                        .call_method1("view_of", (win,))?;
                    if !view.is_none() {
                        let (w, h, rgba): (usize, usize, Bound<'_, pyo3::types::PyBytes>) =
                            image.extract(py)?;
                        view.getattr("renderer")?.call_method1(
                            "ui_set_icons",
                            (vec![("delivery_svg_preview", w, h, rgba)],),
                        )?;
                    }
                }
            }
            Err(e) if e.is_instance_of::<PyInterruptedError>(py) => {
                let current = svg
                    .borrow_mut()
                    .finish_preview(self.ticket, tr(py, "Preview cancelled")?);
                if current {
                    svg.call_method1("receive", ("svg_preview", "0"))?;
                }
            }
            Err(e) => {
                svg.borrow_mut().finish_preview(self.ticket, e.to_string());
            }
        }
        if !form.getattr("closed")?.is_truthy()? {
            py.import("built_render.chrome_sheet")?
                .call_method1("refresh", (win,))?;
        }
        Ok(())
    }
}

#[pyfunction]
pub fn show_delivery_form(
    py: Python<'_>,
    win: &Bound<'_, PyAny>,
    form: &Bound<'_, PyAny>,
    one_off: bool,
    svg: &Bound<'_, crate::svg_options::SvgOptions>,
    render_preview: Py<PyAny>,
) -> PyResult<()> {
    form.setattr("win", win)?;
    svg.call_method1("bind", (form,))?;
    let sheet = Py::new(
        py,
        ExportSheet {
            form: form.clone().unbind(),
            svg: svg.clone().unbind(),
            one_off,
            render_preview,
        },
    )?;
    py.import("built_render.chrome_sheet")?
        .call_method1("open", (win, sheet))?;
    Ok(())
}

/// The format is fixed by the current-view command. Only its destination
/// and the SVG model are editable. Both one-off paths use ExportSheet.
#[pyfunction]
pub fn show_view_export<'py>(
    py: Python<'py>,
    win: &Bound<'py, PyAny>,
    svg: &Bound<'py, crate::svg_options::SvgOptions>,
    write: Py<PyAny>,
    preview: Py<PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let forms = py.import("built_render.chrome_forms")?;
    let field = PyDict::new(py);
    field.set_item("pick", "save")?;
    field.set_item("filters", "SVG (*.svg)")?;
    let destination = forms
        .getattr("Field")?
        .call(("destination", "path", tr(py, "Destination")?, ""), Some(&field))?;
    let args = PyDict::new(py);
    args.set_item("buttons", [("cancel", tr(py, "Cancel")?), ("export", tr(py, "Export")?)])?;
    args.set_item("default", "export")?;
    args.set_item("validate", svg.getattr("validate_view")?)?;
    args.set_item("on_done", write)?;
    let form = forms
        .getattr("Form")?
        .call((tr(py, "Export")?, vec![destination]), Some(&args))?;
    show_delivery_form(py, win, &form, true, svg, preview)?;
    Ok(form)
}

#[pymethods]
impl ExportSheet {
    #[getter]
    fn id(&self) -> &'static str {
        if self.one_off {
            "export"
        } else {
            "delivery"
        }
    }

    fn signature(&self, py: Python<'_>, ctx: &Bound<'_, PyAny>) -> PyResult<(u64, u64)> {
        let version = self.form.bind(py).getattr("version")?.extract()?;
        let scene = if self.svg.borrow(py).preview_enabled() {
            ctx.getattr("win")?
                .getattr("viewport")?
                .getattr("scene")?
                .getattr("version")?
                .extract()?
        } else {
            0
        };
        Ok((version, scene))
    }

    fn build<'py>(&self, py: Python<'py>, ctx: &Bound<'_, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let form = self.form.bind(py);
        let chrome = py.import("built_render.chrome_sheet")?;
        let options = PyDict::new(py);
        let svg = self.svg.bind(py);
        let is_svg = svg.borrow().is_current_view()
            || form
                .call_method1("value", ("format",))?
                .extract::<String>()?
                == "svg";
        let preview = is_svg && svg.getattr("preview_enabled")?.is_truthy()?;
        options.set_item("width", if preview { 1152 } else { 880 })?;
        let sheet = chrome
            .getattr("Sheet")?
            .call((self.id(), form.getattr("title")?), Some(&options))?;
        sheet.setattr("default_verb", form.getattr("default")?)?;
        let content = sheet.call_method0("content")?;
        let wire = form.call_method0("wire")?;
        let note: String = wire.get_item(2)?.extract()?;
        if !note.is_empty() {
            content.call_method1("text", (note,))?;
        }
        content.call_method1("fields", (form, "delivery"))?;
        if !is_svg && form.call_method1("value", ("format",))?.eq("package")? {
            let records = py.import("built_core.deliveries")?.call_method1(
                "records",
                (ctx.getattr("win")?.getattr("viewport")?.getattr("scene")?,),
            )?;
            let selected = form.getattr_opt("_delivery_key")?;
            sheet.call_method1("side_caption", (tr(py, "Kept deliveries")?,))?;
            for (key, record) in records.cast::<PyDict>()?.iter() {
                let kw = PyDict::new(py);
                kw.set_item(
                    "on",
                    selected
                        .as_ref()
                        .is_some_and(|k| k.eq(&key).unwrap_or(false)),
                )?;
                sheet.call_method("side_item", (&key, record.get_item("name")?), Some(&kw))?;
            }
            sheet.call_method1("side_item", ("new", tr(py, "New delivery…")?))?;
            let values = form.call_method0("values")?;
            let get = |k: &str| -> PyResult<String> { values.get_item(k)?.extract() };
            let date = py
                .import("datetime")?
                .getattr("date")?
                .call_method0("today")?
                .str()?
                .to_string();
            let name: String = get("name")?
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || " -_".contains(c) {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let folder = std::path::PathBuf::from(get("destination")?)
                .join(format!("{date}_{}", name.trim()));
            content.call_method1("kv", ([(tr(py, "Folder preview")?, folder.to_string_lossy())],))?;
            content.call_method1("caption", (tr(py, "Outputs")?,))?;
            let items = pyo3::types::PyList::empty(py);
            let mut count = 0;
            let selection = get("output_options")?;
            let mut info = String::new();
            for (id, title, file) in crate::deliveries_host::package_outputs(py)? {
                let item = PyDict::new(py);
                item.set_item("id", &id)?;
                item.set_item("label", format!("{} · {}", id.to_uppercase(), tr(py, &title)?))?;
                let source = match id.as_str() {
                    "pdf" => {
                        if get("sheet_names")?.trim().is_empty() {
                            tr(py, "All sheets")?
                        } else {
                            get("sheet_names")?.replace('\n', ", ")
                        }
                    }
                    "bcf" => {
                        if get("topics")?.trim().is_empty() {
                            tr(py, "All topics")?
                        } else {
                            tr(py, "Selected topics")?
                        }
                    }
                    "manifest" => "JSON + text · SHA-256".into(),
                    "dxf" if !get("drawing_names")?.trim().is_empty() => tr(py, "Selected scenes")?,
                    _ => {
                        let scope = get(&format!("source_{id}"))?;
                        let scope = if scope.is_empty() {
                            get("scope")?
                        } else {
                            scope
                        };
                        match scope.as_str() {
                            "whole" => tr(py, "Whole model")?,
                            "node" => tr(py, "Project node")?,
                            _ => scope,
                        }
                    }
                };
                item.set_item("sub", source)?;
                let enabled = values.get_item(format!("include_{id}"))?.is_truthy()?;
                item.set_item("on", enabled)?;
                count += usize::from(enabled);
                items.append(item)?;
                if id == selection {
                    info = format!("{} · {file}", tr(py, &title)?);
                }
            }
            let block = PyDict::new(py);
            block.set_item("kind", "delivery-outputs")?;
            block.set_item("items", items)?;
            content
                .getattr("blocks")?
                .call_method1("append", (block,))?;
            if !info.is_empty() {
                content.call_method1("text", (info,))?;
            }
            let setup = crate::deliveries_host::unpack(py, svg.as_any(), &values)?;
            let error = py
                .import("built_core.deliveries")?
                .call_method1("validate", (&setup,))
                .err();
            let scene = ctx.getattr("win")?.getattr("viewport")?.getattr("scene")?;
            let sheets = scene.getattr("compositions")?;
            let missing_pdf = values.get_item("include_pdf")?.is_truthy()? && sheets.len()? == 0;
            let unsaved = folder.is_relative()
                && !ctx
                    .getattr("win")?
                    .getattr_opt("_current_path")?
                    .is_some_and(|v| v.is_truthy().unwrap_or(false))
                && !scene
                    .getattr_opt("_project_host_path")?
                    .is_some_and(|v| v.is_truthy().unwrap_or(false));
            let hint = if unsaved {
                tr(py, "Choose an absolute destination for this unsaved model")?
            } else if missing_pdf {
                tr(py, "PDF needs a sheet · Other outputs can run")?
            } else if let Some(e) = error {
                e.value(py).str()?.to_string()
            } else {
                tr(py, "{count} outputs selected · Ready to run")?.replace("{count}", &count.to_string())
            };
            if selection == "review" {
                let detail = sheet.call_method1("detail", (330., tr(py, "Review checks")?))?;
                detail.call_method1("text", (&hint,))?;
                if let Some(run) = form.getattr_opt("_delivery_run")? {
                    crate::deliveries_host::review_content(&detail, &run)?;
                } else {
                    detail.call_method1("text", (tr(py, "IDS checks run on the exported IFC model.")?,))?;
                }
            }
            sheet.call_method1("footer", ([("setup-review", tr(py, "Review")?)], hint))?;
        }
        if is_svg {
            content.call_method1("fields", (svg, "svg"))?;
        }
        let revision = if preview {
            let scene = self.signature(py, ctx)?.1;
            let mut key = format!(
                "{scene}/{}",
                svg.call_method0("options_json")?.extract::<String>()?
            );
            for fid in ["scope", "node", "names"] {
                key.push('\0');
                key.push_str(&form.call_method1("value", (fid,))?.str()?.to_string());
            }
            Some(key)
        } else {
            None
        };
        let ticket = svg
            .call_method1("preview_ticket", (revision,))?
            .extract::<Option<u64>>()?;
        if let Some(ticket) = ticket {
            let task = Py::new(
                py,
                PreviewTask {
                    win: ctx.getattr("win")?.unbind(),
                    form: self.form.clone_ref(py),
                    svg: self.svg.clone_ref(py),
                    ticket,
                    render: self.render_preview.clone_ref(py),
                },
            )?;
            let win = ctx.getattr("win")?;
            if let Ok(queue) = win.getattr("_host").and_then(|host| host.getattr("timers")) {
                queue.call_method1("later", (&win, 180, task))?;
            } else {
                // A window without the shell's timers (tests): render now.
                task.bind(py).call0()?;
            }
        }
        if preview {
            let pane = sheet.call_method1("detail", (400., tr(py, "Preview")?))?;
            let (ready, guide, error) = svg
                .call_method0("preview_block")?
                .extract::<(bool, String, String)>()?;
            if ready {
                let block = PyDict::new(py);
                block.set_item("kind", "svg_preview")?;
                block.set_item("icon", "delivery_svg_preview")?;
                block.set_item("text", guide)?;
                pane.getattr("blocks")?.call_method1("append", (block,))?;
                if let Some(caption) = svg.borrow().preview_caption() {
                    pane.call_method1("text", (caption,))?;
                }
            } else {
                pane.call_method1(
                    "text",
                    (if error.is_empty() {
                        tr(py, "Preparing preview…")?
                    } else {
                        error
                    },),
                )?;
            }
        }
        for (id, label, key, role) in wire
            .get_item(5)?
            .extract::<Vec<(String, String, String, u8)>>()?
        {
            let verb = PyDict::new(py);
            if id == "run" && form.call_method1("value", ("format",))?.eq("package")? {
                verb.set_item("id", &id)?;
                verb.set_item("label", tr(py, "Run handover package")?)?;
                verb.set_item("role", 1)?;
                sheet.call_method1("footer_action", (verb,))?;
                continue;
            }
            verb.set_item("id", id)?;
            verb.set_item("label", label)?;
            verb.set_item("key", key)?;
            verb.set_item("role", role)?;
            sheet.getattr("verbs")?.call_method1("append", (verb,))?;
        }
        Ok(sheet)
    }

    fn act(
        &self,
        py: Python<'_>,
        ctx: &Bound<'_, PyAny>,
        verb: &str,
        args: Vec<String>,
    ) -> PyResult<()> {
        let form = self.form.bind(py);
        if verb == "side" {
            if let Some(id) = args.first() {
                let key = if id == "new" { None } else { Some(id.clone()) };
                crate::deliveries_host::export_form(&ctx.getattr("win")?, key, false, "package")?;
            }
            return Ok(());
        }
        if verb == "delivery-toggle" || verb == "delivery-options" {
            if let Some(id) = args.first() {
                if verb == "delivery-options" {
                    form.call_method1("set", ("output_options", id))?;
                } else {
                    let field = format!("include_{id}");
                    let value = form.call_method1("value", (&field,))?.is_truthy()?;
                    form.call_method1("set", (field, !value))?;
                }
            }
            return Ok(());
        }
        if verb == "verb" && args.first().is_some_and(|v| v == "setup-review") {
            form.call_method1("set", ("output_options", "review"))?;
            return Ok(());
        }
        if verb == "svg_frame" && args.len() == 3 {
            if let [Ok(x), Ok(y), Ok(zoom)] =
                [args[0].parse::<f64>(), args[1].parse(), args[2].parse()]
            {
                self.svg.bind(py).call_method1("move_frame", (x, y, zoom))?;
                self.form.bind(py).call_method0("changed")?;
            }
        } else if verb == "verb" {
            if let Some(button) = args.first() {
                let form = self.form.bind(py);
                form.call_method1("press", (button,))?;
                if form.getattr("closed")?.is_truthy()? {
                    py.import("built_render.chrome_sheet")?
                        .call_method1("close", (ctx.getattr("win")?,))?;
                }
            }
        }
        Ok(())
    }

    fn closed(&self, py: Python<'_>, _ctx: &Bound<'_, PyAny>) -> PyResult<()> {
        self.svg
            .bind(py)
            .call_method1("preview_ticket", (py.None(),))?;
        self.form.bind(py).call_method0("close")?;
        self.form.bind(py).setattr("win", py.None())
    }
}
