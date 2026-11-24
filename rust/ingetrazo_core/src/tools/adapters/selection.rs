// SPDX-License-Identifier: GPL-3.0-or-later
//! Selection orchestration around the native box and mesh-incidence queries.
use super::*;
use pyo3::exceptions::{PyAttributeError, PyKeyError, PyRuntimeError, PyTypeError, PyValueError};

fn visible_elements<'py>(
    indices: Obj<'py>,
    elements: Obj<'py>,
    selectable: &Obj<'py>,
) -> PyResult<Bound<'py, PyList>> {
    let result = PyList::empty(indices.py());
    for index in indices.try_iter()? {
        let element = elements.get_item(index?)?;
        if selectable.call1((&element,))?.is_truthy()? {
            result.append(element)?;
        }
    }
    Ok(result)
}

fn box_loose_arrays<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let viewport = a.get_item(0)?;
    let scene = viewport.getattr("scene")?;
    let Some(camera) = viewport.getattr_opt("_camera_px")? else {
        return Ok(none(py));
    };
    let Some(mesh) = scene.getattr_opt("mesh")? else {
        return Ok(none(py));
    };
    if camera.is_none()
        || !mesh
            .get_type()
            .is(&py.import("core.store_mesh")?.getattr("StoreMesh")?)
    {
        return Ok(none(py));
    }
    let [mvp, w, h] = match camera.call0().and_then(|pose| unpack::<3>(&pose)) {
        Ok(view) => view,
        Err(e)
            if e.is_instance_of::<PyAttributeError>(py) || e.is_instance_of::<PyTypeError>(py) =>
        {
            return Ok(none(py))
        }
        Err(e) => return Err(e),
    };
    let store = mesh.getattr("__dict__")?.get_item("_store")?;
    let masks = store.call_method1("box_pick", (mvp, w, h, a.get_item(1)?, a.get_item(2)?))?;
    let [edges, faces] = unpack::<2>(&masks)?;
    let selectable = scene.getattr("entity_selectable")?;
    (
        visible_elements(edges, mesh.getattr("edges")?, &selectable)?,
        visible_elements(faces, mesh.getattr("faces")?, &selectable)?,
    )
        .into_bound_py_any(py)
}

fn box_loose_fast<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let vp = a.get_item(0)?;
    let rect = a.get_item(1)?;
    let crossing = a.get_item(2)?;
    let Some(project) = vp.getattr_opt("_ledge_screen")? else {
        return Ok(none(py));
    };
    let Some(index) = vp.getattr_opt("_pick_index")? else {
        return Ok(none(py));
    };
    let (Ok(projected), Ok(index)) = (project.call0(), index.call0()) else {
        return Ok(none(py));
    };
    if projected.is_none() || index.getattr("edge_a")?.is_none() {
        return Ok(none(py));
    }
    let [ax, ay, bx, by, valid] = unpack::<5>(&projected)?;
    let edges = index.getattr("edges")?;
    if edges.len()? != ax.len()? {
        return Ok(none(py));
    }
    let scene = vp.getattr("scene")?;
    let selectable = scene.getattr("entity_selectable")?;
    let mask = py
        .import("tools.select_host")?
        .call_method1("_seg_rect_mask", (ax, ay, bx, by, valid, rect, &crossing))?;
    let mut hits = py
        .import("numpy")?
        .call_method1("where", (mask,))?
        .get_item(0)?;
    if let Some(allowed) = index.getattr_opt("edge_sel")?.filter(|v| !v.is_none()) {
        hits = hits.get_item(allowed.get_item(&hits)?)?;
    }
    let picked_edges = PyList::empty(py);
    let picked_faces = PyList::empty(py);
    let crossing = crossing.is_truthy()?;
    let hit_keys = PySet::empty(py)?;
    let seen = PySet::empty(py)?;
    for hit in hits.try_iter()? {
        let edge = edges.get_item(hit?)?;
        if selectable.call1((&edge,))?.is_truthy()? && !optional(&edge, "hidden")?.is_truthy()? {
            picked_edges.append(&edge)?;
        }
        if crossing {
            for face in edge.getattr("faces")?.try_iter()? {
                let face = face?;
                let key = face.getattr("key")?;
                if !seen.contains(&key)? {
                    seen.add(key)?;
                    if selectable.call1((&face,))?.is_truthy()? {
                        picked_faces.append(face)?;
                    }
                }
            }
        } else {
            hit_keys.add(edge.getattr("key")?)?;
        }
    }
    if !crossing {
        let outside = PySet::empty(py)?;
        for edge in edges.try_iter()? {
            let edge = edge?;
            let inside = hit_keys.contains(edge.getattr("key")?)?;
            for face in edge.getattr("faces")?.try_iter()? {
                let key = face?.getattr("key")?;
                seen.add(&key)?;
                if !inside {
                    outside.add(key)?;
                }
            }
        }
        for face in scene.getattr("faces")?.try_iter()? {
            let face = face?;
            let key = face.getattr("key")?;
            if seen.contains(&key)?
                && !outside.contains(key)?
                && selectable.call1((&face,))?.is_truthy()?
            {
                picked_faces.append(face)?;
            }
        }
    }
    (picked_edges, picked_faces).into_bound_py_any(py)
}

