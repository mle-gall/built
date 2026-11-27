// SPDX-License-Identifier: GPL-3.0-or-later
//! `MetalView`: what is under the cursor (the hover and its snap), the edit
//! context's rest rows and boxes, and the selection's marks.
use super::metal_view::{attr, tool_frame, truthy};
use super::*;
use pyo3::types::PyCFunction;
use std::time::Instant;

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.
}
fn id(o: &Obj<'_>) -> usize {
    o.as_ptr() as usize
}
fn input<'py>(py: Python<'py>, name: &str) -> Reply<'py> {
    py.import("built_core")?.getattr("Input")?.getattr(name)
}
/// The modifiers given, `Qt.NoModifier` by default.
fn modifiers_of<'py>(given: Obj<'py>) -> Reply<'py> {
    if given.is_none() { input(given.py(), "NoModifier") } else { Ok(given) }
}
fn isinstance(o: &Obj<'_>, class: &Obj<'_>) -> PyResult<bool> {
    o.is_instance(class)
}
fn namespace<'py>(py: Python<'py>, fields: Bound<'py, PyDict>) -> Reply<'py> {
    py.import("types")?.getattr("SimpleNamespace")?.call((), Some(&fields))
}
/// The entity a tool or the document's viewport has under the cursor.
fn hovered_entity<'py>(view: &Obj<'py>) -> Reply<'py> {
    let source = attr(view, "source_viewport")?;
    let fallback = attr(&source, "_hover_entity")?;
    Ok(view.getattr("tools")?.getattr_opt("_mesh_hover")?.unwrap_or(fallback))
}

fn hover<'py>(a: &Args<'py>) -> Reply<'py> {
    let view = a.get_item(0)?;
    let x = a.get_item(1)?;
    tool_frame(&view, "hover", Some(&x), || hover_frame(&view, &x, &a.get_item(2)?, &modifiers_of(a.get_item(3)?)?))
}

