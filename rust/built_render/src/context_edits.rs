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
    point: &Bound<'_, PyAny>,
    normal: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let (x, y, z): (f64, f64, f64) = (
        normal.call_method0("x")?.extract()?,
        normal.call_method0("y")?.extract()?,
        normal.call_method0("z")?.extract()?,
    );
    let cam = vp.getattr("camera")?;
    let before: (f64, f64) = (cam.getattr("yaw")?.extract()?, cam.getattr("pitch")?.extract()?);
    cam.setattr("target", point)?;
    cam.setattr("pitch", z.clamp(-1., 1.).asin())?;
    if x.abs() > 1e-9 || y.abs() > 1e-9 {
        cam.setattr("yaw", y.atan2(x))?;
    }
    // Looking square at a face is an aligned view: auto perspective (M03).
    let auto = crate::native_view_actions::auto_perspective(vp.py())?;
    cam.call_method1("align_current", (before.0, before.1, auto))?;
    Ok(())
}

#[pyfunction]
fn align_face(vp: &Bound<'_, PyAny>, face: &Bound<'_, PyAny>) -> PyResult<()> {
    face_camera(
        vp,
        &face.call_method0("centroid")?,
        &face.call_method0("normal")?,
    )?;
    if vp
        .getattr("camera")?
        .call_method0("projection_mode")?
        .eq("two_point")?
    {
        vp.call_method1("set_projection", ("perspective",))?;
    } else {
        vp.call_method0("update")?;
    }
    Ok(())
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
            face_camera(&vp, &target, &plane.getattr("normal")?)?;
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
    m.add_function(wrap_pyfunction!(align_face, m)?)?;
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