fn box_group_fast<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    match py
        .import("tools.select_host")?
        .getattr("box_chunk_group")?
        .call1(a)
    {
        Err(e)
            if e.is_instance_of::<PyAttributeError>(py)
                || e.is_instance_of::<PyTypeError>(py)
                || e.is_instance_of::<PyValueError>(py)
                || e.is_instance_of::<PyKeyError>(py)
                || e.is_instance_of::<PyRuntimeError>(py) =>
        {
            Ok(none(py))
        }
        verdict => verdict,
    }
}

fn box_select<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.select_host")?;
    let [vp, rect, crossing, mode] = unpack::<4>(a.as_any())?;
    let scene = vp.getattr("scene")?;
    let picked = PyList::empty(py);
    let mut fast = module.call_method1("_box_loose_fast", (&vp, &rect, &crossing))?;
    if fast.is_none() {
        fast = module.call_method1("_box_loose_arrays", (&vp, &rect, &crossing))?;
    }
    let records = |loose: bool| {
        module.call_method(
            "box_records",
            (&vp, &rect, &crossing),
            Some(&fields!(py; "loose" => loose)),
        )
    };
    if fast.is_none() {
        picked.call_method1("extend", (records(true)?,))?;
    } else {
        for batch in unpack::<2>(&fast)? {
            picked.call_method1("extend", (batch,))?;
        }
    }
    let context = scene.getattr("edit_group")?;
    let groups = if context.is_none() {
        optional(&scene, "groups")?
    } else {
        optional(&context, "children")?
    };
    if groups.is_truthy()? {
        let candidates = PyList::empty(py);
        for group in groups.try_iter()? {
            let group = group?;
            if scene
                .call_method1("entity_selectable", (&group,))?
                .is_truthy()?
            {
                candidates.append(group)?;
            }
        }
        let packed =
            module.call_method1("_box_groups_packed", (&vp, &candidates, &rect, &crossing))?;
        for group in candidates.iter() {
            let id = group.as_ptr() as usize;
            let hit = if !packed.is_none() && packed.get_item(1)?.contains(id)? {
                packed.get_item(0)?.contains(id)?
            } else {
                let verdict =
                    module.call_method1("_box_group_fast", (&vp, &group, &rect, &crossing))?;
                if verdict.is_none() {
                    let placements = py
                        .import("core.group")?
                        .call_method1("iter_placements", (&group,))?;
                    module
                        .call_method1("box_placements", (&vp, &rect, &crossing, placements))?
                        .is_truthy()?
                } else {
                    verdict.is_truthy()?
                }
            };
            if hit {
                picked.append(group)?;
            }
        }
    }
    picked.call_method1("extend", (records(false)?,))?;
    if mode.eq("replace")? && !optional(&vp, "extension_pick")?.is_none() {
        vp.call_method0("clear_extension_pick")?;
    }
    scene.call_method("select", (picked,), Some(&fields!(py; "mode"=>mode)))?;
    vp.call_method0("update")?;
    Ok(none(py))
}

