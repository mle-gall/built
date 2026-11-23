// SPDX-License-Identifier: GPL-3.0-or-later
//! Edits to the scene-folder tree. View handles keep their identity and the
//! ordered document list is updated in place so existing scene tabs see it.
use pyo3::{
    prelude::*,
    types::{PyDict, PySlice},
};
use std::collections::HashSet;
type Obj<'py> = Bound<'py, PyAny>;

fn folders(scene: &Obj<'_>) -> PyResult<Vec<String>> {
    api(scene)?.call_method1("folders_of", (scene,))?.extract()
}
fn api<'py>(scene: &Obj<'py>) -> PyResult<Bound<'py, PyModule>> {
    scene.py().import("core.scene_folders")
}
fn parents<'py>(scene: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let table = PyDict::new(scene.py());
    let original = scene.getattr("scene_folder_parents")?;
    if !original.is_none() {
        table.call_method1("update", (original,))?;
    }
    Ok(table)
}
fn views<'py>(scene: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    scene.getattr("saved_views")?.try_iter()?.collect()
}
fn set_views(scene: &Obj<'_>, ordered: Vec<Obj<'_>>) -> PyResult<()> {
    let all = PySlice::new(scene.py(), 0, isize::MAX, 1);
    scene.getattr("saved_views")?.set_item(all, ordered)
}
#[pyfunction(signature=(scene,base,parent=None))]
fn add_folder(scene: &Obj<'_>, base: &str, parent: Option<&str>) -> PyResult<String> {
    let mut names = folders(scene)?;
    let mut name = base.to_owned();
    let mut i = 2;
    while names.contains(&name) {
        name = format!("{base} {i}");
        i += 1;
    }
    names.push(name.clone());
    scene.setattr("scene_folders", &names)?;
    if let Some(p) = parent.filter(|p| !p.is_empty() && names.iter().any(|f| f == p)) {
        let table = parents(scene)?;
        table.set_item(&name, p)?;
        scene.setattr("scene_folder_parents", table)?;
    }
    Ok(name)
}
#[pyfunction]
fn rename_folder(scene: &Obj<'_>, old: &str, new: Option<&str>) -> PyResult<bool> {
    let new = new.unwrap_or("").trim();
    let mut names = folders(scene)?;
    if new.is_empty() || new == old || names.iter().any(|n| n == new) {
        return Ok(false);
    }
    let Some(at) = names.iter().position(|n| n == old) else {
        return Ok(false);
    };
    names[at] = new.to_owned();
    scene.setattr("scene_folders", names)?;
    let table = PyDict::new(scene.py());
    let new = new.into_pyobject(scene.py())?.into_any();
    for (k, v) in parents(scene)? {
        table.set_item(
            if k.eq(old)? { &new } else { &k },
            if v.eq(old)? { &new } else { &v },
        )?;
    }
    scene.setattr("scene_folder_parents", table)?;
    for view in views(scene)? {
        if view.getattr("folder")?.eq(old)? {
            view.setattr("folder", &new)?;
        }
    }
    Ok(true)
}
#[pyfunction]
fn delete_folder(scene: &Obj<'_>, name: &str) -> PyResult<bool> {
    let mut names = folders(scene)?;
    if !names.iter().any(|f| f == name) {
        return Ok(false);
    }
    let api = api(scene)?;
    let up = api.call_method1("folder_parent", (scene, name))?;
    let table = parents(scene)?;
    for f in &names {
        if table
            .get_item(f)?
            .is_some_and(|v| v.extract::<String>().is_ok_and(|p| p == name))
        {
            if up.is_none() {
                table.del_item(f)?;
            } else {
                table.set_item(f, &up)?;
            }
        }
    }
    if table.contains(name)? {
        table.del_item(name)?;
    }
    for v in views(scene)? {
        if v.getattr("folder")?.eq(name)? {
            v.setattr("folder", &up)?;
        }
    }
    names.retain(|f| f != name);
    scene.setattr("scene_folders", names)?;
    scene.setattr("scene_folder_parents", table)?;
    api.call_method1("sync_order", (scene,))?;
    Ok(true)
}
#[pyfunction]
fn move_folder(scene: &Obj<'_>, name: &str, parent: Option<&str>) -> PyResult<bool> {
    let names = folders(scene)?;
    if !names.iter().any(|f| f == name)
        || parent.is_some_and(|p| p == name || !names.iter().any(|f| f == p))
    {
        return Ok(false);
    }
    if let Some(p) = parent {
        if api(scene)?
            .call_method1("subfolders", (scene, name, true))?
            .contains(p)?
        {
            return Ok(false);
        }
    }
    let table = parents(scene)?;
    if let Some(p) = parent {
        table.set_item(name, p)?;
    } else if table.contains(name)? {
        table.del_item(name)?;
    }
    scene.setattr("scene_folders", names)?;
    scene.setattr("scene_folder_parents", table)?;
    Ok(true)
}
#[pyfunction(signature=(scene,views,folders,folder,before=None))]
fn move_rows(
    scene: &Obj<'_>,
    views: Vec<Obj<'_>>,
    folders: Vec<String>,
    folder: Option<&str>,
    before: Option<&Obj<'_>>,
) -> PyResult<()> {
    for f in folders {
        move_folder(scene, &f, folder)?;
    }
    let ids: HashSet<_> = views.iter().map(|v| v.as_ptr()).collect();
    let (moving, mut rest): (Vec<_>, Vec<_>) = self::views(scene)?
        .into_iter()
        .partition(|v| ids.contains(&v.as_ptr()));
    for v in &moving {
        v.setattr("folder", folder)?;
    }
    let api = api(scene)?;
    if !moving.is_empty() {
        let at = if let Some(i) = before.and_then(|v| rest.iter().position(|r| r.is(v))) {
            i
        } else {
            let mut at = rest.len();
            for (i, v) in rest.iter().enumerate() {
                if api.call_method1("folder_of", (scene, v))?.eq(folder)? {
                    at = i + 1;
                }
            }
            at
        };
        rest.splice(at..at, moving);
        set_views(scene, rest)?;
    }
    api.call_method1("sync_order", (scene,))?;
    Ok(())
}

