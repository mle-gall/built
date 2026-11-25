// SPDX-License-Identifier: GPL-3.0-or-later
//! Scaffold lines and tool primitives of a resident frame: the world-space
//! line slots (`views.wgpu_lines`), the logical-pixel primitives and the
//! snap marker (`views.wgpu_overlay`), and the edit context's rest rows and
//! boxes (`views.wgpu_edit_context`) (render-glue lane; from the Python
//! modules of those names).
use super::*;
use numpy::PyReadonlyArray3;
use pyo3::types::PySlice;

fn attr<'py>(o: &Obj<'py>, name: &str) -> Reply<'py> {
    optional(o, name)
}
fn truthy(o: &Obj<'_>, name: &str) -> PyResult<bool> {
    attr_flag(o, name, false)
}
fn id(o: &Obj<'_>) -> usize {
    o.as_ptr() as usize
}
fn np<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyModule>> {
    py.import("numpy")
}
fn all<'py>(py: Python<'py>) -> Bound<'py, PySlice> {
    PySlice::full(py)
}
fn empty<'py>(py: Python<'py>) -> Bound<'py, PyList> {
    PyList::empty(py)
}
fn number(value: &Obj<'_>) -> PyResult<f64> {
    value.py().get_type::<pyo3::types::PyFloat>().call1((value,))?.extract()
}
fn interaction<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyModule>> {
    py.import("views.document_interaction")
}
/// `(N, 2, 3)` of float32.
fn pairs_of<'py>(py: Python<'py>, values: impl IntoPyObject<'py>) -> Reply<'py> {
    let numpy = np(py)?;
    numpy.call_method1("asarray", (values, numpy.getattr("float32")?))?.call_method1("reshape", (-1, 2, 3))
}
/// The ids of the groups a Move or Rotate drag is previewing.
fn moving_groups<'py>(view: &Obj<'py>) -> PyResult<Bound<'py, PySet>> {
    let py = view.py();
    let moving = PySet::empty(py)?;
    let tools = view.getattr("__dict__")?.call_method1("get", ("tools",))?;
    for held in [attr(&view.getattr("source_viewport")?, "_preview_groups")?, attr(&tools, "_moving_groups")?] {
        if held.is_truthy()? {
            for group in held.try_iter()? {
                moving.add(id(&group?))?;
            }
        }
    }
    Ok(moving)
}
/// `source is not None and source.plano_style is None and
/// source.style_override is None`.
fn scaffold<'py>(view: &Obj<'py>) -> PyResult<bool> {
    let source = view.getattr("source_viewport")?;
    Ok(!source.is_none() && source.getattr("plano_style")?.is_none() && source.getattr("style_override")?.is_none())
}
/// GL's scaffold dash in window pixels, from the field of view.
fn scaffold_dash(view: &Obj<'_>, height: f64) -> PyResult<f64> {
    let fov: f64 = view.getattr("camera")?.getattr("fov_deg")?.extract()?;
    Ok((0.003 * height / (fov.to_radians() / 2.).tan()).max(3.).min(20.))
}
fn selected_groups<'py>(view: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let py = view.py();
    let group = py.import("core.group")?.getattr("Group")?;
    let groups = empty(py);
    for entity in view.getattr("scene")?.getattr("selection")?.try_iter()? {
        let entity = entity?;
        if entity.is_instance(&group)? {
            groups.append(entity)?;
        }
    }
    Ok(groups)
}
/// The segment's two ends on screen once clipped in front of the camera,
/// `None` when either is off it.
fn on_screen<'py>(view: &Obj<'py>, from: Obj<'py>, to: Obj<'py>) -> PyResult<Option<Bound<'py, PyTuple>>> {
    let py = view.py();
    let segment = interaction(py)?.getattr("DocumentInteraction")?.call_method1("_clip_segment_front", (view, from, to))?;
    if segment.is_none() {
        return Ok(None);
    }
    let mut points = Vec::new();
    for point in segment.try_iter()? {
        let pixel = view.call_method1("_world_to_pixel", (point?,))?;
        if pixel.is_none() {
            return Ok(None);
        }
        points.push(pixel);
    }
    Ok(Some(PyTuple::new(py, points)?))
}
fn primitive<'py>(py: Python<'py>, args: impl pyo3::call::PyCallArgs<'py>, extra: Option<Bound<'py, PyDict>>) -> Reply<'py> {
    py.import("views.wgpu_overlay")?.getattr("Primitive")?.call(args, extra.as_ref())
}
fn vec3<'py>(py: Python<'py>, xyz: &Obj<'py>) -> Reply<'py> {
    py.import("ingetrazo_core")?.getattr("Vec3")?.call1(positional(xyz)?)
}

// ---- views.wgpu_lines -------------------------------------------------------

fn lines<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyModule>> {
    py.import("views.wgpu_lines")
}
fn no_lines<'py>(py: Python<'py>) -> Reply<'py> {
    (py.None(), lines(py)?.getattr("EMPTY")?).into_bound_py_any(py)
}
fn make_segments<'py>(py: Python<'py>, pairs: impl IntoPyObject<'py>, rgba: impl IntoPyObject<'py>, width: f64, dash: f64) -> Reply<'py> {
    lines(py)?.call_method1("segments", (pairs, rgba, width, dash))
}

