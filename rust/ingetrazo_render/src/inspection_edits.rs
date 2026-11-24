//! Edits submitted by the model inspector and operation sheets.
use super::model_edit::{matching, translated};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::HashSet;

fn settings(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("core.settings")?.call_method0("settings")
}

fn commit(view: &Bound<'_, PyAny>, command: &Bound<'_, PyAny>) -> PyResult<()> {
    view.getattr("history")?
        .call_method1("execute", (command,))?;
    view.call_method0("notify_scene_changed")?;
    Ok(())
}

#[pyfunction]
#[pyo3(signature=(win, keep, definition=None))]
fn retain_components(
    win: &Bound<'_, PyAny>,
    keep: bool,
    definition: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    let py = win.py();
    let view = win.getattr("viewport")?;
    let history = py.import("core.history")?;
    let cmd = history.getattr("KeepComponentCommand")?;
    let vars = PyDict::new(py);
    let (command, message) = if let Some(group) = definition {
        vars.set_item("name", group.getattr("name")?)?;
        (
            cmd.call1((group, keep))?,
            if keep {
                "{name} is kept in the file, even with no copy placed"
            } else {
                "{name} is no longer kept in the file"
            },
        )
    } else {
        let commands = PyList::empty(py);
        let selection = view.getattr("scene")?.getattr("selection")?;
        let mut meshes = HashSet::new();
        for group in matching(&selection, "core.group", "Group")?.iter() {
            if group.call_method0("is_component")?.is_truthy()?
                && meshes.insert(group.getattr("mesh")?.as_ptr())
            {
                commands.append(cmd.call1((group, keep))?)?;
            }
        }
        vars.set_item("n", commands.len())?;
        let command = match commands.len() {
            0 => return Ok(()),
            1 => commands.get_item(0)?,
            _ => history.call_method1("CompoundCommand", (commands,))?,
        };
        (
            command,
            if keep {
                "{n} components kept in the file"
            } else {
                "{n} components no longer kept in the file"
            },
        )
    };
    commit(&view, &command)?;
    win.call_method0("statusBar")?
        .call_method1("showMessage", (translated(py, message, Some(&vars))?, 4000))?;
    Ok(())
}

#[pyfunction]
#[pyo3(signature=(win, scene, definition, name=None))]
fn name_definition(
    win: &Bound<'_, PyAny>,
    scene: &Bound<'_, PyAny>,
    definition: &Bound<'_, PyAny>,
    name: Option<String>,
) -> PyResult<()> {
    let name = name.as_deref().unwrap_or_default().trim();
    let view = win.getattr("viewport")?;
    if name.is_empty()
        || !view.getattr("scene")?.is(scene)
        || definition.getattr("name")?.eq(name)?
    {
        return Ok(());
    }
    let py = win.py();
    let kept = py.import("core.kept_components")?;
    let mesh = definition.getattr("mesh")?;
    let targets = kept
        .call_method1("copies_of", (scene, &mesh))?
        .cast_into::<PyList>()?;
    let template = kept.call_method1("kept_template", (scene, mesh))?;
    // A component may have lost its last instance while the question was open.
    if targets.is_empty() && template.is_none() {
        return Ok(());
    }
    if !template.is_none() && !targets.contains(&template)? {
        targets.append(template)?;
    }
    if !targets.contains(definition)? {
        targets.append(definition)?;
    }
    let history = py.import("core.history")?;
    let commands = PyList::empty(py);
    for group in targets.iter() {
        commands.append(history.call_method1("RenameGroupCommand", (group, name))?)?;
    }
    commit(
        &view,
        &history.call_method1("CompoundCommand", (commands,))?,
    )
}

#[pyfunction]
fn name_section(
    win: &Bound<'_, PyAny>,
    scene: &Bound<'_, PyAny>,
    plane: &Bound<'_, PyAny>,
    values: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = win.py();
    let view = win.getattr("viewport")?;
    if !view.getattr("scene")?.is(scene) || !scene.getattr("section_planes")?.contains(plane)? {
        return Ok(());
    }
    let field = |key| -> PyResult<_> {
        let input = values.get_item(key)?;
        if input.is_truthy()? {
            Ok(input)
        } else {
            plane.getattr(key)
        }
    };
    let name = field("name")?;
    let symbol = field("symbol")?;
    if !plane.getattr("name")?.eq(&name)? || !plane.getattr("symbol")?.eq(&symbol)? {
        commit(
            &view,
            &py.import("core.history")?
                .call_method1("RenameSectionPlaneCommand", (plane, name, symbol))?,
        )?;
    }
    if values.get_item("skip")?.is_truthy()? {
        settings(py)?.call_method1("setValue", ("section/ask_name", "0"))?;
    }
    Ok(())
}

#[pyfunction]
fn soften_selection(view: &Bound<'_, PyAny>, values: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = view.py();
    let angle: Option<f64> = values.get_item("angle")?.extract()?;
    if let Some(angle) = angle {
        settings(py)?.call_method1("setValue", ("tools/soften_angle", angle))?;
    }
    let backend = py.import("core.soften")?;
    let edges = backend.call_method1("selected_edges", (view.getattr("scene")?,))?;
    let kw = PyDict::new(py);
    for key in ["coplanar", "harden", "materials", "smooth"] {
        kw.set_item(key, values.get_item(key)?)?;
    }
    let cmd = backend
        .getattr("SoftenByAngleCommand")?
        .call((edges, angle.unwrap_or_default()), Some(&kw))?;
    let changes = cmd.call_method0("prepare")?;
    if !changes.is_truthy()? {
        view.call_method1(
            "flash_status",
            (translated(py, "No edges need changing", None)?, 4000),
        )?;
        return Ok(());
    }
    cmd.setattr("changes", &changes)?;
    let harden = values.get_item("harden")?.is_truthy()?;
    let mut n = changes.len()?;
    if !harden {
        n = 0;
        for change in changes.try_iter()? {
            if change?.get_item(3)?.get_item(0)?.is_truthy()? {
                n += 1;
            }
        }
    }
    let vars = PyDict::new(py);
    vars.set_item("n", n)?;
    commit(view, &cmd)?;
    view.call_method1(
        "flash_status",
        (
            translated(
                py,
                if harden {
                    "Unsoftened {n} edge(s)"
                } else {
                    "Smoothed {n} edge(s)"
                },
                Some(&vars),
            )?,
            3000,
        ),
    )?;
    view.call_method0("update")?;
    Ok(())
}

#[pyfunction]
fn lens_angle(win: &Bound<'_, PyAny>, view: &Bound<'_, PyAny>, degrees: f64) -> PyResult<bool> {
    let camera = view.getattr("camera")?;
    if !camera.getattr("perspective")?.is_truthy()? {
        win.call_method0("statusBar")?.call_method1(
            "showMessage",
            (
                translated(
                    win.py(),
                    "Field of view applies to perspective; switch to Perspective first.",
                    None,
                )?,
                5000,
            ),
        )?;
        return Ok(false);
    }
    let (lo, hi): (f64, f64) = win
        .py()
        .import("core.camera")?
        .getattr("FOV_RANGE")?
        .extract()?;
    camera.setattr("fov_deg", degrees.clamp(lo, hi))?;
    view.call_method0("update")?;
    Ok(true)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(retain_components, m)?)?;
    m.add_function(wrap_pyfunction!(name_definition, m)?)?;
    m.add_function(wrap_pyfunction!(name_section, m)?)?;
    m.add_function(wrap_pyfunction!(soften_selection, m)?)?;
    m.add_function(wrap_pyfunction!(lens_angle, m)?)?;
    Ok(())
}
