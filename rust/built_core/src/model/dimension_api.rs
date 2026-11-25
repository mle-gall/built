// SPDX-License-Identifier: GPL-3.0-or-later
//! `built_core.dimension`, served from here (`core/dimension.py` is gone): the
//! linear dimension, a measured annotation between two world points.
//!
//! A `Dimension` records two endpoints `a`/`b` and an `offset` vector
//! giving where the dimension line sits relative to the measured segment.
//!
//! An endpoint placed ON a vertex is **anchored** to it, because a
//! dimension belongs to the geometry it measures. The endpoint reads the
//! vertex's position live, so scaling, moving or stretching the drawing
//! takes the dimension along and re-measures it. That was the first thing
//! a user of DriveMeca's video missed (2026-09-20, Marco: «dibujamos algo,
//! lo acotamos, después lo escalamos y la acotación se queda allí, no se
//! redimensiona con el dibujo y no se actualiza la nueva medición»).
//!
//! A vertex inside a component reads through its placement's transform,
//! so moving the instance moves the dimension too. When the vertex is gone
//! (erased, welded away), the endpoint freezes where it last was. An
//! endpoint snapped to a midpoint, an edge or a face has no vertex to hold
//! and stays static.
//!
//! It is an annotation, not geometry. It lives in `Scene.dimensions` and is
//! drawn as a screen-space overlay (extension lines, dimension line and
//! value label), not in the mesh.
//!
//! There are two kinds, the usual convention:
//! - **Aligned** (`axis` is None): the dimension line is parallel to a–b
//!   and measures its length.
//! - **Linear** (`axis` is `"x"`/`"y"`/`"z"`): the dimension line is
//!   parallel to that axis and measures the segment's extent along it; the
//!   extension lines are square to the axis.
//!
//! A slanted line dimensioned by pulling the cursor to its SIDE gives the
//! vertical extent; pulling ABOVE gives the horizontal one; pulling square
//! off the line gives the aligned length (@pacaeiro, issue #50: «the
//! Dimension tool should be able to measure aligned (as it does now), but
//! also linear»). The frame maths is `dimension_frame.rs`, the anchors'
//! broad phase `anchor_broad.rs`.

use pyo3::intern;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyDict, PyList, PySlice, PyString, PyTuple, PyWeakrefReference};

use super::module;
use super::py_class::{class, method, property, static_method, FastAttr};

type Obj<'py> = Bound<'py, PyAny>;

const MODULE: &str = "built_core.dimension";
/// Holds an endpoint remembers (see `live`).
const MAX_HOLDS: i64 = 8;

static ANCHOR_CLASS: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
static DIMENSION_CLASS: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn native(py: Python<'_>) -> PyResult<Obj<'_>> {
    module(py, "built_core")
}

static VEC3: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

fn vec3<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    VEC3.get_or_try_init(py, || -> PyResult<_> { Ok(native(py)?.getattr("Vec3")?.unbind()) })?.bind(py).call1((v,))
}

fn anchor_class(py: Python<'_>) -> PyResult<Obj<'_>> {
    match ANCHOR_CLASS.get(py) {
        Some(c) => Ok(c.bind(py).clone()),
        None => py.import(MODULE)?.getattr("VertexAnchor"),
    }
}

/// `getattr(scene, "loose_mesh", scene.mesh)`.
fn loose_mesh<'py>(scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = scene.py();
    let mesh = scene.getattr(intern!(py, "mesh"))?;
    Ok(scene.attr_opt(intern!(py, "loose_mesh"))?.unwrap_or(mesh))
}

// ------------------------------------------------------------ VertexAnchor

/// A hold on one mesh vertex: the vertex, the mesh that registers it (to
/// notice when it is gone) and the chain of groups whose transforms place
/// it in the world: empty for loose geometry, `[group]` for a classic
/// group, `[component, child, …]` down a nested placement.
#[pyfunction]
#[pyo3(signature = (me, vertex, mesh, chain=None))]
fn va_init<'py>(me: &Obj<'py>, vertex: &Obj<'py>, mesh: &Obj<'py>, chain: Option<&Obj<'py>>) -> PyResult<()> {
    let py = me.py();
    let chain: Vec<Obj<'py>> = match chain {
        Some(c) => c.try_iter()?.collect::<PyResult<_>>()?,
        None => Vec::new(),
    };
    me.setattr(intern!(py, "vertex"), vertex)?;
    me.setattr(intern!(py, "mesh"), mesh)?;
    me.setattr(intern!(py, "chain"), PyTuple::new(py, &chain)?)?;
    me.setattr(intern!(py, "element_key"), vertex.getattr(intern!(py, "key"))?)?;
    let path = chain.iter().map(|g| g.getattr(intern!(py, "uid"))).collect::<PyResult<Vec<_>>>()?;
    me.setattr(intern!(py, "occurrence_path"), PyTuple::new(py, path)?)
}

