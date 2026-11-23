// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The Python face of the document: `Placement` (the base class of
//! `core.group.Group`, and a scene's root), `Tag` (of `core.layers.Layer`),
//! `HandleList` (the live `g.children` / `scene.groups` / `scene.layers`)
//! and `Transaction` (one History step's journal).
//!
//! Every method follows the lock rule of `doc/mod.rs`: read or write under
//! the lock, build Python objects and release references after it.

use super::value::Value;
use super::{
    attr_id, drain_pending, handle_dropped, lock, str_view, try_lock, FVal, Field, Key, Kind, ListId, Op, Trash,
    Xform, VIEW_AXES, VIEW_MATERIAL, VIEW_XFORM,
};
use numpy::PyArrayMethods;
use pyo3::exceptions::{PyIndexError, PyRuntimeError, PyTypeError, PyValueError};
use pyo3::gc::PyVisit;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PySlice, PyTuple};
use pyo3::PyTraverseError;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

fn err(e: String) -> PyErr {
    if e.contains("no document node") {
        PyRuntimeError::new_err(e)
    } else if e.starts_with("one parent") {
        PyValueError::new_err(e)
    } else {
        PyTypeError::new_err(e)
    }
}

/// The tree-invariant reports of the list op that just ran, as Python
/// warnings (`PARENT_MODE` 1; see `doc_parent_mode`).
fn parent_reports(py: Python<'_>) -> PyResult<()> {
    let msgs: Vec<String> = {
        let mut r = super::PARENT_REPORTS.lock().unwrap_or_else(|e| e.into_inner());
        if r.is_empty() {
            return Ok(());
        }
        std::mem::take(&mut *r)
    };
    let cat = py.get_type::<pyo3::exceptions::PyUserWarning>();
    for m in msgs {
        let c = std::ffi::CString::new(m).unwrap_or_default();
        PyErr::warn(py, &cat, &c, 1)?;
    }
    Ok(())
}

/// `(make, bind)`: the Python side of matrix views (`core/doc_model.py`).
static HOOKS: Mutex<Option<(Py<PyAny>, Py<PyAny>)>> = Mutex::new(None);
/// `track(key, attr, container)`: the tracked container an attribute's dict
/// or list is handed out as (`core.doc_model.track`).
static TRACK: Mutex<Option<Py<PyAny>>> = Mutex::new(None);

fn hooks(py: Python<'_>) -> PyResult<(Py<PyAny>, Py<PyAny>)> {
    let h = HOOKS.lock().unwrap_or_else(|e| e.into_inner());
    match h.as_ref() {
        Some((a, b)) => Ok((a.clone_ref(py), b.clone_ref(py))),
        None => Err(PyRuntimeError::new_err("document matrix hooks not installed (core.doc_model)")),
    }
}

/// `refill(view, value) -> bool`: put `value` into the tracked container
/// `view` in place (`core.doc_model._refill`); False when it cannot.
static REFILL: Mutex<Option<Py<PyAny>>> = Mutex::new(None);

/// After a replay: refill the tracked containers it wrote (see
/// `Doc::refills`); one that cannot take the value stops being the view.
fn run_refills(py: Python<'_>) -> PyResult<()> {
    let todo = std::mem::take(&mut lock().refills);
    if todo.is_empty() {
        return Ok(());
    }
    let hook = REFILL.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|h| h.clone_ref(py));
    let mut first_err = None;
    for (k, a, view, val) in todo {
        let ok = match &hook {
            Some(h) => match val.to_py(py).and_then(|v| h.bind(py).call1((view.bind(py), v))) {
                Ok(r) => r.is_truthy().unwrap_or(false),
                Err(e) => {
                    first_err.get_or_insert(e);
                    false
                }
            },
            None => false,
        };
        if !ok {
            let gone = {
                let mut d = lock();
                d.node_mut(k).and_then(|n| {
                    let i = n.attr_views.iter().position(|(x, v)| *x == a && v.as_ptr() == view.as_ptr())?;
                    Some(n.attr_views.swap_remove(i).1)
                })
            };
            drop(gone);
        }
        drop(view);
    }
    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Install the refill hook (`core.doc_model._refill`).
#[pyfunction]
fn doc_set_refill_hook(refill: Py<PyAny>) {
    let old = REFILL.lock().unwrap_or_else(|e| e.into_inner()).replace(refill);
    drop(old);
}

fn track_hook(py: Python<'_>) -> Option<Py<PyAny>> {
    TRACK.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|t| t.clone_ref(py))
}

// ------------------------------------------------------------- attributes

/// An attribute's value as Python: the tracked container cached for it,
/// or (a dict or a list, `track`) a new one, or a plain value.
fn read_attr<'py>(py: Python<'py>, me: &Bound<'py, PyAny>, k: Key, a: u16, track: bool) -> PyResult<Bound<'py, PyAny>> {
    let (v, serial) = {
        let mut d = lock();
        if !me.is_none() {
            d.set_handle(k, me.as_ptr());
        }
        let n = d.need(k).map_err(err)?;
        if let Some(c) = n.attr_view(a) {
            return Ok(c.clone_ref(py).into_bound(py));
        }
        (n.attr(a).cloned(), d.serial)
    };
    let Some(v) = v else { return Ok(py.None().into_bound(py)) };
    let container = matches!(v, Value::Dict(_) | Value::List(_));
    let out = v.to_py(py)?;
    if !(track && container) {
        return Ok(out);
    }
    let Some(hook) = track_hook(py) else { return Ok(out) };
    let view = hook.bind(py).call1((k, a, out))?;
    let keep = view.clone().unbind();
    let stale = {
        let mut d = lock();
        let same = d.serial == serial;
        match d.node_mut(k) {
            Some(n) if same && n.attr_view(a).is_none() => {
                n.attr_views.push((a, keep));
                None
            }
            Some(n) if n.attr_view(a).is_some() => {
                let other = n.attr_view(a).map(|v| v.clone_ref(py));
                drop(d);
                drop(keep);
                return Ok(other.expect("checked").into_bound(py));
            }
            _ => Some(keep),
        }
    };
    drop(stale);
    Ok(view)
}

fn write_attr(py: Python<'_>, me: &Bound<'_, PyAny>, k: Key, a: u16, value: &Bound<'_, PyAny>) -> PyResult<()> {
    {
        let d = lock();
        if let Some(n) = d.node(k) {
            if n.attr_view(a).map(|v| v.as_ptr()) == Some(value.as_ptr()) {
                return Ok(()); // its own tracked container assigned back
            }
        }
    }
    let v = if value.is_none() { None } else { Some(Value::from_py(value)?) };
    let mut trash = Trash::default();
    let r = {
        let mut d = lock();
        if !me.is_none() {
            d.set_handle(k, me.as_ptr());
        }
        d.write(py, k, Field::Attr(a), FVal::Val(v), &mut trash)
    };
    drop(trash);
    drain_pending();
    r.map(|_| ()).map_err(err)
}

fn handle_key(obj: &Bound<'_, PyAny>) -> PyResult<Key> {
    if let Ok(p) = obj.cast::<PyPlacement>() {
        return Ok(p.get().key);
    }
    if let Ok(t) = obj.cast::<PyTag>() {
        return Ok(t.get().key);
    }
    if let Ok(m) = obj.cast::<PyMaterial>() {
        return Ok(m.get().key);
    }
    Err(PyTypeError::new_err(format!("{} is not a document handle", obj.get_type().name()?)))
}

/// A class attribute that keeps an instance's value in the document
/// (`Field::Attr`): `Group.ifc = DocAttr("ifc", track=True)`. With
/// `track`, a dict or list value is handed out as a tracked container
/// that writes back when edited in place (`g.ext["link"]["x"] = 1`).
#[pyclass(module = "ingetrazo_core", name = "DocAttr", frozen)]
pub struct DocAttr {
    id: u16,
    name: String,
    track: bool,
}

#[pymethods]
impl DocAttr {
    #[new]
    #[pyo3(signature = (name, track = false))]
    fn new(name: &str, track: bool) -> PyResult<Self> {
        Ok(DocAttr { id: attr_id(name).map_err(PyValueError::new_err)?, name: name.to_owned(), track })
    }

    #[getter]
    fn id(&self) -> u16 {
        self.id
    }

    #[getter]
    fn name(&self) -> &str {
        &self.name
    }

    fn __get__<'py>(slf: &Bound<'py, Self>, obj: Option<&Bound<'py, PyAny>>, _owner: Option<&Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyAny>> {
        let Some(obj) = obj.filter(|o| !o.is_none()) else { return Ok(slf.clone().into_any()) };
        let me = slf.get();
        read_attr(slf.py(), obj, handle_key(obj)?, me.id, me.track)
    }

    fn __set__(&self, obj: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        write_attr(obj.py(), obj, handle_key(obj)?, self.id, value)
    }

    fn __delete__(&self, obj: &Bound<'_, PyAny>) -> PyResult<()> {
        write_attr(obj.py(), obj, handle_key(obj)?, self.id, &obj.py().None().into_bound(obj.py()))
    }
}

// ------------------------------------------------------------- conversions

fn str_of(v: &Bound<'_, PyAny>) -> PyResult<Option<String>> {
    if v.is_none() {
        Ok(None)
    } else {
        Ok(Some(v.extract::<String>()?))
    }
}

fn xform_of(data: &Bound<'_, PyAny>, src: &Bound<'_, PyAny>) -> PyResult<Xform> {
    let vals: Vec<f64> = data.extract()?;
    if vals.len() != 16 {
        return Err(PyValueError::new_err("a placement matrix has 16 values"));
    }
    let mut f = [0f32; 16];
    for (o, v) in f.iter_mut().zip(vals) {
        *o = v as f32;
    }
    let src = if src.is_none() {
        None
    } else {
        let (a, b): (Vec<f64>, Vec<f64>) = src.extract()?;
        if a.len() != 16 || b.len() != 16 {
            return Err(PyValueError::new_err("_built_source: two 16-value sequences"));
        }
        let mut x = [0f64; 16];
        let mut y = [0f64; 16];
        x.copy_from_slice(&a);
        y.copy_from_slice(&b);
        Some(Box::new((x, y)))
    };
    Ok(Xform { f, src })
}

