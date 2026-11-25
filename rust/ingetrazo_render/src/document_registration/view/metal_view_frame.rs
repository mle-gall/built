// SPDX-License-Identifier: GPL-3.0-or-later
//! `MetalView`: one presented frame (the camera, the style, the tints, the
//! overlay) and the depth fit.
use super::metal_view::{attr, truthy};
use super::*;

fn perf_counter(py: Python<'_>) -> PyResult<f64> {
    py.import("time")?.call_method0("perf_counter")?.extract()
}

fn present_frame<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, pick_only) = (a.get_item(0)?, a.get_item(1)?.is_truthy()?);
    let np = py.import("numpy")?;
    let start = perf_counter(py)?;
    // A scene packed since the last frame goes up BEFORE anything is
    // written per instance: the override write below is sized by the new
    // pack, and the renderer rejected it against the old resident scene
    // («one override per instance required») on every frame, so the
    // previous document stayed on screen and the first-frame hook never
    // fired (a new document never finished laying out its trays).
    if !attr(&view.getattr("packed")?, "model_translations")?.is_none() {
        view.call_method0("_sync_scene_camera")?;
    }
    if view.getattr("_scene_dirty")?.is_truthy()? {
        view.call_method0("_upload")?;
        if view.getattr("renderer")?.is_none() {
            return Ok(none(py));
        }
    }
    let tools = view.getattr("tools")?;
    if !pick_only {
        tools.call_method0("sync")?;
    }
    let source = view.getattr("source_viewport")?;
    let packed = view.getattr("packed")?;
    if !source.is_none() {
        if packed.getattr("model_translations")?.is_none() {
            view.setattr("camera", source.getattr("camera")?)?;
        }
        view.call_method0("_mirror_cursor")?;
    }
    let renderer = view.getattr("renderer")?;
    let camera = view.getattr("camera")?;
    let (width, height, scale) = (view.call_method0("width")?, view.call_method0("height")?, view.call_method0("devicePixelRatioF")?);
    // R-look 3c: compute each camera matrix once, after updating aspect.
    camera.setattr("aspect", width.extract::<f64>()? / height.extract::<f64>()?.max(1.))?;
    view.call_method0("_fit_depth")?;
    let view_matrix = camera.call_method0("view_matrix")?;
    let projection = camera.call_method0("projection_matrix")?;
    let shared = view.getattr("_shared")?;
    let first = shared.is(&view);
    if !source.is_none() {
        let style = source.call_method0("_effective_style")?;
        let mvp = projection.mul(&view_matrix)?;
        let kw = PyDict::new(py);
        kw.set_item("horizon", view.call_method1("_horizon", (mvp, &camera))?)?;
        kw.set_item("scale", &scale)?;
        kw.set_item("edit_rest_fade", match source.getattr_opt("edit_rest_fade")? {
            Some(fade) => fade,
            None => 0.75.into_bound_py_any(py)?,
        })?;
        kw.set_item("soft_shadows", match source.getattr_opt("_soft_shadows")? {
            Some(level) => level,
            None => 0.into_bound_py_any(py)?,
        })?;
        kw.set_item("camera", &camera)?;
        let payload = py.import("views.wgpu_style")?.call_method("style_payload", (source.getattr("scene")?, style), Some(&kw))?;
        if !payload.eq(attr(&view, "_frame_style")?)? {
            renderer.call_method1("set_style", (&payload,))?;
            view.setattr("_frame_style", payload)?;
        }
        // The resident rows' colours are the document's: the first view's
        // tint (Navigator, structure), not a peer's own.
        let tint_view = if first { source.clone() } else { shared.getattr("source_viewport")? };
        let tint = match attr(&tint_view, "view_tint")? {
            tint if tint.is_truthy()? => tint,
            _ => PyDict::new(py).into_any(),
        };
        // Structure colours can cover the whole model. set_display already
        // versions each replacement, including a restored previous view.
        let signature = if attr(&tint_view, "_view_color")?.eq("structure")? {
            ("structure", tint_view.getattr("_navigator_revision")?).into_bound_py_any(py)?
        } else {
            py.get_type::<PyTuple>().call1((py.import("builtins")?.call_method1("sorted", (tint.call_method0("items")?,))?,))?
        };
        if first && !signature.eq(attr(&view, "_tint_signature")?)? {
            let mut colors: Vec<u32> = Vec::new();
            for uid in packed.getattr("uids")?.try_iter()?.take(packed.getattr("groups")?.len()?) {
                let rgb = tint.call_method1("get", (uid?,))?;
                colors.push(if rgb.is_none() {
                    0
                } else {
                    let mut bytes = [0u8, 0, 0, 255];
                    for (axis, byte) in bytes.iter_mut().take(3).enumerate() {
                        *byte = (rgb.get_item(axis)?.extract::<f64>()? * 255.).clamp(0., 255.) as u8;
                    }
                    u32::from_le_bytes(bytes)
                });
            }
            if attr(&view, "_resident_count")?.eq(colors.len())? {
                renderer.call_method1("set_overrides", (colors,))?;
                view.setattr("_tint_signature", signature)?;
            }
        }
    }
    let frame_style = match attr(&view, "_frame_style")? {
        style if style.is_truthy()? => style,
        _ => PyDict::new(py).into_any(),
    };
    py.import("views.wgpu_references")?.call_method1("sync", (&view, frame_style))?;
    let future = attr(&view, "_inference_future")?;
    if !future.is_none() && future.call_method0("done")?.is_truthy()? {
        let mut index = future.call_method0("result")?;
        view.setattr("inference_index", &index)?;
        if !index.getattr("packed")?.getattr("origin")?.eq(packed.getattr("origin")?)? {
            index = py.import("views.wgpu_inference")?.call_method1("rebase", (index, &packed))?;
            view.setattr("inference_index", index)?;
        }
        let sprites = attr(&view, "_billboards")?;
        if !sprites.is_none() {
            sprites.setattr("key", py.None())?;
        }
        view.setattr("_inference_future", py.None())?;
    }
    view.call_method0("sync_edit_context")?;
    if !pick_only {
        view.call_method0("sync_selection")?;
    }
    let physical = (&width, &height, &scale).into_bound_py_any(py)?;
    if !physical.eq(attr(&view, "_physical_size")?)? {
        renderer.getattr("resize")?.call1(physical.clone().cast_into::<PyTuple>()?)?;
        view.setattr("_physical_size", physical)?;
    }
    // Reverse-Z reaches to infinity unless a far clip was set by hand
    // (Camera Settings ▸ Clipping): only then does the renderer keep the
    // projection's far plane.
    let far_clip = !attr(&camera, "clip_far")?.is_none();
    let tuple_of = |matrix: &Obj<'py>| py.get_type::<PyTuple>().call1((matrix.call_method0("data")?,));
    let camera_key = (tuple_of(&view_matrix)?, tuple_of(&projection)?).into_bound_py_any(py)?;
    let pose = (&camera_key, far_clip).into_bound_py_any(py)?;
    let settle = view.getattr("_settle_timer")?;
    if !pose.eq(view.getattr("_last_camera")?)? {
        view.setattr("_last_camera", pose)?;
        settle.call_method1("start", (80,))?;
    }
    let kw = PyDict::new(py);
    if far_clip {
        kw.set_item("far_clip", true)?;
    }
    renderer.getattr("set_camera")?.call(camera_key.cast_into::<PyTuple>()?, Some(&kw))?;
    // Face-me figures turn to the first view's camera: their rows are the
    // document's, one set for every view.
    let sprites = attr(&view, "_billboards")?;
    if first && !sprites.is_none() {
        sprites.call_method1("update", (&camera, &renderer, view.getattr("inference_index")?))?;
    }
    let scene = view.getattr("scene")?;
    let axes = py.import("views.wgpu_axes")?.call_method1("axes_state", (&scene, &camera, &source, packed.getattr("origin")?))?;
    if !axes.eq(attr(&view, "_axes_state")?)? {
        renderer.getattr("set_axes")?.call1(py.get_type::<PyTuple>().call1((&axes,))?.cast_into::<PyTuple>()?)?;
        view.setattr("_axes_state", axes)?;
    }
    // End R-look 3c camera/style work.
    // Guides and tool-owned overlays exist without dimensions or labels.
    let annotations = !source.is_none();
    // R3c-2a: section widgets share GL's painter implementation.
    py.import("views.wgpu_sections")?.call_method1("sync", (&view,))?;
    if pick_only {
        // Renderer.pick renders its own ID pass. The tool requesting the
        // pick can still be inside event(), so its preview cannot be read here.
        view.setattr("_pick_frame_dirty", false)?;
        return Ok(none(py));
    }
    let shown_planes = match scene.getattr_opt("show_section_planes")? {
        Some(shown) => shown.is_truthy()?,
        None => true,
    };
    let section_widgets = truthy(&scene, "section_planes")? && shown_planes;
    // With several views a tool's preview shows in one of them
    // (ViewGrid.set_tool_view): the others draw without it.
    let preview_here = match source.getattr_opt("tool_preview")? {
        Some(here) => here.is_truthy()?,
        None => true,
    };
    let has_preview_mesh = renderer.hasattr("set_preview_mesh")?;
    if !preview_here {
        if !truthy(&view, "_preview_given_away")? {
            view.setattr("_preview_given_away", true)?;
            view.setattr("overlay_primitives", PyList::empty(py))?;
            view.setattr("last_snap", py.None())?;
            if has_preview_mesh {
                let empty = np.call_method1("empty", (0, np.getattr("uint8")?))?;
                renderer.call_method1("set_preview_mesh", (&empty, &empty))?;
            }
            view.setattr("_selection_signature", py.None())?;
        }
    } else {
        view.setattr("_preview_given_away", false)?;
    }
    if preview_here && tools.getattr("mode")?.eq("mesh")? && !view.getattr("overlay_primitives")?.is_truthy()? {
        tools.call_method0("mesh_feedback")?;
    }
    // Selection boxes ride the native preview edges, depth-tested as in GL;
    // only a renderer without them gets overlay lines.
    let boxes = if has_preview_mesh { PyList::empty(py).into_any() } else { view.call_method0("_selection_box_primitives")? };
    // The edit box, guides and selected hidden edges go to the native line
    // pass, depth-tested as GL draws them; only a renderer without it
    // paints the box in the overlay (no depth there).
    let edit_box = if renderer.hasattr("set_lines")? {
        py.import("views.wgpu_lines")?.call_method1("sync", (&view,))?;
        PyList::empty(py).into_any()
    } else {
        view.call_method0("_edit_box_primitives")?
    };
    let outlines = py.import("views.wgpu_edit_context")?.call_method1("billboard_outline_primitives", (&view,))?;
    let primitives = edit_box.add(boxes)?.add(outlines)?.add(view.getattr("overlay_primitives")?)?;
    if renderer.hasattr("set_center_cue")? {
        let cursor = attr(&view, "_cue_cursor")?;
        let cue = !attr(&source, "_center_ref")?.is_none()
            && !cursor.is_none()
            && preview_here
            && truthy(&tools.getattr("tool")?, "uses_snap")?
            && !attr(&attr(&view, "last_snap")?, "kind")?.eq("center")?;
        let reference = if cue { source.call_method0("_valid_center_ref")? } else { none(py) };
        if !reference.is_none() || truthy(&view, "_had_center_cue")? {
            let shown = reference.is_truthy()?;
            let at = if cursor.is_truthy()? { cursor } else { (0., 0.).into_bound_py_any(py)? };
            if shown {
                renderer.call_method1("set_center_cue", (reference.get_item(0)?.call_method0("toTuple")?, reference.get_item(1)?, at, &scale))?;
            } else {
                renderer.call_method1("set_center_cue", (py.None(), 0., at, &scale))?;
            }
            view.setattr("_had_center_cue", !reference.is_none())?;
        }
    }
    if annotations || primitives.is_truthy()? || section_widgets || truthy(&view, "_had_overlay")? {
        let before = perf_counter(py)?;
        let (w, h, s): (f64, f64, f64) = (width.extract()?, height.extract()?, scale.extract()?);
        let draw = if annotations { tools.getattr("draw_annotations")? } else { none(py) };
        // One canvas while the size holds: clear() blanks only what the
        // last frame drew, instead of a new window-sized image a frame.
        let size = (((w * s) as i64).max(1), ((h * s) as i64).max(1), w).into_bound_py_any(py)?;
        let kept = view.getattr("__dict__")?.call_method1("get", ("_annotation_canvas",))?;
        let layer = if !kept.is_none() && kept.get_item(0)?.eq(&size)? {
            let layer = kept.get_item(1)?;
            layer.call_method0("clear")?;
            layer
        } else {
            let layer = py.import("ingetrazo_render")?.getattr("AnnotationCanvas")?.call1(size.clone().cast_into::<PyTuple>()?)?;
            view.setattr("_annotation_canvas", (size, &layer))?;
            layer
        };
        py.import("views.wgpu_overlay")?.call_method1("paint_layer", (&layer, &primitives, draw))?;
        if section_widgets {
            view.call_method1("_draw_section_widgets", (&layer,))?;
        }
        renderer.call_method1("set_annotation_layer", (layer,))?;
        view.setattr("overlay_ms", (perf_counter(py)? - before) * 1000.)?;
        view.setattr("_had_overlay", annotations || primitives.is_truthy()? || section_widgets)?;
    }
    // End R3c-2a section widgets.
    let ui = py.import("views.wgpu_ui")?;
    ui.call_method1("before_frame", (&view,))?;
    let timing = renderer.call_method0("render")?;
    let (cpu, gpu) = (timing.get_item(0)?, timing.get_item(1)?);
    if attr(&renderer, "needs_shadow_fit")?.is(&pyo3::types::PyBool::new(py, true)) && !settle.call_method0("isActive")?.is_truthy()? {
        settle.call_method1("start", (80,))?;
    }
    ui.call_method1("after_frame", (&view,))?;
    view.setattr("_pick_frame_dirty", false)?;
    let wall = (perf_counter(py)? - start) * 1000.;
    view.getattr("frameReady")?.call_method1("emit", (&cpu, &gpu, wall))?;
    (cpu, gpu, wall).into_bound_py_any(py)
}

