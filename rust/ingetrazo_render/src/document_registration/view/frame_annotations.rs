// SPDX-License-Identifier: GPL-3.0-or-later
//! Document annotations and live tool cues on the annotation canvas
//! (`views.wgpu_annotations`): an isolated scene/camera projection the
//! document's own painters draw through, the guide dashes, and the capture
//! composite (render-glue lane; from `views/wgpu_annotations.py`).
use super::*;
use pyo3::exceptions::PyValueError;
use pyo3::types::PySlice;

fn attr<'py>(o: &Obj<'py>, name: &str) -> Reply<'py> {
    optional(o, name)
}
fn annotations<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyModule>> {
    py.import("views.wgpu_annotations")
}
fn interaction<'py>(py: Python<'py>) -> Reply<'py> {
    py.import("views.document_interaction")?.getattr("DocumentInteraction")
}
fn number(value: &Obj<'_>) -> PyResult<f64> {
    value.py().get_type::<pyo3::types::PyFloat>().call1((value,))?.extract()
}
/// Run `body` between `painter.save()` and `painter.restore()`.
fn saved<'py, T>(painter: &Obj<'py>, body: impl FnOnce() -> PyResult<T>) -> PyResult<T> {
    painter.call_method0("save")?;
    let result = body();
    painter.call_method0("restore")?;
    result
}

// ---- AnnotationLayout -------------------------------------------------------

/// What the layout borrows of the document's painters. Explicit allowlist:
/// no accidental delegation into live widget geometry.
const BORROWED: &[&str] = &[
    "_clip_pixel_line", "_clip_segment_front", "_draw_dimensions", "_draw_text_labels", "_draw_guides", "_draw_section_planes",
    "section_plane_frame", "_draw_geo_surfaces", "_draw_geo_paths", "_draw_geo_points", "_draw_geo_path_label", "ground_height", "drape",
    // Live tool feedback (draw_tool_feedback): GL's own painters.
    "_segment_to_pixels", "_clip_w", "_CLIP_W", "_SNAP_LABELS", "_rubber_band_style", "_draw_rubber_band_overlay", "_draw_guide_preview",
    "_draw_inference_marker", "_draw_scale_box", "_draw_length_label", "_length_label_at", "_draw_route_marker", "_OCCLUSION_STEP_PX",
    "_OCCLUSION_MAX_SAMPLES", "_OCCLUSION_SPLIT", "_OCCLUSION_ROUNDS",
];
const BORROWED_STATIC: &[&str] = &["_text_block_x", "_format_dim_value"];

fn layout_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (this, scene, camera, size) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?, a.get_item(3)?);
    this.setattr("scene", scene)?;
    let camera = py.import("copy")?.call_method1("copy", (camera,))?;
    let [width, height] = unpack::<2>(&size)?;
    let (width, height) = (number(&width)?, number(&height)?);
    this.setattr("camera", &camera)?;
    this.setattr("_width", width)?;
    this.setattr("_height", height)?;
    if width <= 0. || height <= 0. {
        return Err(PyValueError::new_err("Annotation canvas dimensions must be positive"));
    }
    camera.setattr("aspect", width / height)?;
    this.setattr("_mvp", camera.call_method0("projection_matrix")?.mul(camera.call_method0("view_matrix")?)?)?;
    this.setattr("_clear_points", PySet::empty(py)?)?;
    this.setattr("_clear_segment", a.get_item(6)?)?;
    this.setattr("_occluded", a.get_item(4)?)?;
    this.setattr("_hover_entity", a.get_item(5)?)?;
    Ok(none(py))
}

fn layout_width<'py>(a: &Args<'py>) -> Reply<'py> {
    a.get_item(0)?.getattr("_width")
}
fn layout_height<'py>(a: &Args<'py>) -> Reply<'py> {
    a.get_item(0)?.getattr("_height")
}

fn layout_world_to_pixel<'py>(a: &Args<'py>) -> Reply<'py> {
    let (this, given) = (a.get_item(0)?, a.get_item(2)?);
    let mvp = if given.is_none() { this.getattr("_mvp")? } else { given };
    interaction(a.py())?.call_method1("_world_to_pixel", (&this, a.get_item(1)?, mvp))
}

