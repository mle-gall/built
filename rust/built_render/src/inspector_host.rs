// SPDX-License-Identifier: GPL-3.0-or-later
//! The native inspector's host side (`ui_panel_set` / `panel:` actions):
//! what the selection's records, the assembly's parts and the filing rule
//! say, the camera inspector and the bevel tool's panel, and the edits
//! they send back.
//!
//! What stays Python is what records/IFC are by design: the occurrence
//! record, the filing rule, the IFC vocabulary and their protocol commands
//! (`built_core.protocol.records`, `core.bim_registry`, `built_core.structure`). The
//! document, camera and tools are reached through their attributes.
mod terrain;
use pyo3::{
    prelude::*,
    types::{IntoPyDict, PyDict, PyList, PyTuple},
};

fn opt<'py>(o: &Bound<'py, PyAny>, name: &str) -> Option<Bound<'py, PyAny>> {
    o.getattr(name).ok().filter(|v| !v.is_none())
}

fn id_of(o: &Bound<'_, PyAny>) -> usize {
    o.as_ptr() as usize
}

fn type_name(o: &Bound<'_, PyAny>) -> String {
    o.get_type().name().map(|n| n.to_string()).unwrap_or_default()
}

/// Send `value` as the inspector's `key`, as JSON (no NaN).
pub fn send(view: &Bound<'_, PyAny>, key: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = view.py();
    let kwargs = [("allow_nan", false)].into_py_dict(py)?;
    let text = py.import("json")?.getattr("dumps")?.call((value,), Some(&kwargs))?;
    view.getattr("renderer")?.call_method1("ui_panel_set", (key, text))?;
    Ok(())
}

/// The saved view marked in the scene tabs, if any.
#[pyfunction]
pub fn marked_view<'py>(win: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyAny>>> {
    let Some(row) = opt(win, "_scene_tabs").and_then(|t| opt(&t, "tabs")) else { return Ok(None) };
    Ok(Some(row.call_method0("marked_view")?).filter(|m| !m.is_none()))
}

/// The address records and commands use for `group`: its occurrence row id
/// when nested (or an instance of a linked file), else its uid.
fn record_address<'py>(scene: &Bound<'py, PyAny>, group: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = scene.py();
    let path = scene.call_method1("occurrence_path", (group,))?;
    let uid = group.getattr("uid")?;
    let nested = path.is_truthy()? && (path.len()? > 1 || uid.str()?.to_str()?.starts_with("@["));
    if nested {
        py.import("built_core.link_structure")?.call_method1("row_id", (path,))
    } else {
        Ok(uid)
    }
}

fn groups_in<'py>(py: Python<'py>, sel: &[Bound<'py, PyAny>]) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let cls = py.import("built_core.group")?.getattr("Group")?;
    Ok(sel.iter().filter(|e| e.is_instance(&cls).unwrap_or(false)).cloned().collect())
}

/// Match the focus stamp used by the project tree and inspector.
fn selection_stamp<'py>(scene: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let selection = scene.getattr("selection")?;
    if selection.len()? == 0 {
        Ok(0i64.into_pyobject(scene.py())?.into_any())
    } else {
        selection.getattr("revision")
    }
}