/// `(N, 2, 3)` world endpoints as SEGMENT records. `width`, `dash` and
/// `gap` are physical pixels; `dash` 0 is a solid line.
fn segments<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (dash, gap) = (a.get_item(3)?, a.get_item(4)?);
    let pairs = pairs_of(py, a.get_item(0)?)?;
    let out = np(py)?.call_method1("zeros", (pairs.len()?, lines(py)?.getattr("SEGMENT")?))?;
    out.set_item("a", pairs.get_item((all(py), 0))?)?;
    out.set_item("b", pairs.get_item((all(py), 1))?)?;
    out.set_item("color", a.get_item(1)?)?;
    out.set_item("width", a.get_item(2)?)?;
    out.set_item("dash", &dash)?;
    out.set_item("gap", if gap.is_none() { dash } else { gap })?;
    Ok(out)
}

/// GL draws no scaffolding on a plan sheet or a styled composer frame.
pub(super) fn scaffold_shown<'py>(a: &Args<'py>) -> Reply<'py> {
    scaffold(&a.get_item(0)?)?.into_bound_py_any(a.py())
}

/// `(key, segments)`: the twelve dashed edges of the open group's box,
/// GL's `_draw_scaffold` dash (window pixels from the field of view).
fn edit_box<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let module = lines(py)?;
    let corners = if direct(py, scaffold_shown, (&view,))?.is_truthy()? { view.call_method0("_edit_group_box_corners")? } else { none(py) };
    if corners.is_none() {
        return no_lines(py);
    }
    let height = view.call_method0("height")?.extract::<f64>()? * view.call_method0("devicePixelRatioF")?.extract::<f64>()?;
    let dash = scaffold_dash(&view, height)?;
    let points = empty(py);
    for corner in corners.try_iter()? {
        points.append(corner?.call_method0("toTuple")?)?;
    }
    let edges = np(py)?.call_method1("asarray", (interaction(py)?.call_method1("_box_edge_vertices", (&corners,))?,))?;
    ((points.to_tuple(), dash), make_segments(py, edges, module.getattr("EDIT_BOX_RGBA")?, 1.0, dash)?).into_bound_py_any(py)
}

/// `(key, segments)`: GL's world-space guide dashes, keyed as GL keys its
/// guide cache (camera distance, target and eye; the guides; which are
/// selected), so a hover repaint reuses the upload.
fn guides<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let module = lines(py)?;
    let (scene, camera) = (view.getattr("scene")?, view.getattr("camera")?);
    let shown = if direct(py, scaffold_shown, (&view,))?.is_truthy()? { interaction(py)?.call_method1("shown_guides", (&scene,))? } else { empty(py).into_any() };
    if !shown.is_truthy()? {
        return no_lines(py);
    }
    let xyz = |vector: Obj<'py>| -> Reply<'py> {
        if vector.is_none() {
            return Ok(vector);
        }
        (vector.call_method0("x")?, vector.call_method0("y")?, vector.call_method0("z")?).into_bound_py_any(py)
    };
    let selection = scene.getattr("selection")?;
    let (described, picked) = (empty(py), empty(py));
    for guide in shown.try_iter()? {
        let guide = guide?;
        described.append((id(&guide), xyz(guide.getattr("point")?)?, xyz(attr(&guide, "direction")?)?, xyz(attr(&guide, "origin")?)?))?;
        picked.append(selection.contains(&guide)?)?;
    }
    let eye = if camera.getattr("perspective")?.is_truthy()? { xyz(camera.call_method0("eye")?)? } else { none(py) };
    let key = (number(&camera.getattr("distance")?)?, xyz(camera.getattr("target")?)?, eye, described.to_tuple(), picked.to_tuple()).into_bound_py_any(py)?;
    let cache = view.getattr("__dict__")?.call_method1("get", ("_guide_lines",))?;
    if !cache.is_none() && cache.get_item(0)?.eq(&key)? {
        return (key, cache.get_item(1)?).into_bound_py_any(py);
    }
    let [pairs, selected] = unpack::<2>(&py.import("views.wgpu_annotations")?.call_method1("guide_segments", (&scene, &camera))?)?;
    let out = module.call_method1("segments", (pairs, module.getattr("GUIDE_RGBA")?))?;
    out.get_item("color")?.set_item(selected, module.getattr("SELECTED_RGBA")?)?;
    view.setattr("_guide_lines", (&key, &out))?;
    (key, out).into_bound_py_any(py)
}

