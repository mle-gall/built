//! Render a saved sheet view without changing the window's camera or scene.
use pyo3::{
    prelude::*,
    types::{PyBool, PyDict, PyList, PyTuple},
};
mod export;
mod geometry;
mod records;
type Obj<'py> = Bound<'py, PyAny>;
fn opt<'py>(o: &Obj<'py>, key: &str) -> Option<Obj<'py>> {
    o.getattr(key).ok().filter(|v| !v.is_none())
}
fn flag(o: &Obj<'_>, key: &str, default: bool) -> PyResult<bool> {
    o.getattr(key).map_or(Ok(default), |v| v.is_truthy())
}
fn number(o: &Obj<'_>, key: &str) -> PyResult<f64> {
    o.getattr(key)?.extract()
}
fn core(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("ingetrazo_core")
}
fn list<'py>(o: &Obj<'py>) -> PyResult<Obj<'py>> {
    o.py().get_type::<PyList>().call1((o,))
}
fn as_tuple<'py>(o: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    Ok(o.py().get_type::<PyTuple>().call1((o,))?.cast_into()?)
}
fn partial<'py>(py: Python<'py>, f: &Obj<'py>, mut args: Vec<Obj<'py>>) -> PyResult<Obj<'py>> {
    args.insert(0, f.clone());
    py.import("functools")?
        .call_method1("partial", PyTuple::new(py, args)?)
}
fn this(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("ingetrazo_render.sheet_frames")
}
fn absent(view: &Obj<'_>) -> bool {
    view.is(PyBool::new(view.py(), false))
}
#[pyclass(dict, module = "ingetrazo_render.sheet_frames")]
struct FrameScene;

#[pymethods]
impl FrameScene {
    #[new]
    #[pyo3(signature=(scene,**over))]
    fn new(scene: &Obj<'_>, over: Option<&Bound<'_, PyDict>>) -> Self {
        let _ = (scene, over);
        Self
    }