fn layout_occluded_on_screen<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (this, point) = (a.get_item(0)?, a.get_item(1)?);
    let occluded = this.getattr("_occluded")?;
    if !occluded.is_truthy()? || this.getattr("_clear_points")?.contains(point.call_method0("toTuple")?)? {
        return false.into_bound_py_any(py);
    }
    occluded.call1((point,))?.is_truthy()?.into_bound_py_any(py)
}

fn layout_draw_occluded_segment<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (this, painter, from, to) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?, a.get_item(3)?);
    // Keep the same projected strokes and pen joins when the resident boxes
    // prove this entire line is in front of any possible occluder.
    let occluded = this.getattr("_occluded")?;
    let clear = this.getattr("_clear_segment")?;
    if clear.is_truthy()? && clear.call1((&from, &to))?.is_truthy()? {
        // The dimension's arrowheads and label query these same points
        // immediately after the stroke. Its clear segment covers all three.
        let middle = from.add(&to)?.mul(0.5)?;
        this.getattr("_clear_points")?.call_method1("update", ((from.call_method0("toTuple")?, to.call_method0("toTuple")?, middle.call_method0("toTuple")?),))?;
        this.setattr("_occluded", py.None())?;
    }
    let drawn = interaction(py)?.call_method1("_draw_occluded_segment", (&this, painter, from, to));
    this.setattr("_occluded", occluded)?;
    drawn?;
    Ok(none(py))
}

fn layout_project_samples<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let this = a.get_item(0)?;
    let samples = PyList::empty(py);
    for point in a.get_item(1)?.try_iter()? {
        let point = point?;
        let pixel = this.call_method1("_world_to_pixel", (&point,))?;
        samples.append((pixel, !this.call_method1("_occluded_on_screen", (&point,))?.is_truthy()?))?;
    }
    Ok(samples.into_any())
}

#[rustfmt::skip]
const LAYOUT: &[Method] = &[
    ("__init__", Mode::Method, layout_init, &["self", "scene", "camera", "logical_size", "*", "occluded=None", "hover=None", "clear_segment=None"], "", ""),
    ("width", Mode::Method, layout_width, &["self"], "", ""),
    ("height", Mode::Method, layout_height, &["self"], "", ""),
    ("_world_to_pixel", Mode::Method, layout_world_to_pixel, &["self", "point", "mvp=None"], "", ""),
    ("_occluded_on_screen", Mode::Method, layout_occluded_on_screen, &["self", "point"], "", ""),
    ("_draw_occluded_segment", Mode::Method, layout_draw_occluded_segment, &["self", "painter", "a", "b"], "", ""),
    ("_project_samples", Mode::Method, layout_project_samples, &["self", "points"], "", ""),
];

// ---- The module's functions -------------------------------------------------

/// Return GL world dash vertices (N,2,3) and selected flags (N,).
///
/// P1/P2 may upload these to a depth-tested line pass instead of painting
/// them. Includes finite guide-point origin segments and layer visibility
/// filtering.
fn guide_segments<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (scene, camera) = (a.get_item(0)?, a.get_item(1)?);
    let module = annotations(py)?;
    let numpy = py.import("numpy")?;
    let distance = number(&camera.getattr("distance")?)?.max(1e-3);
    let eye = if camera.getattr("perspective")?.is_truthy()? { camera.call_method0("eye")? } else { none(py) };
    let shown = module.call_method1("shown_guides", (&scene,))?;
    let [coords, spans] = unpack::<2>(&module.getattr("_guide_vertices")?.call(
        (shown, distance * 0.03, scene.getattr("selection")?),
        Some(&fields!(py; "centre" => camera.getattr("target")?, "reach" => distance * 15., "eye" => eye)),
    )?)?;
    let segments = numpy.call_method("asarray", (coords,), Some(&fields!(py; "dtype" => numpy.getattr("float32")?)))?.call_method1("reshape", (-1, 2, 3))?;
    let selected = numpy.call_method("zeros", (segments.len()?,), Some(&fields!(py; "dtype" => py.get_type::<pyo3::types::PyBool>())))?;
    for span in spans.try_iter()? {
        let [start, count, flag] = unpack::<3>(&span?)?;
        let (start, count): (isize, isize) = (start.extract()?, count.extract()?);
        selected.set_item(PySlice::new(py, start.div_euclid(2), (start + count).div_euclid(2), 1), flag)?;
    }
    (segments, selected).into_bound_py_any(py)
}