/// Soft and hidden edges of one mesh in its own coordinates, layer
/// filtered: GL's `_hidden_edge_draw(mesh, 'selected_hidden')`. A lazy
/// imported mesh answers from its plan, never materialising its faces.
pub(super) fn local_hidden_edges<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (scene, mesh) = (a.get_item(0)?, a.get_item(1)?);
    let numpy = np(py)?;
    let namespace = py.import("types")?.getattr("SimpleNamespace")?;
    let plan = py.import("ingetrazo_core")?.call_method1("plan_of", (&mesh,))?;
    if plan.is_none() {
        let pairs = empty(py);
        for edge in mesh.getattr("edges")?.try_iter()? {
            let edge = edge?;
            if (edge.getattr("soft")?.is_truthy()? || edge.getattr("hidden")?.is_truthy()?) && scene.call_method1("_layer_state", (&edge,))?.get_item(0)?.is_truthy()? {
                pairs.append((edge.getattr("a")?.call_method0("toTuple")?, edge.getattr("b")?.call_method0("toTuple")?))?;
            }
        }
        return pairs_of(py, pairs);
    }
    let mut mask = plan.getattr("soft")?.bitor(plan.getattr("hidden")?)?;
    let layers = plan.getattr("layer")?;
    let mut layered = false;
    for layer in layers.try_iter()? {
        layered |= !layer?.is_none();
    }
    if layered {
        let visible = PyDict::new(py);
        let shown = empty(py);
        for layer in layers.try_iter()? {
            let layer = layer?;
            if !visible.contains(&layer)? {
                let on = scene.call_method1("_layer_state", (namespace.call((), Some(&fields!(py; "layer" => &layer)))?,))?.get_item(0)?;
                visible.set_item(&layer, on)?;
            }
            shown.append(visible.as_any().get_item(&layer)?)?;
        }
        mask = mask.bitand(numpy.call_method1("array", (shown, py.get_type::<pyo3::types::PyBool>()))?)?;
    }
    let positions = numpy.call_method1("asarray", (plan.getattr("pos")?, numpy.getattr("float32")?))?;
    positions.get_item(numpy.call_method1("asarray", (plan.getattr("edges")?,))?.get_item(mask)?)?.call_method1("reshape", (-1, 2, 3))
}

