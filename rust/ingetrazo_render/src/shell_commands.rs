// SPDX-License-Identifier: GPL-3.0-or-later
//! The native window's commands: [`ShellAction`] binds one entry of the
//! window's Rust command store (`ingetrazo_core.WindowCommands`) to its
//! handler and to the shared shortcut editor, and [`bind_window_commands`]
//! binds the command registry ([`crate::commands::command_registry`]) to the
//! window's tool actions and to the installed menu bar (`built_shell`).
//!
//! The keyboard bindings persist factory and user shortcuts for these actions.
use pyo3::{prelude::*, types::PyDict};

fn opt<'py>(o: &Bound<'py, PyAny>, name: &str) -> Option<Bound<'py, PyAny>> {
    o.getattr(name).ok().filter(|v| !v.is_none())
}

/// A window command: the store's state for it and what runs it. Unknown
/// attributes are the state's (`text()`, `isEnabled()`, `property()`…), as
/// for a QAction.
#[pyclass(module = "ingetrazo_render", dict, weakref)]
pub struct ShellAction {
    #[pyo3(get)]
    window: Py<PyAny>,
    #[pyo3(get)]
    state: Py<PyAny>,
    #[pyo3(get, set)]
    run: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    toggled: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    changed: Option<Py<PyAny>>,
}

fn call_opt<'py, A: pyo3::call::PyCallArgs<'py>>(py: Python<'py>, f: &Option<Py<PyAny>>, args: A) -> PyResult<()> {
    if let Some(f) = f {
        f.bind(py).call1(args)?;
    }
    Ok(())
}

#[pymethods]
impl ShellAction {
    fn __getattr__<'py>(&self, py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyAny>> {
        self.state.bind(py).getattr(name)
    }

    fn parent(&self, py: Python<'_>) -> Py<PyAny> {
        self.window.clone_ref(py)
    }

    #[allow(non_snake_case)]
    fn toolTip<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.state.bind(py).call_method0("text")
    }

    #[allow(non_snake_case)]
    fn statusTip(&self, py: Python<'_>) -> PyResult<String> {
        let tip = self.state.bind(py).call_method1("property", ("statusTip",))?;
        Ok(if tip.is_truthy()? { tip.extract()? } else { String::new() })
    }

    fn menu(&self) -> Option<Py<PyAny>> {
        None
    }

    #[allow(non_snake_case)]
    fn isSeparator(&self) -> bool {
        false
    }

    /// The first key, portable text ("Ctrl+Shift+S"); empty with none.
    fn shortcut(&self, py: Python<'_>) -> PyResult<String> {
        let keys: Vec<String> = self.state.bind(py).call_method0("portable_keys")?.extract()?;
        Ok(keys.into_iter().next().unwrap_or_default())
    }

    /// Every key, portable text.
    fn shortcuts(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        self.state.bind(py).call_method0("portable_keys")?.extract()
    }

    #[allow(non_snake_case)]
    fn setShortcuts(&self, py: Python<'_>, sequences: &Bound<'_, PyAny>) -> PyResult<()> {
        let texts: Vec<String> = crate::keyboard_bindings::portable_keys(sequences)?;
        let keys: Vec<(String, Option<u32>)> = texts
            .iter()
            .filter_map(|t| crate::shortcut_text::shortcut_text(t))
            .map(|(portable, _, code, _)| (portable, code))
            .collect();
        if self.state.bind(py).call_method1("set_keys", (keys,))?.is_truthy()? {
            call_opt(py, &self.changed, ())?;
        }
        Ok(())
    }

    #[allow(non_snake_case)]
    fn setEnabled(&self, py: Python<'_>, enabled: bool) -> PyResult<()> {
        if self.state.bind(py).call_method1("set_enabled", (enabled,))?.is_truthy()? {
            call_opt(py, &self.changed, ())?;
        }
        Ok(())
    }

    #[allow(non_snake_case)]
    fn setChecked(&self, py: Python<'_>, checked: bool) -> PyResult<()> {
        if self.state.bind(py).call_method1("set_checked", (checked,))?.is_truthy()? {
            call_opt(py, &self.toggled, (checked,))?;
        }
        Ok(())
    }

    fn trigger(&self, py: Python<'_>) -> PyResult<()> {
        let state = self.state.bind(py);
        if state.call_method0("isEnabled")?.is_truthy()? {
            self.setChecked(py, !state.call_method0("isChecked")?.is_truthy()?)?;
            call_opt(py, &self.run, ())?;
        }
        Ok(())
    }

    fn remove(&mut self, py: Python<'_>) -> PyResult<()> {
        let window = self.window.bind(py);
        let id = self.state.bind(py).getattr("id")?;
        window.getattr("_command_store")?.call_method1("remove", (&id,))?;
        window.getattr("_command_actions")?.call_method1("pop", (&id, py.None()))?;
        self.changed = None;
        self.run = None;
        self.toggled = None;
        Ok(())
    }
}

