// SPDX-License-Identifier: GPL-3.0-or-later
//! `core.doc_model`, served from here (`core/doc_model.py` is gone): the
//! Python handles and transaction boundaries of the Rust document.
//!
//! - `PlacedMatrix`, the `Matrix4` a placement hands out as its `xform` /
//!   `axes`: edited in place (`g.xform.translate(v)`) it writes the result
//!   back to the document, as editing the stored matrix did when the
//!   matrix was a Python attribute.
//! - The transaction helpers History drives (`begin`, `commit`,
//!   `rollback`).
//! - `new_scene_root`, the root node that owns a scene's `groups` and
//!   `layers`.
//!
//! **Tracked containers.** A custom attribute (`g.ifc`, `g.ext`,
//! `g.text3d`) or a scene table (`scene.folder_parents`,
//! `scene.plugin_data`…) is a document value. When read, a dict or a list
//! comes out as a tracked container. It is a real `dict` / `list`, so json,
//! `isinstance`, `==` and C code see one. Its in-place edits, at any depth,
//! are written back to the document, so `g.ext["link"]["x"] = 1` is
//! journalled and undoable as the assignment is. Once the attribute is
//! assigned another value (or an undo puts back an old one), the
//! container is a plain one, and editing it no longer writes.
//!
//! An edit goes to the document as a keyed patch (`doc_attr_patch`). The
//! patch is the path from the attribute's top to the entry, plus the
//! entry's new value. It is journalled as that entry's before and after,
//! at once, inside a transaction or not, so what Rust reads of the document
//! is what Python sees. Each container knows its parent (weakly) and its
//! key there, so its path is found without a scan of the value.
//!
//! A plain dict or list stored INTO one (`reg = data["links"] = {}`) is
//! kept as that very object, as Python would. Edits made through it cannot
//! be seen as they happen, so it is *suspect*: it is re-read whole at every
//! boundary. That re-read is `flush`, which Rust readers call through
//! `doc::py::fresh` when any suspect is waiting. Once nothing but its
//! parent holds it (and nothing outside holds a container inside it), the
//! flush puts a tracked copy in its place, which nobody can tell apart,
//! and it is no longer suspect. So a suspect lives only as long as some
//! reference to it is kept.

use pyo3::exceptions::{PyAttributeError, PyKeyError, PyTypeError, PyValueError};
use pyo3::intern;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyCFunction, PyDict, PyList, PySlice, PyString, PyTuple, PyType, PyWeakrefReference};

use super::module;
use super::py_class::{class, method, property};

type Obj<'py> = Bound<'py, PyAny>;

const MODULE: &str = "core.doc_model";

/// What every container of one tracked value points at: the binding
/// (`(key, attr)`, or None once replaced) and a weak reference to the top
/// container. Weak, so a tracked value is no reference cycle: a record's
/// dicts go with their node by reference counting, not by the collector (a
/// model's links registry is tens of thousands of them).
#[pyclass(module = "core.doc_model", name = "_Anchor")]
pub struct Anchor {
    #[pyo3(get, set)]
    bind: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    root: Option<Py<PyAny>>,
}

struct State {
    tdict: Py<PyType>,
    tlist: Py<PyType>,
    placed: Py<PyType>,
    matrix: Py<PyType>,
    suspect: Py<PyDict>,
    dict_type: Py<PyType>,
    list_type: Py<PyType>,
    native: Py<PyModule>,
}

static STATE: PyOnceLock<State> = PyOnceLock::new();

fn st(py: Python<'_>) -> PyResult<&State> {
    if STATE.get(py).is_none() {
        // A Rust reader before anything imported the module.
        py.import(MODULE)?;
    }
    STATE.get(py).ok_or_else(|| PyTypeError::new_err("core.doc_model is not installed"))
}

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn weak<'py>(obj: &Obj<'py>) -> PyResult<Obj<'py>> {
    Ok(PyWeakrefReference::new(obj)?.into_any())
}

fn anchor_of<'py>(node: &Obj<'py>) -> PyResult<Bound<'py, Anchor>> {
    Ok(node.getattr(intern!(node.py(), "_doc_root"))?.cast_into::<Anchor>()?)
}

fn bound(anchor: &Bound<'_, Anchor>) -> bool {
    anchor.borrow().bind.is_some()
}

fn is_tdict(py: Python<'_>, t: &Bound<'_, PyType>) -> PyResult<bool> {
    let s = st(py)?;
    Ok(t.is(s.dict_type.bind(py)) || t.is(s.tdict.bind(py)))
}

fn is_tlist(py: Python<'_>, t: &Bound<'_, PyType>) -> PyResult<bool> {
    let s = st(py)?;
    Ok(t.is(s.list_type.bind(py)) || t.is(s.tlist.bind(py)))
}

// ---------------------------------------------------------------- matrices

fn write_back(m: &Obj<'_>) -> PyResult<()> {
    let py = m.py();
    let d = m.getattr(intern!(py, "__dict__"))?;
    let d = d.cast::<PyDict>()?;
    let Some(bind) = d.get_item(intern!(py, "_doc_bind"))? else { return Ok(()) };
    if bind.is_none() {
        return Ok(());
    }
    let src = m.getattr_opt(intern!(py, "_built_source"))?.unwrap_or_else(|| none(py));
    let data = m.call_method0(intern!(py, "data"))?;
    let changed = st(py)?.native.bind(py).getattr(intern!(py, "doc_view_changed"))?.call1((bind.get_item(0)?, bind.get_item(1)?, m, data, src))?;
    if !changed.is_truthy()? {
        // Replaced (or its placement gone): an ordinary matrix from now on.
        d.del_item(intern!(py, "_doc_bind"))?;
    }
    Ok(())
}

/// A `PlacedMatrix` in-place method: the `Matrix4` one, then the result
/// stored in the placement while this is still its matrix.
fn writer<'py>(py: Python<'py>, name: &'static str, ret_self: bool) -> PyResult<Bound<'py, PyCFunction>> {
    PyCFunction::new_closure(py, Some(c"PlacedMatrix in-place method"), None, move |args, kwargs| -> PyResult<Py<PyAny>> {
        let py = args.py();
        let me = args.get_item(0)?;
        let rest = args.get_slice(1, args.len());
        st(py)?.matrix.bind(py).getattr(name)?.call((&me,).into_pyobject(py)?.add(rest)?.cast::<PyTuple>()?, kwargs)?;
        write_back(&me)?;
        Ok(if ret_self { me.unbind() } else { py.None() })
    })
}

#[pyfunction]
fn placed_reduce<'py>(me: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    let py = me.py();
    let data: Vec<f64> = me.call_method0(intern!(py, "data"))?.extract()?;
    let rows: Vec<f64> = (0..4).flat_map(|r| (0..4).map(move |c| (r, c))).map(|(r, c)| data[c * 4 + r]).collect();
    PyTuple::new(py, [st(py)?.matrix.bind(py).clone().into_any(), PyTuple::new(py, rows)?.into_any()])
}

