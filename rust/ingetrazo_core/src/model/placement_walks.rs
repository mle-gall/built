// SPDX-License-Identifier: GPL-3.0-or-later
//! The walks over a group's placements (`core.group.iter_placements` and
//! `iter_painted_placements`) read from the Rust document: each placement's
//! children and matrix come from its node, not from Python attributes.
//!
//! They were generators and callers stop early on big models, so the walk
//! is a native iterator: one placement per `next`, a node's children read
//! when the walk comes back to it after yielding it, as a generator resumed
//! would. The matrices are the objects the Python handed out: a placement's
//! own matrix where nothing above it is transformed (its `xform`, as the
//! document gives it), the composed one above that unchanged where it has
//! none, else a new `Matrix4` of the product, glam's as `Matrix4 * m` is
//! (`tests/test_placement_walks_parity.py`).

use glam::DMat4;
use pyo3::exceptions::{PyRuntimeError, PyStopIteration};
use pyo3::prelude::*;
use pyo3::types::PyTuple;
use pyo3::{PyTraverseError, PyVisit};

use crate::doc::{self, py::handle_key, Data, Field, Key};
use crate::math::Matrix4;

/// A matrix as yielded, and its value (None: untransformed so far).
struct World {
    obj: Option<Py<PyAny>>,
    m: Option<DMat4>,
}

impl World {
    fn copy(&self, py: Python<'_>) -> World {
        World { obj: self.obj.as_ref().map(|o| o.clone_ref(py)), m: self.m }
    }
}

/// A placement of the document, or any object with `xform` / `children`
/// (records, test stand-ins, scoped copies), walked through its attributes
/// as the generator did.
enum Node {
    Doc(Key),
    Obj,
}

struct Frame {
    children: Vec<(Node, Option<Py<PyAny>>)>,
    next: usize,
    world: World,
    paint: Py<PyAny>,
}

#[pyclass(module = "ingetrazo_core", name = "PlacementWalk")]
pub struct PlacementWalk {
    painted: bool,
    start: Option<(Py<PyAny>, World, Py<PyAny>)>,
    /// The node just yielded: its children are read on the next step.
    pending: Option<(Node, Py<PyAny>, World, Py<PyAny>)>,
    stack: Vec<Frame>,
}

/// What a node is: a document placement (by key) or another object.
fn node_of(obj: &Bound<'_, PyAny>) -> Node {
    match handle_key(obj) {
        Ok(k) if matches!(doc::lock().node(k).map(|n| &n.data), Some(Data::Placement(_))) => Node::Doc(k),
        _ => Node::Obj,
    }
}

/// `m = xform; if base is not None: m = base if m is None else base * m`.
fn compose(py: Python<'_>, base: &World, handle: &Bound<'_, PyAny>, node: &Node) -> PyResult<World> {
    let (own, own_obj) = match node {
        Node::Doc(k) => {
            let d = doc::lock();
            match d.node(*k).map(|n| &n.data) {
                Some(Data::Placement(p)) => (p.xform.as_ref().map(|x| DMat4::from_cols_array(&x.f)), None),
                _ => return Err(PyRuntimeError::new_err("not a document placement")),
            }
        }
        Node::Obj => match handle.getattr_opt("xform")? {
            Some(x) if !x.is_none() => (Some(crate::math::mat(&x)?), Some(x.unbind())),
            _ => (None, None),
        },
    };
    Ok(match (&base.obj, own) {
        (Some(_), None) => base.copy(py),
        (None, None) => World { obj: None, m: None },
        (None, Some(m)) => {
            let obj = match (own_obj, node) {
                (Some(o), _) => o,
                (None, Node::Doc(k)) => doc::py::read_matrix(py, handle, *k, Field::Xform)?.unbind(),
                (None, Node::Obj) => unreachable!(),
            };
            World { obj: Some(obj), m: Some(m) }
        }
        (Some(_), Some(m)) => {
            let product = base.m.unwrap() * m;
            World { obj: Some(Py::new(py, Matrix4::from(product))?.into_any()), m: Some(product) }
        }
    })
}

/// A node's children as `(node, handle)`: from the document for a
/// placement (the handle looked up when it is reached), else
/// `getattr(obj, "children", None) or ()`.
fn children_of(py: Python<'_>, node: &Node, obj: &Bound<'_, PyAny>) -> PyResult<Vec<(Node, Option<Py<PyAny>>)>> {
    match node {
        Node::Doc(k) => {
            let d = doc::lock();
            Ok(d.node(*k).map(|n| n.children.iter().map(|c| (Node::Doc(*c), None)).collect()).unwrap_or_default())
        }
        Node::Obj => {
            let c = obj.getattr_opt("children")?;
            let Some(c) = c.filter(|c| c.is_truthy().unwrap_or(false)) else { return Ok(Vec::new()) };
            let _ = py;
            c.try_iter()?.map(|g| { let g = g?; Ok((node_of(&g), Some(g.unbind()))) }).collect()
        }
    }
}

fn paint_of<'py>(group: &Bound<'py, PyAny>, above: &Py<PyAny>) -> PyResult<Py<PyAny>> {
    let own = group.getattr_opt("material")?;
    Ok(match own {
        Some(p) if p.is_truthy()? => p.unbind(),
        _ => above.clone_ref(group.py()),
    })
}

