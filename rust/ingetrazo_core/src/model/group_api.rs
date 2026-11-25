// SPDX-License-Identifier: GPL-3.0-or-later
//! `core.group`, served from here (`core/group.py` is gone). A Group is a
//! self-contained chunk of geometry with its own mesh.
//!
//! The scene's main mesh welds every coincident vertex (sticky geometry),
//! so a block drawn against a wall fuses with it. A Group isolates geometry
//! into its *own* mesh, so nothing welds across the group boundary, and it
//! moves and edits as a unit without dragging the rest of the model. It is
//! selected as a unit, and moved or exploded via the commands in
//! `ingetrazo_core.history`.
//!
//! What is here:
//! - the `Group` and `PlacementProxy` classes;
//! - uids and automatic names;
//! - `copy_group`;
//! - the local axes (`group_frame`, `frame_axes`, `carry_axes`);
//! - the measured boxes (`placement_points`, `oriented_bounds`).
//!
//! The walks are `placement_walks.rs`, the meshes `group_mesh.rs`, the
//! frame maths `group_frame.rs`.

use pyo3::intern;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyDict, PyList, PyString, PyTuple};

use super::module;
use super::py_class::{class, method, FastAttr};

type Obj<'py> = Bound<'py, PyAny>;

const MODULE: &str = "core.group";
/// Every attribute a group has, in one tuple (`link_clone_shell` copies
/// them). The `DOC_FIELDS` live in the document and the rest stay Python
/// slots.
const GROUP_FIELDS: [&str; 20] = [
    "mesh", "name", "layer", "ifc", "billboard", "xform", "children", "owner", "context", "text3d", "hidden", "uid",
    "material", "axes", "component", "exploded", "explode_offset", "ext", "locked", "source_uid",
];
/// Custom attributes: document values too, handed out as tracked
/// containers that write back when edited in place (`core.doc_model`).
const ATTR_FIELDS: [&str; 5] = ["ifc", "ext", "text3d", "exploded", "explode_offset"];
const DOC_FIELDS: [&str; 18] = [
    "name", "layer", "xform", "children", "hidden", "uid", "material", "axes", "component", "locked", "ifc", "ext",
    "text3d", "exploded", "explode_offset", "mesh", "billboard", "source_uid",
];
const PY_FIELDS: [&str; 2] = ["owner", "context"];

static GROUP_CLASS: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
static PROXY_CLASS: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
static MESH_CLASS: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn native(py: Python<'_>) -> PyResult<Obj<'_>> {
    module(py, "ingetrazo_core")
}

fn matrix<'py>(py: Python<'py>) -> PyResult<Obj<'py>> {
    native(py)?.getattr(intern!(py, "Matrix4"))
}

static VEC3: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