/// The composed placement transform, or None when every group on the
/// chain keeps its geometry in world coordinates.
fn va_matrix_of<'py>(me: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let py = me.py();
    let mut m: Option<Obj<'py>> = None;
    for g in me.getattr(intern!(py, "chain"))?.try_iter()? {
        let x = match g?.attr_opt(intern!(py, "xform"))? {
            Some(x) if !x.is_none() => x,
            _ => continue,
        };
        m = Some(match m {
            None => native(py)?.getattr(intern!(py, "Matrix4"))?.call1((x,))?,
            Some(m) => m.mul(x)?,
        });
    }
    Ok(m)
}

#[pyfunction]
fn va_matrix<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    Ok(va_matrix_of(me)?.unwrap_or_else(|| none(me.py())))
}

fn va_alive_of<'py>(me: &Obj<'py>, scene: Option<&Obj<'py>>) -> PyResult<bool> {
    let py = me.py();
    let v = me.getattr(intern!(py, "vertex"))?;
    let mesh = me.getattr(intern!(py, "mesh"))?;
    // By identity: looked up by POSITION, a vertex passing over another
    // during a Move preview read as gone, and the dimension dropped it
    // (#563). A mesh that cannot answer holds nothing.
    let held = (|| -> PyResult<bool> { Ok(v.getattr(intern!(py, "edges"))?.is_truthy()? && mesh.call_method1(intern!(py, "registers"), (&v,))?.is_truthy()?) })();
    match held {
        Ok(true) => {}
        Ok(false) => return Ok(false),
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyException>(py) => return Ok(false),
        Err(e) => return Err(e),
    }
    let Some(scene) = scene.filter(|s| !s.is_none()) else { return Ok(true) };
    let chain = me.getattr(intern!(py, "chain"))?;
    if !chain.is_truthy()? {
        return Ok(mesh.is(&loose_mesh(scene)?));
    }
    let last = chain.get_item(-1)?;
    if !last.attr_opt(intern!(py, "mesh"))?.is_some_and(|m| m.is(&mesh)) {
        // The group holds another mesh now: entering a component swaps the
        // instance onto a world copy of its definition (placement cleared),
        // leaving swaps it back. Read through the chain, the old mesh would
        // land a placement off (#621).
        return Ok(false);
    }
    if scene.has_attr(intern!(py, "resolve_occurrence"))? {
        let path = me.getattr(intern!(py, "occurrence_path"))?;
        return Ok(scene.call_method1(intern!(py, "resolve_occurrence"), (path,))?.is(&last));
    }
    match scene.attr_opt(intern!(py, "groups"))? {
        Some(groups) if groups.is_truthy()? => groups.contains(chain.get_item(0)?),
        _ => Ok(false),
    }
}

/// Whether the hold still means something: the mesh registers this very
/// vertex (not erased, not welded into another one), something references
/// it, and (given the scene) the mesh is still part of the model: the loose
/// mesh, or a group the scene still lists. A group exploded, or grouped into
/// another, leaves its old mesh behind with the vertex in it, positions
/// frozen for ever.
#[pyfunction]
#[pyo3(signature = (me, scene=None))]
fn va_alive<'py>(me: &Obj<'py>, scene: Option<&Obj<'py>>) -> PyResult<bool> {
    va_alive_of(me, scene)
}

fn placed_position<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    let p = vec3(&me.getattr(intern!(py, "vertex"))?.getattr(intern!(py, "position"))?)?;
    match va_matrix_of(me)? {
        Some(m) => m.call_method1(intern!(py, "map"), (p,)),
        None => Ok(p),
    }
}

/// Where the vertex stood when the hold died, in world coordinates (a
/// vertex welded into the one it landed on keeps that very spot), or None
/// when the hold's own mesh is no longer the one its group shows (#621:
/// the old placement would put the point off).
#[pyfunction]
fn va_last_position<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    let chain = me.getattr(intern!(py, "chain"))?;
    if chain.is_truthy()? {
        let mesh = me.getattr(intern!(py, "mesh"))?;
        if !chain.get_item(-1)?.attr_opt(intern!(py, "mesh"))?.is_some_and(|m| m.is(&mesh)) {
            return Ok(none(py));
        }
    }
    placed_position(me)
}

fn va_position_of<'py>(me: &Obj<'py>, scene: Option<&Obj<'py>>) -> PyResult<Option<Obj<'py>>> {
    if !va_alive_of(me, scene)? {
        return Ok(None);
    }
    Ok(Some(placed_position(me)?))
}

/// The vertex's world position now; None once the hold is no longer
/// `alive`, which is the anchor's cue to let go.
#[pyfunction]
#[pyo3(signature = (me, scene=None))]
fn va_position<'py>(me: &Obj<'py>, scene: Option<&Obj<'py>>) -> PyResult<Obj<'py>> {
    Ok(va_position_of(me, scene)?.unwrap_or_else(|| none(me.py())))
}

// ---------------------------------------------------------- resolving holds