fn fval_to_py<'py>(py: Python<'py>, v: FVal) -> PyResult<Bound<'py, PyAny>> {
    Ok(match v {
        FVal::Str(None) | FVal::Val(None) | FVal::Xf(None) => py.None().into_bound(py),
        FVal::Str(Some(s)) => s.into_pyobject(py)?.into_any(),
        FVal::Bool(b) => b.into_pyobject(py)?.to_owned().into_any(),
        FVal::Val(Some(v)) => v.to_py(py)?,
        FVal::Xf(Some(_)) => return Err(PyRuntimeError::new_err("matrix fields go through their views")),
    })
}

fn read<'py>(py: Python<'py>, me: &Bound<'py, PyAny>, k: Key, f: Field) -> PyResult<Bound<'py, PyAny>> {
    if f == Field::Material {
        return read_material(py, me, k);
    }
    let (v, cache) = {
        let mut d = lock();
        d.set_handle(k, me.as_ptr());
        let n = d.need(k).map_err(err)?;
        let slot = str_view(n.kind, f);
        if let Some(slot) = slot {
            if let Some(c) = &n.views[slot] {
                return Ok(c.clone_ref(py).into_bound(py));
            }
        }
        match n.get(f).map_err(err)? {
            FVal::Bool(b) => {
                drop(d);
                return Ok(pyo3::types::PyBool::new(py, b).to_owned().into_any());
            }
            v => (v, slot.map(|s| (s, d.serial))),
        }
    };
    let out = fval_to_py(py, v)?;
    // A string field: keep its `str`, so the next read is an incref.
    if let Some((slot, serial)) = cache {
        if !out.is_none() {
            let keep = out.clone().unbind();
            let stale = {
                let mut d = lock();
                let same = d.serial == serial;
                match d.node_mut(k) {
                    Some(n) if same && n.views[slot].is_none() => {
                        n.views[slot] = Some(keep);
                        None
                    }
                    _ => Some(keep),
                }
            };
            drop(stale);
        }
    }
    Ok(out)
}

/// A placement's paint dict: built once per value and handed out again
/// (as the attribute did) until the next write.
fn read_material<'py>(py: Python<'py>, me: &Bound<'py, PyAny>, k: Key) -> PyResult<Bound<'py, PyAny>> {
    let (v, serial) = {
        let mut d = lock();
        d.set_handle(k, me.as_ptr());
        let n = d.need(k).map_err(err)?;
        if let Some(c) = &n.views[VIEW_MATERIAL] {
            return Ok(c.clone_ref(py).into_bound(py));
        }
        (n.get(Field::Material).map_err(err)?, d.serial)
    };
    let out = fval_to_py(py, v)?;
    if out.is_none() {
        return Ok(out);
    }
    let keep = out.clone().unbind();
    let stale = {
        let mut d = lock();
        let same = d.serial == serial;
        match d.node_mut(k) {
            Some(n) if same && n.views[VIEW_MATERIAL].is_none() => {
                n.views[VIEW_MATERIAL] = Some(keep);
                None
            }
            _ => Some(keep),
        }
    };
    drop(stale);
    Ok(out)
}

fn write(py: Python<'_>, me: &Bound<'_, PyAny>, k: Key, f: Field, v: FVal) -> PyResult<()> {
    let mut trash = Trash::default();
    let r = {
        let mut d = lock();
        d.set_handle(k, me.as_ptr());
        d.write(py, k, f, v, &mut trash)
    };
    drop(trash);
    drain_pending();
    r.map(|_| ()).map_err(err)
}

fn view_slot(f: Field) -> usize {
    if f == Field::Axes { VIEW_AXES } else { VIEW_XFORM }
}

fn which_of(f: Field) -> u8 {
    if f == Field::Axes { 1 } else { 0 }
}

fn read_matrix<'py>(py: Python<'py>, me: &Bound<'py, PyAny>, k: Key, f: Field) -> PyResult<Bound<'py, PyAny>> {
    let slot = view_slot(f);
    let (x, serial) = {
        let mut d = lock();
        d.set_handle(k, me.as_ptr());
        let n = d.need(k).map_err(err)?;
        if let Some(v) = &n.views[slot] {
            return Ok(v.clone_ref(py).into_bound(py));
        }
        let x = match n.get(f).map_err(err)? {
            FVal::Xf(x) => x,
            _ => unreachable!(),
        };
        (x, d.serial)
    };
    let Some(x) = x else { return Ok(py.None().into_bound(py)) };
    let (make, _) = hooks(py)?;
    let data = PyTuple::new(py, x.f.iter().map(|&v| v as f64))?;
    let src = match &x.src {
        None => py.None().into_bound(py),
        Some(b) => PyTuple::new(py, [PyTuple::new(py, b.0)?, PyTuple::new(py, b.1)?])?.into_any(),
    };
    let view = make.bind(py).call1((k, which_of(f), data, src))?;
    let keep = view.clone().unbind();
    let stale = {
        let mut d = lock();
        let same = d.serial == serial;
        match d.node_mut(k) {
            Some(n) if same && n.views[slot].is_none() => {
                n.views[slot] = Some(keep);
                None
            }
            // Someone set a view meanwhile (re-entrant read): hand out that one.
            Some(n) if n.views[slot].is_some() => {
                let other = n.views[slot].as_ref().map(|v| v.clone_ref(py));
                drop(d);
                drop(keep);
                return Ok(other.expect("checked").into_bound(py));
            }
            _ => Some(keep),
        }
    };
    drop(stale);
    Ok(view)
}

