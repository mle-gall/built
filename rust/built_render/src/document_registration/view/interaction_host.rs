// SPDX-License-Identifier: GPL-3.0-or-later
//! Document event dispatch and the renderer's lazy chunk mapping.
//! The registered controller and offscreen views share these two types.
use super::modules::{class, method, py_super};
use super::*;
use crate::settings_rows::{tr, trf};

fn base<'p>(v: &Obj<'p>) -> Reply<'p> {
    py_super(
        &crate::document_host::host(v.py())?,
        "DocumentInteraction",
        v,
    )
}
#[pyfunction]
#[pyo3(signature=(v,parent=None,**surface))]
fn initialize(v: &Obj, parent: Option<&Obj>, surface: Option<&Bound<PyDict>>) -> PyResult<()> {
    let py = v.py();
    base(v)?.call_method("__init__", (parent,), surface)?;
    for name in ["view_filter", "view_tint", "active_view_key"] {
        v.setattr(name, py.None())?;
    }
    v.call_method0("_configure_surface")?;
    v.call_method1("setMinimumSize", (640, 480))?;
    v.call_method1(
        "setFocusPolicy",
        (py.import("built_core")?
            .getattr("Input")?
            .getattr("StrongFocus")?,),
    )?;
    v.call_method1("setMouseTracking", (true,))?;
    v.call_method0("_init_document")?;
    let renderer = py.import("built_render")?;
    renderer.call_method1("init_document_interaction", (v,))?;
    renderer.call_method1("init_navigation", (v,))?;
    Ok(())
}
#[pyfunction]
fn configure_surface(_v: &Obj) {}
#[pyfunction]
fn offer_link_source(v: &Obj, group: &Obj) -> PyResult<()> {
    let py = v.py();
    let dialogs = py.import("built_render.chrome_dialogs")?;
    let link = py.import("built_core.links")?.call_method1("link_of", (group,))?;
    let path = py
        .import("pathlib")?
        .call_method1("Path", (link.get_item("path")?,))?;
    let name = path.getattr("name")?;
    if !path.call_method0("is_file")?.is_truthy()? {
        dialogs.call_method1(
            "warning",
            (
                v,
                tr(py, "Linked document")?,
                trf(
                    py,
                    "“{name}” is a linked document, and its file was not found:\n{path}",
                    &[("name", name), ("path", path)],
                )?,
            ),
        )?;
        return Ok(());
    }
    let title = trf(
        py,
        "Open the source file “{name}”?",
        &[("name", name.clone())],
    )?;
    let message = tr(
        py,
        "It is edited in its own document; every change saved there shows here.",
    )?;
    let options = [
        ("cancel", tr(py, "Cancel")?),
        ("open", trf(py, "Open {name}", &[("name", name)])?),
    ];
    let kwargs = fields!(py;"default"=>"open","cancel"=>"cancel");
    if dialogs
        .getattr("ask")?
        .call((v, title, message, options), Some(&kwargs))?
        .eq("open")?
    {
        if let Some(window) = v.getattr_opt("window")?.filter(Obj::is_callable) {
            let win = window.call0()?;
            if win
                .getattr_opt("open_path")?
                .is_some_and(|f| f.is_callable())
            {
                super::calls::open_link_source(&win, &path, group)?;
            }
        }
    }
    Ok(())
}
fn handle<'p>(
    v: &Obj<'p>,
    ev: &Obj<'p>,
    handler: &str,
    parent: &str,
    handled: Obj<'p>,
) -> Reply<'p> {
    if v.py()
        .import("built_render")?
        .call_method1(handler, (v, ev))?
        .is_truthy()?
    {
        Ok(handled)
    } else {
        base(v)?.call_method1(parent, (ev,))
    }
}
#[pyfunction]
fn event<'p>(v: &Obj<'p>, ev: &Obj<'p>) -> Reply<'p> {
    handle(
        v,
        ev,
        "document_event",
        "event",
        true.into_bound_py_any(v.py())?,
    )
}
#[pyfunction]
fn key_press<'p>(v: &Obj<'p>, ev: &Obj<'p>) -> Reply<'p> {
    handle(v, ev, "document_key_press", "keyPressEvent", none(v.py()))
}
#[pyfunction]
fn key_release<'p>(v: &Obj<'p>, ev: &Obj<'p>) -> Reply<'p> {
    handle(
        v,
        ev,
        "document_key_release",
        "keyReleaseEvent",
        none(v.py()),
    )
}
#[pyfunction]
#[pyo3(signature=(v,*args))]
fn set_focus(v: &Obj, args: &Bound<PyTuple>) -> PyResult<()> {
    if !v
        .py()
        .import("built_render")?
        .call_method1("document_focus", (v, args))?
        .is_truthy()?
    {
        base(v)?.call_method1("setFocus", args)?;
    }
    Ok(())
}
#[pyfunction]
fn focus_next(_v: &Obj, _next: bool) -> bool {
    false
}
#[pyfunction]
fn focus_out(v: &Obj, ev: &Obj) -> PyResult<()> {
    v.py()
        .import("built_render")?
        .call_method1("document_focus_out", (v,))?;
    base(v)?.call_method1("focusOutEvent", (ev,))?;
    Ok(())
}
#[pyfunction]
#[pyo3(signature=(tag,ms,extra="",floor=50.))]
fn plog(py: Python, tag: &str, ms: f64, extra: &str, floor: f64) -> PyResult<()> {
    if ms >= floor
        && crate::document_host::host(py)?
            .getattr("_PERF")?
            .is_truthy()?
    {
        py.import("built_core.history")?
            .call_method1("_plog", (tag, ms, extra, floor))?;
    }
    Ok(())
}

