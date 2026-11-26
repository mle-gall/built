// SPDX-License-Identifier: GPL-3.0-or-later
//! The document tools' viewport on the resident view: what a tool picks,
//! previews and measures through while the Metal view is the one shown
//! (`MeshToolAccess`), and the dispatch of the window's active tool to it
//! (`NativeTools`). They replaced `views/wgpu_mesh_tools.py` and
//! `views/wgpu_tools.py`.
//!
//! Both are document bindings: their state is the instance's own
//! attributes, and what a tool asks that this view does not answer goes to
//! the document controller (`view.source_viewport`), as before.
use pyo3::{
    basic::CompareOp,
    exceptions::PyAttributeError,
    prelude::*,
    types::{PyCFunction, PyDict, PyList, PySet, PySlice, PyTuple},
    IntoPyObjectExt,
};

type Obj<'py> = Bound<'py, PyAny>;
type Reply<'py> = PyResult<Obj<'py>>;

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
/// `getattr(o, name, None)`.
fn attr<'py>(o: &Obj<'py>, name: &str) -> Reply<'py> {
    Ok(o.getattr_opt(name)?.unwrap_or_else(|| none(o.py())))
}
/// The attribute, when it is there and not None.
fn some<'py>(o: &Obj<'py>, name: &str) -> PyResult<Option<Obj<'py>>> {
    Ok(o.getattr_opt(name)?.filter(|v| !v.is_none()))
}
fn class_name(o: &Obj<'_>) -> PyResult<String> {
    Ok(o.get_type().name()?.to_string())
}
fn interaction(py: Python<'_>) -> Reply<'_> {
    py.import("views.document_interaction")?.getattr("DocumentInteraction")
}
fn pair<'py>(o: &Obj<'py>) -> PyResult<(Obj<'py>, Obj<'py>)> {
    Ok((o.get_item(0)?, o.get_item(1)?))
}
fn own_dict<'py>(o: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    Ok(o.getattr("__dict__")?.cast_into::<PyDict>()?)
}
fn id_of(o: &Obj<'_>) -> usize {
    o.as_ptr() as usize
}
fn numpy(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("numpy")
}
fn axis1(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    let kw = PyDict::new(py);
    kw.set_item("axis", 1)?;
    Ok(kw)
}
/// `matrix = projection * view` of the view's camera.
fn mvp<'py>(view: &Obj<'py>) -> Reply<'py> {
    let camera = view.getattr("camera")?;
    camera.call_method0("projection_matrix")?.mul(camera.call_method0("view_matrix")?)
}
/// Run `body` inside the context manager `cm`.
fn within<'py>(cm: &Obj<'py>, body: impl FnOnce() -> Reply<'py>) -> Reply<'py> {
    let py = cm.py();
    cm.call_method0("__enter__")?;
    match body() {
        Ok(value) => {
            cm.call_method1("__exit__", (py.None(), py.None(), py.None()))?;
            Ok(value)
        }
        Err(error) => {
            let swallowed = cm.call_method1(
                "__exit__",
                (error.get_type(py), error.value(py), error.traceback(py)),
            )?;
            if swallowed.is_truthy()? {
                Ok(none(py))
            } else {
                Err(error)
            }
        }
    }
}
/// A positional or keyword argument that may be left out (`...` in the
/// Python signatures these bindings keep).
fn optional<'py>(
    args: &Bound<'py, PyTuple>,
    kwargs: Option<&Bound<'py, PyDict>>,
    at: usize,
    name: &str,
) -> Reply<'py> {
    let py = args.py();
    if args.len() > at {
        return args.get_item(at);
    }
    if let Some(value) = kwargs.map(|k| k.get_item(name)).transpose()?.flatten() {
        return Ok(value);
    }
    Ok(py.Ellipsis().into_bound(py))
}

/// Class of each registered tool on the Metal view: `select`, `tape`,
/// `move` or `mesh` (the machine's own preview and measurement).
pub const TOOL_DISPATCH: &[(&str, &str)] = &[
    ("SelectTool", "select"),
    ("TapeMeasureTool", "tape"),
    ("MoveTool", "move"),
    ("SectionPlaneTool", "mesh"),
    ("LineTool", "mesh"),
    ("RectangleTool", "mesh"),
    ("PushPullTool", "mesh"),
    ("EraserTool", "mesh"),
    ("RotateTool", "mesh"),
    ("ScaleTool", "mesh"),
    ("OffsetTool", "mesh"),
    ("CircleTool", "mesh"),
    ("PolygonTool", "mesh"),
    ("ArcTool", "mesh"),
    ("PaintTool", "mesh"),
    ("TexturePositionTool", "mesh"),
    ("DimensionTool", "mesh"),
    ("TextTool", "mesh"),
    ("FollowMeTool", "mesh"),
    ("FreehandTool", "mesh"),
    ("RotatedRectangleTool", "mesh"),
    ("ThreePointArcTool", "mesh"),
    ("CenterArcTool", "mesh"),
    ("PieTool", "mesh"),
    ("FlipTool", "mesh"),
    ("PlaceGroupTool", "mesh"),
    ("PasteTool", "mesh"),
    ("FilletTool", "mesh"),
    ("ProtractorTool", "mesh"),
    ("GeoPathTool", "mesh"),
    ("ChangeAxesTool", "mesh"),
    ("ScaleManagerTool", "mesh"),
    ("PositionCameraTool", "mesh"),
    ("WalkTool", "mesh"),
    ("LookAroundTool", "mesh"),
    ("FirstPersonTool", "mesh"),
    ("OuterShellTool", "mesh"),
    ("UnionTool", "mesh"),
    ("SubtractTool", "mesh"),
    ("TrimTool", "mesh"),
    ("IntersectTool", "mesh"),
    ("SplitTool", "mesh"),
    ("NorthTool", "mesh"),
];

fn dispatch_of(name: &str) -> Option<&'static str> {
    TOOL_DISPATCH.iter().find(|(tool, _)| *tool == name).map(|(_, mode)| *mode)
}

/// One Move of `groups` by `delta`: a command, or a compound of them.
#[pyfunction]
fn move_command<'py>(groups: &Obj<'py>, delta: &Obj<'py>) -> Reply<'py> {
    let py = groups.py();
    let history = py.import("built_core.history")?;
    let make = history.getattr("MoveGroupCommand")?;
    let commands = PyList::empty(py);
    for group in groups.try_iter()? {
        commands.append(make.call1((group?, delta))?)?;
    }
    if commands.len() == 1 {
        commands.get_item(0)
    } else {
        history.getattr("CompoundCommand")?.call1((commands,))
    }
}

/// Document-tool facade for native mesh editing; no GL picking or drawing.
#[pyclass(module = "built_render", subclass, dict)]
pub struct MeshToolAccess;

/// How near (logical pixels) a click must come to a line to take it.
const EDGE_PICK_PX: f64 = 12.;

fn mesh_feedback(s: &Obj<'_>) -> PyResult<()> {
    let py = s.py();
    py.import("built_render")?.call_method1("sync_tool_preview", (s,))?;
    let tool = s.getattr("tool")?;
    if tool.hasattr("value_label")? {
        let label = tool.call_method0("value_label")?;
        if !label.is_none() {
            s.getattr("measurementChanged")?.call_method1("emit", (label.get_item(0)?,))?;
        }
    }
    Ok(())
}

fn unselectable_line(scene: &Obj<'_>, edge: &Obj<'_>, reveal: bool) -> PyResult<bool> {
    Ok(!scene.call_method1("entity_selectable", (edge,))?.is_truthy()?
        || ((attr(edge, "hidden")?.is_truthy()? || attr(edge, "soft")?.is_truthy()?) && !reveal))
}

/// `edge.a + (edge.b - edge.a) * t`.
fn along<'py>(edge: &Obj<'py>, t: f64) -> Reply<'py> {
    let a = edge.getattr("a")?;
    a.add(edge.getattr("b")?.sub(&a)?.mul(t)?)
}

#[pymethods]
#[allow(non_snake_case)]
impl MeshToolAccess {
    #[new]
    #[pyo3(signature = (*_args, **_kwargs))]
    fn new(_args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>) -> Self {
        Self
    }

    #[classattr]
    fn mesh_outline_preview() -> bool {
        true
    }
    /// Do not borrow the GL viewport's cached loose-edge screen projection.
    #[classattr]
    fn _ledge_screen(py: Python<'_>) -> Py<PyAny> {
        py.None()
    }
    /// SelectTool uses its model-space box traversal.
    #[classattr]
    fn _group_chunk(py: Python<'_>) -> Py<PyAny> {
        py.None()
    }
    #[classattr]
    fn _OCCLUSION_STEP_PX() -> f64 {
        12.
    }
    #[classattr]
    fn _OCCLUSION_MAX_SAMPLES() -> i64 {
        128
    }
    #[classattr]
    fn _OCCLUSION_SPLIT() -> i64 {
        8
    }
    #[classattr]
    fn _OCCLUSION_ROUNDS() -> i64 {
        2
    }
    /// `DocumentInteraction._CLIP_W`.
    #[classattr]
    fn _CLIP_W() -> f64 {
        1e-3
    }
    /// Meshes whose ray picker `_face_picker` keeps at once (the cursor
    /// goes back and forth between a few).
    #[classattr]
    fn FACE_TRIANGLE_MEMO() -> usize {
        8
    }
    #[classattr]
    #[pyo3(name = "EDGE_PICK_PX")]
    fn edge_pick_px() -> f64 {
        EDGE_PICK_PX
    }