#[pyfunction]
#[pyo3(signature = (me, _memo=None))]
fn placed_deepcopy<'py>(me: &Obj<'py>, _memo: Option<&Obj<'py>>) -> PyResult<Obj<'py>> {
    let py = me.py();
    let out = st(py)?.matrix.bind(py).call1((me,))?;
    if let Some(src) = me.getattr_opt(intern!(py, "_built_source"))? {
        if !src.is_none() {
            out.setattr(intern!(py, "_built_source"), src)?;
        }
    }
    Ok(out)
}

/// Rust → a new view of a placement's matrix (`data` column-major).
#[pyfunction]
#[pyo3(name = "_make")]
fn make<'py>(key: &Obj<'py>, which: &Obj<'py>, data: &Obj<'py>, src: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = key.py();
    let d: Vec<Obj<'py>> = data.try_iter()?.collect::<PyResult<_>>()?;
    if d.len() != 16 {
        return Err(PyValueError::new_err("a placement matrix has 16 values"));
    }
    let rows: Vec<Obj<'py>> = (0..4).flat_map(|r| (0..4).map(move |c| c * 4 + r)).map(|i| d[i].clone()).collect();
    let m = st(py)?.placed.bind(py).call1(PyTuple::new(py, rows)?)?;
    if !src.is_none() {
        m.setattr(intern!(py, "_built_source"), src)?;
    }
    m.setattr(intern!(py, "_doc_bind"), (key, which))?;
    Ok(m)
}

fn copy_view<'py>(value: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = value.py();
    let src = value.getattr_opt(intern!(py, "_built_source"))?;
    let out = st(py)?.placed.bind(py).call1((value,))?;
    if let Some(src) = src.filter(|s| !s.is_none()) {
        out.setattr(intern!(py, "_built_source"), src)?;
    }
    Ok(out)
}

/// A matrix assigned to a placement → `(view, data, built source)`.
///
/// A plain `Matrix4` BECOMES the view (its class is switched in place), so
/// `g.xform = m; m.translate(v)` still moves `g` and `g.xform is m` still
/// holds, as with the Python attribute. A view already bound elsewhere is
/// copied: one matrix names one placement.
#[pyfunction]
#[pyo3(name = "_bind")]
fn bind_matrix<'py>(value: &Obj<'py>, key: &Obj<'py>, which: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    let py = value.py();
    let s = st(py)?;
    let t = value.get_type();
    let mut value = value.clone();
    if t.is(s.matrix.bind(py)) {
        value.setattr(intern!(py, "__class__"), s.placed.bind(py))?;
    } else if t.is(s.placed.bind(py)) {
        let d = value.getattr(intern!(py, "__dict__"))?;
        let b = d.cast::<PyDict>()?.get_item(intern!(py, "_doc_bind"))?;
        if let Some(b) = b.filter(|b| !b.is_none()) {
            if !b.eq((key, which))? {
                value = copy_view(&value)?;
            }
        }
    } else if value.is_instance(s.matrix.bind(py))? {
        value = copy_view(&value)?;
    } else {
        // 16 numbers, column-major as `Matrix4.data()` gives them.
        let raw = if value.hasattr(intern!(py, "data"))? { value.call_method0(intern!(py, "data")) } else { Ok(value.clone()) };
        let data: Option<Vec<f64>> = raw.ok().and_then(|r| r.try_iter().ok().and_then(|it| it.map(|v| v.and_then(|v| v.extract::<f64>())).collect::<PyResult<Vec<f64>>>().ok()));
        match data {
            Some(d) if d.len() == 16 => {
                value = make(key, which, &PyList::new(py, d)?.into_any(), &none(py))?;
            }
            _ => {
                let name = t.name()?;
                return Err(PyTypeError::new_err(format!("a placement matrix is a QMatrix4x4, not {name}")));
            }
        }
    }
    value.setattr(intern!(py, "_doc_bind"), (key, which))?;
    let data = value.call_method0(intern!(py, "data"))?;
    let src = value.getattr_opt(intern!(py, "_built_source"))?.unwrap_or_else(|| none(py));
    PyTuple::new(py, [value, data, src])
}

/// The tree invariant (one parent per placement, docs/rust-document-model.md
/// §2): `raise` refuses a second list with `ValueError` (the default),
/// `warn` reports it as a `UserWarning` and lets it be, `off` does not
/// look. `INGETRAZO_DOC_PARENT` chooses.
const PARENT_MODES: [(&str, u8); 3] = [("off", 0), ("warn", 1), ("raise", 2)];

/// Set the tree invariant's mode; returns the previous one.
#[pyfunction]
fn parent_mode(py: Python<'_>, mode: &str) -> PyResult<&'static str> {
    let Some((_, v)) = PARENT_MODES.iter().find(|(k, _)| *k == mode) else {
        return Err(PyKeyError::new_err(mode.to_string()));
    };
    let old: u8 = st(py)?.native.bind(py).call_method1(intern!(py, "doc_parent_mode"), (*v,))?.extract()?;
    PARENT_MODES.iter().find(|(_, x)| *x == old).map(|(k, _)| *k).ok_or_else(|| PyKeyError::new_err(old))
}

// ------------------------------------------------------- tracked containers

/// `value` as tracked containers of `anchor`; `up` (a weak reference) and
/// `key`: where it sits in its parent.
fn wrap<'py>(value: &Obj<'py>, anchor: Option<&Bound<'py, Anchor>>, up: &Obj<'py>, key: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = value.py();
    let s = st(py)?;
    let t = value.get_type();
    let dict = is_tdict(py, &t)?;
    if !dict && !is_tlist(py, &t)? {
        return Ok(value.clone());
    }
    let out = if dict { s.tdict.bind(py).call0()? } else { s.tlist.bind(py).call0()? };
    let anchor = match anchor {
        Some(a) => a.clone(),
        None => Bound::new(py, Anchor { bind: None, root: Some(weak(&out)?.unbind()) })?,
    };
    out.setattr(intern!(py, "_doc_root"), &anchor)?;
    out.setattr(intern!(py, "_doc_up"), up)?;
    out.setattr(intern!(py, "_doc_key"), key)?;
    let me = weak(&out)?;
    let nothing = none(py);
    if dict {
        let od = out.cast::<PyDict>()?;
        for (k, v) in value.cast::<PyDict>()?.iter() {
            od.set_item(&k, wrap(&v, Some(&anchor), &me, &k)?)?;
        }
    } else {
        let ol = out.cast::<PyList>()?;
        for v in value.cast::<PyList>()?.iter() {
            ol.append(wrap(&v, Some(&anchor), &me, &nothing)?)?;
        }
    }
    Ok(out)
}

fn plain_of<'py>(value: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = value.py();
    let t = value.get_type();
    if is_tdict(py, &t)? {
        let out = PyDict::new(py);
        for (k, v) in value.cast::<PyDict>()?.iter() {
            out.set_item(k, plain_of(&v)?)?;
        }
        return Ok(out.into_any());
    }
    if is_tlist(py, &t)? {
        let out = PyList::empty(py);
        for v in value.cast::<PyList>()?.iter() {
            out.append(plain_of(&v)?)?;
        }
        return Ok(out.into_any());
    }
    Ok(value.clone())
}