#[pyfunction]
#[pyo3(name = "_anchor_candidates")]
fn anchor_candidates<'py>(scene: &Obj<'py>, point: &Obj<'py>, tol: f64) -> PyResult<Obj<'py>> {
    let py = scene.py();
    let groups = match scene.attr_opt(intern!(py, "groups"))? {
        Some(g) if g.is_truthy()? => g,
        _ => PyTuple::empty(py).into_any(),
    };
    let version = scene.attr_opt(intern!(py, "version"))?.unwrap_or_else(|| none(py));
    let version = scene.attr_opt(intern!(py, "content_version"))?.unwrap_or(version);
    let edit = scene.attr_opt(intern!(py, "edit_group"))?.unwrap_or_else(|| none(py));
    let mesh = scene.getattr(intern!(py, "mesh"))?;
    let key = PyTuple::new(py, [version, groups.len()?.into_pyobject(py)?.into_any(), (edit.as_ptr() as usize).into_pyobject(py)?.into_any(), (mesh.as_ptr() as usize).into_pyobject(py)?.into_any()])?;
    let own = scene.getattr(intern!(py, "__dict__"))?;
    let cached = own.cast::<PyDict>()?.get_item(intern!(py, "_anchor_placements"))?;
    let cached = match cached {
        Some(c) if !c.is_none() && c.get_item(0)?.eq(&key)? => c,
        _ => {
            let c = PyTuple::new(py, [key.into_any(), native(py)?.getattr(intern!(py, "AnchorPlacements"))?.call1((&groups,))?])?.into_any();
            scene.setattr(intern!(py, "_anchor_placements"), &c)?;
            c
        }
    };
    cached.get_item(1)?.call_method1(intern!(py, "candidates"), (point, tol))
}

/// The vertex of `mesh` standing at `local`, held by something, or None.
fn hit<'py>(mesh: &Obj<'py>, local: &Obj<'py>, tol: f64) -> PyResult<Option<Obj<'py>>> {
    let py = mesh.py();
    if !native(py)?.call_method1(intern!(py, "dimension_may_hold"), (mesh, local, tol))?.is_truthy()? {
        return Ok(None);
    }
    let v = mesh.call_method1(intern!(py, "vertex_at"), (local,))?;
    if v.is_none() {
        return Ok(None);
    }
    let d: f64 = v.getattr(intern!(py, "position"))?.sub(local)?.call_method0(intern!(py, "length"))?.extract()?;
    if d > tol {
        return Ok(None);
    }
    if !v.getattr(intern!(py, "edges"))?.is_truthy()? {
        // An orphan: a vertex nothing references any more. Make Group moves
        // the geometry into the group's mesh and leaves these behind in the
        // loose one, at the very same spots, so a dimension on a fresh group
        // anchored to the ghost, and the group scaled away from under it
        // (Marco, 2026-09-20: «faltaba en grupo»). A point held by nothing
        // holds nothing.
        return Ok(None);
    }
    Ok(Some(v))
}

/// The vertex sitting at `point` (world), as a `VertexAnchor`, or None.
/// Loose geometry first, then every group placement, the point taken back
/// into each placement's local coordinates: O(1) per mesh through the
/// vertex registry, so a click costs nothing on a big model. `tol` is the
/// registry's own weld resolution (0.1 mm): a point that came through a
/// placement's float32 matrix and back lands within a micron of the vertex,
/// which a tighter tolerance rejected on far-out coordinates. A lazy mesh
/// is only built when its own points say the vertex may be there
/// (`dimension_may_hold`; it and the placements' broad phase are
/// `anchor_broad.rs`).
#[pyfunction]
#[pyo3(signature = (scene, point, tol=1e-4))]
pub fn resolve_vertex_anchor<'py>(scene: &Obj<'py>, point: &Obj<'py>, tol: f64) -> PyResult<Obj<'py>> {
    let py = scene.py();
    let cls = anchor_class(py)?;
    // The LOOSE mesh, not `scene.mesh`: inside a group edit that is the
    // open group's, which the loop below finds with its chain; held as
    // loose, the anchor was dead from birth (#621).
    let loose = loose_mesh(scene)?;
    if let Some(v) = hit(&loose, &vec3(point)?, tol)? {
        return cls.call1((v, loose, PyTuple::empty(py)));
    }
    for item in anchor_candidates(scene, point, tol)?.try_iter()? {
        let item = item?;
        let (g, placed, inv) = (item.get_item(0)?, item.get_item(1)?, item.get_item(2)?);
        let local = if inv.is_none() { vec3(point)? } else { inv.call_method1(intern!(py, "map"), (vec3(point)?,))? };
        let mesh = placed.getattr(intern!(py, "mesh"))?;
        if let Some(v) = hit(&mesh, &local, tol)? {
            return cls.call1((v, mesh, chain_to(&g, &placed)?));
        }
    }
    Ok(none(py))
}

fn chain_down<'py>(root: &Obj<'py>, target: &Obj<'py>, out: &mut Vec<Obj<'py>>) -> PyResult<bool> {
    out.push(root.clone());
    if root.is(target) {
        return Ok(true);
    }
    if let Some(kids) = root.attr_opt(intern!(root.py(), "children"))? {
        if kids.is_truthy()? {
            for child in kids.try_iter()? {
                if chain_down(&child?, target, out)? {
                    return Ok(true);
                }
            }
        }
    }
    out.pop();
    Ok(false)
}