fn write_matrix(py: Python<'_>, me: &Bound<'_, PyAny>, k: Key, f: Field, value: &Bound<'_, PyAny>) -> PyResult<()> {
    if value.is_none() {
        return write(py, me, k, f, FVal::Xf(None));
    }
    // Another placement's matrix (the viewport's proxies get their part's
    // on every pack) holding what this one holds already: no write, no
    // copy. A matrix Python made still becomes the view (below).
    if let Ok(dict) = value.getattr(pyo3::intern!(py, "__dict__")) {
        if let Ok(dict) = dict.cast::<PyDict>() {
            if let Some(b) = dict.get_item("_doc_bind")? {
                let bound: (Key, u8) = b.extract()?;
                if bound != (k, which_of(f)) {
                    let data = value.call_method0(pyo3::intern!(py, "data"))?;
                    let src = dict.get_item("_built_source")?.unwrap_or_else(|| py.None().into_bound(py));
                    let x = xform_of(&data, &src)?;
                    let d = lock();
                    if let Some(n) = d.node(k) {
                        if n.views[view_slot(f)].is_some() && n.get(f).ok() == Some(FVal::Xf(Some(x))) {
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
    let (_, bind) = hooks(py)?;
    let got = bind.bind(py).call1((value, k, which_of(f)))?;
    let (view, data, src): (Bound<'_, PyAny>, Bound<'_, PyAny>, Bound<'_, PyAny>) = got.extract()?;
    let x = xform_of(&data, &src)?;
    let keep = view.unbind();
    let mut trash = Trash::default();
    let r = {
        let mut d = lock();
        d.set_handle(k, me.as_ptr());
        let r = d.write(py, k, f, FVal::Xf(Some(x)), &mut trash);
        if r.is_ok() {
            if let Some(n) = d.node_mut(k) {
                n.views[view_slot(f)] = Some(keep);
            }
        } else {
            trash.0.push(keep);
        }
        r
    };
    drop(trash);
    drain_pending();
    r.map(|_| ()).map_err(err)
}

fn list_view<'py>(py: Python<'py>, me: &Bound<'py, PyAny>, k: Key, list: ListId) -> PyResult<Bound<'py, PyAny>> {
    let l = list as usize;
    {
        let mut d = lock();
        d.set_handle(k, me.as_ptr());
        let n = d.need(k).map_err(err)?;
        let allowed = list != ListId::Mats && list.owned_by(n.kind);
        if !allowed {
            return Err(PyTypeError::new_err(format!("a {} has no such list", n.kind.name())));
        }
        let ptr = n.list_views[l].0;
        if !ptr.is_null() {
            return Ok(unsafe { Bound::from_borrowed_ptr(py, ptr) });
        }
    }
    let view = Bound::new(
        py,
        HandleList {
            owner: k,
            list: list as u8,
            owner_ref: me.clone().unbind(),
            cached: AtomicBool::new(true),
            detached: Mutex::new(None),
        },
    )?;
    let mut d = lock();
    match d.node_mut(k) {
        Some(n) if n.list_views[l].0.is_null() => {
            n.list_views[l] = super::HPtr(view.as_ptr());
            drop(d);
            Ok(view.into_any())
        }
        Some(n) => {
            // Made meanwhile (a re-entrant read): hand out that one.
            let other = unsafe { Bound::from_borrowed_ptr(py, n.list_views[l].0) };
            drop(d);
            view.get().cached.store(false, Ordering::Relaxed);
            drop(view);
            Ok(other)
        }
        None => {
            drop(d);
            view.get().cached.store(false, Ordering::Relaxed);
            Ok(view.into_any())
        }
    }
}

/// The key of a document handle (Placement or Tag), recording its pointer.
fn member_key(obj: &Bound<'_, PyAny>) -> PyResult<Key> {
    if let Ok(p) = obj.cast::<PyPlacement>() {
        return Ok(p.get().key);
    }
    if let Ok(t) = obj.cast::<PyTag>() {
        return Ok(t.get().key);
    }
    Err(PyTypeError::new_err(format!(
        "document lists hold Group placements (children, scene.groups) and Layer tags (scene.layers), not {}",
        obj.get_type().name()?
    )))
}

fn keys_of<'py>(items: &Bound<'py, PyAny>) -> PyResult<Vec<(Key, *mut pyo3::ffi::PyObject, Bound<'py, PyAny>)>> {
    // Snapshot first: `scene.groups = scene.groups[::-1]` or a generator
    // reading the list it replaces must see the list as it was.
    let mut out = Vec::new();
    for item in items.try_iter()? {
        let item = item?;
        let k = member_key(&item)?;
        out.push((k, item.as_ptr(), item));
    }
    Ok(out)
}

fn set_list(py: Python<'_>, me: &Bound<'_, PyAny>, k: Key, list: ListId, items: &Bound<'_, PyAny>) -> PyResult<()> {
    let l = list as usize;
    // The list's own view assigned back (`g.children += [x]` ends that
    // way): nothing changes, and the view stays the list.
    {
        let d = lock();
        if let Some(n) = d.node(k) {
            if n.list_views[l].0 == items.as_ptr() {
                return Ok(());
            }
        }
    }
    let members = keys_of(items)?;
    let mut trash = Trash::default();
    let (r, old_view, old_items) = {
        let mut d = lock();
        d.set_handle(k, me.as_ptr());
        for (m, ptr, _) in &members {
            d.set_handle(*m, *ptr);
        }
        // What the old list object held, for the view handed out before.
        let (view, old) = match d.node_mut(k) {
            Some(n) => {
                let ptr = std::mem::replace(&mut n.list_views[l], super::HPtr(std::ptr::null_mut())).0;
                if ptr.is_null() {
                    (None, Vec::new())
                } else {
                    let view: Py<PyAny> = unsafe { Bound::from_borrowed_ptr(py, ptr) }.unbind();
                    (Some(view), n.list(list).clone())
                }
            }
            None => (None, Vec::new()),
        };
        let old: Vec<Py<PyAny>> = old.iter().filter_map(|&c| d.handle_ref(py, c)).collect();
        let r = d.list_replace(py, k, list, members.iter().map(|m| m.0).collect(), &mut trash);
        (r, view, old)
    };
    drop(members);
    drop(trash);
    if let Some(view) = old_view {
        let frozen = PyList::new(py, old_items)?.unbind();
        if let Ok(h) = view.bind(py).cast::<HandleList>() {
            h.get().cached.store(false, Ordering::Relaxed);
            let prev = h.get().detached.lock().unwrap_or_else(|e| e.into_inner()).replace(frozen);
            drop(prev);
        }
    }
    // An old list of this very attribute assigned back (`old =
    // g.children; g.children = new; ...; g.children = old`): it is the
    // list again, as the Python list object was.
    if r.is_ok() {
        if let Ok(h) = items.cast::<HandleList>() {
            let hl = h.get();
            if hl.owner == k && hl.list == list as u8 && !hl.cached.load(Ordering::Relaxed) {
                let mut d = lock();
                if let Some(n) = d.node_mut(k) {
                    if n.list_views[l].0.is_null() {
                        n.list_views[l] = super::HPtr(items.as_ptr());
                        hl.cached.store(true, Ordering::Relaxed);
                        drop(d);
                        let prev = hl.detached.lock().unwrap_or_else(|e| e.into_inner()).take();
                        drop(prev);
                    }
                }
            }
        }
    }
    drain_pending();
    r.map_err(err)?;
    parent_reports(py)
}

fn traverse_node(k: Key, visit: &PyVisit<'_>) -> Result<(), PyTraverseError> {
    // Under GC: never block. A missed edge only keeps objects alive longer.
    let Some(d) = try_lock() else { return Ok(()) };
    let Some(n) = d.node(k) else { return Ok(()) };
    for v in n.views.iter() {
        visit.call(v)?;
    }
    for (_, v) in n.attr_views.iter() {
        visit.call(v)?;
    }
    for l in [&n.children, &n.tags, &n.kept, &n.mats] {
        let mut seen: Option<HashSet<Key>> = if l.len() > 1 { Some(HashSet::new()) } else { None };
        for &c in l {
            if let Some(s) = seen.as_mut() {
                if !s.insert(c) {
                    continue;
                }
            }
            if let Some(cn) = d.node(c) {
                // The arena's one reference is reported by the first owner.
                if cn.owners.first() == Some(&k) {
                    visit.call(&cn.strong)?;
                }
            }
        }
    }
    Ok(())
}

fn clear_node(k: Key) {
    let mut trash = Trash::default();
    if let Some(mut d) = try_lock() {
        d.clear_node(k, &mut trash);
    }
    drop(trash);
}

// -------------------------------------------------------------- Placement

/// A placement (group, component instance, nested part) or, made by
/// `doc_new_root`, a scene's root. `core.group.Group` subclasses it.
#[pyclass(module = "ingetrazo_core", name = "Placement", subclass, frozen)]
pub struct PyPlacement {
    pub key: Key,
}

impl Drop for PyPlacement {
    fn drop(&mut self) {
        handle_dropped(self.key);
    }
}

#[pymethods]
impl PyPlacement {
    /// `Placement.__new__(cls, _detached=True)`: a handle with no node,
    /// for a subclass that keeps every field in Python slots (the
    /// viewport's placement proxies, `core.group.PlacementProxy`): it is a
    /// `Group` to every `isinstance` and no part of the document.
    #[new]
    #[pyo3(signature = (*_args, **_kwargs))]
    fn new(_args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        if let Some(kw) = _kwargs {
            if kw.get_item("_detached")?.is_some_and(|v| v.is_truthy().unwrap_or(false)) {
                return Ok(PyPlacement { key: 0 });
            }
        }
        drain_pending();
        let key = lock().alloc(Kind::Placement);
        Ok(PyPlacement { key })
    }

    #[getter]
    fn _doc_key(&self) -> Key {
        self.key
    }

    #[getter]
    fn uid<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read(slf.py(), slf.as_any(), slf.get().key, Field::Uid)
    }
    #[setter]
    fn set_uid(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        write(slf.py(), slf.as_any(), slf.get().key, Field::Uid, FVal::Str(str_of(v)?))
    }
    #[getter]
    fn name<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read(slf.py(), slf.as_any(), slf.get().key, Field::Name)
    }
    #[setter]
    fn set_name(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        write(slf.py(), slf.as_any(), slf.get().key, Field::Name, FVal::Str(str_of(v)?))
    }
    /// The tag's KEY (`Layer.key`), None for the default tag.
    #[getter]
    fn layer<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read(slf.py(), slf.as_any(), slf.get().key, Field::Tag)
    }
    #[setter]
    fn set_layer(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        write(slf.py(), slf.as_any(), slf.get().key, Field::Tag, FVal::Str(str_of(v)?))
    }
    #[getter]
    fn component<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read(slf.py(), slf.as_any(), slf.get().key, Field::Component)
    }
    #[setter]
    fn set_component(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        write(slf.py(), slf.as_any(), slf.get().key, Field::Component, FVal::Bool(v.is_truthy()?))
    }
    #[getter]
    fn hidden<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read(slf.py(), slf.as_any(), slf.get().key, Field::Hidden)
    }
    #[setter]
    fn set_hidden(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        write(slf.py(), slf.as_any(), slf.get().key, Field::Hidden, FVal::Bool(v.is_truthy()?))
    }
    #[getter]
    fn locked<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read(slf.py(), slf.as_any(), slf.get().key, Field::Locked)
    }
    #[setter]
    fn set_locked(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        write(slf.py(), slf.as_any(), slf.get().key, Field::Locked, FVal::Bool(v.is_truthy()?))
    }
    /// The container's own paint dict, or None. A copy each read: edit it
    /// by assigning a new dict.
    #[getter]
    fn material<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read(slf.py(), slf.as_any(), slf.get().key, Field::Material)
    }
    #[setter]
    fn set_material(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        let (py, k) = (slf.py(), slf.get().key);
        // The very dict this placement shows already: no write.
        {
            let d = lock();
            if let Some(n) = d.node(k) {
                if n.views[VIEW_MATERIAL].as_ref().map(|x| x.as_ptr()) == Some(v.as_ptr()) {
                    return Ok(());
                }
            }
        }
        let val = if v.is_none() { None } else { Some(Value::from_py(v)?) };
        // The same paint again (the viewport hands every proxy its part's
        // dict on each pack): nothing changes, the cached dict stays.
        // The dict assigned is not kept: the document holds the value
        // (keeping the loader's dicts pinned its JSON's memory, +20 MB on
        // the 20k-placement probe).
        {
            let d = lock();
            if let Some(n) = d.node(k) {
                if n.get(Field::Material).ok().as_ref() == Some(&FVal::Val(val.clone())) {
                    return Ok(());
                }
            }
        }
        write(py, slf.as_any(), k, Field::Material, FVal::Val(val))
    }
    #[getter]
    fn xform<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read_matrix(slf.py(), slf.as_any(), slf.get().key, Field::Xform)
    }
    #[setter]
    fn set_xform(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        write_matrix(slf.py(), slf.as_any(), slf.get().key, Field::Xform, v)
    }
    #[getter]
    fn axes<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read_matrix(slf.py(), slf.as_any(), slf.get().key, Field::Axes)
    }
    #[setter]
    fn set_axes(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        write_matrix(slf.py(), slf.as_any(), slf.get().key, Field::Axes, v)
    }
    #[getter]
    fn children<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        list_view(slf.py(), slf.as_any(), slf.get().key, ListId::Children)
    }
    #[setter]
    fn set_children(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        let v = if v.is_none() { PyList::empty(slf.py()).into_any() } else { v.clone() };
        set_list(slf.py(), slf.as_any(), slf.get().key, ListId::Children, &v)
    }
    /// A root's tag list (`scene.layers`).
    #[getter]
    fn _tags<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        list_view(slf.py(), slf.as_any(), slf.get().key, ListId::Tags)
    }
    #[setter(_tags)]
    fn set_tag_list(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        set_list(slf.py(), slf.as_any(), slf.get().key, ListId::Tags, v)
    }

    /// The attribute `name` (`Field::Attr`): a scene root's tables are read
    /// this way (`Scene.layer_folders`…); `track` as for `DocAttr`.
    #[pyo3(signature = (name, track = true))]
    fn _attr<'py>(slf: &Bound<'py, Self>, name: &str, track: bool) -> PyResult<Bound<'py, PyAny>> {
        let a = attr_id(name).map_err(PyValueError::new_err)?;
        read_attr(slf.py(), slf.as_any(), slf.get().key, a, track)
    }

    fn _set_attr(slf: &Bound<'_, Self>, name: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let a = attr_id(name).map_err(PyValueError::new_err)?;
        write_attr(slf.py(), slf.as_any(), slf.get().key, a, value)
    }

    /// Take this placement out of the lists that hold it (journalled);
    /// returns how many held it.
    fn _detach(slf: &Bound<'_, Self>) -> PyResult<usize> {
        let mut trash = Trash::default();
        let r = {
            let mut d = lock();
            d.set_handle(slf.get().key, slf.as_ptr());
            d.detach(slf.py(), slf.get().key, &mut trash)
        };
        drop(trash);
        drain_pending();
        r.map_err(err)
    }

    /// The node holding this placement in one of its lists, or None.
    #[getter]
    fn _parent(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        let d = lock();
        let n = d.need(self.key).map_err(err)?;
        Ok(n.owners.first().and_then(|&o| d.handle_ref(py, o)))
    }

    /// A root's kept component definitions (`scene.kept_components`).
    #[getter]
    fn _kept<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        list_view(slf.py(), slf.as_any(), slf.get().key, ListId::Kept)
    }
    #[setter(_kept)]
    fn set_kept(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        set_list(slf.py(), slf.as_any(), slf.get().key, ListId::Kept, v)
    }

    /// A root's materials registry (`scene.materials`): a live mapping.
    #[getter]
    fn _materials(slf: &Bound<'_, Self>) -> PyResult<Py<Registry>> {
        {
            let mut d = lock();
            d.set_handle(slf.get().key, slf.as_ptr());
            let n = d.need(slf.get().key).map_err(err)?;
            if n.kind != Kind::Root {
                return Err(PyTypeError::new_err("only a scene root has a materials registry"));
            }
        }
        Py::new(slf.py(), Registry { owner: slf.get().key, owner_ref: slf.clone().into_any().unbind() })
    }
    #[setter(_materials)]
    fn set_materials(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        let pairs = registry_pairs(v)?;
        registry_set(slf.py(), slf.get().key, pairs)
    }

    /// A root's tag whose key is `key` (`Scene.layer_by_key`), or None:
    /// compared in the document, no string made per tag.
    fn _tag_by_key<'py>(&self, py: Python<'py>, key: &str) -> PyResult<Option<Py<PyAny>>> {
        self.find_tag(py, key, Field::Key)
    }

    /// The first tag labelled `name`, or None.
    fn _tag_by_name<'py>(&self, py: Python<'py>, name: &str) -> PyResult<Option<Py<PyAny>>> {
        self.find_tag(py, name, Field::Name)
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        traverse_node(self.key, &visit)
    }

    fn __clear__(&self) {
        clear_node(self.key);
    }
}

