//! Commands shared by the native window and the remaining Classic adapter.
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};

const TOOLS: &[(&str, &str, &str)] = &[
    ("select", "tools.rust_tools", "SelectTool"),
    ("cut", "tools.rust_tools", "CutTool"),
    ("sweep", "tools.rust_tools", "SweepTool"),
    ("line", "tools.rust_tools", "LineTool"),
    ("line_on_surface", "tools.rust_tools", "LineOnSurfaceTool"),
    ("freehand", "tools.freehand", "FreehandTool"),
    ("rectangle", "tools.rust_tools", "RectangleTool"),
    (
        "rotated_rect",
        "tools.rotated_rectangle",
        "RotatedRectangleTool",
    ),
    ("circle", "tools.circle", "CircleTool"),
    ("polygon", "tools.circle", "PolygonTool"),
    ("arc", "tools.arc", "ArcTool"),
    ("arc3", "tools.arc", "ThreePointArcTool"),
    ("center_arc", "tools.arc", "CenterArcTool"),
    ("pie", "tools.arc", "PieTool"),
    ("pushpull", "tools.pushpull", "PushPullTool"),
    ("offset", "tools.offset", "OffsetTool"),
    ("move", "tools.move", "MoveTool"),
    ("rotate", "tools.rotate", "RotateTool"),
    ("scale", "tools.scale", "ScaleTool"),
    ("flip", "tools.flip", "FlipTool"),
    ("followme", "tools.followme", "FollowMeTool"),
    ("fillet", "tools.fillet", "FilletTool"),
    ("chamfer", "tools.fillet", "ChamferTool"),
    (
        "texture_position",
        "tools.texture_position",
        "TexturePositionTool",
    ),
    ("paint", "tools.paint", "PaintTool"),
    ("dimension", "tools.dimension", "DimensionTool"),
    ("eraser", "tools.eraser", "EraserTool"),
    ("tape", "tools.tape", "TapeMeasureTool"),
    ("protractor", "tools.protractor", "ProtractorTool"),
    ("north", "tools.north", "NorthTool"),
    ("text", "tools.text", "TextTool"),
    ("geopath", "tools.geopath", "GeoPathTool"),
    ("section", "tools.section", "SectionPlaneTool"),
    ("position_camera", "tools.walkthrough", "PositionCameraTool"),
    ("walk", "tools.walkthrough", "WalkTool"),
    ("look_around", "tools.walkthrough", "LookAroundTool"),
    ("first_person", "tools.walkthrough", "FirstPersonTool"),
];
// These controllers are registered after the solid-tool family in a window.
const EXTRA_TOOLS: &[(&str, &str, &str)] = &[
    ("change_axes", "tools.change_axes", "ChangeAxesTool"),
    ("scale_manager", "tools.scale_manager", "ScaleManagerTool"),
];

// These tools are constructed with a clipboard or placement at activation.
const TRANSIENT_TOOLS: &[(&str, &str, &str)] = &[
    ("paste", "tools.paste", "PasteTool"),
    ("place_group", "tools.place_group", "PlaceGroupTool"),
];

#[pyfunction]
pub(super) fn transient_tool_class<'py>(py: Python<'py>, key: &str) -> PyResult<Bound<'py, PyAny>> {
    let (_, module, name) = TRANSIENT_TOOLS
        .iter()
        .find(|(k, _, _)| *k == key)
        .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(key.to_owned()))?;
    py.import(*module)?.getattr(*name)
}

/// Let the coverage audit read the same fixed entries without constructing
/// tools or opening a document. Solid tools publish their own ordered family.
#[pyfunction]
fn window_tool_types() -> Vec<(&'static str, &'static str)> {
    TOOLS
        .iter()
        .chain(EXTRA_TOOLS)
        .chain(TRANSIENT_TOOLS)
        .map(|&(key, _, name)| (key, name))
        .collect()
}

#[pyfunction]
pub fn build_tools(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    let tools = PyDict::new(py);
    let add = |key: &str, class: Bound<'_, PyAny>| -> PyResult<()> {
        let instance = class.call0()?;
        if !instance
            .getattr("icon")
            .is_ok_and(|v| v.is_truthy().unwrap_or(false))
        {
            instance.setattr("icon", key)?;
        }
        tools.set_item(key, instance)
    };
    for &(key, module, name) in TOOLS {
        add(key, py.import(module)?.getattr(name)?)?;
    }
    for pair in py
        .import("tools.solid_tools")?
        .getattr("SOLID_TOOLS")?
        .try_iter()?
    {
        let (key, class): (String, Bound<PyAny>) = pair?.extract()?;
        add(&key, class)?;
    }
    for &(key, module, name) in EXTRA_TOOLS {
        add(key, py.import(module)?.getattr(name)?)?;
    }
    Ok(tools)
}