#[pyfunction]
fn chunk_init(chunk: &Obj, data: &Obj, later: &Obj) -> PyResult<()> {
    chunk
        .py()
        .get_type::<PyDict>()
        .call_method1("__init__", (chunk, data))?;
    chunk.setattr("_later", later)
}
#[pyfunction]
fn chunk_set(chunk: &Obj, key: &Obj, value: &Obj) -> PyResult<()> {
    chunk
        .getattr("_later")?
        .call_method1("pop", (key, chunk.py().None()))?;
    chunk.cast::<PyDict>()?.set_item(key, value)
}
#[pyfunction]
fn chunk_contains(chunk: &Obj, key: &Obj) -> PyResult<bool> {
    Ok(chunk.cast::<PyDict>()?.contains(key)? || chunk.getattr("_later")?.contains(key)?)
}
#[pyfunction]
#[pyo3(signature=(chunk,key,default=None))]
fn chunk_get<'p>(chunk: &Obj<'p>, key: &Obj<'p>, default: Option<Obj<'p>>) -> Reply<'p> {
    if chunk_contains(chunk, key)? {
        chunk.get_item(key)
    } else {
        Ok(default.unwrap_or_else(|| none(chunk.py())))
    }
}
fn ready<'p>(chunk: &Obj<'p>, method: &str) -> Reply<'p> {
    crate::document_records::lazy_chunk_fill(chunk)?;
    chunk
        .py()
        .get_type::<PyDict>()
        .call_method1(method, (chunk,))
}
#[pyfunction]
fn chunk_keys<'p>(chunk: &Obj<'p>) -> Reply<'p> {
    ready(chunk, "keys")
}
#[pyfunction]
fn chunk_items<'p>(chunk: &Obj<'p>) -> Reply<'p> {
    ready(chunk, "items")
}
#[pyfunction]
fn chunk_values<'p>(chunk: &Obj<'p>) -> Reply<'p> {
    ready(chunk, "values")
}
#[pyfunction]
fn chunk_iter<'p>(chunk: &Obj<'p>) -> Reply<'p> {
    ready(chunk, "__iter__")
}
#[pyfunction]
fn chunk_len<'p>(chunk: &Obj<'p>) -> Reply<'p> {
    ready(chunk, "__len__")
}
#[pyfunction]
fn chunk_copy<'p>(chunk: &Obj<'p>) -> Reply<'p> {
    chunk
        .py()
        .get_type::<PyDict>()
        .call1((chunk_items(chunk)?,))
}