    #[staticmethod]
    #[pyo3(signature = (*args))]
    fn _text_block_x<'py>(args: &Bound<'py, PyTuple>) -> Reply<'py> {
        interaction(args.py())?.getattr("_text_block_x")?.call1(args.clone())
    }
    #[staticmethod]
    #[pyo3(signature = (*args))]
    fn _format_dim_value<'py>(args: &Bound<'py, PyTuple>) -> Reply<'py> {
        interaction(args.py())?.getattr("_format_dim_value")?.call1(args.clone())
    }

    /// Other placements are queried through their world-space face plane;
    /// their local vertices must not be projected as world coordinates.
    fn inference_meshes<'py>(slf: &Bound<'py, Self>, _x: &Obj<'py>, _y: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
        PyList::new(slf.py(), [slf.as_any().getattr("scene")?.getattr("mesh")?])
    }

    #[pyo3(signature = (x, y, **kwargs))]
    fn work_point<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>, kwargs: Option<&Bound<'py, PyDict>>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let picked = optional(&PyTuple::empty(py), kwargs, 0, "picked_row")?;
        let axes = py.import("built_core.axes")?;
        let tool = s.getattr("tool")?;
        let own = tool.call_method1("drag_plane", (s,))?;
        let mut plane = if own.is_truthy()? { own.clone() } else { attr(&tool, "work_plane")? };
        if own.is_none() && !plane.is_none() {
            // GL's _current_work_plane (#545): a PLANAR shape re-plans onto
            // the face the cursor is over, capture or not, and takes its
            // capture with it — or the preview stayed on the face it started
            // on while the point and the badge went to the other one.
            let face_plane = s.call_method1("_face_replan", (&tool, x, y, &plane, &picked))?;
            if !face_plane.is_none() {
                tool.setattr("work_plane", &face_plane)?;
                plane = face_plane;
            }
        }
        let anchor = || -> Reply<'py> {
            let start = s.getattr("start_point")?;
            if start.is_none() { axes.call_method0("origin") } else { Ok(start) }
        };
        if plane.is_none() {
            let lock = attr(&tool, "locked_work_plane")?;
            if lock.is_truthy()? {
                plane = lock.call1((anchor()?,))?;
            }
        }
        if plane.is_none() {
            plane = s.call_method1("_face_plane_under", (x, y, &picked))?;
        }
        if plane.is_none() {
            let mut normal = axes.call_method1("axis", ("z",))?;
            let forward = s.getattr("camera")?.call_method0("forward")?;
            let dot: f64 = py.import("built_core")?.getattr("Vec3")?
                .call_method1("dotProduct", (&forward, &normal))?.extract()?;
            if dot.abs() < 0.2 {
                normal = forward;
            }
            plane = (anchor()?, normal).into_bound_py_any(py)?;
        }
        s.setattr("_last_work_plane", &plane)?;
        s.call_method1("_ray_hit_plane", ((x, y), plane))
    }

    /// `DocumentInteraction._planar_face_plane` with the face under the
    /// cursor from this view's picks (`_face_plane_under`). Only a planar
    /// shape pays for the ray.
    fn _face_replan<'py>(slf: &Bound<'py, Self>, tool: &Obj<'py>, x: &Obj<'py>, y: &Obj<'py>, captured: &Obj<'py>, picked_row: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        if !tool.is_instance(&py.import("tools.base")?.getattr("PlaneLock")?)? {
            return Ok(none(py));
        }
        let face_plane = s.call_method1("_face_plane_under", (x, y, picked_row))?;
        if face_plane.is_none() {
            return Ok(none(py));
        }
        let kw = PyDict::new(py);
        kw.set_item("face_plane", face_plane)?;
        interaction(py)?.call_method("_planar_face_plane", (s, tool, (x, y), captured), Some(&kw))
    }

    /// World `(point, unit normal)` of the face under the cursor, or None:
    /// the placement under the cursor (the ID pass) and its triangle on the
    /// cursor ray, from the snap index — one ray over one placement, where
    /// the mesh walk (`pick_face_placement`) costs the mesh's size. The
    /// walk still answers before the index is built and while a tool moves
    /// faces (a moving face is no plane to land on, and only the walk knows
    /// which triangles are its).
    #[pyo3(signature = (x, y, *args, **kwargs))]
    fn _face_plane_under<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>, args: &Bound<'py, PyTuple>, kwargs: Option<&Bound<'py, PyDict>>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let picked = optional(args, kwargs, 0, "picked_row")?;
        let view = s.getattr("view")?;
        let index = attr(&view, "inference_index")?;
        let moving = attr(&s.getattr("tool")?, "moving_face_ids")?;
        if index.is_none() || (moving.is_callable() && moving.call0()?.is_truthy()?) {
            let (face, placement) = pair(&s.call_method1("pick_face_placement", (x, y))?)?;
            if face.is_none() {
                return Ok(none(py));
            }
            return py.import("built_core.snap")?.call_method1("face_plane_world", (face, attr(&placement, "xform")?));
        }
        let row = if picked.is(&py.Ellipsis()) { view.call_method1("pick_row", (x, y))? } else { picked };
        if row.is_none() {
            return Ok(none(py));
        }
        let (origin, direction) = pair(&s.call_method1("_pixel_to_ray", (x, y))?)?;
        if origin.is_none() || direction.is_none() {
            return Ok(none(py));
        }
        let hit = py.import("built_core")?.call_method1("surface_hit", (index, row, origin, direction))?;
        if hit.is_none() || hit.get_item(1)?.is_none() {
            return Ok(none(py));
        }
        Ok(hit)
    }

    /// GL's: cut where the segment passes behind the eye instead of
    /// dropping it (a guide preview is kilometres long).
    fn _segment_to_pixels<'py>(slf: &Bound<'py, Self>, a: &Obj<'py>, b: &Obj<'py>) -> Reply<'py> {
        interaction(slf.py())?.call_method1("_segment_to_pixels", (slf, a, b))
    }
    fn _clip_w<'py>(slf: &Bound<'py, Self>) -> Reply<'py> {
        interaction(slf.py())?.call_method1("_clip_w", (slf,))
    }
    fn _pixel_to_ray<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        slf.as_any().getattr("view")?.call_method1("_pixel_to_ray", (x, y))
    }
    #[pyo3(signature = (point, mvp = None))]
    fn _world_to_pixel<'py>(slf: &Bound<'py, Self>, point: &Obj<'py>, mvp: Option<Obj<'py>>) -> Reply<'py> {
        match mvp {
            None => slf.as_any().getattr("view")?.call_method1("_world_to_pixel", (point,)),
            // Called once per vertex by the walks (box select's fallback).
            Some(mvp) => interaction(slf.py())?.call_method1("_world_to_pixel", (slf, point, mvp)),
        }
    }
    #[pyo3(signature = (*args))]
    fn _project_to_lock_line<'py>(slf: &Bound<'py, Self>, args: &Bound<'py, PyTuple>) -> Reply<'py> {
        slf.as_any().getattr("view")?.getattr("_project_to_lock_line")?.call1(args.clone())
    }
    fn _ray_hit_plane<'py>(slf: &Bound<'py, Self>, cursor: &Obj<'py>, plane: &Obj<'py>) -> Reply<'py> {
        interaction(slf.py())?.call_method1("_ray_hit_plane", (slf, cursor, plane))
    }

    fn _picked_mesh<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        let s = slf.as_any();
        let row = if s.getattr("mode")?.eq("select")? {
            s.call_method1("_selection_row", (x, y))?
        } else {
            s.getattr("view")?.call_method1("pick_row", (x, y))?
        };
        s.call_method1("_mesh_for_row", (row,))
    }

    fn _mesh_for_row<'py>(slf: &Bound<'py, Self>, row: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let view = s.getattr("view")?;
        let resolved = view.call_method1("resolve", (row,))?;
        if resolved.is_none() {
            return Ok(none(py));
        }
        let group = resolved.get_item(0)?;
        let scene = s.getattr("scene")?;
        let packed = view.getattr("packed")?;
        let instances = packed.getattr("instances")?;
        let mut world: Vec<Vec<f64>> = instances.get_item("world")?.get_item(row)?.call_method0("tolist")?.extract()?;
        let mut mesh = if group.is_none() { scene.getattr("loose_mesh")? } else { group.getattr("mesh")? };
        let code: usize = instances.get_item("info")?.get_item((row, 0))?.call_method0("__int__")?.extract()?;
        let origins = match packed.getattr_opt("prototype_origins")? {
            Some(origins) => origins,
            None => PyTuple::empty(py).into_any(),
        };
        if code < origins.len()? {
            let anchor: Vec<f64> = origins.get_item(code)?.extract()?;
            let base: Vec<f64> = packed.getattr("origin")?.extract()?;
            if anchor.iter().chain(&base).any(|v| *v != 0.) {
                // The tool's active mesh is in scene space; the GPU row
                // expects vertices relative to the prototype anchor.
                let store = own_dict(&mesh)?.get_item("_store")?.filter(|st| !st.is_none());
                let frame: Vec<f64> = match store {
                    Some(store) => store.getattr("frame_origin")?.extract()?,
                    None => vec![0.; 3],
                };
                for column in 0..3 {
                    let moved: f64 = (0..3).map(|i| (frame[i] - anchor[i]) * world[i][column]).sum();
                    world[3][column] += moved;
                }
            }
        }
        let matrix4 = py.import("built_core")?.getattr("Matrix4")?;
        let flat: Vec<f64> = (0..4).flat_map(|r| (0..4).map(move |c| (r, c))).map(|(r, c)| world[c][r]).collect();
        let mut matrix = matrix4.call1((flat,))?;
        if !group.is_none() && group.is(&scene.getattr("edit_group")?) {
            // The editing mesh is in world coordinates even if the retained
            // GPU row still draws its local-space definition.
            mesh = scene.getattr("mesh")?;
            matrix = matrix4.call0()?;
        }
        (mesh, group, matrix).into_bound_py_any(py)
    }

    /// The face of the placement under the cursor that the cursor ray meets
    /// first, and that placement (None for loose geometry): one native ray
    /// test over the mesh's triangles, kept per mesh (`_face_picker`).
    fn pick_face_placement<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let view = s.getattr("view")?;
        let any = py.import("builtins")?.getattr("any")?;
        let packed = attr(&view, "packed")?;
        let shifted = match packed.getattr_opt("origin")? {
            Some(origin) => any.call1((origin,))?.is_truthy()?,
            None => false,
        };
        if shifted || view.getattr_opt("_remote_meshes")?.is_some_and(|m| m.is_truthy().unwrap_or(false)) {
            let frame = view.call_method0("_geometry_frame")?;
            return within(&frame, || s.call_method1("_face_placement_at", (x, y)));
        }
        s.call_method1("_face_placement_at", (x, y))
    }

    fn _face_placement_at<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let nothing = || (py.None(), py.None()).into_bound_py_any(py);
        let picked = s.call_method1("_picked_mesh", (x, y))?;
        if picked.is_none() {
            return nothing();
        }
        let (mesh, group, matrix) = (picked.get_item(0)?, picked.get_item(1)?, picked.get_item(2)?);
        let (origin, direction) = pair(&s.call_method1("_pixel_to_ray", (x, y))?)?;
        let (inverse, ok) = pair(&matrix.call_method0("inverted")?)?;
        if !ok.is_truthy()? {
            return nothing();
        }
        let local_origin = inverse.call_method1("map", (&origin,))?;
        let local_direction = inverse.call_method1("mapVector", (&direction,))?;
        let excluded = match s.getattr("tool")?.getattr_opt("moving_face_ids")? {
            Some(moving) => moving.call0()?,
            None => PySet::empty(py)?.into_any(),
        };
        let hits = s.call_method1("_face_picker", (&mesh,))?.call_method1(
            "hits",
            (local_origin.call_method0("toTuple")?, local_direction.call_method0("toTuple")?, excluded),
        )?;
        let scene = s.getattr("scene")?;
        let section = if scene.getattr("show_section_cuts")?.is_truthy()? {
            scene.call_method0("active_section")?
        } else { none(py) };
        let faces = mesh.getattr("faces")?;
        let mut best = none(py);
        for hit in hits.try_iter()? {
            let (ordinal, distance) = pair(&hit?)?;
            if !section.is_none() {
                let point = origin.add(direction.mul(distance)?)?;
                if section.call_method1("side", (point,))?.extract::<f64>()? > 0. { continue; }
            }
            let face = faces.get_item(ordinal)?;
            // Visible locked geometry still supplies drawing planes. Revealed
            // hidden faces can be picked too, according to the scene rules.
            if scene.call_method1("entity_visible", (&face,))?.is_truthy()?
                || scene.call_method1("entity_selectable", (&face,))?.is_truthy()? {
                best = face;
                break;
            }
        }
        let placement = if group.is_none() {
            none(py)
        } else {
            let kw = PyDict::new(py);
            kw.set_item("xform", matrix)?;
            kw.set_item("source_occurrence", group)?;
            py.import("types")?.getattr("SimpleNamespace")?.call((), Some(&kw))?
        };
        (best, placement).into_bound_py_any(py)
    }

    /// The native ray picker for this mesh revision and tool coordinate frame.
    fn _face_picker<'py>(slf: &Bound<'py, Self>, mesh: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let store = own_dict(mesh)?.get_item("_store")?.filter(|st| !st.is_none());
        let frame = match &store {
            Some(store) => py.get_type::<PyTuple>().call1((store.getattr("frame_origin")?,))?,
            None => none(py),
        };
        let key = (attr(mesh, "_mut_serial")?, attr(&s.getattr("scene")?, "version")?, frame).into_bound_py_any(py)?;
        let memo = own_dict(s)?.call_method1("setdefault", ("_face_tri_memo", PyDict::new(py)))?.cast_into::<PyDict>()?;
        let id = id_of(mesh);
        if let Some(hit) = memo.get_item(id)? {
            if hit.get_item(0)?.is(mesh) && hit.get_item(1)?.eq(&key)? {
                return hit.get_item(2);
            }
        }
        let hypot = py.import("core._native")?.call_method0("qt_hypot")?;
        let value = match &store {
            Some(store) if hypot.is_truthy()? => store.call_method1("tool_face_picker", (hypot,))?,
            _ => py.import("built_core")?.getattr("FacePicker")?.call1((mesh.getattr("faces")?,))?,
        };
        let _ = memo.call_method1("pop", (id, py.None()))?;
        let limit: usize = s.getattr("FACE_TRIANGLE_MEMO")?.extract()?;
        while memo.len() >= limit {
            let Some(first) = memo.keys().try_iter()?.next().transpose()? else { break };
            memo.del_item(first)?;
        }
        memo.set_item(id, (mesh, key, &value))?;
        Ok(value)
    }

    fn pick_face_any<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let nothing = || (py.None(), py.None()).into_bound_py_any(py);
        let scene = s.getattr("scene")?;
        let selectable = |entity: &Obj<'py>| -> PyResult<bool> { scene.call_method1("entity_selectable", (entity,))?.is_truthy() };
        let (face, placement) = pair(&s.call_method1("pick_face_placement", (x, y))?)?;
        let group = match placement.getattr_opt("source_occurrence")? {
            Some(group) => group,
            None => placement.clone(),
        };
        if face.is_none() || !selectable(&face)? {
            return nothing();
        }
        if group.is(&scene.getattr("edit_group")?) {
            return (face, py.None()).into_bound_py_any(py);
        }
        // The ray hit used for a drawing plane can also hit a locked
        // reference. Paint and the other editing tools need an editable
        // placement, including the container selected in this context.
        let owner = s.getattr("view")?.call_method1("context_placement", (&group,))?;
        if owner.is_none() || !selectable(&group)? || !selectable(&owner)? {
            return nothing();
        }
        (face, group).into_bound_py_any(py)
    }

    fn pick_face<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        let s = slf.as_any();
        let (face, group) = pair(&s.call_method1("pick_face_any", (x, y))?)?;
        if group.is_none() && s.getattr("scene")?.call_method1("entity_selectable", (&face,))?.is_truthy()? {
            Ok(face)
        } else {
            Ok(none(slf.py()))
        }
    }

    fn _selection_row<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let view = s.getattr("view")?;
        let mut row = view.call_method1("pick_row", (x, y))?;
        let indexed = !view.getattr("inference_index")?.is_none();
        if !row.is_none() && indexed && s.call_method0("_linework_rows")?.get_item(&row)?.is_truthy()? {
            // GL's pick_group: the face under the cursor before a line
            // drawn over it (the lines are SelectTool._linework's).
            row = s.call_method1("_face_row_under", (x, y, &row))?;
        }
        let context = s.getattr("scene")?.getattr("edit_group")?;
        if !context.is_none() {
            let group = if row.is_none() { none(py) } else { view.getattr("packed")?.getattr("groups")?.get_item(&row)? };
            if !group.is(&context) && view.call_method1("context_placement", (&group,))?.is_none() {
                // Fade retains depth. GL picks only the editable subtree,
                // even when another placement covers it in the ID pass.
                row = if indexed {
                    py.import("built_core")?.call_method1("context_hit", (&view, &context, x, y))?
                } else {
                    s.call_method1("_context_mesh_row", (&context, x, y))?
                };
            }
        }
        Ok(row)
    }

    /// The open group's row when the cursor ray meets its own faces — what
    /// `context_hit` answers while the snap index is still being built on
    /// its worker. Without it a face covered in the ID pass by the faded
    /// rest took no click at all until the index came back.
    fn _context_mesh_row<'py>(slf: &Bound<'py, Self>, context: &Obj<'py>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let view = s.getattr("view")?;
        let packed = view.getattr("packed")?;
        let instances = packed.getattr("instances")?;
        let count = instances.len()? as i64;
        let groups = packed.getattr("groups")?;
        let info = instances.get_item("info")?;
        let mut first = None;
        let candidates = view.getattr("uid_rows")?.call_method1("get", (context.getattr("uid")?, PyTuple::empty(py)))?;
        for row in candidates.try_iter()? {
            let row = row?;
            let at: i64 = row.call_method0("__int__")?.extract()?;
            if at < 0 || at >= count || !groups.get_item(at)?.is(context) {
                continue;
            }
            let flags: i64 = info.get_item((at, 2))?.call_method0("__int__")?.extract()?;
            if flags & 129 == 0 {
                first = Some(row);
                break;
            }
        }
        let Some(first) = first else { return Ok(none(py)) };
        let (origin, direction) = pair(&s.call_method1("_pixel_to_ray", (x, y))?)?;
        let face = s.call_method1("_face_picker", (s.getattr("scene")?.getattr("mesh")?,))?.call_method1(
            "first",
            (origin.call_method0("toTuple")?, direction.call_method0("toTuple")?, PySet::empty(py)?),
        )?;
        Ok(if face.is_none() { none(py) } else { first })
    }

    /// The row of the front face at `(x, y)` among the placements that have
    /// faces — the ID pass shows the line of a lines-only placement drawn
    /// over it — else `default`.
    fn _face_row_under<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>, default: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let inference = py.import("built_core")?;
        let view = s.getattr("view")?;
        let index = view.getattr("inference_index")?;
        let rows = inference.call_method1("eligible_rows", (&index, view.getattr("packed")?))?;
        let wire = s.call_method0("_linework_rows")?.get_item(&rows)?.call_method0("__invert__")?;
        let rows = rows.get_item(wire)?;
        if rows.len()? == 0 {
            return Ok(default.clone());
        }
        let hit = inference.call_method1("front_at", (&index, &view, x, y, rows, mvp(&view)?))?;
        if hit.is_none() { Ok(default.clone()) } else { hit.get_item(2) }
    }

    fn pick_group<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let view = s.getattr("view")?;
        let row = s.call_method1("_selection_row", (x, y))?;
        view.setattr("hovered_row", &row)?;
        let group = if row.is_none() { none(py) } else { view.getattr("packed")?.getattr("groups")?.get_item(&row)? };
        let target = view.call_method1("context_placement", (group,))?;
        if !target.is_none() && s.getattr("scene")?.call_method1("entity_selectable", (&target,))?.is_truthy()? {
            Ok(target)
        } else {
            Ok(none(py))
        }
    }

    fn pick_locked_group<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let picked = s.call_method1("_picked_mesh", (x, y))?;
        let target = if picked.is_truthy()? {
            s.getattr("view")?.call_method1("context_placement", (picked.get_item(1)?,))?
        } else {
            none(py)
        };
        if !target.is_none() && attr(&target, "locked")?.is_truthy()? {
            let (visible, layer_locked) = pair(&s.getattr("scene")?.call_method1("_layer_state", (&target,))?)?;
            if visible.is_truthy()? && !layer_locked.is_truthy()? {
                return Ok(target);
            }
        }
        Ok(none(py))
    }

    fn pick_locked_image_plane<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        interaction(slf.py())?.call_method1("pick_locked_image_plane", (slf, x, y))
    }
    fn _face_or_image_in_front<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        slf.as_any().call_method1("pick_face", (x, y))
    }
    fn _context_pick<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        interaction(slf.py())?.call_method1("_context_pick", (slf, x, y))
    }

    /// `(edge, pixels)` of the nearest selectable loose edge within
    /// `EDGE_PICK_PX` that `hidden` does not hide, else `(None, inf)`. The
    /// store's segments near the cursor come from Rust, nearest first; the
    /// tag, hidden-geometry and occlusion rules read the first few.
    fn _pick_loose_edge<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>, hidden: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let scene = s.getattr("scene")?;
        let mesh = scene.getattr("mesh")?;
        let Some(store) = own_dict(&mesh)?.get_item("_store")?.filter(|st| !st.is_none()) else {
            return s.call_method1("_pick_loose_edge_walk", (x, y, hidden));
        };
        let (matrix, width, height) = {
            let camera = s.call_method0("_camera_px")?;
            (camera.get_item(0)?, camera.get_item(1)?, camera.get_item(2)?)
        };
        let (px, py_): (f64, f64) = (x.extract()?, y.extract()?);
        let hits = store.call_method1("nearest_edges", (matrix, width, height, px, py_, s.getattr("EDGE_PICK_PX")?))?;
        let edges = mesh.getattr("edges")?;
        let reveal = scene.getattr("show_hidden_geometry")?.is_truthy()?;
        for hit in hits.try_iter()? {
            let hit = hit?;
            let edge = edges.get_item(hit.get_item(0)?)?;
            if unselectable_line(&scene, &edge, reveal)? {
                continue;
            }
            let t: f64 = hit.get_item(2)?.extract()?;
            if !hidden.call1((along(&edge, t)?,))?.is_truthy()? {
                let distance: f64 = hit.get_item(1)?.extract()?;
                return (edge, distance).into_bound_py_any(py);
            }
        }
        (py.None(), f64::INFINITY).into_bound_py_any(py)
    }

    /// `_pick_loose_edge` for a mesh without a store.
    fn _pick_loose_edge_walk<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>, hidden: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let scene = s.getattr("scene")?;
        let (cx, cy): (f64, f64) = (x.extract()?, y.extract()?);
        let mut best = None;
        let mut distance: f64 = s.getattr("EDGE_PICK_PX")?.extract()?;
        for edge in scene.getattr("mesh")?.getattr("edges")?.try_iter()? {
            let edge = edge?;
            if unselectable_line(&scene, &edge, scene.getattr("show_hidden_geometry")?.is_truthy()?)? {
                continue;
            }
            let a = s.call_method1("_world_to_pixel", (edge.getattr("a")?,))?;
            let b = s.call_method1("_world_to_pixel", (edge.getattr("b")?,))?;
            if a.is_none() || b.is_none() {
                continue;
            }
            let (ax, ay): (f64, f64) = (a.get_item(0)?.extract()?, a.get_item(1)?.extract()?);
            let (bx, by): (f64, f64) = (b.get_item(0)?.extract()?, b.get_item(1)?.extract()?);
            let (dx, dy) = (bx - ax, by - ay);
            let t = (((cx - ax) * dx + (cy - ay) * dy) / (dx * dx + dy * dy).max(1e-12)).clamp(0., 1.);
            let dist = (ax + t * dx - cx).hypot(ay + t * dy - cy);
            if dist < distance && !hidden.call1((along(&edge, t)?,))?.is_truthy()? {
                best = Some(edge);
                distance = dist;
            }
        }
        match best {
            Some(edge) => (edge, distance).into_bound_py_any(py),
            None => (py.None(), f64::INFINITY).into_bound_py_any(py),
        }
    }

    fn pick_edge<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        let s = slf.as_any();
        s.call_method1("_pick_loose_edge", (x, y, s.getattr("_is_occluded")?))?.get_item(0)
    }
    fn pick_visible_edge<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        let s = slf.as_any();
        s.call_method1("_pick_loose_edge", (x, y, s.getattr("_is_occluded")?))?.get_item(0)
    }

    /// `DocumentInteraction.linework_hidden`: occlusion with the depth
    /// slack of a line lying on (or a hair off) a face.
    fn linework_hidden<'py>(slf: &Bound<'py, Self>, point: &Obj<'py>) -> Reply<'py> {
        let s = slf.as_any();
        let slack = slf.py().import("views.document_interaction")?
            .call_method1("linework_slack_point", (s.getattr("view")?.getattr("camera")?, point))?;
        s.call_method1("_is_occluded", (slack,))
    }

    /// `(edge, pixels)`: the loose edge the Select tool takes before the
    /// face under the cursor.
    fn pick_linework_edge<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        let s = slf.as_any();
        s.call_method1("_pick_loose_edge", (x, y, s.getattr("linework_hidden")?))
    }

    /// Mask over the resident rows: placements of lines only (their
    /// prototype draws no face, shown or hidden).
    fn _linework_rows<'py>(slf: &Bound<'py, Self>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let packed = s.getattr("view")?.getattr("packed")?;
        if let Some(mask) = some(&packed, "_linework_rows")? {
            return Ok(mask);
        }
        let int64 = numpy(py)?.getattr("int64")?;
        let mut faces = packed.getattr("commands")?.get_item("count")?.call_method1("astype", (int64,))?;
        if let Some(hidden) = some(&packed, "hidden_commands")? {
            faces = faces.add(hidden.get_item("count")?)?;
        }
        let codes = packed.getattr("instances")?.get_item("info")?.get_item((PySlice::full(py), 0))?;
        let mask = faces.get_item(codes)?.rich_compare(0, CompareOp::Eq)?;
        packed.setattr("_linework_rows", &mask)?;
        Ok(mask)
    }

    /// `[(group, pixels)]`, nearest first: the objects made of lines only
    /// whose visible lines pass within `EDGE_PICK_PX` of the cursor. Their
    /// 1-px lines in the ID pass were all the hitbox they had.
    fn pick_linework_groups<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
        let py = slf.py();
        let s = slf.as_any();
        let out = PyList::empty(py);
        let view = s.getattr("view")?;
        let index = view.getattr("inference_index")?;
        let packed = view.getattr("packed")?;
        if index.is_none() || packed.is_none() || packed.getattr("instances")?.len()? == 0 {
            return Ok(out);
        }
        let wire = s.call_method0("_linework_rows")?;
        if !wire.call_method0("any")?.is_truthy()? {
            return Ok(out);
        }
        let scene = s.getattr("scene")?;
        let radius: f64 = s.getattr("EDGE_PICK_PX")?.extract()?;
        let kw = PyDict::new(py);
        kw.set_item("radius_px", radius)?;
        kw.set_item("cap", 48)?;
        kw.set_item("rows", numpy(py)?.call_method1("flatnonzero", (wire,))?)?;
        kw.set_item("edit_group", scene.getattr("edit_group")?)?;
        let edges = py.import("built_core")?.call_method(
            "nearby_edges",
            (index, packed, view.getattr("camera")?, view.call_method0("width")?, view.call_method0("height")?, x, y),
            Some(&kw),
        )?;
        let under = interaction(py)?.getattr("edge_point_under_cursor")?;
        let mut seen = Vec::new();
        for edge in edges.try_iter()? {
            let edge = edge?;
            let distance: f64 = edge.getattr("distance")?.extract()?;
            if !edge.getattr("in_group")?.is_truthy()? || !(distance < radius) {
                continue;
            }
            let target = view.call_method1("context_placement", (edge.getattr("group")?,))?;
            if target.is_none() || seen.contains(&id_of(&target)) || !scene.call_method1("entity_selectable", (&target,))?.is_truthy()? {
                continue;
            }
            let point = under.call1((s, &edge, x, y))?;
            if point.is_none() || s.call_method1("linework_hidden", (point,))?.is_truthy()? {
                continue;
            }
            seen.push(id_of(&target));
            out.append((target, distance))?;
        }
        Ok(out)
    }

    /// The objects made of lines only whose screen box holds the cursor,
    /// smallest first.
    fn linework_groups_around<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let empty = || Ok(PyList::empty(py).into_any());
        let view = s.getattr("view")?;
        let index = view.getattr("inference_index")?;
        let packed = view.getattr("packed")?;
        if index.is_none() || packed.is_none() || packed.getattr("instances")?.len()? == 0 {
            return empty();
        }
        let wire = s.call_method0("_linework_rows")?;
        if !wire.call_method0("any")?.is_truthy()? {
            return empty();
        }
        let inference = py.import("built_core")?;
        let np = numpy(py)?;
        let boxes = inference.call_method1(
            "screen_boxes",
            (&index, &packed, mvp(&view)?, view.call_method0("width")?, view.call_method0("height")?),
        )?;
        let (lo, hi) = (boxes.get_item(0)?, boxes.get_item(1)?);
        let rows = inference.call_method1("eligible_rows", (&index, &packed))?;
        let finite = |boxes: &Obj<'py>| -> Reply<'py> {
            np.call_method1("isfinite", (boxes.get_item(&rows)?,))?.call_method("all", (), Some(&axis1(py)?))
        };
        let keep = wire.get_item(&rows)?.bitand(finite(&lo)?)?.bitand(finite(&hi)?)?;
        let rows = rows.get_item(keep)?;
        let scene = s.getattr("scene")?;
        let edit = scene.getattr("edit_group")?.unbind();
        let (groups, placer) = (packed.getattr("groups")?.unbind(), view.getattr("context_placement")?.unbind());
        let owner = PyCFunction::new_closure(py, None, None, move |args: &Bound<'_, PyTuple>, _: Option<&Bound<'_, PyDict>>| -> PyResult<Py<PyAny>> {
            let py = args.py();
            let group = groups.bind(py).get_item(args.get_item(0)?)?;
            if group.is(edit.bind(py)) {
                Ok(py.None())
            } else {
                Ok(placer.bind(py).call1((group,))?.unbind())
            }
        })?;
        let column = |boxes: &Obj<'py>, at: usize| boxes.get_item((&rows, at));
        py.import("views.document_interaction")?.call_method1(
            "_boxes_around",
            (&rows, column(&lo, 0)?, column(&lo, 1)?, column(&hi, 0)?, column(&hi, 1)?, x, y, owner, scene),
        )
    }

    fn _camera_px<'py>(slf: &Bound<'py, Self>) -> Reply<'py> {
        let py = slf.py();
        let view = slf.as_any().getattr("view")?;
        let matrix = py.get_type::<PyList>().call1((mvp(&view)?.call_method0("data")?,))?;
        let width: f64 = view.call_method0("width")?.extract()?;
        let height: f64 = view.call_method0("height")?.extract()?;
        (matrix, width, height).into_bound_py_any(py)
    }

    fn _project_px<'py>(slf: &Bound<'py, Self>, pts: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let np = numpy(py)?;
        let kw = PyDict::new(py);
        kw.set_item("dtype", np.getattr("float64")?)?;
        let points = np.call_method("ascontiguousarray", (pts,), Some(&kw))?.call_method1("reshape", (-1, 3))?;
        let camera = slf.as_any().call_method0("_camera_px")?;
        py.import("built_core")?.call_method1(
            "project_px",
            (points, camera.get_item(0)?, camera.get_item(1)?, camera.get_item(2)?),
        )
    }

    /// Whether every viewing ray to this segment ends before resident geometry.
    fn _clear_annotation_segment<'py>(slf: &Bound<'py, Self>, a: &Obj<'py>, b: &Obj<'py>) -> PyResult<bool> {
        let py = slf.py();
        let s = slf.as_any();
        let view = s.getattr("view")?;
        let index = attr(&view, "inference_index")?;
        if index.is_none() {
            return Ok(true);
        }
        let pa = s.call_method1("_world_to_pixel", (a,))?;
        let pb = s.call_method1("_world_to_pixel", (b,))?;
        if pa.is_none() || pb.is_none() {
            return Ok(false);
        }
        let (ax, ay): (f64, f64) = (pa.get_item(0)?.extract()?, pa.get_item(1)?.extract()?);
        let (bx, by): (f64, f64) = (pb.get_item(0)?.extract()?, pb.get_item(1)?.extract()?);
        let (x, y) = ((ax + bx) * 0.5, (ay + by) * 0.5);
        let radius = 0.5 * (ax - bx).hypot(ay - by) + 2.;
        let width: f64 = view.call_method0("width")?.extract()?;
        let height: f64 = view.call_method0("height")?.extract()?;
        if radius > 128. || !(0. <= x && x <= width && 0. <= y && y <= height) {
            return Ok(false);
        }
        let rows = py.import("built_core")?.call_method1(
            "screen_rows",
            (&index, view.getattr("packed")?, mvp(&view)?, width, height, x, y, radius, py.None()),
        )?;
        if rows.len()? == 0 {
            return Ok(true);
        }
        // The near-plane origins and the segment endpoints span a convex
        // quadrilateral. Reject a box only if a whole axis separates it
        // from that quadrilateral (touching at the measured surface is OK).
        let oa = s.call_method1("_pixel_to_ray", (ax, ay))?.get_item(0)?;
        let ob = s.call_method1("_pixel_to_ray", (bx, by))?.get_item(0)?;
        if oa.is_none() || ob.is_none() {
            return Ok(false);
        }
        let xyz = |point: &Obj<'py>| -> PyResult<[f64; 3]> { point.call_method0("toTuple")?.extract() };
        let (oa, ob, ea, eb) = (xyz(&oa)?, xyz(&ob)?, xyz(a)?, xyz(b)?);
        let (centers, extents) = pair(&index.getattr("world_box_cache")?)?;
        let rows_of = |values: &Obj<'py>| -> PyResult<Vec<[f64; 3]>> { values.get_item(&rows)?.call_method0("tolist")?.extract() };
        let (centers, extents) = (rows_of(&centers)?, rows_of(&extents)?);
        Ok(centers.iter().zip(&extents).all(|(c, e)| {
            (0..3).any(|k| {
                let (low, high) = (c[k] - e[k], c[k] + e[k]);
                let before = oa[k].max(ob[k]) < low && ea[k].max(eb[k]) <= low;
                let after = oa[k].min(ob[k]) > high && ea[k].min(eb[k]) >= high;
                before || after
            })
        }))
    }

    fn _is_occluded<'py>(slf: &Bound<'py, Self>, point: &Obj<'py>) -> PyResult<bool> {
        let py = slf.py();
        let s = slf.as_any();
        let view = s.getattr("view")?;
        let pixel = s.call_method1("_world_to_pixel", (point,))?;
        if pixel.is_none() {
            return Ok(true);
        }
        let (px, py_) = pair(&pixel)?;
        let row = view.call_method1("pick_row", (&px, &py_))?;
        let index = view.getattr("inference_index")?;
        if row.is_none() || index.is_none() {
            return Ok(false);
        }
        let (origin, direction) = pair(&s.call_method1("_pixel_to_ray", (px, py_))?)?;
        let hit = py.import("built_core")?.call_method1("surface", (index, row, &origin, direction))?;
        if hit.is_none() {
            return Ok(false);
        }
        let near: f64 = hit.sub(&origin)?.call_method0("length")?.extract()?;
        let far: f64 = point.sub(&origin)?.call_method0("length")?.extract()?;
        Ok(near + 1e-4 < far)
    }

    #[pyo3(signature = (entity, placement = None))]
    fn set_hover<'py>(slf: &Bound<'py, Self>, entity: &Obj<'py>, placement: Option<Obj<'py>>) -> PyResult<()> {
        let s = slf.as_any();
        s.setattr("_hover_entity", entity)?;
        s.setattr("_mesh_hover", entity)?;
        // The component placement a face was picked through: its dots are
        // drawn through that matrix (GL's _hover_placement).
        s.setattr("_mesh_hover_placement", placement)
    }

    fn set_suppressed_faces<'py>(slf: &Bound<'py, Self>, faces: &Obj<'py>) -> PyResult<()> {
        let py = slf.py();
        let s = slf.as_any();
        if class_name(&attr(s, "tool")?)? == "TexturePositionTool" {
            // TexturePreview replaces the resident material stream itself;
            // suppressing the base would also suppress its mapped preview.
            return Ok(());
        }
        let view = s.getattr("view")?;
        let faces = if faces.is_truthy()? { py.get_type::<PyTuple>().call1((faces,))? } else { PyTuple::empty(py).into_any() };
        let keys = |faces: &Obj<'py>| -> PyResult<Bound<'py, PySet>> {
            let keys = PySet::empty(py)?;
            for face in faces.try_iter()? {
                keys.add(face?.getattr("key")?)?;
            }
            Ok(keys)
        };
        let shown = match view.getattr_opt("suppressed_faces")? {
            Some(shown) => shown,
            None => PyTuple::empty(py).into_any(),
        };
        if keys(&faces)?.as_any().eq(keys(&shown)?)? {
            return Ok(());
        }
        view.setattr("suppressed_faces", &faces)?;
        if let Some(renderer) = some(&view, "renderer")? {
            if renderer.hasattr("set_suppressed_faces")? {
                let ranges = py.import("views.wgpu_parity")?.call_method1("face_ranges", (&view, &faces))?;
                renderer.call_method1("set_suppressed_faces", (ranges,))?;
            }
        }
        view.call_method0("update")?;
        Ok(())
    }

    /// Publish the machine's transient mesh and its formatted measurement.
    #[pyo3(name = "mesh_feedback")]
    fn publish_mesh_feedback(slf: &Bound<'_, Self>) -> PyResult<()> {
        mesh_feedback(slf.as_any())
    }

    fn draw_annotations<'py>(slf: &Bound<'py, Self>, painter: &Obj<'py>) -> PyResult<()> {
        let py = slf.py();
        let s = slf.as_any();
        let annotations = py.import("views.wgpu_annotations")?;
        let view = s.getattr("view")?;
        let hover = attr(s, "_mesh_hover")?;
        s.setattr("_hover_entity", &hover)?;
        if let Some(marks) = some(&view, "_draw_selection_marks")? {
            if attr(s, "_moving_marks")?.is_none() {
                marks.call1((painter,))?;
            }
        }
        let device = painter.call_method0("device")?;
        let scale: f64 = device.call_method0("devicePixelRatioF")?.extract()?;
        let width: f64 = device.call_method0("width")?.extract()?;
        let height: f64 = device.call_method0("height")?.extract()?;
        let kw = PyDict::new(py);
        kw.set_item("occluded", s.getattr("_is_occluded")?)?;
        kw.set_item("hover", hover)?;
        kw.set_item("clear_segment", s.getattr("_clear_annotation_segment")?)?;
        kw.set_item("guide_lines", !attr(&view, "renderer")?.hasattr("set_lines")?)?;
        let layout = annotations.call_method(
            "draw_annotations",
            (painter, s.getattr("scene")?, s.getattr("camera")?, (width / scale, height / scale)),
            Some(&kw),
        )?;
        crate::plugin_panels::draw_tool_feedback(painter, s.as_any(), &layout)?;
        let snap = s.getattr("last_snap")?;
        if !snap.is_none() && attr(&snap, "kind")?.is_truthy()? && snap.hasattr("color")? {
            // With its tooltip ("Endpoint", "On Edge in group"), as GL.
            let kw = PyDict::new(py);
            kw.set_item("scene", attr(s, "scene")?)?;
            py.import("views.wgpu_overlay")?.call_method(
                "draw_snap_indicator",
                (painter, snap, layout.getattr("_world_to_pixel")?, layout.getattr("_clip_pixel_line")?),
                Some(&kw),
            )?;
        }
        annotations.call_method1("draw_tool_overlays", (painter, s))?;
        Ok(())
    }

    /// Extension API (`ExtensionApp.add_overlay`) on the Metal view.
    fn world_to_pixel<'py>(slf: &Bound<'py, Self>, world: &Obj<'py>) -> Reply<'py> {
        slf.as_any().call_method1("_world_to_pixel", (world,))
    }
    fn world_to_pixels<'py>(slf: &Bound<'py, Self>, pts: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let kw = PyDict::new(py);
        kw.set_item("dtype", py.get_type::<pyo3::types::PyFloat>())?;
        let points = numpy(py)?.call_method("asarray", (pts,), Some(&kw))?.call_method1("reshape", (-1, 3))?;
        slf.as_any().call_method1("_project_px", (points,))
    }
    fn _draw_occluded_segment<'py>(slf: &Bound<'py, Self>, painter: &Obj<'py>, a: &Obj<'py>, b: &Obj<'py>) -> PyResult<()> {
        interaction(slf.py())?.call_method1("_draw_occluded_segment", (slf, painter, a, b))?;
        Ok(())
    }
    fn _project_samples<'py>(slf: &Bound<'py, Self>, points: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
        let s = slf.as_any();
        let out = PyList::empty(slf.py());
        for point in points.try_iter()? {
            let point = point?;
            let pixel = s.call_method1("_world_to_pixel", (&point,))?;
            let hidden = s.call_method1("_is_occluded", (&point,))?.is_truthy()?;
            out.append((pixel, !hidden))?;
        }
        Ok(out)
    }
    fn _occluded_on_screen<'py>(slf: &Bound<'py, Self>, point: &Obj<'py>) -> Reply<'py> {
        slf.as_any().call_method1("_is_occluded", (point,))
    }

    #[pyo3(signature = (*args, **kwargs))]
    fn pick_dimension<'py>(slf: &Bound<'py, Self>, args: &Bound<'py, PyTuple>, kwargs: Option<&Bound<'py, PyDict>>) -> Reply<'py> {
        forward(slf.as_any(), "pick_dimension", args, kwargs)
    }
    #[pyo3(signature = (*args, **kwargs))]
    fn pick_text_label<'py>(slf: &Bound<'py, Self>, args: &Bound<'py, PyTuple>, kwargs: Option<&Bound<'py, PyDict>>) -> Reply<'py> {
        forward(slf.as_any(), "pick_text_label", args, kwargs)
    }
    #[pyo3(signature = (*_args, **_kwargs))]
    fn _unsupported_pick(&self, _args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>) {}
    #[pyo3(signature = (*_args, **_kwargs))]
    fn pick_image(&self, _args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>) {}
    #[pyo3(signature = (*_args, **_kwargs))]
    fn image_plane_at(&self, _args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>) {}
    #[pyo3(signature = (*args))]
    fn pick_geopath<'py>(slf: &Bound<'py, Self>, args: &Bound<'py, PyTuple>) -> Reply<'py> {
        forward(slf.as_any(), "pick_geopath", args, None)
    }
    #[pyo3(signature = (*args))]
    fn _draw_geo_path_label<'py>(slf: &Bound<'py, Self>, args: &Bound<'py, PyTuple>) -> Reply<'py> {
        forward(slf.as_any(), "_draw_geo_path_label", args, None)
    }
    fn section_plane_frame<'py>(slf: &Bound<'py, Self>, plane: &Obj<'py>) -> Reply<'py> {
        interaction(slf.py())?.call_method1("section_plane_frame", (slf, plane))
    }
    fn _clip_segment_front<'py>(slf: &Bound<'py, Self>, a: &Obj<'py>, b: &Obj<'py>) -> Reply<'py> {
        interaction(slf.py())?.call_method1("_clip_segment_front", (slf, a, b))
    }
    fn pick_section_plane<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        interaction(slf.py())?.call_method1("pick_section_plane", (slf, x, y))
    }
    fn pick_image_plane<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let kw = PyDict::new(py);
        kw.set_item("selectable_only", true)?;
        interaction(py)?.call_method("image_plane_at", (slf, x, y), Some(&kw))
    }
    fn image_in_front_of<'py>(slf: &Bound<'py, Self>, face: &Obj<'py>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        interaction(slf.py())?.call_method1("image_in_front_of", (slf, face, x, y))
    }
    fn edge_point_under_cursor<'py>(slf: &Bound<'py, Self>, edge: &Obj<'py>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        interaction(slf.py())?.call_method1("edge_point_under_cursor", (slf, edge, x, y))
    }
    fn pick_extension_item<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        interaction(slf.py())?.call_method1("pick_extension_item", (slf, x, y))
    }
    fn _group_obb<'py>(slf: &Bound<'py, Self>, group: &Obj<'py>) -> Reply<'py> {
        match some(&slf.as_any().getattr("view")?, "_group_obb")? {
            Some(bounds) => bounds.call1((group,)),
            None => Ok(none(slf.py())),
        }
    }

    fn begin_loose_preview<'py>(slf: &Bound<'py, Self>, groups: &Obj<'py>, vertices: &Obj<'py>) -> PyResult<bool> {
        let py = slf.py();
        let s = slf.as_any();
        let view = s.getattr("view")?;
        let scene = s.getattr("scene")?;
        let mesh = scene.getattr("mesh")?;
        let renderer = attr(&view, "renderer")?;
        if renderer.is_none() || !mesh.is(&scene.getattr("loose_mesh")?) {
            return Ok(false);
        }
        let rows = view.getattr("uid_rows")?.call_method1("get", ("loose", PyTuple::empty(py)))?;
        if rows.len()? != 1 {
            return Ok(false);
        }
        let keys = PySet::empty(py)?;
        for vertex in vertices.try_iter()? {
            keys.add(vertex?.getattr("key")?)?;
        }
        let loose = PyDict::new(py);
        if keys.len() == mesh.getattr("vertices")?.len()? {
            s.call_method1("_begin_marks", (&renderer, &keys))?;
            loose.set_item("include_loose", true)?;
            s.call_method("_begin_rows", (groups,), Some(&loose))?;
            return Ok(true);
        }
        let store = own_dict(&mesh)?.get_item("_store")?.filter(|st| !st.is_none());
        let Some(store) = store.filter(|_| renderer.hasattr("begin_tool_geometry").unwrap_or(false)) else {
            return Ok(false);
        };
        let np = numpy(py)?;
        let packed = view.getattr("packed")?;
        let info = packed.getattr("instances")?.get_item("info")?;
        let code: usize = info.get_item((rows.get_item(0)?, 0))?.call_method0("__int__")?.extract()?;
        let same = info.get_item((PySlice::full(py), 0))?.rich_compare(code, CompareOp::Eq)?;
        let placements: usize = np.call_method1("count_nonzero", (same,))?.extract()?;
        if attr(&scene, "show_hidden_geometry")?.is_truthy()? || placements != 1 {
            return Ok(false);
        }
        let origins = packed.getattr("prototype_origins")?;
        let anchor = if code < origins.len()? { origins.get_item(code)? } else { (0., 0., 0.).into_bound_py_any(py)? };
        let origin = np.call_method1("subtract", (anchor, store.getattr("frame_origin")?))?;
        let points = store.call_method1("preview_points", (py.get_type::<PyList>().call1((&keys,))?, &origin))?;
        if points.is_none() {
            return Ok(false);
        }
        let face = packed.getattr("commands")?.get_item(code)?;
        let int = |field: &str| -> PyResult<i64> { face.get_item(field)?.call_method0("__int__")?.extract() };
        let (at, count) = (int("first")?, int("count")?);
        let indices = packed.getattr("indices")?.get_item(PySlice::new(py, at as isize, (at + count) as isize, 1))?
            .call_method1("astype", (np.getattr("int64")?,))?.add(int("base")?)?;
        let corners = packed.getattr("vertices")?.get_item(indices)?
            .call_method1("view", (np.getattr("uint8")?,))?.call_method1("reshape", (-1,))?;
        if !renderer.call_method1("begin_tool_geometry", (code, corners, points, origin))?.is_truthy()? {
            return Ok(false);
        }
        s.call_method1("_begin_marks", (&renderer, &keys))?;
        loose.set_item("include_loose", false)?;
        s.call_method("_begin_rows", (groups,), Some(&loose))?;
        s.setattr("_loose_deform", renderer)?;
        Ok(true)
    }

    fn _begin_marks<'py>(slf: &Bound<'py, Self>, renderer: &Obj<'py>, keys: &Obj<'py>) -> PyResult<()> {
        let s = slf.as_any();
        if renderer.hasattr("begin_tool_marks")? {
            renderer.call_method1(
                "begin_tool_marks",
                (s.getattr("scene")?.getattr("selection")?, attr(s, "_mesh_hover")?, keys),
            )?;
            s.setattr("_moving_marks", renderer)?;
        }
        Ok(())
    }

    #[pyo3(signature = (groups, external = false))]
    fn begin_groups_preview<'py>(slf: &Bound<'py, Self>, groups: &Obj<'py>, external: bool) -> PyResult<()> {
        let py = slf.py();
        let s = slf.as_any();
        let kw = PyDict::new(py);
        if external {
            let view = s.getattr("view")?;
            kw.set_item("roots", groups)?;
            kw.set_item("texture_images", false)?;
            let pack = py.import("built_core")?.call_method("pack_scene", (s.getattr("scene")?,), Some(&kw))?;
            let renderer = view.getattr("renderer")?;
            renderer.call_method1("begin_group_feedback", (pack, view.getattr("packed")?.getattr("origin")?))?;
            return s.setattr("_clipboard_renderer", renderer);
        }
        kw.set_item("include_loose", false)?;
        s.call_method("_begin_rows", (groups,), Some(&kw))?;
        Ok(())
    }

    #[pyo3(signature = (groups, include_loose))]
    fn _begin_rows<'py>(slf: &Bound<'py, Self>, groups: &Obj<'py>, include_loose: bool) -> PyResult<()> {
        let py = slf.py();
        let s = slf.as_any();
        let np = numpy(py)?;
        let view = s.getattr("view")?;
        let moving = py.get_type::<PyTuple>().call1((groups,))?;
        s.setattr("_moving_groups", &moving)?;
        let selected = PySet::empty(py)?;
        for group in py.import("built_core")?.call_method1("purge_iter_groups", (&moving,))?.try_iter()? {
            selected.add(group?.getattr("uid")?)?;
        }
        if include_loose {
            selected.add("loose")?;
        }
        let rows = PyList::empty(py);
        let uid_rows = view.getattr("uid_rows")?;
        for uid in selected.iter() {
            for row in uid_rows.call_method1("get", (uid, PyTuple::empty(py)))?.try_iter()? {
                rows.append(row?)?;
            }
        }
        let ids = np.call_method1("asarray", (rows, np.getattr("int64")?))?;
        s.setattr("ids", &ids)?;
        if let Some(billboards) = some(&view, "_billboards")? {
            billboards.call_method0("restore")?;
            billboards.setattr("key", py.None())?;
        }
        let original = view.getattr("packed")?.getattr("instances")?.get_item(ids)?;
        s.setattr("original", &original)?;
        let bytes = original.call_method1("view", (np.getattr("uint8")?,))?.call_method1("reshape", (-1,))?;
        s.setattr("_instance_preview", py.import("built_render")?.getattr("InstancePreview")?.call1((bytes,))?)
    }

    fn set_groups_preview_offset<'py>(slf: &Bound<'py, Self>, delta: &Obj<'py>) -> PyResult<()> {
        let matrix = slf.py().import("built_core")?.getattr("Matrix4")?.call0()?;
        matrix.call_method1("translate", (delta,))?;
        slf.as_any().call_method1("set_groups_preview_matrix", (matrix,))?;
        Ok(())
    }

    fn set_groups_preview_matrix<'py>(slf: &Bound<'py, Self>, matrix: &Obj<'py>) -> PyResult<()> {
        let s = slf.as_any();
        if let Some(clipboard) = own_dict(s)?.get_item("_clipboard_renderer")?.filter(|c| !c.is_none()) {
            clipboard.call_method1("pose_group_feedback", (matrix.call_method0("data")?,))?;
            s.call_method0("update")?;
            return Ok(());
        }
        let original = s.getattr("original")?;
        if original.is_none() {
            return Ok(());
        }
        let transform = matrix.call_method0("data")?;
        if let Some(marks) = some(s, "_moving_marks")? {
            let count = match s.getattr("view")?.getattr_opt("_selected_mark_count")? {
                Some(count) => count,
                None => 0.into_bound_py_any(slf.py())?,
            };
            marks.call_method1("pose_tool_marks", (&transform, count))?;
        }
        if original.len()? != 0 {
            let rows = s.getattr("_instance_preview")?.call_method1("transformed", (&transform,))?
                .call_method1("view", (original.getattr("dtype")?,))?;
            s.call_method1("write_rows", (rows,))?;
        }
        if let Some(renderer) = some(s, "_loose_deform")? {
            renderer.call_method1("pose_tool_geometry", (transform,))?;
        }
        Ok(())
    }

    fn end_groups_preview(slf: &Bound<'_, Self>) -> PyResult<()> {
        let py = slf.py();
        let s = slf.as_any();
        let clipboard = own_dict(s)?.call_method1("pop", ("_clipboard_renderer", py.None()))?;
        if !clipboard.is_none() {
            clipboard.call_method0("end_group_feedback")?;
            s.call_method0("update")?;
            return Ok(());
        }
        if let Some(marks) = some(s, "_moving_marks")? {
            marks.call_method0("end_tool_marks")?;
            s.setattr("_moving_marks", py.None())?;
        }
        if let Some(renderer) = some(s, "_loose_deform")? {
            renderer.call_method0("end_tool_geometry")?;
            s.setattr("_loose_deform", py.None())?;
        }
        s.setattr("_moving_groups", PyTuple::empty(py))?;
        s.setattr("_instance_preview", py.None())?;
        s.call_method0("restore")?;
        s.setattr("original", py.None())
    }

    #[pyo3(signature = (px, py, radius_px = 48., cap = 48))]
    fn _resident_nearby_edges<'py>(slf: &Bound<'py, Self>, px: &Obj<'py>, py: &Obj<'py>, radius_px: f64, cap: i64) -> Reply<'py> {
        nearby_edges(slf.as_any(), px, py, radius_px, cap)
    }
    #[pyo3(signature = (px, py, radius_px = 64., cap = 400))]
    fn _nearby_loose_edges<'py>(slf: &Bound<'py, Self>, px: &Obj<'py>, py: &Obj<'py>, radius_px: f64, cap: i64) -> PyResult<Bound<'py, PyList>> {
        grouped_edges(slf.as_any(), px, py, radius_px, cap, false)
    }
    #[pyo3(signature = (px, py, radius_px = 48., cap = 48))]
    fn _nearby_group_edges<'py>(slf: &Bound<'py, Self>, px: &Obj<'py>, py: &Obj<'py>, radius_px: f64, cap: i64) -> PyResult<Bound<'py, PyList>> {
        grouped_edges(slf.as_any(), px, py, radius_px, cap, true)
    }

    /// Arc continuation from bounded resident curve features, not GL scans.
    fn reference_tangent<'py>(slf: &Bound<'py, Self>, point: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let view = s.getattr("view")?;
        let index = attr(&view, "inference_index")?;
        if index.is_none() {
            return Ok(none(py));
        }
        let pixel = view.call_method1("_world_to_pixel", (point,))?;
        if pixel.is_none() {
            return Ok(none(py));
        }
        let kw = PyDict::new(py);
        kw.set_item("radius_px", 12.)?;
        kw.set_item("cap", 48)?;
        let edges = s.call_method("_resident_nearby_edges", (pixel.get_item(0)?, pixel.get_item(1)?), Some(&kw))?;
        let inference = py.import("built_core")?;
        let packed = view.getattr("packed")?;
        let mut seen = Vec::new();
        for edge in edges.try_iter()? {
            let row = edge?.getattr("row")?;
            let key: i64 = row.call_method0("__int__")?.extract()?;
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            let direction = inference.call_method1("tangent_direction", (&index, &packed, row, point))?;
            if !direction.is_none() {
                return Ok(direction);
            }
        }
        Ok(none(py))
    }
}

