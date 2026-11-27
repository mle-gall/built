//! Context-menu edits of the active document. The menus and inspector share
//! these operations, including their history commands and selection rules.
use super::model_edit::{matching, translated};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::path::Path;

fn selected<'py>(
    win: &Bound<'py, PyAny>,
    module: &str,
    class: &str,
) -> PyResult<Bound<'py, PyList>> {
    matching(
        &win.getattr("viewport")?
            .getattr("scene")?
            .getattr("selection")?,
        module,
        class,
    )
}
fn execute(view: &Bound<'_, PyAny>, cmd: &Bound<'_, PyAny>) -> PyResult<()> {
    view.getattr("history")?.call_method1("execute", (cmd,))?;
    Ok(())
}
fn status(
    win: &Bound<'_, PyAny>,
    text: &str,
    vars: Option<&Bound<'_, PyDict>>,
    ms: u32,
) -> PyResult<()> {
    win.call_method0("statusBar")?
        .call_method1("showMessage", (translated(win.py(), text, vars)?, ms))?;
    Ok(())
}

#[pyfunction]
pub fn unique_components(win: &Bound<'_, PyAny>) -> PyResult<()> {
    unique_rows(win, &selected(win, "built_core.group", "Group")?, false)
}

pub(super) fn unique_rows(
    win: &Bound<'_, PyAny>,
    groups: &Bound<'_, PyList>,
    batch: bool,
) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let links = py.import("built_core.links")?;
    let hist = py.import("built_core.history")?;
    let linked = PyList::empty(py);
    let kw = PyDict::new(py);
    kw.set_item("rename", true)?;
    let steps = PyList::empty(py);
    for group in groups.iter() {
        if !group.call_method0("is_component")?.is_truthy()? {
            continue;
        }
        if links.call_method1("is_linked", (&group,))?.is_truthy()? {
            linked.append(group)?;
        } else {
            steps.append(
                hist.getattr("MakeUniqueCommand")?
                    .call((group,), Some(&kw))?,
            )?;
        }
    }
    let cmd = hist.call_method1("UnlinkCommand", (linked,))?;
    let unlink = cmd.getattr("groups")?.is_truthy()?;
    if unlink {
        steps.append(&cmd)?;
    }
    if steps.is_empty() {
        return Ok(());
    }
    let last = if batch && steps.len() > 1 {
        let all = hist.call_method1("CompoundCommand", (&steps,))?;
        execute(&vp, &all)?;
        all
    } else {
        for step in steps.iter() {
            execute(&vp, &step)?;
        }
        steps.get_item(steps.len() - 1)?
    };
    if unlink {
        let stack = vp.getattr("history")?.getattr("undo_stack")?;
        if stack.is_truthy()? && stack.get_item(-1)?.is(&last) {
            win.call_method1("_unlinked", (&cmd,))?;
            let mut names = Vec::new();
            for path in cmd.getattr("paths")?.try_iter()? {
                let path = path?.str()?.to_string();
                names.push(
                    Path::new(&path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
            let kw = PyDict::new(py);
            kw.set_item("name", names.join(", "))?;
            vp.call_method1(
                "flash_status",
                (
                    translated(
                        py,
                        "“{name}” is no longer linked: its geometry is now part of this model",
                        Some(&kw),
                    )?,
                    5000,
                ),
            )?;
        }
    }
    vp.call_method0("update")?;
    Ok(())
}

#[pyfunction]
fn merge_groups(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let groups = selected(win, "built_core.group", "Group")?;
    if groups.len() < 2 {
        return Ok(());
    }
    let py = win.py();
    let vars = PyDict::new(py);
    vars.set_item("n", groups.len())?;
    let vp = win.getattr("viewport")?;
    vp.call_method0("end_group_edit")?;
    execute(
        &vp,
        &py.import("built_core.history")?
            .call_method1("MergeGroupsCommand", (groups,))?,
    )?;
    vp.call_method0("update")?;
    status(win, "{n} groups merged into one", Some(&vars), 3000)
}

pub fn can_weld(scene: &Bound<'_, PyAny>) -> PyResult<bool> {
    let selection = scene.getattr("selection")?;
    if selection.len()? < 2 {
        return Ok(false);
    }
    let snap = selection.call_method0("snapshot")?;
    if snap.get_item(3)?.extract::<usize>()? != selection.len()? {
        return Ok(false);
    }
    let mesh = scene.getattr("mesh")?;
    let store = mesh.getattr("_store")?;
    let stamp = (
        snap.get_item(6)?.extract::<u64>()?,
        store.getattr("revision")?.extract::<u64>()?,
        store.as_ptr() as usize,
    );
    thread_local! { static LAST: std::cell::Cell<Option<((u64, u64, usize), bool)>> = const { std::cell::Cell::new(None) }; }
    if let Some((_, valid)) = LAST.get().filter(|(prior, _)| *prior == stamp) {
        return Ok(valid);
    }
    let keys = selection.call_method1("element_keys", ("edge",))?;
    let valid = store.call_method1("can_weld_edges", (keys,))?.is_truthy()?;
    LAST.set(Some((stamp, valid)));
    Ok(valid)
}

#[pyfunction]
fn explode_curves(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let mesh_edges = vp.getattr("scene")?.getattr("mesh")?.getattr("edges")?;
    let edges = PyList::empty(py);
    let mut curved = false;
    for edge in selected(win, "built_core.mesh", "Edge")?.iter() {
        if mesh_edges.contains(&edge)? {
            curved |= !edge.getattr("curve")?.is_none();
            edges.append(edge)?;
        }
    }
    if !curved {
        return status(win, "No curve selected.", None, 3000);
    }
    let cmd = py
        .import("built_core.history")?
        .call_method1("ExplodeCurvesCommand", (edges,))?;
    execute(&vp, &cmd)?;
    let vars = PyDict::new(py);
    vars.set_item("n", cmd.getattr("curves")?)?;
    vp.call_method0("update")?;
    status(win, "{n} curves exploded into segments", Some(&vars), 3000)
}

fn face_camera(
    vp: &Bound<'_, PyAny>,
    point: Option<&Bound<'_, PyAny>>,
    normal: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let (x, y, z): (f64, f64, f64) = (
        normal.call_method0("x")?.extract()?,
        normal.call_method0("y")?.extract()?,
        normal.call_method0("z")?.extract()?,
    );
    let cam = vp.getattr("camera")?;
    let before: (f64, f64) = (cam.getattr("yaw")?.extract()?, cam.getattr("pitch")?.extract()?);
    if let Some(point) = point {
        cam.setattr("target", point)?;
    } else {
        crate::native_view_actions::alignment_focus(vp)?;
    }
    cam.setattr("pitch", z.clamp(-1., 1.).asin())?;
    if x.abs() > 1e-9 || y.abs() > 1e-9 {
        cam.setattr("yaw", y.atan2(x))?;
    }
    // Looking square at a face is an aligned view: auto perspective (M03).
    let auto = crate::native_view_actions::auto_perspective(vp.py())?;
    cam.call_method1("align_current", (before.0, before.1, auto))?;
    Ok(())
}

/// The camera's (yaw, pitch) looking straight at a face with `normal`
/// (world, unit or not) and outline `points`: from the side the normal
/// points to. The screen's up is the world's up, so a wall's or a roof's
/// horizontal edges run across the screen; a horizontal face turns the
/// least from `yaw_now` that lays its longest edge across. None for a
/// degenerate normal.
pub(crate) fn look_angles(normal: glam::DVec3, points: &[glam::DVec3], yaw_now: f64) -> Option<(f64, f64)> {
    use std::f64::consts::{FRAC_PI_2, PI, TAU};
    let n = normal.try_normalize()?;
    let pitch = n.z.clamp(-1., 1.).asin();
    if n.truncate().length() > 1e-3 {
        return Some((n.y.atan2(n.x), pitch));
    }
    // Looking down (or up): the screen's up is -(cos yaw, sin yaw), so an
    // edge runs across when it is square to (cos yaw, sin yaw).
    let longest = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| (*b - *a).truncate())
        .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))
        .filter(|e| e.length() > 1e-12);
    let Some(e) = longest else { return Some((yaw_now, pitch)) };
    let base = e.y.atan2(e.x);
    let turn = |yaw: f64| ((yaw - yaw_now + PI).rem_euclid(TAU) - PI).abs();
    let yaw = [base + FRAC_PI_2, base - FRAC_PI_2, base + 3. * FRAC_PI_2]
        .into_iter()
        .min_by(|a, b| turn(*a).total_cmp(&turn(*b)))?;
    let yaw = yaw_now + ((yaw - yaw_now + PI).rem_euclid(TAU) - PI);
    Some((yaw, pitch))
}