/// `ShellAction(window, label, run=…, toggled=…, checkable=…, listed=…)`:
/// listed actions are also the window's `_command_actions[id]`.
#[pyfunction]
#[pyo3(signature = (window, label, *, run=None, toggled=None, checkable=false, listed=true))]
fn shell_action<'py>(
    window: &Bound<'py, PyAny>,
    label: &Bound<'py, PyAny>,
    run: Option<Py<PyAny>>,
    toggled: Option<Py<PyAny>>,
    checkable: bool,
    listed: bool,
) -> PyResult<Bound<'py, ShellAction>> {
    make(window, label, run, toggled, checkable, listed)
}

impl ShellAction {
    fn new(window: &Bound<'_, PyAny>, label: &Bound<'_, PyAny>, run: Option<Py<PyAny>>, toggled: Option<Py<PyAny>>, checkable: bool) -> PyResult<Self> {
        let state = window.getattr("_command_store")?.call_method1("add", (label, checkable))?;
        Ok(Self { window: window.clone().unbind(), state: state.unbind(), run, toggled, changed: None })
    }
}

pub fn make<'py>(
    window: &Bound<'py, PyAny>,
    label: &Bound<'py, PyAny>,
    run: Option<Py<PyAny>>,
    toggled: Option<Py<PyAny>>,
    checkable: bool,
    listed: bool,
) -> PyResult<Bound<'py, ShellAction>> {
    let action = Bound::new(window.py(), ShellAction::new(window, label, run, toggled, checkable)?)?;
    if listed {
        let id = action.borrow().state.bind(window.py()).getattr("id")?;
        window.getattr("_command_actions")?.set_item(id, &action)?;
    }
    Ok(action)
}

// ---- the registry on the window ---------------------------------------------------------

/// What a registry command runs: a tool, a camera command, the installed
/// menu item, or the window's command by name.
#[pyclass(module = "ingetrazo_render")]
struct RegistryRun {
    win: Py<PyAny>,
    key: String,
}

#[pymethods]
impl RegistryRun {
    fn __call__(&self, py: Python<'_>) -> PyResult<()> {
        run_registered(self.win.bind(py), &self.key)
    }
}

/// Mirror a registry command's keys onto the menu item and the view bindings.
#[pyclass(module = "ingetrazo_render")]
struct RegistryReflect {
    win: Py<PyAny>,
    key: String,
}

#[pymethods]
impl RegistryReflect {
    fn __call__(&self, py: Python<'_>) -> PyResult<()> {
        let win = self.win.bind(py);
        reflect(win, &self.key)?;
        crate::commands::invalidate_command_rails(win)
    }
}

fn menu_id<'py>(act: &Bound<'py, PyAny>) -> PyResult<Option<String>> {
    let id = act.call_method1("property", ("nativeMenuId",))?;
    Ok(if id.is_none() { None } else { Some(id.extract()?) })
}

fn run_registered(win: &Bound<'_, PyAny>, key: &str) -> PyResult<()> {
    let py = win.py();
    let act = win.getattr("_registry_actions")?.get_item(key)?;
    let menu = menu_id(&act)?;
    let item = match &menu {
        Some(id) => Some(win.getattr("_menu_items")?.call_method1("get", (id,))?).filter(|i| !i.is_none()),
        None => None,
    };
    if key == "command_search" {
        let view = win.getattr("_host")?.getattr("view")?;
        view.getattr("renderer")?.call_method0("ui_open_commands")?;
        view.call_method0("update")?;
    } else if let Some(tool) = key.strip_prefix("tool:") {
        win.call_method1("_activate_tool", (tool,))?;
    } else if let Some(view) = key.strip_prefix("view:") {
        py.import("views.view_commands")?.call_method1("run", (win, view))?;
    } else if let Some(item) = item {
        item.call_method0("trigger")?;
    } else if let Some(id) = menu.filter(|m| !m.is_empty()) {
        win.call_method1("run_command", (id,))?;
    }
    Ok(())
}