/// `DocumentInteraction.<name>(self, *args, **kwargs)`.
fn forward<'py>(s: &Obj<'py>, name: &str, args: &Bound<'py, PyTuple>, kwargs: Option<&Bound<'py, PyDict>>) -> Reply<'py> {
    let py = s.py();
    let all = PyList::new(py, [s.clone()])?;
    for value in args.iter() {
        all.append(value)?;
    }
    interaction(py)?.getattr(name)?.call(all.to_tuple(), kwargs)
}

fn nearby_edges<'py>(s: &Obj<'py>, px: &Obj<'py>, py_: &Obj<'py>, radius_px: f64, cap: i64) -> Reply<'py> {
    let py = s.py();
    let view = s.getattr("view")?;
    let index = view.getattr("inference_index")?;
    if index.is_none() {
        return Ok(PyList::empty(py).into_any());
    }
    let kw = PyDict::new(py);
    kw.set_item("radius_px", radius_px)?;
    kw.set_item("cap", cap)?;
    kw.set_item("edit_group", s.getattr("scene")?.getattr("edit_group")?)?;
    py.import("built_core")?.call_method(
        "nearby_edges",
        (index, view.getattr("packed")?, view.getattr("camera")?, view.call_method0("width")?, view.call_method0("height")?, px, py_),
        Some(&kw),
    )
}

