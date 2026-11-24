//! Commands owned by a winit window. IDs stay valid across registry refreshes.
//! Single-stroke keys arrive in the same integer representation as winit tool events.
use pyo3::prelude::*;
use std::collections::BTreeMap;

#[derive(Default)]
#[pyclass(module = "ingetrazo_core")]
pub struct WindowCommands {
    next_id: u64,
    last: Option<(String, Py<PyAny>)>,
    actions: BTreeMap<u64, Py<WindowCommand>>,
    plugin_keys: BTreeMap<u32, Py<PyAny>>,
}

#[pymethods]
impl WindowCommands {
    #[new]
    fn new() -> Self {
        Self::default()
    }
    fn remember(&mut self, label: String, callback: Py<PyAny>) {
        self.last = Some((label, callback));
    }
    #[getter]
    fn last_command(&self, py: Python<'_>) -> Option<(String, Py<PyAny>)> {
        self.last
            .as_ref()
            .map(|(label, run)| (label.clone(), run.clone_ref(py)))
    }
    fn __traverse__(&self, visit: pyo3::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        if let Some((_, callback)) = &self.last {
            visit.call(callback)?;
        }
        for action in self.plugin_keys.values() {
            visit.call(action)?;
        }
        for action in self.actions.values() {
            visit.call(action)?;
        }
        Ok(())
    }
    fn __clear__(&mut self) {
        self.last = None;
        self.plugin_keys.clear();
        self.actions.clear();
    }
    fn add(
        &mut self,
        py: Python<'_>,
        label: String,
        checkable: bool,
    ) -> PyResult<Py<WindowCommand>> {
        let id = self.next_id;
        self.next_id += 1;
        let action = Py::new(
            py,
            WindowCommand {
                id,
                label,
                checkable,
                enabled: true,
                live: true,
                ..Default::default()
            },
        )?;
        self.actions.insert(id, action.clone_ref(py));
        Ok(action)
    }
    fn remove(&mut self, py: Python<'_>, id: u64) {
        if let Some(action) = self.actions.remove(&id) {
            action.borrow_mut(py).live = false;
        }
    }
    // Third-party actions are scoped to this window. Preserve tree order for
    // duplicate bindings. Built-in commands take priority at dispatch.
    fn replace_plugin_keys(&mut self, bindings: Vec<(u32, Py<PyAny>)>) {
        self.plugin_keys.clear();
        for (chord, action) in bindings {
            self.plugin_keys.entry(chord).or_insert(action);
        }
    }
    fn plugin_action(&self, py: Python<'_>, key: u32, modifiers: u32) -> Option<Py<PyAny>> {
        self.plugin_keys
            .get(&(key | modifiers))
            .map(|action| action.clone_ref(py))
    }
    fn resolve(&self, py: Python<'_>, key: u32, modifiers: u32) -> Option<u64> {
        let chord = key | modifiers;
        self.actions.iter().find_map(|(id, action)| {
            let action = action.borrow(py);
            (action.enabled && action.keys.iter().any(|(_, k)| *k == Some(chord))).then_some(*id)
        })
    }
}

#[derive(Default)]
#[pyclass(module = "ingetrazo_core")]
pub struct WindowCommand {
    #[pyo3(get)]
    id: u64,
    label: String,
    name: String,
    live: bool,
    enabled: bool,
    checked: bool,
    checkable: bool,
    blocked: bool,
    properties: BTreeMap<String, String>,
    keys: Vec<(String, Option<u32>)>,
}

#[pymethods]
#[allow(non_snake_case)]
impl WindowCommand {
    fn text(&self) -> &str {
        &self.label
    }
    fn setText(&mut self, label: String) {
        self.label = label;
    }
    fn objectName(&self) -> &str {
        &self.name
    }
    fn setObjectName(&mut self, name: String) {
        self.name = name;
    }
    fn isVisible(&self) -> bool {
        self.live
    }
    fn isEnabled(&self) -> bool {
        self.enabled && self.live
    }
    fn isCheckable(&self) -> bool {
        self.checkable
    }
    fn isChecked(&self) -> bool {
        self.checked
    }
    fn set_enabled(&mut self, on: bool) -> bool {
        std::mem::replace(&mut self.enabled, on) != on && !self.blocked
    }
    fn set_checked(&mut self, on: bool) -> bool {
        self.checkable && std::mem::replace(&mut self.checked, on) != on && !self.blocked
    }
    fn blockSignals(&mut self, on: bool) -> bool {
        std::mem::replace(&mut self.blocked, on)
    }
    fn property(&self, name: &str) -> Option<&str> {
        self.properties.get(name).map(String::as_str)
    }
    fn setProperty(&mut self, name: String, value: String) {
        self.properties.insert(name, value);
    }
    fn portable_keys(&self) -> Vec<&str> {
        self.keys.iter().map(|(s, _)| s.as_str()).collect()
    }
    fn set_keys(&mut self, keys: Vec<(String, Option<u32>)>) -> bool {
        if self.keys == keys {
            return false;
        }
        self.keys = keys;
        !self.blocked
    }
}