impl PyPlacement {
    fn find_tag(&self, py: Python<'_>, want: &str, f: Field) -> PyResult<Option<Py<PyAny>>> {
        let d = lock();
        let n = d.need(self.key).map_err(err)?;
        for &t in &n.tags {
            let Some(tn) = d.node(t) else { continue };
            let hit = match &tn.data {
                super::Data::Tag(tag) => match f {
                    Field::Key => tag.key.as_deref() == Some(want),
                    _ => tag.name.as_deref() == Some(want),
                },
                _ => false,
            };
            if hit {
                return Ok(d.handle_ref(py, t));
            }
        }
        Ok(None)
    }
}

// -------------------------------------------------------------------- Tag

/// A tag's document fields; `core.layers.Layer` subclasses it.
#[pyclass(module = "ingetrazo_core", name = "Tag", subclass, frozen)]
pub struct PyTag {
    pub key: Key,
}

impl Drop for PyTag {
    fn drop(&mut self) {
        handle_dropped(self.key);
    }
}

#[pymethods]
impl PyTag {
    #[new]
    #[pyo3(signature = (*_args, **_kwargs))]
    fn new(_args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>) -> Self {
        drain_pending();
        let key = lock().alloc(Kind::Tag);
        PyTag { key }
    }

    #[getter]
    fn _doc_key(&self) -> Key {
        self.key
    }
    #[getter]
    fn uid<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read(slf.py(), slf.as_any(), slf.get().key, Field::Uid)
    }
    #[setter]
    fn set_uid(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        write(slf.py(), slf.as_any(), slf.get().key, Field::Uid, FVal::Str(str_of(v)?))
    }
    #[getter]
    fn name<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read(slf.py(), slf.as_any(), slf.get().key, Field::Name)
    }
    #[setter]
    fn set_name(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        write(slf.py(), slf.as_any(), slf.get().key, Field::Name, FVal::Str(str_of(v)?))
    }
    #[getter]
    fn key<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read(slf.py(), slf.as_any(), slf.get().key, Field::Key)
    }
    #[setter]
    fn set_key(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        write(slf.py(), slf.as_any(), slf.get().key, Field::Key, FVal::Str(str_of(v)?))
    }
    #[getter]
    fn visible<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read(slf.py(), slf.as_any(), slf.get().key, Field::Visible)
    }
    #[setter]
    fn set_visible(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        write(slf.py(), slf.as_any(), slf.get().key, Field::Visible, FVal::Bool(v.is_truthy()?))
    }
    #[getter]
    fn locked<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read(slf.py(), slf.as_any(), slf.get().key, Field::Locked)
    }
    #[setter]
    fn set_locked(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        write(slf.py(), slf.as_any(), slf.get().key, Field::Locked, FVal::Bool(v.is_truthy()?))
    }
    #[getter]
    fn color<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read(slf.py(), slf.as_any(), slf.get().key, Field::Color)
    }
    #[setter]
    fn set_color(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        let val = if v.is_none() { None } else { Some(Value::from_py(v)?) };
        write(slf.py(), slf.as_any(), slf.get().key, Field::Color, FVal::Val(val))
    }
    #[getter]
    fn folder<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read(slf.py(), slf.as_any(), slf.get().key, Field::Folder)
    }
    #[setter]
    fn set_folder(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        write(slf.py(), slf.as_any(), slf.get().key, Field::Folder, FVal::Str(str_of(v)?))
    }
}

// --------------------------------------------------------------- Material

fn opt_value(v: &Bound<'_, PyAny>) -> PyResult<FVal> {
    Ok(FVal::Val(if v.is_none() { None } else { Some(Value::from_py(v)?) }))
}

/// A material's fields; `core.materials.Material` subclasses it.
#[pyclass(module = "ingetrazo_core", name = "MaterialData", subclass, frozen)]
pub struct PyMaterial {
    pub key: Key,
}

impl Drop for PyMaterial {
    fn drop(&mut self) {
        handle_dropped(self.key);
    }
}

#[pymethods]
impl PyMaterial {
    #[new]
    #[pyo3(signature = (*_args, **_kwargs))]
    fn new(_args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>) -> Self {
        drain_pending();
        let key = lock().alloc(Kind::Material);
        PyMaterial { key }
    }

    #[getter]
    fn _doc_key(&self) -> Key {
        self.key
    }
    #[getter]
    fn name<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read(slf.py(), slf.as_any(), slf.get().key, Field::Name)
    }
    #[setter]
    fn set_name(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        write(slf.py(), slf.as_any(), slf.get().key, Field::Name, FVal::Str(str_of(v)?))
    }
    #[getter]
    fn color<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read(slf.py(), slf.as_any(), slf.get().key, Field::Color)
    }
    #[setter]
    fn set_color(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        write(slf.py(), slf.as_any(), slf.get().key, Field::Color, opt_value(v)?)
    }
    #[getter]
    fn texture<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read(slf.py(), slf.as_any(), slf.get().key, Field::Texture)
    }
    #[setter]
    fn set_texture(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        write(slf.py(), slf.as_any(), slf.get().key, Field::Texture, opt_value(v)?)
    }
    #[getter]
    fn opacity<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read(slf.py(), slf.as_any(), slf.get().key, Field::Opacity)
    }
    #[setter]
    fn set_opacity(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        write(slf.py(), slf.as_any(), slf.get().key, Field::Opacity, opt_value(v)?)
    }
    #[getter]
    fn finish<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read(slf.py(), slf.as_any(), slf.get().key, Field::Finish)
    }
    #[setter]
    fn set_finish(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        write(slf.py(), slf.as_any(), slf.get().key, Field::Finish, FVal::Str(str_of(v)?))
    }
    #[getter]
    fn pbr<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        read(slf.py(), slf.as_any(), slf.get().key, Field::Pbr)
    }
    #[setter]
    fn set_pbr(slf: &Bound<'_, Self>, v: &Bound<'_, PyAny>) -> PyResult<()> {
        write(slf.py(), slf.as_any(), slf.get().key, Field::Pbr, opt_value(v)?)
    }
}

// ------------------------------------------------------------- HandleList

/// A live list of document handles: a placement's children, a scene's
/// root placements or its tags. Behaves as the Python list it replaces;
/// slices and copies are plain lists.
#[pyclass(module = "ingetrazo_core", name = "HandleList", frozen, sequence)]
pub struct HandleList {
    owner: Key,
    list: u8,
    /// The owner's handle: a list keeps what holds its members alive.
    owner_ref: Py<PyAny>,
    /// Whether the owner's node names this view as its live list.
    cached: AtomicBool,
    /// Set when the attribute this list was read from is assigned a new
    /// list (`g.children = [...]`): from then on this object is the OLD
    /// list, a plain Python list with what it held, as the list object a
    /// Python attribute pointed at stayed (`old = g.children; g.children =
    /// new; ...; g.children = old` restores).
    detached: Mutex<Option<Py<PyList>>>,
}

impl Drop for HandleList {
    fn drop(&mut self) {
        if !self.cached.load(Ordering::Relaxed) {
            return;
        }
        // Only the live view is cached, one per list: it is this one.
        let mut d = lock();
        let l = self.list as usize;
        if let Some(n) = d.node_mut(self.owner) {
            n.list_views[l] = super::HPtr(std::ptr::null_mut());
        }
    }
}

impl HandleList {
    fn l(&self) -> ListId {
        ListId::from_u8(self.list)
    }

    fn keys(&self) -> PyResult<Vec<Key>> {
        let d = lock();
        Ok(d.need(self.owner).map_err(err)?.list(self.l()).clone())
    }