fn hover_frame<'py>(view: &Obj<'py>, x: &Obj<'py>, y: &Obj<'py>, modifiers: &Obj<'py>) -> Reply<'py> {
    let py = view.py();
    let start = Instant::now();
    let tools = view.getattr("tools")?;
    let mode = tools.call_method0("sync")?;
    // Select's priority walk already queries the row for its group pick.
    view.setattr("_cue_cursor", (x, y))?;
    view.setattr("hovered_row", py.None())?;
    let source = view.getattr("source_viewport")?;
    let selecting = mode.eq("select")? && !attr(&source, "nav_mode")?.is_truthy()?;
    if !selecting || source.is_none() {
        view.setattr("hovered_row", view.call_method1("pick_row", (x, y))?)?;
    }
    view.setattr("hover_rows", PyTuple::empty(py))?; // GL paints hovered edges and annotations only.
    view.setattr("last_snap", py.None())?;
    view.setattr("overlay_primitives", PyList::empty(py))?;
    if selecting {
        if !source.is_none() {
            tools.call_method0("select_tool")?.call_method1("hover_at", (&tools, x, y, modifiers))?;
        } else {
            tools.call_method1("set_hover", (py.None(),))?;
        }
        view.setattr("hover_ms", ms(start))?;
        view.call_method0("update")?;
        return Ok(none(py));
    }
    let mut work_point = none(py);
    let mesh_mode = mode.eq("mesh")?;
    if mesh_mode {
        work_point = tools.call_method("work_point", (x, y), Some(&fields!(py; "picked_row" => view.getattr("hovered_row")?)))?;
    }
    let tool = attr(&source, "active_tool")?;
    if !tool.is_none() && !tool.getattr("uses_snap")?.is_truthy()? && tools.getattr("mode")?.eq("mesh")? {
        if !work_point.is_none() {
            view.setattr("last_snap", namespace(py, fields!(py; "point" => &work_point, "kind" => py.None(), "context" => py.None()))?)?;
        }
        view.call_method0("update")?;
        return view.getattr("last_snap");
    }
    let inference = attr(&source, "inference")?;
    // The snap rules keep it no longer than the hover; it holds the view
    // weakly all the same (a Rust closure is opaque to the collector).
    let (weak, px, py_) = (py.import("weakref")?.call_method1("ref", (view,))?.unbind(), x.clone().unbind(), y.clone().unbind());
    let project = PyCFunction::new_closure(py, None, None, move |args: &Bound<'_, PyTuple>, _: Option<&Bound<'_, PyDict>>| -> PyResult<Py<PyAny>> {
        let py = args.py();
        let view = weak.bind(py).call0()?;
        if view.is_none() {
            return Ok(py.None());
        }
        Ok(view.call_method1("_project_to_lock_line", (args.get_item(0)?, args.get_item(1)?, px.bind(py), py_.bind(py)))?.unbind())
    })?;
    let options: Bound<'py, PyDict> = if inference.hasattr("snap_options")? {
        // The same tool/state keywords as GL's InferenceEngine.snap: the
        // short list this used to build left out the acquired edge and
        // face, the Shift lock, the plane rules and the rectangle's
        // diagonal (see InferenceEngine.snap_options).
        let options = inference.call_method1("snap_options", (&tool, modifiers))?.cast_into::<PyDict>()?;
        options.set_item("start_point", tools.getattr("start_point")?)?;
        options.set_item("project_onto_line", &project)?;
        options
    } else {
        fields!(py;
            "start_point" => tools.getattr("start_point")?,
            "chain_first_point" => attr(&tool, "chain_first_point")?,
            "acquired_point" => attr(&inference, "_acquired_point")?,
            "acquired_points" => attr(&inference, "_encouraged")?,
            "axis_lock" => attr(&inference, "axis_lock")?,
            "shift_held" => modifiers.bitand(input(py, "ShiftModifier")?)?.is_truthy()?,
            "reference_edge" => attr(&inference, "reference_edge")?,
            "reference_mode" => attr(&inference, "reference_mode")?,
            "magnetic_axis_deg" => attr(&tool, "magnetic_axis_deg")?,
            "screen_axis_px" => attr(&tool, "screen_axis_px")?,
            "linear_mode" => source.getattr_opt("linear_inference_mode")?.unwrap_or("all".into_bound_py_any(py)?),
            "project_onto_line" => &project,
        )
    };
    if !view.getattr("inference_index")?.is_none() {
        if inference.hasattr("note_drawing_refs")? {
            options.set_item("inference", &inference)?;
            options.set_item("modifiers", modifiers)?;
        }
        if mesh_mode {
            options.set_item("work_point", &work_point)?;
        }
        let snap = py.import("built_core")?.getattr("resident_snap")?.call((view.getattr("inference_index")?, view.getattr("hovered_row")?, view, x, y), Some(&options))?;
        view.setattr("last_snap", &snap)?;
        if inference.hasattr("finish_snap")? && !snap.is_none() {
            // The axis-source cue picks through this view's tools (the snap
            // index answers its occlusion); the extension providers get the
            // document's viewport, as on GL.
            let snap = inference.call_method("finish_snap", (&snap, x, y, &tool), Some(&fields!(py; "picks" => &tools)))?;
            view.setattr("last_snap", &snap)?;
            if snap.getattr("kind")?.eq("endpoint")? {
                // GL's mid-segment corner pick: the corner under the cursor
                // is a soft, instant reference (through point, from point)
                // while a segment is drawn.
                inference.call_method("note_drawing_refs", (&tool,), Some(&fields!(py; "corner" => snap.getattr("point")?)))?;
            }
        }
    } else if ["tape", "move", "mesh"].contains(&tools.call_method0("sync")?.extract::<String>().unwrap_or_default().as_str()) {
        let [origin, direction] = unpack::<2>(&view.call_method1("_pixel_to_ray", (x, y))?)?;
        let camera = view.getattr("camera")?;
        let plane = match tools.getattr("start_point")? {
            start if !start.is_none() => start,
            _ => camera.getattr("target")?,
        };
        let normal = camera.call_method0("forward")?;
        let dot = py.import("built_core")?.getattr("Vec3")?.getattr("dotProduct")?;
        let denominator: f64 = dot.call1((&direction, &normal))?.extract()?;
        if denominator.abs() > 1e-9 {
            let along: f64 = dot.call1((plane.sub(&origin)?, &normal))?.extract()?;
            let mut point = origin.add(direction.mul(along / denominator)?)?;
            if tools.getattr("mode")?.eq("mesh")? && work_point.is_truthy()? {
                point = work_point.clone();
            }
            let snap = py.import("built_core.snap")?.getattr("compute_snap")?.call(
                (point, (x, y), namespace(py, fields!(py; "edges" => PyList::empty(py)))?, view.getattr("_world_to_pixel")?, 12.),
                Some(&options),
            )?;
            view.setattr("last_snap", snap)?;
        }
    }
    let snap = view.getattr("last_snap")?;
    if !inference.is_none() && !snap.is_none() {
        source.call_method1("_note_snap", (&snap,))?;
        let encouraging = source.getattr("_ENCOURAGING_KINDS")?.contains(snap.getattr("kind")?)?;
        inference.call_method1("_dwell_on", (if encouraging { snap.getattr("point")? } else { none(py) },))?;
    }
    view.call_method0("sync_selection")?;
    let spent = ms(start);
    view.setattr("hover_ms", spent)?;
    let interaction = py.import("views.document_interaction")?;
    if interaction.getattr("_PERF")?.is_truthy()? {
        interaction.call_method("_plog", ("hover", spent), Some(&fields!(py; "extra" => "renderer=wgpu id-local", "floor" => 0.)))?;
    }
    view.call_method0("update")?;
    view.getattr("last_snap")
}