/// The world matrix of what the face hangs in: a picked placement's, or
/// the open group's when the face comes from the selection.
fn face_matrix(scene: &Bound<'_, PyAny>, placement: Option<&Bound<'_, PyAny>>) -> PyResult<glam::DMat4> {
    let matrix = match placement.filter(|p| !p.is_none()) {
        Some(p) => p.getattr_opt("xform")?.filter(|m| !m.is_none()),
        None => crate::edge_commands::placement(scene)?,
    };
    Ok(match matrix {
        Some(m) => glam::DMat4::from_cols_array(&m.call_method0("data")?.extract()?),
        None => glam::DMat4::IDENTITY,
    })
}

/// Look at Face (V; View menu; right-click on a face): the camera turns
/// to look straight at the face from the side it sees, in parallel
/// projection until the next orbit (Auto perspective), its horizontal
/// edges (a floor's longest edge) across the screen, at the same scale.
/// `face` is the right-clicked face; without it, the face under the
/// cursor. The move glides like a standard view.
#[pyfunction]
#[pyo3(signature=(win, face=None))]
pub(crate) fn look_at_face(win: &Bound<'_, PyAny>, face: Option<Bound<'_, PyAny>>) -> PyResult<bool> {
    use crate::document_host::{vec3, xyz};
    let vp = win.call_method0("_active_view")?;
    let scene = vp.getattr("scene")?;
    let mut cursor = None;
    let (face, placement) = match face.filter(|f| !f.is_none()) {
        Some(f) => (f, None),
        None => {
            // The window's drawing view takes the pointer (`_cursor_pos`)
            // and picks through its tools; a view without one (tests)
            // keeps its own hover position and picks itself.
            let display = vp.getattr_opt("keys_stand_in")?.filter(|v| !v.is_none());
            let shown = display.as_ref()
                .and_then(|v| v.getattr_opt("_cursor_pos").ok().flatten())
                .filter(|p| !p.is_none());
            let (picker, at) = match (shown, &display) {
                (Some(at), Some(view)) => {
                    let tools = view.getattr_opt("tools")?.filter(|t| t.hasattr("pick_face_placement").unwrap_or(false));
                    (tools.unwrap_or_else(|| vp.clone()), Some(at))
                }
                _ => (vp.clone(), vp.getattr_opt("_last_mouse_pos")?.filter(|p| !p.is_none())),
            };
            let picked = match at {
                Some(at) => {
                    let (x, y): (f64, f64) = (at.call_method0("x")?.extract()?, at.call_method0("y")?.extract()?);
                    cursor = Some((x, y));
                    let (f, p): (Bound<'_, PyAny>, Bound<'_, PyAny>) =
                        picker.call_method1("pick_face_placement", (x, y))?.extract()?;
                    (!f.is_none()).then_some((f, Some(p)))
                }
                None => None,
            };
            match picked {
                Some(hit) => hit,
                None => {
                    status(win, "Point at a face, then press V to look at it.", None, 3000)?;
                    return Ok(false);
                }
            }
        }
    };
    let m = face_matrix(&scene, placement.as_ref())?;
    let mut points = Vec::new();
    if let Some(vertices) = face.getattr_opt("vertices")? {
        for p in vertices.try_iter()? {
            points.push(m.transform_point3(xyz(&p?)?));
        }
    }
    let mut normal = m.transform_vector3(xyz(&face.call_method0("normal")?)?);
    // The point of the face under the cursor: where its ray meets the plane.
    let hovered = match (cursor, points.first()) {
        (Some((x, y)), Some(on)) => {
            let ray = vp.call_method1("_pixel_to_ray", (x, y)).ok().filter(|r| !r.is_none());
            match ray {
                Some(ray) => {
                    let (o, d) = (xyz(&ray.get_item(0)?)?, xyz(&ray.get_item(1)?)?);
                    let n = normal.normalize_or_zero();
                    let t = (*on - o).dot(n) / d.dot(n);
                    (t.is_finite() && t > 0.).then(|| o + d * t)
                }
                None => None,
            }
        }
        _ => None,
    };
    let cam = vp.getattr("camera")?;
    // From the side the camera sees: a floor drawn face down is still
    // looked at from above.
    let centre = points.iter().copied().sum::<glam::DVec3>() / points.len().max(1) as f64;
    if !points.is_empty() && (xyz(&cam.call_method0("eye")?)? - centre).dot(normal) < 0. {
        normal = -normal;
    }
    let before: (f64, f64) = (cam.getattr("yaw")?.extract()?, cam.getattr("pitch")?.extract()?);
    let Some((yaw, pitch)) = look_angles(normal, &points, before.0) else {
        return Ok(false);
    };
    let py = win.py();
    let auto = crate::native_view_actions::auto_perspective(py)?;
    crate::native_view_actions::glide_after(win, |vp| {
        // The point looked at comes to the centre at the scale it had: the
        // face's point under the cursor (V), else what is under the centre
        // of the view, as a standard view keeps it.
        match hovered {
            Some(point) => {
                let cam = vp.getattr("camera")?;
                if cam.getattr("perspective")?.is_truthy()? {
                    let eye = xyz(&cam.call_method0("eye")?)?;
                    let depth = (point - eye).dot(xyz(&cam.call_method0("forward")?)?);
                    if depth.is_finite() && depth > 0. {
                        cam.setattr("distance", depth)?;
                    }
                }
                cam.setattr("target", vec3(py, point)?)?;
            }
            None => crate::native_view_actions::alignment_focus(vp)?,
        }
        let cam = vp.getattr("camera")?;
        cam.setattr("yaw", yaw)?;
        cam.setattr("pitch", pitch)?;
        // Square to a face is an aligned view: parallel until the next
        // orbit (Settings › Navigation › Auto perspective).
        cam.call_method1("align_current", (before.0, before.1, auto))?;
        Ok(())
    })?;
    if vp.hasattr("_sync_projection")? {
        vp.call_method0("_sync_projection")?;
    }
    vp.call_method0("update")?;
    Ok(true)
}