/// The groups from `root` down to `target` (inclusive): the transforms an
/// anchor composes. Depth-first, like `iter_placements`.
#[pyfunction]
#[pyo3(name = "_chain_to")]
fn chain_to<'py>(root: &Obj<'py>, target: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    let mut out = Vec::new();
    if !chain_down(root, target, &mut out)? {
        out.clear();
    }
    PyTuple::new(root.py(), out)
}

// --------------------------------------------------------------- Dimension

/// An endpoint (`"a"` or `"b"`); its attribute names are `nm_*`.
struct End {
    anchor: &'static str,
    key: &'static str,
}

const ENDS: [End; 2] = [
    End { anchor: "anchor_a", key: "a" },
    End { anchor: "anchor_b", key: "b" },
];

fn nm_attachment<'py>(py: Python<'py>, e: &End) -> &'py Bound<'py, PyString> {
    if e.key == "a" { intern!(py, "attachment_a") } else { intern!(py, "attachment_b") }
}

fn nm_frozen<'py>(py: Python<'py>, e: &End) -> &'py Bound<'py, PyString> {
    if e.key == "a" { intern!(py, "_a") } else { intern!(py, "_b") }
}

fn nm_anchor<'py>(py: Python<'py>, e: &End) -> &'py Bound<'py, PyString> {
    if e.key == "a" { intern!(py, "anchor_a") } else { intern!(py, "anchor_b") }
}

fn end(which: &str) -> PyResult<&'static End> {
    ENDS.iter().find(|e| e.key == which).ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(which.to_string()))
}

fn fresh_holds(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("a", PyList::empty(py))?;
    d.set_item("b", PyList::empty(py))?;
    Ok(d)
}

fn scene_of<'py>(me: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let py = me.py();
    match me.attr_opt(intern!(py, "_scene"))? {
        Some(r) if !r.is_none() => {
            let s = r.call0()?;
            Ok(if s.is_none() { None } else { Some(s) })
        }
        _ => Ok(None),
    }
}

#[pyfunction]
#[pyo3(signature = (me, a, b, offset, layer=None, text=None, axis=None))]
#[allow(clippy::too_many_arguments)]
fn dim_init<'py>(me: &Obj<'py>, a: &Obj<'py>, b: &Obj<'py>, offset: &Obj<'py>, layer: Option<&Obj<'py>>, text: Option<&Obj<'py>>, axis: Option<&Obj<'py>>) -> PyResult<()> {
    let py = me.py();
    let nothing = none(py);
    me.setattr(intern!(py, "attachment_a"), &nothing)?;
    me.setattr(intern!(py, "attachment_b"), &nothing)?;
    me.setattr(intern!(py, "_a"), vec3(a)?)?;
    me.setattr(intern!(py, "_b"), vec3(b)?)?;
    // displacement from the a–b segment to the dimension line
    me.setattr(intern!(py, "offset"), vec3(offset)?)?;
    // None = aligned; "x"/"y"/"z" = linear along that world axis.
    let axis = match axis {
        Some(x) if PyTuple::new(py, ["x", "y", "z"])?.contains(x)? => x.clone(),
        _ => nothing.clone(),
    };
    me.setattr(intern!(py, "axis"), axis)?;
    // Layer (tag) the annotation lives on; None = default layer.
    me.setattr(intern!(py, "layer"), layer.cloned().unwrap_or_else(|| nothing.clone()))?;
    // Custom text (double-click the value to edit it); None shows the
    // measured value, and `<>` inside the text stands for it.
    me.setattr(intern!(py, "text"), text.cloned().unwrap_or_else(|| nothing.clone()))?;
    me.setattr(intern!(py, "anchor_a"), &nothing)?;
    me.setattr(intern!(py, "anchor_b"), &nothing)?;
    // Every hold each endpoint has had, oldest first (see `live`).
    me.setattr(intern!(py, "_holds"), fresh_holds(py)?)?;
    me.setattr(intern!(py, "_scene"), &nothing)
}

fn holds_of<'py>(me: &Obj<'py>, which: &str) -> PyResult<Obj<'py>> {
    let py = me.py();
    let holds = match me.attr_opt(intern!(py, "_holds"))? {
        Some(h) if !h.is_none() => h,
        _ => {
            let h = fresh_holds(py)?.into_any();
            me.setattr(intern!(py, "_holds"), &h)?;
            h
        }
    };
    holds.get_item(if which == "a" { intern!(py, "a") } else { intern!(py, "b") })
}

#[pyfunction]
#[pyo3(name = "_holds_of")]
fn dim_holds_of<'py>(me: &Obj<'py>, which: &str) -> PyResult<Obj<'py>> {
    holds_of(me, which)
}