fn sync_edit_context<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let (renderer, packed) = (view.getattr("renderer")?, view.getattr("packed")?);
    if renderer.is_none() || packed.is_none() {
        return Ok(none(py));
    }
    let mode = view.getattr("source_viewport")?.getattr_opt("edit_rest_mode")?.unwrap_or("fade".into_bound_py_any(py)?);
    let edit_group = view.getattr("scene")?.getattr("edit_group")?;
    let signature = (id(&packed), id(&edit_group), &mode).into_bound_py_any(py)?;
    if signature.eq(attr(&view, "_rest_signature")?)? {
        return Ok(none(py));
    }
    let rows = py.import("views.wgpu_edit_context")?.call_method1("rest_rows", (packed.getattr("groups")?, &edit_group))?;
    let before = Instant::now();
    // Always: a reset signature (an incremental update clears it) does not
    // mean the renderer holds no rest bits. Skipping the call when there
    // were no rows left the whole model hidden after leaving a group with
    // "hide" (and faded with "fade"). The native call is one pass over the
    // flags and writes nothing when they are unchanged.
    renderer.call_method1("set_rest", (&rows, &mode))?;
    view.setattr("rest_write_ms", ms(before))?;
    let np = py.import("numpy")?;
    let flags = packed.getattr("instances")?.get_item("info")?.get_item((pyo3::types::PySlice::full(py), 2))?;
    flags.call_method1("__iand__", (np.call_method1("uint32", (!192u32,))?,))?;
    let hide = mode.eq("hide")?;
    if hide || mode.eq("fade")? {
        flags.set_item(&rows, flags.get_item(&rows)?.bitor(np.call_method1("uint32", (64 | if hide { 128 } else { 0 },))?)?)?;
    }
    view.setattr("_pick_frame_dirty", true)?;
    view.setattr("_rest_signature", signature)?;
    py.import("views.wgpu_parity")?.call_method1("hidden_geometry", (&view,))?;
    Ok(none(py))
}

fn document_interaction<'py>(py: Python<'py>) -> Reply<'py> {
    py.import("views.document_interaction")?.getattr("DocumentInteraction")
}
fn shared_dict<'py>(view: &Obj<'py>, name: &str) -> Reply<'py> {
    view.getattr("_shared")?.getattr("__dict__")?.call_method1("setdefault", (name, PyDict::new(view.py())))
}

fn group_obb<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, group) = (a.get_item(0)?, a.get_item(1)?);
    let boxes = shared_dict(&view, "_container_boxes")?;
    if !boxes.contains(&group)? {
        boxes.set_item(&group, document_interaction(py)?.call_method1("_compute_obb", (&group,))?)?;
        // Which rows it measured: an edit elsewhere keeps the box.
        if !attr(&view, "uid_rows")?.is_none() {
            let np = py.import("numpy")?;
            let rows = np.call_method1("asarray", (view.call_method1("_rows_for_group", (&group,))?, np.getattr("int64")?))?;
            shared_dict(&view, "_container_box_rows")?.set_item(&group, rows)?;
        }
    }
    boxes.get_item(&group)
}