#[pyfunction]
fn section_action(win: &Bound<'_, PyAny>, verb: &str) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let planes = selected(win, "built_core.section", "SectionPlane")?;
    let hist = py.import("built_core.history")?;
    match verb {
        "align" => {
            let plane = if planes.is_empty() {
                vp.getattr("scene")?.call_method0("active_section")?
            } else {
                planes.get_item(0)?
            };
            if plane.is_none() {
                return Ok(());
            }
            let target = py
                .import("built_core")?
                .call_method1("Vec3", (plane.getattr("point")?,))?;
            face_camera(&vp, Some(&target), &plane.getattr("normal")?)?;
        }
        "reverse" => {
            for plane in planes.iter() {
                execute(
                    &vp,
                    &hist.call_method1("ReverseSectionPlaneCommand", (plane,))?,
                )?;
            }
        }
        "toggle" => {
            if planes.is_empty() {
                return Ok(());
            }
            let plane = planes.get_item(0)?;
            let target = if plane.getattr("active")?.is_truthy()? {
                None
            } else {
                Some(plane)
            };
            execute(
                &vp,
                &hist.call_method1("SetActiveSectionCommand", (target,))?,
            )?;
        }
        _ => {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "Unknown section action",
            ))
        }
    }
    vp.call_method0("update")?;
    Ok(())
}