/// `value` with every tracked container made a plain one.
#[pyfunction]
fn plain<'py>(value: &Obj<'py>) -> PyResult<Obj<'py>> {
    plain_of(value)
}

/// `node`'s place in `parent` (a key or an index), or None; `key`: where
/// it was put (a dict's key), looked at first.
fn index<'py>(parent: &Obj<'py>, node: &Obj<'py>, key: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let py = parent.py();
    if let Ok(pd) = parent.cast::<PyDict>() {
        let mut key = key.clone();
        if key.is_none() && node.getattr_opt(intern!(py, "_doc_root"))?.is_some() {
            key = node.getattr(intern!(py, "_doc_key"))?;
        }
        if !key.is_none() {
            if let Some(v) = pd.get_item(&key)? {
                if v.is(node) {
                    return Ok(Some(key));
                }
            }
        }
        for (k, v) in pd.iter() {
            if v.is(node) {
                return Ok(Some(k));
            }
        }
        return Ok(None);
    }
    for (i, v) in parent.cast::<PyList>()?.iter().enumerate() {
        if v.is(node) {
            return Ok(Some(i.into_pyobject(py)?.into_any()));
        }
    }
    Ok(None)
}

/// The keys from the attribute's top to `node`, or None when it is no
/// longer in the value (taken out, or its attribute replaced).
fn path_of<'py>(node: &Obj<'py>) -> PyResult<Option<Vec<Obj<'py>>>> {
    let py = node.py();
    let mut path = Vec::new();
    let mut node = node.clone();
    let nothing = none(py);
    loop {
        let up = node.getattr(intern!(py, "_doc_up"))?;
        if up.is_none() {
            let root = anchor_of(&node)?.borrow().root.as_ref().map(|r| r.clone_ref(py));
            let top = match root {
                Some(r) => r.bind(py).call0()?.is(&node),
                None => false,
            };
            if !top {
                return Ok(None);
            }
            path.reverse();
            return Ok(Some(path));
        }
        let parent = up.call0()?;
        if parent.is_none() {
            return Ok(None);
        }
        match index(&parent, &node, &nothing)? {
            Some(k) => path.push(k),
            None => return Ok(None),
        }
        node = parent;
    }
}

/// `edits` ([(path, value, delete)]) into the document now; the whole
/// value when a path does not lead into it any more.
fn send<'py>(anchor: &Bound<'py, Anchor>, edits: Bound<'py, PyList>) -> PyResult<()> {
    let py = anchor.py();
    let (root, bind) = {
        let a = anchor.borrow();
        let root = match &a.root {
            Some(r) => r.bind(py).call0()?,
            None => none(py),
        };
        (root, a.bind.as_ref().map(|b| b.clone_ref(py)))
    };
    let Some(bind) = bind.filter(|_| !root.is_none()) else {
        anchor.borrow_mut().bind = None;
        return Ok(());
    };
    let bind = bind.bind(py);
    let (k, a) = (bind.get_item(0)?, bind.get_item(1)?);
    let native = st(py)?.native.bind(py);
    let ok = match native.getattr(intern!(py, "doc_attr_patch"))?.call1((&k, &a, &root, &edits)) {
        Ok(b) => b,
        Err(e) if e.is_instance_of::<PyValueError>(py) || e.is_instance_of::<PyTypeError>(py) => {
            native.getattr(intern!(py, "doc_attr_changed"))?.call1((&k, &a, &root))?
        }
        Err(e) => return Err(e),
    };
    drop(edits);
    if !ok.is_truthy()? {
        anchor.borrow_mut().bind = None; // replaced: a plain container now
    }
    Ok(())
}

/// `node` edited: `rel` is the path below it (`[key]` for one entry, `[]`
/// for the node itself, as it is now).
fn edited<'py>(node: &Obj<'py>, rel: &[Obj<'py>], value: Option<&Obj<'py>>, delete: bool) -> PyResult<()> {
    let py = node.py();
    let anchor = anchor_of(node)?;
    if !bound(&anchor) {
        return Ok(());
    }
    let Some(mut path) = path_of(node)? else { return Ok(()) };
    path.extend(rel.iter().cloned());
    let v = if rel.is_empty() && !delete { node.clone() } else { value.cloned().unwrap_or_else(|| none(py)) };
    let edit = PyTuple::new(py, [PyList::new(py, path)?.into_any(), v, pyo3::types::PyBool::new(py, delete).to_owned().into_any()])?;
    send(&anchor, PyList::new(py, [edit])?)
}

/// `value` going into `node` at `key` (None: a list's): a container this
/// root does not track there is re-read at boundaries.
fn stored<'py>(node: &Obj<'py>, key: &Obj<'py>, value: &Obj<'py>) -> PyResult<()> {
    let py = node.py();
    if !(value.is_instance_of::<PyDict>() || value.is_instance_of::<PyList>()) {
        return Ok(());
    }
    let root = anchor_of(node)?;
    let tracked = value.getattr_opt(intern!(py, "_doc_root"))?.is_some_and(|r| r.is(&root));
    let here = tracked && {
        let up = value.getattr(intern!(py, "_doc_up"))?;
        !up.is_none() && up.call0()?.is(node) && value.getattr(intern!(py, "_doc_key"))?.eq(key)?
    };
    if !here && bound(&root) {
        let entry = PyTuple::new(py, [root.into_any(), weak(node)?, key.clone(), value.clone()])?;
        st(py)?.suspect.bind(py).set_item(value.as_ptr() as usize, entry)?;
    }
    Ok(())
}

/// Write back the suspect subtrees as they are now (plain containers
/// stored into tracked ones; see the module notes), and put a tracked copy
/// in the place of each one nothing else holds any more.
#[pyfunction]
pub fn flush(py: Python<'_>) -> PyResult<()> {
    let suspect = st(py)?.suspect.bind(py).clone();
    if suspect.is_empty() {
        return Ok(());
    }
    {
        let mut by_anchor: Vec<(Bound<'_, Anchor>, Bound<'_, PyList>)> = Vec::new();
        for (sid, entry) in suspect.items().iter().map(|i| i.extract::<(Obj<'_>, Obj<'_>)>()).collect::<PyResult<Vec<_>>>()? {
            let anchor = entry.get_item(0)?.cast_into::<Anchor>()?;
            let parent = entry.get_item(1)?.call0()?;
            let key = entry.get_item(2)?;
            let value = entry.get_item(3)?;
            let place = if parent.is_none() { None } else { index(&parent, &value, &key)? };
            let path = match &place {
                Some(_) if bound(&anchor) => path_of(&parent)?,
                _ => None,
            };
            let (Some(mut path), Some(place)) = (path, place) else {
                suspect.del_item(&sid)?;
                continue;
            };
            path.push(place);
            let edit = PyTuple::new(py, [PyList::new(py, path)?.into_any(), value, pyo3::types::PyBool::new(py, false).to_owned().into_any()])?;
            match by_anchor.iter().find(|(a, _)| a.is(&anchor)) {
                Some((_, edits)) => edits.append(edit)?,
                None => by_anchor.push((anchor, PyList::new(py, [edit])?)),
            }
        }
        for (anchor, edits) in by_anchor {
            send(&anchor, edits)?;
            if !bound(&anchor) {
                let gone: Vec<Obj<'_>> = suspect.iter().filter(|(_, e)| e.get_item(0).is_ok_and(|a| a.is(&anchor))).map(|(k, _)| k).collect();
                for sid in gone {
                    suspect.del_item(sid)?;
                }
            }
        }
    } // no name holds a suspect
    for sid in suspect.keys() {
        promote(&suspect, &sid)?;
    }
    Ok(())
}