/// Keep the container boxes an incremental edit did not touch.
///
/// Clearing them all on every edit threw away the big containers' boxes
/// measured after the open (0.9 s each on the ride, measured again at the
/// next double-click). A box stays when none of its rows moved or changed
/// prototype; rows added (or a diff that walked every placement) may land
/// inside any container, so then none stays.
fn keep_container_boxes<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, tracker, ids, boxes, box_rows) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?, a.get_item(3)?, a.get_item(4)?);
    if !boxes.is_truthy()? || !tracker.getattr("last_local")?.is_truthy()? || tracker.getattr("added")?.is_truthy()? {
        return Ok(none(py));
    }
    let np = py.import("numpy")?;
    let int64 = np.getattr("int64")?;
    let touched = PyList::empty(py);
    touched.append(np.call_method1("asarray", (&ids, &int64))?)?;
    touched.append(np.call_method1("asarray", (tracker.getattr("removed")?, &int64))?)?;
    if tracker.getattr("changed")?.is_truthy()? {
        touched.call_method1("extend", (tracker.getattr("changed_code_rows")?.call_method0("values")?,))?;
    }
    let touched = np.call_method1("concatenate", (touched,))?;
    for entry in boxes.call_method0("items")?.try_iter()? {
        let [group, bounds] = unpack::<2>(&entry?)?;
        let rows = box_rows.call_method1("get", (&group,))?;
        if !rows.is_none() && !np.call_method1("isin", (&rows, &touched))?.call_method0("any")?.is_truthy()? {
            view.getattr("_container_boxes")?.set_item(&group, bounds)?;
            view.getattr("_container_box_rows")?.set_item(&group, rows)?;
        }
    }
    Ok(none(py))
}

fn edit_group_box_corners<'py>(a: &Args<'py>) -> Reply<'py> {
    document_interaction(a.py())?.call_method1("_edit_group_box_corners", (a.get_item(0)?,))
}
fn edit_group_box_contains_pixel<'py>(a: &Args<'py>) -> Reply<'py> {
    document_interaction(a.py())?.call_method1("edit_group_box_contains_pixel", (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?))
}
fn selection_box_primitives<'py>(a: &Args<'py>) -> Reply<'py> {
    a.py().import("views.wgpu_edit_context")?.call_method1("selection_box_primitives", (a.get_item(0)?,))
}
fn selection_box_vertices<'py>(a: &Args<'py>) -> Reply<'py> {
    a.py().import("views.wgpu_edit_context")?.call_method1("selection_box_vertices", (a.get_item(0)?,))
}
fn edit_box_primitives<'py>(a: &Args<'py>) -> Reply<'py> {
    a.py().import("views.wgpu_edit_context")?.call_method1("box_primitives", (a.get_item(0)?,))
}

fn leave_edit_on_empty_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, x, y, modifiers) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?, a.get_item(3)?);
    let scene = view.getattr("scene")?;
    let chord = input(py, "ControlModifier")?.bitor(input(py, "MetaModifier")?)?.bitor(input(py, "ShiftModifier")?)?;
    let leaves = !scene.getattr("edit_group")?.is_none()
        && !scene.getattr("selection")?.is_truthy()?
        && !modifiers.bitand(chord)?.is_truthy()?
        && !view.call_method1("edit_group_box_contains_pixel", (x, y))?.is_truthy()?;
    if leaves {
        view.getattr("source_viewport")?.call_method0("end_one_group_edit")?;
        view.call_method0("update")?;
    }
    leaves.into_bound_py_any(py)
}

/// What changes exactly when the selection does: its native revision
/// (`ElementSet.revision`, a process-wide counter taken on every change of
/// content and for every new set), or the identities of a stand-in set.
pub(super) fn selection_key<'py>(selected: &Obj<'py>) -> Reply<'py> {
    let py = selected.py();
    if let Some(revision) = selected.getattr_opt("revision")? {
        return (id(selected), revision).into_bound_py_any(py);
    }
    let identity = py.import("built_core.mesh")?.getattr("element_identity")?;
    py.get_type::<pyo3::types::PyFrozenSet>().call1((py.import("builtins")?.call_method1("map", (&identity, selected))?,))
}

/// `(key, groups, faces, edges, face keys, edge keys)`: what a frame reads
/// of the selection, sorted out once per change of it (`selection_key`).
/// Every frame used to walk the whole selection (a type test per entry,
/// several times a frame): with 600k entries that alone was frames of
/// hundreds of milliseconds while nothing changed. The keys are None for
/// a stand-in set (the marks then read them off the handles).
pub(super) fn selection_digest<'py>(view: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    let py = view.py();
    let selected = view.getattr("scene")?.getattr("selection")?;
    let key = selection_key(&selected)?;
    if let Some(digest) = view.getattr("__dict__")?.call_method1("get", ("_selection_digest",))?.cast_into::<PyTuple>().ok() {
        if digest.get_item(0)?.eq(&key)? {
            return Ok(digest);
        }
    }
    let group = py.import("built_core.group")?.getattr("Group")?;
    let (groups, faces, edges) = (PyList::empty(py), PyList::empty(py), PyList::empty(py));
    let (mut face_keys, mut edge_keys) = (none(py), none(py));
    if selected.hasattr("values_of")? {
        for placement in selected.call_method1("values_of", ("placement",))?.try_iter()? {
            let placement = placement?;
            if isinstance(&placement, &group)? {
                groups.append(placement)?;
            }
        }
        faces.call_method1("extend", (selected.call_method1("values_of", ("face",))?,))?;
        edges.call_method1("extend", (selected.call_method1("values_of", ("edge",))?,))?;
        face_keys = selected.call_method1("element_keys", ("face",))?;
        edge_keys = selected.call_method1("element_keys", ("edge",))?;
    } else {
        let mesh = py.import("built_core.mesh")?;
        let (face, edge) = (mesh.getattr("Face")?, mesh.getattr("Edge")?);
        for entity in selected.try_iter()? {
            let entity = entity?;
            if isinstance(&entity, &group)? {
                groups.append(entity)?;
            } else if isinstance(&entity, &face)? {
                faces.append(entity)?;
            } else if isinstance(&entity, &edge)? {
                edges.append(entity)?;
            }
        }
    }
    let digest = PyTuple::new(py, [key, groups.into_any(), faces.into_any(), edges.into_any(), face_keys, edge_keys])?;
    view.setattr("_selection_digest", &digest)?;
    Ok(digest)
}

