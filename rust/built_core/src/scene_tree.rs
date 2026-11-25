// SPDX-License-Identifier: GPL-3.0-or-later
//! Edits to the scene-folder tree. View handles keep their identity and the
//! ordered document list is updated in place so existing scene tabs see it.
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySlice, PyTuple},
};
use std::collections::{HashMap, HashSet};
type Obj<'py> = Bound<'py, PyAny>;

/// One snapshot of the folder hierarchy. A broken link or cycle puts that
/// folder at the root, preserving the order in the explicit document list.
struct Tree {
    names: Vec<String>,
    known: HashSet<String>,
    parents: HashMap<String, String>,
}
impl Tree {
    fn read(scene: &Obj<'_>) -> PyResult<Self> {
        let mut names = Vec::new();
        let mut known = HashSet::new();
        if let Some(explicit) = scene.getattr_opt("scene_folders")?.filter(|v| !v.is_none()) {
            for name in explicit.try_iter()? {
                if let Ok(name) = name?.extract::<String>() {
                    if !name.is_empty() && known.insert(name.clone()) {
                        names.push(name);
                    }
                }
            }
        }
        if let Some(saved) = scene.getattr_opt("saved_views")?.filter(|v| !v.is_none()) {
            for v in saved.try_iter()? {
                let folder = v?.getattr("folder")?;
                if folder.is_truthy()? {
                    let name: String = folder.extract()?;
                    if known.insert(name.clone()) {
                        names.push(name);
                    }
                }
            }
        }
        let table = parents(scene)?;
        let mut links = HashMap::new();
        for (key, value) in table {
            if let (Ok(key), Ok(parent)) = (key.extract::<String>(), value.extract::<String>()) {
                if known.contains(&parent) {
                    links.insert(key, parent);
                }
            }
        }
        Ok(Self {
            names,
            known,
            parents: links,
        })
    }

    fn parent(&self, name: &str) -> Option<&str> {
        let direct = self.parents.get(name)?;
        let mut current = direct.as_str();
        let mut seen = HashSet::from([name]);
        loop {
            if !seen.insert(current) {
                return None;
            }
            match self.parents.get(current) {
                Some(next) => current = next,
                None => return Some(direct),
            }
        }
    }

    fn children(&self, name: Option<&str>) -> Vec<String> {
        self.names
            .iter()
            .filter(|f| self.parent(f) == name)
            .cloned()
            .collect()
    }

    fn view_folder(&self, view: &Obj<'_>) -> PyResult<Option<String>> {
        Ok(view
            .getattr("folder")?
            .extract::<Option<String>>()?
            .filter(|f| self.known.contains(f)))
    }
}
#[pyfunction]
fn folders_of(scene: &Obj<'_>) -> PyResult<Vec<String>> {
    Ok(Tree::read(scene)?.names)
}

#[pyfunction]
fn folder_parent(scene: &Obj<'_>, name: &str) -> PyResult<Option<String>> {
    Ok(Tree::read(scene)?.parent(name).map(str::to_owned))
}

#[pyfunction]
fn folder_of(scene: &Obj<'_>, view: &Obj<'_>) -> PyResult<Option<String>> {
    Tree::read(scene)?.view_folder(view)
}

#[pyfunction(signature=(scene,name,deep=false))]
fn subfolders(scene: &Obj<'_>, name: Option<&str>, deep: bool) -> PyResult<Vec<String>> {
    let tree = Tree::read(scene)?;
    let mut found = tree.children(name);
    if deep {
        let mut todo: Vec<_> = found.iter().rev().cloned().collect();
        while let Some(parent) = todo.pop() {
            let children = tree.children(Some(&parent));
            todo.extend(children.iter().rev().cloned());
            found.extend(children);
        }
    }
    Ok(found)
}

#[pyfunction]
fn scenes_in<'py>(scene: &Obj<'py>, name: Option<&str>) -> PyResult<Vec<Obj<'py>>> {
    let tree = Tree::read(scene)?;
    let mut found = Vec::new();
    for view in views(scene)? {
        if tree.view_folder(&view)?.as_deref() == name {
            found.push(view);
        }
    }
    Ok(found)
}