    fn handles(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        let d = lock();
        let n = d.need(self.owner).map_err(err)?;
        let mut out = Vec::with_capacity(n.list(self.l()).len());
        for &k in n.list(self.l()) {
            out.push(d.handle_ref(py, k).ok_or_else(|| PyRuntimeError::new_err("a listed node lost its handle"))?);
        }
        Ok(out)
    }

    /// The plain list this view became, if its attribute was reassigned.
    fn det<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyList>> {
        let d = self.detached.lock().unwrap_or_else(|e| e.into_inner());
        d.as_ref().map(|l| l.clone_ref(py).into_bound(py))
    }

    fn materialize<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        if let Some(l) = self.det(py) {
            return l.as_any().call_method0("copy")?.cast_into::<PyList>().map_err(Into::into);
        }
        let hs = self.handles(py)?;
        PyList::new(py, hs)
    }

    fn insert_at(&self, py: Python<'_>, at: usize, obj: &Bound<'_, PyAny>) -> PyResult<()> {
        let k = member_key(obj)?;
        let r = {
            let mut d = lock();
            d.set_handle(k, obj.as_ptr());
            d.list_insert(py, self.owner, self.l(), at, k)
        };
        drain_pending();
        r.map_err(err)?;
        parent_reports(py)
    }

    fn remove_at(&self, py: Python<'_>, at: usize) -> PyResult<Py<PyAny>> {
        let mut trash = Trash::default();
        let r = {
            let mut d = lock();
            let k = d.need(self.owner).map_err(err)?.list(self.l()).get(at).copied();
            match k {
                None => Err(PyIndexError::new_err("pop index out of range")),
                Some(k) => {
                    let h = d.handle_ref(py, k);
                    d.list_remove(py, self.owner, self.l(), at, &mut trash).map_err(err)?;
                    h.ok_or_else(|| PyRuntimeError::new_err("a listed node lost its handle"))
                }
            }
        };
        drop(trash);
        drain_pending();
        r
    }

    fn replace(&self, py: Python<'_>, items: &Bound<'_, PyAny>) -> PyResult<()> {
        let members = keys_of(items)?;
        let mut trash = Trash::default();
        let r = {
            let mut d = lock();
            for (m, ptr, _) in &members {
                d.set_handle(*m, *ptr);
            }
            d.list_replace(py, self.owner, self.l(), members.iter().map(|m| m.0).collect(), &mut trash)
        };
        drop(members);
        drop(trash);
        drain_pending();
        r.map_err(err)?;
        parent_reports(py)
    }

    fn norm(&self, i: isize, len: usize) -> Option<usize> {
        let i = if i < 0 { i + len as isize } else { i };
        if i < 0 || i as usize >= len { None } else { Some(i as usize) }
    }

    /// Run `f` on a plain list copy and store the result (slice
    /// assignment, sorting: exactly Python's list semantics).
    fn via_list<'py, F>(&self, py: Python<'py>, f: F) -> PyResult<()>
    where
        F: FnOnce(&Bound<'py, PyList>) -> PyResult<()>,
    {
        if let Some(l) = self.det(py) {
            return f(&l);
        }
        let l = self.materialize(py)?;
        f(&l)?;
        self.replace(py, l.as_any())
    }
}

#[pymethods]
impl HandleList {
    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        if let Some(l) = self.det(py) {
            return Ok(l.len());
        }
        let d = lock();
        Ok(d.need(self.owner).map_err(err)?.list(self.l()).len())
    }

    fn __bool__(&self, py: Python<'_>) -> PyResult<bool> {
        Ok(self.__len__(py)? > 0)
    }

    fn __getitem__<'py>(&self, py: Python<'py>, idx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        if let Some(l) = self.det(py) {
            return l.as_any().get_item(idx);
        }
        if idx.is_instance_of::<PySlice>() {
            let l = self.materialize(py)?;
            return l.as_any().get_item(idx);
        }
        let i: isize = idx.extract()?;
        let d = lock();
        let n = d.need(self.owner).map_err(err)?;
        let list = n.list(self.l());
        let i = self.norm(i, list.len()).ok_or_else(|| PyIndexError::new_err("list index out of range"))?;
        let h = d.handle_ref(py, list[i]).ok_or_else(|| PyRuntimeError::new_err("a listed node lost its handle"))?;
        drop(d);
        Ok(h.into_bound(py))
    }

    fn __setitem__(&self, py: Python<'_>, idx: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        if let Some(l) = self.det(py) {
            return l.as_any().set_item(idx, value);
        }
        self.via_list(py, |l| l.as_any().set_item(idx, value))
    }

    fn __delitem__(&self, py: Python<'_>, idx: &Bound<'_, PyAny>) -> PyResult<()> {
        if let Some(l) = self.det(py) {
            return l.as_any().del_item(idx);
        }
        if !idx.is_instance_of::<PySlice>() {
            let i: isize = idx.extract()?;
            let len = self.__len__(py)?;
            let i = self.norm(i, len).ok_or_else(|| PyIndexError::new_err("list assignment index out of range"))?;
            self.remove_at(py, i)?;
            return Ok(());
        }
        self.via_list(py, |l| l.as_any().del_item(idx))
    }

    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        Ok(self.materialize(py)?.as_any().try_iter()?.into_any())
    }

    fn __reversed__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let l = self.materialize(py)?;
        l.reverse()?;
        Ok(l.as_any().try_iter()?.into_any())
    }

    fn __contains__(&self, py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<bool> {
        if let Some(l) = self.det(py) {
            return l.contains(obj);
        }
        let Ok(k) = member_key(obj) else { return Ok(false) };
        Ok(self.keys()?.contains(&k))
    }

    fn __richcmp__<'py>(&self, py: Python<'py>, other: &Bound<'py, PyAny>, op: pyo3::basic::CompareOp) -> PyResult<Bound<'py, PyAny>> {
        let me = self.materialize(py)?;
        let other = match other.cast::<HandleList>() {
            Ok(h) => h.get().materialize(py)?.into_any(),
            Err(_) => other.clone(),
        };
        me.as_any().rich_compare(other, op)
    }

    fn __add__<'py>(&self, py: Python<'py>, other: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let me = self.materialize(py)?;
        let other = match other.cast::<HandleList>() {
            Ok(h) => h.get().materialize(py)?.into_any(),
            Err(_) => other.clone(),
        };
        me.as_any().add(other)
    }

    fn __radd__<'py>(&self, py: Python<'py>, other: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let me = self.materialize(py)?;
        other.add(me)
    }

    fn __iadd__(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<()> {
        if let Some(l) = slf.get().det(slf.py()) {
            l.as_any().call_method1("extend", (other,))?;
            return Ok(());
        }
        slf.get().extend(slf.py(), other)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(self.materialize(py)?.repr()?.to_string())
    }

    fn __copy__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        self.materialize(py)
    }

    fn copy<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        self.materialize(py)
    }

    fn append(&self, py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<()> {
        if let Some(l) = self.det(py) {
            return l.append(obj);
        }
        self.insert_at(py, usize::MAX, obj)
    }

    fn insert(&self, py: Python<'_>, i: isize, obj: &Bound<'_, PyAny>) -> PyResult<()> {
        if let Some(l) = self.det(py) {
            l.as_any().call_method1("insert", (i, obj))?;
            return Ok(());
        }
        let len = self.__len__(py)? as isize;
        let at = if i < 0 { (i + len).max(0) } else { i.min(len) };
        self.insert_at(py, at as usize, obj)
    }

    fn extend(&self, py: Python<'_>, items: &Bound<'_, PyAny>) -> PyResult<()> {
        if let Some(l) = self.det(py) {
            l.as_any().call_method1("extend", (items,))?;
            return Ok(());
        }
        let members = keys_of(items)?;
        let r = {
            let mut d = lock();
            let mut r = Ok(());
            for (m, ptr, _) in &members {
                d.set_handle(*m, *ptr);
                r = d.list_insert(py, self.owner, self.l(), usize::MAX, *m);
                if r.is_err() {
                    break;
                }
            }
            r
        };
        drop(members);
        drain_pending();
        r.map_err(err)?;
        parent_reports(py)
    }

    fn remove(&self, py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<()> {
        if let Some(l) = self.det(py) {
            l.as_any().call_method1("remove", (obj,))?;
            return Ok(());
        }
        let missing = || PyValueError::new_err("list.remove(x): x not in list");
        let k = member_key(obj).map_err(|_| missing())?;
        let at = self.keys()?.iter().position(|&x| x == k).ok_or_else(missing)?;
        self.remove_at(py, at)?;
        Ok(())
    }

    #[pyo3(signature = (i = -1))]
    fn pop(&self, py: Python<'_>, i: isize) -> PyResult<Py<PyAny>> {
        if let Some(l) = self.det(py) {
            return Ok(l.as_any().call_method1("pop", (i,))?.unbind());
        }
        let len = self.__len__(py)?;
        if len == 0 {
            return Err(PyIndexError::new_err("pop from empty list"));
        }
        let at = self.norm(i, len).ok_or_else(|| PyIndexError::new_err("pop index out of range"))?;
        self.remove_at(py, at)
    }

    fn clear(&self, py: Python<'_>) -> PyResult<()> {
        if let Some(l) = self.det(py) {
            l.as_any().call_method0("clear")?;
            return Ok(());
        }
        self.replace(py, PyList::empty(py).as_any())
    }

    #[pyo3(signature = (obj, start = 0, stop = isize::MAX))]
    fn index(&self, py: Python<'_>, obj: &Bound<'_, PyAny>, start: isize, stop: isize) -> PyResult<usize> {
        if let Some(l) = self.det(py) {
            return l.as_any().call_method1("index", (obj, start, stop))?.extract();
        }
        let missing = || PyValueError::new_err("list.index(x): x not in list");
        let k = member_key(obj).map_err(|_| missing())?;
        let keys = self.keys()?;
        let len = keys.len() as isize;
        let clamp = |i: isize| if i < 0 { (i + len).max(0) } else { i.min(len) } as usize;
        let (a, b) = (clamp(start), clamp(stop));
        (a..b.max(a)).find(|&i| keys[i] == k).ok_or_else(missing)
    }

    fn count(&self, py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<usize> {
        if let Some(l) = self.det(py) {
            return l.as_any().call_method1("count", (obj,))?.extract();
        }
        let Ok(k) = member_key(obj) else { return Ok(0) };
        Ok(self.keys()?.iter().filter(|&&x| x == k).count())
    }

    #[pyo3(signature = (*, key = None, reverse = false))]
    fn sort(&self, py: Python<'_>, key: Option<&Bound<'_, PyAny>>, reverse: bool) -> PyResult<()> {
        self.via_list(py, |l| {
            let kw = PyDict::new(py);
            if let Some(k) = key {
                kw.set_item("key", k)?;
            }
            kw.set_item("reverse", reverse)?;
            l.as_any().call_method("sort", (), Some(&kw))?;
            Ok(())
        })
    }

    fn reverse(&self, py: Python<'_>) -> PyResult<()> {
        self.via_list(py, |l| l.reverse())
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.owner_ref)?;
        if let Ok(d) = self.detached.try_lock() {
            visit.call(d.as_ref())?;
        }
        Ok(())
    }

    fn __clear__(&self) {
        let old = self.detached.lock().unwrap_or_else(|e| e.into_inner()).take();
        drop(old);
    }
}

// --------------------------------------------------------------- Registry

/// `(name, material)` pairs of a mapping (a dict, a `Registry`, or pairs).
fn registry_pairs<'py>(v: &Bound<'py, PyAny>) -> PyResult<Vec<(String, Key, Bound<'py, PyAny>)>> {
    let mut out = Vec::new();
    if v.is_none() {
        return Ok(out);
    }
    let items = if v.hasattr("items")? { v.call_method0("items")? } else { v.clone() };
    for item in items.try_iter()? {
        let (name, mat): (String, Bound<'py, PyAny>) = item?.extract()?;
        let k = match mat.cast::<PyMaterial>() {
            Ok(m) => m.get().key,
            Err(_) => {
                return Err(PyTypeError::new_err(format!(
                    "scene.materials holds Material entries, not {}",
                    mat.get_type().name()?
                )))
            }
        };
        out.push((name, k, mat));
    }
    Ok(out)
}

fn registry_set(py: Python<'_>, owner: Key, pairs: Vec<(String, Key, Bound<'_, PyAny>)>) -> PyResult<()> {
    let mut trash = Trash::default();
    let r = {
        let mut d = lock();
        for (_, k, h) in &pairs {
            d.set_handle(*k, h.as_ptr());
        }
        let new = pairs.iter().map(|(n, k, _)| (n.clone(), *k)).collect();
        d.registry_replace(py, owner, new, &mut trash)
    };
    drop(pairs);
    drop(trash);
    drain_pending();
    r.map_err(err)
}