/// `(key, segments)`: the selected groups' soft and hidden edges, dashed
/// orange, while View > Hidden Geometry is on (GL's
/// `_draw_selected_group_edges` `selected_hidden` kind: 3 px dashes times
/// the pixel ratio). Placed by the rows' world matrices, so every instance
/// of a shared component shows its own.
fn selected_hidden<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let module = lines(py)?;
    let numpy = np(py)?;
    let (scene, packed) = (view.getattr("scene")?, attr(&view, "packed")?);
    if !truthy(&scene, "show_hidden_geometry")? || packed.is_none() || !direct(py, scaffold_shown, (&view,))?.is_truthy()? {
        return no_lines(py);
    }
    let groups = selected_groups(&view)?;
    if groups.is_empty() {
        return no_lines(py);
    }
    let moving = moving_groups(&view)?;
    let edits = py.import("ingetrazo_core")?.getattr("tag_edits")?;
    let layers = empty(py);
    if let Some(all_layers) = scene.getattr_opt("layers")? {
        for layer in all_layers.try_iter()? {
            let layer = layer?;
            layers.append((layer.getattr("key")?, edits.call_method1("effective_visible", (&scene, &layer))?))?;
        }
    }
    let version = scene.getattr("version")?.sub(scene.getattr("view_version")?)?;
    let frozen = py.get_type::<pyo3::types::PyFrozenSet>();
    let ids = empty(py);
    for group in groups.iter() {
        ids.append(id(&group))?;
    }
    let key = (version, id(&packed), frozen.call1((ids,))?, frozen.call1((&moving,))?, layers.to_tuple()).into_bound_py_any(py)?;
    let cache = view.getattr("__dict__")?.call_method1("get", ("_hidden_lines",))?;
    if !cache.is_none() && cache.get_item(0)?.eq(&key)? {
        return (key, cache.get_item(1)?).into_bound_py_any(py);
    }
    let visible = scene.getattr_opt("entity_visible")?;
    let rows = PySet::empty(py)?;
    for group in groups.iter() {
        rows.call_method1("update", (view.call_method1("_rows_for_group", (group,))?,))?;
    }
    let numpy_float32 = numpy.getattr("float32")?;
    let placed_groups = packed.getattr("groups")?;
    // Instance matrices are stored column by column (the Rust ABI).
    let worlds: PyReadonlyArray3<'_, f32> = numpy.call_method1("asarray", (packed.getattr("instances")?.get_item("world")?, &numpy_float32))?.extract()?;
    let worlds = worlds.as_array();
    let dash = (3.0 * view.call_method0("devicePixelRatioF")?.extract::<f64>()?.max(1.0)) as f32;
    let rgba: [u8; 4] = module.getattr("SELECTED_RGBA")?.extract()?;
    // Each mesh's own soft and hidden edges, read once; then one record
    // per edge per placement: both ends through the row's matrix, in
    // binary32 as NumPy's product gave them.
    let mut local: Vec<(usize, Option<PyReadonlyArray3<'py, f32>>)> = Vec::new();
    let mut records: Vec<u8> = Vec::new();
    for row in py.import("builtins")?.call_method1("sorted", (rows,))?.try_iter()? {
        let row = row?;
        let placed = placed_groups.get_item(&row)?;
        if placed.is_none() || truthy(&placed, "billboard")? || moving.contains(id(&placed))? {
            continue;
        }
        if let Some(visible) = &visible {
            if !visible.call1((&placed,))?.is_truthy()? {
                continue;
            }
        }
        let mesh = placed.getattr("mesh")?;
        let known = match local.iter().position(|(mesh_id, _)| *mesh_id == id(&mesh)) {
            Some(known) => known,
            None => {
                let pairs = direct(py, local_hidden_edges, (&scene, &mesh))?;
                let pairs = if pairs.len()? == 0 { None } else { Some(numpy.call_method1("ascontiguousarray", (pairs, &numpy_float32))?.extract()?) };
                local.push((id(&mesh), pairs));
                local.len() - 1
            }
        };
        let Some(pairs) = &local[known].1 else {
            continue;
        };
        let row: usize = row.extract()?;
        let w = |from: usize, to: usize| worlds[[row, from, to]];
        records.reserve(pairs.as_array().dim().0 * 40);
        for pair in pairs.as_array().outer_iter() {
            for end in 0..2 {
                for value in super::frame_scene::through([pair[[end, 0]], pair[[end, 1]], pair[[end, 2]]], &w) {
                    records.extend_from_slice(&value.to_le_bytes());
                }
            }
            records.extend_from_slice(&rgba);
            for value in [1.0f32, dash, dash] {
                records.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    let out = numpy.call_method1("frombuffer", (pyo3::types::PyByteArray::new(py, &records), module.getattr("SEGMENT")?))?;
    view.setattr("_hidden_lines", (&key, &out))?;
    (key, out).into_bound_py_any(py)
}

/// Send each slot whose key changed; return the slots sent (tests and
/// timing). Called once per frame; an unchanged slot costs its key.
fn lines_sync<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let numpy = np(py)?;
    let renderer = view.getattr("renderer")?;
    // A new renderer starts with empty slots (None keys); its keys are its own.
    let held = view.getattr("__dict__")?.call_method1("get", ("_line_keys",))?;
    let keys = if !held.is_none() && held.get_item(0)?.is(&renderer) {
        held.get_item(1)?
    } else {
        let keys = PyDict::new(py).into_any();
        view.setattr("_line_keys", (&renderer, &keys))?;
        keys
    };
    let sent = empty(py);
    for slot in lines(py)?.getattr("SLOTS")?.try_iter()? {
        let [slot, build, above_edges] = unpack::<3>(&slot?)?;
        let [key, data] = unpack::<2>(&build.call1((&view,))?)?;
        if keys.call_method1("get", (&slot,))?.ne(&key)? {
            let bytes = numpy.call_method1("ascontiguousarray", (data,))?.call_method1("view", (numpy.getattr("uint8")?,))?.call_method1("reshape", (-1,))?;
            renderer.call_method1("set_lines", (&slot, bytes, above_edges))?;
            keys.set_item(&slot, key)?;
            sent.append(slot)?;
        }
    }
    Ok(sent.into_any())
}

pub(super) fn install_lines(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    let numpy = np(py)?;
    module.add(
        "__doc__",
        "World-space scaffold lines for the native line pass (``lines.rs``).\n\nGL draws the open group's dashed box, the guide lines and a selected\ngroup's hidden edges as GL_LINES in its scene pass: depth-tested, so the\nparts behind faces are hidden. The Metal overlay has no depth, so these\nshowed through the model there. They now go to ``Renderer.set_lines``,\none named slot each, and are uploaded only when their key changes.\n\nThe dash of the box and of the hidden edges is a screen pattern computed\non the GPU from the projected segment (GL's ``u_stipple 4``), so a camera\nmove re-uploads nothing for them. The box's corners do move with the\ncamera (GL pads it by a screen gutter), which re-sends twelve segments.\nGuides keep GL's world-space dashes (``_guide_vertices``), which GL\nitself rebuilds when the camera changes; they are sent as solid pieces.\n",
    )?;
    let segment = numpy.call_method1(
        "dtype",
        (PyList::new(
            py,
            [
                ("a", "<f4", 3).into_bound_py_any(py)?,
                ("b", "<f4", 3).into_bound_py_any(py)?,
                ("color", "u1", 4).into_bound_py_any(py)?,
                ("width", "<f4").into_bound_py_any(py)?,
                ("dash", "<f4").into_bound_py_any(py)?,
                ("gap", "<f4").into_bound_py_any(py)?,
            ],
        )?,),
    )?;
    if segment.getattr("itemsize")?.ne(40)? {
        return Err(pyo3::exceptions::PyAssertionError::new_err("a line segment record is 40 bytes"));
    }
    module.add("EMPTY", numpy.call_method1("zeros", (0, &segment))?)?;
    module.add("SEGMENT", segment)?;
    module.add("EDIT_BOX_RGBA", (90, 110, 140, 255))?; // GL's (90, 110, 140) / 255
    module.add("GUIDE_RGBA", (69, 89, 120, 255))?; // GL's (.27, .35, .47)
    module.add("SELECTED_RGBA", (242, 115, 41, 255))?; // GL's (.95, .45, .16)
    install(
        module.dict().as_any(),
        &module.dict(),
        &[
            ("segments", Mode::Function, segments, &["pairs", "rgba", "width=1.0", "dash=0.0", "gap=None"], "", ""),
            ("edit_box", Mode::Function, edit_box, &["view"], "", ""),
            ("guides", Mode::Function, guides, &["view"], "", ""),
            ("selected_hidden", Mode::Function, selected_hidden, &["view"], "", ""),
            ("sync", Mode::Function, lines_sync, &["view"], "", ""),
        ],
    )?;
    // (slot, builder, above_edges). GL draws the box and the guides in
    // `_draw_scaffold`, before the scene edges, which paint over them; the
    // selected hidden edges come after the scene edges.
    module.add(
        "SLOTS",
        (
            ("edit_box", module.getattr("edit_box")?, false),
            ("guides", module.getattr("guides")?, false),
            ("selected_hidden", module.getattr("selected_hidden")?, true),
        ),
    )
}

// ---- views.wgpu_overlay -----------------------------------------------------

fn overlay<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyModule>> {
    py.import("views.wgpu_overlay")
}

/// The primitives on a fresh annotation canvas, as a straight-alpha
/// `SurveyRaster` at physical resolution (scripts and tests read its
/// pixels).
fn rasterize<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (width, height, scale) = (number(&a.get_item(1)?)?, number(&a.get_item(2)?)?, number(&a.get_item(3)?)?);
    let render = py.import("ingetrazo_render")?;
    let (w, h) = (((width * scale) as i64).max(1), ((height * scale) as i64).max(1));
    let canvas = render.getattr("AnnotationCanvas")?.call1((w, h, width))?;
    overlay(py)?.call_method1("paint_layer", (&canvas, a.get_item(0)?, a.get_item(4)?))?;
    let blank = py.get_type::<pyo3::types::PyBytes>().call1((w * h * 4,))?;
    render.getattr("SurveyRaster")?.call1((w, h, canvas.call_method1("composite", (blank,))?))
}

/// The document's annotations (`annotations(painter)`), then the tool
/// primitives, on the annotation canvas (`AnnotationCanvas.draw_primitives`).
pub(super) fn paint_layer<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (painter, primitives, annotations) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?);
    if !annotations.is_none() {
        annotations.call1((&painter,))?;
    }
    if primitives.is_truthy()? {
        let rows = empty(py);
        for primitive in primitives.try_iter()? {
            let primitive = primitive?;
            let points = empty(py);
            for point in primitive.getattr("points")?.try_iter()? {
                let [x, y] = unpack::<2>(&point?)?;
                points.append((number(&x)?, number(&y)?))?;
            }
            let dash = empty(py);
            for length in primitive.getattr("dash")?.try_iter()? {
                dash.append(number(&length?)?)?;
            }
            let fill = primitive.getattr("fill")?;
            let pen = primitive.getattr("pen_style")?;
            rows.append((
                primitive.getattr("kind")?,
                points,
                direct(py, colour, (primitive.getattr("color")?,))?,
                number(&primitive.getattr("width")?)?,
                primitive.getattr("text")?.str()?,
                if fill.is_truthy()? { direct(py, colour, (fill,))? } else { none(py) },
                dash,
                pen.getattr_opt("value")?.unwrap_or(pen).call_method0("__int__")?,
                primitive.getattr("snap_kind")?,
            ))?;
        }
        painter.call_method1("draw_primitives", (rows,))?;
    }
    Ok(none(py))
}

