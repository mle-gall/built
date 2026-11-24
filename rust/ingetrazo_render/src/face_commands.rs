//! Window verbs over the active document's geometry and undo history.
use super::model_edit::{matching, translated};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

fn status(win: &Bound<'_, PyAny>, message: &str, count: Option<usize>) -> PyResult<()> {
    let py = win.py();
    let values = PyDict::new(py);
    if let Some(count) = count {
        values.set_item("n", count)?;
    }
    win.call_method0("statusBar")?.call_method1(
        "showMessage",
        (translated(py, message, Some(&values))?, 3000),
    )?;
    Ok(())
}

fn apply(vp: &Bound<'_, PyAny>, command: &Bound<'_, PyAny>) -> PyResult<()> {
    vp.getattr("history")?.call_method1("execute", (command,))?;
    vp.call_method0("update")?;
    Ok(())
}

#[pyfunction]
fn face_winding(win: &Bound<'_, PyAny>, orient: bool) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let faces = matching(
        &vp.getattr("scene")?.getattr("selection")?,
        "core.mesh",
        "Face",
    )?;
    let targets = if orient {
        if faces.len() != 1 {
            return status(win, "Select the one face the others should match.", None);
        }
        py.import("core.orient_faces")?
            .call_method1("faces_to_flip", (faces.get_item(0)?,))?
    } else {
        if faces.is_empty() {
            return status(win, "Select one or more faces first.", None);
        }
        faces.into_any()
    };
    let count = targets.len()?;
    if count == 0 {
        return status(win, "The connected faces already match.", None);
    }
    let command = py
        .import("core.history")?
        .call_method1("FlipFacesCommand", (targets,))?;
    apply(&vp, &command)?;
    status(
        win,
        if orient {
            "Oriented {n} face(s) like the selected one."
        } else {
            "Reversed {n} face(s)."
        },
        Some(count),
    )
}

#[pyfunction]
fn heal_overlaps(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let command = win
        .py()
        .import("core.history")?
        .call_method0("HealOverlapsCommand")?;
    apply(&win.getattr("viewport")?, &command)?;
    let count = command.getattr("healed")?.extract::<usize>()?;
    if count == 0 {
        status(win, "No overlapping faces found.", None)
    } else {
        status(win, "Healed {n} overlapping face(s).", Some(count))
    }
}

#[pyfunction]
fn delete_guides(vp: &Bound<'_, PyAny>) -> PyResult<()> {
    let scene = vp.getattr("scene")?;
    let guides = vp
        .py()
        .import("builtins")?
        .call_method1("list", (scene.getattr("guides")?,))?;
    if guides.is_truthy()? {
        apply(
            vp,
            &vp.py()
                .import("core.history")?
                .call_method1("DeleteGuidesCommand", (guides,))?,
        )?;
    }
    Ok(())
}

#[pyfunction]
fn intersect_faces(win: &Bound<'_, PyAny>, mode: &str) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    if !scene.getattr("selection")?.is_truthy()? {
        vp.call_method1(
            "flash_status",
            (translated(py, "Select faces or groups first", None)?,),
        )?;
        return Ok(());
    }
    let (count, command): (usize, Bound<'_, PyAny>) = py
        .import("core.intersect")?
        .call_method1("build_intersect_faces", (scene, mode))?
        .extract()?;
    if command.is_none() {
        vp.call_method1(
            "flash_status",
            (translated(py, "No faces cross the selection", None)?,),
        )?;
        return Ok(());
    }
    let history = vp.getattr("history")?;
    history.call_method1("execute", (command,))?;
    let error = history.getattr("last_error")?;
    if error.is_truthy()? {
        py.import("views.chrome_dialogs")?.call_method1(
            "warning",
            (win, translated(py, "Intersect Faces failed", None)?, error),
        )?;
    } else {
        let values = PyDict::new(py);
        values.set_item("n", count)?;
        vp.call_method1(
            "flash_status",
            (
                translated(py, "{n} intersection edges added", Some(&values))?,
                3000,
            ),
        )?;
    }
    Ok(())
}

#[pyfunction]
fn convert_geopath(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let paths = matching(&scene.getattr("selection")?, "georef.geopath", "GeoPath")?;
    if paths.is_empty() {
        return status(win, "Select a path to convert to geometry.", None);
    }
    let cmds = PyList::empty(py);
    let geometry = py.import("core.edits")?.getattr("build_add_edges")?;
    let list = py.import("builtins")?.getattr("list")?;
    let opts = PyDict::new(py);
    opts.set_item("detect_faces", true)?;
    for path in paths.iter() {
        let segments = list.call1((path.call_method0("segments")?,))?;
        if segments.is_truthy()? {
            cmds.append(geometry.call((&scene, segments), Some(&opts))?)?;
        }
    }
    let history = py.import("core.history")?;
    let deletion = history.call_method1("DeleteGeoPathsCommand", (paths,))?;
    if cmds.is_empty() {
        return apply(&vp, &deletion);
    }
    cmds.append(deletion)?;
    apply(&vp, &history.call_method1("CompoundCommand", (cmds,))?)
}

#[pyfunction]
fn repeat_last_command(win: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let last = py
        .import("builtins")?
        .call_method1("getattr", (win, "_last_command", py.None()))?;
    if !last.is_none() {
        let label: String = last.get_item(0)?.extract()?;
        last.get_item(1)?.call0()?;
        let vars = PyDict::new(py);
        vars.set_item("name", translated(py, &label, None)?)?;
        vp.call_method1(
            "flash_status",
            (translated(py, "Repeat: {name}", Some(&vars))?, 2000),
        )?;
        vp.call_method0("update")?;
        return Ok(true);
    }
    vp.call_method1(
        "flash_status",
        (translated(py, "Nothing to repeat yet", None)?, 3000),
    )?;
    Ok(false)
}