/// `scene.materials`: the scene's materials registry, a live mapping of
/// name → `Material` kept in the document (slice 2). Behaves as the dict it
/// replaces; `copy()` and `dict(...)` are plain dicts.
#[pyclass(module = "ingetrazo_core", name = "Registry", frozen, mapping)]
pub struct Registry {
    owner: Key,
    owner_ref: Py<PyAny>,
}

impl Registry {
    fn pairs(&self) -> PyResult<Vec<(String, Key)>> {
        lock().registry(self.owner).map_err(err)
    }

    fn handles<'py>(&self, py: Python<'py>) -> PyResult<Vec<(String, Py<PyAny>)>> {
        let d = lock();
        let pairs = d.registry(self.owner).map_err(err)?;
        let mut out = Vec::with_capacity(pairs.len());
        for (n, k) in pairs {
            out.push((n, d.handle_ref(py, k).ok_or_else(|| PyRuntimeError::new_err("a registered material lost its handle"))?));
        }
        Ok(out)
    }

    fn dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let out = PyDict::new(py);
        for (n, h) in self.handles(py)? {
            out.set_item(n, h)?;
        }
        Ok(out)
    }

    fn store(&self, py: Python<'_>, pairs: Vec<(String, Key)>, extra: &[Bound<'_, PyAny>]) -> PyResult<()> {
        let mut trash = Trash::default();
        let r = {
            let mut d = lock();
            for h in extra {
                if let Ok(m) = h.cast::<PyMaterial>() {
                    d.set_handle(m.get().key, h.as_ptr());
                }
            }
            d.registry_replace(py, self.owner, pairs, &mut trash)
        };
        drop(trash);
        drain_pending();
        r.map_err(err)
    }

    fn material_key(v: &Bound<'_, PyAny>) -> PyResult<Key> {
        match v.cast::<PyMaterial>() {
            Ok(m) => Ok(m.get().key),
            Err(_) => Err(PyTypeError::new_err(format!(
                "scene.materials holds Material entries, not {}",
                v.get_type().name()?
            ))),
        }
    }
}

#[pymethods]
impl Registry {
    fn __len__(&self) -> PyResult<usize> {
        Ok(self.pairs()?.len())
    }

    fn __bool__(&self) -> PyResult<bool> {
        Ok(!self.pairs()?.is_empty())
    }

    fn __contains__(&self, name: &Bound<'_, PyAny>) -> PyResult<bool> {
        let Ok(name) = name.extract::<String>() else { return Ok(false) };
        Ok(self.pairs()?.iter().any(|(n, _)| *n == name))
    }

    fn __getitem__<'py>(&self, py: Python<'py>, name: &Bound<'py, PyAny>) -> PyResult<Py<PyAny>> {
        let key_err = || pyo3::exceptions::PyKeyError::new_err(name.clone().unbind());
        let Ok(want) = name.extract::<String>() else { return Err(key_err()) };
        let d = lock();
        let n = d.need(self.owner).map_err(err)?;
        match n.mat_names.iter().position(|x| *x == want) {
            Some(i) => d.handle_ref(py, n.mats[i]).ok_or_else(|| PyRuntimeError::new_err("a registered material lost its handle")),
            None => Err(key_err()),
        }
    }

    fn __setitem__(&self, py: Python<'_>, name: String, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let k = Self::material_key(value)?;
        let mut pairs = self.pairs()?;
        match pairs.iter_mut().find(|(n, _)| *n == name) {
            Some(p) => p.1 = k,
            None => pairs.push((name, k)),
        }
        self.store(py, pairs, std::slice::from_ref(value))
    }

    fn __delitem__(&self, py: Python<'_>, name: &Bound<'_, PyAny>) -> PyResult<()> {
        let want: String = name.extract().map_err(|_| pyo3::exceptions::PyKeyError::new_err(name.clone().unbind()))?;
        let mut pairs = self.pairs()?;
        let Some(i) = pairs.iter().position(|(n, _)| *n == want) else {
            return Err(pyo3::exceptions::PyKeyError::new_err(name.clone().unbind()));
        };
        pairs.remove(i);
        self.store(py, pairs, &[])
    }

    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let names: Vec<String> = self.pairs()?.into_iter().map(|(n, _)| n).collect();
        Ok(PyList::new(py, names)?.as_any().try_iter()?.into_any())
    }

    fn keys<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.dict(py)?.call_method0("keys")
    }

    fn values<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.dict(py)?.call_method0("values")
    }

    fn items<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.dict(py)?.call_method0("items")
    }

    #[pyo3(signature = (name, default = None))]
    fn get<'py>(&self, py: Python<'py>, name: &Bound<'py, PyAny>, default: Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
        match self.__getitem__(py, name) {
            Ok(v) => Ok(v),
            Err(e) if e.is_instance_of::<pyo3::exceptions::PyKeyError>(py) => Ok(default.unwrap_or_else(|| py.None())),
            Err(e) => Err(e),
        }
    }

    #[pyo3(signature = (name, *default))]
    fn pop<'py>(&self, py: Python<'py>, name: &Bound<'py, PyAny>, default: &Bound<'py, PyTuple>) -> PyResult<Py<PyAny>> {
        match self.__getitem__(py, name) {
            Ok(v) => {
                self.__delitem__(py, name)?;
                Ok(v)
            }
            Err(e) if e.is_instance_of::<pyo3::exceptions::PyKeyError>(py) && !default.is_empty() => {
                Ok(default.get_item(0)?.unbind())
            }
            Err(e) => Err(e),
        }
    }

    #[pyo3(signature = (name, default = None))]
    fn setdefault<'py>(&self, py: Python<'py>, name: &Bound<'py, PyAny>, default: Option<&Bound<'py, PyAny>>) -> PyResult<Py<PyAny>> {
        if let Ok(v) = self.__getitem__(py, name) {
            return Ok(v);
        }
        let value = default.cloned().unwrap_or_else(|| py.None().into_bound(py));
        self.__setitem__(py, name.extract()?, &value)?;
        Ok(value.unbind())
    }

    #[pyo3(signature = (other = None, **kw))]
    fn update<'py>(&self, py: Python<'py>, other: Option<&Bound<'py, PyAny>>, kw: Option<&Bound<'py, PyDict>>) -> PyResult<()> {
        let mut pairs = self.pairs()?;
        let mut items: Vec<(String, Key, Bound<'py, PyAny>)> = Vec::new();
        if let Some(o) = other {
            items.extend(registry_pairs(o)?);
        }
        if let Some(kw) = kw {
            items.extend(registry_pairs(kw.as_any())?);
        }
        let mut keep: Vec<Bound<'py, PyAny>> = Vec::new();
        for (n, k, h) in items {
            match pairs.iter_mut().find(|(x, _)| *x == n) {
                Some(p) => p.1 = k,
                None => pairs.push((n, k)),
            }
            keep.push(h);
        }
        self.store(py, pairs, &keep)
    }

    fn clear(&self, py: Python<'_>) -> PyResult<()> {
        self.store(py, Vec::new(), &[])
    }

    fn copy<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        self.dict(py)
    }

    fn __copy__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        self.dict(py)
    }

    fn __richcmp__<'py>(&self, py: Python<'py>, other: &Bound<'py, PyAny>, op: pyo3::basic::CompareOp) -> PyResult<Bound<'py, PyAny>> {
        let me = self.dict(py)?;
        let other = match other.cast::<Registry>() {
            Ok(r) => r.get().dict(py)?.into_any(),
            Err(_) => other.clone(),
        };
        me.as_any().rich_compare(other, op)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(self.dict(py)?.repr()?.to_string())
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.owner_ref)
    }
}

