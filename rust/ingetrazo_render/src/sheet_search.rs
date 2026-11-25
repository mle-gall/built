//! The sheet's command palette and keyboard route share the page's tool rail
//! and action handles. A modelling command must never run over a sheet.
use pyo3::{prelude::*, types::PyDict};

const EDITS: &[(&str, &str, &str, &[&str])] = &[
    (
        "undo",
        "Undo",
        "Undo the last sheet or model edit.",
        &["Ctrl+Z"],
    ),
    (
        "redo",
        "Redo",
        "Restore the last undone edit.",
        &["Ctrl+Shift+Z", "Ctrl+Y"],
    ),
    (
        "delete",
        "Delete",
        "Delete the selected sheet items.",
        &["Backspace", "Del"],
    ),
    (
        "duplicate",
        "Duplicate",
        "Make a copy of the selected items.",
        &["Ctrl+D"],
    ),
    (
        "lock",
        "Lock items",
        "Toggle the lock on the selected items.",
        &["Ctrl+L"],
    ),
    (
        "group",
        "Group",
        "Group the selected sheet items.",
        &["Ctrl+G"],
    ),
    (
        "ungroup",
        "Ungroup",
        "Separate the selected group into its items.",
        &["Ctrl+Shift+G"],
    ),
    (
        "copy_style",
        "Copy style",
        "Copy the selected item's appearance.",
        &["Ctrl+Shift+C"],
    ),
    (
        "paste_style",
        "Paste style",
        "Apply the copied appearance.",
        &["Ctrl+Shift+V"],
    ),
    (
        "previous",
        "Previous sheet",
        "Show the preceding sheet.",
        &["Ctrl+PgUp"],
    ),
    (
        "next",
        "Next sheet",
        "Show the following sheet.",
        &["Ctrl+PgDown"],
    ),
    (
        "model",
        "Back to the model",
        "Leave Sheets and show the model.",
        &["Ctrl+Tab", "Meta+Tab", "Ctrl+Shift+Tab"],
    ),
    (
        "rename",
        "Rename sheet",
        "Edit the current sheet's name.",
        &["F2"],
    ),
];

/// Resolve the workspace at invocation; a shortcut or an F3 row never owns
/// the document that happened to be active when the catalogue was built.
#[pyclass]
struct SheetRun {
    window_ref: Py<PyAny>,
    command: String,
    tool: bool,
}

#[pymethods]
impl SheetRun {
    fn __call__(&self, py: Python<'_>) -> PyResult<()> {
        let win = self.window_ref.call0(py)?.into_bound(py);
        if win.is_none() {
            return Ok(());
        }
        let doc = win.getattr("_doc")?;
        if doc.is_none() {
            return Ok(());
        }
        let view = doc.getattr("stand_in")?;
        let ws = py
            .import("ingetrazo_render.sheet_workspace")?
            .call_method1("workspace_of", (view,))?;
        if !ws.is_none() {
            if self.tool {
                ws.call_method1("action", (format!("tool\x1f{}", self.command),))?;
            } else {
                ws.call_method0("_command_callbacks")?
                    .get_item(&self.command)?
                    .call0()?;
            }
        }
        Ok(())
    }
}

#[pyfunction]
pub fn sheet_command_scope<'py>(win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = win.py();
    let attrs = win.getattr("__dict__")?;
    if attrs.contains("_sheet_command_scope")? {
        return attrs.get_item("_sheet_command_scope");
    }
    let window_ref = py.import("weakref")?.call_method1("ref", (win,))?.unbind();
    let owner = py.import("types")?.call_method0("SimpleNamespace")?;
    let store = py
        .import("ingetrazo_core")?
        .getattr("WindowCommands")?
        .call0()?;
    owner.setattr("_command_store", store)?;
    owner.setattr("_command_actions", PyDict::new(py))?;
    let registry = PyDict::new(py);
    owner.setattr("_registry_actions", &registry)?;
    let paths = PyDict::new(py);
    owner.setattr("_command_paths", &paths)?;
    let tr = py.import("ingetrazo_core.i18n")?.getattr("tr")?;
    let bind =
        |id: &str, label: &str, path: &str, tip: &str, keys: &[&str], tool: bool| -> PyResult<()> {
            let action = crate::shell_commands::make(&owner, &tr.call1((label,))?, None, None, false, true)?;
            let key = format!("sheet:{id}");
            action.call_method1("setObjectName", (&key,))?;
            action.call_method1("setProperty", ("statusTip", tr.call1((tip,))?))?;
            action.call_method1("setShortcuts", (keys.to_vec(),))?;
            action.setattr(
                "run",
                Py::new(
                    py,
                    SheetRun {
                        window_ref: window_ref.clone_ref(py),
                        command: id.into(),
                        tool,
                    },
                )?,
            )?;
            registry.set_item(&key, &action)?;
            paths.set_item(key, tr.call1((path,))?)
        };
    let mut page = crate::sheet_page::Page::default();
    for (group, tools) in crate::sheet_workspace::RAIL.iter().enumerate() {
        let path = ["Navigation", "Sheet items", "Draw", "Dimensions"]
            .get(group)
            .copied()
            .unwrap_or("Sheets");
        for &(tool, name, _) in *tools {
            tool.clone_into(&mut page.tool);
            bind(tool, name, path, page.hint(), &[], true)?;
        }
    }
    for &(id, name, tip, keys) in EDITS {
        bind(id, name, "Sheet", tip, keys, false)?;
    }
    let shortcuts = py.import("ingetrazo_render.keyboard_bindings")?;
    shortcuts.call_method1("remember_defaults", (&owner,))?;
    shortcuts.call_method1("apply_user_shortcuts", (&owner,))?;
    win.setattr("_sheet_command_scope", &owner)?;
    Ok(owner)
}