/// Restoring an owned viewport dismisses its tab's replacement even when
/// another tab is active. An independent extension view belongs to the front tab.
#[pyfunction]
fn set_front_view(win: &Bound<'_, PyAny>, view: &Bound<'_, PyAny>) -> PyResult<()> {
    for doc in win.getattr("_documents")?.try_iter()? {
        let doc = doc?;
        if doc.getattr("viewport")?.is(view) {
            return doc.setattr("front_view", win.py().None());
        }
    }
    win.getattr("_doc")?.setattr("front_view", view)
}

#[pyfunction]
pub fn document_blank(doc: &Bound<'_, PyAny>) -> PyResult<bool> {
    let vp = doc.getattr("viewport")?;
    if !doc.getattr("path")?.is_none()
        || doc.getattr("import_name")?.is_truthy()?
        || vp.is_none()
        || !doc.getattr("front_view")?.is_none()
    {
        return Ok(false);
    }
    let history = vp.getattr("history")?;
    Ok(!history.getattr("undo_stack")?.is_truthy()?
        && !history.getattr("redo_stack")?.is_truthy()?
        && vp
            .getattr("scene")?
            .getattr("content_version")?
            .eq(doc.getattr("saved_content")?)?)
}

#[pyfunction]
pub fn untitled_slot(docs: &Bound<'_, PyAny>) -> PyResult<u64> {
    let mut used = std::collections::HashSet::new();
    for doc in docs.try_iter()? {
        if let Some(slot) = doc?.getattr("untitled_no")?.extract::<Option<u64>>()? {
            used.insert(slot);
        }
    }
    let py = docs.py();
    let pending = py.import("built_core.autosave")?.getattr("pending")?;
    let kw = PyDict::new(py);
    for slot in 1.. {
        if used.contains(&slot) {
            continue;
        }
        kw.set_item("untitled", slot)?;
        if !pending.call((py.None(),), Some(&kw))?.is_truthy()? {
            return Ok(slot);
        }
    }
    unreachable!()
}

#[pyfunction]
pub fn document_dirty(win: &Bound<'_, PyAny>) -> PyResult<bool> {
    let workspace = win.getattr("_workspace")?;
    if !workspace.is_none() {
        return workspace.call_method0("is_dirty")?.is_truthy();
    }
    win.getattr("_doc")?
        .getattr("viewport")?
        .getattr("scene")?
        .getattr("content_version")?
        .ne(win.getattr("_saved_content")?)
}

#[pyfunction]
pub fn saved_content(doc: &Bound<'_, PyAny>, version: i64) -> PyResult<i64> {
    let view_version = if version >= 0 {
        doc.getattr("viewport")
            .and_then(|vp| vp.getattr("scene"))
            .and_then(|s| s.getattr("view_version"))
            .map_or(Ok(0), |v| v.extract())?
    } else {
        0
    };
    Ok(version - view_version)
}

pub(super) const CAMERA_FIELDS: [&str; 6] = [
    "distance",
    "yaw",
    "pitch",
    "fov_deg",
    "perspective",
    "two_point",
];

#[pyfunction]
pub fn camera_record(vp: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = vp.py();
    let cam = vp.getattr("camera")?;
    if !cam.call_method0("is_finite")?.is_truthy()? {
        return Ok(vp
            .getattr("scene")?
            .getattr("camera_home")
            .map_or_else(|_| py.None(), Bound::unbind));
    }
    let home = PyDict::new(py);
    let target = coords(&cam.getattr("target")?)?;
    home.set_item("target", target.to_vec())?;
    let up = coords(&cam.getattr("up")?)?;
    if up != [0., 0., 1.] {
        home.set_item("up", up.to_vec())?;
    }
    for key in CAMERA_FIELDS {
        home.set_item(key, cam.getattr(key)?)?;
    }
    let lens = cam.call_method0("lens_dict")?;
    if lens.is_truthy()? {
        home.set_item("lens", lens)?;
    }
    Ok(home.into_any().unbind())
}

