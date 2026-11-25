// SPDX-License-Identifier: GPL-3.0-or-later
//! `MetalView`: pictures of the resident scene off the screen (exports, the
//! sheet frames) and the ID pass the picks read.
use super::metal_view::{attr, truthy};
use super::*;

fn slice_all(py: Python<'_>) -> Bound<'_, pyo3::types::PySlice> {
    pyo3::types::PySlice::full(py)
}

/// R3c-2a: render sheet targets using this view's resident scene.
///
/// A sheet frame passes what it shows instead of the live state: `scene` (a
/// read-only facade), `style`, `camera(origin)` and `rows()` (resident rows
/// with the frame's visibility), and `page_key`, the page texture the
/// picture goes to: then nothing is read back and None is returned.
fn render_image<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let kw = PyDict::new(py);
    for (at, name) in ["overlays", "sharp_shadows", "transparent", "annotations_only", "page_key", "scene", "style", "camera", "rows", "edge_px", "profile_px"].iter().enumerate() {
        kw.set_item(*name, a.get_item(3 + at)?)?;
    }
    let frame = view.call_method("render_pixels", (a.get_item(1)?, a.get_item(2)?), Some(&kw))?;
    if frame.is_none() {
        return Ok(frame);
    }
    let pixels = py.get_type::<pyo3::types::PyBytes>().call1((frame.get_item(2)?,))?;
    py.import("ingetrazo_render")?.getattr("SurveyRaster")?.call1((frame.get_item(0)?, frame.get_item(1)?, pixels))
}

/// RGBA of the requested frame, including any document marks for that camera.
#[pyfunction(signature = (this, width_px, height_px = None, *, overlays = None, annotations_only = None, **frame_options))]
fn render_pixels<'py>(this: &Obj<'py>, width_px: &Obj<'py>, height_px: Option<Obj<'py>>, overlays: Option<Obj<'py>>, annotations_only: Option<Obj<'py>>, frame_options: Option<&Bound<'py, PyDict>>) -> Reply<'py> {
    let on = |flag: Option<Obj<'py>>| flag.map_or(Ok(false), |flag| flag.is_truthy());
    let (overlays, annotations_only) = (on(overlays)?, on(annotations_only)?);
    let py = this.py();
    let frame = this.call_method("_render_resident", (width_px, height_px), frame_options)?;
    if frame.is_none() {
        return Ok(frame);
    }
    let (w, h, mut pixels, camera) = (frame.get_item(0)?, frame.get_item(1)?, frame.get_item(2)?, frame.get_item(3)?);
    if overlays {
        let kw = PyDict::new(py);
        kw.set_item("logical_size", (this.call_method0("width")?, this.call_method0("height")?))?;
        kw.set_item("annotations_only", annotations_only)?;
        pixels = py.import("views.wgpu_annotations")?.call_method(
            "composite_pixels",
            (pixels, &w, &h, this.getattr("source_viewport")?.getattr("scene")?, camera),
            Some(&kw),
        )?;
    }
    (w, h, pixels).into_bound_py_any(py)
}