/// A colour string as Qt names one, from a string or a QColor.
pub(super) fn colour<'py>(a: &Args<'py>) -> Reply<'py> {
    let given = a.get_item(0)?;
    if given.is_instance_of::<PyString>() {
        return Ok(given);
    }
    let (r, g, b, alpha): (u8, u8, u8, u8) = given.call_method0("getRgb")?.extract()?;
    format!("#{alpha:02x}{r:02x}{g:02x}{b:02x}").into_bound_py_any(a.py())
}

/// The image's straight RGBA bytes as a flat array.
fn pixels<'py>(a: &Args<'py>) -> Reply<'py> {
    let numpy = np(a.py())?;
    numpy.call_method("frombuffer", (a.get_item(0)?.call_method0("rgba")?,), Some(&fields!(a.py(); "dtype" => numpy.getattr("uint8")?)))
}

/// Return one haloed marker; color accepts a Qt color string or QColor.
///
/// P1 should prefer draw_snap_indicator for the full SnapResult, including
/// guide colors, contextual/translated labels, and axis marker suppression.
/// The legacy vertex alias still means endpoint.
fn snap_glyph<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let kind = a.get_item(1)?;
    let kind = if kind.eq("vertex")? { "endpoint".into_bound_py_any(py)? } else { kind };
    primitive(py, ("snap", (a.get_item(0)?,), a.get_item(2)?), Some(fields!(py; "snap_kind" => kind)))
}

/// Return GL's direction-sensitive box in logical pixels (no drag
/// threshold).
///
/// P1 calls this after its drag threshold, with unsorted start/end tuples.
/// Set zoom=True for Zoom Window; append the returned Primitive.
fn selection_rectangle<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (start, end) = (a.get_item(0)?, a.get_item(1)?);
    let zoom = a.get_item(2)?.is_truthy()?;
    let crossing = end.get_item(0)?.lt(start.get_item(0)?)?;
    let colour = if zoom {
        "#f37329"
    } else if crossing {
        "#289e5c"
    } else {
        "#3366c7"
    };
    let fill = format!("#{}{}", if zoom { "1a" } else { "1c" }, &colour[1..]);
    let input = py.import("ingetrazo_core")?.getattr("Input")?;
    let pen = input.getattr(if zoom || crossing { "DashLine" } else { "SolidLine" })?;
    primitive(py, ("rectangle", (start, end), colour, 1.5), Some(fields!(py; "fill" => fill, "pen_style" => pen)))
}

/// Projected camera axes; callers supply three normalized screen vectors.
fn overlay_axes<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let origin = a.get_item(0)?;
    let (x, y): (f64, f64) = origin.extract()?;
    let length = number(&a.get_item(2)?)?;
    let result = empty(py);
    for ((label, colour), direction) in [("X", "#ff5555"), ("Y", "#55dd55"), ("Z", "#5599ff")].into_iter().zip(a.get_item(1)?.try_iter()?) {
        let (dx, dy): (f64, f64) = direction?.extract()?;
        let norm = dx.hypot(dy).max(1e-12);
        let end = (x + length * dx / norm, y + length * dy / norm);
        result.append(primitive(py, ("polyline", (&origin, end), colour), None)?)?;
        result.append(primitive(py, ("text", (end,), colour), Some(fields!(py; "text" => label)))?)?;
    }
    Ok(result.into_any())
}