#[pyfunction]
pub fn apply_camera_record(vp: &Bound<'_, PyAny>, record: &Bound<'_, PyAny>) -> PyResult<()> {
    let Ok(home) = record.cast::<PyDict>() else {
        return Ok(());
    };
    let cam = vp.getattr("camera")?;
    let triple = |v: &Bound<PyAny>| {
        (v.is_instance_of::<PyTuple>() || v.is_instance_of::<PyList>())
            && v.len().is_ok_and(|n| n == 3)
    };
    if let Some(t) = home.get_item("target")?.filter(triple) {
        let float = vp.py().import("builtins")?.getattr("float")?;
        let xyz = t
            .try_iter()?
            .map(|x| float.call1((x?,)))
            .collect::<PyResult<Vec<_>>>()?;
        cam.setattr("target", PyTuple::new(vp.py(), xyz)?)?;
    }
    cam.setattr("up", (0., 0., 1.))?;
    if let Some(u) = home.get_item("up")?.filter(triple) {
        if let Ok(xyz) = u.extract::<[f64; 3]>() {
            if xyz.iter().all(|n| n.is_finite()) && xyz.iter().any(|n| *n != 0.) {
                cam.setattr("up", u)?;
            }
        }
    }
    for key in CAMERA_FIELDS {
        if let Some(v) = home.get_item(key)? {
            let result = cam
                .getattr(key)?
                .get_type()
                .call1((v,))
                .and_then(|v| cam.setattr(key, v));
            if let Err(e) = result {
                if !e.is_instance_of::<pyo3::exceptions::PyValueError>(vp.py())
                    && !e.is_instance_of::<pyo3::exceptions::PyTypeError>(vp.py())
                {
                    return Err(e);
                }
            }
        }
    }
    cam.call_method1("apply_lens", (home.get_item("lens")?,))?;
    vp.call_method0("update")?;
    Ok(())
}
fn coords(p: &Bound<'_, PyAny>) -> PyResult<[f64; 3]> {
    Ok([
        p.call_method0("x")?.extract()?,
        p.call_method0("y")?.extract()?,
        p.call_method0("z")?.extract()?,
    ])
}

fn unlinks(
    command: &Bound<'_, PyAny>,
    compound: &Bound<'_, PyAny>,
    unlink: &Bound<'_, PyAny>,
) -> PyResult<bool> {
    if command.is_instance(compound)? {
        for sub in command.getattr("commands")?.try_iter()? {
            if unlinks(&sub?, compound, unlink)? {
                return Ok(true);
            }
        }
    }
    command.is_instance(unlink)
}