macro_rules! methods{
    ($module:expr,$owner:expr,$($name:literal => $fun:ident),+ $(,)?)=>{{
        $(method($owner,$name,&wrap_pyfunction!($fun,$module)?.into_any())?;)+
    }};
}
fn lazy_chunk(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    let cls = class(
        m,
        "_LazyChunk",
        &[py.get_type::<PyDict>().into_any()],
        &fields!(py;"__slots__"=>("_later",)),
    )?;
    methods!(m,&cls,"__init__"=>chunk_init,"__setitem__"=>chunk_set,"__contains__"=>chunk_contains,"get"=>chunk_get,
        "keys"=>chunk_keys,"items"=>chunk_items,"values"=>chunk_values,"__iter__"=>chunk_iter,"__len__"=>chunk_len,"copy"=>chunk_copy);
    method(
        &cls,
        "__missing__",
        &py.import("built_render")?.getattr("lazy_chunk_value")?,
    )?;
    method(
        &cls,
        "_made",
        &py.import("built_render")?.getattr("lazy_chunk_fill")?,
    )?;
    Ok(())
}
pub(super) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add(
        "annotations",
        py.import("__future__")?.getattr("annotations")?,
    )?;
    dependencies(m)?;
    register_document_module(&m.dict())?;
    let engine = py.import("built_render")?;
    m.add(
        "_HoverEvent",
        engine.getattr("MouseEvent")?.getattr("hover")?,
    )?;
    m.add(
        "_chunk_uid",
        py.import("itertools")?.call_method1("count", (1,))?,
    )?;
    m.add("_BAKE_STORES", PyDict::new(py))?;
    m.add(
        "_LIVE_VIEWS",
        py.import("weakref")?.call_method0("WeakSet")?,
    )?;
    m.add("_DECODED_TEXTURES", PyDict::new(py))?;
    m.add(
        "_QV_HYPOT3",
        py.import("core._native")?.call_method0("qt_hypot")?,
    )?;
    m.add("_plog", wrap_pyfunction!(plog, m)?)?;
    lazy_chunk(m)?;
    let attrs = PyDict::new(py);
    let slot = m.getattr("CallbackSlot")?;
    let builtins = py.import("builtins")?;
    for name in ["valueBufferChanged", "sceneVersionChanged", "measurementChanged",
                 "tilesChanged", "coordinateChanged", "projectionChanged", "smartZoomRequested"] {
        attrs.set_item(name, slot.call0()?)?;
    }
    attrs.set_item("DEFAULT_FACE_COLOR", m.getattr("DEFAULT_FACE_COLOR")?)?;
    attrs.set_item(
        "_LIGHT",
        m.getattr("QVector3D")?
            .call1((0.35, 0.25, 1.))?
            .call_method0("normalized")?,
    )?;
    attrs.set_item("_bind_bake_store", m.getattr("_bind_bake_store")?)?;
    let input = m.getattr("Qt")?;
    let keys = [
        "Key_Control",
        "Key_Shift",
        "Key_Alt",
        "Key_Meta",
        "Key_AltGr",
    ]
    .map(|k| input.getattr(k))
    .into_iter()
    .collect::<PyResult<Vec<_>>>()?;
    attrs.set_item(
        "_MODIFIER_KEYS",
        builtins.call_method1("frozenset", (keys,))?,
    )?;
    attrs.set_item("extension_pick", py.None())?;
    attrs.set_item("keys_stand_in", py.None())?;
    let cls = class(
        m,
        "DocumentInteraction",
        &[m.getattr("DocumentTools")?, m.getattr("ViewNavigation")?],
        &attrs,
    )?;
    methods!(m,&cls,"__init__"=>initialize,"_configure_surface"=>configure_surface,"_offer_link_source"=>offer_link_source,
       "event"=>event,"keyPressEvent"=>key_press,"keyReleaseEvent"=>key_release,"setFocus"=>set_focus,
       "focusNextPrevChild"=>focus_next,"focusOutEvent"=>focus_out);
    register_document_host(&cls)?;
    register_view_class(&cls)?;
    register_view_functions(&m.dict())
}