    #[pyo3(signature=(scene,**over))]
    fn __init__(
        slf: &Bound<'_, Self>,
        scene: &Obj<'_>,
        over: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let attrs = slf.getattr("__dict__")?;
        attrs.set_item("_scene", scene)?;
        if let Some(values) = over {
            attrs.call_method1("update", (values,))?;
        }
        Ok(())
    }
    fn __getattr__<'py>(slf: &Bound<'py, Self>, name: &str) -> PyResult<Obj<'py>> {
        slf.getattr("__dict__")?.get_item("_scene")?.getattr(name)
    }
    fn __setattr__(&self, name: &str, _value: &Obj<'_>) -> PyResult<()> {
        Err(pyo3::exceptions::PyAttributeError::new_err(format!(
            "a sheet frame does not change the scene ({name})"
        )))
    }
}
#[pyclass]
struct Shown {
    source: Py<PyAny>,
    #[pyo3(get)]
    visible: bool,
}
#[pymethods]
impl Shown {
    fn __getattr__<'py>(&self, py: Python<'py>, name: &str) -> PyResult<Obj<'py>> {
        self.source.bind(py).getattr(name)
    }
}
#[pyfunction]
fn constant<'py>(value: &Obj<'py>) -> Obj<'py> {
    value.clone()
}
#[pyfunction]
fn noop() {}
#[pyfunction]
fn matrix_copy<'py>(matrix: &Obj<'py>) -> PyResult<Obj<'py>> {
    core(matrix.py())?
        .call_method1("Matrix4", (list(&matrix.call_method0("data")?)?,))?
        .call_method0("transposed")
}
#[pyfunction]
fn saved_view<'py>(scene: &Obj<'py>, frame: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = scene.py();
    let key = opt(frame, "view_key")
        .and_then(|v| v.extract::<String>().ok())
        .unwrap_or_default();
    let Some(name) = key.strip_prefix("scene:") else {
        return Ok(py.None().into_bound(py));
    };
    if let Some(views) = opt(scene, "saved_views") {
        for v in views.try_iter()? {
            let v = v?;
            if v.getattr("name")?.eq(name)? {
                return Ok(v);
            }
        }
    }
    Ok(PyBool::new(py, false).to_owned().into_any())
}
#[pyfunction]
fn frame_scene<'py>(scene: &Obj<'py>, frame: &Obj<'py>, view: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = scene.py();
    let over = PyDict::new(py);
    over.set_item("edit_group", py.None())?;
    over.set_item("_bounds_cache", py.None())?;
    if flag(scene, "image_planes", false)? || flag(scene, "photo_mesh", false)? {
        over.set_item(
            "hidden_reference_tags",
            core(py)?.call_method1("sheet_frame_hidden_tags", (scene.getattr("_root")?, view))?,
        )?;
    }
    if let Some(style) = opt(view, "style").filter(|v| v.is_truthy().unwrap_or(false)) {
        over.set_item(
            "display_style",
            py.import("core.style")?
                .getattr("Style")?
                .call_method1("from_dict", (style,))?,
        )?;
    }
    if let Some(section) = opt(view, "section") {
        let uid = section.call_method1("get", ("active",))?;
        let mut target = py.None().into_bound(py);
        if let Some(planes) = opt(scene, "section_planes") {
            for plane in planes.try_iter()? {
                let plane = plane?;
                if plane.getattr("uid")?.eq(&uid)? {
                    target = plane;
                    break;
                }
            }
        }
        over.set_item(
            "active_section",
            partial(py, &this(py)?.getattr("constant")?, vec![target])?,
        )?;
        for (key, field) in [
            ("planes_shown", "show_section_planes"),
            ("cuts_shown", "show_section_cuts"),
        ] {
            over.set_item(
                field,
                section.call_method1("get", (key, true))?.is_truthy()?,
            )?;
        }
    }
    if let Some(geo) = opt(view, "georef") {
        for row in core(py)?
            .call_method1("saved_view_georef_objects", (scene,))?
            .try_iter()?
        {
            let (key, obj): (String, Obj) = row?.extract()?;
            if obj.is_none() || !geo.contains(&key)? {
                continue;
            }
            let field = match key.as_str() {
                "map" => "tile_layer",
                "terrain" => "terrain",
                "survey" => "photo_mesh",
                _ => continue,
            };
            over.set_item(
                field,
                Py::new(
                    py,
                    Shown {
                        source: obj.unbind(),
                        visible: geo.get_item(key)?.is_truthy()?,
                    },
                )?,
            )?;
        }
    }
    if let Some(hidden) = opt(view, "hidden_shown") {
        for (k, f) in [
            ("objects", "show_hidden_objects"),
            ("geometry", "show_hidden_geometry"),
        ] {
            over.set_item(f, hidden.call_method1("get", (k,))?.is_truthy()?)?;
        }
    }
    if let Some(shadows) = opt(scene, "shadows") {
        let class = py.import("core.sun")?.getattr("ShadowSettings")?;
        let state = if let Some(v) = opt(view, "shadows") {
            class
                .call_method1("from_dict", (v,))?
                .call_method0("to_dict")?
        } else {
            shadows.call_method0("to_dict")?
        };
        if let Some(v) = opt(frame, "shadows") {
            state.set_item("enabled", v.is_truthy()?)?;
        }
        if let Some(v) = opt(frame, "sun_hour") {
            let h = v.extract::<f64>()?.clamp(0., 23.999);
            state.set_item("hour", h as i64)?;
            state.set_item("minute", (h.fract() * 60.).round_ties_even() as i64)?;
        }
        over.set_item("shadows", class.call_method1("from_dict", (state,))?)?;
    }
    py.get_type::<FrameScene>().call((scene,), Some(&over))
}
#[pyfunction]
fn frame_style<'py>(viewport: &Obj<'py>, scene: &Obj<'py>, frame: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = frame.py();
    let module = py.import("core.style")?;
    let style = frame
        .getattr("style")?
        .extract::<Option<String>>()?
        .unwrap_or_default();
    let kw = PyDict::new(py);
    kw.set_item("sky", false)?;
    kw.set_item("background", (1., 1., 1.))?;
    if matches!(style.as_str(), "tecnico" | "lineas") {
        kw.set_item("name", &style)?;
        kw.set_item(
            "face_mode",
            if style == "tecnico" {
                "hidden_line"
            } else {
                "wireframe"
            },
        )?;
        return module.getattr("Style")?.call((), Some(&kw));
    }
    let selected = style
        .strip_prefix("style:")
        .map(|n| module.call_method1("style_by_name", (n,)))
        .transpose()?
        .filter(|v| !v.is_none())
        .or_else(|| opt(viewport, "view_style"))
        .or_else(|| opt(scene, "display_style"));
    let style = match selected {
        Some(v) => v,
        None => module.call_method0("Style")?,
    };
    if flag(frame, "paper_bg", false)? {
        py.import("dataclasses")?
            .getattr("replace")?
            .call((style,), Some(&kw))
    } else {
        Ok(style)
    }
}
type V3 = (f64, f64, f64);
type Bounds = Option<(V3, V3)>;
#[pyfunction]
fn camera_inputs<'py>(live: &Obj<'py>, scene: &Obj<'py>) -> PyResult<(Bound<'py, PyDict>, Bounds)> {
    let py = live.py();
    let base = PyDict::new(py);
    for k in [
        "distance",
        "yaw",
        "pitch",
        "fov_deg",
        "perspective",
        "two_point",
        "shift_x",
        "shift_y",
        "roll_deg",
        "clip_near",
        "clip_far",
    ] {
        base.set_item(k, live.getattr(k)?)?;
    }
    for k in ["target", "up"] {
        base.set_item(k, live.getattr(k)?.call_method0("toTuple")?)?;
    }
    let (lo, hi): (Obj, Obj) = scene.call_method0("bounds")?.extract()?;
    let bounds = if lo.is_none() {
        None
    } else {
        let a: V3 = lo.call_method0("toTuple")?.extract()?;
        let b: V3 = hi.call_method0("toTuple")?.extract()?;
        base.set_item(
            "depth_reach",
            (
                ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5, (a.2 + b.2) * 0.5),
                ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2) + (b.2 - a.2).powi(2)).sqrt() * 0.5,
            ),
        )?;
        Some((a, b))
    };
    Ok((base, bounds))
}
#[pyfunction]
#[pyo3(signature=(live,frame,view,scene,origin=None))]
fn frame_camera<'py>(
    live: &Obj<'py>,
    frame: &Obj<'py>,
    view: &Obj<'py>,
    scene: &Obj<'py>,
    origin: Option<&Obj<'py>>,
) -> PyResult<Obj<'py>> {
    let py = live.py();
    let native = core(py)?;
    let (base, bounds) = camera_inputs(live, scene)?;
    let got = native.call_method1("sheet_frame_camera", (frame, view, &base, bounds))?;
    let cam = py.import("copy")?.call_method1("copy", (live,))?;
    for k in [
        "distance",
        "yaw",
        "pitch",
        "fov_deg",
        "perspective",
        "aspect",
        "two_point",
        "shift_x",
        "shift_y",
        "roll_deg",
        "clip_near",
        "clip_far",
    ] {
        cam.setattr(k, got.get_item(k)?)?;
    }
    let shift = origin
        .map(|v| v.extract::<[f64; 3]>())
        .transpose()?
        .unwrap_or_default();
    let shifted = |p: [f64; 3]| {
        native.call_method1("Vec3", (p[0] - shift[0], p[1] - shift[1], p[2] - shift[2]))
    };
    cam.setattr("target", shifted(got.get_item("target")?.extract()?)?)?;
    cam.setattr(
        "up",
        native
            .getattr("Vec3")?
            .call1(as_tuple(&got.get_item("up")?)?)?,
    )?;
    if let Some(depth) = base.get_item("depth_reach")? {
        cam.setattr(
            "depth_reach",
            (shifted(depth.get_item(0)?.extract()?)?, depth.get_item(1)?),
        )?;
    } else {
        cam.setattr("depth_reach", py.None())?;
    }
    let copier = this(py)?.getattr("matrix_copy")?;
    for (key, field) in [("view", "view_matrix"), ("projection", "projection_matrix")] {
        let m = native
            .call_method1("Matrix4", (got.get_item(key)?,))?
            .call_method0("transposed")?;
        if key == "view" && origin.is_some() {
            m.call_method1("translate", (shift[0], shift[1], shift[2]))?;
        }
        cam.setattr(field, partial(py, &copier, vec![m])?)?;
    }
    Ok(cam)
}
#[pyfunction]
fn frame_rows<'py>(
    view_widget: &Obj<'py>,
    scene: &Obj<'py>,
    frame: &Obj<'py>,
    view: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let _ = frame;
    let py = scene.py();
    let packed = view_widget.getattr("packed")?;
    let groups = list(&packed.getattr("groups")?)?;
    let hidden = core(py)?
        .call_method1(
            "sheet_frame_hidden",
            (
                scene.getattr("_root")?,
                list(&scene.getattr("groups")?)?,
                &groups,
                view,
            ),
        )?
        .extract::<Vec<u8>>()?;
    let rows = packed.getattr("instances")?.call_method0("copy")?;
    let info = rows.get_item("info")?;
    let mut flags = Vec::with_capacity(hidden.len());
    let mut colors = vec![0u32; hidden.len()];
    for (g, h) in groups.try_iter()?.zip(hidden) {
        flags.push(u32::from(h) | if flag(&g?, "billboard", false)? { 8 } else { 0 });
    }
    if let Some(key) = opt(view, "navigator_view").filter(|v| v.is_truthy().unwrap_or(false)) {
        let registry = py
            .import("core.bim_registry")?
            .call_method1("get", (scene,))?;
        if let Some(saved) = registry
            .call_method1("get", ("views", PyDict::new(py)))?
            .cast::<PyDict>()?
            .get_item(&key)?
        {
            let shadow = py.import("copy")?.call_method1("copy", (scene,))?;
            shadow.setattr(
                "selection",
                py.import("builtins")?
                    .call_method1("set", (scene.getattr("selection")?,))?,
            )?;
            shadow.setattr("edit_group", py.None())?;
            let proxy = py.import("types")?.call_method0("SimpleNamespace")?;
            proxy.setattr("scene", &shadow)?;
            proxy.setattr("update", this(py)?.getattr("noop")?)?;
            let kw = PyDict::new(py);
            kw.set_item("key", key)?;
            kw.set_item("remember", false)?;
            for name in ["isolate", "exact"] {
                kw.set_item(name, saved.call_method1("get", (name, false))?)?;
            }
            py.import("core.agent_facets")?
                .getattr("set_display")?
                .call(
                    (
                        &proxy,
                        saved.get_item("filter")?,
                        saved.call_method1("get", ("color_by",))?,
                    ),
                    Some(&kw),
                )?;
            let allowed = shadow.getattr("_navigator_allowed")?;
            let through = shadow.getattr("_navigator_through")?;
            let tint = opt(&proxy, "view_tint");
            for (i, (uid, g)) in packed
                .getattr("uids")?
                .try_iter()?
                .zip(groups.try_iter()?)
                .take(flags.len())
                .enumerate()
            {
                let g = g?;
                if !g.is_none() && !allowed.is_none() {
                    let k = shadow.call_method1("navigator_key", (g,))?;
                    if !allowed.contains(&k)? && (through.is_none() || !through.contains(k)?) {
                        flags[i] |= 1;
                    }
                }
                if let Some(ref tint) = tint {
                    let rgb = tint.call_method1("get", (uid?,))?;
                    if !rgb.is_none() {
                        let mut bytes = [255; 4];
                        for (i, ch) in bytes[..3].iter_mut().enumerate() {
                            *ch = (rgb.get_item(i)?.extract::<f64>()? * 255.) as u8;
                        }
                        colors[i] = u32::from_le_bytes(bytes);
                    }
                }
            }
        }
    }
    let all = pyo3::types::PySlice::full(py);
    info.set_item((&all, 2), flags)?;
    info.set_item((&all, 3), colors)?;
    Ok(rows)
}
fn render_target<'py>(
    widget: &Obj<'py>,
    frame: &Obj<'py>,
    method: &str,
    key: Option<&Obj<'py>>,
) -> PyResult<Obj<'py>> {
    let py = widget.py();
    let source = widget.getattr("source_viewport")?;
    let scene = source.getattr("scene")?;
    let view = saved_view(&scene, frame)?;
    if absent(&view) {
        return Ok(view);
    }
    let facade = frame_scene(&scene, frame, &view)?;
    let style = frame_style(&source, &facade, frame)?;
    let module = this(py)?;
    let comp = py.import("core.composition")?;
    let dpi = comp.getattr("RENDER_DPI")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("scene", facade)?;
    kwargs.set_item("style", style)?;
    kwargs.set_item("sharp_shadows", true)?;
    kwargs.set_item(
        "camera",
        partial(
            py,
            &module.getattr("frame_camera")?,
            vec![
                source.getattr("camera")?,
                frame.clone(),
                view.clone(),
                scene.clone(),
            ],
        )?,
    )?;
    kwargs.set_item(
        "rows",
        partial(
            py,
            &module.getattr("frame_rows")?,
            vec![widget.clone(), scene, frame.clone(), view],
        )?,
    )?;
    if let Some(k) = key {
        kwargs.set_item("page_key", k)?;
    }
    for (k, field, default) in [
        ("edge_px", "pen_edge_mm", 0.18),
        ("profile_px", "pen_profile_mm", 0.35),
    ] {
        kwargs.set_item(
            k,
            comp.call_method1(
                "pen_px",
                (
                    opt(frame, field).map_or(Ok(default), |v| v.extract())?,
                    &dpi,
                ),
            )?,
        )?;
    }
    widget.getattr(method)?.call(
        as_tuple(&frame.call_method1("render_px", (dpi,))?)?,
        Some(&kwargs),
    )
}
#[pyfunction]
fn render(view_widget: &Obj<'_>, frame: &Obj<'_>, key: &Obj<'_>) -> PyResult<bool> {
    Ok(!absent(&render_target(
        view_widget,
        frame,
        "render_image",
        Some(key),
    )?))
}
#[pyfunction]
fn picture<'py>(
    view_widget: &Obj<'py>,
    frame: &Obj<'py>,
) -> PyResult<Option<(usize, usize, Obj<'py>)>> {
    let result = render_target(view_widget, frame, "render_pixels", None)?;
    if result.is_none() || absent(&result) {
        return Ok(None);
    }
    let (w, h, data): (usize, usize, Obj) = result.extract()?;
    Ok(Some((
        w,
        h,
        view_widget
            .py()
            .import("ingetrazo_render")?
            .call_method1("opaque_frame", (w, h, data))?,
    )))
}
pub fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = parent.py();
    let m = PyModule::new(py, "sheet_frames")?;
    m.add_class::<FrameScene>()?;
    for f in [
        wrap_pyfunction!(saved_view, &m)?,
        wrap_pyfunction!(frame_scene, &m)?,
        wrap_pyfunction!(frame_style, &m)?,
        wrap_pyfunction!(camera_inputs, &m)?,
        wrap_pyfunction!(frame_camera, &m)?,
        wrap_pyfunction!(frame_rows, &m)?,
        wrap_pyfunction!(render, &m)?,
        wrap_pyfunction!(picture, &m)?,
        wrap_pyfunction!(matrix_copy, &m)?,
        wrap_pyfunction!(constant, &m)?,
        wrap_pyfunction!(noop, &m)?,
    ] {
        m.add_function(f)?;
    }
    geometry::register(&m)?;
    records::register(&m)?;
    export::register(&m)?;
    parent.add_submodule(&m)?;
    py.import("sys")?
        .getattr("modules")?
        .set_item("ingetrazo_render.sheet_frames", m)
}