fn selection(view: &Bound<'_, PyAny>, win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = view.py();
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let selected = scene.getattr("selection")?;
    let focus = opt(view, "_ui_insp_node").filter(|f| f.is_truthy().unwrap_or(false));
    let stamp = selection_stamp(&scene)?;
    let focused = match &focus {
        Some(f) => f.get_item(1)?.eq(&stamp)?,
        None => false,
    };
    let delivery_key = opt(view, "_delivery_key");
    // The selection by its revision (a stand-in set by its members): this
    // runs every frame, and a tuple of every member's id was hundreds of
    // milliseconds a frame with a select-all of a big model.
    let ids = if selected.hasattr("revision")? {
        (id_of(&selected), &stamp).into_pyobject(py)?.into_any()
    } else {
        PyTuple::new(py, selected.try_iter()?.map(|e| e.map(|e| id_of(&e))).collect::<PyResult<Vec<_>>>()?)?.into_any()
    };
    let focus_node = if focused { Some(focus.as_ref().expect("focused").get_item(0)?) } else { None };
    let sig = PyTuple::new(py, [
        id_of(&scene).into_pyobject(py)?.into_any(),
        scene.getattr("version")?,
        ids.into_any(),
        focus_node.clone().unwrap_or_else(|| py.None().into_bound(py)),
        id_of(&scene.getattr("edit_group")?).into_pyobject(py)?.into_any(),
        delivery_key.clone().unwrap_or_else(|| py.None().into_bound(py)),
    ])?;
    if opt(view, "_panel_selection_sig").is_some_and(|s| s.eq(&sig).unwrap_or(false)) {
        return Ok(());
    }
    view.setattr("_panel_selection_sig", &sig)?;
    view.setattr("_assembly_drag", py.None())?;
    // Only the placements are read one by one: a select-all's faces and
    // edges are counted, never walked.
    let count = selected.len()?;
    let sel: Vec<Bound<'_, PyAny>> = if selected.hasattr("values_of")? {
        selected.call_method1("values_of", ("placement",))?.try_iter()?.collect::<PyResult<_>>()?
    } else {
        selected.try_iter()?.collect::<PyResult<_>>()?
    };
    let groups = if focused { Vec::new() } else { groups_in(py, &sel)? };
    let records = PyList::empty(py);
    let mut schema = "IFC4".into_pyobject(py)?.into_any();
    let bim = py.import("core.bim_registry")?;
    let mut vocabulary = py.None().into_bound(py);
    if !groups.is_empty() {
        let registry = bim.call_method1("get", (&scene,))?;
        vocabulary = py.import("core.record_inspector")?.call_method1("vocabulary", (&registry,))?;
        let all = py.import("built_core.deliveries")?.call_method1("records", (&scene,))?;
        let key = delivery_key.clone().unwrap_or_else(|| py.None().into_bound(py));
        let delivery = all.call_method1("get", (key, PyDict::new(py)))?;
        if delivery.call_method1("get", ("format",))?.eq("ifc")? {
            let profile = delivery.call_method1("get", ("options", PyDict::new(py)))?.call_method1("get", ("profile",))?;
            let profiles = registry.call_method1("get", ("export_profiles", PyDict::new(py)))?;
            schema = profiles.call_method1("get", (profile, PyDict::new(py)))?.call_method1("get", ("schema", &schema))?;
        }
        let links = crate::host_context::core_module(py)?;
        let occurrence = py.import("built_core.protocol.records")?.getattr("occurrence")?;
        for g in &groups {
            let linked = links.call_method1("link_linked_ancestor", (g,))?.is_truthy()?;
            let record = occurrence.call1((g, &registry, linked))?;
            if scene.call_method1("entity_locked",(g,))?.is_truthy()? || scene.call_method1("_layer_state",(g,))?.get_item(1)?.is_truthy()? || scene.getattr_opt("worksession_read_only")?.is_some_and(|s|s.is_truthy().unwrap_or(false)) {
                record.set_item("editable",false)?;
                for field in record.get_item("fields")?.try_iter()? {field?.set_item("editable",false)?;}
            }
            let structure = py.import("built_core.structure")?;
            let node = structure.call_method1("node_of", (g,))?;
            if structure.call_method1("_nodes", (&registry,))?.contains(&node)? {
                record.set_item("node", &node)?;
                record.set_item("node_path", structure.call_method1("path", (&registry, &node))?)?;
                let recipe = structure.call_method1("resolve", (&registry, &node))?;
                record.set_item("classes_allowed", recipe.call_method1("get", ("classes_allowed",))?)?;
            }
            let old = g.getattr("ifc")?;
            if !old.is_none() {
                let effective = py.import("core.record")?.call_method1("apply_type", (&old,&registry))?;
                record.set_item("systems", effective.call_method1("get", ("systems",PyList::empty(py)))?)?;
                record.set_item("quantities", effective.call_method1("get", ("quantities",PyDict::new(py)))?)?;
            }
            record.set_item("uid", record_address(&scene, g)?)?;
            if groups.len()==1 && g.getattr("ext")?.is_truthy()? && g.getattr("ext")?.contains("built.live")?{
                record.set_item("sweep",py.import("built_core.command_protocol")?.call_method1("live_sweep",(&scene,g.getattr("uid")?))?)?;
            }
            records.append(record)?;
        }
    }
    let assembly = if focused { None } else { crate::report_sheets::assembly_of(&scene)? };
    let parts = match &assembly {
        Some(g) => py.import("built_core")?.call_method1("assembly_selection", (&scene, g, record_address(&scene, g)?))?,
        None => py.None().into_bound(py),
    };
    let structure = py.import("built_core.structure")?;
    let node = match focus_node {
        Some(n) => Some(n),
        None if groups.len() == 1 => Some(structure.call_method1("node_of", (&groups[0],))?),
        None => None,
    }
    .filter(|n| n.is_truthy().unwrap_or(false));
    let mut rule = py.None().into_bound(py);
    if let Some(node) = node {
        let registry = bim.call_method1("get", (&scene,))?;
        let source = structure.call_method1("_nodes", (registry,))?.call_method1("get", (&node,))?;
        if source.is_truthy()? && !source.contains("source")? {
            let r = PyDict::new(py);
            r.set_item("node", &node)?;
            r.set_item("focused", focused)?;
            r.set_item("name", source.get_item("name")?)?;
            r.set_item("recipe", source.call_method1("get", ("recipe", PyDict::new(py)))?)?;
            rule = r.into_any();
        }
    }
    view.setattr("_panel_schema", &schema)?;
    let geopath = py.import("built_core.georef.geopath")?.getattr("GeoPath")?;
    let out = PyDict::new(py);
    out.set_item("schema", schema)?;
    out.set_item("vocabulary", vocabulary)?;
    out.set_item("selection_count", count)?;
    if !groups.is_empty() && groups.len() == count {
        let face_count = py.import("built_core.mesh")?.getattr("face_count")?;
        let mut faces = 0usize;
        for group in &groups {
            faces += face_count.call1((group.getattr("mesh")?,))?.extract::<usize>()?;
        }
        out.set_item("health_faces", faces)?;
    }
    out.set_item("rule", rule)?;
    out.set_item("records", records)?;
    out.set_item("parts", parts)?;
    let only = if count == 1 { selected.try_iter()?.next().transpose()? } else { None };
    out.set_item("path", match only { Some(one) => one.is_instance(&geopath)?, None => false })?;
    send(view, "selection", out.as_any())
}

