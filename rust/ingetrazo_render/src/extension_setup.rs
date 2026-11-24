//! Bind installed Python extension entry points after a window has its document.
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};


#[pyclass(unsendable)]
struct ToolEntry {
    window: Py<PyAny>,
    tool: Py<PyAny>,
}
#[pymethods]
impl ToolEntry {
    fn __call__(&self, py: Python<'_>) -> PyResult<()> {
        let win = self.window.call0(py)?;
        if !win.is_none(py) {
            self.tool
                .call_method1(py, "on_activate", (win.getattr(py, "viewport")?,))?;
        }
        Ok(())
    }
}

#[pyfunction]
pub fn setup_extension_menus(win: &Bound<'_, PyAny>, plugins: &Bound<'_, PyList>) -> PyResult<()> {
    let py = win.py();
    let menus = win.getattr("_extension_menus")?;
    let app_class = py.import("views.extension_api")?.getattr("ExtensionApp")?;
    let tr = py.import("core.i18n")?.getattr("tr")?;
    let weak = py.import("weakref")?.call_method1("ref", (win,))?;
    for plug in plugins {
        let owner = plug.getattr("stem")?.extract::<String>()?;
        // Bundled native panels have their own command in the shell.
        if plug.getattr("setup")?.is_none() && plug.getattr("tools")?.len()? == 0 { continue; }
        let app = app_class.call1((win, &owner))?;
        let setup = || -> PyResult<()> {
            let hook = plug.getattr("setup")?;
            if !hook.is_none() {
                hook.call1((&app,))?;
            }
            let root = menus.call_method1("root", (&owner,))?;
            for tool in plug.getattr("tools")?.try_iter()? {
                let tool = tool?;
                let label = tr.call1((tool.getattr("name")?,))?;
                let tip = tr.call1((tool.getattr("description")?,))?;
                let key = tool.getattr("shortcut")?;
                let run = ToolEntry {
                    window: weak.clone().unbind(),
                    tool: tool.unbind(),
                };
                root.call_method1("add_command", (label, run, key, tip))?;
            }
            win.getattr("_running_extensions")?
                .call_method1("add", (&owner,))?;
            Ok(())
        };
        if let Err(error) = setup() {
            menus.call_method1("discard", (&owner,))?;
            app.call_method0("remove_agent_tools")?;
            let details = PyDict::new(py);
            details.set_item("path", plug.getattr("path")?.str()?)?;
            details.set_item("error", error.to_string())?;
            win.getattr("_extension_load_errors")?
                .set_item(owner, details)?;
            error.print(py);
        }
    }
    Ok(())
}