/// Selected loose edges the overlay painter strokes itself under a mesh
/// tool; more go to the native line slot below.
const PAINTED_EDGES: usize = 2000;
const SELECTED_EDGES_SLOT: &str = "selected_edges";

/// The renderer's view of the selection and of what is under the cursor.
///
/// Two parts, each uploaded only when it changes: the selection's (its
/// placements' highlight rows, its faces' dots, its loose edges and its
/// groups' boxes), keyed on the selection's revision and the document's
/// version; and the hover's (the hovered rows, a hovered face's dots, a
/// hovered edge), which a renderer with `set_hover_marks` keeps apart, so
/// hovering over a huge selection re-sends a few vertices. Nothing here
/// depends on the camera: the side of a face the camera sees picks its dot
/// colour on the GPU (`preview.wgsl`), so orbiting re-sends nothing (the
/// eye was part of the key, and every camera move rebuilt the dots of
/// every selected face).
fn sync_selection<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let renderer = view.getattr("renderer")?;
    if renderer.is_none() {
        return Ok(none(py));
    }
    let mesh = py.import("built_core.mesh")?;
    let (face, edge, identity) = (mesh.getattr("Face")?, mesh.getattr("Edge")?, mesh.getattr("element_identity")?);
    let parity = py.import("views.wgpu_parity")?;
    let np = py.import("numpy")?;
    let (tools, packed, scene) = (view.getattr("tools")?, view.getattr("packed")?, view.getattr("scene")?);
    let selected = scene.getattr("selection")?;
    let digest = selection_digest(&view)?;
    let hover = match attr(&view, "hover_rows")? {
        rows if rows.is_truthy()? => positional(&rows)?,
        _ => PyTuple::empty(py),
    };
    let review = crate::tasks_host::preview_rows(&view)?.unwrap_or_else(|| PyTuple::empty(py).into_any());
    let hovered = hovered_entity(&view)?;
    let mode = tools.getattr("mode")?;
    // A tool's face under the cursor wears the selection dots (GL's
    // _paint_hover); Select never hovers a face, and a selected face
    // already shows them.
    let hover_face = if isinstance(&hovered, &face)? && mode.ne("select")? && !selected.contains(&hovered)? { hovered.clone() } else { none(py) };
    let placement = if hover_face.is_truthy()? { attr(&tools, "_mesh_hover_placement")? } else { none(py) };
    let hover_edge = if isinstance(&hovered, &edge)? && !selected.contains(&hovered)? { hovered.clone() } else { none(py) };
    // Outside the mesh tools (whose feedback owns the preview), the native
    // preview edges carry the selected loose edges and the selected groups'
    // boxes, all depth-tested as in GL.
    let edges_here = mode.ne("mesh")? && renderer.hasattr("set_preview_mesh")?;
    let [box_key, boxes] = if edges_here {
        unpack::<2>(&view.call_method0("_selection_box_vertices")?)?
    } else {
        [none(py), PyTuple::empty(py).into_any()]
    };
    let split = renderer.hasattr("set_hover_marks")?;
    let mesh_revision = attr(&scene.getattr("mesh")?, "revision")?;
    let geometry = (scene.getattr("version")?, id(&scene.getattr("mesh")?), mesh_revision).into_bound_py_any(py)?;
    let selection_part = (digest.get_item(0)?, id(&packed), &mode, &box_key, geometry, edges_here).into_bound_py_any(py)?;
    let hover_part = (&hover, &review, identity.call1((&hovered,))?, id(&placement), edges_here).into_bound_py_any(py)?;
    let signature = (&selection_part, &hover_part).into_bound_py_any(py)?;
    let old = view.getattr("_selection_signature")?;
    if signature.eq(&old)? {
        return Ok(none(py));
    }
    let selection_changed = old.is_none() || !old.get_item(0)?.eq(&selection_part)? || !split;
    // The selected placements' rows, once per change of the selection.
    if selection_changed || attr(&view, "_selection_rows_list")?.is_none() {
        let rows = PySet::empty(py)?;
        for root in digest.get_item(1)?.try_iter()? {
            rows.call_method1("update", (view.call_method1("_rows_for_group", (root?,))?,))?;
        }
        // GL never tints a face-me figure (its mesh quad is not what is on
        // screen): its cue is the outline of the quad as drawn.
        let groups = packed.getattr("groups")?;
        let indices = PyList::empty(py);
        for row in rows.iter() {
            if !truthy(&groups.get_item(&row)?, "billboard")? {
                indices.append(row)?;
            }
        }
        view.setattr("_selection_rows_list", indices)?;
    }
    renderer.call_method1("set_highlight", (view.getattr("_selection_rows_list")?, PyList::new(py, hover.iter())?, review))?;
    let (faces, edges) = (digest.get_item(2)?, digest.get_item(3)?);
    let hover_dots = if hover_face.is_none() { None } else { Some(parity.call_method1("hover_face_marks", (&view, &hover_face, &placement))?) };
    let vertex = py.import("built_core")?.getattr("VERTEX")?;
    let hover_line = |edge: &Obj<'py>| -> Reply<'py> {
        let ends = PyList::empty(py);
        for end in ["a", "b"] {
            ends.append((edge.getattr(end)?.call_method0("toTuple")?, (0, 0, 1), (76, 140, 242, 255)))?;
        }
        np.call_method1("array", (ends, &vertex))
    };
    if split {
        if selection_changed && renderer.hasattr("set_face_marks")? {
            renderer.call_method1("set_face_marks", (parity.call_method1("face_marks", (&view, &faces, digest.get_item(4)?))?,))?;
        }
        let no_bytes = np.call_method1("empty", (0, np.getattr("uint8")?))?;
        let dots = hover_dots.unwrap_or_else(|| no_bytes.clone());
        let line = if edges_here && !hover_edge.is_none() { parity.call_method1("raw", (hover_line(&hover_edge)?,))? } else { no_bytes };
        renderer.call_method1("set_hover_marks", (dots, line))?;
    } else if renderer.hasattr("set_face_marks")? {
        let mut marks = parity.call_method1("face_marks", (&view, &faces, digest.get_item(4)?))?;
        if let Some(dots) = hover_dots {
            marks = np.call_method1("concatenate", ((marks, dots),))?;
        }
        renderer.call_method1("set_face_marks", (marks,))?;
    }
    // Under a mesh tool the overlay painter strokes the selected loose
    // edges every frame; past a few thousand they go to a native line slot
    // instead, uploaded once per change of the selection.
    if selection_changed && renderer.hasattr("set_lines")? {
        let bulk = !edges_here && edges.len()? > PAINTED_EDGES;
        let was = truthy(&view, "_selected_edge_lines")?;
        if bulk {
            let ends = parity.call_method1("edge_marks", (&view, &edges, digest.get_item(5)?))?.get_item("pos")?.call_method1("reshape", (-1, 2, 3))?;
            let scale: f64 = view.call_method0("devicePixelRatioF")?.extract()?;
            let segments = py.import("views.wgpu_lines")?.call_method1("segments", (ends, (242, 114, 40, 255), 1.5 * scale))?;
            renderer.call_method("set_lines", (SELECTED_EDGES_SLOT, parity.call_method1("raw", (segments,))?), Some(&fields!(py; "above_edges" => true)))?;
        } else if was {
            renderer.call_method1("set_lines", (SELECTED_EDGES_SLOT, np.call_method1("empty", (0, np.getattr("uint8")?))?))?;
        }
        view.setattr("_selected_edge_lines", bulk)?;
    }
    if edges_here && selection_changed {
        let mut parts = vec![parity.call_method1("edge_marks", (&view, &edges, digest.get_item(5)?))?];
        if !split && !hover_edge.is_none() {
            parts.push(hover_line(&hover_edge)?);
        }
        parts.push(boxes);
        let lines = np.call_method1("concatenate", (PyList::new(py, parts)?,))?;
        renderer.call_method1("set_preview_mesh", (np.call_method1("empty", (0, np.getattr("uint8")?))?, parity.call_method1("raw", (lines,))?))?;
    }
    view.setattr("_selection_signature", signature)?;
    Ok(none(py))
}