fn grouped_edges<'py>(s: &Obj<'py>, px: &Obj<'py>, py_: &Obj<'py>, radius_px: f64, cap: i64, in_group: bool) -> PyResult<Bound<'py, PyList>> {
    let out = PyList::empty(s.py());
    for edge in s.call_method1("_resident_nearby_edges", (px, py_, radius_px, cap))?.try_iter()? {
        let edge = edge?;
        if edge.getattr("in_group")?.is_truthy()? == in_group {
            out.append(edge)?;
        }
    }
    Ok(out)
}

/// Native tool dispatch shares measurement and History semantics with GL.
///
/// Live placement moves alter resident matrices only; the document is
/// touched once, by the same commands used by MoveTool, after the user
/// commits.
#[pyclass(module = "built_render", extends = MeshToolAccess, subclass)]
pub struct NativeTools;

/// What a camera mode parks of the tool it suspends and gives back when
/// that tool resumes: the first point, a Move's preview rows, the typed value.
const PARKED: [&str; 6] = ["tool", "mode", "start_point", "original", "ids", "value_buffer"];

fn apply_cursor(s: &Obj<'_>) -> PyResult<()> {
    if s.getattr("view")?.hasattr("setCursor")? {
        s.call_method0("_apply_tool_cursor")?;
    }
    Ok(())
}
fn adopt_viewport(s: &Obj<'_>, tool: &Obj<'_>) -> PyResult<()> {
    if tool.hasattr("_viewport")? {
        tool.setattr("_viewport", s)?;
    }
    Ok(())
}
fn start_of<'py>(tool: &Obj<'py>) -> Reply<'py> {
    attr(tool, "start_point")
}