/// Whether no container inside `value` is held outside it (each one
/// referenced once, by its parent).
fn unshared(value: &Obj<'_>) -> PyResult<bool> {
    let check = |v: Obj<'_>| -> PyResult<bool> {
        // The parent's slot and this loop's reference.
        Ok(!((v.is_instance_of::<PyDict>() || v.is_instance_of::<PyList>()) && (v.get_refcnt() > 2 || !unshared(&v)?)))
    };
    if let Ok(d) = value.cast::<PyDict>() {
        for (_, v) in d.iter() {
            if !check(v)? {
                return Ok(false);
            }
        }
    } else {
        for v in value.cast::<PyList>()?.iter() {
            if !check(v)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Suspect `sid` tracked in place when nothing else holds it.
fn promote<'py>(suspect: &Bound<'py, PyDict>, sid: &Obj<'py>) -> PyResult<()> {
    let py = suspect.py();
    let Some(entry) = suspect.get_item(sid)? else { return Ok(()) };
    suspect.del_item(sid)?;
    let anchor = entry.get_item(0)?.cast_into::<Anchor>()?;
    let parent_ref = entry.get_item(1)?;
    let key = entry.get_item(2)?;
    let value = entry.get_item(3)?;
    let parent = parent_ref.call0()?;
    // While the entry lives it holds `value` too: drop it before counting.
    let keep = (anchor.clone(), parent_ref.clone(), key.clone());
    drop(entry);
    // The parent's slot and the name `value`.
    if parent.is_none() || value.get_refcnt() > 2 || !unshared(&value)? {
        let entry = PyTuple::new(py, [keep.0.into_any(), keep.1, keep.2, value])?;
        suspect.set_item(sid, entry)?;
        return Ok(());
    }
    let Some(place) = index(&parent, &value, &key)? else { return Ok(()) };
    let is_dict = parent.is_instance_of::<PyDict>();
    let tracked = wrap(&value, Some(&anchor), &weak(&parent)?, &if is_dict { place.clone() } else { none(py) })?;
    if let Ok(d) = parent.cast::<PyDict>() {
        d.set_item(place, tracked)?;
    } else {
        parent.cast::<PyList>()?.set_item(place.extract()?, tracked)?;
    }
    Ok(())
}

fn base<'py>(py: Python<'py>, dict: bool, name: &str) -> PyResult<Obj<'py>> {
    let s = st(py)?;
    if dict { s.dict_type.bind(py).getattr(name) } else { s.list_type.bind(py).getattr(name) }
}

fn call_base<'py>(me: &Obj<'py>, dict: bool, name: &str, args: &Bound<'py, PyTuple>, kwargs: Option<&Bound<'py, PyDict>>) -> PyResult<Obj<'py>> {
    let py = me.py();
    let full = (me,).into_pyobject(py)?.add(args)?;
    base(py, dict, name)?.call(full.cast::<PyTuple>()?, kwargs)
}

// TrackedDict -----------------------------------------------------------

#[pyfunction]
fn td_doc_bind<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    Ok(anchor_of(me)?.borrow().bind.as_ref().map(|b| b.bind(py).clone()).unwrap_or_else(|| none(py)))
}

#[pyfunction]
fn td_setitem<'py>(me: &Obj<'py>, k: &Obj<'py>, v: &Obj<'py>) -> PyResult<()> {
    stored(me, k, v)?;
    me.cast::<PyDict>()?.set_item(k, v)?;
    edited(me, std::slice::from_ref(k), Some(v), false)
}

#[pyfunction]
fn td_delitem<'py>(me: &Obj<'py>, k: &Obj<'py>) -> PyResult<()> {
    me.cast::<PyDict>()?.del_item(k)?;
    edited(me, std::slice::from_ref(k), None, true)
}

#[pyfunction]
fn td_clear(me: &Obj<'_>) -> PyResult<()> {
    me.cast::<PyDict>()?.clear();
    edited(me, &[], None, false)
}

#[pyfunction]
#[pyo3(signature = (me, k, *a))]
fn td_pop<'py>(me: &Obj<'py>, k: &Obj<'py>, a: &Bound<'py, PyTuple>) -> PyResult<Obj<'py>> {
    let had = me.cast::<PyDict>()?.contains(k)?;
    let args = PyTuple::new(me.py(), [k.clone()])?.as_any().add(a)?.cast_into::<PyTuple>()?;
    let out = call_base(me, true, "pop", &args, None)?;
    if had {
        edited(me, std::slice::from_ref(k), None, true)?;
    }
    Ok(out)
}

#[pyfunction]
fn td_popitem<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    let kv = call_base(me, true, "popitem", &PyTuple::empty(py), None)?;
    edited(me, &[kv.get_item(0)?], None, true)?;
    Ok(kv)
}

#[pyfunction]
#[pyo3(signature = (me, k, default=None))]
fn td_setdefault<'py>(me: &Obj<'py>, k: &Obj<'py>, default: Option<&Obj<'py>>) -> PyResult<Obj<'py>> {
    let py = me.py();
    let d = me.cast::<PyDict>()?;
    if let Some(v) = d.get_item(k)? {
        return Ok(v);
    }
    if d.contains(k)? {
        return Ok(none(py));
    }
    let default = default.cloned().unwrap_or_else(|| none(py));
    stored(me, k, &default)?;
    d.set_item(k, &default)?;
    edited(me, std::slice::from_ref(k), Some(&default), false)?;
    Ok(default)
}

fn update_from<'py>(me: &Obj<'py>, items: &Bound<'py, PyDict>) -> PyResult<()> {
    let d = me.cast::<PyDict>()?;
    for (k, v) in items.iter() {
        stored(me, &k, &v)?;
        d.set_item(&k, &v)?;
        edited(me, std::slice::from_ref(&k), Some(&v), false)?;
    }
    Ok(())
}

#[pyfunction]
#[pyo3(signature = (me, *a, **kw))]
fn td_update<'py>(me: &Obj<'py>, a: &Bound<'py, PyTuple>, kw: Option<&Bound<'py, PyDict>>) -> PyResult<()> {
    let items = st(me.py())?.dict_type.bind(me.py()).call(a, kw)?;
    update_from(me, items.cast::<PyDict>()?)
}

#[pyfunction]
fn td_ior<'py>(me: &Obj<'py>, other: &Obj<'py>) -> PyResult<Obj<'py>> {
    let items = st(me.py())?.dict_type.bind(me.py()).call1((other,))?;
    update_from(me, items.cast::<PyDict>()?)?;
    Ok(me.clone())
}

#[pyfunction]
fn td_copy<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    st(me.py())?.dict_type.bind(me.py()).call1((me,))
}