fn vec3_class(py: Python<'_>) -> PyResult<&Bound<'_, PyAny>> {
    Ok(VEC3.get_or_try_init(py, || -> PyResult<_> { Ok(native(py)?.getattr("Vec3")?.unbind()) })?.bind(py))
}

fn vec3<'py>(py: Python<'py>, x: impl IntoPyObject<'py>, y: impl IntoPyObject<'py>, z: impl IntoPyObject<'py>) -> PyResult<Obj<'py>> {
    vec3_class(py)?.call1((x, y, z))
}

fn attr_or_none<'py>(o: &Obj<'py>, name: &Bound<'py, PyString>) -> PyResult<Obj<'py>> {
    Ok(o.attr_opt(name)?.unwrap_or_else(|| none(o.py())))
}

pub fn group_class(py: Python<'_>) -> PyResult<Obj<'_>> {
    match GROUP_CLASS.get(py) {
        Some(c) => Ok(c.bind(py).clone()),
        None => py.import(MODULE)?.getattr("Group"),
    }
}

// --------------------------------------------------------- uids and names

/// Fresh uids come from a pool filled by one `os.urandom` call: one `uuid4`
/// (one urandom syscall) per group was 0.26 s of opening the ride, whose
/// 104k groups each take one. Emptied in a forked child, so two processes
/// never hand out the same pooled bytes.
static UID_POOL: PyOnceLock<Py<PyList>> = PyOnceLock::new();
const UID_BATCH: usize = 4096;

/// The pool, the module's `_UID_POOL` list.
fn uid_pool(py: Python<'_>) -> PyResult<&Bound<'_, PyList>> {
    Ok(UID_POOL.get_or_try_init(py, || -> PyResult<_> { Ok(PyList::empty(py).unbind()) })?.bind(py))
}

/// Refill the pool with `UID_BATCH` uids of the very form
/// `uuid4().hex[:16]` had: 16 lowercase hex digits, 64 bits of which the
/// 13th digit is the version nibble «4» (a uid read by an older build is
/// indistinguishable from one it made).
fn refill_uids(py: Python<'_>) -> PyResult<()> {
    let raw: Vec<u8> = module(py, "os")?.call_method1(intern!(py, "urandom"), (8 * UID_BATCH,))?.extract()?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let pool = uid_pool(py)?;
    for c in raw.chunks(8) {
        let mut s = String::with_capacity(16);
        for b in c {
            s.push(HEX[(b >> 4) as usize] as char);
            s.push(HEX[(b & 15) as usize] as char);
        }
        s.replace_range(12..13, "4");
        pool.append(s)?;
    }
    Ok(())
}

#[pyfunction]
#[pyo3(name = "_drop_uid_pool")]
fn drop_uid_pool(py: Python<'_>) -> PyResult<()> {
    uid_pool(py)?.call_method0("clear")?;
    Ok(())
}

/// A fresh group identity: random, so two documents merged (a `.igz`
/// imported as a component) never collide.
#[pyfunction]
pub fn new_uid(py: Python<'_>) -> PyResult<Obj<'_>> {
    // list.pop under the GIL: two threads never get the same one.
    loop {
        let pool = uid_pool(py)?;
        if !pool.is_empty() {
            return pool.call_method0(intern!(py, "pop"));
        }
        refill_uids(py)?;
    }
}

/// The largest N among the automatic "Group N" entries in `names` (read in
/// Rust: a Python regex per name was 67 ms of opening 50 000 placements).
fn highest_group_number<'py>(names: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = names.py();
    let n = native(py)?.call_method1(intern!(py, "highest_group_number"), (names,))?;
    py.import("builtins")?.getattr("int")?.call1((n,))
}

fn count_from<'py>(py: Python<'py>, start: &Obj<'py>) -> PyResult<Obj<'py>> {
    module(py, "itertools")?.call_method1(intern!(py, "count"), (start,))
}

fn next<'py>(it: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = it.py();
    py.import("builtins")?.getattr(intern!(py, "next"))?.call1((it,))
}

fn max<'py>(a: &Obj<'py>, b: &Obj<'py>) -> PyResult<Obj<'py>> {
    Ok(if b.gt(a)? { b.clone() } else { a.clone() })
}

/// Move the automatic "Group N" counter past every N in `names`: a
/// document that comes back with its names must not hand a new group a
/// name it already uses.
#[pyfunction]
fn reserve_group_names<'py>(names: &Obj<'py>) -> PyResult<()> {
    let py = names.py();
    let m = module(py, MODULE)?;
    let top = highest_group_number(names)?;
    let nxt = next(&m.getattr(intern!(py, "_counter"))?)?;
    m.setattr(intern!(py, "_counter"), count_from(py, &max(&nxt, &top.add(1)?)?)?)
}

/// Number auto-named groups from a document's own `names` while it loads,
/// not from the session's counter.
///
/// A file can carry groups with no name (everything saved before names
/// were written, some imports). Numbering those from the session counter
/// made the names a document saves back depend on whatever was opened
/// earlier in the session (issue #943). Inside the scope the counter
/// continues the document's own highest "Group N", in document order. On
/// exit, the session counter moves past everything the document used, so
/// a group created by hand afterwards never reuses one of its numbers
/// either.
#[pyclass(module = "core.group", name = "document_name_scope")]
struct NameScope {
    local: Py<PyAny>,
    token: Option<Py<PyAny>>,
}

#[pymethods]
impl NameScope {
    #[new]
    fn new(names: &Obj<'_>) -> PyResult<Self> {
        let py = names.py();
        let local = count_from(py, &highest_group_number(names)?.add(1)?)?;
        Ok(Self { local: local.unbind(), token: None })
    }

    fn __enter__(&mut self, py: Python<'_>) -> PyResult<()> {
        let var = module(py, MODULE)?.getattr(intern!(py, "_document_counter"))?;
        self.token = Some(var.call_method1(intern!(py, "set"), (self.local.bind(py),))?.unbind());
        Ok(())
    }

    #[pyo3(signature = (*_exc))]
    fn __exit__(&mut self, py: Python<'_>, _exc: &Bound<'_, PyTuple>) -> PyResult<bool> {
        let m = module(py, MODULE)?;
        let first_unused = next(self.local.bind(py))?;
        if let Some(token) = self.token.take() {
            m.getattr(intern!(py, "_document_counter"))?.call_method1(intern!(py, "reset"), (token,))?;
        }
        let session_next = next(&m.getattr(intern!(py, "_counter"))?)?;
        m.setattr(intern!(py, "_counter"), count_from(py, &max(&first_unused, &session_next)?)?)?;
        Ok(false)
    }
}

// ------------------------------------------------------------- the classes

fn init_fields<'py>(me: &Obj<'py>, mesh: Option<&Obj<'py>>, name: Option<&Obj<'py>>, uid: Option<&Obj<'py>>) -> PyResult<()> {
    let py = me.py();
    // Rust document: its fields start at these defaults already (no tag,
    // shown, unlocked, unpainted, no matrix, no children, a component), so
    // only the name and the uid are written.
    let mesh = match mesh.filter(|m| !m.is_none()) {
        Some(m) => m.clone(),
        None => MESH_CLASS.get_or_try_init(py, || -> PyResult<_> { Ok(py.import("ingetrazo_core.mesh")?.getattr("Mesh")?.unbind()) })?.bind(py).call0()?,
    };
    me.setattr(intern!(py, "mesh"), mesh)?;
    let name = match name.filter(|n| n.is_truthy().unwrap_or(false)) {
        Some(n) => n.clone(),
        None => {
            let m = module(py, MODULE)?;
            let local = m.getattr(intern!(py, "_document_counter"))?.call_method0(intern!(py, "get"))?;
            let counter = if local.is_truthy()? { local } else { m.getattr(intern!(py, "_counter"))? };
            PyString::new(py, &format!("Group {}", next(&counter)?.str()?)).into_any()
        }
    };
    me.setattr(intern!(py, "name"), name)?;
    // Stable identity that survives save/load: what a scene names when it
    // remembers which objects it hides. Fresh per object; a copy gets its
    // own (see `copy_group`). `uid`: a stand-in that carries another
    // group's identity (the viewport's placement proxies).
    let uid = match uid.filter(|u| u.is_truthy().unwrap_or(false)) {
        Some(u) => u.clone(),
        // Through the module, as the Python looked it up: scripts and tests
        // that hand out their own uids replace `core.group.new_uid`.
        None => module(py, MODULE)?.getattr(intern!(py, "new_uid"))?.call0()?,
    };
    me.setattr(intern!(py, "uid"), uid)?;
    // Set only on the throwaway placement proxies the viewport builds for
    // nested children: the top-level object a click must select.
    me.setattr(intern!(py, "owner"), py.None())?;
    // The group-edit context this entry belongs to, while the viewport is
    // INSIDE a group. `source_uid` is set only on the parts of a linked
    // file, never here; read it with `getattr`.
    me.setattr(intern!(py, "context"), py.None())
}

fn fast_group_init<'py>(me: &Obj<'py>, args: &[Obj<'py>], kw: Option<&Bound<'py, PyTuple>>) -> PyResult<Obj<'py>> {
    let a = super::py_class::bind_args(args, kw, &["mesh", "name", "uid"])?;
    init_fields(me, a[0], a[1], a[2])?;
    Ok(none(me.py()))
}
crate::fast!(kw group_init_entry, fast_group_init);

fn fast_proxy_init<'py>(me: &Obj<'py>, args: &[Obj<'py>], kw: Option<&Bound<'py, PyTuple>>) -> PyResult<Obj<'py>> {
    let a = super::py_class::bind_args(args, kw, &["mesh", "name", "uid"])?;
    proxy_init(me, a[0], a[1], a[2])?;
    Ok(none(me.py()))
}
crate::fast!(kw proxy_init_entry, fast_proxy_init);