#[pyfunction]
pub(crate) fn context_lock(vp: &Bound<'_, PyAny>, groups: Vec<Bound<'_, PyAny>>, lock: bool) -> PyResult<()> {
    let scene = vp.getattr("scene")?;
    if scene.getattr("worksession_read_only").is_ok_and(|v| v.is_truthy().unwrap_or(false)) { return Ok(()); }
    let py = vp.py();
    let root = scene.getattr("_root")?;
    let open = root.getattr("_edit_path")?;
    let picks = PyList::empty(py);
    let mut editing = false;
    for g in groups {
        if root.call_method1("_occurrence_path", (&g,))?.is_none() || g.getattr("locked")?.is_truthy()? == lock { continue; }
        editing |= open.contains(&g)?;
        picks.append(g)?;
    }
    if picks.is_empty() { return Ok(()); }
    if editing && lock { vp.call_method0("end_group_edit")?; }
    py.import("built_render.tree_navigation")?.call_method1("execute", (
        vp, py.import("built_core")?.getattr("NativeCommand")?.call_method1("lock", (picks, lock))?,
    ))?;
    Ok(())
}

#[pyfunction]
fn unlock_group(vp: &Bound<'_, PyAny>, group: &Bound<'_, PyAny>) -> PyResult<()> {
    let groups = PyList::new(vp.py(), [group])?;
    vp.py()
        .import("built_core.command_protocol")?
        .call_method1("lock", (vp.getattr("history")?, &groups, false))?;
    let scene = vp.getattr("scene")?;
    if scene
        .call_method1("entity_selectable", (group,))?
        .is_truthy()?
    {
        scene.call_method1("select", (groups,))?;
    }
    vp.call_method0("update")?;
    Ok(())
}