// ------------------------------------------------------------ Transaction

/// One History step's journal: what it wrote, in order, and the handles it
/// touched (kept alive while the step can still be undone or redone).
#[pyclass(module = "ingetrazo_core", name = "Transaction")]
pub struct PyTransaction {
    ops: Vec<Op>,
    pins: Vec<Py<PyAny>>,
}

/// The record names of `keys` as Python strings: the nodes' cached `str`
/// where they have one (an incref), new strings for the rest.
fn record_names<'py>(py: Python<'py>, d: &super::Doc, keys: &[Key], into: &mut Vec<Result<Py<PyAny>, String>>) {
    for &k in keys {
        let Some(n) = d.node(k) else { continue };
        let Some((name, slot)) = n.record_name() else { continue };
        match &n.views[slot] {
            Some(v) => into.push(Ok(v.clone_ref(py))),
            None => into.push(Err(name.to_owned())),
        }
    }
}

fn changes_dict<'py>(py: Python<'py>, ops: &[Op], inverse: bool, root: Option<Key>) -> PyResult<Bound<'py, PyDict>> {
    let scene: Vec<&'static str>;
    let registry: [Vec<String>; 3];
    let groups: Vec<Vec<Result<Py<PyAny>, String>>> = {
        let d = lock();
        let mut c = d.changes(ops, inverse, root);
        scene = c.scene_modified.clone();
        registry = [
            std::mem::take(&mut c.materials_created),
            std::mem::take(&mut c.materials_removed),
            std::mem::take(&mut c.materials_replaced),
        ];
        [&c.created, &c.removed, &c.modified, &c.tags_created, &c.tags_removed, &c.tags_modified, &c.materials_modified]
            .iter()
            .map(|keys| {
                let mut v = Vec::with_capacity(keys.len());
                record_names(py, &d, keys, &mut v);
                v
            })
            .collect()
    };
    let mut lists = Vec::with_capacity(groups.len());
    for g in groups {
        let l = PyList::empty(py);
        for item in g {
            match item {
                Ok(o) => l.append(o)?,
                Err(s) => l.append(s)?,
            }
        }
        lists.push(l);
    }
    let out = PyDict::new(py);
    let p = PyDict::new(py);
    p.set_item("created", &lists[0])?;
    p.set_item("removed", &lists[1])?;
    p.set_item("modified", &lists[2])?;
    let t = PyDict::new(py);
    t.set_item("created", &lists[3])?;
    t.set_item("removed", &lists[4])?;
    t.set_item("modified", &lists[5])?;
    out.set_item("placements", p)?;
    out.set_item("tags", t)?;
    let m = PyDict::new(py);
    m.set_item("modified", &lists[6])?;
    let [mc, mr, mx] = registry;
    m.set_item("created", mc)?;
    m.set_item("removed", mr)?;
    m.set_item("replaced", mx)?;
    out.set_item("materials", m)?;
    let sc = PyDict::new(py);
    sc.set_item("modified", scene)?;
    out.set_item("scene", sc)?;
    Ok(out)
}

#[pymethods]
impl PyTransaction {
    fn __len__(&self) -> usize {
        self.ops.len()
    }

    /// Put back what the step wrote (newest first).
    fn undo(&self, py: Python<'_>) -> PyResult<()> {
        self.run(py, true)
    }

    /// Write it again (oldest first).
    fn redo(&self, py: Python<'_>) -> PyResult<()> {
        self.run(py, false)
    }

    /// `{placements: {created, removed, modified: [uid]}, tags: {...: [tag
    /// key]}}`; `inverse` swaps created and removed (an undo's record).
    /// `root`: only that scene root's lists count for created/removed.
    #[pyo3(signature = (inverse = false, root = None))]
    fn changes<'py>(&self, py: Python<'py>, inverse: bool, root: Option<Key>) -> PyResult<Bound<'py, PyDict>> {
        changes_dict(py, &self.ops, inverse, root)
    }

    /// The handles of the placements this step changed: `fields` (names,
    /// as `"hidden"`, `"layer"`; all fields when empty) written to a new
    /// value, or, with `lists`, a child list edited (the owner and the
    /// members that came or went). Each once.
    #[pyo3(signature = (fields = Vec::new(), lists = false))]
    fn placements<'py>(&self, py: Python<'py>, fields: Vec<String>, lists: bool) -> PyResult<Bound<'py, PyList>> {
        let mut fs = Vec::with_capacity(fields.len());
        for name in &fields {
            fs.push(Field::from_name(name).ok_or_else(|| PyValueError::new_err(format!("no document field {name}")))?);
        }
        let hs: Vec<Py<PyAny>> = {
            let d = lock();
            let keys = d.written_placements(&self.ops, &fs, lists);
            keys.iter().filter_map(|&k| d.handle_ref(py, k)).collect()
        };
        PyList::new(py, hs)
    }

    /// The ops as `(kind, ...)` tuples, for tests and debugging.
    fn ops<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let out = PyList::empty(py);
        for op in &self.ops {
            let t = match op {
                Op::Set { k, f, .. } => ("set", *k, f.name().to_string()).into_pyobject(py)?.into_any(),
                Op::SetMany { keys, f, .. } => ("set_many", keys.len(), f.name().to_string()).into_pyobject(py)?.into_any(),
                Op::Ins { owner, at, k, .. } => ("insert", *owner, *at, *k).into_pyobject(py)?.into_any(),
                Op::Rem { owner, at, k, .. } => ("remove", *owner, *at, *k).into_pyobject(py)?.into_any(),
                Op::Rep { owner, old, new, .. } => ("replace", *owner, old.len(), new.len()).into_pyobject(py)?.into_any(),
                Op::Reg { owner, old, new } => ("registry", *owner, old.len(), new.len()).into_pyobject(py)?.into_any(),
            };
            out.append(t)?;
        }
        Ok(out)
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        for p in &self.pins {
            visit.call(p)?;
        }
        Ok(())
    }

    fn __clear__(&mut self) {
        self.pins.clear();
    }
}

impl PyTransaction {
    fn run(&self, py: Python<'_>, backwards: bool) -> PyResult<()> {
        let mut trash = Trash::default();
        let r = {
            let mut d = lock();
            d.apply(py, &self.ops, backwards, &mut trash)
        };
        drop(trash);
        drain_pending();
        let refilled = run_refills(py);
        r.map_err(PyRuntimeError::new_err)?;
        refilled
    }
}

// --------------------------------------------------------------- functions

/// A new scene root: owns `scene.groups` (`.children`) and `scene.layers`
/// (`._tags`).
#[pyfunction]
fn doc_new_root(py: Python<'_>) -> PyResult<Py<PyPlacement>> {
    drain_pending();
    let key = lock().alloc(Kind::Root);
    Py::new(py, PyPlacement { key })
}

/// Open a transaction (or a savepoint inside the open one). Returns the
/// depth; 0 when another thread's transaction is open (not journalled).
#[pyfunction]
fn doc_begin() -> usize {
    lock().begin()
}

#[pyfunction]
fn doc_depth() -> usize {
    lock().depth()
}

/// Close the innermost level; the outermost returns its `Transaction`.
#[pyfunction]
fn doc_commit(py: Python<'_>) -> PyResult<Option<Py<PyTransaction>>> {
    let got = { lock().commit() };
    match got {
        None => Ok(None),
        Some((ops, pins)) => Ok(Some(Py::new(py, PyTransaction { ops, pins })?)),
    }
}

/// Undo what the innermost level wrote and close it.
#[pyfunction]
fn doc_rollback(py: Python<'_>) -> PyResult<()> {
    let mut trash = Trash::default();
    let r = {
        let mut d = lock();
        d.rollback(py, &mut trash)
    };
    drop(trash);
    drain_pending();
    let refilled = run_refills(py);
    r.map_err(PyRuntimeError::new_err)?;
    refilled
}

/// Install the matrix-view hooks: `make(key, which, data16, src)` builds a
/// view; `bind(value, key, which)` → `(view, data16, src)`.
#[pyfunction]
fn doc_set_hooks(make: Py<PyAny>, bind: Py<PyAny>) {
    let old = HOOKS.lock().unwrap_or_else(|e| e.into_inner()).replace((make, bind));
    drop(old);
}

/// Install the tracked-container hook: `track(key, attr, container)`.
#[pyfunction]
fn doc_set_track_hook(track: Py<PyAny>) {
    let old = TRACK.lock().unwrap_or_else(|e| e.into_inner()).replace(track);
    drop(old);
}

/// A tracked container was edited in place: store its value as attribute
/// `attr` of `key` if it is still that attribute's view. Returns whether it
/// was (False: it has been replaced; the container is a plain one now).
#[pyfunction]
fn doc_attr_changed(py: Python<'_>, key: Key, attr: u16, view: &Bound<'_, PyAny>) -> PyResult<bool> {
    let bound = |d: &super::Doc| d.node(key).and_then(|n| n.attr_view(attr)).map(|v| v.as_ptr()) == Some(view.as_ptr());
    if !bound(&lock()) {
        return Ok(false);
    }
    let v = Value::from_py(view)?;
    let mut trash = Trash::default();
    let r = {
        let mut d = lock();
        if bound(&d) {
            d.write_keep_view(py, key, Field::Attr(attr), FVal::Val(Some(v)), &mut trash).map(|_| true)
        } else {
            Ok(false)
        }
    };
    drop(trash);
    drain_pending();
    r.map_err(err)
}

/// A bound matrix view was edited in place: store its values if it is
/// still the node's view. Returns whether it was.
#[pyfunction]
fn doc_view_changed(py: Python<'_>, key: Key, which: u8, view: &Bound<'_, PyAny>, data: &Bound<'_, PyAny>, src: &Bound<'_, PyAny>) -> PyResult<bool> {
    let f = if which == 1 { Field::Axes } else { Field::Xform };
    let slot = view_slot(f);
    let x = xform_of(data, src)?;
    let mut trash = Trash::default();
    let r = {
        let mut d = lock();
        let bound = match d.node(key) {
            Some(n) => n.views[slot].as_ref().map(|v| v.as_ptr()) == Some(view.as_ptr()),
            None => false,
        };
        if bound {
            d.write_keep_view(py, key, f, FVal::Xf(Some(x)), &mut trash).map(|_| true)
        } else {
            Ok(false)
        }
    };
    drop(trash);
    drain_pending();
    r.map_err(err)
}