/// The GL view's depth fit (`DocumentInteraction._fit_scene_depth`) on the
/// shown placements' box. Reverse-Z already keeps millimetres apart at any
/// distance here, so this only keeps the shared camera's planes the same in
/// both views; the same refusals apply, and a live mesh tool preview (the
/// renderer's preview mesh) refuses it too.
fn fit_depth<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let camera = view.getattr("camera")?;
    camera.setattr("_depth_range", py.None())?;
    let source = view.getattr("source_viewport")?;
    let packed = view.getattr("packed")?;
    if source.is_none() || packed.is_none() || packed.getattr("instances")?.len()? == 0
        || view.getattr("tools")?.getattr("mode")?.eq("mesh")? || truthy(&view, "preview_faces")?
    {
        return Ok(none(py));
    }
    let interaction = py.import("views.document_interaction")?.getattr("DocumentInteraction")?;
    let refused = match interaction.call_method1("_depth_fit_refused", (&source,)) {
        Ok(refused) => refused.is_truthy()?,
        Err(error) if error.is_instance_of::<pyo3::exceptions::PyException>(py) => return Ok(none(py)),
        Err(error) => return Err(error),
    };
    let np = py.import("numpy")?;
    let bounds = np.call_method1("asarray", (packed.getattr("world_bounds")?, py.get_type::<pyo3::types::PyFloat>()))?;
    let (low, high): ([f64; 3], [f64; 3]) = (bounds.get_item(0)?.extract()?, bounds.get_item(1)?.extract()?);
    if refused || !(0..3).all(|k| high[k] >= low[k]) || (0..3).all(|k| high[k] == low[k]) {
        return Ok(none(py));
    }
    camera.call_method1("fit_depth_range", (bounds.get_item(0)?.get_item(py.None())?, bounds.get_item(1)?.get_item(py.None())?))?;
    Ok(none(py))
}