/// Paint a SnapResult on the annotation canvas: its guides, glyph and
/// label (`AnnotationCanvas.draw_snap`, annotation_canvas.rs).
///
/// `project` maps world to logical pixels and `clip_line` clips a screen
/// segment, (a, b) -> (a, b) or None. Colours come from the SnapResult,
/// guides included. Axis snaps draw nothing.
fn draw_snap_indicator<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (painter, snap, project, clip_line, labels, scene) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?, a.get_item(3)?, a.get_item(4)?, a.get_item(5)?);
    let kind = snap.getattr("kind")?;
    if kind.eq("axis_inference")? || kind.eq("axis")? {
        return Ok(none(py));
    }
    let pixel = project.call1((snap.getattr("point")?,))?;
    if pixel.is_none() {
        return Ok(none(py));
    }
    let [r, g, b] = unpack::<3>(&snap.getattr("color")?)?;
    let rgb = (r, g, b).into_bound_py_any(py)?;
    let guides = empty(py);
    let guide = snap.getattr("guide")?;
    if !guide.is_none() {
        // The 'from point' guide is axis-coloured while the point is green.
        let own = snap.getattr("guide_color")?;
        let [from, to] = unpack::<2>(&guide)?;
        guides.append((from, to, if own.is_none() { rgb.clone() } else { own }))?;
    }
    let more = snap.getattr("guides")?;
    if more.is_truthy()? {
        guides.call_method1("extend", (more,))?;
    }
    let tuple = |value: Obj<'py>| py.get_type::<PyTuple>().call1((value,));
    let segments = empty(py);
    for guide in guides.iter() {
        let [from, to, colour] = unpack::<3>(&guide)?;
        let (from, to) = (project.call1((from,))?, project.call1((to,))?);
        if from.is_none() || to.is_none() {
            continue;
        }
        let shown = clip_line.call1((from, to))?;
        if !shown.is_none() {
            segments.append((tuple(shown.get_item(0)?)?, tuple(shown.get_item(1)?)?, tuple(colour.get_item(PySlice::new(py, 0, 3, 1))?)?))?;
        }
    }
    let label = if labels.is_truthy()? { direct(py, snap_label, (&snap, scene))? } else { none(py) };
    painter.call_method1("draw_snap", (kind, tuple(pixel)?, rgb, segments, label))?;
    Ok(none(py))
}

/// The words beside the marker ("On Edge", "in group"), translated.
pub(super) fn snap_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (snap, scene) = (a.get_item(0)?, a.get_item(1)?);
    let tr = py.import("ingetrazo_core.i18n")?.getattr("tr")?;
    let kind = snap.getattr("kind")?;
    let mut label = interaction(py)?.getattr("DocumentInteraction")?.getattr("_SNAP_LABELS")?.call_method1("get", (&kind,))?;
    if kind.eq("aligned")? && snap.getattr("axis")?.eq("z")? {
        label = "Level with point".into_bound_py_any(py)?;
    }
    if label.is_truthy()? {
        label = tr.call1((label,))?;
    }
    let own = attr(&snap, "label")?;
    if own.is_truthy()? {
        label = own; // an extension's own words
    }
    if !label.is_truthy()? {
        return Ok(none(py));
    }
    let context = attr(&snap, "context")?;
    let suffix = if context.eq("component")? {
        Some("in component")
    } else if context.eq("group")? {
        Some("in group")
    } else if context.eq("outside")? && !scene.is_none() {
        // As DocumentInteraction._snap_context_suffix (#1327): geometry
        // outside the group or component open for editing.
        let component = !scene.getattr("edit_group")?.is_none() && !scene.call_method0("edit_definition")?.is_none();
        Some(if component { "outside the component" } else { "outside the group" })
    } else {
        None
    };
    match suffix {
        Some(suffix) => label.add(" ")?.add(tr.call1((suffix,))?),
        None => Ok(label),
    }
}