#[pyfunction]
fn tl_copy<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    st(me.py())?.list_type.bind(me.py()).call1((me,))
}

#[pyfunction]
fn tracked_deepcopy<'py>(me: &Obj<'py>, memo: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    module(py, "copy")?.getattr(intern!(py, "deepcopy"))?.call1((plain_of(me)?, memo))
}

#[pyfunction]
fn tracked_reduce<'py>(me: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    let py = me.py();
    let s = st(py)?;
    let t = if me.is_instance_of::<PyDict>() { s.dict_type.bind(py) } else { s.list_type.bind(py) };
    PyTuple::new(py, [t.clone().into_any(), PyTuple::new(py, [plain_of(me)?])?.into_any()])
}

// TrackedList -----------------------------------------------------------

#[pyfunction]
fn tl_append<'py>(me: &Obj<'py>, v: &Obj<'py>) -> PyResult<()> {
    let py = me.py();
    stored(me, &none(py), v)?;
    let l = me.cast::<PyList>()?;
    l.append(v)?;
    edited(me, &[(l.len() - 1).into_pyobject(py)?.into_any()], Some(v), false)
}

#[pyfunction]
fn tl_extend<'py>(me: &Obj<'py>, items: &Obj<'py>) -> PyResult<()> {
    let py = me.py();
    let vals: Vec<Obj<'py>> = items.try_iter()?.collect::<PyResult<_>>()?;
    for v in &vals {
        stored(me, &none(py), v)?;
    }
    call_base(me, false, "extend", &PyTuple::new(py, [PyList::new(py, vals)?])?, None)?;
    edited(me, &[], None, false)
}

#[pyfunction]
fn tl_insert<'py>(me: &Obj<'py>, i: &Obj<'py>, v: &Obj<'py>) -> PyResult<()> {
    let py = me.py();
    stored(me, &none(py), v)?;
    call_base(me, false, "insert", &PyTuple::new(py, [i, v])?, None)?;
    edited(me, &[], None, false)
}

/// A `TrackedList` method that is the `list` one, then the list written
/// again (`ret`: what it returns, `"self"` for the list).
fn list_rewriter<'py>(py: Python<'py>, name: &'static str, ret_self: bool) -> PyResult<Bound<'py, PyCFunction>> {
    PyCFunction::new_closure(py, Some(c"TrackedList edit: the list method, then the list written again"), None, move |args, kwargs| -> PyResult<Py<PyAny>> {
        let me = args.get_item(0)?;
        let rest = args.get_slice(1, args.len());
        let out = call_base(&me, false, name, &rest, kwargs)?;
        edited(&me, &[], None, false)?;
        Ok(if ret_self { me.unbind() } else { out.unbind() })
    })
}

#[pyfunction]
fn tl_setitem<'py>(me: &Obj<'py>, i: &Obj<'py>, v: &Obj<'py>) -> PyResult<()> {
    let py = me.py();
    let nothing = none(py);
    if i.is_instance_of::<PySlice>() {
        let vals: Vec<Obj<'py>> = v.try_iter()?.collect::<PyResult<_>>()?;
        for x in &vals {
            stored(me, &nothing, x)?;
        }
        call_base(me, false, "__setitem__", &PyTuple::new(py, [i.clone(), PyList::new(py, vals)?.into_any()])?, None)?;
        return edited(me, &[], None, false);
    }
    stored(me, &nothing, v)?;
    call_base(me, false, "__setitem__", &PyTuple::new(py, [i, v])?, None)?;
    let n = me.cast::<PyList>()?.len();
    let at = if i.lt(0)? { i.rem(n)? } else { i.clone() };
    edited(me, &[at], Some(v), false)
}

#[pyfunction]
fn tl_iadd<'py>(me: &Obj<'py>, items: &Obj<'py>) -> PyResult<Obj<'py>> {
    tl_extend(me, items)?;
    Ok(me.clone())
}

/// Rust → the tracked container attribute `attr` of node `key` is handed
/// out as.
#[pyfunction]
#[pyo3(name = "_track")]
fn track<'py>(key: &Obj<'py>, attr: &Obj<'py>, value: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = key.py();
    let nothing = none(py);
    let out = wrap(value, None, &nothing, &nothing)?;
    anchor_of(&out)?.borrow_mut().bind = Some(PyTuple::new(py, [key, attr])?.into_any().unbind());
    Ok(out)
}

/// A weak reference to `container` when it is tracked (a plain one held
/// inside a tracked value is re-read whole: its children need none).
fn up_of<'py>(container: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = container.py();
    if container.getattr_opt(intern!(py, "_doc_root"))?.is_some() { weak(container) } else { Ok(none(py)) }
}

fn same_kind(cur: &Obj<'_>, v: &Obj<'_>) -> PyResult<bool> {
    let py = cur.py();
    let s = st(py)?;
    let t = v.get_type();
    Ok((cur.is_instance_of::<PyDict>() && t.is(s.dict_type.bind(py))) || (cur.is_instance_of::<PyList>() && t.is(s.list_type.bind(py))))
}

/// `old` (a container in a tracked attribute, tracked or plain) made equal
/// to `new` (plain) in place, keeping the containers it holds where they
/// still are containers of the same kind: a reference held to one of them
/// sees the value too. Order follows `new`.
fn patch<'py>(old: &Obj<'py>, new: &Obj<'py>, root: &Bound<'py, Anchor>) -> PyResult<()> {
    let py = old.py();
    if let Ok(od) = old.cast::<PyDict>() {
        let up = up_of(old)?;
        let mut items = Vec::new();
        for (k, v) in new.cast::<PyDict>()?.iter() {
            match od.get_item(&k)? {
                Some(cur) if same_kind(&cur, &v)? => {
                    patch(&cur, &v, root)?;
                    items.push((k, cur));
                }
                _ => {
                    let w = wrap(&v, Some(root), &up, &k)?;
                    items.push((k, w));
                }
            }
        }
        od.clear();
        for (k, v) in items {
            od.set_item(k, v)?;
        }
        return Ok(());
    }
    let ol = old.cast::<PyList>()?;
    let up = up_of(old)?;
    let nothing = none(py);
    let mut out = Vec::new();
    for (i, v) in new.try_iter()?.enumerate() {
        let v = v?;
        let cur = if i < ol.len() { Some(ol.get_item(i)?) } else { None };
        match cur {
            Some(cur) if same_kind(&cur, &v)? => {
                patch(&cur, &v, root)?;
                out.push(cur);
            }
            _ => out.push(wrap(&v, Some(root), &up, &nothing)?),
        }
    }
    base(py, false, "__setitem__")?.call1((old, PySlice::full(py), PyList::new(py, out)?))?;
    Ok(())
}