fn render_resident<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let (sharp_shadows, transparent, page_key, scene, style, camera, rows, edge_px, profile_px) = (
        a.get_item(3)?.is_truthy()?, a.get_item(4)?, a.get_item(5)?, a.get_item(6)?, a.get_item(7)?, a.get_item(8)?,
        a.get_item(9)?, a.get_item(10)?, a.get_item(11)?,
    );
    let np = py.import("numpy")?;
    let copy = py.import("copy")?.getattr("copy")?;
    let source = view.getattr("source_viewport")?;
    view.getattr("tools")?.call_method0("sync")?;
    let width_px: i64 = a.get_item(1)?.call_method0("__int__")?.extract()?;
    let height_px: i64 = match a.get_item(2)? {
        height if height.is_none() => {
            let (w, h): (f64, f64) = (view.call_method0("width")?.extract()?, view.call_method0("height")?.extract()?);
            (a.get_item(1)?.extract::<f64>()? * h / w.max(1.)).round_ties_even() as i64
        }
        height => height.call_method0("__int__")?.extract()?,
    };
    if width_px <= 0 || height_px <= 0 {
        return Err(pyo3::exceptions::PyValueError::new_err("Image dimensions must be positive"));
    }
    let shared = view.getattr("_shared")?;
    if shared.hasattr("_content_token")? {
        shared.call_method1("set_scene", (source.getattr("scene")?,))?;
    }
    if view.getattr("packed")?.is_none() {
        let kw = PyDict::new(py);
        kw.set_item("focus", source.getattr("camera")?.getattr("target")?.call_method0("toTuple")?)?;
        let packed = py.import("views.wgpu_view")?.getattr("pack_scene")?.call((source.getattr("scene")?, attr(&source, "view_tint")?), Some(&kw))?;
        view.setattr("packed", packed)?;
    }
    if !view.getattr("packed")?.getattr("model_translations")?.is_none() {
        view.call_method0("_sync_scene_camera")?;
    }
    if view.getattr("renderer")?.is_none() || view.getattr("_scene_dirty")?.is_truthy()? {
        let kw = PyDict::new(py);
        kw.set_item("force", true)?;
        view.call_method("_upload", (), Some(&kw))?;
    }
    let packed = view.getattr("packed")?;
    let renderer = view.getattr("renderer")?;
    let translated = !packed.getattr("model_translations")?.is_none();
    let camera = if !camera.is_none() {
        camera.call1((if translated { packed.getattr("origin")? } else { none(py) },))?
    } else {
        let camera = copy.call1((if translated { view.getattr("camera")? } else { source.getattr("camera")? },))?;
        camera.setattr("aspect", width_px as f64 / height_px as f64)?;
        camera
    };
    let (matrix, projection) = (camera.call_method0("view_matrix")?, camera.call_method0("projection_matrix")?);
    let shown = if scene.is_none() { source.getattr("scene")? } else { scene.clone() };
    let look = if style.is_none() { source.call_method0("_effective_style")? } else { style };
    let or_default = |name: &str, default: Obj<'py>| -> Reply<'py> {
        Ok(source.getattr_opt(name)?.unwrap_or(default))
    };
    let kw = PyDict::new(py);
    kw.set_item("horizon", view.call_method1("_horizon", (projection.mul(&matrix)?, &camera))?)?;
    kw.set_item("edit_rest_fade", or_default("edit_rest_fade", 0.75.into_bound_py_any(py)?)?)?;
    kw.set_item("soft_shadows", or_default("_soft_shadows", 0.into_bound_py_any(py)?)?)?;
    kw.set_item("camera", &camera)?;
    let payload = py.import("views.wgpu_style")?.call_method("style_payload", (&shown, &look), Some(&kw))?;
    let references = py.import("views.wgpu_references")?.getattr("sync")?;
    payload.set_item("transparent_background", transparent)?;
    let width_of = |given: &Obj<'py>, name: &str| -> PyResult<f64> {
        if given.is_none() { or_default(name, 1.into_bound_py_any(py)?)?.extract() } else { given.extract() }
    };
    payload.set_item("edge_width", width_of(&edge_px, "_export_edge_px")?)?;
    payload.set_item("profile_width", width_of(&profile_px, "_export_profile_px")?)?;
    let sections = py.import("views.wgpu_sections")?;
    let rows = if rows.is_none() {
        sections.call_method1("capture_rows", (&packed, source.getattr("scene")?, attr(&source, "view_tint")?))?
    } else {
        rows.call0()?
    };
    let info = rows.get_item("info")?;
    info.get_item((slice_all(py), 2))?.call_method1("__iand__", (np.call_method1("uint32", (!192u32,))?,))?;
    let mode = or_default("edit_rest_mode", "fade".into_bound_py_any(py)?)?;
    let hide = mode.eq("hide")?;
    if (hide || mode.eq("fade")?) && !shown.getattr("edit_group")?.is_none() {
        let rest = py.import("views.wgpu_edit_context")?.call_method1("rest_rows", (packed.getattr("groups")?, shown.getattr("edit_group")?))?;
        let at = (rest, 2).into_bound_py_any(py)?;
        info.set_item(&at, info.get_item(&at)?.bitor(64 | if hide { 128 } else { 0 })?)?;
    }
    let sprite_type = py.import("views.wgpu_billboards")?.getattr("Billboards")?;
    let axes_state = py.import("views.wgpu_axes")?.getattr("axes_state")?;
    let capture = copy.call1((&packed,))?;
    capture.setattr("instances", &rows)?;
    let sprites = attr(&view, "_billboards")?;
    if !sprites.is_none() {
        rows.get_item("world")?.set_item(sprites.getattr("ids")?, sprites.getattr("base")?)?;
        sprite_type.call1((&capture, sprites.getattr("ids")?))?.call_method1("update", (&camera,))?;
    }
    // Capture rows may change tint/visibility only. Install the
    // camera-facing matrices through the resident API, restoring them even
    // on failure.
    let sprite_ids = if sprites.is_none() { PyList::empty(py).into_any() } else { sprites.getattr("ids")?.call_method0("tolist")? };
    let turned = sprite_ids.is_truthy()?;
    let flat = |array: Obj<'py>| array.call_method1("reshape", (-1,));
    let resident_world = packed.getattr("instances")?.get_item("world")?.get_item(&sprite_ids)?.call_method0("copy")?;
    let origin = packed.getattr("origin")?;
    let old_axes = match attr(&view, "_axes_state")? {
        axes if axes.is_truthy()? => axes,
        _ => axes_state.call1((source.getattr("scene")?, view.getattr("camera")?, &source, &origin))?,
    };
    let previous = match view.getattr_opt("_section_uploaded")? {
        Some(previous) => previous,
        None => np.call_method1("empty", (0, np.getattr("uint8")?))?,
    };
    let data = sections.call_method1("geometry", (&view, &shown, &look, &camera, height_px, &rows))?;
    let spread = |values: Obj<'py>| -> PyResult<Bound<'py, PyTuple>> { Ok(py.get_type::<PyTuple>().call1((values,))?.cast_into()?) };
    let picture = (|| -> Reply<'py> {
        let kw = PyDict::new(py);
        kw.set_item("scene", &scene)?;
        references.call((&view, &payload), Some(&kw))?;
        if turned {
            renderer.call_method1("set_instance_world", (&sprite_ids, flat(rows.get_item("world")?.get_item(&sprite_ids)?)?))?;
        }
        renderer.getattr("set_axes")?.call1(spread(axes_state.call1((&shown, &camera, &source, &origin))?)?)?;
        renderer.call_method1("set_section_geometry", (&data,))?;
        let bytes = if rows.len()? != 0 {
            flat(rows.call_method1("view", (np.getattr("uint8")?,))?)?
        } else {
            none(py)
        };
        let args = (width_px, height_px, spread(matrix.call_method0("data")?)?, spread(projection.call_method0("data")?)?, &payload, bytes);
        let options = PyDict::new(py);
        if sharp_shadows && payload.get_item("shadows")?.is_truthy()? {
            options.set_item("sharp_shadows", true)?;
        }
        // A far clip set by hand; otherwise reverse-Z reaches to infinity
        // (stand-in renderers need not know the flag).
        if !attr(&camera, "clip_far")?.is_none() {
            options.set_item("far_clip", true)?;
        }
        if !page_key.is_none() {
            renderer.call_method("page_frame_render", (&page_key, args.0, args.1, args.2, args.3, args.4, args.5), Some(&options))?;
            return Ok(none(py));
        }
        py.get_type::<pyo3::types::PyBytes>().call1((renderer.call_method("render_image", args, Some(&options))?,))
    })();
    let restored = (|| -> PyResult<()> {
        if turned {
            renderer.call_method1("set_instance_world", (&sprite_ids, flat(resident_world)?))?;
        }
        renderer.getattr("set_axes")?.call1(spread(old_axes)?)?;
        renderer.call_method1("set_section_geometry", (previous,))?;
        if !scene.is_none() {
            let style = match attr(&view, "_frame_style")? {
                style if style.is_truthy()? => style,
                _ => PyDict::new(py).into_any(),
            };
            references.call1((&view, style))?;
        }
        Ok(())
    })();
    let pixels = picture?;
    restored?;
    if !page_key.is_none() {
        return Ok(none(py));
    }
    (width_px, height_px, pixels, camera).into_bound_py_any(py)
}