fn fast_is_component<'py>(me: &Obj<'py>, _args: &[Obj<'py>]) -> PyResult<Obj<'py>> {
    Ok(pyo3::types::PyBool::new(me.py(), group_is_component(me)?).to_owned().into_any())
}
crate::fast!(is_component_entry, fast_is_component);

fn fast_is_instance<'py>(me: &Obj<'py>, _args: &[Obj<'py>]) -> PyResult<Obj<'py>> {
    Ok(pyo3::types::PyBool::new(me.py(), group_is_instance(me)?).to_owned().into_any())
}
crate::fast!(is_instance_entry, fast_is_instance);

/// A component instance, as the user knows it: not merely a group that
/// carries a matrix.
fn group_is_component(me: &Obj<'_>) -> PyResult<bool> {
    let py = me.py();
    Ok(!me.getattr(intern!(py, "xform"))?.is_none() && me.getattr(intern!(py, "component"))?.is_truthy()?)
}

fn group_is_instance(me: &Obj<'_>) -> PyResult<bool> {
    Ok(!me.getattr(intern!(me.py(), "xform"))?.is_none())
}

fn doc_key_set(o: &Obj<'_>) -> PyResult<bool> {
    match o.attr_opt(intern!(o.py(), "_doc_key"))? {
        Some(k) => k.is_truthy(),
        None => Ok(false),
    }
}

/// Take `children` as nested placements, guaranteeing the invariant every
/// consumer relies on: **a group that owns placements is always an
/// instance**.
///
/// Move, Rotate and Scale compose into `xform` for an instance and walk the
/// vertices otherwise, and walking the vertices would move the group's own
/// mesh while leaving its children behind. An identity transform costs
/// nothing and closes that road.
#[pyfunction]
fn group_adopt<'py>(me: &Obj<'py>, children: &Obj<'py>) -> PyResult<()> {
    let py = me.py();
    let kids: Vec<Obj<'py>> = children.try_iter()?.collect::<PyResult<_>>()?;
    if doc_key_set(me)? {
        // One parent per placement: adopting takes a child out of the list
        // that held it (a loaded file's top level, a subset of a tree),
        // unless that list is this group's own. (A viewport proxy holds no
        // document list: it takes nothing away.)
        for c in &kids {
            if doc_key_set(c)? {
                let parent = c.getattr(intern!(py, "_parent"))?;
                if !parent.is_none() && !parent.is(me) {
                    c.call_method0(intern!(py, "_detach"))?;
                }
            }
        }
    }
    me.setattr(intern!(py, "children"), PyList::new(py, kids)?)?;
    if me.getattr(intern!(py, "children"))?.is_truthy()? && me.getattr(intern!(py, "xform"))?.is_none() {
        me.setattr(intern!(py, "xform"), matrix(py)?.call0()?)?;
    }
    Ok(())
}

/// Bake this instance into its OWN world-space mesh ('make unique'):
/// sibling instances keep the shared prototype untouched. Nested placements
/// are baked in too and then dropped: `world_mesh` already folded them, so
/// keeping them would draw and export the child geometry a second time.
#[pyfunction]
fn group_materialize<'py>(me: &Obj<'py>) -> PyResult<()> {
    let py = me.py();
    if me.getattr(intern!(py, "xform"))?.is_none() && !me.getattr(intern!(py, "children"))?.is_truthy()? {
        return Ok(());
    }
    let frame = group_frame(me)?;
    me.setattr(intern!(py, "mesh"), native(py)?.call_method1(intern!(py, "group_world_mesh"), (me,))?)?;
    me.setattr(intern!(py, "xform"), py.None())?;
    me.setattr(intern!(py, "axes"), frame)?; // baked, but it still faces its way
    me.setattr(intern!(py, "children"), PyList::empty(py))
}

/// Make Unique: this instance stops sharing with its siblings. A COMPONENT
/// stays a component, a definition of its own with this one copy, placed
/// where it was (issue #90, #620); a copied GROUP bakes into a classic group
/// or keeps its tree privately (`group_geometry.rs: unique_placement`).
#[pyfunction]
fn group_make_unique<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    native(py)?.call_method1(intern!(py, "unique_placement"), (me,))
}

#[pyfunction]
fn group_repr(me: &Obj<'_>) -> PyResult<String> {
    let py = me.py();
    let kind = if me.getattr(intern!(py, "xform"))?.is_none() { "" } else { " instance" };
    let mesh = me.getattr(intern!(py, "mesh"))?;
    Ok(format!(
        "Group({}{kind}: {} faces, {} edges)",
        me.getattr(intern!(py, "name"))?.repr()?,
        mesh.getattr(intern!(py, "faces"))?.len()?,
        mesh.getattr(intern!(py, "edges"))?.len()?
    ))
}

fn fields(py: Python<'_>) -> PyResult<Bound<'_, PyTuple>> {
    PyTuple::new(py, GROUP_FIELDS)
}

/// copy, deepcopy and pickle: a new handle, then every field.
#[pyfunction]
fn group_reduce<'py>(me: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    let py = me.py();
    let state = super::doc_model_api::handle_state(me, fields(py)?.as_any())?;
    let new = py.import("core.doc_model")?.getattr(intern!(py, "new_handle"))?;
    PyTuple::new(py, [new, PyTuple::new(py, [me.get_type()])?.into_any(), state.into_any()])
}

#[pyfunction]
#[pyo3(signature = (cls, *_args, **_kwargs))]
fn proxy_new<'py>(cls: &Obj<'py>, _args: &Bound<'py, PyTuple>, _kwargs: Option<&Bound<'py, PyDict>>) -> PyResult<Obj<'py>> {
    let py = cls.py();
    let kw = PyDict::new(py);
    kw.set_item("_detached", true)?;
    native(py)?.getattr(intern!(py, "Placement"))?.getattr(intern!(py, "__new__"))?.call((cls,), Some(&kw))
}

fn proxy_init<'py>(me: &Obj<'py>, mesh: Option<&Obj<'py>>, name: Option<&Obj<'py>>, uid: Option<&Obj<'py>>) -> PyResult<()> {
    let py = me.py();
    init_fields(me, mesh, name, uid)?;
    let nothing = none(py);
    let f = pyo3::types::PyBool::new(py, false).to_owned().into_any();
    me.setattr(intern!(py, "billboard"), &f)?;
    for n in ["ifc", "ext", "text3d", "exploded", "explode_offset", "layer"] {
        me.setattr(n, &nothing)?;
    }
    me.setattr(intern!(py, "hidden"), &f)?;
    me.setattr(intern!(py, "locked"), &f)?;
    me.setattr(intern!(py, "material"), &nothing)?;
    me.setattr(intern!(py, "xform"), &nothing)?;
    me.setattr(intern!(py, "children"), PyList::empty(py))?;
    me.setattr(intern!(py, "axes"), &nothing)?;
    me.setattr(intern!(py, "component"), true)
}

// ------------------------------------------------- local axes (issue #44)

/// A classic group's geometry was just transformed by `m` in world space:
/// its axes go with it (the analogue of composing `xform` for an instance).
/// Instances are left alone: their `xform` already moved.
#[pyfunction]
fn carry_axes<'py>(group: &Obj<'py>, m: &Obj<'py>) -> PyResult<()> {
    let py = group.py();
    if group.is_none() || !attr_or_none(group, intern!(py, "xform"))?.is_none() {
        return Ok(());
    }
    let axes = group.getattr(intern!(py, "axes"))?;
    let base = if axes.is_none() { matrix(py)?.call0()? } else { axes };
    group.setattr(intern!(py, "axes"), m.mul(base)?)
}