#[pymethods]
#[allow(non_snake_case)]
impl NativeTools {
    #[new]
    #[pyo3(signature = (*_args, **_kwargs))]
    fn new(_args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(MeshToolAccess).add_subclass(Self)
    }

    fn __init__(slf: &Bound<'_, Self>, view: &Obj<'_>) -> PyResult<()> {
        let py = slf.py();
        let s = slf.as_any();
        s.setattr("view", view)?;
        s.setattr("tool", py.None())?;
        s.setattr("mode", "select")?;
        s.setattr("start_point", py.None())?;
        s.setattr("hover_point", py.None())?;
        let np = numpy(py)?;
        s.setattr("ids", np.call_method1("empty", (0, np.getattr("int64")?))?)?;
        s.setattr("original", py.None())?;
        s.setattr("groups", PyList::empty(py))?;
        s.setattr("_value_buffer", "")
    }

    #[classattr]
    fn _PARKED() -> [&'static str; 6] {
        PARKED
    }

    #[getter]
    fn value_buffer<'py>(slf: &Bound<'py, Self>) -> Reply<'py> {
        let s = slf.as_any();
        match s.getattr("view")?.getattr("source_viewport")?.getattr_opt("_value_buffer")? {
            Some(value) => Ok(value),
            None => s.getattr("_value_buffer"),
        }
    }
    #[setter]
    fn set_value_buffer(slf: &Bound<'_, Self>, value: &Obj<'_>) -> PyResult<()> {
        let s = slf.as_any();
        s.setattr("_value_buffer", value)?;
        let source = s.getattr("view")?.getattr("source_viewport")?;
        if source.hasattr("_value_buffer")? {
            source.setattr("_value_buffer", value)?;
        }
        Ok(())
    }
    #[getter]
    fn scene<'py>(slf: &Bound<'py, Self>) -> Reply<'py> {
        let view = slf.as_any().getattr("view")?;
        let own = attr(&view, "scene")?;
        if own.is_truthy()? {
            return Ok(own);
        }
        attr(&view.getattr("source_viewport")?, "scene")
    }
    #[getter]
    fn last_snap<'py>(slf: &Bound<'py, Self>) -> Reply<'py> {
        attr(&slf.as_any().getattr("view")?, "last_snap")
    }
    #[getter]
    fn active_tool<'py>(slf: &Bound<'py, Self>) -> Reply<'py> {
        slf.as_any().getattr("tool")
    }

    fn sync<'py>(slf: &Bound<'py, Self>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let view = s.getattr("view")?;
        let source = view.getattr("source_viewport")?;
        let tool = attr(&source, "active_tool")?;
        let current = s.getattr("tool")?;
        if tool.is(&current) {
            let mode: String = s.getattr("mode")?.extract()?;
            if matches!(mode.as_str(), "tape" | "move") && !s.getattr("start_point")?.is_none() && start_of(&tool)?.is_none() {
                s.call_method0("cancel")?;
            }
            return s.getattr("mode");
        }
        if tool.is_none() && !current.is_none() && attr(&source, "suspended_tool")?.is(&current) {
            return s.call_method0("_suspend");
        }
        let parked = own_dict(s)?.call_method1("pop", ("_parked", py.None()))?;
        if !parked.is_none() {
            if tool.is(&parked.get_item("tool")?) {
                return s.call_method1("_resume", (parked,));
            }
            // Another tool after the camera mode: the parked one is
            // cancelled here as any switch cancels it.
            for item in parked.call_method0("items")?.try_iter()? {
                let (name, value) = pair(&item?)?;
                s.setattr(name.extract::<String>()?.as_str(), value)?;
            }
        }
        if let Some(preview) = some(&view, "_texture_preview")? {
            preview.call_method0("restore")?;
        }
        let current = s.getattr("tool")?;
        if class_name(&current)? == "TexturePositionTool" {
            current.call_method1("_leave", (s,))?;
        }
        s.call_method0("cancel")?;
        s.setattr("tool", &tool)?;
        let default = if tool.is_none() {
            "select"
        } else if tool.is_instance(&py.import("tools.base")?.getattr("Tool")?)? {
            "mesh"
        } else {
            "inert"
        };
        let mut mode = dispatch_of(&class_name(&tool)?).unwrap_or(default);
        if mode == "move" {
            // Share MoveTool's copy toggle, numeric values and array window.
            mode = "mesh";
        }
        s.setattr("mode", mode)?;
        if mode == "tape" {
            tool.call_method1("on_cancel", (s,))?;
        }
        adopt_viewport(s, &tool)?;
        let revision = match some(&source, "history")? {
            Some(history) => match history.getattr_opt("revision")? {
                Some(revision) => revision,
                None => 0.into_bound_py_any(py)?,
            },
            None => 0.into_bound_py_any(py)?,
        };
        s.setattr("_notified_revision", revision)?;
        apply_cursor(s)?;
        if mode == "inert" {
            source.call_method1("flash_status", ("Unsupported document tool",))?;
        }
        s.getattr("mode")
    }

    /// Orbit / Pan / Zoom took the view from the tool: park it as it is. No
    /// `on_cancel`, and a Move's rows stay where the preview put them (the
    /// preview mesh too), so the model shows what was being done while the
    /// camera turns.
    fn _suspend<'py>(slf: &Bound<'py, Self>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let parked = PyDict::new(py);
        for name in s.getattr("_PARKED")?.try_iter()? {
            let name = name?;
            parked.set_item(&name, s.getattr(name.extract::<String>()?.as_str())?)?;
        }
        s.setattr("_parked", parked)?;
        s.setattr("tool", py.None())?;
        s.setattr("mode", "select")?;
        s.setattr("start_point", py.None())?;
        s.setattr("original", py.None())?;
        s.call_method1("_set_value_buffer", ("",))?;
        s.setattr("_mesh_hover", py.None())?;
        s.getattr("view")?.setattr("overlay_primitives", PyList::empty(py))?;
        apply_cursor(s)?;
        s.getattr("mode")
    }

    fn _resume<'py>(slf: &Bound<'py, Self>, parked: &Obj<'py>) -> Reply<'py> {
        let s = slf.as_any();
        for item in parked.call_method0("items")?.try_iter()? {
            let (name, value) = pair(&item?)?;
            s.setattr(name.extract::<String>()?.as_str(), value)?;
        }
        s.call_method1("_set_value_buffer", (s.getattr("value_buffer")?,))?;
        adopt_viewport(s, &s.getattr("tool")?)?;
        apply_cursor(s)?;
        s.getattr("mode")
    }

    /// The Select machine: the active tool, or the one this view keeps for
    /// the gestures made while no tool is active.
    fn select_tool<'py>(slf: &Bound<'py, Self>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let select = py.import("tools.rust_tools")?.getattr("SelectTool")?;
        let tool = s.getattr("tool")?;
        if tool.is_instance(&select)? {
            return Ok(tool);
        }
        let own = own_dict(s)?;
        if let Some(idle) = own.get_item("_idle_select")?.filter(|idle| !idle.is_none()) {
            return Ok(idle);
        }
        let idle = select.call0()?;
        own.set_item("_idle_select", &idle)?;
        Ok(idle)
    }

    /// The pointer came into this view (one of several, sharing the
    /// window's tool): the tool's viewport is this one again, so what it
    /// draws and picks is here.
    fn claim(slf: &Bound<'_, Self>) -> PyResult<()> {
        let s = slf.as_any();
        let tool = s.getattr("tool")?;
        if !tool.is_none() {
            adopt_viewport(s, &tool)?;
        }
        Ok(())
    }

    fn context<'py>(slf: &Bound<'py, Self>, event: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let view = s.getattr("view")?;
        let snap = attr(&view, "last_snap")?;
        let position = event.call_method0("position")?;
        let xy = || -> PyResult<(Obj<'py>, Obj<'py>)> {
            let position = event.call_method0("position")?;
            Ok((position.call_method0("x")?, position.call_method0("y")?))
        };
        let point = if snap.is_none() { view.call_method1("_world_under_cursor", xy()?)? } else { snap.getattr("point")? };
        if point.is_none() {
            return Ok(none(py));
        }
        let tool = s.getattr("tool")?;
        if class_name(&tool)? == "FilletTool" {
            s.setattr("_hover_edge", s.call_method1("pick_visible_edge", xy()?)?)?;
        }
        if !tool.is_none() {
            tool.setattr("snap_kind", attr(&view.getattr("last_snap")?, "kind")?)?;
        }
        py.import("tools.base")?.getattr("ToolContext")?.call1((
            s,
            point,
            position,
            event.call_method0("modifiers")?,
            view.getattr("last_snap")?,
        ))
    }

    fn click<'py>(slf: &Bound<'py, Self>, event: &Obj<'py>) -> PyResult<()> {
        let py = slf.py();
        let s = slf.as_any();
        let mode = s.call_method0("sync")?;
        let ctx = s.call_method1("context", (event,))?;
        if ctx.is_none() {
            return Ok(());
        }
        let view = s.getattr("view")?;
        let tool = s.getattr("tool")?;
        if mode.eq("mesh")? {
            if class_name(&tool)? == "MoveTool" && tool.getattr("start_point")?.is_none() {
                let control = py.import("built_core")?.getattr("Input")?.getattr("ControlModifier")?;
                if ctx.getattr("modifiers")?.bitand(control)?.is_truthy()? && !tool.getattr("_copy")?.is_truthy()? {
                    let kw = PyDict::new(py);
                    kw.set_item("say", false)?;
                    tool.call_method("_toggle_copy", (s,), Some(&kw))?;
                }
            }
            let free = |tool: &Obj<'py>| -> PyResult<bool> { Ok(tool.hasattr("work_plane")? && tool.getattr("work_plane")?.is_none()) };
            let mut plane = None;
            if start_of(&tool)?.is_none() && free(&tool)? && attr(&view, "pick_row")?.is_callable() {
                // GL's _dispatch_tool_click: the first click on a face
                // captures its plane for the rest of the operation, so a
                // rectangle started on a sloped face and dragged past its
                // border stays on that face's plane and splits it. Asked
                // at the click only.
                let position = event.call_method0("position")?;
                let (face, placement) = pair(&s.call_method1(
                    "pick_face_placement",
                    (position.call_method0("x")?, position.call_method0("y")?),
                )?)?;
                if !face.is_none() {
                    plane = Some(py.import("built_core.snap")?.call_method1("face_plane_world", (face, attr(&placement, "xform")?))?);
                }
            }
            tool.call_method1("on_hover", (&ctx,))?;
            tool.call_method1("on_click", (&ctx,))?;
            let start = start_of(&tool)?;
            s.setattr("start_point", &start)?;
            if let Some(plane) = plane {
                if !start.is_none() && free(&tool)? {
                    tool.setattr("work_plane", plane)?;
                }
            }
            s.call_method0("update")?;
            view.setattr("overlay_primitives", PyList::empty(py))?;
            s.call_method0("mesh_feedback")?;
        } else if mode.eq("tape")? {
            tool.call_method1("on_click", (&ctx,))?;
            s.setattr("start_point", start_of(&tool)?)?;
            s.call_method0("measurements")?;
        }
        view.call_method0("update")?;
        Ok(())
    }

    fn hover<'py>(slf: &Bound<'py, Self>, event: &Obj<'py>) -> PyResult<()> {
        let s = slf.as_any();
        let mode = s.call_method0("sync")?;
        let ctx = s.call_method1("context", (event,))?;
        if ctx.is_none() {
            return Ok(());
        }
        s.setattr("hover_point", ctx.getattr("world")?)?;
        let tool = s.getattr("tool")?;
        if mode.eq("mesh")? {
            tool.call_method1("on_hover", (&ctx,))?;
            s.setattr("start_point", start_of(&tool)?)?;
            s.call_method0("mesh_feedback")?;
        } else if mode.eq("tape")? {
            // The band, the guide preview and the length label are painted
            // each frame by draw_annotations, with GL's painters.
            tool.call_method1("on_hover", (&ctx,))?;
            s.call_method0("measurements")?;
        }
        Ok(())
    }

    fn key<'py>(slf: &Bound<'py, Self>, event: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        if !s.getattr("mode")?.eq("mesh")? {
            return false.into_bound_py_any(py);
        }
        s.setattr("_value_buffer", s.getattr("value_buffer")?)?;
        if py.import("views.document_interaction")?.call_method1("_value_key", (s, event))?.is_truthy()? {
            s.setattr("value_buffer", s.getattr("_value_buffer")?)?;
            s.setattr("start_point", start_of(&s.getattr("tool")?)?)?;
            s.getattr("view")?.setattr("overlay_primitives", PyList::empty(py))?;
            s.call_method0("update")?;
            s.call_method0("mesh_feedback")?;
            return true.into_bound_py_any(py);
        }
        s.getattr("tool")?.call_method1("on_key", (s, event.call_method0("key")?, event.call_method0("modifiers")?))
    }

    fn _array_pending<'py>(slf: &Bound<'py, Self>) -> Reply<'py> {
        let s = slf.as_any();
        match s.getattr("tool")?.getattr_opt("array_pending")? {
            Some(pending) => pending.call1((s,)),
            None => false.into_bound_py_any(slf.py()),
        }
    }

    #[pyo3(signature = (committed = false))]
    fn _release_axis_lock_after_operation(slf: &Bound<'_, Self>, committed: bool) -> PyResult<()> {
        let py = slf.py();
        let source = slf.as_any().getattr("view")?.getattr("source_viewport")?;
        if let Some(release) = some(&source, "_release_axis_lock_after_operation")? {
            let kw = PyDict::new(py);
            kw.set_item("committed", committed)?;
            release.call((), Some(&kw))?;
        }
        Ok(())
    }

    fn _set_value_buffer<'py>(slf: &Bound<'py, Self>, value: &Obj<'py>) -> PyResult<()> {
        let s = slf.as_any();
        s.setattr("value_buffer", value)?;
        s.setattr("_value_buffer", value)?;
        if let Some(signal) = some(&s.getattr("view")?.getattr("source_viewport")?, "valueBufferChanged")? {
            signal.call_method1("emit", (value,))?;
        }
        Ok(())
    }

    #[pyo3(signature = (*args, **kwargs))]
    fn _parse_value_buffer<'py>(slf: &Bound<'py, Self>, args: &Bound<'py, PyTuple>, kwargs: Option<&Bound<'py, PyDict>>) -> Reply<'py> {
        interaction(slf.py())?.getattr("_parse_value_buffer")?.call(args.clone(), kwargs)
    }

    fn write_rows<'py>(slf: &Bound<'py, Self>, rows: &Obj<'py>) -> PyResult<()> {
        let py = slf.py();
        let s = slf.as_any();
        let view = s.getattr("view")?;
        let ids = s.getattr("ids")?;
        let renderer = view.getattr("renderer")?;
        if !renderer.is_none() {
            let bytes = rows.call_method1("view", (numpy(py)?.getattr("uint8")?,))?.call_method1("reshape", (-1,))?;
            renderer.call_method1("update_instances", (ids.call_method0("tolist")?, bytes))?;
        }
        let billboards = attr(&view, "_billboards")?;
        if !billboards.is_none() {
            billboards.call_method0("restore")?;
        }
        view.getattr("packed")?.getattr("instances")?.set_item(&ids, rows)?;
        py.import("views.wgpu_parity")?.call_method1("hidden_geometry", (&view,))?;
        if !billboards.is_none() && billboards.getattr("ids")?.len()? != 0 {
            let fresh = py.import("views.wgpu_billboards")?.getattr("Billboards")?.call1((view.getattr("packed")?,))?;
            view.setattr("_billboards", &fresh)?;
            fresh.call_method1("update", (view.getattr("camera")?, view.getattr("renderer")?, view.getattr("inference_index")?))?;
        }
        Ok(())
    }

    fn restore(slf: &Bound<'_, Self>) -> PyResult<()> {
        let s = slf.as_any();
        let original = s.getattr("original")?;
        if !original.is_none() {
            s.call_method1("write_rows", (original,))?;
        }
        Ok(())
    }

    fn cancel(slf: &Bound<'_, Self>) -> PyResult<()> {
        let py = slf.py();
        let s = slf.as_any();
        let view = s.getattr("view")?;
        if let Some(renderer) = some(&view, "renderer")? {
            if renderer.hasattr("set_preview_mesh")? {
                let np = numpy(py)?;
                let empty = np.call_method1("empty", (0, np.getattr("uint8")?))?;
                renderer.call_method1("set_preview_mesh", (&empty, &empty))?;
                // mesh_feedback skips an unchanged preview: this one is gone.
                view.setattr("_preview_key", py.None())?;
            }
        }
        s.call_method0("restore")?;
        let tool = s.getattr("tool")?;
        let mode: String = s.getattr("mode")?.extract()?;
        if mode == "mesh" && !tool.is_none() {
            tool.call_method1("on_cancel", (s,))?;
            s.setattr("_mesh_hover", py.None())?;
            s.call_method0("update")?;
        }
        if tool.is_instance(&py.import("tools.tape")?.getattr("TapeMeasureTool")?)? {
            tool.call_method1("on_cancel", (s,))?;
        }
        s.setattr("original", py.None())?;
        s.setattr("start_point", py.None())?;
        if mode == "move" && !tool.is_none() {
            tool.setattr("start_point", py.None())?;
        }
        if s.getattr("value_buffer")?.is_truthy()? {
            s.call_method1("_set_value_buffer", ("",))?;
        }
        view.setattr("overlay_primitives", PyList::empty(py))
    }

    fn measurements(slf: &Bound<'_, Self>) -> PyResult<()> {
        let py = slf.py();
        let s = slf.as_any();
        let source = s.getattr("view")?.getattr("source_viewport")?;
        let measured = attr(&s.getattr("tool")?, "_measured")?;
        let text = if s.getattr("mode")?.eq("tape")? && s.getattr("start_point")?.is_none() && !measured.is_none() {
            py.import("built_core.units")?.call_method1("fmt_len_fine", (measured,))?
        } else {
            source.call_method0("_measurement_text")?
        };
        source.getattr("measurementChanged")?.call_method1("emit", (text,))?;
        Ok(())
    }

    #[pyo3(signature = (*args))]
    fn flash_status<'py>(slf: &Bound<'py, Self>, args: &Bound<'py, PyTuple>) -> PyResult<()> {
        slf.as_any().getattr("view")?.getattr("source_viewport")?.getattr("flash_status")?.call1(args.clone())?;
        Ok(())
    }

    #[pyo3(name = "mesh_feedback")]
    fn tool_mesh_feedback(slf: &Bound<'_, Self>) -> PyResult<()> {
        let s = slf.as_any();
        if class_name(&s.getattr("tool")?)? == "TexturePositionTool" {
            s.call_method0("measurements")?;
            return Ok(());
        }
        mesh_feedback(s)
    }

    fn update(slf: &Bound<'_, Self>) -> PyResult<()> {
        let py = slf.py();
        let s = slf.as_any();
        let view = s.getattr("view")?;
        if s.getattr("mode")?.eq("mesh")? {
            let tool = s.getattr("tool")?;
            let mut preview = attr(&view, "_texture_preview")?;
            if !preview.is_none() && !attr(&tool, "active")?.is_truthy()? {
                preview.call_method0("restore")?;
            }
            view.call_method1("set_scene", (s.getattr("scene")?,))?;
            if class_name(&tool)? == "TexturePositionTool" {
                if preview.is_none() {
                    preview = py.import("tools.texture_position_metal")?.getattr("TexturePreview")?.call1((&view,))?;
                    view.setattr("_texture_preview", &preview)?;
                }
                preview.call_method1("update", (&tool,))?;
            }
            let revision = s.getattr("history")?.getattr("revision")?;
            let notified = match s.getattr_opt("_notified_revision")? {
                Some(notified) => notified,
                None => revision.clone(),
            };
            if !revision.eq(&notified)? {
                if let Some(notify) = some(&view.getattr("source_viewport")?, "notify_scene_changed")? {
                    let frame = py.import("built_core.coordinate_frame")?.call_method0("model_frame")?;
                    within(&frame, || notify.call0())?;
                }
            }
            s.setattr("_notified_revision", revision)?;
        }
        view.call_method0("update")?;
        Ok(())
    }

    fn pick_edge_any<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let view = s.getattr("view")?;
        let index = attr(&view, "inference_index")?;
        if index.is_none() {
            return Ok(none(py));
        }
        let kw = PyDict::new(py);
        kw.set_item("radius_px", s.getattr("pick_threshold_px")?)?;
        py.import("built_core")?.call_method("reference_edge", (index, view, x, y), Some(&kw))
    }

    /// These shared projection-only pickers never read GL caches.
    fn pick_guide<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        if slf.as_any().getattr("scene")?.is_none() {
            return Ok(none(py));
        }
        interaction(py)?.call_method1("pick_guide", (slf, x, y))
    }
    fn pick_axis<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        if slf.as_any().getattr("scene")?.is_none() {
            return Ok(none(py));
        }
        interaction(py)?.call_method1("pick_axis", (slf, x, y))
    }

    fn width<'py>(slf: &Bound<'py, Self>) -> Reply<'py> {
        slf.as_any().getattr("view")?.call_method0("width")
    }
    fn height<'py>(slf: &Bound<'py, Self>) -> Reply<'py> {
        slf.as_any().getattr("view")?.call_method0("height")
    }

    fn _apply_tool_cursor(slf: &Bound<'_, Self>) -> PyResult<()> {
        let view = slf.as_any().getattr("view")?;
        let source = view.getattr("source_viewport")?;
        // A nav mode (Orbit, Pan, Zoom) has no active tool: asking for the
        // tool cursor then unset the hand the mode had just put on.
        let apply = if attr(&source, "nav_mode")?.is_truthy()? {
            attr(&source, "_apply_nav_cursor")?
        } else {
            attr(&source, "_apply_tool_cursor")?
        };
        if !apply.is_none() {
            apply.call0()?;
            if view.hasattr("setCursor")? {
                view.call_method1("setCursor", (source.call_method0("cursor")?,))?;
            }
        }
        Ok(())
    }

    /// The document controller owns undo, links, locks and edit sharing.
    fn begin_group_edit<'py>(slf: &Bound<'py, Self>, group: &Obj<'py>) -> PyResult<()> {
        slf.as_any().getattr("view")?.getattr("source_viewport")?.call_method1("begin_group_edit", (group,))?;
        Ok(())
    }
    fn end_one_group_edit(slf: &Bound<'_, Self>) -> PyResult<()> {
        slf.as_any().getattr("view")?.getattr("source_viewport")?.call_method0("end_one_group_edit")?;
        Ok(())
    }
    fn set_extension_pick<'py>(slf: &Bound<'py, Self>, hit: &Obj<'py>) -> PyResult<()> {
        slf.as_any().getattr("view")?.getattr("source_viewport")?.call_method1("set_extension_pick", (hit,))?;
        Ok(())
    }
    #[pyo3(signature = (notify = true))]
    fn clear_extension_pick(slf: &Bound<'_, Self>, notify: bool) -> PyResult<()> {
        let kw = PyDict::new(slf.py());
        kw.set_item("notify", notify)?;
        slf.as_any().getattr("view")?.getattr("source_viewport")?.call_method("clear_extension_pick", (), Some(&kw))?;
        Ok(())
    }
    fn edit_group_box_contains_pixel<'py>(slf: &Bound<'py, Self>, x: &Obj<'py>, y: &Obj<'py>) -> Reply<'py> {
        slf.as_any().getattr("view")?.call_method1("edit_group_box_contains_pixel", (x, y))
    }

    /// What this view does not answer is the document controller's.
    fn __getattr__<'py>(slf: &Bound<'py, Self>, name: &str) -> Reply<'py> {
        if name == "view" {
            return Err(PyAttributeError::new_err("view"));
        }
        let view = slf.as_any().getattr("view")?;
        if name == "camera" {
            if let Some(camera) = some(&view, "camera")? {
                return Ok(camera);
            }
        }
        view.getattr("source_viewport")?.getattr(name)
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add_class::<MeshToolAccess>()?;
    m.add_class::<NativeTools>()?;
    m.add_function(wrap_pyfunction!(move_command, m)?)?;
    let dispatch = PyDict::new(py);
    for (tool, mode) in TOOL_DISPATCH {
        dispatch.set_item(tool, mode)?;
    }
    m.add("TOOL_DISPATCH", dispatch)
}
