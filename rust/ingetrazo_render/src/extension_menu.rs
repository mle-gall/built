//! Per-window extension menu contributions. The plugin may retain and change
//! its action after setup. Both menu surfaces read this same ordered tree.
use pyo3::{exceptions::PyValueError, prelude::*, PyTraverseError, PyVisit};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
};

/// Stable id, path below Plugins, caption, default key, checked (-1 for a
/// plain command), enabled. A blank caption denotes a separator.
pub type Row = (String, Vec<String>, String, String, i8, bool);

#[derive(Default)]
struct State {
    nodes: Vec<Node>,
    next: HashMap<String, u64>,
    revision: u64,
    binding_revision: u64,
    bindings: HashMap<String, String>,
    registered: Option<u64>,
}

struct Node {
    owner: String,
    id: String,
    parent: Option<usize>,
    kind: Kind,
    label: String,
    shortcut: String,
    tip: String,
    enabled: bool,
    visible: bool,
    removed: bool,
    mark: i8,
    legacy: Option<Py<PyAny>>,
    triggered: Vec<Callback>,
    toggled: Vec<Callback>,
}
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Menu,
    Action,
    Separator,
}

struct Callback {
    handler: Py<PyAny>,
    takes_check: bool,
}
impl Callback {
    fn new(handler: &Bound<'_, PyAny>, signal: bool) -> PyResult<Self> {
        if !handler.is_callable() {
            return Err(PyValueError::new_err("expected a menu callback"));
        }
        // Match Qt's shorter-slot convention without running the callback twice
        // if the plugin itself raises TypeError.
        let takes_check = signal
            && handler
                .py()
                .import("inspect")?
                .call_method1("signature", (handler,))
                .is_ok_and(|signature| signature.call_method1("bind", (false,)).is_ok());
        Ok(Self {
            handler: handler.clone().unbind(),
            takes_check,
        })
    }
    fn copy(&self, py: Python<'_>) -> Self {
        Self {
            handler: self.handler.clone_ref(py),
            takes_check: self.takes_check,
        }
    }
    fn emit(&self, py: Python<'_>, checked: bool) -> PyResult<()> {
        if self.takes_check {
            self.handler.call1(py, (checked,))?;
        } else {
            self.handler.call0(py)?;
        }
        Ok(())
    }
}

impl State {
    fn rows(&self) -> Vec<Row> {
        self.nodes
            .iter()
            .filter_map(|n| {
                if n.kind == Kind::Menu || n.removed || !n.visible {
                    return None;
                }
                let mut enabled = n.enabled;
                let mut path = Vec::new();
                let mut parent = n.parent;
                while let Some(i) = parent {
                    let menu = &self.nodes[i];
                    if menu.removed || !menu.visible {
                        return None;
                    }
                    path.push(menu.label.clone());
                    enabled &= menu.enabled;
                    parent = menu.parent;
                }
                path.reverse();
                Some((
                    n.id.clone(),
                    path,
                    n.label.clone(),
                    n.shortcut.clone(),
                    n.mark,
                    enabled,
                ))
            })
            .collect()
    }
    fn add(&mut self, owner: &str, parent: Option<usize>, kind: Kind, label: String) -> usize {
        let seq = self.next.entry(owner.into()).or_default();
        let id = format!("extension:{owner}:{seq}");
        *seq += 1;
        let idx = self.nodes.len();
        self.nodes.push(Node {
            owner: owner.into(),
            id,
            parent,
            kind,
            label,
            shortcut: String::new(),
            tip: String::new(),
            enabled: true,
            visible: true,
            removed: false,
            mark: -1,
            legacy: None,
            triggered: vec![],
            toggled: vec![],
        });
        self.revision += 1;
        idx
    }
}

type Tree = Rc<RefCell<State>>;
#[derive(Clone)]
struct Handle {
    state: Weak<RefCell<State>>,
    index: usize,
}
impl Handle {
    fn tree(&self) -> PyResult<Tree> {
        self.state
            .upgrade()
            .ok_or_else(|| PyValueError::new_err("extension window closed"))
    }
    fn read<T>(&self, f: impl FnOnce(&Node) -> T) -> PyResult<T> {
        let tree = self.tree()?;
        let s = tree.borrow();
        Ok(f(s.nodes.get(self.index).ok_or_else(|| {
            PyValueError::new_err("extension action removed")
        })?))
    }
    fn update<T>(&self, f: impl FnOnce(&mut Node) -> T) -> PyResult<T> {
        let tree = self.tree()?;
        let mut s = tree.borrow_mut();
        let node = s
            .nodes
            .get_mut(self.index)
            .ok_or_else(|| PyValueError::new_err("extension action removed"))?;
        let result = f(node);
        s.revision += 1;
        Ok(result)
    }
    fn callbacks(&self, py: Python<'_>, toggle: bool) -> PyResult<Vec<Callback>> {
        self.read(|n| {
            (if toggle { &n.toggled } else { &n.triggered })
                .iter()
                .map(|c| c.copy(py))
                .collect()
        })
    }
}