fn reflect(win: &Bound<'_, PyAny>, key: &str) -> PyResult<()> {
    let py = win.py();
    let act = win.getattr("_registry_actions")?.get_item(key)?;
    let keys: Vec<String> = act.call_method0("portable_keys")?.extract()?;
    let portable = keys.first().cloned().unwrap_or_default();
    let menu = menu_id(&act)?;
    if let Some(id) = menu.as_deref().filter(|m| m.starts_with("extension:")) {
        win.getattr("_extension_menus")?.call_method1("bind_shortcut", (id, &portable))?;
    }
    if let Some(id) = &menu {
        let item = win.getattr("_menu_items")?.call_method1("get", (id,))?;
        if !item.is_none() {
            let factory: String = item.getattr("keys")?.extract()?;
            let factory = crate::shortcut_text::shortcut_text(&factory).map(|s| s.0).unwrap_or_default();
            item.call_method1("set_shortcut", (&portable, portable == factory))?;
        }
    }
    if let Some(view) = key.strip_prefix("view:") {
        let dict = win.getattr("__dict__")?;
        let bindings = dict.call_method1("setdefault", ("_view_bindings", PyDict::new(py)))?;
        let shown = if keys.is_empty() { String::new() } else { crate::shortcut_text::shortcut_text(&portable).map(|s| s.1).unwrap_or_default() };
        let value = (shown, act.call_method0("isEnabled")?.is_truthy()?).into_pyobject(py)?;
        let current = bindings.call_method1("get", (view,))?;
        if current.is_none() || !current.eq(&value)? {
            bindings.set_item(view, value)?;
            let version: i64 = dict.call_method1("get", ("_view_binding_version", 0))?.extract()?;
            win.setattr("_view_binding_version", version + 1)?;
        }
    }
    Ok(())
}