pub(super) fn install_overlay(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add(
        "__doc__",
        "Logical-pixel tool primitives and the snap marker on the annotation\ncanvas (``ingetrazo_render.AnnotationCanvas``), which the renderer\ncomposites over the scene.",
    )?;
    modules::import_names(module, "ingetrazo_core", &[("Input", "Qt")])?;
    let input = module.getattr("Qt")?;
    let builtins = py.import("builtins")?;
    let (text, tuple, float, object) = (builtins.getattr("str")?, builtins.getattr("tuple")?, builtins.getattr("float")?, builtins.getattr("object")?);
    let fields = PyList::new(
        py,
        [
            ("kind", &text).into_bound_py_any(py)?,
            ("points", &tuple).into_bound_py_any(py)?,
            ("color", &text, "#00dfff").into_bound_py_any(py)?,
            ("width", &float, 1.5).into_bound_py_any(py)?,
            ("text", &text, "").into_bound_py_any(py)?,
            ("fill", &object, py.None()).into_bound_py_any(py)?,
            ("dash", &tuple, PyTuple::empty(py)).into_bound_py_any(py)?,
            ("pen_style", &object, input.getattr("SolidLine")?).into_bound_py_any(py)?,
            ("snap_kind", &text, "").into_bound_py_any(py)?,
        ],
    )?;
    let primitive = py.import("dataclasses")?.call_method("make_dataclass", ("Primitive", fields), Some(&fields!(py; "frozen" => true)))?;
    primitive.setattr("__module__", module.name()?)?;
    module.add("Primitive", primitive)?;
    install(
        module.dict().as_any(),
        &module.dict(),
        &[
            ("rasterize", Mode::Function, rasterize, &["primitives", "width", "height", "scale", "annotations=None"], "", ""),
            ("paint_layer", Mode::Function, paint_layer, &["painter", "primitives", "annotations=None"], "", ""),
            ("pixels", Mode::Function, pixels, &["image"], "", ""),
            ("snap_glyph", Mode::Function, snap_glyph, &["point", "kind", "color='#00dfff'"], "", ""),
            ("selection_rectangle", Mode::Function, selection_rectangle, &["start", "end", "*", "zoom=False"], "", ""),
            ("axes", Mode::Function, overlay_axes, &["origin", "directions", "length=32"], "", ""),
            ("draw_snap_indicator", Mode::Function, draw_snap_indicator, &["painter", "snap", "project", "clip_line", "*", "labels=True", "scene=None"], "", ""),
        ],
    )
}

// ---- views.wgpu_edit_context ------------------------------------------------

fn edit_context<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyModule>> {
    py.import("views.wgpu_edit_context")
}

fn rest_rows<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (groups, context) = (a.get_item(0)?, a.get_item(1)?);
    let rows = empty(py);
    if context.is_none() {
        return Ok(rows.into_any());
    }
    let mut subject = std::collections::HashSet::new();
    for group in py.import("ingetrazo_core")?.call_method1("purge_iter_groups", ((&context,),))?.try_iter()? {
        subject.insert(id(&group?));
    }
    for (row, group) in groups.try_iter()?.enumerate() {
        if !subject.contains(&id(&group?)) {
            rows.append(row)?;
        }
    }
    Ok(rows.into_any())
}

fn box_primitives<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let result = empty(py);
    if !scaffold(&view)? {
        return Ok(result.into_any());
    }
    let corners = view.call_method0("_edit_group_box_corners")?;
    if corners.is_none() {
        return Ok(result.into_any());
    }
    let scale: f64 = view.call_method0("devicePixelRatioF")?.extract()?;
    let dash = scaffold_dash(&view, view.call_method0("height")?.extract::<f64>()? * scale)? / scale;
    let edges = np(py)?.call_method1("asarray", (interaction(py)?.call_method1("_box_edge_vertices", (&corners,))?,))?.call_method1("reshape", (-1, 2, 3))?;
    for edge in edges.try_iter()? {
        let [from, to] = unpack::<2>(&edge?)?;
        if let Some(points) = on_screen(&view, vec3(py, &from)?, vec3(py, &to)?)? {
            result.append(primitive(py, ("polyline", points, "#5a6e8c", 1. / scale), Some(fields!(py; "dash" => (dash * scale, dash * scale))))?)?;
        }
    }
    Ok(result.into_any())
}

/// `(key, segments)`: the box around each selected group, in the group's
/// own axes, as world segments `(N, 2, 3)`. GL's selection cue
/// (`_box_edges` in the selected-edges pass) had no Metal counterpart, so a
/// selected group only tinted; on the ride a support picked in a forest of
/// others gave no sense of its extent.
///
/// GL draws the box in the scene pass, depth-tested: the edges behind the
/// group are hidden. The Metal view sends these segments to the native
/// preview's edges (depth-tested) for the same look; `key` changes exactly
/// when the segments do, so the upload is skipped otherwise.
///
/// Skipped like GL: face-me billboards (their mesh quad is not what is on
/// screen) and groups a Move/Rotate drag is previewing. Boxes are cached
/// per group and document version: a selection change reuses them, an edit
/// (move, scale, geometry) measures again.
pub(super) fn selection_box_segments<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let module = edit_context(py)?;
    let no_boxes = module.getattr("_NO_BOXES")?;
    if !scaffold(&view)? {
        return Ok(no_boxes);
    }
    let groups = selected_groups(&view)?;
    if groups.is_empty() {
        return Ok(no_boxes);
    }
    let moving = moving_groups(&view)?;
    let scene = view.getattr("scene")?;
    let version = scene.getattr("version")?.sub(scene.getattr("view_version")?)?;
    let cache = view.getattr("__dict__")?.call_method1("setdefault", ("_selection_boxes", PyDict::new(py)))?;
    if cache.call_method1("get", (py.None(),))?.ne(&version)? {
        cache.call_method0("clear")?;
        cache.set_item(py.None(), &version)?;
    }
    let (shown, ids) = (empty(py), empty(py));
    for group in groups.iter() {
        if !truthy(&group, "billboard")? && !moving.contains(id(&group))? {
            ids.append(id(&group))?;
            shown.append(group)?;
        }
    }
    let key = (version, py.get_type::<pyo3::types::PyFrozenSet>().call1((ids,))?).into_bound_py_any(py)?;
    if cache.call_method1("get", ("key",))?.eq(&key)? {
        return (key, cache.get_item("segments")?).into_bound_py_any(py);
    }
    let measure = interaction(py)?.getattr("DocumentInteraction")?.getattr("_compute_obb")?;
    let corners_of = py.import("core.group")?.getattr("oriented_box_corners")?;
    let parts = empty(py);
    for group in shown.iter() {
        let mut corners = cache.call_method1("get", (&group,))?;
        if corners.is_none() {
            corners = corners_of.call1(positional(&measure.call1((&group,))?)?)?;
            cache.set_item(&group, &corners)?;
        }
        parts.append(pairs_of(py, interaction(py)?.call_method1("_box_edge_vertices", (corners,))?)?)?;
    }
    let segments = if parts.is_empty() { no_boxes.get_item(1)? } else { np(py)?.call_method1("concatenate", (parts,))? };
    cache.set_item("key", &key)?;
    cache.set_item("segments", &segments)?;
    (key, segments).into_bound_py_any(py)
}