#[pyclass(unsendable)]
pub struct ExtensionMenus {
    state: Tree,
}
#[pymethods]
impl ExtensionMenus {
    #[new]
    fn new() -> Self {
        Self {
            state: Default::default(),
        }
    }
    #[getter]
    fn revision(&self) -> u64 {
        self.state.borrow().revision
    }
    fn root(&self, owner: String) -> ExtensionMenu {
        ExtensionMenu {
            state: Rc::downgrade(&self.state),
            owner,
            parent: None,
        }
    }
    pub fn rows(&self) -> Vec<Row> {
        let state = self.state.borrow();
        let mut rows = state.rows();
        for row in &mut rows {
            if let Some(key) = state.bindings.get(&row.0) {
                row.3.clone_from(key);
            }
        }
        rows
    }
    fn stamp(&self) -> (u64, u64) {
        let s = self.state.borrow();
        (s.revision, s.binding_revision)
    }
    fn bind_shortcut(&self, id: &str, key: &str) {
        let mut s = self.state.borrow_mut();
        if s.bindings.get(id).map(String::as_str) != Some(key) {
            s.bindings.insert(id.into(), key.into());
            s.binding_revision += 1;
        }
    }
    fn refresh(&self, win: &Bound<'_, PyAny>) -> PyResult<()> {
        let revision = self.revision();
        if self.state.borrow().registered != Some(revision) {
            win.call_method0("refresh_commands")?;
            self.state.borrow_mut().registered = Some(revision);
        }
        Ok(())
    }
    fn present(&self, view: &Bound<'_, PyAny>) -> PyResult<()> {
        let stamp = self.stamp();
        if view
            .getattr("_ui_extension_menu_sig")
            .and_then(|v| v.extract::<(u64, u64)>())
            .ok()
            != Some(stamp)
        {
            view.getattr("renderer")?
                .call_method1("ui_extension_menu", (self.rows(),))?;
            view.setattr("_ui_extension_menu_sig", stamp)?;
        }
        Ok(())
    }
    fn commands(&self) -> Vec<(String, String, String, String)> {
        self.state
            .borrow()
            .rows()
            .into_iter()
            .filter(|r| !r.2.is_empty())
            .map(|(id, path, label, keys, _, _)| {
                let mut full = vec!["Plugins".to_owned()];
                full.extend(path);
                (id, full.join(" ▸ "), label, keys)
            })
            .collect()
    }
    fn trigger(&self, py: Python<'_>, id: &str) -> PyResult<bool> {
        let index = self
            .state
            .borrow()
            .nodes
            .iter()
            .position(|n| n.id == id && n.kind == Kind::Action);
        let Some(i) = index else { return Ok(false) };
        let enabled = self.rows().into_iter().any(|r| r.0 == id && r.5);
        if enabled {
            ExtensionAction(Handle {
                state: Rc::downgrade(&self.state),
                index: i,
            })
            .trigger(py)?;
        }
        Ok(true)
    }
    fn discard(&self, owner: &str) {
        let mut s = self.state.borrow_mut();
        for n in &mut s.nodes {
            if n.owner == owner {
                n.removed = true;
                n.enabled = false;
                n.triggered.clear();
                n.toggled.clear();
            }
        }
        s.revision += 1;
    }
    fn clear(&mut self) {
        self.state = Default::default();
    }
    fn __clear__(&mut self) {
        self.clear();
    }
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        if let Ok(s) = self.state.try_borrow() {
            for n in &s.nodes {
                if let Some(action) = &n.legacy {
                    visit.call(action)?;
                }
                for cb in n.triggered.iter().chain(&n.toggled) {
                    visit.call(&cb.handler)?;
                }
            }
        }
        Ok(())
    }
}