/// Rust → an undo, redo or rollback put `value` back into the attribute
/// `view` is the tracked container of: refill it in place (a held reference
/// sees the value, as a command's own undo editing the dict in place showed
/// it). False when it cannot (another kind).
#[pyfunction]
#[pyo3(name = "_refill")]
fn refill<'py>(view: &Obj<'py>, value: &Obj<'py>) -> PyResult<bool> {
    let py = view.py();
    let s = st(py)?;
    let vt = value.get_type();
    let ok = (view.is_instance(s.tdict.bind(py))? && vt.is(s.dict_type.bind(py))) || (view.is_instance(s.tlist.bind(py))? && vt.is(s.list_type.bind(py)));
    if ok {
        patch(view, value, &anchor_of(view)?)?;
    }
    Ok(ok)
}

// ------------------------------------------------------------------ records

/// The class attribute that keeps `name` in the document.
#[pyfunction]
#[pyo3(signature = (name, track=true))]
fn attribute<'py>(py: Python<'py>, name: &str, track: bool) -> PyResult<Obj<'py>> {
    let kw = PyDict::new(py);
    kw.set_item("track", track)?;
    st(py)?.native.bind(py).getattr(intern!(py, "DocAttr"))?.call((name,), Some(&kw))
}

fn record_fields_of<'py>(me: &Obj<'py>) -> PyResult<Vec<Bound<'py, PyString>>> {
    let f = me.getattr(intern!(me.py(), "_record_fields"))?;
    f.try_iter()?.map(|n| Ok(n?.cast_into::<PyString>()?)).collect()
}

fn own_dict<'py>(me: &Obj<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
    Ok(me.getattr_opt(intern!(me.py(), "__dict__"))?.and_then(|d| d.cast_into::<PyDict>().ok()))
}

#[pyfunction]
fn record_reduce<'py>(me: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    let py = me.py();
    let state = PyDict::new(py);
    if let Some(d) = own_dict(me)? {
        state.update(d.as_mapping())?;
    }
    let fields = me.getattr(intern!(py, "_record_fields"))?;
    state.update(handle_state(me, &fields)?.as_mapping())?;
    let new = module(py, MODULE)?.getattr(intern!(py, "new_handle"))?;
    PyTuple::new(py, [new, PyTuple::new(py, [me.get_type()])?.into_any(), state.into_any()])
}

#[pyfunction]
fn record_setstate<'py>(me: &Obj<'py>, state: &Obj<'py>) -> PyResult<()> {
    set_handle_state(me, state)
}

#[pyfunction]
fn record_copy<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    let cls = me.get_type();
    let out = cls.getattr(intern!(py, "__new__"))?.call1((&cls,))?;
    for name in record_fields_of(me)? {
        out.setattr(&name, me.getattr(&name)?)?;
    }
    if let Some(d) = own_dict(me)? {
        out.getattr(intern!(py, "__dict__"))?.call_method1(intern!(py, "update"), (d,))?;
    }
    Ok(out)
}

#[pyfunction]
fn record_deepcopy<'py>(me: &Obj<'py>, memo: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    let deepcopy = module(py, "copy")?.getattr(intern!(py, "deepcopy"))?;
    let cls = me.get_type();
    let out = cls.getattr(intern!(py, "__new__"))?.call1((&cls,))?;
    memo.set_item(me.as_ptr() as usize, &out)?;
    for name in record_fields_of(me)? {
        out.setattr(&name, deepcopy.call1((me.getattr(&name)?, memo))?)?;
    }
    if let Some(d) = own_dict(me)? {
        out.getattr(intern!(py, "__dict__"))?.call_method1(intern!(py, "update"), (deepcopy.call1((d, memo))?,))?;
    }
    Ok(out)
}

/// Bind the editor's declared fields to native journalled values.
#[pyfunction]
#[pyo3(signature = (*names, extra=None))]
fn record_fields<'py>(names: &Bound<'py, PyTuple>, extra: Option<&Obj<'py>>) -> PyResult<Bound<'py, PyCFunction>> {
    let py = names.py();
    let names = names.clone().unbind();
    let extra: Py<PyTuple> = match extra {
        Some(e) => PyTuple::new(py, e.try_iter()?.collect::<PyResult<Vec<_>>>()?)?.unbind(),
        None => PyTuple::empty(py).unbind(),
    };
    PyCFunction::new_closure(py, Some(c"Bind the record class's fields to native journalled values."), None, move |args, _kw| -> PyResult<Py<PyAny>> {
        let py = args.py();
        let cls = args.get_item(0)?;
        let names = names.bind(py);
        let extra = extra.bind(py);
        let own = if names.is_empty() {
            PyTuple::new(py, cls.getattr(intern!(py, "__dataclass_fields__"))?.try_iter()?.collect::<PyResult<Vec<_>>>()?)?
        } else {
            names.clone()
        };
        let fields = own.as_any().add(extra)?.cast_into::<PyTuple>()?;
        cls.setattr(intern!(py, "_record_fields"), &fields)?;
        for name in extra.iter() {
            cls.setattr(name.cast::<PyString>()?, py.None())?;
        }
        for name in fields.iter() {
            let n: String = name.extract()?;
            cls.setattr(name.cast::<PyString>()?, attribute(py, &n, true)?)?;
        }
        Ok(cls.unbind())
    })
}

// ------------------------------------------------------------ transactions

/// Document engine name for logs and probes.
#[pyfunction]
fn engine() -> &'static str {
    "Rust"
}

/// Open the step's transaction (or a savepoint in the open one). The
/// suspect containers are written back first, so their edits made before
/// it are not this step's.
#[pyfunction]
#[pyo3(signature = (document=None))]
fn begin<'py>(py: Python<'py>, document: Option<&Obj<'py>>) -> PyResult<Obj<'py>> {
    st(py)?.native.bind(py).call_method1(intern!(py, "doc_begin"), (document,))
}

/// Close it: the outermost level returns its `Transaction` (with the
/// step's in-place edits of tracked containers written back into it).
#[pyfunction]
fn commit(py: Python<'_>) -> PyResult<Obj<'_>> {
    let native = st(py)?.native.bind(py);
    let depth: usize = native.call_method0(intern!(py, "doc_depth"))?.extract()?;
    if depth == 1 {
        flush(py)?;
    }
    native.call_method0(intern!(py, "doc_commit"))
}

/// Put back what the innermost level wrote (in-place edits of tracked
/// containers included), and close it.
#[pyfunction]
fn rollback(py: Python<'_>) -> PyResult<()> {
    flush(py)?;
    st(py)?.native.bind(py).call_method0(intern!(py, "doc_rollback"))?;
    Ok(())
}

#[pyfunction]
fn stats(py: Python<'_>) -> PyResult<Obj<'_>> {
    st(py)?.native.bind(py).call_method0(intern!(py, "doc_stats"))
}

// ----------------------------------------------------------------- handles

/// An empty handle of `cls` (`Group`, `Layer`): what copy and pickle make
/// before `set_handle_state` fills it.
#[pyfunction]
fn new_handle<'py>(cls: &Obj<'py>) -> PyResult<Obj<'py>> {
    cls.getattr(intern!(cls.py(), "__new__"))?.call1((cls,))
}