/// Bind the command registry to the window: one action per command (a tool
/// action when the command is a tool's, else a new [`ShellAction`]), its
/// factory keys, the user's keys, its menu item and enabled state. Gone
/// commands are removed. Settings' Keyboard rebuilds when the set changed.
#[pyfunction]
fn bind_window_commands(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let tr = py.import("ingetrazo_core.i18n")?.getattr("tr")?;
    let shortcuts = py.import("ingetrazo_render.keyboard_bindings")?;
    let defaults_key = crate::keyboard_bindings::DEFAULTS;
    // The installed menu bar's items (the `built` binary), not extensions'.
    let native_items = PyDict::new(py);
    if let Ok(shell) = py.import("built_shell") {
        if let Ok(get) = shell.getattr("menu_commands") {
            for item in get.call0()?.try_iter()? {
                let item = item?;
                let id: String = item.getattr("id")?.extract()?;
                if !id.starts_with("extension:") {
                    native_items.set_item(id, item)?;
                }
            }
        }
    }
    win.setattr("_menu_items", &native_items)?;
    let mut menu: Vec<(String, String, String, String)> = Vec::new();
    for item in native_items.values().iter() {
        menu.push((item.getattr("id")?.extract()?, item.getattr("path")?.extract()?, item.getattr("name")?.extract()?, item.getattr("keys")?.extract()?));
    }
    let menus = win.getattr("__dict__")?.call_method1("get", ("_extension_menus",))?;
    let enabled = PyDict::new(py);
    let mut tips = None;
    if !menus.is_none() {
        menu.extend(menus.call_method0("commands")?.extract::<Vec<(String, String, String, String)>>()?);
        tips = Some(menus.call_method0("tips")?);
        for row in menus.call_method0("rows")?.try_iter()? {
            let row = row?;
            enabled.set_item(row.get_item(0)?, row.get_item(-1)?)?;
        }
    }
    let actions = win.getattr("_registry_actions")?.cast_into::<PyDict>()?;
    let old_keys: Vec<String> = actions.keys().extract()?;
    let mut previous: std::collections::HashSet<String> = old_keys.iter().cloned().collect();
    let paths = PyDict::new(py);
    let tools: Vec<(String, String, Vec<String>)> = win.getattr("_tool_defaults")?.extract()?;
    let tool_actions = win.getattr("_tool_actions")?;
    for (key, group, label, defaults, tool_id, menu_id) in crate::commands::command_registry(tools, menu) {
        let mut sections = Vec::new();
        for section in group.split(" ▸ ") {
            sections.push(tr.call1((section,))?.extract::<String>()?);
        }
        paths.set_item(&key, sections.join(" ▸ "))?;
        if !actions.contains(&key)? {
            let found = if tool_id.is_empty() { None } else { Some(tool_actions.call_method1("get", (&tool_id,))?).filter(|a| !a.is_none()) };
            let act = match found {
                Some(act) => {
                    act.call_method1("setProperty", ("legacyShortcutKey", shortcuts.call_method1("action_key", (&act,))?))?;
                    act
                }
                None => make(win, &tr.call1((&label,))?, None, None, false, true)?.into_any(),
            };
            act.setattr("run", Py::new(py, RegistryRun { win: win.clone().unbind(), key: key.clone() })?)?;
            act.call_method1("setObjectName", (&key,))?;
            actions.set_item(&key, &act)?;
            act.setattr("changed", Py::new(py, RegistryReflect { win: win.clone().unbind(), key: key.clone() })?)?;
        }
        let act = actions.get_item(&key)?.ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(key.clone()))?;
        let old = act.call_method1("property", (defaults_key,))?;
        let sequences: Vec<String> = defaults.iter().filter_map(|s| crate::shortcut_text::shortcut_text(s)).map(|s| s.0).collect();
        let portable = sequences.join("; ");
        if old.is_none() || old.extract::<String>().ok().as_deref() != Some(portable.as_str()) {
            let current: Vec<String> = act.call_method0("portable_keys")?.extract()?;
            act.call_method1("setProperty", (defaults_key, &portable))?;
            if old.is_none() || old.extract::<String>().ok() == Some(current.join("; ")) {
                act.call_method1("setShortcuts", (sequences,))?;
            }
        }
        act.call_method1("setProperty", ("nativeMenuId", &menu_id))?;
        if enabled.contains(&menu_id)? {
            act.call_method1("setText", (&label,))?;
            let tip = match &tips {
                Some(t) => t.call_method1("get", (&menu_id, ""))?,
                None => "".into_pyobject(py)?.into_any(),
            };
            act.call_method1("setProperty", ("statusTip", tip))?;
            act.call_method1("setEnabled", (enabled.get_item(&menu_id)?,))?;
        } else if let Some(item) = native_items.get_item(&menu_id)? {
            act.call_method1("setEnabled", (item.call_method0("isEnabled")?,))?;
        }
        previous.remove(&key);
    }
    for key in &previous {
        if let Some(dead) = actions.get_item(key)? {
            actions.del_item(key)?;
            dead.call_method0("remove")?;
        }
    }
    win.setattr("_command_paths", paths)?;
    shortcuts.call_method1("apply_user_shortcuts", (win,))?;
    let keys: Vec<String> = actions.keys().extract()?;
    for key in &keys {
        reflect(win, key)?;
    }
    let mut now = keys.clone();
    let mut before = old_keys;
    now.sort();
    before.sort();
    if now != before {
        let sheets = py.import("ingetrazo_render.chrome_sheet")?;
        let sheet = sheets.call_method1("current", (win,))?;
        if opt(&sheet, "id").is_some_and(|id| id.extract::<String>().ok().as_deref() == Some("settings")) {
            sheet.setattr("_actions", py.None())?;
            sheet.setattr("_paths", py.None())?;
            sheets.call_method1("refresh", (win,))?;
        }
    }
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<ShellAction>()?;
    m.add_function(wrap_pyfunction!(shell_action, m)?)?;
    m.add_function(wrap_pyfunction!(bind_window_commands, m)?)?;
    Ok(())
}
