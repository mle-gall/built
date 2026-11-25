//! The selection's storage: what `built_core.ElementSet` holds, with
//! a native snapshot readers can use without walking Python handles
//! (`docs/model-port.md`, selection).
//!
//! Membership is by identity, as it always was: a mesh element (a
//! `built_core.mesh` Vertex, Edge or Face, store views included) by its `key`,
//! anything else (a placement, a guide, a dimension…) by `id()`. The
//! entries live in a Python dict keyed by that identity, so insertion order,
//! replacement of a handle with the same key and every edge case are the
//! old dict's. What this adds:
//!
//! - **counts by kind**, kept as entries come and go: placements, faces,
//!   edges, vertices and the rest;
//! - **a revision**, taken from one process-wide counter on every change of
//!   content (and when a set is made), so a reader that keeps it notices a
//!   change even when the scene's selection object is replaced by another;
//! - `snapshot()`: `(count, placements, faces, edges, vertices, others,
//!   revision)` in one call, and `placement_uids()` / `element_keys(kind)`.

use std::sync::atomic::{AtomicU64, Ordering};

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

use crate::doc::py::PyPlacement;

static REVISION: AtomicU64 = AtomicU64::new(1);

fn next_revision() -> u64 {
    REVISION.fetch_add(1, Ordering::Relaxed)
}

const PLACEMENT: usize = 0;
const FACE: usize = 1;
const EDGE: usize = 2;
const VERTEX: usize = 3;
const OTHER: usize = 4;

/// `built_core.mesh`'s Vertex, Edge and Face classes.
fn element_classes(py: Python<'_>) -> PyResult<&'static [Py<PyAny>; 3]> {
    static CLASSES: pyo3::sync::PyOnceLock<[Py<PyAny>; 3]> = pyo3::sync::PyOnceLock::new();
    CLASSES.get_or_try_init(py, || {
        let mesh = py.import("built_core.mesh")?;
        Ok([mesh.getattr("Vertex")?.unbind(), mesh.getattr("Edge")?.unbind(), mesh.getattr("Face")?.unbind()])
    })
}

/// The kind of an entry, and its identity (`built_core.mesh.element_identity`).
fn kind_and_identity<'py>(item: &Bound<'py, PyAny>) -> PyResult<(usize, Bound<'py, PyAny>)> {
    let py = item.py();
    if item.is_instance_of::<PyPlacement>() {
        return Ok((PLACEMENT, identity_of(item)?));
    }
    let [vertex, edge, face] = element_classes(py)?;
    for (class, kind) in [(face, FACE), (edge, EDGE), (vertex, VERTEX)] {
        if item.is_instance(class.bind(py))? {
            return Ok((kind, item.getattr("key")?));
        }
    }
    Ok((OTHER, identity_of(item)?))
}

fn identity_of<'py>(item: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    Ok((item.as_ptr() as usize).into_pyobject(item.py())?.into_any())
}

fn kind_of(item: &Bound<'_, PyAny>) -> PyResult<usize> {
    Ok(kind_and_identity(item)?.0)
}

/// The entries of one selection (see the module notes).
#[pyclass(module = "built_core", subclass, name = "_ElementSetStorage")]
pub struct SelectionStore {
    items: Py<PyDict>,
    counts: [usize; 5],
    revision: u64,
}

impl SelectionStore {
    fn changed(&mut self) {
        self.revision = next_revision();
    }

    fn put(&mut self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let (kind, ident) = kind_and_identity(value)?;
        let items = self.items.bind(py);
        match items.get_item(&ident)? {
            Some(old) if old.is(value) => {}
            Some(old) => {
                self.counts[kind_of(&old)?] -= 1;
                self.counts[kind] += 1;
                items.set_item(&ident, value)?;
                self.changed();
            }
            None => {
                self.counts[kind] += 1;
                items.set_item(&ident, value)?;
                self.changed();
            }
        }
        Ok(())
    }

    fn recount(&mut self, py: Python<'_>) -> PyResult<()> {
        let mut counts = [0usize; 5];
        for v in self.items.bind(py).values().iter() {
            counts[kind_of(&v)?] += 1;
        }
        self.counts = counts;
        Ok(())
    }
}

#[pymethods]
impl SelectionStore {
    #[new]
    #[pyo3(signature = (values=None))]
    fn new(py: Python<'_>, values: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let mut store = SelectionStore { items: PyDict::new(py).unbind(), counts: [0; 5], revision: 0 };
        if let Some(values) = values {
            for v in values.try_iter()? {
                store.put(py, &v?)?;
            }
        }
        store.changed();
        Ok(store)
    }