/// The world matrix of `group`'s own axes, or None for the world axes.
/// `axes` lives in the space of the group's mesh, so the frame is
/// `xform · axes`: an instance's placement, a classic group's stored axes,
/// or both when a classic group became an instance (a container's child
/// once the container is opened).
#[pyfunction]
pub fn group_frame<'py>(group: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = group.py();
    if group.is_none() {
        return Ok(none(py));
    }
    let xform = attr_or_none(group, intern!(py, "xform"))?;
    let axes = attr_or_none(group, intern!(py, "axes"))?;
    if xform.is_none() {
        return Ok(axes);
    }
    if axes.is_none() { Ok(xform) } else { xform.mul(axes) }
}

/// `(origin, x, y, z)` of a frame matrix, the three axes made unit and
/// square to each other (a scaled or skewed placement still draws
/// orthogonal axes; red first, green squared to it, blue from both, and a
/// mirrored placement keeps its handedness). `m=None` is the world.
#[pyfunction]
fn frame_axes<'py>(m: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    let py = m.py();
    let v3 = vec3_class(py)?;
    if m.is_none() {
        return PyTuple::new(py, [vec3(py, 0, 0, 0)?, vec3(py, 1, 0, 0)?, vec3(py, 0, 1, 0)?, vec3(py, 0, 0, 1)?]);
    }
    let o = m.call_method1(intern!(py, "map"), (vec3(py, 0, 0, 0)?,))?;
    let mut x = m.call_method1(intern!(py, "mapVector"), (vec3(py, 1, 0, 0)?,))?;
    let y = m.call_method1(intern!(py, "mapVector"), (vec3(py, 0, 1, 0)?,))?;
    let z = m.call_method1(intern!(py, "mapVector"), (vec3(py, 0, 0, 1)?,))?;
    let len = |v: &Obj<'py>| -> PyResult<f64> { v.call_method0(intern!(py, "length"))?.extract() };
    if len(&x)? < 1e-12 {
        x = vec3(py, 1, 0, 0)?;
    }
    let x = x.call_method0(intern!(py, "normalized"))?;
    let mut y = y.sub(x.mul(v3.call_method1(intern!(py, "dotProduct"), (&y, &x))?)?)?;
    if len(&y)? < 1e-12 {
        y = v3.call_method1(intern!(py, "crossProduct"), (&z, &x))?;
    }
    let y = y.call_method0(intern!(py, "normalized"))?;
    let mut zz = v3.call_method1(intern!(py, "crossProduct"), (&x, &y))?.call_method0(intern!(py, "normalized"))?;
    if v3.call_method1(intern!(py, "dotProduct"), (&zz, &z))?.lt(0)? {
        zz = zz.neg()?; // a mirrored placement
    }
    PyTuple::new(py, [o, x, y, zz])
}

/// A face-me billboard group: one textured quad, `height` metres tall,
/// anchored at `position` (origin by default). The viewport rotates it
/// around that anchor to face the camera.
#[pyfunction]
#[pyo3(signature = (image_path, height, name, aspect, position=None))]
fn make_billboard_group<'py>(image_path: &Obj<'py>, height: &Obj<'py>, name: &Obj<'py>, aspect: &Obj<'py>, position: Option<&Obj<'py>>) -> PyResult<Obj<'py>> {
    let py = image_path.py();
    let mesh = py.import("ingetrazo_core.mesh")?.getattr("Mesh")?.call0()?;
    let w = height.mul(aspect)?;
    let p = match position.filter(|p| !p.is_none()) {
        Some(p) => p.clone(),
        None => vec3(py, 0, 0, 0)?,
    };
    let half = w.div(2)?;
    let neg_half = w.neg()?.div(2)?;
    let quad = PyList::new(py, [
        p.add(vec3(py, &neg_half, 0, 0)?)?,
        p.add(vec3(py, &half, 0, 0)?)?,
        p.add(vec3(py, &half, 0, height)?)?,
        p.add(vec3(py, &neg_half, 0, height)?)?,
    ])?;
    let face = mesh.call_method1(intern!(py, "add_face"), (quad,))?;
    let tex = PyDict::new(py);
    tex.set_item("path", image_path.str()?)?;
    tex.set_item("sw", &w)?;
    tex.set_item("sh", height)?;
    face.getattr(intern!(py, "attrs"))?.set_item("texture", tex)?;
    let kw = PyDict::new(py);
    kw.set_item("name", name)?;
    let g = group_class(py)?.call((mesh,), Some(&kw))?;
    g.setattr(intern!(py, "billboard"), true)?;
    Ok(g)
}

// ----------------------------------------------------------------- copies

/// Whether `group` is a placed linked file (`core.links`).
#[pyfunction]
#[pyo3(name = "_is_link")]
fn is_link(group: &Obj<'_>) -> PyResult<bool> {
    let py = group.py();
    let ext = attr_or_none(group, intern!(py, "ext"))?;
    if !ext.is_instance_of::<PyDict>() {
        return Ok(false);
    }
    let link = ext.call_method1(intern!(py, "get"), ("link",))?;
    if !link.is_instance_of::<PyDict>() {
        return Ok(false);
    }
    link.call_method1(intern!(py, "get"), ("path",))?.is_truthy()
}

fn without_guid<'py>(d: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(d.py());
    for item in d.call_method0(intern!(d.py(), "items"))?.try_iter()? {
        let (k, v): (Obj<'py>, Obj<'py>) = item?.extract()?;
        if !k.eq("guid")? {
            out.set_item(k, v)?;
        }
    }
    Ok(out)
}

fn dict_copy_or_none<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    if v.is_truthy()? { py.import("builtins")?.getattr("dict")?.call1((v,)) } else { Ok(none(py)) }
}

fn openings_of<'py>(ifc: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let py = ifc.py();
    let o = ifc.call_method1(intern!(py, "get"), ("openings", PyList::empty(py)))?;
    if o.is_truthy()? { o.try_iter()?.collect() } else { Ok(Vec::new()) }
}