/// `(key, VERTEX array)`: the boxes as native preview line vertices in
/// GL's selection orange, cached with the segments: a mesh tool's feedback
/// asks for them on every frame.
fn selection_box_vertices<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let numpy = np(py)?;
    let [key, segments] = unpack::<2>(&direct(py, selection_box_segments, (&view,))?)?;
    let cache = view.getattr("__dict__")?.call_method1("setdefault", ("_selection_boxes", PyDict::new(py)))?;
    if cache.call_method1("get", ("vertex_key",))?.ne(&key)? || !cache.contains("vertices")? {
        let vertices = numpy.call_method1("zeros", (segments.len()? * 2, py.import("ingetrazo_core")?.getattr("VERTEX")?))?;
        vertices.set_item("pos", numpy.call_method1("asarray", (&segments,))?.call_method1("reshape", (-1, 3))?)?;
        vertices.set_item("normal", (0, 0, 1))?;
        vertices.set_item("color", (242, 114, 40, 255))?; // GL's (.95, .45, .16)
        cache.set_item("vertex_key", &key)?;
        cache.set_item("vertices", vertices)?;
    }
    (key, cache.get_item("vertices")?).into_bound_py_any(py)
}

/// The same boxes as overlay lines, for a renderer without native preview
/// edges (no depth test there).
fn selection_box_primitives<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let segments = direct(py, selection_box_segments, (&view,))?.get_item(1)?;
    let scale: f64 = view.call_method0("devicePixelRatioF")?.extract()?;
    let result = empty(py);
    for segment in segments.try_iter()? {
        let [from, to] = unpack::<2>(&segment?)?;
        if let Some(points) = on_screen(&view, vec3(py, &from)?, vec3(py, &to)?)? {
            result.append(primitive(py, ("polyline", points, "#f2732a", 1.5 / scale), None)?)?;
        }
    }
    Ok(result.into_any())
}

/// The orange outline GL draws around a selected face-me figure, on the
/// quad as it faces the camera this frame (`_draw_billboard_outlines`). The
/// Metal view tinted the figure's rows instead, which GL deliberately does
/// not do (a stray plane through the figure), and gave no outline.
///
/// The quad is GL's own `_billboard_quad` on the source viewport: its
/// extent is cached per figure content, so this is four projections per
/// selected figure per frame; nothing when no figure is selected.
fn billboard_outline_primitives<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let result = empty(py);
    let source = view.getattr("source_viewport")?;
    let quad_of = attr(&source, "_billboard_quad")?;
    if quad_of.is_none() || !source.getattr("plano_style")?.is_none() || !source.getattr("style_override")?.is_none() {
        return Ok(result.into_any());
    }
    let visible = view.getattr("scene")?.getattr_opt("entity_visible")?;
    for group in selected_groups(&view)?.iter() {
        if !truthy(&group, "billboard")? {
            continue;
        }
        if let Some(visible) = &visible {
            if !visible.call1((&group,))?.is_truthy()? {
                continue;
            }
        }
        let quad = quad_of.call1((&group,))?;
        if quad.is_none() {
            continue;
        }
        let corners: Vec<Obj<'py>> = quad.get_item(0)?.try_iter()?.collect::<PyResult<_>>()?;
        for (at, from) in corners.iter().enumerate() {
            let to = &corners[(at + 1) % corners.len()];
            if let Some(points) = on_screen(&view, from.clone(), to.clone())? {
                // A GL line is one physical pixel; as the selection box.
                let width = 1.5 / view.call_method0("devicePixelRatioF")?.extract::<f64>()?;
                result.append(primitive(py, ("polyline", points, "#f2732a", width), None)?)?;
            }
        }
    }
    Ok(result.into_any())
}

pub(super) fn install_edit_context(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    let numpy = np(py)?;
    module.add("__doc__", "Edit-context membership and screen scaffolding shared by native captures.")?;
    module.add("_NO_BOXES", (py.None(), numpy.call_method1("empty", ((0, 2, 3), numpy.getattr("float32")?))?))?;
    install(
        module.dict().as_any(),
        &module.dict(),
        &[
            ("rest_rows", Mode::Function, rest_rows, &["groups", "context"], "", ""),
            ("box_primitives", Mode::Function, box_primitives, &["view"], "", ""),
            ("selection_box_vertices", Mode::Function, selection_box_vertices, &["view"], "", ""),
            ("selection_box_primitives", Mode::Function, selection_box_primitives, &["view"], "", ""),
            ("billboard_outline_primitives", Mode::Function, billboard_outline_primitives, &["view"], "", ""),
        ],
    )
}