#[pyfunction]
fn tree_order<'py>(scene: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let tree = Tree::read(scene)?;
    let mut buckets: HashMap<Option<String>, Vec<Obj>> = HashMap::new();
    for view in views(scene)? {
        buckets
            .entry(tree.view_folder(&view)?)
            .or_default()
            .push(view);
    }
    let mut children: HashMap<Option<&str>, Vec<&str>> = HashMap::new();
    for folder in &tree.names {
        children
            .entry(tree.parent(folder))
            .or_default()
            .push(folder);
    }
    let mut out = Vec::new();
    let mut todo = vec![(None, false)];
    while let Some((folder, visited)) = todo.pop() {
        if visited {
            if let Some(group) = buckets.remove(&folder.map(str::to_owned)) {
                out.extend(group);
            }
        } else {
            todo.push((folder, true));
            if let Some(direct) = children.get(&folder) {
                todo.extend(direct.iter().rev().map(|f| (Some(*f), false)));
            }
        }
    }
    Ok(out)
}

#[pyfunction]
fn sync_order(scene: &Obj<'_>) -> PyResult<()> {
    let order = tree_order(scene)?;
    if !order
        .iter()
        .map(|v| v.as_ptr())
        .eq(views(scene)?.iter().map(|v| v.as_ptr()))
    {
        set_views(scene, order)?;
    }
    Ok(())
}
fn parents<'py>(scene: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let table = PyDict::new(scene.py());
    let original = scene
        .getattr_opt("scene_folder_parents")?
        .unwrap_or_else(|| scene.py().None().into_bound(scene.py()));
    if !original.is_none() {
        table.call_method1("update", (original,))?;
    }
    Ok(table)
}
fn views<'py>(scene: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    scene.getattr("saved_views")?.try_iter()?.collect()
}
fn set_views(scene: &Obj<'_>, ordered: Vec<Obj<'_>>) -> PyResult<()> {
    let all = PySlice::full(scene.py());
    scene.getattr("saved_views")?.set_item(all, ordered)
}
#[pyfunction(signature=(scene,base,parent=None))]
fn add_folder(scene: &Obj<'_>, base: &str, parent: Option<&str>) -> PyResult<String> {
    let mut names = folders_of(scene)?;
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
    let stripped = new
        .unwrap_or("")
        .into_pyobject(scene.py())?
        .call_method0("strip")?;
    let new: String = stripped.extract()?;
    let new = new.as_str();
    let mut names = folders_of(scene)?;
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
    let mut names = folders_of(scene)?;
    if !names.iter().any(|f| f == name) {
        return Ok(false);
    }
    let up = folder_parent(scene, name)?;
    let table = parents(scene)?;
    for f in &names {
        if table
            .get_item(f)?
            .map(|v| v.eq(name))
            .transpose()?
            .unwrap_or(false)
        {
            if up.is_none() {
                table.del_item(f)?;
            } else {
                table.set_item(f, up.as_deref())?;
            }
        }
    }
    if table.contains(name)? {
        table.del_item(name)?;
    }
    for v in views(scene)? {
        if v.getattr("folder")?.eq(name)? {
            v.setattr("folder", up.as_deref())?;
        }
    }
    names.retain(|f| f != name);
    scene.setattr("scene_folders", names)?;
    scene.setattr("scene_folder_parents", table)?;
    sync_order(scene)?;
    Ok(true)
}
#[pyfunction]
fn move_folder(scene: &Obj<'_>, name: &str, parent: Option<&str>) -> PyResult<bool> {
    let names = folders_of(scene)?;
    if !names.iter().any(|f| f == name)
        || parent.is_some_and(|p| p == name || !names.iter().any(|f| f == p))
    {
        return Ok(false);
    }
    if let Some(p) = parent {
        if subfolders(scene, Some(name), true)?.iter().any(|f| f == p) {
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
#[pyfunction(name = "move_rows", signature = (scene, views, folders, folder, before=None))]
fn move_row_inputs<'p>(
    scene: &Obj<'p>,
    views: &Obj<'p>,
    folders: &Obj<'p>,
    folder: Option<&str>,
    before: Option<&Obj<'p>>,
) -> PyResult<()> {
    let views = views.try_iter()?.collect::<PyResult<Vec<_>>>()?;
    let folders = folders.try_iter()?.map(|row| row?.extract()).collect::<PyResult<Vec<_>>>()?;
    move_rows(scene, views, folders, folder, before)
}
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
    if !moving.is_empty() {
        let at = if let Some(i) = before.and_then(|v| rest.iter().position(|r| r.is(v))) {
            i
        } else {
            let tree = Tree::read(scene)?;
            let mut at = rest.len();
            for (i, v) in rest.iter().enumerate() {
                if tree.view_folder(v)?.as_deref() == folder {
                    at = i + 1;
                }
            }
            at
        };
        rest.splice(at..at, moving);
        set_views(scene, rest)?;
    }
    sync_order(scene)?;
    Ok(())
}

fn explicit_folders<'py>(scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    Ok(scene
        .getattr_opt("scene_folders")?
        .filter(|v| !v.is_none())
        .unwrap_or_else(|| PyList::empty(scene.py()).into_any()))
}