/// The fields `obj` has set, by name; lists as plain lists.
#[pyfunction]
pub fn handle_state<'py>(obj: &Obj<'py>, fields: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let py = obj.py();
    let out = PyDict::new(py);
    let handle_list = st(py)?.native.bind(py).getattr(intern!(py, "HandleList"))?;
    for name in fields.try_iter()? {
        let name = name?;
        let value = match obj.getattr(name.cast::<PyString>()?) {
            Ok(v) => v,
            Err(e) if e.is_instance_of::<PyAttributeError>(py) => continue,
            Err(e) => return Err(e),
        };
        let value = if value.get_type().is(&handle_list) { PyList::new(py, value.try_iter()?.collect::<PyResult<Vec<_>>>()?)?.into_any() } else { value };
        out.set_item(name, value)?;
    }
    Ok(out)
}

/// A list, or a document list (`g.children`, `scene.groups`,
/// `scene.layers`) that behaves as one.
#[pyfunction]
fn is_list(value: &Obj<'_>) -> PyResult<bool> {
    let py = value.py();
    Ok(value.is_instance_of::<PyList>() || value.get_type().is(&st(py)?.native.bind(py).getattr(intern!(py, "HandleList"))?))
}

/// Every attribute of a placement or a tag by name, what `vars(obj)` or
/// its `__slots__` gave when all of them were Python attributes (document
/// lists as plain lists).
#[pyfunction]
fn object_state<'py>(obj: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = obj.py();
    let group = py.import("core.group")?;
    if obj.is_instance(&group.getattr("Group")?)? {
        return Ok(handle_state(obj, &group.getattr("GROUP_FIELDS")?)?.into_any());
    }
    let layers = py.import("core.layers")?;
    if obj.is_instance(&layers.getattr("Layer")?)? {
        return layers.call_method1("layer_values", (obj,));
    }
    if let Some(d) = obj.getattr_opt(intern!(py, "__dict__"))? {
        return Ok(d);
    }
    let out = PyDict::new(py);
    for k in obj.getattr("__slots__")?.try_iter()? {
        let k = k?;
        if let Some(v) = obj.getattr_opt(k.cast::<PyString>()?)? {
            out.set_item(k, v)?;
        }
    }
    Ok(out.into_any())
}

/// Put back an `object_state` (the instance dict replaced, as
/// `vars(obj).clear(); vars(obj).update(values)` did).
#[pyfunction]
fn set_object_state<'py>(obj: &Obj<'py>, values: &Obj<'py>) -> PyResult<()> {
    let py = obj.py();
    let values = st(py)?.dict_type.bind(py).call1((values,))?.cast_into::<PyDict>()?;
    let doc = PyDict::new(py);
    let layers = py.import("core.layers")?;
    if obj.is_instance(&layers.getattr("Layer")?)? {
        for k in layers.getattr("TAG_FIELDS")?.try_iter()? {
            let k = k?;
            if let Some(v) = values.get_item(&k)? {
                values.del_item(&k)?;
                doc.set_item(k, v)?;
            }
        }
    }
    if let Some(d) = obj.getattr_opt(intern!(py, "__dict__"))? {
        d.call_method0(intern!(py, "clear"))?;
        d.call_method1(intern!(py, "update"), (&values,))?;
    } else {
        set_handle_state(obj, values.as_any())?;
    }
    set_handle_state(obj, doc.as_any())
}

pub fn set_handle_state<'py>(obj: &Obj<'py>, state: &Obj<'py>) -> PyResult<()> {
    for item in state.call_method0(intern!(obj.py(), "items"))?.try_iter()? {
        let (k, v): (Bound<'py, PyString>, Obj<'py>) = item?.extract()?;
        obj.setattr(k, v)?;
    }
    Ok(())
}

#[pyfunction]
#[pyo3(name = "set_handle_state")]
fn set_handle_state_py<'py>(obj: &Obj<'py>, state: &Obj<'py>) -> PyResult<()> {
    set_handle_state(obj, state)
}

// ------------------------------------------------------------------ roots