#[pyfunction]
pub(crate) fn assign_tag(win: &Bound<'_, PyAny>, name: &str) -> PyResult<()> {
    let vp = win.getattr("viewport")?;
    let py = win.py();
    let layers = py.import("built_core.layers")?;
    let entities =
        layers.call_method1("taggable", (vp.getattr("scene")?.getattr("selection")?,))?;
    let targets = PyList::empty(py);
    for e in entities.try_iter()? {
        let e = e?;
        if !layers.call_method1("layer_of", (&e,))?.eq(name)? {
            targets.append(e)?;
        }
    }
    if targets.is_empty() {
        return Ok(());
    }
    let vars = PyDict::new(py);
    vars.set_item("n", targets.len())?;
    vars.set_item("layer", name)?;
    execute(
        &vp,
        &py.import("built_core.history")?
            .call_method1("AssignLayerCommand", (targets, name))?,
    )?;
    vp.call_method0("update")?;
    status(win, "{n} entities moved to '{layer}'", Some(&vars), 2500)
}

#[pyfunction]
pub(crate) fn next_tag(scene: &Bound<'_, PyAny>) -> PyResult<String> {
    let base = translated(scene.py(), "Layer", None)?.extract::<String>()?;
    for n in 1.. {
        let name = format!("{base} {n}");
        if scene.call_method1("layer", (&name,))?.is_none() {
            return Ok(name);
        }
    }
    unreachable!()
}

#[pyfunction]
fn toggle_paths(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let paths = selected(win, "built_core.georef.geopath", "GeoPath")?;
    if !paths.is_empty() {
        let vp = win.getattr("viewport")?;
        execute(
            &vp,
            &win.py()
                .import("built_core.history")?
                .call_method1("ToggleGeoPathClosedCommand", (paths,))?,
        )?;
        vp.call_method0("update")?;
    }
    Ok(())
}

#[pyfunction]
fn set_path_surface(win: &Bound<'_, PyAny>, mode: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let paths = PyList::empty(py);
    for path in selected(win, "built_core.georef.geopath", "GeoPath")?.iter() {
        if path.getattr("points")?.len()? >= 3 {
            paths.append(path)?;
        }
    }
    if !paths.is_empty() {
        execute(
            &win.getattr("viewport")?,
            &py.import("built_core.history")?
                .call_method1("SetGeoPathSurfaceCommand", (paths, mode))?,
        )?;
        win.call_method0("_rebuild_surfaces")?;
    }
    Ok(())
}

#[pyfunction]
fn refresh_surfaces(weak_view: &Bound<'_, PyAny>, weak_datum: &Bound<'_, PyAny>) -> PyResult<()> {
    let vp = weak_view.call0()?;
    if !vp.is_none()
        && vp
            .getattr("scene")?
            .getattr("georef")?
            .is(&weak_datum.call0()?)
    {
        rebuild_path_surfaces(&vp)?;
        crate::document_terrain::rebuild_terrain(&vp)?;
    }
    Ok(())
}

pub(super) fn surface_sampler<'py>(
    vp: &Bound<'py, PyAny>,
    datum: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let py = vp.py();
    let cached = vp.getattr("_surface_sampler").ok();
    let sampler = if let Some(s) = cached.filter(|s| s.getattr("datum").is_ok_and(|d| d.is(datum)))
    {
        s
    } else {
        let s = py
            .import("built_core.georef.dem")?
            .call_method1("DEMSampler", (datum,))?;
        if s.hasattr("_host")? {
            s.setattr("_host", vp.getattr("_host")?)?;
        }
        let weak = py.import("weakref")?;
        let done = py.import("functools")?.call_method1(
            "partial",
            (
                wrap_pyfunction!(refresh_surfaces, py)?,
                weak.call_method1("ref", (vp,))?,
                weak.call_method1("ref", (datum,))?,
            ),
        )?;
        s.getattr("changed")?.call_method1("connect", (done,))?;
        vp.setattr("_surface_sampler", &s)?;
        s
    };
    Ok(sampler)
}