#[pyfunction]
pub(crate) fn state<'py>(scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = scene.py();
    let views = views(scene)?;
    let links = views
        .iter()
        .map(|v| Ok((v.clone(), v.getattr("folder")?)))
        .collect::<PyResult<Vec<_>>>()?;
    let saved = py.get_type::<PyList>().call1((explicit_folders(scene)?,))?;
    Ok((views, links, saved, parents(scene)?)
        .into_pyobject(py)?
        .into_any())
}

#[pyfunction]
fn signature<'py>(scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = scene.py();
    let links = views(scene)?
        .iter()
        .map(|v| Ok((v.as_ptr() as usize, v.getattr("folder")?)))
        .collect::<PyResult<Vec<_>>>()?;
    let names = py
        .get_type::<PyTuple>()
        .call1((explicit_folders(scene)?,))?;
    let pairs = parents(scene)?.items();
    pairs.call_method0("sort")?;
    Ok((
        PyTuple::new(py, links)?,
        names,
        py.get_type::<PyTuple>().call1((pairs,))?,
    )
        .into_pyobject(py)?
        .into_any())
}

#[pyfunction]
pub(crate) fn restore(scene: &Obj<'_>, saved: &Obj<'_>) -> PyResult<()> {
    let py = scene.py();
    scene
        .getattr("saved_views")?
        .set_item(PySlice::full(py), saved.get_item(0)?)?;
    for item in saved.get_item(1)?.try_iter()? {
        let pair = item?;
        pair.get_item(0)?.setattr("folder", pair.get_item(1)?)?;
    }
    scene.setattr(
        "scene_folders",
        py.get_type::<PyList>().call1((saved.get_item(2)?,))?,
    )?;
    scene.setattr(
        "scene_folder_parents",
        py.get_type::<PyDict>().call1((saved.get_item(3)?,))?,
    )
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
#[pyclass(module = "built_core")]
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
        let before = signature(scene)?;
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
        before.ne(signature(scene)?)
    }
}
type MenuItem = (String, String, String, String, i8, bool);

/// Menu rows carry the document indices used by scene recall. Folder keys
/// and parent names stay separate from display text (both may contain '/').
#[pyfunction]
#[pyo3(signature=(scene,marked=None))]
fn menu_rows(
    scene: &Obj<'_>,
    marked: Option<&Obj<'_>>,
) -> PyResult<(Vec<MenuItem>, Vec<MenuItem>)> {
    let tree = Tree::read(scene)?;
    let content: i64 = scene.getattr("content_version")?.extract()?;
    let catalog: i64 = scene.getattr("_scene_catalog_ticks")?.extract()?;
    let model = content - catalog;
    let scenes = views(scene)?
        .into_iter()
        .enumerate()
        .map(|(i, v)| {
            Ok((
                format!("scene:{i}"),
                v.getattr("name")?.extract()?,
                format!(
                    "{}:{model}:{}",
                    v.getattr("_doc_key")?.extract::<u64>()?,
                    v.getattr("_photo_tick")?.extract::<i64>()?
                ),
                tree.view_folder(&v)?.unwrap_or_default(),
                i8::from(marked.is_some_and(|m| v.is(m))),
                true,
            ))
        })
        .collect::<PyResult<_>>()?;
    let owner = (scene.as_ptr() as usize).to_string();
    let mut folders = vec![(
        String::new(),
        "All scenes".into(),
        scene.getattr("content_version")?.str()?.to_string(),
        owner.clone(),
        -1,
        true,
    )];
    for folder in &tree.names {
        folders.push((
            folder.clone(),
            folder.clone(),
            tree.parent(folder).unwrap_or("").into(),
            owner.clone(),
            -1,
            true,
        ));
    }
    Ok((scenes, folders))
}