/// A pastable duplicate of `group`, optionally translated by `delta`.
///
/// A component instance stays an instance: the duplicate SHARES the
/// prototype mesh and only gets its own transform (copying an instance adds
/// a sibling, O(1)). A classic group gets a deep mesh copy.
///
/// Everything inside a COMPONENT belongs to its definition, so the groups
/// nested in it are shared too, at every depth (issue #97: editing a group
/// inside one copy left the other copies as they were). A classic child
/// becomes an identity placement first (same place in the world), so the
/// original and the copy hold the same mesh, and an edit inside either
/// shares back to both.
#[pyfunction]
#[pyo3(signature = (group, delta=None, _in_definition=false, _ifc_copy_context=None, _project_occurrence=false))]
pub fn copy_group<'py>(group: &Obj<'py>, delta: Option<&Obj<'py>>, _in_definition: bool, _ifc_copy_context: Option<&Obj<'py>>, _project_occurrence: bool) -> PyResult<Obj<'py>> {
    let py = group.py();
    let rs = native(py)?;
    let mat = matrix(py)?;
    let copy_root = _ifc_copy_context.is_none_or(|c| c.is_none());
    let ctx = match _ifc_copy_context.filter(|c| !c.is_none()) {
        Some(c) => c.clone(),
        None => {
            let c = PyDict::new(py);
            c.set_item("groups", PyDict::new(py))?;
            c.set_item("openings", PyDict::new(py))?;
            c.set_item("copies", PyList::empty(py))?;
            c.into_any()
        }
    };
    let xform = group.getattr(intern!(py, "xform"))?;
    let component = group.attr_opt(intern!(py, "component"))?.map(|c| c.is_truthy()).transpose()?.unwrap_or(true);
    let share = _in_definition || (!xform.is_none() && component);
    // A linked file's placement: its parts are the source's, read-only, and
    // the copy is a live link of the same file (`link_copy_parts`).
    let linked = is_link(group)?;
    let kids = group.getattr(intern!(py, "children"))?;
    let kids: Vec<Obj<'py>> = if kids.is_truthy()? { kids.try_iter()?.collect::<PyResult<_>>()? } else { Vec::new() };
    if share && !linked && !_project_occurrence {
        for child in &kids {
            if child.getattr(intern!(py, "xform"))?.is_none() {
                child.setattr(intern!(py, "xform"), mat.call0()?)?;
                child.setattr(intern!(py, "component"), false)?; // it was a group, it stays one
            }
        }
    }
    let t = mat.call0()?;
    if let Some(d) = delta.filter(|d| !d.is_none()) {
        t.call_method1(intern!(py, "translate"), (rs.getattr(intern!(py, "Vec3"))?.call1((d,))?,))?;
    }
    let cls = group_class(py)?;
    let name = group.getattr(intern!(py, "name"))?;
    let kw = PyDict::new(py);
    kw.set_item("name", &name)?;
    let gmesh = group.getattr(intern!(py, "mesh"))?;
    let axes = attr_or_none(group, intern!(py, "axes"))?;
    let g = if !xform.is_none() || _project_occurrence {
        let g = cls.call((&gmesh,), Some(&kw))?;
        g.setattr(intern!(py, "xform"), if xform.is_none() { none(py) } else { t.mul(&xform)? })?;
        // The local axes live in the mesh's space, which the copy shares:
        // they come along as they are. Dropping them turned a pasted rotated
        // group's axes back to the world's (issue #78, @pacaeiro: Copy makes
        // a classic group an instance, and Paste copies that).
        if !axes.is_none() {
            g.setattr(intern!(py, "axes"), mat.call1((&axes,))?)?;
        }
        g
    } else {
        let moved = rs.call_method1(intern!(py, "group_transformed_mesh"), (&gmesh, &t))?;
        let g = cls.call((moved,), Some(&kw))?;
        if !axes.is_none() {
            g.setattr(intern!(py, "axes"), t.mul(&axes)?)?;
        }
        g
    };
    let new_mesh = g.getattr(intern!(py, "mesh"))?;
    if !new_mesh.is(&gmesh) {
        // A deep geometry copy owns its face tags; shared definitions instead
        // scope their tags to the placement uid when exported.
        for face in new_mesh.getattr(intern!(py, "faces"))?.try_iter()? {
            let attrs = face?.getattr(intern!(py, "attrs"))?;
            let ifc = attrs.call_method1(intern!(py, "get"), ("ifc",))?;
            if ifc.is_truthy()? {
                let ifc = attrs.get_item("ifc")?;
                attrs.set_item("ifc", without_guid(&ifc)?)?;
            }
        }
    }
    g.setattr(intern!(py, "layer"), group.getattr(intern!(py, "layer"))?)?;
    let deepcopy = module(py, "copy")?.getattr(intern!(py, "deepcopy"))?;
    let ifc = group.getattr(intern!(py, "ifc"))?;
    g.setattr(intern!(py, "ifc"), if ifc.is_truthy()? { deepcopy.call1((without_guid(&ifc)?,))? } else { none(py) })?;
    ctx.get_item("groups")?.set_item(group.getattr(intern!(py, "uid"))?, g.getattr(intern!(py, "uid"))?)?;
    ctx.get_item("copies")?.call_method1(intern!(py, "append"), (&g,))?;
    let gifc = g.getattr(intern!(py, "ifc"))?;
    if gifc.is_truthy()? {
        let ensure_guid = py.import("core.ifc_identity")?.getattr("ensure_guid")?;
        for opening in openings_of(&gifc)? {
            if opening.is_instance_of::<PyDict>() {
                let previous = opening.call_method1(intern!(py, "pop"), ("guid", py.None()))?;
                if previous.is_truthy()? {
                    ctx.get_item("openings")?.set_item(previous, ensure_guid.call1((&opening,))?)?;
                }
            }
        }
    }
    g.setattr(intern!(py, "billboard"), group.getattr(intern!(py, "billboard"))?)?;
    g.setattr(intern!(py, "text3d"), dict_copy_or_none(&group.getattr(intern!(py, "text3d"))?)?)?;
    let ext = attr_or_none(group, intern!(py, "ext"))?;
    g.setattr(intern!(py, "ext"), if ext.is_truthy()? { deepcopy.call1((ext,))? } else { none(py) })?;
    g.setattr(intern!(py, "hidden"), group.getattr(intern!(py, "hidden"))?)?;
    g.setattr(intern!(py, "locked"), group.attr_opt(intern!(py, "locked"))?.unwrap_or_else(|| pyo3::types::PyBool::new(py, false).to_owned().into_any()))?;
    g.setattr(intern!(py, "material"), dict_copy_or_none(&attr_or_none(group, intern!(py, "material"))?)?)?;
    g.setattr(intern!(py, "component"), group.attr_opt(intern!(py, "component"))?.unwrap_or_else(|| pyo3::types::PyBool::new(py, true).to_owned().into_any()))?;
    g.setattr(intern!(py, "exploded"), dict_copy_or_none(&group.getattr(intern!(py, "exploded"))?)?)?;
    g.setattr(intern!(py, "explode_offset"), group.getattr(intern!(py, "explode_offset"))?)?;
    // Nested placements ride along untranslated: `delta` already moved the
    // parent, and a child's transform is relative to it.
    if linked {
        g.setattr(intern!(py, "children"), rs.call_method1(intern!(py, "link_copy_parts"), (group, &g))?)?;
    } else {
        let copies = PyList::empty(py);
        for c in &kids {
            copies.append(copy_group(c, None, share, Some(&ctx), _project_occurrence)?)?;
        }
        g.setattr(intern!(py, "children"), copies)?;
    }
    if copy_root {
        // Resolve after the complete tree is copied: a filling can precede
        // its host in traversal order. External references stay external.
        let groups = ctx.get_item("groups")?;
        let openings = ctx.get_item("openings")?;
        for placed in ctx.get_item("copies")?.try_iter()? {
            let placed = placed?;
            let pifc = placed.getattr(intern!(py, "ifc"))?;
            if !pifc.is_truthy()? {
                continue;
            }
            for opening in openings_of(&pifc)? {
                if opening.is_instance_of::<PyDict>() {
                    let host = opening.call_method1(intern!(py, "get"), ("group_uid",))?;
                    if groups.contains(&host)? {
                        opening.set_item("group_uid", groups.get_item(&host)?)?;
                    }
                }
            }
            let pifc = placed.getattr(intern!(py, "ifc"))?;
            let filling = pifc.call_method1(intern!(py, "get"), ("fills",))?;
            if filling.is_instance_of::<PyString>() && openings.contains(&filling)? {
                pifc.set_item("fills", openings.get_item(&filling)?)?;
            }
        }
    }
    Ok(g)
}