/// Make `anchor` the endpoint's hold and remember it.
fn hold<'py>(me: &Obj<'py>, which: &str, anchor: &Obj<'py>) -> PyResult<()> {
    let py = me.py();
    me.setattr(end(which)?.anchor, anchor)?;
    if anchor.is_none() {
        return Ok(());
    }
    let holds = holds_of(me, which)?;
    let vertex = anchor.getattr(intern!(py, "vertex"))?;
    let mesh = anchor.getattr(intern!(py, "mesh"))?;
    for h in holds.try_iter()? {
        let h = h?;
        if h.getattr(intern!(py, "vertex"))?.eq(&vertex)? && h.getattr(intern!(py, "mesh"))?.is(&mesh) {
            return Ok(());
        }
    }
    holds.call_method1(intern!(py, "append"), (anchor,))?;
    holds.del_item(PySlice::new(py, 0, (-MAX_HOLDS) as isize, 1))?;
    Ok(())
}

#[pyfunction]
#[pyo3(name = "_hold")]
fn dim_hold<'py>(me: &Obj<'py>, which: &str, anchor: &Obj<'py>) -> PyResult<()> {
    hold(me, which, anchor)
}

#[pyfunction]
fn dim_deepcopy<'py>(me: &Obj<'py>, memo: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    let out = super::doc_model_api::record_deepcopy(me, memo)?;
    if let Some(scene) = scene_of(me)? {
        let target = memo.call_method1(intern!(py, "get"), ((scene.as_ptr() as usize), &scene))?;
        out.setattr(intern!(py, "_scene"), PyWeakrefReference::new(&target)?)?;
    }
    Ok(out)
}

fn live<'py>(me: &Obj<'py>, which: &str) -> PyResult<Obj<'py>> {
    let py = me.py();
    let e = end(which)?;
    let attachment = me.getattr(nm_attachment(me.py(), e))?;
    if !attachment.is_none() {
        let point = attachment.call_method0(intern!(py, "position"))?;
        if !point.is_none() {
            me.setattr(nm_frozen(me.py(), e), &point)?;
        }
        return me.getattr(nm_frozen(me.py(), e));
    }
    let holds = holds_of(me, which)?;
    let anchor = me.getattr(nm_anchor(me.py(), e))?;
    if anchor.is_none() && !holds.is_truthy()? {
        return me.getattr(nm_frozen(me.py(), e));
    }
    let scene = scene_of(me)?;
    // The OLDEST hold still alive wins. A push collapsing the measured edge
    // welds its vertex into the one below, and the endpoint takes that one;
    // undo (or the next preview frame of the same drag) brings the first
    // vertex back, and the dimension must go back to it, not stay on the
    // one it fell onto (#254).
    let mut listed = false;
    for h in holds.try_iter()? {
        let h = h?;
        if h.is(&anchor) {
            listed = true;
        }
        if let Some(p) = va_position_of(&h, scene.as_ref())? {
            me.setattr(nm_anchor(me.py(), e), &h)?;
            me.setattr(nm_frozen(me.py(), e), &p)?;
            return Ok(p);
        }
    }
    if !anchor.is_none() && !listed && !holds.contains(&anchor)? {
        if let Some(p) = va_position_of(&anchor, scene.as_ref())? {
            me.setattr(nm_frozen(me.py(), e), &p)?;
            return Ok(p);
        }
    }
    // The hold is gone, but the geometry may just have changed hands: Make
    // Group puts new vertices in the group's mesh at the very spots the
    // loose ones stood, Explode the other way round (Marco, 2026-09-20: a
    // cube dimensioned, then cube AND dimension grouped, then scaled: «la
    // cota no sigue»). Look again where the endpoint last was; freeze only
    // when nothing stands there any more.
    let frozen = me.getattr(nm_frozen(me.py(), e))?;
    let again = resolve_again(me, which, scene.as_ref())?;
    let p = match &again {
        Some(a) => va_position_of(a, scene.as_ref())?,
        None => None,
    };
    let Some(p) = p else {
        me.setattr(nm_anchor(me.py(), e), none(py))?;
        return Ok(frozen);
    };
    hold(me, which, again.as_ref().unwrap())?;
    me.setattr(nm_frozen(me.py(), e), &p)?;
    Ok(p)
}

#[pyfunction]
#[pyo3(name = "_live")]
fn dim_live<'py>(me: &Obj<'py>, which: &str) -> PyResult<Obj<'py>> {
    live(me, which)
}

/// The vertex to hold now that every hold of the endpoint died. First where
/// the last hold's vertex died: a vertex welded into the one it was moved
/// onto keeps that spot, while the frozen position may be older (read
/// before the move) and another vertex may stand there by now (#563). Then
/// the frozen position, as before.
fn resolve_again<'py>(me: &Obj<'py>, which: &str, scene: Option<&Obj<'py>>) -> PyResult<Option<Obj<'py>>> {
    let py = me.py();
    let Some(scene) = scene else { return Ok(None) };
    let e = end(which)?;
    let holds = holds_of(me, which)?;
    let last = if holds.is_truthy()? { holds.get_item(-1)? } else { me.getattr(nm_anchor(me.py(), e))? };
    let frozen = me.getattr(nm_frozen(me.py(), e))?;
    if !last.is_none() {
        let spot = va_last_position(&last)?;
        if !spot.is_none() {
            let d: f64 = spot.sub(&frozen)?.call_method0(intern!(py, "lengthSquared"))?.extract()?;
            if d > 1e-12 {
                let found = resolve_vertex_anchor(scene, &spot, 1e-4)?;
                if !found.is_none() {
                    return Ok(Some(found));
                }
            }
        }
    }
    let found = resolve_vertex_anchor(scene, &frozen, 1e-4)?;
    Ok(if found.is_none() { None } else { Some(found) })
}