impl PlacementWalk {
    fn emit(&self, py: Python<'_>, group: Bound<'_, PyAny>, world: &World, paint: &Py<PyAny>) -> PyResult<Py<PyAny>> {
        let m = match &world.obj {
            Some(o) => o.clone_ref(py).into_bound(py),
            None => py.None().into_bound(py),
        };
        Ok(if self.painted {
            PyTuple::new(py, [group, m, paint.clone_ref(py).into_bound(py)])?.into_any().unbind()
        } else {
            PyTuple::new(py, [group, m])?.into_any().unbind()
        })
    }
}

#[pymethods]
impl PlacementWalk {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some((group, base, paint)) = self.start.take() {
            let group = group.into_bound(py);
            let node = node_of(&group);
            let world = compose(py, &base, &group, &node)?;
            let paint = if self.painted { paint_of(&group, &paint)? } else { paint };
            let out = self.emit(py, group.clone(), &world, &paint)?;
            self.pending = Some((node, group.unbind(), world, paint));
            return Ok(out);
        }
        loop {
            if let Some((node, obj, world, paint)) = self.pending.take() {
                let children = children_of(py, &node, obj.bind(py))?;
                if !children.is_empty() {
                    self.stack.push(Frame { children, next: 0, world, paint });
                }
            }
            let Some(top) = self.stack.last_mut() else {
                return Err(PyStopIteration::new_err(()));
            };
            if top.next == top.children.len() {
                self.stack.pop();
                continue;
            }
            let i = top.next;
            top.next += 1;
            let (node, handle) = {
                let (node, handle) = &mut top.children[i];
                let node = match node { Node::Doc(k) => Node::Doc(*k), Node::Obj => Node::Obj };
                (node, handle.take())
            };
            let handle = match (handle, &node) {
                (Some(h), _) => h,
                (None, Node::Doc(k)) => doc::lock().handle_ref(py, *k)
                    .ok_or_else(|| PyRuntimeError::new_err("a child placement without its handle"))?,
                (None, Node::Obj) => unreachable!(),
            };
            let child = handle.into_bound(py);
            let (world, above) = (top.world.copy(py), top.paint.clone_ref(py));
            let world = compose(py, &world, &child, &node)?;
            let paint = if self.painted { paint_of(&child, &above)? } else { above };
            let out = self.emit(py, child.clone(), &world, &paint)?;
            self.pending = Some((node, child.unbind(), world, paint));
            return Ok(out);
        }
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        let world = |w: &World| w.obj.iter().try_for_each(|o| visit.call(o));
        if let Some((g, w, p)) = &self.start {
            visit.call(g)?;
            world(w)?;
            visit.call(p)?;
        }
        if let Some((_, o, w, p)) = &self.pending {
            visit.call(o)?;
            world(w)?;
            visit.call(p)?;
        }
        for f in &self.stack {
            world(&f.world)?;
            visit.call(&f.paint)?;
            for (_, h) in &f.children {
                if let Some(h) = h {
                    visit.call(h)?;
                }
            }
        }
        Ok(())
    }

    fn __clear__(&mut self) {
        self.start = None;
        self.pending = None;
        self.stack.clear();
    }
}

fn walk(py: Python<'_>, painted: bool, group: &Bound<'_, PyAny>, base: Option<Bound<'_, PyAny>>,
        paint: Option<Bound<'_, PyAny>>) -> PyResult<PlacementWalk> {
    let base = match base.filter(|b| !b.is_none()) {
        Some(b) => World { m: Some(crate::math::mat(&b)?), obj: Some(b.unbind()) },
        None => World { obj: None, m: None },
    };
    let paint = paint.map(Bound::unbind).unwrap_or_else(|| py.None());
    Ok(PlacementWalk { painted, start: Some((group.clone().unbind(), base, paint)), pending: None, stack: Vec::new() })
}

/// `iter_placements(group, base=None)`: `(group, world_matrix)` for `group`
/// and every nested placement below it, depth first, each matrix composed
/// from the root down (None while the whole chain is untransformed).
#[pyfunction]
#[pyo3(signature = (group, base=None))]
fn group_iter_placements(py: Python<'_>, group: &Bound<'_, PyAny>, base: Option<Bound<'_, PyAny>>) -> PyResult<PlacementWalk> {
    walk(py, false, group, base, None)
}

/// `iter_painted_placements(group, base=None, paint=None)`: the same with
/// the paint each placement's default faces show, its own or the nearest
/// painted container's above it.
#[pyfunction]
#[pyo3(signature = (group, base=None, paint=None))]
fn group_iter_painted_placements(py: Python<'_>, group: &Bound<'_, PyAny>, base: Option<Bound<'_, PyAny>>,
                                 paint: Option<Bound<'_, PyAny>>) -> PyResult<PlacementWalk> {
    walk(py, true, group, base, paint)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PlacementWalk>()?;
    m.add_function(wrap_pyfunction!(group_iter_placements, m)?)?;
    m.add_function(wrap_pyfunction!(group_iter_painted_placements, m)?)?;
    Ok(())
}