enum Operation {
    Add(String, Option<String>),
    Rename(String, String),
    Move(
        Vec<Py<PyAny>>,
        Vec<String>,
        Option<String>,
        Option<Py<PyAny>>,
    ),
    Delete(Vec<Py<PyAny>>, Vec<String>),
}
#[pyclass(module = "ingetrazo_core")]
struct FolderEdit {
    operation: Operation,
    #[pyo3(get)]
    name: Option<String>,
}
#[pymethods]
impl FolderEdit {
    #[staticmethod]
    fn add(base: String, parent: Option<String>) -> Self {
        Self {
            operation: Operation::Add(base, parent),
            name: None,
        }
    }
    #[staticmethod]
    fn rename(old: String, new: String) -> Self {
        Self {
            operation: Operation::Rename(old, new),
            name: None,
        }
    }
    #[staticmethod]
    fn move_rows(
        views: Vec<Py<PyAny>>,
        folders: Vec<String>,
        folder: Option<String>,
        before: Option<Py<PyAny>>,
    ) -> Self {
        Self {
            operation: Operation::Move(views, folders, folder, before),
            name: None,
        }
    }
    #[staticmethod]
    fn delete_rows(views: Vec<Py<PyAny>>, folders: Vec<String>) -> Self {
        Self {
            operation: Operation::Delete(views, folders),
            name: None,
        }
    }
    fn __call__(&mut self, scene: &Obj<'_>) -> PyResult<bool> {
        let api = api(scene)?;
        let before = api.call_method1("signature", (scene,))?;
        let py = scene.py();
        match &self.operation {
            Operation::Add(base, parent) => {
                self.name = Some(add_folder(scene, base, parent.as_deref())?)
            }
            Operation::Rename(old, new) => {
                rename_folder(scene, old, Some(new))?;
            }
            Operation::Move(v, f, p, b) => move_rows(
                scene,
                v.iter().map(|v| v.bind(py).clone()).collect(),
                f.clone(),
                p.as_deref(),
                b.as_ref().map(|b| b.bind(py)),
            )?,
            Operation::Delete(v, folders) => {
                for f in folders {
                    delete_folder(scene, f)?;
                }
                let ids: HashSet<_> = v.iter().map(|v| v.as_ptr()).collect();
                set_views(
                    scene,
                    views(scene)?
                        .into_iter()
                        .filter(|v| !ids.contains(&v.as_ptr()))
                        .collect(),
                )?;
            }
        }
        before.ne(api.call_method1("signature", (scene,))?)
    }
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "scene_tree")?;
    sub.add_class::<FolderEdit>()?;
    sub.add_function(wrap_pyfunction!(add_folder, &sub)?)?;
    sub.add_function(wrap_pyfunction!(rename_folder, &sub)?)?;
    sub.add_function(wrap_pyfunction!(delete_folder, &sub)?)?;
    sub.add_function(wrap_pyfunction!(move_folder, &sub)?)?;
    sub.add_function(wrap_pyfunction!(move_rows, &sub)?)?;
    m.add_submodule(&sub)
}