#[pyfunction]
#[pyo3(name = "_resolve_again")]
fn dim_resolve_again<'py>(me: &Obj<'py>, which: &str, scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    let s = if scene.is_none() { None } else { Some(scene) };
    Ok(resolve_again(me, which, s)?.unwrap_or_else(|| none(me.py())))
}

#[pyfunction]
fn dim_get_a<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    live(me, "a")
}

#[pyfunction]
fn dim_get_b<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    live(me, "b")
}

#[pyfunction]
fn dim_set_a<'py>(me: &Obj<'py>, value: &Obj<'py>) -> PyResult<()> {
    me.setattr(intern!(me.py(), "_a"), vec3(value)?)
}

#[pyfunction]
fn dim_set_b<'py>(me: &Obj<'py>, value: &Obj<'py>) -> PyResult<()> {
    me.setattr(intern!(me.py(), "_b"), vec3(value)?)
}

/// Anchor each endpoint to the vertex under it, if there is one: at
/// placement, and again when a document is opened (the .igz keeps
/// positions, and the vertex at that very position is the one the drafter
/// snapped to).
#[pyfunction]
fn dim_bind<'py>(me: &Obj<'py>, scene: &Obj<'py>) -> PyResult<()> {
    let py = me.py();
    me.setattr(intern!(py, "_scene"), PyWeakrefReference::new(scene)?)?;
    me.setattr(intern!(py, "_holds"), fresh_holds(py)?)?;
    let a = resolve_vertex_anchor(scene, &me.getattr(intern!(py, "_a"))?, 1e-4)?;
    hold(me, "a", &a)?;
    let b = resolve_vertex_anchor(scene, &me.getattr(intern!(py, "_b"))?, 1e-4)?;
    hold(me, "b", &b)
}

/// Re-take a hold that died (the vertex changed meshes) from the vertex
/// now standing at the endpoint; the history calls this after every
/// command, so a group made and scaled in one breath still carries its
/// dimension. An endpoint never anchored stays static.
#[pyfunction]
fn dim_refresh_anchors<'py>(me: &Obj<'py>, scene: &Obj<'py>) -> PyResult<()> {
    let py = me.py();
    me.setattr(intern!(py, "_scene"), PyWeakrefReference::new(scene)?)?;
    for e in &ENDS {
        let anchor = me.getattr(nm_anchor(me.py(), e))?;
        let holds = holds_of(me, e.key)?;
        if anchor.is_none() && !holds.is_truthy()? {
            continue;
        }
        let mut alive = false;
        for h in holds.try_iter()? {
            if va_alive_of(&h?, Some(scene))? {
                alive = true;
                break;
            }
        }
        if alive || (!anchor.is_none() && va_alive_of(&anchor, Some(scene))?) {
            continue;
        }
        let again = resolve_again(me, e.key, Some(scene))?.unwrap_or_else(|| none(py));
        hold(me, e.key, &again)?;
    }
    Ok(())
}

fn end_held(me: &Obj<'_>, e: &End) -> PyResult<bool> {
    Ok(!me.getattr(nm_anchor(me.py(), e))?.is_none() || !me.getattr(nm_attachment(me.py(), e))?.is_none())
}

/// Both endpoints held by vertices.
#[pyfunction]
fn dim_anchored(me: &Obj<'_>) -> PyResult<bool> {
    Ok(end_held(me, &ENDS[0])? && end_held(me, &ENDS[1])?)
}

/// The world positions of the anchored endpoints (0, 1 or 2).
#[pyfunction]
fn dim_anchor_points<'py>(me: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let out = PyList::empty(me.py());
    for e in &ENDS {
        if end_held(me, e)? {
            out.append(live(me, e.key)?)?;
        }
    }
    Ok(out)
}

/// What the annotation shows: the custom text with `<>` swapped for the
/// formatted measurement, or the measurement itself.
#[pyfunction]
fn dim_display_text<'py>(me: &Obj<'py>, measured: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    let text = me.getattr(intern!(py, "text"))?;
    if text.is_truthy()? {
        return text.call_method1(intern!(py, "replace"), ("<>", measured));
    }
    Ok(measured.clone())
}