#[pyfunction]
#[pyo3(name = "_copy_definition_mesh")]
fn copy_definition_mesh<'py>(mesh: &Obj<'py>) -> PyResult<Obj<'py>> {
    // StoreMesh.__deepcopy__ forks the store chunks under fresh keys.
    let py = mesh.py();
    module(py, "copy")?.call_method1(intern!(py, "deepcopy"), (mesh,))
}

// ------------------------------------------------------------------ boxes

fn np<'py>(py: Python<'py>) -> PyResult<Obj<'py>> {
    module(py, "numpy")
}

fn f64_array<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let kw = PyDict::new(py);
    kw.set_item("dtype", np(py)?.getattr("float64")?)?;
    np(py)?.call_method("asarray", (v,), Some(&kw))
}

fn xy<'py>(points: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = points.py();
    let sl = PyTuple::new(py, [pyo3::types::PySlice::full(py), pyo3::types::PySlice::new(py, 0, 2, 1)])?;
    np(py)?.call_method1("ascontiguousarray", (f64_array(points)?.get_item(sl)?,))
}

/// Convex hull of 2D points (monotone chain), counter-clockwise from the
/// lowest x, as an `(H, 2)` array (`group_frame.rs`).
#[pyfunction]
#[pyo3(name = "_hull_2d")]
fn hull_2d<'py>(pts: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = pts.py();
    native(py)?.call_method1(intern!(py, "group_hull_2d"), (xy(pts)?,))
}

/// An orthonormal frame for a group, derived from its vertices: the yaw
/// about Z whose footprint is tightest, with Z kept upright.
///
/// The usual convention gives every group its own axes and draws the
/// selection box in them, so the box hugs the object. Nothing stores those
/// axes here yet (an imported group is baked to world coordinates, a
/// classic group never had a frame), so they are derived, and derived
/// *exactly*. The minimum-area rectangle around a point set always has a
/// side flush with an edge of its convex hull, so trying each hull edge
/// finds the true optimum rather than guessing at it. The barbecue in
/// piscina.igz sits at 23.5 degrees and needs half the footprint the world
/// axes give it.
///
/// Keeping Z upright is the domain's assumption, not a shortcut: buildings,
/// walls and furniture stand up, and a box tilted off vertical would read
/// as an error even where it wrapped tighter.
#[pyfunction]
fn frame_from_points<'py>(positions: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    let py = positions.py();
    let world = || -> PyResult<Bound<'py, PyTuple>> { PyTuple::new(py, [vec3(py, 1.0, 0.0, 0.0)?, vec3(py, 0.0, 1.0, 0.0)?, vec3(py, 0.0, 0.0, 1.0)?]) };
    if positions.is_none() || positions.len()? < 3 {
        return world();
    }
    let u = native(py)?.call_method1(intern!(py, "group_frame_axis"), (xy(positions)?,))?;
    if u.is_none() {
        return world();
    }
    let (a, b): (f64, f64) = u.extract()?;
    PyTuple::new(py, [vec3(py, a, b, 0.0)?, vec3(py, -b, a, 0.0)?, vec3(py, 0.0, 0.0, 1.0)?])
}

/// The vertex positions of `mesh` as an `(N, 3)` float64 array, or None
/// when it has none. A mesh still deferred (a lazy mesh of a big document)
/// is NOT built for it: its points are the welded positions the build would
/// make, in the same order. A box drawn over the 590 groups of a 5-million-
/// face model built every one of them, past 12 GB.
#[pyfunction]
#[pyo3(name = "_local_points")]
fn local_points<'py>(mesh: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let np = np(py)?;
    let kw = PyDict::new(py);
    kw.set_item("dtype", np.getattr("float64")?)?;
    let array = |pos: &Obj<'py>| -> PyResult<Obj<'py>> { np.call_method("array", (pos,), Some(&kw)) };
    let mesh_mod = py.import("ingetrazo_core.mesh")?;
    let t = mesh.get_type();
    if t.is(&mesh_mod.getattr("DeferredMesh")?) {
        let d = mesh.getattr(intern!(py, "__dict__"))?;
        let hit = d.call_method1(intern!(py, "get"), ("_deferred_plan",))?;
        let pos = if !hit.is_none() {
            hit.getattr("pos")?
        } else {
            let s = d.get_item("_deferred_source")?;
            let seq = s.call_method1(intern!(py, "get"), ("sequential", false))?;
            native(py)?.call_method1(intern!(py, "deferred_weld"), (s.get_item("pos")?, seq))?.get_item(0)?
        };
        return if pos.len()? > 0 { array(&pos) } else { Ok(none(py)) };
    }
    if t.is(&py.import("ingetrazo_core.store_mesh")?.getattr("StoreMesh")?) {
        // The store's dense positions, in `mesh.vertices` order: walking the
        // views made a cached Vec3 per vertex, and selecting a linked file
        // measures every mesh of it (the selection box).
        let pos = mesh.getattr(intern!(py, "__dict__"))?.get_item("_store")?.call_method0(intern!(py, "arrays"))?.get_item("pos")?;
        return if pos.len()? > 0 { array(&pos)?.call_method1("reshape", (-1, 3)) } else { Ok(none(py)) };
    }
    let verts = mesh.getattr(intern!(py, "vertices"))?;
    if !verts.is_truthy()? {
        return Ok(none(py));
    }
    let rows = PyList::empty(py);
    for v in verts.try_iter()? {
        rows.append(v?.call_method0(intern!(py, "position_tuple"))?)?;
    }
    array(&rows.into_any())
}

