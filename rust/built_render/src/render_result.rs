//! Blender's image utility. The plugin supplies only the backend actions.
use pyo3::prelude::*;
use serde_json::json;

/// Resolve the executable and explain whether this package can launch it.
#[pyfunction]
pub fn blender_availability<'py>(py: Python<'py>, path: &str) -> PyResult<(Option<Bound<'py, PyAny>>, String)> {
    let backend = py.import("core.render_blender")?;
    let (found, message) = if backend.call_method0("in_snap")?.is_truthy()? {
        let steps = backend.call_method1("install_steps", ("snap",))?;
        (None, steps.get_item(0)?.get_item(0)?.extract::<String>()?)
    } else {
        let found = backend.call_method1("find_blender", (path,))?;
        if found.is_none() { (None, "Choose the Blender executable to render".into()) }
        else { (Some(found), "Ready".into()) }
    };
    Ok((found, py.import("built_core.i18n")?.call_method1("tr", (message,))?.extract()?))
}

#[pyclass]
pub struct RenderResult {
    #[pyo3(get, set)]
    path: String,
    #[pyo3(get, set)]
    note: String,
    #[pyo3(get, set)]
    following: bool,
    save: Py<PyAny>,
    folder: Py<PyAny>,
    follow: Py<PyAny>,
}

#[pymethods]
impl RenderResult {
    #[classattr]
    #[pyo3(name = "id")]
    const ID: &'static str = "render_result";
    #[classattr]
    #[pyo3(name = "title")]
    const TITLE: &'static str = "Render preview";
    #[classattr]
    #[pyo3(name = "slot")]
    const SLOT: &'static str = "utility";

    #[classattr]
    #[pyo3(name = "notify_close")]
    const NOTIFY_CLOSE: bool = true;

    #[new]
    fn new(
        path: String,
        following: bool,
        save: Py<PyAny>,
        folder: Py<PyAny>,
        follow: Py<PyAny>,
    ) -> Self {
        Self {
            path,
            following,
            save,
            folder,
            follow,
            note: String::new(),
        }
    }

    fn sections(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let section = json!([{"id":"result","title":"","items":[
            {"kind":"image","id":"image","path":self.path},
            {"kind":"button","id":"save","label":"Save image…"},
            {"kind":"button","id":"folder","label":"Open folder"},
            {"kind":"field","id":"follow","type":"toggle","label":"Follow camera","value":self.following},
            {"kind":"log","id":"status","lines":if self.note.is_empty() {vec![]} else {vec![&self.note]}}
        ]}]);
        Ok(py
            .import("json")?
            .call_method1("loads", (section.to_string(),))?
            .unbind())
    }

    fn event(
        &mut self,
        py: Python<'_>,
        control: &str,
        event: &str,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        match (control, event) {
            ("save", "invoke") => {
                self.save.call0(py)?;
            }
            ("folder", "invoke") => {
                self.folder.call0(py)?;
            }
            ("follow", "commit") | ("_panel", "closed") => {
                self.following = control == "follow" && value.extract::<bool>()?;
                self.follow.call1(py, (self.following,))?;
            }
            _ => (),
        }
        Ok(())
    }
}