fn frame_call<'py>(me: &Obj<'py>, name: &Bound<'py, PyString>, with_offset: bool) -> PyResult<Obj<'py>> {
    let py = me.py();
    let a = live(me, "a")?;
    let b = live(me, "b")?;
    let axis = me.getattr(intern!(py, "axis"))?;
    let f = native(py)?.getattr(name)?;
    if with_offset {
        f.call1((a, b, me.getattr(intern!(py, "offset"))?, axis))
    } else {
        f.call1((a, b, axis))
    }
}

/// Measured length (metres): the segment's, or (linear) its extent along
/// the axis.
#[pyfunction]
fn dim_value<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    frame_call(me, intern!(me.py(), "dimension_value"), false)
}

#[pyfunction]
fn dim_label<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    let v = dim_value(me)?;
    py.import("built_core.units")?.call_method1(intern!(py, "fmt_len"), (v,))
}

/// The offset squared to the measured segment (the geometry may have
/// turned under an anchored dimension since it was placed); linear: only
/// the pull along the extension direction.
#[pyfunction]
fn dim_perpendicular_offset<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    frame_call(me, intern!(me.py(), "dimension_perpendicular_offset"), true)
}

/// The dimension line's endpoints. Aligned: `a`/`b` shifted by the offset.
/// Linear: each endpoint pushed along the extension direction until both
/// sit on one line parallel to the axis, the one through `a + offset`.
#[pyfunction]
fn dim_line_points<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    frame_call(me, intern!(me.py(), "dimension_line_points"), true)
}

#[pyfunction]
fn dim_line_point_a<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    dim_line_points(me)?.get_item(0)
}

#[pyfunction]
fn dim_line_point_b<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    dim_line_points(me)?.get_item(1)
}

#[pyfunction]
fn dim_midpoint<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    frame_call(me, intern!(me.py(), "dimension_midpoint"), true)
}

/// Which endpoints (`"a"`, `"b"`) hold a vertex placed by one of the
/// groups whose `id` is in `group_ids`: the ones a transform of those
/// groups carries along.
#[pyfunction]
fn dim_ends_riding<'py>(me: &Obj<'py>, group_ids: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    let py = me.py();
    let mut out = Vec::new();
    for e in &ENDS {
        let an = me.getattr(nm_anchor(me.py(), e))?;
        if an.is_none() {
            continue;
        }
        for g in an.getattr(intern!(py, "chain"))?.try_iter()? {
            if group_ids.contains(g?.as_ptr() as usize)? {
                out.push(e.key);
                break;
            }
        }
    }
    PyTuple::new(py, out)
}

/// A frozen copy with the endpoints in `ends` taken through `m`: what the
/// dimension will read once a groups transform previewed through `m` is
/// committed (#621). The offset stays, as the commit leaves it;
/// `line_points` squares it to the new segment.
#[pyfunction]
fn dim_carried<'py>(me: &Obj<'py>, m: &Obj<'py>, ends: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    let c = module(py, "copy")?.call_method1(intern!(py, "copy"), (me,))?;
    let a = live(me, "a")?;
    let b = live(me, "b")?;
    let ta = if ends.contains("a")? { m.call_method1(intern!(py, "map"), (&a,))? } else { vec3(&a)? };
    let tb = if ends.contains("b")? { m.call_method1(intern!(py, "map"), (&b,))? } else { vec3(&b)? };
    c.setattr(intern!(py, "_a"), ta)?;
    c.setattr(intern!(py, "_b"), tb)?;
    c.setattr(intern!(py, "offset"), vec3(&me.getattr(intern!(py, "offset"))?)?)?;
    let nothing = none(py);
    for name in ["attachment_a", "attachment_b", "anchor_a", "anchor_b"] {
        c.setattr(name, &nothing)?;
    }
    c.setattr(intern!(py, "_holds"), fresh_holds(py)?)?;
    c.setattr(intern!(py, "_scene"), &nothing)?;
    Ok(c)
}

