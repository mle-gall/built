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
use std::collections::HashMap;
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
    ctx: Ctx,
}

/// What `visible_placements` carries down to a node's children: whether a
/// placement above is hidden, the tag a hidden or locked tag above forces
/// on every instance below, the tag the node shows (its own or inherited),
/// and the node's own tag.
#[derive(Clone, Default)]
struct Ctx {
    hidden: bool,
    forced: Option<String>,
    inherited: Option<String>,
    own: Option<String>,
}

/// The tags' (shown, locked), as `scene.layer(name)` finds them: by key
/// (the first with it), else by label (the first with it).
struct Vis {
    by_key: HashMap<String, (bool, bool)>,
    by_name: HashMap<String, (bool, bool)>,
}

impl Vis {
    fn read(py: Python<'_>, scene: &Bound<'_, PyAny>) -> PyResult<Vis> {
        let effective = super::module(py, "built_core")?.getattr("tag_edits")?.getattr("effective_visible")?;
        let (mut by_key, mut by_name) = (HashMap::new(), HashMap::new());
        for ly in scene.getattr("layers")?.try_iter()? {
            let ly = ly?;
            let state = || -> PyResult<(bool, bool)> {
                Ok((effective.call1((scene, &ly))?.is_truthy()?, ly.getattr("locked")?.is_truthy()?))
            };
            let st = state()?;
            if let Ok(k) = ly.getattr("key")?.extract::<String>() {
                by_key.entry(k).or_insert(st);
            }
            if let Ok(n) = ly.getattr("name")?.extract::<String>() {
                by_name.entry(n).or_insert(st);
            }
        }
        Ok(Vis { by_key, by_name })
    }
    fn of(&self, tag: &Option<String>) -> Option<(bool, bool)> {
        let t = tag.as_ref()?;
        self.by_key.get(t).or_else(|| self.by_name.get(t)).copied()
    }
    fn shown(&self, tag: &Option<String>) -> bool {
        self.of(tag).is_none_or(|(shown, _)| shown)
    }
    fn blocking(&self, tag: &Option<String>) -> bool {
        self.of(tag).is_some_and(|(shown, locked)| !shown || locked)
    }
}

/// A placement's tag and Hide: from its node, else its attributes.
fn tag_and_hide(node: &Node, obj: &Bound<'_, PyAny>) -> PyResult<(Option<String>, bool)> {
    if let Node::Doc(k) = node {
        let d = doc::lock();
        if let Some(Data::Placement(p)) = d.node(*k).map(|n| &n.data) {
            return Ok((p.tag.clone().filter(|t| !t.is_empty()), p.hidden));
        }
    }
    let tag = obj.getattr_opt("layer")?.filter(|t| t.is_truthy().unwrap_or(false)).and_then(|t| t.extract::<String>().ok());
    let hidden = obj.getattr_opt("hidden")?.is_some_and(|h| h.is_truthy().unwrap_or(false));
    Ok((tag, hidden))
}