/// Project GL world dashes; no depth-buffer occlusion is possible here.
fn draw_guide_lines<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (painter, layout) = (a.get_item(0)?, a.get_item(1)?);
    let module = annotations(py)?;
    let [segments, selected] = unpack::<2>(&module.call_method1("guide_segments", (layout.getattr("scene")?, layout.getattr("camera")?))?)?;
    let (pen, colour, point, vector) = (module.getattr("QPen")?, module.getattr("QColor")?, module.getattr("QPointF")?, module.getattr("QVector3D")?);
    let flat = module.getattr("Qt")?.getattr("FlatCap")?;
    let mut pens = Vec::new();
    for rgb in [(0.27, 0.35, 0.47), (0.95, 0.45, 0.16)] {
        let pen = pen.call1((colour.call_method1("fromRgbF", rgb)?, 1.0))?;
        pen.call_method1("setCapStyle", (&flat,))?;
        pens.push(pen);
    }
    saved(&painter, || {
        for (segment, flag) in segments.try_iter()?.zip(selected.try_iter()?) {
            let [from, to] = unpack::<2>(&segment?)?;
            let clipped = layout.call_method1("_clip_segment_front", (vector.call1(positional(&from)?)?, vector.call1(positional(&to)?)?))?;
            if clipped.is_none() {
                continue;
            }
            let [from, to] = unpack::<2>(&clipped)?;
            let (from, to) = (layout.call_method1("_world_to_pixel", (from,))?, layout.call_method1("_world_to_pixel", (to,))?);
            if from.is_none() || to.is_none() {
                continue;
            }
            let visible = layout.call_method1("_clip_pixel_line", (from, to))?;
            if !visible.is_none() {
                painter.call_method1("setPen", (&pens[flag?.is_truthy()? as usize],))?;
                painter.call_method1("drawLine", (point.call1(positional(&visible.get_item(0)?)?)?, point.call1(positional(&visible.get_item(1)?)?)?))?;
            }
        }
        Ok(())
    })?;
    Ok(none(py))
}

/// Paint document annotations; return the isolated AnnotationLayout for
/// snaps.
///
/// `guide_lines=False` leaves the guide lines to the native depth-tested
/// line pass (`views.wgpu_lines`); their points stay painted here.
fn draw_annotations<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (painter, scene, camera, size) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?, a.get_item(3)?);
    let (annotations_only, guide_lines) = (a.get_item(4)?.is_truthy()?, a.get_item(7)?.is_truthy()?);
    let module = annotations(py)?;
    let layout = module.getattr("AnnotationLayout")?.call(
        (scene, camera, size),
        Some(&fields!(py; "occluded" => a.get_item(5)?, "hover" => a.get_item(6)?, "clear_segment" => a.get_item(8)?)),
    )?;
    saved(&painter, || {
        if !annotations_only {
            if guide_lines {
                module.call_method1("draw_guide_lines", (&painter, &layout))?;
            }
            for draw in ["_draw_guides", "_draw_section_planes", "_draw_geo_surfaces", "_draw_geo_paths", "_draw_geo_points"] {
                layout.call_method1(draw, (&painter,))?;
            }
            interaction(py)?.call_method1("_draw_image_outlines", (&layout, &painter))?;
        }
        layout.call_method1("_draw_dimensions", (&painter,))?;
        layout.call_method1("_draw_text_labels", (&painter,))?;
        Ok(())
    })?;
    Ok(layout)
}