/// Fill the module `built_core.dimension`.
pub(crate) fn install(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("__doc__", "Linear dimension: a measured annotation between two world points (rust/built_core/src/model/dimension_api.rs).")?;
    let dm = py.import("built_core.doc_model")?;
    let record = dm.getattr("Record")?;
    let record_fields = dm.getattr("record_fields")?;
    m.add("_MAX_HOLDS", MAX_HOLDS)?;
    m.add("_AXES", PyTuple::new(py, ["x", "y", "z"])?)?;
    m.add_function(wrap_pyfunction!(anchor_candidates, m)?)?;
    m.add_function(wrap_pyfunction!(resolve_vertex_anchor, m)?)?;
    m.add_function(wrap_pyfunction!(chain_to, m)?)?;

    let anchor = ANCHOR_CLASS.get_or_try_init(py, || -> PyResult<Py<PyAny>> {
        let fields = ["vertex", "mesh", "chain", "element_key", "occurrence_path"];
        let ns = PyDict::new(py);
        ns.set_item("__slots__", PyTuple::new(py, fields)?)?;
        ns.set_item("__doc__", "A hold on one mesh vertex: the vertex, the mesh that registers it\n(to notice when it is gone) and the chain of groups whose transforms\nplace it in the world: empty for loose geometry, ``[group]`` for a\nclassic group, ``[component, child, ...]`` down a nested placement.")?;
        method(&ns, "__init__", wrap_pyfunction!(va_init, m)?)?;
        method(&ns, "matrix", wrap_pyfunction!(va_matrix, m)?)?;
        method(&ns, "alive", wrap_pyfunction!(va_alive, m)?)?;
        method(&ns, "last_position", wrap_pyfunction!(va_last_position, m)?)?;
        method(&ns, "position", wrap_pyfunction!(va_position, m)?)?;
        let cls = class(py, MODULE, "VertexAnchor", &[record.clone()], &ns)?;
        Ok(record_fields.call1(PyTuple::new(py, fields)?)?.call1((cls,))?.unbind())
    })?;
    m.add("VertexAnchor", anchor.bind(py))?;

    let dimension = DIMENSION_CLASS.get_or_try_init(py, || -> PyResult<Py<PyAny>> {
        let ns = PyDict::new(py);
        ns.set_item("__doc__", "See the module docstring. ``a`` and ``b`` read live through their\nanchors; assigning them sets the frozen position (what a rigid\ntransform of the whole model does) and keeps the anchor.")?;
        method(&ns, "__init__", wrap_pyfunction!(dim_init, m)?)?;
        method(&ns, "_hold", wrap_pyfunction!(dim_hold, m)?)?;
        method(&ns, "__deepcopy__", wrap_pyfunction!(dim_deepcopy, m)?)?;
        method(&ns, "_holds_of", wrap_pyfunction!(dim_holds_of, m)?)?;
        method(&ns, "_live", wrap_pyfunction!(dim_live, m)?)?;
        method(&ns, "_resolve_again", wrap_pyfunction!(dim_resolve_again, m)?)?;
        property(&ns, "a", wrap_pyfunction!(dim_get_a, m)?.into_any(), Some(wrap_pyfunction!(dim_set_a, m)?.into_any()))?;
        property(&ns, "b", wrap_pyfunction!(dim_get_b, m)?.into_any(), Some(wrap_pyfunction!(dim_set_b, m)?.into_any()))?;
        method(&ns, "bind", wrap_pyfunction!(dim_bind, m)?)?;
        method(&ns, "refresh_anchors", wrap_pyfunction!(dim_refresh_anchors, m)?)?;
        property(&ns, "anchored", wrap_pyfunction!(dim_anchored, m)?.into_any(), None)?;
        method(&ns, "anchor_points", wrap_pyfunction!(dim_anchor_points, m)?)?;
        method(&ns, "display_text", wrap_pyfunction!(dim_display_text, m)?)?;
        method(&ns, "value", wrap_pyfunction!(dim_value, m)?)?;
        method(&ns, "label", wrap_pyfunction!(dim_label, m)?)?;
        method(&ns, "perpendicular_offset", wrap_pyfunction!(dim_perpendicular_offset, m)?)?;
        property(&ns, "line_point_a", wrap_pyfunction!(dim_line_point_a, m)?.into_any(), None)?;
        property(&ns, "line_point_b", wrap_pyfunction!(dim_line_point_b, m)?.into_any(), None)?;
        method(&ns, "line_points", wrap_pyfunction!(dim_line_points, m)?)?;
        method(&ns, "midpoint", wrap_pyfunction!(dim_midpoint, m)?)?;
        method(&ns, "ends_riding", wrap_pyfunction!(dim_ends_riding, m)?)?;
        method(&ns, "carried", wrap_pyfunction!(dim_carried, m)?)?;
        // The offset placing the dimension line through `cursor`, parallel
        // to a–b: `offset_for_cursor(a, b, cursor)`.
        static_method(&ns, "offset_for_cursor", native(py)?.getattr("dimension_offset_for_cursor")?)?;
        // `(offset, axis)` for the cursor's position: pulled PAST both
        // endpoints along one axis's extension direction, the linear
        // dimension along that axis; square off the segment, anywhere else,
        // or past both in two directions at once, the aligned one. An axis
        // within 15° of the segment, or of square to it, is never offered.
        static_method(&ns, "placement_for_cursor", native(py)?.getattr("dimension_placement_for_cursor")?)?;
        let cls = class(py, MODULE, "Dimension", &[record.clone()], &ns)?;
        let fields = PyTuple::new(py, ["_a", "_b", "offset", "axis", "layer", "text", "anchor_a", "anchor_b", "_holds", "attachment_a", "attachment_b"])?;
        Ok(record_fields.call1(fields)?.call1((cls,))?.unbind())
    })?;
    m.add("Dimension", dimension.bind(py))?;
    // Names the Python module imported, for plugins that reach them.
    m.add("weakref", py.import("weakref")?)?;
    m.add("Record", record)?;
    m.add("record_fields", record_fields)?;
    m.add("_NATIVE", native(py)?)?;
    m.add("QMatrix4x4", native(py)?.getattr("Matrix4")?)?;
    m.add("QVector3D", native(py)?.getattr("Vec3")?)?;
    Ok(())
}