#[pyclass(unsendable)]
struct ExtensionMenu {
    state: Weak<RefCell<State>>,
    owner: String,
    parent: Option<usize>,
}
impl ExtensionMenu {
    fn add(&self, kind: Kind, label: String) -> PyResult<Handle> {
        let tree = self
            .state
            .upgrade()
            .ok_or_else(|| PyValueError::new_err("extension window closed"))?;
        let index = tree.borrow_mut().add(&self.owner, self.parent, kind, label);
        Ok(Handle {
            state: self.state.clone(),
            index,
        })
    }
}
#[pymethods]
#[allow(non_snake_case)]
impl ExtensionMenu {
    fn addMenu(&self, title: String) -> PyResult<Self> {
        let h = self.add(Kind::Menu, title)?;
        Ok(Self {
            state: h.state,
            owner: self.owner.clone(),
            parent: Some(h.index),
        })
    }
    fn addSeparator(&self) -> PyResult<ExtensionAction> {
        Ok(ExtensionAction(self.add(Kind::Separator, String::new())?))
    }
    #[pyo3(signature=(text,callback=None))]
    fn addAction(
        &self,
        text: &Bound<'_, PyAny>,
        callback: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<ExtensionAction> {
        let cb = callback.map(|f| Callback::new(f, true)).transpose()?;
        let label = if let Ok(label) = text.extract::<String>() {
            label
        } else {
            let label = text.call_method0("text")?.extract()?;
            let h = self.add(Kind::Action, label)?;
            let mirror = Py::new(text.py(), LegacyMirror(h.clone()))?;
            h.update(|n| n.legacy = Some(text.clone().unbind()))?;
            let mirror = mirror.bind(text.py());
            let connect = || -> PyResult<()> {
                mirror.call_method0("changed")?;
                text.getattr("changed")?
                    .call_method1("connect", (mirror.getattr("changed")?,))?;
                text.getattr("destroyed")?
                    .call_method1("connect", (mirror.getattr("deleted")?,))?;
                Ok(())
            };
            if let Err(error) = connect() {
                h.update(|n| {
                    n.removed = true;
                    n.legacy = None;
                })?;
                return Err(error);
            }
            return Ok(ExtensionAction(h));
        };
        let h = self.add(Kind::Action, label)?;
        if let Some(cb) = cb {
            h.update(|n| n.triggered.push(cb))?;
        }
        Ok(ExtensionAction(h))
    }
    #[pyo3(signature=(text,callback,shortcut=None,tip=None))]
    fn add_command(
        &self,
        text: String,
        callback: &Bound<'_, PyAny>,
        shortcut: Option<String>,
        tip: Option<String>,
    ) -> PyResult<ExtensionAction> {
        let cb = Callback::new(callback, false)?;
        let h = self.add(Kind::Action, text)?;
        h.update(|n| {
            n.triggered.push(cb);
            n.shortcut = shortcut.unwrap_or_default();
            n.tip = tip.unwrap_or_default();
        })?;
        Ok(ExtensionAction(h))
    }
    fn menuAction(&self) -> PyResult<ExtensionAction> {
        Ok(ExtensionAction(Handle {
            state: self.state.clone(),
            index: self
                .parent
                .ok_or_else(|| PyValueError::new_err("root menu has no action"))?,
        }))
    }
    fn clear(&self) -> PyResult<()> {
        let tree = self
            .state
            .upgrade()
            .ok_or_else(|| PyValueError::new_err("extension window closed"))?;
        let mut s = tree.borrow_mut();
        for n in &mut s.nodes {
            if n.owner == self.owner && n.parent == self.parent {
                n.removed = true;
            }
        }
        s.revision += 1;
        Ok(())
    }
    fn removeAction(&self, action: &ExtensionAction) -> PyResult<()> {
        if self.state.ptr_eq(&action.0.state) {
            action.0.update(|n| {
                if n.parent == self.parent && n.owner == self.owner {
                    n.removed = true;
                }
            })?;
        }
        Ok(())
    }
}
#[pyclass(unsendable)]
struct ExtensionAction(Handle);
#[pymethods]
#[allow(non_snake_case)]
impl ExtensionAction {
    #[getter]
    fn triggered(&self) -> ActionSignal {
        ActionSignal {
            handle: self.0.clone(),
            toggle: false,
        }
    }
    #[getter]
    fn toggled(&self) -> ActionSignal {
        ActionSignal {
            handle: self.0.clone(),
            toggle: true,
        }
    }
    fn setCheckable(&self, on: bool) -> PyResult<()> {
        self.0
            .update(|n| n.mark = if on { n.mark.max(0) } else { -1 })
    }
    fn isCheckable(&self) -> PyResult<bool> {
        self.0.read(|n| n.mark >= 0)
    }
    fn isChecked(&self) -> PyResult<bool> {
        self.0.read(|n| n.mark == 1)
    }
    fn setChecked(&self, py: Python<'_>, on: bool) -> PyResult<()> {
        if self.isCheckable()? && self.isChecked()? != on {
            self.0.update(|n| n.mark = i8::from(on))?;
            for cb in self.0.callbacks(py, true)? {
                cb.emit(py, on)?;
            }
        }
        Ok(())
    }
    fn setEnabled(&self, on: bool) -> PyResult<()> {
        self.0.update(|n| n.enabled = on)
    }
    fn isEnabled(&self) -> PyResult<bool> {
        self.0.read(|n| n.enabled)
    }
    fn setVisible(&self, on: bool) -> PyResult<()> {
        self.0.update(|n| n.visible = on)
    }
    fn isVisible(&self) -> PyResult<bool> {
        self.0.read(|n| n.visible)
    }
    fn setText(&self, label: String) -> PyResult<()> {
        self.0.update(|n| n.label = label)
    }
    fn text(&self) -> PyResult<String> {
        self.0.read(|n| n.label.clone())
    }
    fn setStatusTip(&self, tip: String) -> PyResult<()> {
        self.0.update(|n| n.tip = tip)
    }
    fn statusTip(&self) -> PyResult<String> {
        self.0.read(|n| n.tip.clone())
    }
    fn setShortcut(&self, key: String) -> PyResult<()> {
        self.0.update(|n| n.shortcut = key)
    }
    fn trigger(&self, py: Python<'_>) -> PyResult<()> {
        if !self.isEnabled()? {
            return Ok(());
        }
        if let Some(action) = self
            .0
            .read(|n| n.legacy.as_ref().map(|a| a.clone_ref(py)))?
        {
            action.call_method0(py, "trigger")?;
            return Ok(());
        }
        if self.isCheckable()? {
            self.setChecked(py, !self.isChecked()?)?;
        }
        let on = self.isChecked()?;
        for cb in self.0.callbacks(py, false)? {
            cb.emit(py, on)?;
        }
        Ok(())
    }
}
/// Observe existing plugin-created actions without loading Qt for native ones.
#[pyclass(unsendable)]
struct LegacyMirror(Handle);
#[pymethods]
impl LegacyMirror {
    fn changed(&self, py: Python<'_>) -> PyResult<()> {
        if self.0.state.strong_count() == 0 {
            return Ok(());
        }
        let Some(raw) = self
            .0
            .read(|n| n.legacy.as_ref().map(|a| a.clone_ref(py)))?
        else {
            return Ok(());
        };
        let a = raw.bind(py);
        let label = a.call_method0("text")?.extract()?;
        let enabled = a.call_method0("isEnabled")?.extract()?;
        let visible = a.call_method0("isVisible")?.extract()?;
        let tip = a.call_method0("statusTip")?.extract()?;
        let shortcut = a
            .call_method0("shortcut")?
            .call_method0("toString")?
            .extract()?;
        let mark = if a.call_method0("isCheckable")?.is_truthy()? {
            i8::from(a.call_method0("isChecked")?.is_truthy()?)
        } else {
            -1
        };
        self.0.update(|n| {
            n.label = label;
            n.enabled = enabled;
            n.visible = visible;
            n.tip = tip;
            n.shortcut = shortcut;
            n.mark = mark;
        })
    }
    #[pyo3(signature=(*_args))]
    fn deleted(&self, _args: &Bound<'_, pyo3::types::PyTuple>) -> PyResult<()> {
        if self.0.state.strong_count() > 0 {
            self.0.update(|n| {
                n.removed = true;
                n.legacy = None;
            })?;
        }
        Ok(())
    }
}

#[pyclass(unsendable)]
struct ActionSignal {
    handle: Handle,
    toggle: bool,
}
#[pymethods]
impl ActionSignal {
    fn connect(&self, f: &Bound<'_, PyAny>) -> PyResult<()> {
        let cb = Callback::new(f, true)?;
        self.handle.update(|n| {
            if self.toggle {
                n.toggled.push(cb)
            } else {
                n.triggered.push(cb)
            }
        })
    }
    fn disconnect(&self, f: &Bound<'_, PyAny>) -> PyResult<()> {
        self.handle.update(|n| {
            (if self.toggle {
                &mut n.toggled
            } else {
                &mut n.triggered
            })
            .retain(|cb| !cb.handler.bind(f.py()).is(f))
        })
    }
}