fn paint_event<'py>(a: &Args<'py>) -> Reply<'py> {
    let view = a.get_item(0)?;
    // During a splitter/window drag keep the last Metal frame: the surface
    // is configured once the resize settles (_finish_resize).
    if !view.getattr("_resize_timer")?.call_method0("isActive")?.is_truthy()? {
        view.call_method0("draw_frame")?;
    }
    Ok(none(a.py()))
}

/// The resident row identifies both the placement and shared prototype.
fn resolve<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, row) = (a.get_item(0)?, a.get_item(1)?);
    let packed = view.getattr("packed")?;
    if packed.is_none() || row.is_none() {
        return Ok(none(py));
    }
    let at: i64 = row.call_method0("__int__")?.extract()?;
    if at < 0 || at as usize >= packed.getattr("uids")?.len()? {
        return Ok(none(py));
    }
    let info = packed.getattr("instances")?.get_item("info")?;
    let flags: u32 = info.get_item((at, 2))?.call_method0("__int__")?.extract()?;
    if flags & 129 != 0 {
        return Ok(none(py));
    }
    (packed.getattr("groups")?.get_item(at)?, info.get_item((at, 0))?.call_method0("__int__")?).into_bound_py_any(py)
}

fn prepare_pick_frame<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    if truthy(&view, "_drawing_frame")? {
        return Ok(none(py)); // overlay occlusion reads IDs during this very presentation
    }
    if !attr(&view.getattr("packed")?, "model_translations")?.is_none() {
        view.call_method0("_sync_scene_camera")?;
    }
    // A click immediately after release must use the new pixel mapping.
    let resize = view.getattr("_resize_timer")?;
    if resize.call_method0("isActive")?.is_truthy()? {
        resize.call_method0("stop")?;
        view.call_method0("_finish_resize")?;
    }
    if view.getattr("_scene_dirty")?.is_truthy()? && view.call_method0("isVisible")?.is_truthy()? {
        view.call_method0("_upload")?;
    }
    let timer = attr(&view, "_refresh_timer")?;
    if !timer.is_none() && timer.call_method0("isActive")?.is_truthy()? {
        view.call_method1("set_scene", (view.getattr("scene")?,))?;
    }
    view.call_method0("sync_edit_context")?;
    if truthy(&view, "_pick_frame_dirty")? && view.getattr("renderer")?.hasattr("render")? {
        view.setattr("_drawing_frame", true)?;
        let kw = PyDict::new(py);
        kw.set_item("pick_only", true)?;
        let frame = view.call_method("_present_frame", (), Some(&kw));
        view.setattr("_drawing_frame", false)?;
        frame?;
    }
    Ok(none(py))
}