/// Move one position among the scenes filed in this folder. The adjacent
/// folder starts at the next global index when the scene is the last sibling.
#[pyfunction]
fn move_among_siblings(history: &Obj<'_>, view: &Obj<'_>, earlier: bool) -> PyResult<bool> {
    let scene = history.getattr("scene")?;
    let folder = folder_of(&scene, view)?;
    let peers = scenes_in(&scene, folder.as_deref())?;
    let Some(at) = peers.iter().position(|peer| peer.is(view)) else {
        return Ok(false);
    };
    let before = if earlier {
        let Some(previous) = at.checked_sub(1) else {
            return Ok(false);
        };
        peers.get(previous)
    } else {
        if at + 1 >= peers.len() {
            return Ok(false);
        }
        peers.get(at + 2)
    };
    let change = FolderEdit::move_rows(
        vec![view.clone().unbind()],
        vec![],
        folder,
        before.cloned().map(Obj::unbind),
    );
    let py = scene.py();
    let command = py
        .import("built_core.history")?
        .call_method1("SceneTreeCommand", (Py::new(py, change)?,))?;
    history.call_method1("execute", (command,))?;
    Ok(true)
}

fn selected_views<'py>(scene: &Obj<'py>, keys: &[u64]) -> PyResult<Vec<Obj<'py>>> {
    let selected: HashSet<_> = keys.iter().copied().collect();
    let mut found = Vec::new();
    for view in views(scene)? {
        if selected.contains(&view.getattr("_doc_key")?.extract::<u64>()?) {
            found.push(view);
        }
    }
    if found.is_empty() || found.len() != selected.len() {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "The scene selection has changed",
        ));
    }
    Ok(found)
}

#[pyfunction]
fn menu_selection<'py>(scene: &Obj<'py>, request: &str) -> PyResult<Vec<Obj<'py>>> {
    use pyo3::exceptions::PyValueError;
    let (owner, keys): (String, Vec<u64>) =
        serde_json::from_str(request).map_err(|err| PyValueError::new_err(err.to_string()))?;
    if owner != (scene.as_ptr() as usize).to_string() {
        return Err(PyValueError::new_err(
            "This scene menu belongs to another document",
        ));
    }
    selected_views(scene, &keys)
}