/// A scene's root: `.children` is `scene.groups`, `._tags` is
/// `scene.layers` (with the default tag, as the Python field had).
#[pyfunction]
fn new_scene_root(py: Python<'_>) -> PyResult<Obj<'_>> {
    let layers = py.import("core.layers")?;
    let root = st(py)?.native.bind(py).call_method0(intern!(py, "doc_new_root"))?;
    let tag = layers.getattr("Layer")?.call1((layers.getattr("DEFAULT_LAYER")?,))?;
    root.getattr(intern!(py, "_tags"))?.call_method1(intern!(py, "append"), (tag,))?;
    Ok(root)
}

// ----------------------------------------------------------------- install

fn build_state(m: &Bound<'_, PyModule>) -> PyResult<State> {
    let py = m.py();
    let native = py.import("ingetrazo_core")?;
    let builtins = py.import("builtins")?;
    let dict_type = builtins.getattr("dict")?.cast_into::<PyType>()?;
    let list_type = builtins.getattr("list")?.cast_into::<PyType>()?;
    let matrix = native.getattr("Matrix4")?.cast_into::<PyType>()?;

    // PlacedMatrix
    let ns = PyDict::new(py);
    ns.set_item("__slots__", PyTuple::empty(py))?;
    ns.set_item("__doc__", "A placement's matrix as handed out by the document. Arithmetic\nreturns plain ``QMatrix4x4`` values; the in-place methods also store the\nresult in the placement while this is still its matrix.")?;
    for name in ["translate", "rotate", "scale", "setColumn", "setRow", "setToIdentity", "fill", "lookAt", "ortho", "frustum", "perspective", "viewport", "flipCoordinates"] {
        method(&ns, name, writer(py, name, false)?)?;
    }
    for name in ["__imul__", "__iadd__", "__isub__", "__itruediv__"] {
        method(&ns, name, writer(py, name, true)?)?;
    }
    method(&ns, "__reduce__", wrap_pyfunction!(placed_reduce, m)?)?;
    method(&ns, "__deepcopy__", wrap_pyfunction!(placed_deepcopy, m)?)?;
    method(&ns, "__copy__", wrap_pyfunction!(placed_deepcopy, m)?)?;
    let placed = class(py, MODULE, "PlacedMatrix", &[matrix.clone().into_any()], &ns)?.cast_into::<PyType>()?;

    let slots = PyTuple::new(py, ["_doc_root", "_doc_up", "_doc_key", "__weakref__"])?;
    // TrackedDict
    let ns = PyDict::new(py);
    ns.set_item("__slots__", &slots)?;
    ns.set_item("__doc__", "A document attribute's dict (see core.doc_model).")?;
    property(&ns, "_doc_bind", wrap_pyfunction!(td_doc_bind, m)?.into_any(), None)?;
    method(&ns, "__setitem__", wrap_pyfunction!(td_setitem, m)?)?;
    method(&ns, "__delitem__", wrap_pyfunction!(td_delitem, m)?)?;
    method(&ns, "clear", wrap_pyfunction!(td_clear, m)?)?;
    method(&ns, "pop", wrap_pyfunction!(td_pop, m)?)?;
    method(&ns, "popitem", wrap_pyfunction!(td_popitem, m)?)?;
    method(&ns, "setdefault", wrap_pyfunction!(td_setdefault, m)?)?;
    method(&ns, "update", wrap_pyfunction!(td_update, m)?)?;
    method(&ns, "__ior__", wrap_pyfunction!(td_ior, m)?)?;
    method(&ns, "copy", wrap_pyfunction!(td_copy, m)?)?;
    method(&ns, "__copy__", wrap_pyfunction!(td_copy, m)?)?;
    method(&ns, "__deepcopy__", wrap_pyfunction!(tracked_deepcopy, m)?)?;
    method(&ns, "__reduce__", wrap_pyfunction!(tracked_reduce, m)?)?;
    let tdict = class(py, MODULE, "TrackedDict", &[dict_type.clone().into_any()], &ns)?.cast_into::<PyType>()?;

    // TrackedList
    let ns = PyDict::new(py);
    ns.set_item("__slots__", &slots)?;
    ns.set_item("__doc__", "A document attribute's list (see core.doc_model). Setting an index or\nappending is one entry; any other edit writes the list again.")?;
    property(&ns, "_doc_bind", wrap_pyfunction!(td_doc_bind, m)?.into_any(), None)?;
    method(&ns, "append", wrap_pyfunction!(tl_append, m)?)?;
    method(&ns, "extend", wrap_pyfunction!(tl_extend, m)?)?;
    method(&ns, "insert", wrap_pyfunction!(tl_insert, m)?)?;
    for name in ["remove", "pop", "clear", "sort", "reverse", "__delitem__"] {
        method(&ns, name, list_rewriter(py, name, false)?)?;
    }
    method(&ns, "__imul__", list_rewriter(py, "__imul__", true)?)?;
    method(&ns, "__setitem__", wrap_pyfunction!(tl_setitem, m)?)?;
    method(&ns, "__iadd__", wrap_pyfunction!(tl_iadd, m)?)?;
    method(&ns, "copy", wrap_pyfunction!(tl_copy, m)?)?;
    method(&ns, "__copy__", wrap_pyfunction!(tl_copy, m)?)?;
    method(&ns, "__deepcopy__", wrap_pyfunction!(tracked_deepcopy, m)?)?;
    method(&ns, "__reduce__", wrap_pyfunction!(tracked_reduce, m)?)?;
    let tlist = class(py, MODULE, "TrackedList", &[list_type.clone().into_any()], &ns)?.cast_into::<PyType>()?;

    Ok(State {
        tdict: tdict.unbind(),
        tlist: tlist.unbind(),
        placed: placed.unbind(),
        matrix: matrix.unbind(),
        suspect: PyDict::new(py).unbind(),
        dict_type: dict_type.unbind(),
        list_type: list_type.unbind(),
        native: native.unbind(),
    })
}

static RECORD: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

/// Fill the module `core.doc_model`.
pub(crate) fn install(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("__doc__", "Python handles and transaction boundaries for the Rust document (rust/ingetrazo_core/src/model/doc_model_api.rs).")?;
    let s = STATE.get_or_try_init(py, || build_state(m))?;
    let native = s.native.bind(py);
    m.add("NATIVE", native)?;
    m.add("PlacedMatrix", s.placed.bind(py))?;
    m.add("TrackedDict", s.tdict.bind(py))?;
    m.add("TrackedList", s.tlist.bind(py))?;
    m.add("_SUSPECT", s.suspect.bind(py))?;
    m.add("_Anchor", py.get_type::<Anchor>())?;
    let dict_t = s.dict_type.bind(py).clone().into_any();
    let list_t = s.list_type.bind(py).clone().into_any();
    m.add("JSON_DICTS", PyTuple::new(py, [dict_t, s.tdict.bind(py).clone().into_any()])?)?;
    m.add("JSON_LISTS", PyTuple::new(py, [list_t, s.tlist.bind(py).clone().into_any()])?)?;
    let modes = PyDict::new(py);
    for (k, v) in PARENT_MODES {
        modes.set_item(k, v)?;
    }
    m.add("PARENT_MODES", modes)?;
    m.add_function(wrap_pyfunction!(make, m)?)?;
    m.add_function(wrap_pyfunction!(bind_matrix, m)?)?;
    m.add_function(wrap_pyfunction!(parent_mode, m)?)?;
    m.add_function(wrap_pyfunction!(plain, m)?)?;
    m.add_function(wrap_pyfunction!(flush, m)?)?;
    m.add_function(wrap_pyfunction!(track, m)?)?;
    m.add_function(wrap_pyfunction!(refill, m)?)?;
    m.add_function(wrap_pyfunction!(attribute, m)?)?;
    m.add_function(wrap_pyfunction!(record_fields, m)?)?;
    m.add_function(wrap_pyfunction!(engine, m)?)?;
    m.add_function(wrap_pyfunction!(begin, m)?)?;
    m.add_function(wrap_pyfunction!(commit, m)?)?;
    m.add_function(wrap_pyfunction!(rollback, m)?)?;
    m.add_function(wrap_pyfunction!(stats, m)?)?;
    m.add_function(wrap_pyfunction!(new_handle, m)?)?;
    m.add_function(wrap_pyfunction!(handle_state, m)?)?;
    m.add_function(wrap_pyfunction!(is_list, m)?)?;
    m.add_function(wrap_pyfunction!(object_state, m)?)?;
    m.add_function(wrap_pyfunction!(set_object_state, m)?)?;
    // A function, and a method where a class keeps it as `__setstate__`
    // (`Material.__setstate__ = set_handle_state`), as the Python one was.
    m.add("set_handle_state", super::py_class::native_method(wrap_pyfunction!(set_handle_state_py, m)?)?)?;
    m.add_function(wrap_pyfunction!(new_scene_root, m)?)?;

    // Record
    let record = RECORD.get_or_try_init(py, || -> PyResult<Py<PyAny>> {
        let ns = PyDict::new(py);
        ns.set_item("__slots__", PyTuple::empty(py))?;
        ns.set_item("__doc__", "Python editor handle for a document record.")?;
        method(&ns, "__reduce__", wrap_pyfunction!(record_reduce, m)?)?;
        method(&ns, "__setstate__", wrap_pyfunction!(record_setstate, m)?)?;
        method(&ns, "__copy__", wrap_pyfunction!(record_copy, m)?)?;
        method(&ns, "__deepcopy__", wrap_pyfunction!(record_deepcopy, m)?)?;
        Ok(class(py, MODULE, "Record", &[native.getattr("DocRecord")?], &ns)?.unbind())
    })?;
    m.add("Record", record.bind(py))?;

    native.call_method1("doc_set_hooks", (m.getattr("_make")?, m.getattr("_bind")?))?;
    let env = py.import("os")?.getattr("environ")?.call_method1("get", ("INGETRAZO_DOC_PARENT",))?;
    let mode = env.extract::<Option<String>>()?.filter(|e| PARENT_MODES.iter().any(|(k, _)| k == e)).unwrap_or_else(|| "raise".into());
    parent_mode(py, &mode)?;
    native.call_method1("doc_set_track_hook", (m.getattr("_track")?,))?;
    native.call_method1("doc_set_refill_hook", (m.getattr("_refill")?,))?;
    native.call_method1("doc_set_suspects", (s.suspect.bind(py), m.getattr("flush")?))?;
    Ok(())
}