fn draw_section_widgets<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, painter) = (a.get_item(0)?, a.get_item(1)?);
    let interaction = py.import("views.document_interaction")?.getattr("DocumentInteraction")?;
    let functools = py.import("functools")?;
    let kw = PyDict::new(py);
    kw.set_item("scene", view.getattr("scene")?)?;
    kw.set_item("camera", view.getattr("camera")?)?;
    kw.set_item("width", view.getattr("width")?)?;
    kw.set_item("height", view.getattr("height")?)?;
    let context = py.import("types")?.getattr("SimpleNamespace")?.call((), Some(&kw))?;
    // The painters read the frame's own projection off this stand-in.
    for name in ["section_plane_frame", "_clip_segment_front", "_world_to_pixel"] {
        context.setattr(name, functools.call_method1("partial", (interaction.getattr(name)?, &context))?)?;
    }
    interaction.call_method1("_draw_section_planes", (&context, painter))?;
    // The stand-in holds itself through those three: let it go.
    for name in ["section_plane_frame", "_clip_segment_front", "_world_to_pixel"] {
        context.delattr(name)?;
    }
    Ok(none(py))
}

#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("_present_frame", Mode::Method, present_frame, &["self", "*", "pick_only=False"], "", ""),
    ("_fit_depth", Mode::Method, fit_depth, &["self"], "", ""),
    ("_draw_section_widgets", Mode::Method, draw_section_widgets, &["self", "painter"], "", ""),
];