    fn __traverse__(&self, visit: pyo3::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        visit.call(&self.items)
    }

    fn __clear__(&mut self, py: Python<'_>) {
        self.clear(py);
    }

    fn __len__(&self, py: Python<'_>) -> usize {
        self.items.bind(py).len()
    }

    fn __contains__(&self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<bool> {
        self.items.bind(py).contains(kind_and_identity(value)?.1)
    }

    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py,PyAny>> {
        self.items.bind(py).call_method0("values")?.try_iter().map(|i| i.into_any())
    }

    fn copy<'p>(slf: &Bound<'p,Self>) -> PyResult<Bound<'p,PyAny>> {
        slf.get_type().call1((slf,))
    }

    fn __deepcopy__<'p>(slf: &Bound<'p,Self>, memo: &Bound<'p,PyDict>) -> PyResult<Bound<'p,PyAny>> {
        let py = slf.py();
        let copied = slf.get_type().call0()?;
        memo.set_item(slf.as_ptr() as usize, &copied)?;
        let values = slf.borrow().items.bind(py).values();
        let items = py.import("copy")?.call_method1("deepcopy", (values,memo))?;
        copied.call_method1("update",(items,))?;
        Ok(copied)
    }

    #[pyo3(signature=(*others))]
    fn update(slf:&Bound<Self>, others:&Bound<PyTuple>) -> PyResult<()> {
        for group in others { for item in group.try_iter()? {
            slf.borrow_mut().put(slf.py(), &item?)?;
        }}
        Ok(())
    }
    #[pyo3(signature=(*others))]
    fn difference_update(slf:&Bound<Self>, others:&Bound<PyTuple>) -> PyResult<()> {
        for group in others {
            let snapshot = group.try_iter()?.collect::<PyResult<Vec<_>>>()?;
            for item in snapshot { slf.borrow_mut().discard(slf.py(), &item)?; }
        }
        Ok(())
    }
    #[pyo3(signature=(*others))]
    fn intersection_update(slf:&Bound<Self>, others:&Bound<PyTuple>) -> PyResult<()> {
        for group in others {
            let snapshot = PyList::new(slf.py(), group.try_iter()?.collect::<PyResult<Vec<_>>>()?)?;
            slf.borrow_mut().retain(slf.py(),snapshot.as_any())?;
        }
        Ok(())
    }
    #[pyo3(signature=(*others))]
    fn union<'p>(slf:&Bound<'p,Self>,others:&Bound<'p,PyTuple>) -> PyResult<Bound<'p,PyAny>> {
        let result = Self::copy(slf)?;
        result.call_method1("update",others)?;
        Ok(result)
    }
    #[pyo3(signature=(*others))]
    fn difference<'p>(slf:&Bound<'p,Self>,others:&Bound<'p,PyTuple>) -> PyResult<Bound<'p,PyAny>> {
        let result = Self::copy(slf)?;
        result.call_method1("difference_update",others)?;
        Ok(result)
    }
    #[pyo3(signature=(*others))]
    fn intersection<'p>(slf:&Bound<'p,Self>,others:&Bound<'p,PyTuple>) -> PyResult<Bound<'p,PyAny>> {
        let result = Self::copy(slf)?;
        result.call_method1("intersection_update",others)?;
        Ok(result)
    }
    fn symmetric_difference<'p>(slf:&Bound<'p,Self>,other:&Bound<'p,PyAny>) -> PyResult<Bound<'p,PyAny>> {
        slf.as_any().bitxor(slf.get_type().call1((other,))?)
    }
    fn symmetric_difference_update(slf:&Bound<Self>,other:&Bound<PyAny>) -> PyResult<()> {
        let result=Self::symmetric_difference(slf,other)?;
        slf.borrow_mut().replace(slf.py(),result.extract()?)
    }
    fn issubset(slf:&Bound<Self>,other:&Bound<PyAny>) -> PyResult<bool> {
        slf.as_any().le(slf.get_type().call1((other,))?)
    }
    fn issuperset(slf:&Bound<Self>,other:&Bound<PyAny>) -> PyResult<bool> {
        slf.as_any().ge(slf.get_type().call1((other,))?)
    }
    fn add(&mut self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.put(py, value)
    }

    fn discard(&mut self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let ident = kind_and_identity(value)?.1;
        let items = self.items.bind(py);
        if let Some(old) = items.get_item(&ident)? {
            self.counts[kind_of(&old)?] -= 1;
            items.del_item(&ident)?;
            self.changed();
        }
        Ok(())
    }

    fn clear(&mut self, py: Python<'_>) {
        let items = self.items.bind(py);
        if !items.is_empty() {
            items.clear();
            self.counts = [0; 5];
            self.changed();
        }
    }

    /// Keep only the entries whose identity one of `others` has, in their
    /// order.
    fn retain(&mut self, py: Python<'_>, others: &Bound<'_, PyAny>) -> PyResult<()> {
        let keep = pyo3::types::PySet::empty(py)?;
        for v in others.try_iter()? {
            keep.add(kind_and_identity(&v?)?.1)?;
        }
        let items = self.items.bind(py);
        let kept = PyDict::new(py);
        for (k, v) in items.iter() {
            if keep.contains(&k)? {
                kept.set_item(k, v)?;
            }
        }
        if kept.len() != items.len() {
            self.items = kept.unbind();
            self.recount(py)?;
            self.changed();
        }
        Ok(())
    }

    /// Take the entries of `other` (another store) in place of these.
    fn replace(&mut self, py: Python<'_>, other: PyRef<'_, SelectionStore>) -> PyResult<()> {
        let theirs = other.items.bind(py);
        let same = {
            let ours = self.items.bind(py);
            ours.len() == theirs.len()
                && ours.iter().zip(theirs.iter()).all(|((a, x), (b, y))| a.eq(&b).unwrap_or(false) && x.is(&y))
        };
        if !same {
            self.items = theirs.copy()?.unbind();
            self.counts = other.counts;
            self.changed();
        }
        Ok(())
    }

    /// The revision of the last change of content (process-wide counter).
    #[getter]
    fn revision(&self) -> u64 {
        self.revision
    }

    /// `(count, placements, faces, edges, vertices, others, revision)`.
    fn snapshot(&self, py: Python<'_>) -> (usize, usize, usize, usize, usize, usize, u64) {
        let c = self.counts;
        (self.items.bind(py).len(), c[PLACEMENT], c[FACE], c[EDGE], c[VERTEX], c[OTHER], self.revision)
    }

    /// The persistent uids of the selected placements, in selection order.
    fn placement_uids<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let out = PyList::empty(py);
        if self.counts[PLACEMENT] == 0 {
            return Ok(out);
        }
        for v in self.items.bind(py).values().iter() {
            if v.is_instance_of::<PyPlacement>() {
                out.append(v.getattr("uid")?)?;
            }
        }
        Ok(out)
    }

    /// The keys of the selected mesh elements of `kind` ("face", "edge",
    /// "vertex"), in selection order.
    fn element_keys<'py>(&self, py: Python<'py>, kind: &str) -> PyResult<Bound<'py, PyList>> {
        let want = match kind {
            "face" => FACE,
            "edge" => EDGE,
            "vertex" => VERTEX,
            _ => return Err(pyo3::exceptions::PyValueError::new_err("kind: face, edge or vertex")),
        };
        let out = PyList::empty(py);
        if self.counts[want] == 0 {
            return Ok(out);
        }
        for (k, v) in self.items.bind(py).iter() {
            if kind_of(&v)? == want {
                out.append(k)?;
            }
        }
        Ok(out)
    }

    /// The first and the last entry, or None (what a panel keys a cheap
    /// signature on).
    fn ends<'py>(&self, py: Python<'py>) -> PyResult<(Option<Bound<'py, PyAny>>, Option<Bound<'py, PyAny>>)> {
        let items = self.items.bind(py);
        if items.is_empty() {
            return Ok((None, None));
        }
        let first = items.iter().next().map(|(_, v)| v);
        let last_key = items.call_method0("__reversed__")?.try_iter()?.next().transpose()?;
        let last = match last_key {
            Some(k) => items.get_item(k)?,
            None => None,
        };
        Ok((first, last))
    }
}

/// The identity `ElementSet` files an entry under (`element_identity`).
#[pyfunction]
pub fn selection_element_identity<'py>(item: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    Ok(kind_and_identity(item)?.1)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    let bases = (py.get_type::<SelectionStore>(), py.import("collections.abc")?.getattr("MutableSet")?);
    let attrs = PyDict::new(py);
    attrs.set_item("__module__", "built_core")?;
    let cls = py.import("abc")?.call_method1("ABCMeta", ("ElementSet", bases, attrs))?;
    m.add("ElementSet", cls)?;
    m.add_function(wrap_pyfunction!(selection_element_identity, m)?)?;
    Ok(())
}