#[pyclass(module = "built_core", name = "PlacementWalk")]
pub struct PlacementWalk {
    painted: bool,
    /// `visible_placements`: the tags' state (None: every placement).
    vis: Option<Vis>,
    start: Option<(Py<PyAny>, World, Py<PyAny>)>,
    /// The node just yielded: its children are read on the next step.
    pending: Option<(Node, Py<PyAny>, World, Py<PyAny>, Ctx)>,
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
            let ctx = match &self.vis {
                None => Ctx::default(),
                Some(vis) => {
                    let (own, hidden) = tag_and_hide(&node, &group)?;
                    let forced = if vis.blocking(&own) { own.clone() } else { None };
                    Ctx { hidden, forced, inherited: None, own }
                }
            };
            self.pending = Some((node, group.unbind(), world, paint, ctx));
            return Ok(out);
        }
        loop {
            if let Some((node, obj, world, paint, ctx)) = self.pending.take() {
                let children = children_of(py, &node, obj.bind(py))?;
                if !children.is_empty() {
                    self.stack.push(Frame { children, next: 0, world, paint, ctx });
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
            let parent = top.ctx.clone();
            let world = compose(py, &world, &child, &node)?;
            let paint = if self.painted { paint_of(&child, &above)? } else { above };
            let Some(vis) = &self.vis else {
                let out = self.emit(py, child.clone(), &world, &paint)?;
                self.pending = Some((node, child.unbind(), world, paint, Ctx::default()));
                return Ok(out);
            };
            // layer = forced or child.layer or inherited or node.layer
            let (own, hidden) = tag_and_hide(&node, &child)?;
            let own_tag = own.clone();
            let tag = parent.forced.clone().or(own).or(parent.inherited.clone()).or(parent.own.clone());
            let off = parent.hidden || hidden;
            let show = !off && vis.shown(&tag);
            let forced = parent.forced.clone().or_else(|| vis.blocking(&tag).then(|| tag.clone()).flatten());
            let ctx = Ctx { hidden: off, forced, inherited: tag, own: own_tag };
            if show {
                let out = self.emit(py, child.clone(), &world, &paint)?;
                self.pending = Some((node, child.unbind(), world, paint, ctx));
                return Ok(out);
            }
            self.pending = Some((node, child.unbind(), world, paint, ctx));
        }
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        let world = |w: &World| w.obj.iter().try_for_each(|o| visit.call(o));
        if let Some((g, w, p)) = &self.start {
            visit.call(g)?;
            world(w)?;
            visit.call(p)?;
        }
        if let Some((_, o, w, p, _)) = &self.pending {
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
    Ok(PlacementWalk { painted, vis: None, start: Some((group.clone().unbind(), base, paint)), pending: None, stack: Vec::new() })
}

/// `visible_placements(scene, group)`: `iter_placements` without the
/// nested placements the viewport does not draw, for a top-level `group`
/// the caller already found visible. Hide on a group takes its subtree
/// along; a nested group with no tag of its own takes the nearest one
/// above it; a hidden or locked tag above puts every instance below it on
/// that tag. The tags' state is read once per walk; each placement's tag
/// and Hide from its node.
#[pyfunction]
fn group_visible_placements(py: Python<'_>, scene: &Bound<'_, PyAny>, group: &Bound<'_, PyAny>) -> PyResult<PlacementWalk> {
    let mut w = walk(py, false, group, None, None)?;
    w.vis = Some(Vis::read(py, scene)?);
    Ok(w)
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

// ---- model_placements: f64 matrices as NumPy composes them ------------------

/// A 4 x 4 product `a @ b` (row-major) as NumPy's on this Mac (Accelerate):
/// each element `fma(a2, b2, a0 b0) + fma(a3, b3, a1 b1)`, measured equal on
/// 48,000 elements (`tests/test_model_placements_golden.py`).
pub(crate) fn numpy_matmul4(a: &[f64; 16], b: &[f64; 16]) -> [f64; 16] {
    std::array::from_fn(|i| {
        let (r, c) = (i / 4, i % 4);
        let t = |k: usize| (a[r * 4 + k], b[k * 4 + c]);
        let ((a0, b0), (a1, b1), (a2, b2), (a3, b3)) = (t(0), t(1), t(2), t(3));
        a2.mul_add(b2, a0 * b0) + a3.mul_add(b3, a1 * b1)
    })
}

/// A placement's matrix as `formats.built.placement_values` reads it (16
/// values, Qt's column-major storage), or None without one.
fn model_values(py: Python<'_>, node: &Node, obj: &Bound<'_, PyAny>) -> PyResult<Option<[f64; 16]>> {
    if let Node::Doc(k) = node {
        let d = doc::lock();
        if let Some(Data::Placement(p)) = d.node(*k).map(|n| &n.data) {
            return Ok(p.xform.as_ref().map(|x| x.doubles()));
        }
    }
    match obj.getattr_opt("xform")? {
        Some(x) if !x.is_none() => {
            let v: Vec<f64> = super::module(py, "built_core")?.call_method1("built_placement_values", (x,))?.extract()?;
            Ok(Some(v.try_into().map_err(|_| PyRuntimeError::new_err("a matrix of 16 values"))?))
        }
        _ => Ok(None),
    }
}

fn array4<'py>(py: Python<'py>, v: &[f64; 16]) -> PyResult<Py<PyAny>> {
    let a = numpy::ndarray::Array2::from_shape_vec((4, 4), v.to_vec()).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    Ok(numpy::PyArray2::from_owned_array(py, a).into_any().unbind())
}

struct ModelFrame {
    children: Vec<(Node, Option<Py<PyAny>>)>,
    next: usize,
    matrix: Option<(Py<PyAny>, [f64; 16])>,
}

#[pyclass(module = "built_core", name = "ModelPlacementWalk")]
pub struct ModelWalk {
    start: Option<(Py<PyAny>, Option<(Py<PyAny>, [f64; 16])>)>,
    pending: Option<(Node, Py<PyAny>, Option<(Py<PyAny>, [f64; 16])>)>,
    stack: Vec<ModelFrame>,
    visible: Option<Py<PyAny>>,
}

impl ModelWalk {
    /// `matrix = base; if xform: matrix = values (@ base)`.
    fn compose(py: Python<'_>, node: &Node, obj: &Bound<'_, PyAny>, base: &Option<(Py<PyAny>, [f64; 16])>)
               -> PyResult<Option<(Py<PyAny>, [f64; 16])>> {
        Ok(match (model_values(py, node, obj)?, base) {
            (None, b) => b.as_ref().map(|(o, v)| (o.clone_ref(py), *v)),
            (Some(own), None) => Some((array4(py, &own)?, own)),
            (Some(own), Some((_, b))) => {
                let m = numpy_matmul4(&own, b);
                Some((array4(py, &m)?, m))
            }
        })
    }

    fn emit(py: Python<'_>, group: &Bound<'_, PyAny>, m: &Option<(Py<PyAny>, [f64; 16])>) -> PyResult<Py<PyAny>> {
        let m = match m {
            Some((o, _)) => o.clone_ref(py).into_bound(py),
            None => py.None().into_bound(py),
        };
        Ok(PyTuple::new(py, [group.clone(), m])?.into_any().unbind())
    }
}

#[pymethods]
impl ModelWalk {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some((group, base)) = self.start.take() {
            let group = group.into_bound(py);
            let node = node_of(&group);
            let m = Self::compose(py, &node, &group, &base)?;
            let out = Self::emit(py, &group, &m)?;
            self.pending = Some((node, group.unbind(), m));
            return Ok(out);
        }
        loop {
            if let Some((node, obj, m)) = self.pending.take() {
                let children = children_of(py, &node, obj.bind(py))?;
                if !children.is_empty() {
                    self.stack.push(ModelFrame { children, next: 0, matrix: m });
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
            let base = top.matrix.as_ref().map(|(o, v)| (o.clone_ref(py), *v));
            if let Some(f) = &self.visible {
                if !f.call1(py, (&child,))?.is_truthy(py)? {
                    continue;
                }
            }
            let m = Self::compose(py, &node, &child, &base)?;
            let out = Self::emit(py, &child, &m)?;
            self.pending = Some((node, child.unbind(), m));
            return Ok(out);
        }
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        if let Some((g, b)) = &self.start {
            visit.call(g)?;
            if let Some((o, _)) = b {
                visit.call(o)?;
            }
        }
        if let Some((_, o, m)) = &self.pending {
            visit.call(o)?;
            if let Some((a, _)) = m {
                visit.call(a)?;
            }
        }
        for f in &self.stack {
            if let Some((a, _)) = &f.matrix {
                visit.call(a)?;
            }
            for (_, h) in &f.children {
                if let Some(h) = h {
                    visit.call(h)?;
                }
            }
        }
        if let Some(v) = &self.visible {
            visit.call(v)?;
        }
        Ok(())
    }

    fn __clear__(&mut self) {
        self.start = None;
        self.pending = None;
        self.stack.clear();
        self.visible = None;
    }
}

/// `model_placements(group, base=None, visible=None)`: `(group, matrix)`
/// for `group` and every nested placement below it (a child left out with
/// its subtree when `visible(child)` is false), each matrix a 4 x 4 f64
/// array in Qt's column-major storage composed `own @ above` as NumPy
/// does, the one above handed on unchanged where a placement has none
/// (None while the chain has no matrix). Lazy, as the generator.
#[pyfunction]
#[pyo3(signature = (group, base=None, visible=None))]
fn group_model_placements(py: Python<'_>, group: &Bound<'_, PyAny>, base: Option<Bound<'_, PyAny>>,
                          visible: Option<Bound<'_, PyAny>>) -> PyResult<ModelWalk> {
    let base = match base.filter(|b| !b.is_none()) {
        Some(b) => {
            let flat = super::module(py, "numpy")?.call_method1("asarray", (&b, "float64"))?.call_method0("ravel")?;
            let v: Vec<f64> = flat.extract()?;
            let v: [f64; 16] = v.try_into().map_err(|_| PyRuntimeError::new_err("a 4 x 4 base matrix"))?;
            Some((b.unbind(), v))
        }
        None => None,
    };
    Ok(ModelWalk { start: Some((group.clone().unbind(), base)), pending: None, stack: Vec::new(),
                   visible: visible.filter(|v| !v.is_none()).map(Bound::unbind) })
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PlacementWalk>()?;
    m.add_function(wrap_pyfunction!(group_iter_placements, m)?)?;
    m.add_function(wrap_pyfunction!(group_iter_painted_placements, m)?)?;
    m.add_function(wrap_pyfunction!(group_visible_placements, m)?)?;
    m.add_class::<ModelWalk>()?;
    m.add_function(wrap_pyfunction!(group_model_placements, m)?)?;
    Ok(())
}