#[pyfunction]
fn inspector_selection(view: &Bound<'_, PyAny>, win: &Bound<'_, PyAny>) -> PyResult<()> {
    selection(view, win)
}

fn xyz<'py>(py: Python<'py>, v: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    PyList::new(py, [v.call_method0("x")?, v.call_method0("y")?, v.call_method0("z")?])
}

/// The bevel panel's state while the Fillet tool is active.
fn bevel_state<'py>(py: Python<'py>, tool: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    if !matches!(type_name(tool).as_str(), "FilletTool" | "ChamferTool") {
        return Ok(py.None().into_bound(py));
    }
    let profiles = py.import("built_core.bevel_profiles")?;
    let shape = tool.getattr("shape")?;
    let segments = tool.getattr("segments")?;
    let b = PyDict::new(py);
    b.set_item("affect", tool.getattr("affect")?)?;
    b.set_item("measure", tool.getattr("measure")?)?;
    b.set_item("amount", py.import("built_core.units")?.call_method1("fmt_len_fine", (tool.getattr("radius")?,))?)?;
    // A size the picked edges refuse: the Size field reads in the error
    // colour with the reason (and what fits) under it.
    let refused = match tool.getattr_opt("value_error")? {
        Some(flag) if flag.is_truthy()? => tool.getattr("_message")?,
        _ => py.None().into_bound(py),
    };
    b.set_item("error", refused)?;
    b.set_item("segments", &segments)?;
    b.set_item("shape", &shape)?;
    b.set_item("numeric_profile", tool.getattr("numeric_profile")?)?;
    b.set_item("profile", tool.getattr("profile")?)?;
    b.set_item("outline", if shape.is_truthy()? { profiles.call_method1("sample", (&shape, &segments))? } else { PyList::empty(py).into_any() })?;
    b.set_item("corners", tool.getattr("corners")?)?;
    b.set_item("points", profiles.call_method1("controls", (&shape, &segments))?)?;
    Ok(b.into_any())
}