/// GL's rubber band for the active tool: `(segments, rgba, width,
/// depth_tested)` or None. A depth-tested band (Push/Pull's forming box,
/// Offset, Paste) goes to the native preview's edges so faces hide its back
/// side; the rest floats in the overlay, as in GL.
fn rubber_band<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tools = a.get_item(0)?;
    let style = interaction(py)?.call_method1("_rubber_band_style", (&tools,))?;
    if style.is_none() {
        return Ok(style);
    }
    let depth_tested = attr_flag(&attr(&tools, "active_tool")?, "wireframe_depth_tested", false)?;
    py.get_type::<PyTuple>().call1((style,))?.add((depth_tested,))
}

/// Paint the live tool cues GL's `_draw_overlay` paints, with GL's
/// painters: the floating rubber band in its axis/lock colour, the dashed
/// guide preview of Tape and Protractor, Scale's grip box, Push/Pull's
/// «level with» marker, the route marker, extension overlays and the bold
/// length label (warm while a value is typed).
///
/// The Metal view had its own stand-ins (one cyan pen for every line, plain
/// text, flat grips) that dropped the colours telling you which axis a line
/// follows. `layout` is the frame's AnnotationLayout (live camera at the
/// overlay's logical size); `tools` is NativeTools.
fn draw_tool_feedback<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (painter, tools, layout) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?);
    let module = annotations(py)?;
    let tool = attr(&tools, "active_tool")?;
    let view = attr(&tools, "view")?;
    let native_wire = attr(&attr(&view, "renderer")?, "set_tool_wire")?;
    if tool.is_none() {
        if !native_wire.is_none() {
            native_wire.call1((py.None(), PyTuple::empty(py), 1.))?;
        }
        return Ok(none(py));
    }
    layout.setattr("active_tool", &tool)?;
    layout.setattr("last_snap", attr(&tools, "last_snap")?)?;
    let buffer = attr(&tools, "value_buffer")?;
    layout.setattr("_value_buffer", if buffer.is_truthy()? { buffer } else { "".into_bound_py_any(py)? })?;
    // With the chrome on, the value field on the geometry is the chrome's
    // (ui/value_field.rs): the bold label is not painted under it.
    layout.setattr("_chrome_owns_labels", attr_flag(&view, "_ui_attached", false)?)?;
    layout.setattr("_route_marker", attr(&tools, "_route_marker")?)?;
    let bands = attr(&tool, "rubber_band_lines")?.is_callable();
    saved(&painter, || {
        let floating = !attr_flag(&tool, "wireframe_depth_tested", false)? && bands;
        let band = if floating { module.call_method1("rubber_band", (&tools,))? } else { none(py) };
        if native_wire.is_none() {
            layout.setattr("_overlay_rubber", if band.is_none() { none(py) } else { band.get_item(PySlice::new(py, 0, 3, 1))? })?;
            layout.call_method1("_draw_rubber_band_overlay", (&painter,))?;
            if tool.hasattr("guide_preview_lines")? {
                layout.call_method1("_draw_guide_preview", (&painter,))?;
            }
        } else {
            let visible = match attr(&view, "source_viewport")?.getattr_opt("tool_preview")? {
                Some(visible) => visible.is_truthy()?,
                None => true,
            };
            let guides = match tool.getattr_opt("guide_preview_lines")? {
                Some(lines) if visible => lines.call0()?,
                _ => PyTuple::empty(py).into_any(),
            };
            let ratio = painter.call_method0("device")?.call_method0("devicePixelRatioF")?;
            native_wire.call1((if visible { band } else { none(py) }, guides, ratio))?;
        }
        for draw in ["_draw_inference_marker", "_draw_scale_box", "_draw_route_marker"] {
            layout.call_method1(draw, (&painter,))?;
        }
        let draw = attr(&interaction(py)?, "_draw_extension_overlays")?;
        if !draw.is_none() && attr(&tools, "_ext_overlays")?.is_truthy()? {
            direct(py, plugin_paint, (&painter, py.import("functools")?.call_method1("partial", (draw, &tools))?))?;
        }
        if attr(&tool, "value_label")?.is_callable() || bands {
            layout.call_method1("_draw_length_label", (&painter,))?;
        }
        Ok(())
    })?;
    Ok(none(py))
}