#[pyfunction]
#[pyo3(signature = (history, request, add_scene=None))]
fn edit_menu_folder(
    history: &Obj<'_>,
    request: &str,
    add_scene: Option<&Obj<'_>>,
) -> PyResult<String> {
    use pyo3::exceptions::PyValueError;
    let (verb, owner, folder, target, label): (String, String, String, String, String) =
        serde_json::from_str(request).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let scene = history.getattr("scene")?;
    if owner != (scene.as_ptr() as usize).to_string() {
        return Err(PyValueError::new_err(
            "This scene menu belongs to another document",
        ));
    }
    let tree = Tree::read(&scene)?;
    if [&folder, &target]
        .iter()
        .any(|f| !f.is_empty() && !tree.known.contains(*f))
    {
        return Err(PyValueError::new_err("This folder no longer exists"));
    }
    if verb == "capture" {
        let add = add_scene.ok_or_else(|| PyValueError::new_err("A live view is required"))?;
        add.call1((&folder,))?;
        return Ok(folder);
    }
    if verb == "rename" && label.trim() != folder && tree.known.contains(label.trim()) {
        return Err(PyValueError::new_err(format!(
            "Another folder is already called «{}»",
            label.trim()
        )));
    }
    let parent = (!target.is_empty()).then_some(target.clone());
    let name = label.trim();
    let op = match verb.as_str() {
        "add" if !name.is_empty() => FolderEdit::add(name.into(), parent),
        "rename" if !folder.is_empty() && !name.is_empty() && !tree.known.contains(name) => {
            FolderEdit::rename(folder.clone(), name.into())
        }
        "remove" if !folder.is_empty() => FolderEdit::delete_rows(vec![], vec![folder.clone()]),
        "move"
            if !folder.is_empty()
                && folder != target
                && !subfolders(&scene, Some(&folder), true)?.contains(&target) =>
        {
            FolderEdit::move_rows(vec![], vec![folder.clone()], parent, None)
        }
        "file_selected" => {
            let keys: Vec<u64> =
                serde_json::from_str(&label).map_err(|e| PyValueError::new_err(e.to_string()))?;
            let views = selected_views(&scene, &keys)?
                .into_iter()
                .map(Obj::unbind)
                .collect();
            FolderEdit::move_rows(views, vec![], parent, None)
        }
        "file" => {
            let saved = views(&scene)?
                .into_iter()
                .find(|v| {
                    v.getattr("name")
                        .and_then(|n| n.eq(&label))
                        .unwrap_or(false)
                })
                .ok_or_else(|| PyValueError::new_err("This scene no longer exists"))?;
            FolderEdit::move_rows(vec![saved.unbind()], vec![], parent, None)
        }
        _ => return Err(PyValueError::new_err("Choose a valid name and destination")),
    };
    let py = scene.py();
    let edit = Py::new(py, op)?;
    let cmd = py
        .import("built_core.history")?
        .call_method1("SceneTreeCommand", (&edit,))?;
    let before: u64 = history.getattr("revision")?.extract()?;
    history.call_method1("execute", (cmd,))?;
    if history.getattr("revision")?.extract::<u64>()? == before {
        return Err(PyValueError::new_err("No change to the scene folders"));
    }
    Ok(match verb.as_str() {
        "add" => edit.borrow(py).name.clone().unwrap_or_default(),
        "rename" => name.into(),
        "remove" => tree.parent(&folder).unwrap_or("").into(),
        _ => folder,
    })
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let name = "built_core.scene_tree";
    let sub = PyModule::new(m.py(), name)?;
    m.py().import("sys")?.getattr("modules")?.set_item(name, &sub)?;
    sub.add_function(wrap_pyfunction!(folders_of, &sub)?)?;
    sub.add_function(wrap_pyfunction!(folder_parent, &sub)?)?;
    sub.add_function(wrap_pyfunction!(folder_of, &sub)?)?;
    sub.add_function(wrap_pyfunction!(move_among_siblings, &sub)?)?;
    sub.add_function(wrap_pyfunction!(subfolders, &sub)?)?;
    sub.add_function(wrap_pyfunction!(scenes_in, &sub)?)?;
    sub.add_function(wrap_pyfunction!(tree_order, &sub)?)?;
    sub.add_function(wrap_pyfunction!(sync_order, &sub)?)?;
    sub.add_function(wrap_pyfunction!(state, &sub)?)?;
    sub.add_function(wrap_pyfunction!(signature, &sub)?)?;
    sub.add_function(wrap_pyfunction!(restore, &sub)?)?;
    sub.add_function(wrap_pyfunction!(menu_selection, &sub)?)?;
    sub.add_function(wrap_pyfunction!(menu_rows, &sub)?)?;
    sub.add_function(wrap_pyfunction!(edit_menu_folder, &sub)?)?;
    sub.add_class::<FolderEdit>()?;
    sub.add_function(wrap_pyfunction!(add_folder, &sub)?)?;
    sub.add_function(wrap_pyfunction!(rename_folder, &sub)?)?;
    sub.add_function(wrap_pyfunction!(delete_folder, &sub)?)?;
    sub.add_function(wrap_pyfunction!(move_folder, &sub)?)?;
    sub.add_function(wrap_pyfunction!(move_row_inputs, &sub)?)?;
    m.add("scene_tree", sub)
}