/// The camera inspector's state, when it is open.
fn camera_state<'py>(py: Python<'py>, view: &Bound<'py, PyAny>, win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let Some(inspector) = opt(view, "_camera_inspector").filter(|i| i.is_truthy().unwrap_or(false)) else {
        return Ok(py.None().into_bound(py));
    };
    let cam = win.call_method0("_active_view")?.getattr("camera")?;
    let marked = marked_view(win)?;
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let none = py.None().into_bound(py);
    let bound = inspector.call_method1("observe", (&cam, &scene, marked.as_ref().unwrap_or(&none)))?;
    let c = cam.call_method0("lens_dict")?.cast_into::<PyDict>()?.copy()?;
    c.set_item("projection", cam.call_method0("projection_mode")?)?;
    c.set_item("focal", cam.getattr("focal_mm")?)?;
    c.set_item("fov", cam.getattr("fov_deg")?)?;
    c.set_item("sensor", cam.call_method0("sensor_size")?)?;
    c.set_item("eye", xyz(py, &cam.call_method0("eye")?)?)?;
    c.set_item("target", xyz(py, &cam.getattr("target")?)?)?;
    c.set_item("saved", marked.is_some())?;
    c.set_item("bound", bound)?;
    c.set_item("advanced", inspector.getattr("advanced")?)?;
    c.set_item("custom_sensor", inspector.getattr("custom_sensor")?)?;
    c.set_item("scene", match &marked {
        Some(m) => m.getattr("name")?,
        None => "Current view".into_pyobject(py)?.into_any(),
    })?;
    Ok(c.into_any())
}

/// Each frame: the document's bindings when the view changed, the
/// selection, the bevel and camera panels when they changed, the terrain
/// profile.
#[pyfunction]
fn inspector_sync(view: &Bound<'_, PyAny>, win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = view.py();
    let vp = win.getattr("viewport")?;
    if opt(&view.getattr("renderer")?, "ui_panel_set").is_none() {
        return Ok(());
    }
    // The native renderer follows the tab. Resend this document's bindings
    // whenever it reacquires the surface, including a new empty document.
    if !opt(win, "_panel_view").is_some_and(|v| v.is(view)) {
        win.setattr("_panel_view", view)?;
        view.setattr("_panel_selection_sig", py.None())?;
        view.setattr("_panel_live", py.None())?;
        send(view, "terrain", &py.None().into_bound(py))?;
        if let Some(binding) = opt(view, "_profile_binding").filter(|b| b.is_truthy().unwrap_or(false)) {
            binding.setattr("dirty", true)?;
        }
    }
    selection(view, win)?;
    let b = bevel_state(py, &vp.getattr("active_tool")?)?;
    let c = camera_state(py, view, win)?;
    let live = PyTuple::new(py, [&b, &c])?;
    if !opt(view, "_panel_live").is_some_and(|l| l.eq(&live).unwrap_or(false)) {
        send(view, "bevel", &b)?;
        send(view, "camera", &c)?;
        view.setattr("_panel_live", live)?;
    }
    if let Some(terrain) = opt(view, "_profile_binding").filter(|b| b.is_truthy().unwrap_or(false)) {
        terrain.call_method1("sync", (view, &vp))?;
    }
    Ok(())
}

fn doc_view<'py>(doc: &Bound<'py, PyAny>) -> Option<Bound<'py, PyAny>> {
    opt(doc, "stand_in").filter(|v| v.is_truthy().unwrap_or(true)).or_else(|| opt(doc, "viewport"))
}

/// Close the camera inspector of a document that leaves the window.
#[pyfunction]
fn leave_camera(doc: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = doc.py();
    let Some(view) = doc_view(doc) else { return Ok(()) };
    if let Some(inspector) = opt(&view, "_camera_inspector").filter(|i| i.is_truthy().unwrap_or(false)) {
        inspector.call_method0("clear")?;
        view.setattr("_camera_inspector", py.None())?;
    }
    Ok(())
}

/// Open the camera inspector on the active view's camera (or `target`).
#[pyfunction]
#[pyo3(signature = (win, target=None))]
fn open_camera<'py>(win: &Bound<'py, PyAny>, target: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyAny>> {
    let py = win.py();
    let doc = win.getattr("_doc")?;
    let view = match opt(&doc, "stand_in").filter(|v| v.is_truthy().unwrap_or(true)) {
        Some(v) => v,
        None => win.getattr("viewport")?,
    };
    leave_camera(&doc)?;
    let inspector = py.import("built_render")?.getattr("CameraInspector")?.call0()?;
    let kwargs = [("type", py.import("builtins")?.getattr("bool")?)].into_py_dict(py)?;
    let advanced = py.import("built_core.settings")?.call_method0("settings")?.getattr("value")?.call(("camera_settings/advanced", false), Some(&kwargs))?;
    inspector.setattr("advanced", advanced)?;
    let none = py.None().into_bound(py);
    inspector.call_method1("bind", (win.call_method0("_active_view")?.getattr("camera")?, target.as_ref().unwrap_or(&none)))?;
    view.setattr("_camera_inspector", &inspector)?;
    view.call_method0("update")?;
    Ok(inspector)
}