fn bounded<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let vp = a.get_item(0)?;
    let entity = a.get_item(1)?;
    let module = py.import("tools.select_host")?;
    let mesh = vp.getattr("scene")?.getattr("mesh")?;
    let face = module.getattr("Face")?;
    let edge = module.getattr("Edge")?;
    let faces = entity.is_instance(&face)?;
    let picked = py
        .get_type::<PyList>()
        .call1((module.call_method1("expand_pick", (&vp, &entity))?,))?
        .cast_into::<PyList>()?;
    for element in picked.get_slice(0, picked.len()).iter() {
        if faces && element.is_instance(&face)? {
            let mut loops = vec![element.getattr("loop")?];
            loops.extend(
                element
                    .getattr("hole_loops")?
                    .try_iter()?
                    .collect::<PyResult<Vec<_>>>()?,
            );
            for ring in loops {
                let n = ring.len()?;
                for i in 0..n {
                    let edge = mesh.call_method1(
                        "find_edge",
                        (ring.get_item(i)?, ring.get_item((i + 1) % n)?),
                    )?;
                    if !edge.is_none() {
                        picked.append(edge)?;
                    }
                }
            }
        } else if !faces && element.is_instance(&edge)? {
            picked.call_method1("extend", (element.getattr("faces")?,))?;
        }
    }
    Ok(picked.into_any())
}

fn erase_entities<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let vp = a.get_item(0)?;
    let entities = a.get_item(1)?;
    let module = py.import("tools.select_host")?;
    let bucket = |name: &str| -> PyResult<Bound<'py, PyList>> {
        let cls = module.getattr(name)?;
        let group = PyList::empty(py);
        for entity in entities.try_iter()? {
            let entity = entity?;
            if entity.is_instance(&cls)? {
                group.append(entity)?;
            }
        }
        Ok(group)
    };
    let edges = bucket("Edge")?;
    let faces = bucket("Face")?;
    let groups = bucket("Group")?;
    let dims = bucket("Dimension")?;
    let labels = bucket("TextLabel")?;
    let paths = bucket("GeoPath")?;
    let guides = PyList::empty(py);
    for entity in entities.try_iter()? {
        let entity = entity?;
        if module.call_method1("is_guide", (&entity,))?.is_truthy()? {
            guides.append(entity)?;
        }
    }
    let planes = bucket("SectionPlane")?;
    let cmds = PyList::empty(py);
    if !edges.is_empty() || !faces.is_empty() {
        cmds.append(module.call_method1("EraseSelectionCommand", (edges, faces))?)?;
    }
    for group in groups.iter() {
        cmds.append(module.call_method1("DeleteGroupCommand", (group,))?)?;
    }
    for (objects, command) in [
        (guides, "DeleteGuidesCommand"),
        (planes, "DeleteSectionPlanesCommand"),
        (dims, "DeleteDimensionsCommand"),
        (labels, "DeleteTextLabelsCommand"),
        (paths, "DeleteGeoPathsCommand"),
    ] {
        if !objects.is_empty() {
            cmds.append(module.call_method1(command, (objects,))?)?;
        }
    }
    if cmds.is_empty() {
        return false.into_bound_py_any(py);
    }
    let cmd = if cmds.len() == 1 {
        cmds.get_item(0)?
    } else {
        module.call_method1("CompoundCommand", (cmds,))?
    };
    vp.getattr("history")?.call_method1("execute", (cmd,))?;
    vp.call_method0("update")?;
    true.into_bound_py_any(py)
}

fn delete_selection_or_hover<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.select_host")?;
    let viewport = a.get_item(0)?;
    if !optional(&viewport, "extension_pick")?.is_none() {
        return viewport.call_method0("delete_extension_pick");
    }
    let selection = py
        .get_type::<pyo3::types::PyList>()
        .call1((viewport.getattr("scene")?.getattr("selection")?,))?;
    if selection.is_truthy()? {
        return module.call_method1("erase_entities", (&viewport, &selection));
    }
    let hovered = optional(&viewport, "_hover_entity")?;
    if hovered.is_none() {
        return false.into_bound_py_any(py);
    }
    viewport.call_method1("set_hover", (py.None(),))?;
    module.call_method1(
        "erase_entities",
        (
            &viewport,
            module.call_method1("expand_pick", (&viewport, &hovered))?,
        ),
    )
}

fn expand_pick<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.select_host")?;
    let entity = a.get_item(1)?;
    let viewport = a.get_item(0)?;
    if entity.is_instance(&module.getattr("Face")?)? {
        return viewport
            .getattr("scene")?
            .getattr("mesh")?
            .call_method1("surface_of", (&entity,));
    }
    if entity.is_instance(&module.getattr("Edge")?)? && !optional(&entity, "curve")?.is_none() {
        return viewport
            .getattr("scene")?
            .getattr("mesh")?
            .call_method1("curve_edges", (&entity,));
    }
    Ok(PyList::new(py, [entity.clone()])?.into_any())
}

