//! Host adapters for the shared sheet. Files and preview decoding remain with
//! the existing recent-file worker. Export reuses the delivery form's model.
use pyo3::{prelude::*, types::PyDict};

#[pyfunction]
pub fn start_backdrop(
    open_document: bool,
    last: String,
    recents: &Bound<'_, PyAny>,
    thumbnail: &Bound<'_, PyAny>,
    sheet: &Bound<'_, PyAny>,
) -> PyResult<String> {
    if open_document {
        return Ok(String::new());
    }
    // Pinning changes the display order. Try the actual last document first,
    // then the other cached file previews, independently of the search filter.
    let mut paths = vec![last];
    for item in recents.try_iter()? {
        let path = item?.get_item("path")?.extract::<String>()?;
        if path != paths[0] {
            paths.push(path);
        }
    }
    for path in paths.into_iter().filter(|p| !p.is_empty()) {
        let key: String = thumbnail.call1((sheet, path))?.extract()?;
        if !key.is_empty() {
            return Ok(key);
        }
    }
    Ok(String::new())
}

#[pyclass]
pub struct ExportSheet {
    form: Py<PyAny>,
}

#[pyfunction]
pub fn show_delivery_form(
    py: Python<'_>,
    win: &Bound<'_, PyAny>,
    form: &Bound<'_, PyAny>,
    one_off: bool,
) -> PyResult<()> {
    if one_off {
        form.setattr("win", win)?;
        let sheet = Py::new(
            py,
            ExportSheet {
                form: form.clone().unbind(),
            },
        )?;
        py.import("views.chrome_sheet")?
            .call_method1("open", (win, sheet))?;
    } else {
        py.import("views.chrome_forms")?
            .call_method1("show", (win, form))?;
    }
    Ok(())
}

#[pymethods]
impl ExportSheet {
    #[getter]
    fn id(&self) -> &'static str {
        "export"
    }

    fn signature(&self, py: Python<'_>, _ctx: &Bound<'_, PyAny>) -> PyResult<u64> {
        self.form.bind(py).getattr("version")?.extract()
    }

    fn build<'py>(&self, py: Python<'py>, _ctx: &Bound<'_, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let form = self.form.bind(py);
        let chrome = py.import("views.chrome_sheet")?;
        let options = PyDict::new(py);
        options.set_item("width", 880)?;
        let sheet = chrome
            .getattr("Sheet")?
            .call(("export", form.getattr("title")?), Some(&options))?;
        sheet.setattr("default_verb", form.getattr("default")?)?;
        let content = sheet.call_method0("content")?;
        let wire = form.call_method0("wire")?;
        let note: String = wire.get_item(2)?.extract()?;
        if !note.is_empty() {
            content.call_method1("text", (note,))?;
        }
        content.call_method1("fields", (form, "delivery"))?;
        for (id, label, key, role) in wire
            .get_item(5)?
            .extract::<Vec<(String, String, String, u8)>>()?
        {
            let verb = PyDict::new(py);
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
        if verb == "verb" {
            if let Some(button) = args.first() {
                let form = self.form.bind(py);
                form.call_method1("press", (button,))?;
                if form.getattr("closed")?.is_truthy()? {
                    py.import("views.chrome_sheet")?
                        .call_method1("close", (ctx.getattr("win")?,))?;
                }
            }
        }
        Ok(())
    }

    fn closed(&self, py: Python<'_>, _ctx: &Bound<'_, PyAny>) -> PyResult<()> {
        self.form.bind(py).call_method0("close")?;
        self.form.bind(py).setattr("win", py.None())
    }
}