fn camera_edit(win: &Bound<'_, PyAny>, view: &Bound<'_, PyAny>, key: &str, v: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let cam = win.call_method0("_active_view")?.getattr("camera")?;
    if key == "open" {
        open_camera(win, None)?;
        return Ok(());
    }
    let Some(inspector) = opt(view, "_camera_inspector").filter(|i| i.is_truthy().unwrap_or(false)) else { return Ok(()) };
    let none = py.None().into_bound(py);
    let scene = vp.getattr("scene")?;
    inspector.call_method1("observe", (&cam, &scene, marked_view(win)?.as_ref().unwrap_or(&none)))?;
    let last_digit = || key.chars().last().and_then(|c| c.to_digit(10)).map(|d| d as usize);
    match key {
        "close" => {
            inspector.call_method0("clear")?;
            view.setattr("_camera_inspector", py.None())?;
        }
        "update" => {
            if let Some(marked) = marked_view(win)? {
                if marked.getattr("name")?.eq(v)? {
                    win.getattr("tray")?.getattr("scenes")?.call_method1("update_view", (marked,))?;
                }
            }
        }
        "reset" => {
            cam.call_method0("reset_lens")?;
        }
        "projection" => {
            cam.call_method1("set_projection", (v,))?;
        }
        "focal" | "fov" => {
            let range = py.import("built_core.camera")?.getattr("FOV_RANGE")?;
            let (lo, hi): (f64, f64) = (range.get_item(0)?.extract()?, range.get_item(1)?.extract()?);
            let wanted: f64 = if key == "focal" { cam.call_method1("fov_for_focal", (v,))?.extract()? } else { v.extract()? };
            cam.setattr("fov_deg", hi.min(lo.max(wanted)))?;
        }
        "advanced" => {
            inspector.setattr("advanced", v)?;
            py.import("built_core.settings")?.call_method0("settings")?.call_method1("setValue", ("camera_settings/advanced", v))?;
        }
        "sensor" => {
            inspector.setattr("custom_sensor", v.is_none())?;
            if !v.is_none() {
                cam.call_method1("set_sensor", (v,))?;
            }
        }
        _ if key.starts_with("sensor") => {
            let size = PyList::new(py, cam.call_method0("sensor_size")?.try_iter()?.collect::<PyResult<Vec<_>>>()?)?;
            if let Some(i) = last_digit() {
                size.set_item(i, v)?;
            }
            cam.call_method1("set_sensor", (size,))?;
        }
        _ if key.starts_with("eye") || key.starts_with("target") => {
            let pts = [xyz(py, &cam.call_method0("eye")?)?, xyz(py, &cam.getattr("target")?)?];
            if let Some(i) = last_digit() {
                pts[usize::from(key.starts_with("target"))].set_item(i, v)?;
            }
            let point = py.import("built_core")?.getattr("Vec3")?;
            let a = point.call1(pts[0].to_tuple())?;
            let b = point.call1(pts[1].to_tuple())?;
            cam.call_method1("place", (a, b))?;
        }
        "frame_aspect" | "shift_x" | "shift_y" | "roll_deg" | "clip_near" | "clip_far" => {
            let lens = cam.call_method0("lens_dict")?.cast_into::<PyDict>()?.copy()?;
            lens.set_item(key, if v.is_truthy()? { v.clone() } else { py.None().into_bound(py) })?;
            cam.call_method1("apply_lens", (lens,))?;
        }
        _ => {}
    }
    inspector.call_method1("applied", (&cam, &scene))?;
    vp.call_method0("update")?;
    Ok(())
}