/// Project shared document marks and composite them into a capture in Rust.
fn composite_pixels<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (rgba, width, height, scene, camera, size) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?, a.get_item(3)?, a.get_item(4)?, a.get_item(5)?);
    if py.import("builtins")?.call_method1("min", (&size,))?.le(0)? {
        return Err(PyValueError::new_err("Annotation canvas dimensions must be positive"));
    }
    let target = py.import("ingetrazo_render")?.getattr("AnnotationCanvas")?.call1((width, height, size.get_item(0)?))?;
    annotations(py)?.getattr("draw_annotations")?.call(
        (&target, scene, camera, target.getattr("logical_size")?),
        Some(&fields!(py; "annotations_only" => a.get_item(6)?, "occluded" => a.get_item(7)?)),
    )?;
    target.call_method1("composite", (rgba,))
}

/// `composite_pixels` for a caller holding a `SurveyRaster`: a new one with
/// the marks.
fn composite_annotations<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (image, scene, camera, size) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?, a.get_item(3)?);
    let raster = py.import("ingetrazo_render")?.getattr("SurveyRaster")?;
    let (width, height) = (image.getattr("width")?, image.getattr("height")?);
    if !a.get_item(4)?.is_truthy()? {
        return raster.call1((width, height, image.call_method0("rgba")?));
    }
    if !width.is_truthy()? || !height.is_truthy()? {
        return Err(PyValueError::new_err("Cannot annotate a null image"));
    }
    let rgba = annotations(py)?.getattr("composite_pixels")?.call(
        (image.call_method0("rgba")?, &width, &height, scene, camera),
        Some(&fields!(py; "logical_size" => size, "annotations_only" => a.get_item(5)?, "occluded" => a.get_item(6)?)),
    )?;
    raster.call1((width, height, rgba))
}

/// Call active tool's GL painter hook, then prioritized lock/reference
/// labels.
///
/// P1 passes NativeTools (or a GL-compatible facade), every live frame
/// regardless of document annotations. No export call. Returns without
/// altering painter state.
fn draw_tool_overlays<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (painter, tools) = (a.get_item(0)?, a.get_item(1)?);
    let labels = a.get_item(2)?.is_truthy()?;
    let module = annotations(py)?;
    let document = interaction(py)?;
    let tool = match tools.getattr_opt("tool")? {
        Some(tool) => tool,
        None => attr(&tools, "active_tool")?,
    };
    saved(&painter, || {
        painter.call_method1("setBrush", (module.getattr("Qt")?.getattr("NoBrush")?,))?;
        let hook = attr(&tool, "draw_overlay")?;
        if hook.is_callable() {
            let own = match hook.getattr_opt("__module__")? {
                Some(name) => name.call_method1("startswith", ("tools.",))?.is_truthy()?,
                None => false,
            };
            if own {
                hook.call1((&tools, &painter))?;
            } else {
                direct(py, plugin_paint, (&painter, py.import("functools")?.call_method1("partial", (hook, &tools))?))?;
            }
        }
        if !labels {
            return Ok(());
        }
        if !attr(&tools, "reference_mode")?.is_none() {
            document.call_method1("_draw_reference_label", (&tools, &painter))?;
        } else if !attr(&tools, "axis_lock")?.is_none() {
            document.call_method1("_draw_axis_lock_label", (&tools, &painter))?;
        } else if !attr(&tool, "plane_lock")?.is_none() {
            // NativeTools may still expose source.active_tool; use the actual tool.
            let holder = py.import("types")?.getattr("SimpleNamespace")?.call((), Some(&fields!(py; "active_tool" => &tool)))?;
            document.call_method1("_draw_plane_lock_label", (holder, &painter))?;
        } else {
            document.call_method1("_draw_inference_label", (&tools, &painter))?;
        }
        let mode = tools.getattr_opt("linear_inference_mode")?;
        if mode.is_some_and(|mode| mode.ne("all").unwrap_or(false)) {
            document.call_method1("_draw_linear_mode_label", (&tools, &painter))?;
        }
        Ok(())
    })?;
    Ok(none(py))
}