fn fval_for(f: Field, v: &Bound<'_, PyAny>) -> PyResult<FVal> {
    Ok(match f {
        Field::Component | Field::Hidden | Field::Locked | Field::Visible => FVal::Bool(v.is_truthy()?),
        Field::Uid | Field::Name | Field::Tag | Field::Key | Field::Folder | Field::Finish => FVal::Str(str_of(v)?),
        Field::Material | Field::Color | Field::Texture | Field::Opacity | Field::Pbr => opt_value(v)?,
        Field::Attr(_) => opt_value(v)?,
        Field::Xform | Field::Axes => return Err(PyTypeError::new_err("matrices are written one placement at a time")),
    })
}

/// `setattr(h, field, value)` for every handle of `handles` in one call,
/// journalled as one op: what a command writing one value over a
/// selection uses (Lock, Hide, Tag of 20,000 placements).
#[pyfunction]
fn doc_set_many(py: Python<'_>, handles: &Bound<'_, PyAny>, field: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
    let f = Field::from_name(field).ok_or_else(|| PyValueError::new_err(format!("no document field {field}")))?;
    let v = fval_for(f, value)?;
    let members = keys_of(handles)?;
    let keys: Vec<Key> = members.iter().map(|m| m.0).collect();
    let mut trash = Trash::default();
    let r = {
        let mut d = lock();
        for (m, ptr, _) in &members {
            d.set_handle(*m, *ptr);
        }
        d.write_many(py, &keys, f, &v, &mut trash)
    };
    drop(members);
    drop(trash);
    drain_pending();
    r.map_err(err)
}

/// One field of every handle of `handles`, as a list (one call instead of
/// one attribute read each). Matrices are not read this way.
#[pyfunction]
fn doc_get_many<'py>(py: Python<'py>, handles: &Bound<'py, PyAny>, field: &str) -> PyResult<Bound<'py, PyList>> {
    let f = Field::from_name(field).ok_or_else(|| PyValueError::new_err(format!("no document field {field}")))?;
    if matches!(f, Field::Xform | Field::Axes) {
        return Err(PyTypeError::new_err("matrices are read one placement at a time"));
    }
    let members = keys_of(handles)?;
    let vals: Vec<FVal> = {
        let d = lock();
        let mut out = Vec::with_capacity(members.len());
        for (k, _, _) in &members {
            out.push(d.need(*k).map_err(err)?.get(f).map_err(err)?);
        }
        out
    };
    let out = PyList::empty(py);
    for v in vals {
        out.append(fval_to_py(py, v)?)?;
    }
    Ok(out)
}

/// The distinct placements of the trees under `roots`, in document order
/// (`core.purge.iter_groups`): one call, no list view per node.
#[pyfunction]
fn doc_walk<'py>(py: Python<'py>, roots: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let members = keys_of(roots)?;
    let hs: Vec<Py<PyAny>> = {
        let mut d = lock();
        for (m, ptr, _) in &members {
            d.set_handle(*m, *ptr);
        }
        if let Some((m, _, _)) = members.iter().find(|m| d.node(m.0).is_none()) {
            return Err(PyTypeError::new_err(format!("not a document placement: {m:#x}")));
        }
        let mut seen: super::FastSet<Key> = super::FastSet::default();
        let mut out = Vec::new();
        let mut stack: Vec<Key> = members.iter().rev().map(|m| m.0).collect();
        while let Some(k) = stack.pop() {
            if !seen.insert(k) {
                continue;
            }
            let Some(n) = d.node(k) else { continue };
            if let Some(h) = d.handle_ref(py, k) {
                out.push(h);
            }
            stack.extend(n.children.iter().rev());
        }
        out
    };
    drop(members);
    PyList::new(py, hs)
}

/// The placements under `roots` as `model_placements` walks them (pre-order,
/// a shared subtree once per occurrence): `(handles, parent, local, has,
/// hidden)` — `parent[i]` the index of the placement holding `i` (-1 for a
/// root), `local[i]` its matrix in f64 as `formats.built.placement_values`
/// reads it (16 values, Qt's column-major order; identity where `has[i]`
/// is false: no matrix), `hidden[i]` its own Hide flag. The caller composes
/// (`views.wgpu_scene`), so the products are NumPy's, bit for bit.
#[pyfunction]
fn doc_tree<'py>(
    py: Python<'py>,
    roots: &Bound<'py, PyAny>,
) -> PyResult<(
    Bound<'py, PyList>,
    Bound<'py, numpy::PyArray1<i64>>,
    Bound<'py, numpy::PyArray2<f64>>,
    Bound<'py, numpy::PyArray1<bool>>,
    Bound<'py, numpy::PyArray1<bool>>,
)> {
    let members = keys_of(roots)?;
    let mut hs: Vec<Py<PyAny>> = Vec::new();
    let mut parent: Vec<i64> = Vec::new();
    let mut local: Vec<f64> = Vec::new();
    let mut has: Vec<bool> = Vec::new();
    let mut hidden: Vec<bool> = Vec::new();
    {
        let mut d = lock();
        for (m, ptr, _) in &members {
            d.set_handle(*m, *ptr);
        }
        if let Some((m, _, _)) = members.iter().find(|m| d.node(m.0).is_none_or(|n| n.kind != Kind::Placement)) {
            return Err(PyTypeError::new_err(format!("not a document placement: {m:#x}")));
        }
        let mut stack: Vec<(Key, i64)> = members.iter().rev().map(|m| (m.0, -1)).collect();
        while let Some((k, up)) = stack.pop() {
            let Some(n) = d.node(k) else { continue };
            let super::Data::Placement(p) = &n.data else { continue };
            let i = hs.len() as i64;
            hs.push(d.handle_ref(py, k).ok_or_else(|| PyRuntimeError::new_err("a listed node lost its handle"))?);
            parent.push(up);
            hidden.push(p.hidden);
            match &p.xform {
                Some(x) => {
                    has.push(true);
                    for j in 0..16 {
                        let c = x.f[j] as f64;
                        local.push(match &x.src {
                            Some(b) if b.1[j] == c => b.0[j],
                            _ => c,
                        });
                    }
                }
                None => {
                    has.push(false);
                    for j in 0..16 {
                        local.push(if j % 5 == 0 { 1.0 } else { 0.0 });
                    }
                }
            }
            for &c in n.children.iter().rev() {
                stack.push((c, i));
            }
        }
    }
    drop(members);
    let n = hs.len();
    let local = numpy::PyArray1::from_vec(py, local).reshape([n, 16])?;
    Ok((
        PyList::new(py, hs)?,
        numpy::PyArray1::from_vec(py, parent),
        local,
        numpy::PyArray1::from_vec(py, has),
        numpy::PyArray1::from_vec(py, hidden),
    ))
}

/// The tree invariant's mode: 0 off, 1 report as warnings, 2 refuse
/// (`ValueError`, the default). Returns the previous mode.
#[pyfunction]
fn doc_parent_mode(mode: u8) -> u8 {
    super::PARENT_MODE.swap(mode.min(2), Ordering::Relaxed)
}

/// Counts for probes and tests.
#[pyfunction]
fn doc_stats(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    let (live, slots, free, depth, serial, owned) = {
        let d = lock();
        let owned = d.slots.iter().filter_map(|s| s.node.as_ref()).filter(|n| n.strong.is_some()).count();
        (d.live, d.slots.len(), d.free.len(), d.depth(), d.serial, owned)
    };
    let out = PyDict::new(py);
    out.set_item("roots", live[Kind::Root as usize])?;
    out.set_item("placements", live[Kind::Placement as usize])?;
    out.set_item("tags", live[Kind::Tag as usize])?;
    out.set_item("materials", live[Kind::Material as usize])?;
    out.set_item("slots", slots)?;
    out.set_item("free", free)?;
    out.set_item("owned", owned)?;
    out.set_item("depth", depth)?;
    out.set_item("serial", serial)?;
    Ok(out)
}

/// Bumped by every document write: a cheap change test for caches.
#[pyfunction]
fn doc_serial() -> u64 {
    lock().serial
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyPlacement>()?;
    m.add_class::<PyTag>()?;
    m.add_class::<PyMaterial>()?;
    m.add_class::<HandleList>()?;
    m.add_class::<Registry>()?;
    m.add_class::<PyTransaction>()?;
    m.add_function(wrap_pyfunction!(doc_new_root, m)?)?;
    m.add_function(wrap_pyfunction!(doc_begin, m)?)?;
    m.add_function(wrap_pyfunction!(doc_depth, m)?)?;
    m.add_function(wrap_pyfunction!(doc_commit, m)?)?;
    m.add_function(wrap_pyfunction!(doc_rollback, m)?)?;
    m.add_function(wrap_pyfunction!(doc_set_hooks, m)?)?;
    m.add_function(wrap_pyfunction!(doc_view_changed, m)?)?;
    m.add_function(wrap_pyfunction!(doc_stats, m)?)?;
    m.add_function(wrap_pyfunction!(doc_set_many, m)?)?;
    m.add_function(wrap_pyfunction!(doc_parent_mode, m)?)?;
    m.add_function(wrap_pyfunction!(doc_set_track_hook, m)?)?;
    m.add_function(wrap_pyfunction!(doc_set_refill_hook, m)?)?;
    m.add_function(wrap_pyfunction!(doc_attr_changed, m)?)?;
    m.add_class::<DocAttr>()?;
    m.add_function(wrap_pyfunction!(doc_walk, m)?)?;
    m.add_function(wrap_pyfunction!(doc_tree, m)?)?;
    m.add_function(wrap_pyfunction!(doc_get_many, m)?)?;
    m.add_function(wrap_pyfunction!(doc_serial, m)?)?;
    Ok(())
}