/// A bevel panel edit for the Fillet tool.
#[pyfunction]
fn bevel_edit(host: &Bound<'_, PyAny>, tool: &Bound<'_, PyAny>, key: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = host.py();
    if !matches!(type_name(tool).as_str(), "FilletTool" | "ChamferTool") {
        return Ok(());
    }
    let text = value.extract::<String>().ok();
    match (key, text.as_deref()) {
        ("apply" | "finish", _) => {
            tool.call_method1("apply_from_panel", (host,))?;
        }
        ("amount", _) => {
            let n = py.import("built_core.units")?.call_method1("parse_length_field", (value,))?;
            let number = !n.is_instance_of::<pyo3::types::PyBool>() && (n.is_instance_of::<pyo3::types::PyInt>() || n.is_instance_of::<pyo3::types::PyFloat>());
            if number && n.extract::<f64>()? > 0.0 {
                tool.call_method1("set_size", (host, n))?;
            }
        }
        ("segments", _) => {
            tool.call_method1("on_segments_value", (host, value))?;
        }
        ("affect" | "measure" | "corners", _) => {
            tool.call_method1(format!("set_{key}").as_str(), (host, value))?;
        }
        ("shape", Some(s @ ("round" | "chamfer"))) => {
            tool.call_method1("on_profile_value", (host, if s == "round" { 0.5 } else { 0.25 }))?;
            tool.get_type().setattr("numeric_profile", false)?;
        }
        ("profile", _) => {
            tool.call_method1("on_profile_value", (host, value))?;
        }
        ("shape", Some("profile value")) => {
            tool.call_method1("set_shape", (host, py.None()))?;
            tool.get_type().setattr("numeric_profile", true)?;
        }
        ("shape", Some("drawn" | "custom")) => {
            let controls = py.import("built_core.bevel_profiles")?.call_method1("controls", (tool.getattr("shape")?, tool.getattr("segments")?))?;
            tool.call_method1("set_shape", (host, controls))?;
        }
        ("shape" | "curve", _) => {
            tool.call_method1("set_shape", (host, value))?;
        }
        _ => {}
    }
    Ok(())
}

fn commands(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("built_core.command_protocol")?.getattr("run")
}

fn record_edit(view: &Bound<'_, PyAny>, vp: &Bound<'_, PyAny>, verb:&str, v: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = view.py();
    let scene = vp.getattr("scene")?;
    let placements = v.get_item(0)?;
    if scene.getattr_opt("worksession_read_only")?.is_some_and(|s|s.is_truthy().unwrap_or(false)) {return Ok(());}
    let sel: Vec<Bound<'_, PyAny>> = scene.getattr("selection")?.try_iter()?.collect::<PyResult<_>>()?;
    let targets = PyDict::new(py);
    for g in groups_in(py, &sel)? {
        targets.set_item(record_address(&scene, &g)?, g)?;
    }
    let mut paths = Vec::new();
    for uid in placements.try_iter()? {
        let Some(g) = targets.get_item(uid?)? else { return Ok(()) };
        paths.push(scene.call_method1("occurrence_path", (g,))?);
    }
    if verb != "patch" {
        let schema = v.get_item(2)?;
        py.import("core.record_inspector")?.call_method1("edit", (vp.getattr("history")?,paths,verb,v.get_item(1)?,schema))?;
        vp.call_method0("notify_scene_changed")?;
        return Ok(());
    }
    let (path,value)=(v.get_item(1)?,v.get_item(2)?);
    let kind = if v.len()? > 3 { v.get_item(3)? } else { py.None().into_bound(py) };
    let kwargs = PyDict::new(py);
    kwargs.set_item("placements", paths)?;
    kwargs.set_item("path", path)?;
    kwargs.set_item("value", value)?;
    kwargs.set_item("property_kind", kind)?;
    kwargs.set_item("schema", opt(view, "_panel_schema").unwrap_or_else(|| "IFC4".into_pyobject(py).expect("str").into_any()))?;
    commands(py)?.call((vp.getattr("history")?, "patch_record_field"), Some(&kwargs))?;
    vp.call_method0("notify_scene_changed")?;
    Ok(())
}

fn rule_edit(view: &Bound<'_, PyAny>, vp: &Bound<'_, PyAny>, verb: &str, v: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = view.py();
    if verb == "open" {
        let scene = vp.getattr("scene")?;
        view.setattr("_ui_insp_node", (v, selection_stamp(&scene)?))?;
        return Ok(());
    }
    let (node, expected, recipe) = (v.get_item(0)?, v.get_item(1)?, v.get_item(2)?);
    if verb == "preview" {
        let kwargs = [("expected", &expected)].into_py_dict(py)?;
        let filing_rule = py.import("built_core.protocol.records")?.getattr("filing_rule")?;
        let result = match filing_rule.call((vp.getattr("scene")?, &node, &recipe), Some(&kwargs)) {
            Ok(r) => r.cast_into::<PyDict>()?.copy()?,
            Err(e) if e.is_instance_of::<pyo3::exceptions::PyKeyError>(py) || e.is_instance_of::<pyo3::exceptions::PyTypeError>(py) || e.is_instance_of::<pyo3::exceptions::PyValueError>(py) => {
                let d = PyDict::new(py);
                d.set_item("error", e.value(py).str()?)?;
                d
            }
            Err(e) => return Err(e),
        };
        result.set_item("request", v)?;
        send(view, "rulePreview", result.as_any())?;
    } else if verb == "apply" {
        let kwargs = PyDict::new(py);
        kwargs.set_item("node", node)?;
        kwargs.set_item("expected", expected)?;
        kwargs.set_item("recipe", recipe)?;
        commands(py)?.call((vp.getattr("history")?, "set_filing_rule"), Some(&kwargs))?;
        vp.call_method0("notify_scene_changed")?;
    }
    Ok(())
}