/// An extension's overlay hook paints on the annotation canvas itself,
/// which answers the painter calls it makes (pens, brushes, lines,
/// polygons, rectangles, ellipses, text). There is no Qt painter.
pub(super) fn plugin_paint<'py>(a: &Args<'py>) -> Reply<'py> {
    a.get_item(1)?.call1((a.get_item(0)?,))
}

pub(super) fn install_annotations(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add(
        "__doc__",
        "Document annotations and live tool cues on the annotation canvas.\n\nLive: call draw_annotations(painter, scene, camera, (width, height),\noccluded=tools._is_occluded), then draw_tool_overlays(painter, tools),\nindependently of document annotation presence. Draw snap feedback with\nwgpu_overlay.draw_snap_indicator(painter, snap, layout._world_to_pixel,\nlayout._clip_pixel_line), where layout is the AnnotationLayout returned by\ndraw_annotations.\n\nExport: after native capture, call composite_annotations(image, scene, camera,\nlogical_size=(live_width, live_height), annotations_only=False). It returns a\nnew image. The camera is copied and its aspect is set to the output aspect, so\nsheet frames do not project through the live widget. Pass an\noccluded(world_point)->bool callback only if valid for that capture camera.\n\nThe overlay has no depth attachment, so the live view sends guide lines to the\nnative depth-tested line pass instead (``views.wgpu_lines``,\n``guide_lines=False``); exports still paint them here. Only painter-safe\nmethods of the document are bound on the layout.\n",
    )?;
    modules::import_names(module, "copy", &[("copy", "copy")])?;
    module.add("np", py.import("numpy")?)?;
    modules::import_names(module, "core.input_codes", &[("Input", "Qt")])?;
    modules::import_names(module, "ingetrazo_core", &[("ScreenPoint", "QPointF"), ("Vec3", "QVector3D")])?;
    modules::import_names(module, "ingetrazo_render", &[("PaintColor", "QColor"), ("PaintPen", "QPen")])?;
    modules::import_names(
        module,
        "views.document_interaction",
        &[("DocumentInteraction", "DocumentInteraction"), ("_guide_vertices", "_guide_vertices"), ("shown_guides", "shown_guides")],
    )?;
    let document = module.getattr("DocumentInteraction")?;
    let attrs = fields!(py; "__doc__" => "Isolated scene/camera projection for a logical annotation canvas.");
    for name in BORROWED {
        attrs.set_item(name, document.getattr(*name)?)?;
    }
    let fixed = py.import("builtins")?.getattr("staticmethod")?;
    for name in BORROWED_STATIC {
        attrs.set_item(name, fixed.call1((document.getattr(*name)?,))?)?;
    }
    let layout = modules::class(module, "AnnotationLayout", &[], &attrs)?;
    install(&layout, &module.dict(), LAYOUT)?;
    install(
        module.dict().as_any(),
        &module.dict(),
        &[
            ("guide_segments", Mode::Function, guide_segments, &["scene", "camera"], "", ""),
            ("draw_guide_lines", Mode::Function, draw_guide_lines, &["painter", "layout"], "", ""),
            (
                "draw_annotations",
                Mode::Function,
                draw_annotations,
                &["painter", "scene", "camera", "logical_size", "*", "annotations_only=False", "occluded=None", "hover=None", "guide_lines=True", "clear_segment=None"],
                "",
                "",
            ),
            ("rubber_band", Mode::Function, rubber_band, &["tools"], "", ""),
            ("draw_tool_feedback", Mode::Function, draw_tool_feedback, &["painter", "tools", "layout"], "", ""),
            ("composite_pixels", Mode::Function, composite_pixels, &["rgba", "width", "height", "scene", "camera", "*", "logical_size", "annotations_only=False", "occluded=None"], "", ""),
            (
                "composite_annotations",
                Mode::Function,
                composite_annotations,
                &["image", "scene", "camera", "*", "logical_size", "overlays=True", "annotations_only=False", "occluded=None"],
                "",
                "",
            ),
            ("draw_tool_overlays", Mode::Function, draw_tool_overlays, &["painter", "tools", "*", "labels=True"], "", ""),
        ],
    )
}