fn coordinates(point: &Bound<'_, PyAny>) -> PyResult<glam::DVec3> {
    Ok(glam::DVec3::from_array(
        point.call_method0("toTuple")?.extract()?,
    ))
}

fn common_plane(faces: &Bound<'_, PyList>) -> PyResult<Option<(glam::DVec3, glam::DVec3)>> {
    let first = faces.get_item(0)?;
    let normal = coordinates(&first.call_method0("normal")?)?;
    if normal.length() < 1e-9 {
        return Ok(None);
    }
    let normal = normal.normalize();
    let origin = coordinates(&first.getattr("loop")?.get_item(0)?.getattr("position")?)?;
    for face in faces.iter() {
        let outer = face.getattr("loop")?;
        let holes = face.getattr("hole_loops")?;
        for ring in std::iter::once(Ok(outer)).chain(holes.try_iter()?) {
            for vertex in ring?.try_iter()? {
                let point = coordinates(&vertex?.getattr("position")?)?;
                if (point - origin).dot(normal).abs() > 1e-4 {
                    return Ok(None);
                }
            }
        }
    }
    Ok(Some((origin, normal)))
}

#[pyfunction]
fn rebuild_faces(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let picked = matching(
        &vp.getattr("scene")?.getattr("selection")?,
        "core.mesh",
        "Face",
    )?;
    let history = py.import("core.history")?;
    let (cmd, message) = if picked.is_empty() {
        (
            history.call_method0("RebuildPlanarFacesCommand")?,
            "Rebuilt {n} face(s) from the edge graph.",
        )
    } else {
        let Some((origin, normal)) = common_plane(&picked)? else {
            return status(win, "The selected faces must lie on one plane.", None);
        };
        let vec = py.import("ingetrazo_core")?.getattr("Vec3")?;
        let origin = vec.call1((origin.to_array(),))?;
        let normal = vec.call1((normal.to_array(),))?;
        (
            history.call_method1("RebuildPlaneFacesCommand", (origin, normal))?,
            "Rebuilt {n} face(s) on the selected plane.",
        )
    };
    apply(&vp, &cmd)?;
    if picked.is_empty() && !cmd.getattr("flat")?.is_truthy()? {
        return status(
            win,
            "Rebuild Faces only works on a flat (single-plane) drawing.",
            None,
        );
    }
    status(win, message, Some(cmd.getattr("rebuilt")?.extract()?))
}

#[pyfunction]
fn ask_simplify_mesh(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    if !scene.getattr("mesh")?.getattr("faces")?.is_truthy()? {
        let message = if scene.getattr("groups")?.is_truthy()? {
            "Explode the imported group first, then simplify the mesh"
        } else {
            "There is no mesh to simplify"
        };
        vp.call_method1("flash_status", (translated(py, message, None)?,))?;
        return Ok(());
    }
    let kw = PyDict::new(py);
    let done = py.import("functools")?.call_method1(
        "partial",
        (wrap_pyfunction!(simplify_mesh, py)?, win, &scene),
    )?;
    kw.set_item("done", done)?;
    kw.set_item("suffix", "°")?;
    py.import("views.chrome_forms")?.getattr("ask_number")?.call((win,
        translated(py, "Simplify Mesh", None)?,
        translated(py, "Maximum facet angle in degrees (0 = coplanar only; higher values may change rounded geometry):", None)?,
        1.0, 0.0, 5.0, 2), Some(&kw))?;
    Ok(())
}

#[pyfunction]
#[pyo3(signature=(win, scene, angle=None))]
fn simplify_mesh(
    win: &Bound<'_, PyAny>,
    scene: &Bound<'_, PyAny>,
    angle: Option<f64>,
) -> PyResult<()> {
    let Some(angle) = angle else { return Ok(()) };
    let py = win.py();
    let vp = win.getattr("viewport")?;
    if !vp.getattr("scene")?.is(scene) {
        return Ok(());
    }
    let cmd = py
        .import("core.history")?
        .call_method1("SimplifyMeshCommand", (angle,))?;
    let history = vp.getattr("history")?;
    history.call_method1("execute", (&cmd,))?;
    let error = history.getattr("last_error")?;
    if error.is_truthy()? {
        py.import("views.chrome_dialogs")?.call_method1(
            "warning",
            (win, translated(py, "Simplify Mesh failed", None)?, error),
        )?;
        return Ok(());
    }
    let count: usize = cmd.getattr("faces_removed")?.extract()?;
    if count > 0 {
        let values = PyDict::new(py);
        values.set_item("count", count)?;
        vp.call_method1(
            "flash_status",
            (
                translated(py, "Simplified mesh: removed {count} faces", Some(&values))?,
                5000,
            ),
        )?;
    } else {
        vp.call_method1(
            "flash_status",
            (translated(py, "No faces could be simplified", None)?,),
        )?;
    }
    vp.call_method0("update")?;
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(ask_simplify_mesh, m)?)?;
    m.add_function(wrap_pyfunction!(rebuild_faces, m)?)?;
    m.add_function(wrap_pyfunction!(face_winding, m)?)?;
    m.add_function(wrap_pyfunction!(heal_overlaps, m)?)?;
    m.add_function(wrap_pyfunction!(delete_guides, m)?)?;
    m.add_function(wrap_pyfunction!(intersect_faces, m)?)?;
    m.add_function(wrap_pyfunction!(convert_geopath, m)?)?;
    m.add_function(wrap_pyfunction!(repeat_last_command, m)?)?;
    Ok(())
}