/// Every vertex position of `group` and its nested placements, in WORLD
/// space, as an `(N, 3)` float64 array. `cache` (`{id(mesh): array}`) lets a
/// caller keep each prototype's local array across calls: filled here,
/// validated by the caller (the viewport keys it on the chunk's revision).
/// Rebuilding the plaza's arrays cost 73 ms per box.
///
/// A bounding box only ever needed the POINTS, but the only way to get
/// them used to be `world_mesh`, which welds a merged copy of the whole
/// component to answer a question about eight corners. On the hedge that
/// was 23 s per selection once its geometry moved into nested placements.
/// Here each prototype's array is built ONCE and every placement is one
/// matrix multiply over it.
#[pyfunction]
#[pyo3(signature = (group, cache=None))]
fn placement_points<'py>(group: &Obj<'py>, cache: Option<&Obj<'py>>) -> PyResult<Obj<'py>> {
    let py = group.py();
    let np = np(py)?;
    let rs = native(py)?;
    let cache = match cache.filter(|c| !c.is_none()) {
        Some(c) => c.clone(),
        None => PyDict::new(py).into_any(),
    };
    let kw = PyDict::new(py);
    kw.set_item("dtype", np.getattr("float64")?)?;
    let out = PyList::empty(py);
    for item in rs.call_method1(intern!(py, "group_model_placements"), (group,))?.try_iter()? {
        let item = item?;
        let (g, m) = (item.get_item(0)?, item.get_item(1)?);
        let mesh = g.getattr(intern!(py, "mesh"))?;
        let key = mesh.as_ptr() as usize;
        let mut arr = cache.call_method1(intern!(py, "get"), (key,))?;
        if arr.is_none() {
            arr = local_points(&mesh)?;
            if arr.is_none() {
                continue;
            }
            cache.set_item(key, &arr)?;
        }
        if !m.is_none() {
            let a = np.call_method("ascontiguousarray", (&arr,), Some(&kw))?;
            let mm = np.call_method("ascontiguousarray", (&m,), Some(&kw))?;
            arr = rs.call_method1(intern!(py, "group_placed_points"), (a, mm))?;
        }
        out.append(arr)?;
    }
    if out.is_empty() { np.call_method1("empty", ((0, 3),)) } else { np.call_method1("concatenate", (out,)) }
}

/// `(frame, lo, hi)`: the mesh's extent along `frame`'s axes (the world's
/// when not given). `lo`/`hi` are the min/max coordinates in that frame, so
/// the box corners are `sum(axis * c for axis, c in zip(frame, corner))`.
///
/// `points` short-circuits the mesh: an `(N, 3)` world-space array from
/// `placement_points`, so a component with nested placements is not merged
/// into one mesh just to be measured.
#[pyfunction]
#[pyo3(signature = (mesh, frame=None, points=None))]
fn oriented_bounds<'py>(mesh: &Obj<'py>, frame: Option<&Obj<'py>>, points: Option<&Obj<'py>>) -> PyResult<Bound<'py, PyTuple>> {
    let py = mesh.py();
    let frame = match frame.filter(|f| !f.is_none()) {
        Some(f) => f.clone(),
        None => PyTuple::new(py, [vec3(py, 1.0, 0.0, 0.0)?, vec3(py, 0.0, 1.0, 0.0)?, vec3(py, 0.0, 0.0, 1.0)?])?.into_any(),
    };
    let zero = || -> PyResult<Bound<'py, PyTuple>> { PyTuple::new(py, [frame.clone(), PyTuple::new(py, [0.0, 0.0, 0.0])?.into_any(), PyTuple::new(py, [0.0, 0.0, 0.0])?.into_any()]) };
    let pos = match points.filter(|p| !p.is_none()) {
        Some(p) => {
            if p.len()? == 0 {
                return zero();
            }
            p.clone()
        }
        None => {
            let verts = mesh.getattr(intern!(py, "vertices"))?;
            if !verts.is_truthy()? {
                return zero();
            }
            let rows = PyList::empty(py);
            for v in verts.try_iter()? {
                let p = v?.getattr(intern!(py, "position"))?;
                rows.append((p.call_method0("x")?, p.call_method0("y")?, p.call_method0("z")?))?;
            }
            f64_array(&rows.into_any())?
        }
    };
    let np = np(py)?;
    let axes = PyList::empty(py);
    for a in frame.try_iter()? {
        let a = a?;
        axes.append((a.call_method0("x")?, a.call_method0("y")?, a.call_method0("z")?))?;
    }
    let axes = f64_array(&axes.into_any())?;
    let pts = np.call_method1("ascontiguousarray", (f64_array(&pos)?.call_method1("reshape", (-1, 3))?,))?;
    let lohi = native(py)?.call_method1(intern!(py, "group_oriented_extent"), (pts, axes))?;
    PyTuple::new(py, [frame, lohi.get_item(0)?, lohi.get_item(1)?])
}

/// The eight corners of an oriented box, in world coordinates.
#[pyfunction]
fn oriented_box_corners<'py>(frame: &Obj<'py>, lo: &Obj<'py>, hi: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let py = frame.py();
    let (u, v, w) = (frame.get_item(0)?, frame.get_item(1)?, frame.get_item(2)?);
    let v3 = native(py)?.getattr(intern!(py, "Vec3"))?;
    let out = PyList::empty(py);
    for i in 0..8 {
        let pick = |bit: usize, k: usize| if i & bit != 0 { hi.get_item(k) } else { lo.get_item(k) };
        let p = u.mul(pick(1, 0)?)?.add(v.mul(pick(2, 1)?)?)?.add(w.mul(pick(4, 2)?)?)?;
        out.append(v3.call1((p,))?)?;
    }
    Ok(out)
}

/// The paint a placement's default faces show: its own, else the
/// top-level object's it belongs to (a nested piece of a painted
/// component). None for an unpainted one.
#[pyfunction]
fn effective_material<'py>(group: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = group.py();
    if group.is_none() {
        return Ok(none(py));
    }
    let own = attr_or_none(group, intern!(py, "material"))?;
    if own.is_truthy()? {
        return Ok(own);
    }
    let owner = attr_or_none(group, intern!(py, "owner"))?;
    if !owner.is_none() && !owner.is(group) {
        let m = attr_or_none(&owner, intern!(py, "material"))?;
        return Ok(if m.is_truthy()? { m } else { none(py) });
    }
    Ok(none(py))
}