fn pick_row<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let (x, y): (f64, f64) = (a.get_item(1)?.extract()?, a.get_item(2)?.extract()?);
    let renderer = view.getattr("renderer")?;
    if renderer.is_none() || x < 0. || y < 0. {
        return Ok(none(py));
    }
    view.call_method0("_prepare_pick_frame")?;
    let scale: f64 = view.call_method0("devicePixelRatioF")?.extract()?;
    let row = view.getattr("renderer")?.call_method1("pick", ((x * scale) as i64, (y * scale) as i64))?;
    Ok(if view.call_method1("resolve", (&row,))?.is_none() { none(py) } else { row })
}

#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("render_image", Mode::Method, render_image, &["self", "width_px", "height_px=None", "overlays=False", "sharp_shadows=False", "*",
        "transparent=False", "annotations_only=False", "page_key=None", "scene=None", "style=None", "camera=None", "rows=None",
        "edge_px=None", "profile_px=None"], "", ""),
    ("_render_resident", Mode::Method, render_resident, &["self", "width_px", "height_px", "sharp_shadows=False", "transparent=False",
        "page_key=None", "scene=None", "style=None", "camera=None", "rows=None", "edge_px=None", "profile_px=None"], "", ""),
    ("paintEvent", Mode::Method, paint_event, &["self", "event"], "", ""),
    ("resolve", Mode::Method, resolve, &["self", "row"], "", ""),
    ("_prepare_pick_frame", Mode::Method, prepare_pick_frame, &["self"], "", ""),
    ("pick_row", Mode::Method, pick_row, &["self", "x", "y"], "", ""),
];

pub(super) fn register(cls: &Obj, module: &Bound<PyModule>) -> PyResult<()> {
    install(cls, &module.dict(), METHODS)?;
    modules::method(cls, "render_pixels", wrap_pyfunction!(render_pixels, module)?.as_any())
}