/// Loose edge feedback; faces/groups are depth-tested native passes.
fn draw_selection_marks<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, painter) = (a.get_item(0)?, a.get_item(1)?);
    if view.getattr("tools")?.getattr("mode")?.ne("mesh")? && view.getattr("renderer")?.hasattr("set_preview_mesh")? {
        return Ok(none(py));
    }
    let edge = py.import("built_core.mesh")?.getattr("Edge")?;
    let point = py.import("built_core")?.getattr("ScreenPoint")?;
    let render = py.import("built_render")?;
    let (pen, colour) = (render.getattr("PaintPen")?, render.getattr("PaintColor")?);
    let selection = view.getattr("scene")?.getattr("selection")?;
    let mut entities = Vec::new();
    if !truthy(&view, "_selected_edge_lines")? {
        for entity in selection_digest(&view)?.get_item(3)?.try_iter()? {
            entities.push((entity?, false));
        }
    }
    let hovered = hovered_entity(&view)?;
    if isinstance(&hovered, &edge)? && !selection.contains(&hovered)? {
        entities.push((hovered, true));
    }
    painter.call_method0("save")?;
    let drawn = (|| -> PyResult<()> {
        for (entity, hover) in &entities {
            let rgb = if *hover { (0.30, 0.55, 0.95) } else { (0.95, 0.45, 0.16) };
            painter.call_method1("setPen", (pen.call1((colour.call_method1("fromRgbF", rgb)?, 1.5))?,))?;
            let from = view.call_method1("_world_to_pixel", (entity.getattr("a")?,))?;
            let to = view.call_method1("_world_to_pixel", (entity.getattr("b")?,))?;
            if !from.is_none() && !to.is_none() {
                painter.call_method1("drawLine", (point.call1(positional(&from)?)?, point.call1(positional(&to)?)?))?;
            }
        }
        Ok(())
    })();
    painter.call_method0("restore")?;
    drawn?;
    Ok(none(py))
}