#[pyfunction]
pub fn history_step(win: &Bound<'_, PyAny>, redo: bool) -> PyResult<()> {
    // The Sketch workspace steps through its own edits until Done.
    if crate::sketch_workspace::command(win, if redo { "redo" } else { "undo" })? {
        return Ok(());
    }
    let vp = win.getattr("viewport")?;
    let tool = vp.getattr("active_tool")?;
    let handler = tool
        .getattr(if redo { "on_redo" } else { "on_undo" })
        .ok()
        .filter(|h| h.is_callable());
    let handled = if let Some(h) = handler {
        h.call1((&vp,))?.is_truthy()?
    } else {
        false
    };
    if !handled
        && !vp
            .getattr("history")?
            .call_method0(if redo { "redo" } else { "undo" })?
            .is_truthy()?
    {
        return Ok(());
    }
    if let Ok(inference) = vp.getattr("inference") {
        if !inference.is_none() {
            inference.call_method0("clear_references")?;
        }
    }
    vp.call_method0("notify_scene_changed")?;
    if !handled {
        let stack =
            vp.getattr("history")?
                .getattr(if redo { "undo_stack" } else { "redo_stack" })?;
        if stack.is_truthy()? {
            let history = win.py().import("built_core.history")?;
            if unlinks(
                &stack.get_item(-1)?,
                &history.getattr("CompoundCommand")?,
                &history.getattr("UnlinkCommand")?,
            )? {
                win.call_method0("_unlinked")?;
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Bounds {
    lo: [f64; 3],
    hi: [f64; 3],
}
impl Bounds {
    fn extend(bounds: &mut Option<Self>, other: Self) {
        if let Some(current) = bounds {
            for i in 0..3 {
                current.lo[i] = current.lo[i].min(other.lo[i]);
                current.hi[i] = current.hi[i].max(other.hi[i]);
            }
        } else {
            *bounds = Some(other);
        }
    }
    fn from_points(pair: &Bound<'_, PyAny>) -> PyResult<Option<Self>> {
        let lo = pair.get_item(0)?;
        if lo.is_none() {
            return Ok(None);
        }
        Ok(Some(Self {
            lo: coords(&lo)?,
            hi: coords(&pair.get_item(1)?)?,
        }))
    }
    fn python(py: Python<'_>, bounds: Option<Self>) -> PyResult<Py<PyAny>> {
        let Some(b) = bounds else {
            return Ok(py.None());
        };
        let vec = py.import("built_core")?.getattr("Vec3")?;
        let point = |p: [f64; 3]| vec.call1((p[0], p[1], p[2]));
        Ok((point(b.lo)?, point(b.hi)?)
            .into_pyobject(py)?
            .into_any()
            .unbind())
    }
}

#[pyfunction]
pub fn figure_bounds(scene: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = scene.py();
    let placement = py.import("built_core.group")?.getattr("placement_points")?;
    let marked = py.import("built_core.scale_figure")?.getattr("is_marked")?;
    let limit: f64 = py
        .import("built_core.coordinate_frame")?
        .getattr("REBASE_DISTANCE")?
        .extract()?;
    let mut result = None;
    for g in scene.getattr("groups")?.try_iter()? {
        let g = g?;
        if !g
            .getattr("billboard")
            .is_ok_and(|v| v.is_truthy().unwrap_or(false))
            || !scene.call_method1("entity_visible", (&g,))?.is_truthy()?
        {
            continue;
        }
        let pts = placement.call1((&g,))?;
        if pts.len()? == 0 {
            continue;
        }
        let b = Bounds {
            lo: pts.call_method1("min", (0,))?.extract()?,
            hi: pts.call_method1("max", (0,))?.extract()?,
        };
        if marked.call1((g,))?.is_truthy()? {
            let model = Bounds::from_points(&scene.call_method0("bounds")?)?;
            if model.is_some_and(|m| {
                (0..3).any(|i| m.lo[i] - b.hi[i] > limit || b.lo[i] - m.hi[i] > limit)
            }) {
                continue;
            }
        }
        Bounds::extend(&mut result, b);
    }
    Bounds::python(py, result)
}

#[pyfunction]
pub fn image_bounds(scene: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let mut result = None;
    if let Some(images) = scene.getattr("image_planes").ok().filter(|v| !v.is_none()) {
        for im in images.try_iter()? {
            let im = im?;
            if scene.call_method1("entity_visible", (&im,))?.is_truthy()? {
                for p in im.call_method0("corners")?.try_iter()? {
                    let xyz = coords(&p?)?;
                    Bounds::extend(&mut result, Bounds { lo: xyz, hi: xyz });
                }
            }
        }
    }
    Bounds::python(scene.py(), result)
}

#[pyfunction]
pub fn extents_bounds(win: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let mut result = Bounds::from_points(&scene.call_method0("bounds")?)?;
    for method in ["_figure_bounds", "_image_bounds"] {
        let extra = win.call_method0(method)?;
        if !extra.is_none() {
            if let Some(b) = Bounds::from_points(&extra)? {
                Bounds::extend(&mut result, b);
            }
        }
    }
    if result.is_none() {
        if let Some(survey) = scene.getattr("photo_mesh").ok().filter(|s| !s.is_none()) {
            if survey.getattr("visible")?.is_truthy()? {
                result = Bounds::from_points(&survey.call_method0("bounds")?)?;
            }
        }
    }
    Bounds::python(win.py(), result)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add(
        "DOCUMENT_CAMERA_FIELDS",
        PyTuple::new(m.py(), CAMERA_FIELDS)?,
    )?;
    m.add_function(wrap_pyfunction!(build_tools, m)?)?;
    m.add_function(wrap_pyfunction!(window_tool_types, m)?)?;
    m.add_function(wrap_pyfunction!(transient_tool_class, m)?)?;
    m.add_function(wrap_pyfunction!(set_front_view, m)?)?;
    m.add_function(wrap_pyfunction!(document_blank, m)?)?;
    m.add_function(wrap_pyfunction!(document_dirty, m)?)?;
    m.add_function(wrap_pyfunction!(saved_content, m)?)?;
    m.add_function(wrap_pyfunction!(untitled_slot, m)?)?;
    m.add_function(wrap_pyfunction!(camera_record, m)?)?;
    m.add_function(wrap_pyfunction!(apply_camera_record, m)?)?;
    m.add_function(wrap_pyfunction!(history_step, m)?)?;
    m.add_function(wrap_pyfunction!(figure_bounds, m)?)?;
    m.add_function(wrap_pyfunction!(image_bounds, m)?)?;
    m.add_function(wrap_pyfunction!(extents_bounds, m)?)?;
    Ok(())
}