fn parts_edit(win: &Bound<'_, PyAny>, view: &Bound<'_, PyAny>, vp: &Bound<'_, PyAny>, verb: &str, v: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = view.py();
    let scene = vp.getattr("scene")?;
    let g = crate::report_sheets::assembly_of(&scene)?;
    let address = match &g {
        Some(g) => Some(record_address(&scene, g)?),
        None => None,
    };
    if verb == "measure" {
        if let (Some(g), Some(address)) = (&g, &address) {
            let current = PyList::new(py, [address.clone(), scene.getattr("content_version")?])?;
            if v.eq(&current)? {
                let out = PyDict::new(py);
                out.set_item("target", v)?;
                out.set_item("rows", py.import("built_core")?.call_method1("part_measures", (g,))?)?;
                send(view, "partMeasures", out.as_any())?;
            }
        }
        return Ok(());
    }
    if verb == "report" {
        if g.is_some() && address.as_ref().is_some_and(|a| a.eq(v).unwrap_or(false)) {
            py.import("built_render")?.call_method1("show_report_sheet", (win, "cutlist"))?;
        }
        return Ok(());
    }
    let uid = if verb == "split" { v.clone() } else { v.get_item(0)? };
    let (Some(g), Some(address)) = (g, address) else { return Ok(()) };
    if !address.eq(&uid)?
        || crate::host_context::core_module(py)?.call_method1("link_linked_ancestor", (&g,))?.is_truthy()?
        || scene.call_method1("entity_locked", (&g,))?.is_truthy()?
    {
        return Ok(());
    }
    let drag = opt(view, "_assembly_drag");
    if verb == "preview" {
        if let Some(packed) = opt(view, "packed") {
            let drag = match drag {
                Some(d) => d,
                None => {
                    let d = py.import("built_core")?.getattr("AssemblyPreview")?.call1((&scene, &g, packed, view.getattr("uid_rows")?, v.get_item(2)?))?;
                    view.setattr("_assembly_drag", &d)?;
                    d
                }
            };
            view.getattr("renderer")?.call_method1("preview_parts", (drag.call_method1("offsets", (v.get_item(1)?,))?,))?;
            view.call_method0("update")?;
        }
        return Ok(());
    }
    if (verb == "explode" || verb == "cancel") && drag.is_some() {
        view.setattr("_assembly_drag", py.None())?;
        view.getattr("renderer")?.call_method1("preview_parts", (PyList::empty(py),))?;
    }
    if verb == "cancel" {
        view.call_method0("update")?;
        return Ok(());
    }
    if verb == "select" || verb == "select_many" {
        let requested: Vec<String> = if verb == "select" {
            vec![v.get_item(1)?.str()?.to_string()]
        } else {
            v.get_item(1)?.try_iter()?.map(|u| Ok(u?.str()?.to_string())).collect::<PyResult<_>>()?
        };
        let mut wanted = requested.clone();
        wanted.sort();
        wanted.dedup();
        let mut picks = Vec::new();
        for c in g.getattr("children")?.try_iter()? {
            let c = c?;
            if wanted.contains(&c.getattr("uid")?.str()?.to_string())
                && !c.getattr("hidden")?.is_truthy()?
                && scene.call_method1("entity_selectable", (&c,))?.is_truthy()?
            {
                picks.push(c);
            }
        }
        if picks.len() != wanted.len() {
            return Ok(());
        }
        if !scene.getattr("edit_group")?.is(&g) {
            vp.call_method1("begin_group_edit", (&g,))?;
        }
        let scene = vp.getattr("scene")?;
        if scene.getattr("edit_group")?.is(&g) {
            scene.call_method1("select", (picks,))?;
            scene.call_method0("bump_view")?;
            view.call_method0("update")?;
        }
        return Ok(());
    }
    if scene.getattr("edit_group")?.is(&g) && (verb == "explode" || verb == "split") {
        vp.call_method0("end_group_edit")?;
        vp.getattr("scene")?.call_method1("select", (vec![&g],))?;
    }
    let history = vp.getattr("history")?;
    let placement = vp.getattr("scene")?.call_method1("occurrence_path", (&g,))?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("placement", placement)?;
    match verb {
        "visible" | "rename" => {
            kwargs.set_item("child", v.get_item(1)?)?;
            kwargs.set_item(if verb == "visible" { "visible" } else { "name" }, v.get_item(2)?)?;
            commands(py)?.call((history, "edit_assembly_part"), Some(&kwargs))?;
        }
        "split" => {
            commands(py)?.call((history, "split_into_pieces"), Some(&kwargs))?;
        }
        "explode" => {
            kwargs.set_item("factor", v.get_item(1)?)?;
            kwargs.set_item("mode", v.get_item(2)?)?;
            commands(py)?.call((history, "explode_view"), Some(&kwargs))?;
        }
        _ => {}
    }
    vp.call_method0("notify_scene_changed")?;
    Ok(())
}