/// Fill the module `core.group`.
pub(crate) fn install(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("__doc__", "Group: a self-contained chunk of geometry with its own mesh (rust/ingetrazo_core/src/model/group_api.rs).")?;
    let rs = native(py)?;
    let dm = py.import("core.doc_model")?;
    m.add("_NATIVE", &rs)?;
    m.add("ingetrazo_core", &rs)?;
    m.add("_dm", &dm)?;
    m.add("Mesh", py.import("ingetrazo_core.mesh")?.getattr("Mesh")?)?;
    m.add("_counter", module(py, "itertools")?.call_method1("count", (1,))?)?;
    m.add("_document_counter", py.import("contextvars")?.getattr("ContextVar")?.call(("document_group_counter",), Some(&{
        let kw = PyDict::new(py);
        kw.set_item("default", py.None())?;
        kw
    }))?)?;
    m.add("GROUP_FIELDS", PyTuple::new(py, GROUP_FIELDS)?)?;
    m.add("DOC_FIELDS", PyTuple::new(py, DOC_FIELDS)?)?;
    m.add("ATTR_FIELDS", PyTuple::new(py, ATTR_FIELDS)?)?;
    m.add("_PY_FIELDS", PyTuple::new(py, PY_FIELDS)?)?;
    m.add("_UID_POOL", uid_pool(py)?)?;
    m.add("_UID_BATCH", UID_BATCH)?;
    m.add_function(wrap_pyfunction!(drop_uid_pool, m)?)?;
    m.add_function(wrap_pyfunction!(new_uid, m)?)?;
    m.add_function(wrap_pyfunction!(reserve_group_names, m)?)?;
    m.add_class::<NameScope>()?;
    m.add_function(wrap_pyfunction!(carry_axes, m)?)?;
    m.add_function(wrap_pyfunction!(group_frame, m)?)?;
    m.add_function(wrap_pyfunction!(frame_axes, m)?)?;
    m.add_function(wrap_pyfunction!(make_billboard_group, m)?)?;
    m.add_function(wrap_pyfunction!(copy_group, m)?)?;
    m.add_function(wrap_pyfunction!(is_link, m)?)?;
    m.add_function(wrap_pyfunction!(copy_definition_mesh, m)?)?;
    m.add_function(wrap_pyfunction!(hull_2d, m)?)?;
    m.add_function(wrap_pyfunction!(frame_from_points, m)?)?;
    m.add_function(wrap_pyfunction!(local_points, m)?)?;
    m.add_function(wrap_pyfunction!(placement_points, m)?)?;
    m.add_function(wrap_pyfunction!(oriented_bounds, m)?)?;
    m.add_function(wrap_pyfunction!(oriented_box_corners, m)?)?;
    m.add_function(wrap_pyfunction!(effective_material, m)?)?;
    // `iter_placements(group, base=None)`: `(group, world_matrix)` for
    // `group` and every nested placement below it, depth first, each matrix
    // composed from the root down (None only while the whole chain is
    // untransformed). A native iterator over the document
    // (`placement_walks.rs`), lazy as a generator.
    m.add("iter_placements", rs.getattr("group_iter_placements")?)?;
    // `world_mesh(group, placements=None)`: a world-space mesh for a group
    // (`group_mesh.rs`).
    m.add("world_mesh", rs.getattr("group_world_mesh")?)?;
    if let Ok(register) = module(py, "os")?.getattr("register_at_fork") {
        let kw = PyDict::new(py);
        kw.set_item("after_in_child", m.getattr("_drop_uid_pool")?)?;
        register.call((), Some(&kw))?;
    }

    let group = GROUP_CLASS.get_or_try_init(py, || -> PyResult<Py<PyAny>> {
        let ns = PyDict::new(py);
        ns.set_item("__doc__", "A placement: a group, a component instance or a nested part.\nIts document fields (``DOC_FIELDS``) live in the Rust document;\n``owner`` and ``context`` are Python slots.")?;
        ns.set_item("__slots__", PyTuple::new(py, PY_FIELDS)?)?;
        let kw = PyDict::new(py);
        kw.set_item("optional", true)?;
        ns.set_item("source_uid", rs.getattr("DocAttr")?.call(("source_uid",), Some(&kw))?)?;
        super::py_class::fast_kw_method(&ns, c"__init__", group_init_entry)?;
        super::py_class::fast_method(&ns, c"is_component", is_component_entry)?;
        method(&ns, "adopt", wrap_pyfunction!(group_adopt, m)?)?;
        super::py_class::fast_method(&ns, c"is_instance", is_instance_entry)?;
        method(&ns, "materialize", wrap_pyfunction!(group_materialize, m)?)?;
        method(&ns, "make_unique", wrap_pyfunction!(group_make_unique, m)?)?;
        method(&ns, "__repr__", wrap_pyfunction!(group_repr, m)?)?;
        method(&ns, "__reduce__", wrap_pyfunction!(group_reduce, m)?)?;
        ns.set_item("__setstate__", dm.getattr("set_handle_state")?)?;
        let cls = class(py, MODULE, "Group", &[rs.getattr("Placement")?], &ns)?;
        for name in ATTR_FIELDS {
            cls.setattr(name, dm.call_method1("attribute", (name,))?)?;
        }
        Ok(cls.unbind())
    })?;
    m.add("Group", group.bind(py))?;

    let proxy = PROXY_CLASS.get_or_try_init(py, || -> PyResult<Py<PyAny>> {
        let ns = PyDict::new(py);
        ns.set_item("__doc__", "A placement the VIEWPORT makes (one per nested part, carrying its\ncomposed world matrix, inherited tag and paint): a ``Group`` to every\ndraw, pick and ``isinstance``, and no part of the document. Every field\nis a Python slot of its own (they shadow the document's descriptors) and\nthe handle names no node (``_detached``), so building thousands of them\nper pack writes nothing into the document and reads at slot speed.")?;
        let slots: Vec<&str> = DOC_FIELDS.iter().copied().chain(["source_occurrence"]).collect();
        ns.set_item("__slots__", PyTuple::new(py, slots)?)?;
        ns.set_item("__new__", py.import("builtins")?.getattr("staticmethod")?.call1((wrap_pyfunction!(proxy_new, m)?,))?)?;
        super::py_class::fast_kw_method(&ns, c"__init__", proxy_init_entry)?;
        method(&ns, "__reduce__", wrap_pyfunction!(group_reduce, m)?)?;
        Ok(class(py, MODULE, "PlacementProxy", &[group.bind(py).clone()], &ns)?.unbind())
    })?;
    m.add("PlacementProxy", proxy.bind(py))?;
    Ok(())
}