fn toggle_section<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.select_host")?;
    let entity = a.get_item(1)?;
    let viewport = a.get_item(0)?;
    viewport.getattr("history")?.call_method1(
        "execute",
        (module.call_method1(
            "SetActiveSectionCommand",
            ((if entity.getattr("active")?.is_truthy()? {
                none(py)
            } else {
                entity.clone()
            }),),
        )?,),
    )?;
    viewport
        .getattr("scene")?
        .call_method1("select", (PyList::new(py, [entity.clone()])?.into_any(),))?;
    Ok(none(py))
}

fn apply<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.select_host")?;
    let entities = a.get_item(1)?;
    let grow = a.get_item(3)?;
    let mode = a.get_item(2)?;
    let mut query: Obj<'py>;
    let mut result: Obj<'py>;
    let viewport = a.get_item(0)?;
    let picked = PyList::empty(py).into_any();
    for item in entities.try_iter()? {
        let entity = item?;
        if grow.ne("natural")?
            && py
                .import("builtins")?
                .getattr("isinstance")?
                .call1((
                    &entity,
                    (module.getattr("Face")?, module.getattr("Edge")?).into_bound_py_any(py)?,
                ))?
                .is_truthy()?
        {
            query = optional(&viewport.getattr("scene")?.getattr("mesh")?, "grow_pick")?;
            result = if query.is_truthy()? {
                query.call1((&entity, &grow))?
            } else {
                none(py)
            };
            if !result.is_none() {
                picked.call_method1("extend", (&result,))?;
                continue;
            }
        }
        if grow.eq("connected")? {
            let all_connected = py.import("core.select_ops")?.getattr("all_connected")?;
            picked.call_method1(
                "extend",
                (all_connected.call1((PyList::new(py, [entity.clone()])?.into_any(),))?,),
            )?;
        } else {
            if grow.eq("bounded")? {
                picked.call_method1(
                    "extend",
                    (module.call_method1("_bounded", (&viewport, &entity))?,),
                )?;
            } else {
                picked.call_method1(
                    "extend",
                    (module.call_method1("expand_pick", (&viewport, &entity))?,),
                )?;
            }
        }
    }
    viewport.getattr("scene")?.call_method(
        "select",
        (&picked,),
        Some(&fields!(py; "mode" => &mode)),
    )?;
    Ok(none(py))
}

fn cycle_status<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.select_host")?;
    let count = a.get_item(2)?;
    let entity = a.get_item(3)?;
    let index = a.get_item(1)?;
    let viewport = a.get_item(0)?;
    let flash = optional(&viewport, "flash_status")?;
    if flash.is_callable() {
        let tr = py.import("core.i18n")?.getattr("tr")?;
        flash.call1((tr.call1(("{index} of {count}: {what} — click again for the next",))?.call_method("format", (), Some(&fields!(py; "index" => index.add(1)?, "count" => &count, "what" => module.call_method1("describe_pick", (&entity,))?)))?,))?;
    }
    Ok(none(py))
}

