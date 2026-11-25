// SPDX-License-Identifier: GPL-3.0-or-later
//! Scene menu and host commands. Resolve the live target before constructing a
//! history step: an open prompt can outlive its scene or even its document.
use pyo3::{exceptions::PyValueError, prelude::*, types::PyList};
use std::collections::HashSet;

type Obj<'py> = Bound<'py, PyAny>;

#[pyfunction]
#[pyo3(signature=(scene, verb, *, views=Vec::new(), name="", index=0, folder=None, camera=None, style=None))]
pub(crate) fn plan_scene_command<'py>(
    scene: &Obj<'py>,
    verb: &str,
    views: Vec<Obj<'py>>,
    name: &str,
    index: isize,
    folder: Option<&str>,
    camera: Option<&Obj<'py>>,
    style: Option<&Obj<'py>>,
) -> PyResult<Option<Obj<'py>>> {
    let py = scene.py();
    let saved: Vec<_> = scene
        .getattr("saved_views")?
        .try_iter()?
        .collect::<PyResult<_>>()?;
    let history = py.import("core.history")?;
    let recording = py.import("core.saved_views")?;
    let tree = py.import("ingetrazo_core.scene_tree")?;
    let mut targets = Vec::new();
    let mut seen = HashSet::new();
    for view in views {
        if !saved.iter().any(|v| v.is(&view)) {
            return Ok(None);
        }
        if seen.insert(view.as_ptr()) {
            targets.push(view);
        }
    }
    let camera = || camera.ok_or_else(|| PyValueError::new_err("A live camera is required"));
    if verb == "add" {
        let base = if name.is_empty() { "Scene" } else { name };
        let taken: HashSet<String> = saved
            .iter()
            .map(|v| v.getattr("name")?.extract())
            .collect::<PyResult<_>>()?;
        let mut n = 1;
        while taken.contains(&format!("{base} {n}")) {
            n += 1;
        }
        let view = recording
            .getattr("SavedView")?
            .call_method1("capture", (format!("{base} {n}"), scene, camera()?, style))?;
        view.setattr("folder", folder)?;
        return Ok(Some(history.call_method1("AddSceneCommand", (view,))?));
    }
    let Some(view) = targets.first() else {
        return Ok(None);
    };
    let command = match verb {
        "duplicate" => {
            let suffix: String = py
                .import("core.i18n")?
                .call_method1("tr", (" (copy)",))?
                .extract()?;
            let base = format!("{}{suffix}", view.getattr("name")?.extract::<String>()?);
            let taken: HashSet<String> = saved
                .iter()
                .map(|v| v.getattr("name")?.extract())
                .collect::<PyResult<_>>()?;
            let mut name = base.clone();
            let mut n = 2;
            while taken.contains(&name) {
                name = format!("{base} {n}");
                n += 1;
            }
            let copy = recording
                .getattr("SavedView")?
                .call_method1("from_dict", (view.call_method0("to_dict")?,))?;
            copy.setattr("name", name)?;
            let next = saved.iter().position(|v| v.is(view)).unwrap() + 1;
            history.call_method1("AddSceneCommand", (copy, next))?
        }
        "delete" => history.call_method1("DeleteScenesCommand", (targets,))?,
        "update" => history.call_method1("UpdateSceneCommand", (view, camera()?, style))?,
        "rename" => {
            let name = name.trim();
            if name.is_empty() || view.getattr("name")?.extract::<String>()? == name {
                return Ok(None);
            }
            recording.call_method1("RenameSceneCommand", (view, name))?
        }
        "move" => {
            let rest: Vec<_> = saved.iter().filter(|v| !v.is(view)).collect();
            let i = index.clamp(0, rest.len() as isize) as usize;
            let before = rest.get(i).copied();
            let neighbour = before.or_else(|| rest.last().copied());
            let folder = match neighbour {
                Some(v) => tree.call_method1("folder_of", (scene, v))?,
                None => py.None().into_bound(py),
            };
            let op = tree
                .getattr("FolderEdit")?
                .call_method1("move_rows", (vec![view], PyList::empty(py), folder, before))?;
            history.call_method1("SceneTreeCommand", (op,))?
        }
        _ => {
            return Err(PyValueError::new_err(format!(
                "Unknown scene command: {verb}"
            )))
        }
    };
    Ok(Some(command))
}