#[pyfunction]
fn rebuild_path_surfaces(vp: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = vp.py();
    let scene = vp.getattr("scene")?;
    let datum = scene.getattr("georef")?;
    let mut paths = Vec::new();
    for path in scene.getattr("geo_paths")?.try_iter()? {
        let p = path?;
        if p.getattr("surface")?.is_truthy()? {
            if datum.is_none() {
                p.setattr("_surface_tris", py.None())?;
            } else {
                paths.push(p);
            }
        }
    }
    if !paths.is_empty() {
        let sampler = surface_sampler(vp, &datum)?;
        let vec = py.import("built_core")?.getattr("Vec3")?;
        let build = py.import("built_core")?.getattr("build_surface")?;
        for p in paths {
            let mut lo = (f64::INFINITY, f64::INFINITY);
            let mut hi = (f64::NEG_INFINITY, f64::NEG_INFINITY);
            for pt in p.getattr("points")?.try_iter()? {
                let pt = pt?;
                let (x, y): (f64, f64) = (
                    pt.call_method0("x")?.extract()?,
                    pt.call_method0("y")?.extract()?,
                );
                lo = (lo.0.min(x), lo.1.min(y));
                hi = (hi.0.max(x), hi.1.max(y));
            }
            if !lo.0.is_finite() {
                continue;
            }
            let (a, b, _): (f64, f64, f64) = datum
                .call_method1("local_to_geodetic", (vec.call1((lo.0, lo.1, 0.))?,))?
                .extract()?;
            let (c, d, _): (f64, f64, f64) = datum
                .call_method1("local_to_geodetic", (vec.call1((hi.0, hi.1, 0.))?,))?
                .extract()?;
            sampler.call_method1("ensure_area", (a.min(c), b.min(d), a.max(c), b.max(d)))?;
            p.setattr("_surface_tris", build.call1((&p, &sampler, &datum))?)?;
        }
    }
    vp.call_method0("update")?;
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(unique_components, m)?)?;
    m.add_function(wrap_pyfunction!(merge_groups, m)?)?;
    m.add_function(wrap_pyfunction!(explode_curves, m)?)?;
    m.add_function(wrap_pyfunction!(look_at_face, m)?)?;
    m.add_function(wrap_pyfunction!(section_action, m)?)?;
    m.add_function(wrap_pyfunction!(context_lock, m)?)?;
    m.add_function(wrap_pyfunction!(unlock_group, m)?)?;
    m.add_function(wrap_pyfunction!(assign_tag, m)?)?;
    m.add_function(wrap_pyfunction!(next_tag, m)?)?;
    m.add_function(wrap_pyfunction!(toggle_paths, m)?)?;
    m.add_function(wrap_pyfunction!(set_path_surface, m)?)?;
    m.add_function(wrap_pyfunction!(rebuild_path_surfaces, m)?)?;
    Ok(())
}

#[cfg(test)]
mod look_tests {
    use super::look_angles;
    use glam::DVec3;
    use std::f64::consts::FRAC_PI_2;

    fn near(a: f64, b: f64) -> bool { (a - b).abs() < 1e-9 }

    #[test]
    fn a_wall_is_seen_from_its_front_with_the_world_up() {
        // A wall facing -Y (the Front view's direction).
        let (yaw, pitch) = look_angles(DVec3::NEG_Y, &[], 0.3).unwrap();
        assert!(near(yaw, -FRAC_PI_2) && near(pitch, 0.));
        // A roof sloping up to +X: seen square, from above.
        let n = DVec3::new(-1., 0., 1.);
        let (yaw, pitch) = look_angles(n, &[], 0.).unwrap();
        assert!(near(yaw, std::f64::consts::PI) && near(pitch, FRAC_PI_2 / 2.));
    }

    #[test]
    fn a_floor_turns_the_least_to_lay_its_longest_edge_across() {
        // 4 x 1 along X: from the top, X across is yaw -90° (the Top view).
        let floor = [DVec3::ZERO, DVec3::new(4., 0., 0.), DVec3::new(4., 1., 0.), DVec3::new(0., 1., 0.)];
        let (yaw, pitch) = look_angles(DVec3::Z, &floor, -1.4).unwrap();
        assert!(near(yaw, -FRAC_PI_2) && near(pitch, FRAC_PI_2));
        // From a yaw near +90° the other way round is the shorter turn.
        let (yaw, _) = look_angles(DVec3::Z, &floor, 1.4).unwrap();
        assert!(near(yaw, FRAC_PI_2));
        // Longest along Y: Y across, so yaw 0 or 180°.
        let tall = floor.map(|p| DVec3::new(p.y, p.x, 0.));
        let (yaw, _) = look_angles(DVec3::Z, &tall, 0.2).unwrap();
        assert!(near(yaw, 0.));
        // The eye stays on the face's front: a ceiling is seen from below.
        let (_, pitch) = look_angles(DVec3::NEG_Z, &floor, 0.).unwrap();
        assert!(near(pitch, -FRAC_PI_2));
        assert!(look_angles(DVec3::ZERO, &floor, 0.).is_none());
    }
}