fn dependencies(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("built_core", py.import("built_core")?)?;
    m.add("copy", py.import("copy")?)?;
    m.add("importlib", py.import("importlib")?)?;
    m.add("itertools", py.import("itertools")?)?;
    m.add("math", py.import("math")?)?;
    m.add("os", py.import("os")?)?;
    m.add("sys", py.import("sys")?)?;
    m.add("_time_mod", py.import("time")?)?;
    m.add("types", py.import("types")?)?;
    m.add("np", py.import("numpy")?)?;
    m.add("built_render", py.import("built_render")?)?;
    m.add("_layers", py.import("built_core.layers")?)?;
    m.add("weakref", py.import("weakref")?)?;
    let groups: &[(&str, &[(&str, &str)])] = &[
        (
            "built_core.mesh",
            &[
                ("element_identity", "element_identity"),
                ("Edge", "Edge"),
                ("Face", "Face"),
                ("smooths", "smooths"),
            ],
        ),
        ("built_core", &[("ElementSet", "ElementSet")]),
        ("array", &[("array", "array")]),
        ("pathlib", &[("Path", "Path")]),
        ("typing", &[("Optional", "Optional")]),
        (
            "built_core.units",
            &[
                ("fmt_len", "_fmt_len"),
                ("typed_value_text", "_typed_value_text"),
            ],
        ),
        (
            "built_core",
            &[
                ("Type", "QEvent"),
                ("Input", "Qt"),
                ("ScreenPoint", "QPointF"),
                ("ScreenRect", "QRectF"),
                ("ray_aabb_span", "_ray_aabb_span"),
                ("Matrix4", "QMatrix4x4"),
                ("Vec3", "QVector3D"),
                ("Vec4", "QVector4D"),
                ("point_in_screen_hull", "_point_in_screen_hull"),
                ("MIN_CIRCLE_DASHES", "_MIN_CIRCLE_DASHES"),
            ],
        ),
        ("built_render", &[("CallbackSlot", "CallbackSlot")]),
        (
            "built_core.timers",
            &[("later", "later"), ("timer", "window_timer")],
        ),
        (
            "built_render",
            &[
                ("AnnotationFont", "QFont"),
                ("PaintColor", "QColor"),
                ("PaintPen", "QPen"),
                ("document_value_key", "_value_key"),
            ],
        ),
        ("built_core.camera", &[("OrbitCamera", "OrbitCamera")]),
        ("built_core.i18n", &[("tr", "tr")]),
        (
            "built_core.group",
            &[
                ("Group", "Group"),
                ("PlacementProxy", "PlacementProxy"),
                ("copy_group", "copy_group"),
                ("effective_material", "effective_material"),
                ("world_mesh", "world_mesh"),
            ],
        ),
        (
            "built_core.history",
            &[
                ("EraseSelectionCommand", "EraseSelectionCommand"),
                ("History", "History"),
            ],
        ),
        ("built_core.scene", &[("Scene", "Scene")]),
        (
            "built_core.style",
            &[
                ("DEFAULT_BACK_COLOR", "DEFAULT_BACK_COLOR"),
                ("DEFAULT_FACE_COLOR", "DEFAULT_FACE_COLOR"),
                ("MONOCHROME_GLASS_RGB", "MONOCHROME_GLASS_RGB"),
                ("effective_back_color", "effective_back_color"),
            ],
        ),
        (
            "built_core.materials",
            &[
                ("effective_attrs", "effective_attrs"),
                ("material_sig", "_material_sig"),
            ],
        ),
        (
            "built_core.snap",
            &[
                ("SnapResult", "SnapResult"),
                ("_AXIS_VECTORS", "_AXIS_VECTORS"),
            ],
        ),
        ("built_core.texture", &[("face_uv_axes", "face_uv_axes")]),
        ("built_core.triangulate", &[("plane_axes", "plane_axes")]),
        (
            "tools.base",
            &[("Tool", "Tool"), ("ToolContext", "ToolContext")],
        ),
        ("built_core.inference", &[("InferenceEngine", "InferenceEngine")]),
        ("core._native", &[("qt_hypot", "qt_hypot")]),
        (
            "views.document_tools",
            &[("DocumentTools", "DocumentTools")],
        ),
        (
            "views.view_navigation",
            &[("ViewNavigation", "ViewNavigation")],
        ),
    ];
    for (source, names) in groups {
        super::modules::import_names(m, source, names)?;
    }
    Ok(())
}