/// `panel:<json [surface.verb, value]>` from the inspector.
#[pyfunction]
fn inspector_action(win: &Bound<'_, PyAny>, view: &Bound<'_, PyAny>, payload: &str) -> PyResult<()> {
    let py = win.py();
    let msg = py.import("json")?.call_method1("loads", (payload,))?;
    let key: String = msg.get_item(0)?.extract()?;
    let v = msg.get_item(1)?;
    let (surface, verb) = key.split_once('.').ok_or_else(|| pyo3::exceptions::PyValueError::new_err(key.clone()))?;
    let vp = win.getattr("viewport")?;
    let result = match surface {
        "sweep" => {
            let row=py.import("built_core.command_protocol")?.call_method1("live_sweep",(vp.getattr("scene")?,&v))?;
            if !row.is_none(){
                if verb=="path"{
                    win.call_method1("_activate_tool",("sweep",))?;
                    win.getattr("_tools")?.get_item("sweep")?.getattr("_machine")?.call_method1("set",("edit",&row))?;
                }else if matches!(verb,"mitre"|"butt"){
                    row.set_item("corner",verb)?;
                    let kw=PyDict::new(py);kw.set_item("row",&row)?;
                    commands(py)?.call((vp.getattr("history")?,"live_set"),Some(&kw))?;
                    vp.call_method0("notify_scene_changed")?;
                }else if verb=="release"{
                    let kw=PyDict::new(py);kw.set_item("uid",row.get_item("uid")?)?;
                    commands(py)?.call((vp.getattr("history")?,"live_release"),Some(&kw))?;
                    vp.call_method0("notify_scene_changed")?;
                }
            } Ok(())
        }
        "record" => record_edit(view, &vp, verb, &v),
        "rule" => rule_edit(view, &vp, verb, &v),
        "parts" => parts_edit(win, view, &vp, verb, &v),
        "bevel" => {
            let tools = view.getattr("tools")?;
            let history = vp.getattr("history")?;
            let revision = history.getattr("revision")?;
            bevel_edit(&tools, &tools.getattr("tool")?, verb, &v)?;
            if verb == "finish" && history.getattr("revision")?.ne(&revision)? {
                win.call_method1("_activate_tool", ("select",))?;
            }
            Ok(())
        }
        "camera" => camera_edit(win, view, verb, &v),
        "terrain" => terrain::action(view,&vp,verb),
        _ => Ok(()),
    };
    if let Err(e) = result {
        let shown = e.is_instance_of::<pyo3::exceptions::PyKeyError>(py)
            || e.is_instance_of::<pyo3::exceptions::PyValueError>(py)
            || e.is_instance_of::<pyo3::exceptions::PyOSError>(py);
        if !shown {
            return Err(e);
        }
        vp.call_method1("flash_status", (e.value(py).str()?,))?;
    }
    view.call_method0("update")?;
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    for f in [
        wrap_pyfunction!(marked_view, m)?,
        wrap_pyfunction!(inspector_selection, m)?,
        wrap_pyfunction!(inspector_sync, m)?,
        wrap_pyfunction!(leave_camera, m)?,
        wrap_pyfunction!(open_camera, m)?,
        wrap_pyfunction!(bevel_edit, m)?,
        wrap_pyfunction!(inspector_action, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