/// Wear the cursor the tools set on the (hidden) GL viewport: the tool
/// icons, Orbit's and Pan's hands, a tool's own changes while it works.
/// Copying it once per tool change missed every other change (Orbit chosen
/// over Select kept the arrow, Maxence 2026-11-09).
fn mirror_cursor<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let source = view.getattr("source_viewport")?;
    if source.is_none() {
        return Ok(none(py));
    }
    let key = if !source.call_method1("testAttribute", (input(py, "WA_SetCursor")?,))?.is_truthy()? {
        none(py)
    } else {
        let cursor = source.call_method0("cursor")?;
        let shape = if cursor.hasattr("shape")? { cursor.call_method0("shape")? } else { cursor.clone() };
        if shape.eq(input(py, "BitmapCursor")?)? && cursor.hasattr("pixmap")? {
            (&shape, cursor.call_method0("pixmap")?.call_method0("cacheKey")?).into_bound_py_any(py)?
        } else {
            shape
        }
    };
    if let Some(worn) = view.getattr_opt("_cursor_key")? {
        if key.eq(worn)? {
            return Ok(none(py));
        }
    }
    view.setattr("_cursor_key", &key)?;
    if key.is_none() {
        view.call_method0("unsetCursor")?;
    } else {
        view.call_method1("setCursor", (source.call_method0("cursor")?,))?;
    }
    Ok(none(py))
}

/// `{id(child): parent}` over the scene's groups, per pack.
fn parents<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let token = attr(&view, "_content_token")?;
    let cached = attr(&view, "_parent_map")?;
    if !cached.is_none() && cached.get_item(0)?.eq(&token)? {
        return cached.get_item(1);
    }
    let parents = PyDict::new(py);
    let mut todo: Vec<Obj<'py>> = view.getattr("scene")?.getattr("groups")?.try_iter()?.collect::<PyResult<_>>()?;
    while let Some(group) = todo.pop() {
        let children = attr(&group, "children")?;
        if children.is_truthy()? {
            for child in children.try_iter()? {
                let child = child?;
                parents.set_item(id(&child), &group)?;
                todo.push(child);
            }
        }
    }
    view.setattr("_parent_map", (token, &parents))?;
    Ok(parents.into_any())
}