fn seg_rect_mask<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let [ax, ay, bx, by, valid, rect, crossing] = unpack::<7>(a.as_any())?;
    let np = py.import("numpy")?;
    let dtype = np.getattr("float64")?;
    let array = |v| {
        np.call_method(
            "ascontiguousarray",
            (v,),
            Some(&fields!(py;"dtype"=>&dtype)),
        )?
        .call_method1("reshape", (-1,))
    };
    let ax = array(ax)?;
    let valid = np.call_method1("broadcast_to", (valid, (ax.len()?,)))?;
    let valid = np.call_method(
        "ascontiguousarray",
        (valid,),
        Some(&fields!(py;"dtype"=>py.get_type::<pyo3::types::PyBool>())),
    )?;
    let floats = py.get_type::<pyo3::types::PyFloat>();
    let rect = rect
        .try_iter()?
        .map(|v| floats.call1((v?,)))
        .collect::<PyResult<Vec<_>>>()?;
    py.import("ingetrazo_core")?.call_method1(
        "seg_rect_mask",
        (
            ax,
            array(ay)?,
            array(bx)?,
            array(by)?,
            valid,
            PyTuple::new(py, rect)?,
            crossing.is_truthy()?,
        ),
    )
}
fn describe_pick<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let entity = a.get_item(0)?;
    let module = py.import("tools.select_host")?;
    let tr = py.import("core.i18n")?.getattr("tr")?;
    if entity.is_instance(&module.getattr("Group")?)? {
        let group = tr.call1((if entity.call_method0("is_instance")?.is_truthy()? {
            "Component"
        } else {
            "Group"
        },))?;
        let name = optional(&entity, "name")?;
        if name.is_truthy()? {
            let name: String = name.call_method0("strip")?.extract()?;
            if !name.is_empty() {
                return format!("{} «{}»", group.extract::<String>()?, name).into_bound_py_any(py);
            }
        }
        return Ok(group);
    }
    for (ty, word) in [
        ("Edge", "Edge"),
        ("Face", "Face"),
        ("Dimension", "Dimension"),
        ("TextLabel", "Text"),
        ("SectionPlane", "Section Plane"),
    ] {
        if entity.is_instance(&module.getattr(ty)?)? {
            return tr.call1((word,));
        }
    }
    let ty = entity.get_type().getattr("__name__")?;
    if ty.eq("ImagePlane")? {
        tr.call1(("Image",))
    } else {
        Ok(ty)
    }
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("_seg_rect_mask", Mode::Function, seg_rect_mask, &["ax", "ay", "bx", "by", "ok", "rect", "crossing"], "", "The rectangle test of N screen segments: window mode = both\nendpoints inside; crossing mode = an endpoint inside or the segment\nthrough one of the four borders (``ingetrazo_core.seg_rect_mask``)."),
    ("describe_pick", Mode::Function, describe_pick, &["entity"], "str", "A few words for the status bar naming what a click picked."),
    ("_box_loose_fast", Mode::Function, box_loose_fast, &["viewport", "rect", "crossing"], "", "Vectorized box select over the loose mesh: edges via the cached\nscreen projection, faces derived from their boundary edges. Returns\n(edges, faces) or None to fall back to the per-entity walk (stub\nviewports in tests, or a viewport without the caches)."),
    ("_box_loose_arrays", Mode::Function, box_loose_arrays, &["viewport", "rect", "crossing"], "", "The loose mesh's box catch from the store's dense arrays\n(``ingetrazo_core.box_mesh_arrays``): element views made only for\nwhat the box takes. Returns (edges, faces), or None when the loose\nmesh is not store-backed or the view cannot project."),
    ("_box_group_fast", Mode::Function, box_group_fast, &["viewport", "group", "rect", "crossing"], "", ""),
    ("cycle_status", Mode::Function, cycle_status, &["viewport", "index:int", "count:int", "entity"], "None", ""),
    ("_bounded", Mode::Function, bounded, &["viewport", "entity"], "list", "A double click's catch: a face with its bounding edges, an edge\nwith its faces."),
    ("apply", Mode::Function, apply, &["viewport", "entities", "mode:str", "grow:str"], "None", "Select what the machine picked, grown as it says: ``natural`` (a\nsmooth surface, a whole curve), ``bounded`` (double click) or\n``connected`` (triple click: the whole solid)."),
    ("_toggle_section", Mode::Function, toggle_section, &["viewport", "entity"], "None", ""),
    ("box_select", Mode::Function, box_select, &["viewport", "rect", "crossing:bool", "mode:str"], "None", "What a released box catches (``rect`` normalised, ``crossing`` when\ndragged right to left), selected the way ``mode`` says."),
    ("expand_pick", Mode::Function, expand_pick, &["viewport", "entity"], "list", "Grow a pick to its natural whole: a curved surface (faces joined by\nsoft edges) for a face, or the whole drawn curve (circle/arc) for one of\nits segments — the classic behaviour. Plain entities select alone."),
    ("erase_entities", Mode::Function, erase_entities, &["viewport", "entities"], "bool", "Erase ``entities`` (any mix of pickable types) as ONE undoable step.\nTrue if anything was erased."),
    ("delete_selection_or_hover", Mode::Function, delete_selection_or_hover, &["viewport"], "bool", "The Delete key: erase the selection; with NOTHING selected, erase\nwhat is highlighted under the cursor — hover an edge or a face and press\nSupr, no click needed. The hover grows like a click would (a whole\ncircle, a whole smooth surface), so what goes is what a click-then-Supr\nwould have erased. True if anything was erased."),
];