/// The placement a click selects for a hit on `group`: the top-level
/// object at the root, the child directly under the group being edited
/// inside one, `None` for a hit outside the context — what the GL
/// viewport's `pick_group` returns.
fn context_placement<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, group) = (a.get_item(0)?, a.get_item(1)?);
    if group.is_none() {
        return Ok(group);
    }
    let parents = view.call_method0("_parents")?;
    let mut chain = vec![group];
    loop {
        let parent = parents.call_method1("get", (id(&chain[chain.len() - 1]),))?;
        if parent.is_none() {
            break;
        }
        chain.push(parent);
    }
    let context = view.getattr("scene")?.getattr("edit_group")?;
    if context.is_none() {
        return Ok(chain.pop().unwrap_or_else(|| none(py)));
    }
    Ok(match chain.iter().position(|group| group.is(&context)) {
        Some(at) if at > 0 => chain[at - 1].clone(),
        _ => none(py),
    })
}

fn rows_for_group<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, group) = (a.get_item(0)?, a.get_item(1)?);
    let cache = shared_dict(&view, "_selection_rows")?;
    if !cache.contains(&group)? {
        let rows = PyList::empty(py);
        let (uid_rows, packed) = (view.getattr("uid_rows")?, view.getattr("packed")?);
        let mut pending = vec![group.clone()];
        while let Some(child) = pending.pop() {
            rows.call_method1("extend", (uid_rows.call_method1("get", (packed.call_method1("row_key", (&child,))?, PyTuple::empty(py)))?,))?;
            let children = child.getattr("children")?;
            if children.is_truthy()? {
                for child in children.try_iter()? {
                    pending.push(child?);
                }
            }
        }
        cache.set_item(&group, rows)?;
    }
    cache.get_item(&group)
}

fn select_rows<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let modifiers = modifiers_of(a.get_item(2)?)?;
    let start = Instant::now();
    let scene = view.getattr("scene")?;
    let hit = PySet::empty(py)?;
    let groups = view.getattr("packed")?.getattr("groups")?;
    for row in a.get_item(1)?.try_iter()? {
        let row = row?;
        if !view.call_method1("resolve", (&row,))?.is_none() {
            hit.add(view.call_method1("context_placement", (groups.get_item(&row)?,))?)?;
        }
    }
    let chosen = PyList::empty(py);
    for group in hit.iter() {
        if !group.is_none() && scene.call_method1("entity_selectable", (&group,))?.is_truthy()? {
            chosen.append(group)?;
        }
    }
    let mode = py.import("tools.rust_tools")?.call_method1("selection_mode", (&modifiers,))?;
    scene.call_method("select", (chosen,), Some(&fields!(py; "mode" => mode)))?;
    view.call_method0("sync_selection")?;
    let source = view.getattr("source_viewport")?;
    if !source.is_none() {
        source.call_method0("notify_scene_changed")?;
    }
    view.call_method0("update")?;
    view.setattr("selection_ms", ms(start))?;
    Ok(none(py))
}

#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("hover", Mode::Method, hover, &["self", "x", "y", "modifiers=None"], "", ""),
    ("sync_edit_context", Mode::Method, sync_edit_context, &["self"], "", ""),
    ("_group_obb", Mode::Method, group_obb, &["self", "group"], "", ""),
    ("_keep_container_boxes", Mode::Method, keep_container_boxes, &["self", "tracker", "ids", "boxes", "box_rows"], "", ""),
    ("_edit_group_box_corners", Mode::Method, edit_group_box_corners, &["self"], "", ""),
    ("edit_group_box_contains_pixel", Mode::Method, edit_group_box_contains_pixel, &["self", "x", "y"], "", ""),
    ("_leave_edit_on_empty_click", Mode::Method, leave_edit_on_empty_click, &["self", "x", "y", "modifiers"], "", ""),
    ("_selection_box_primitives", Mode::Method, selection_box_primitives, &["self"], "", ""),
    ("_selection_box_vertices", Mode::Method, selection_box_vertices, &["self"], "", ""),
    ("_edit_box_primitives", Mode::Method, edit_box_primitives, &["self"], "", ""),
    ("sync_selection", Mode::Method, sync_selection, &["self"], "", ""),
    ("_draw_selection_marks", Mode::Method, draw_selection_marks, &["self", "painter"], "", ""),
    ("_mirror_cursor", Mode::Method, mirror_cursor, &["self"], "", ""),
    ("_parents", Mode::Method, parents, &["self"], "", ""),
    ("context_placement", Mode::Method, context_placement, &["self", "group"], "", ""),
    ("_rows_for_group", Mode::Method, rows_for_group, &["self", "group"], "", ""),
    ("select_rows", Mode::Method, select_rows, &["self", "rows", "modifiers=None"], "", ""),
];
