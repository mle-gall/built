// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::types::PySet;

#[pyfunction]
pub(super) fn resolve_placements<'py>(
    history: &Obj<'py>,
    uids: &Obj<'py>,
) -> PyResult<Bound<'py, PyList>> {
    let py = history.py();
    let scene = history.getattr("scene")?;
    let want = PyDict::new(py);
    let found = PyDict::new(py);
    let placement = py.import("built_core")?.getattr("Placement")?;
    for item in uids.try_iter()? {
        let item = item?;
        let key = if item.is_instance_of::<PyList>() {
            py.get_type::<PyTuple>().call1((item,))?
        } else {
            item
        };
        if want.contains(&key)? {
            continue;
        }
        want.set_item(&key, py.None())?;
        if key.is_instance(&placement)? {
            found.set_item(&key, &key)?;
        } else if key.is_instance_of::<PyTuple>() {
            let group = scene.call_method1("resolve_occurrence", (&key,))?;
            if group.is_none() {
                return Err(PyKeyError::new_err(format!(
                    "no placement at occurrence path {}",
                    key.repr()?
                )));
            }
            found.set_item(key, group)?;
        }
    }
    if want.len() > found.len() {
        let mut stack: Vec<_> = scene
            .getattr("groups")?
            .try_iter()?
            .collect::<PyResult<_>>()?;
        stack.reverse();
        while let Some(group) = stack.pop() {
            let uid = group
                .getattr_opt("uid")?
                .unwrap_or_else(|| py.None().into_bound(py));
            if want.contains(&uid)? && !found.contains(&uid)? {
                found.set_item(uid, &group)?;
            }
            if found.len() == want.len() {
                break;
            }
            if let Some(children) = group.getattr_opt("children")?.filter(|x| !x.is_none()) {
                let children = children.try_iter()?.collect::<PyResult<Vec<_>>>()?;
                stack.extend(children.into_iter().rev());
            }
        }
    }
    let ordered = PyList::empty(py);
    let emitted = PySet::empty(py)?;
    for key in want.keys().iter() {
        let Some(group) = found.get_item(&key)? else {
            return Err(PyKeyError::new_err(format!(
                "no placement with uid {} in the open context",
                key.repr()?
            )));
        };
        if !emitted.contains(&group)? {
            emitted.add(&group)?;
            ordered.append(group)?;
        }
    }
    Ok(ordered)
}

pub(super) fn one<'py>(h: &Obj<'py>, uid: &Obj<'py>) -> PyResult<Obj<'py>> {
    resolve_placements(h, PyList::new(h.py(), [uid])?.as_any())?.get_item(0)
}

fn visibility<'py>(history: &Obj<'py>, placements: &Obj<'py>, hidden: bool) -> Reply<'py> {
    let groups = resolve_placements(history, placements)?;
    execute(
        history,
        &history_command(history.py(), "HideCommand", (groups, hidden))?,
    )
}

#[pyfunction]
fn hide<'py>(history: &Obj<'py>, placements: &Obj<'py>) -> Reply<'py> {
    visibility(history, placements, true)
}
#[pyfunction]
fn unhide<'py>(history: &Obj<'py>, placements: &Obj<'py>) -> Reply<'py> {
    visibility(history, placements, false)
}

#[pyfunction]
#[pyo3(signature=(history, placements, locked=true))]
fn lock<'py>(
    history: &Obj<'py>,
    placements: &Obj<'py>,
    #[pyo3(from_py_with = PyAnyMethods::is_truthy)] locked: bool,
) -> Reply<'py> {
    let groups = resolve_placements(history, placements)?;
    let class = history
        .py()
        .import("built_core")?
        .getattr("NativeCommand")?;
    execute(history, &class.call_method1("lock", (groups, locked))?)
}

#[pyfunction]
#[pyo3(signature=(history, placement, rename=false))]
fn make_unique<'py>(
    history: &Obj<'py>,
    placement: &Obj<'py>,
    #[pyo3(from_py_with = PyAnyMethods::is_truthy)] rename: bool,
) -> Reply<'py> {
    let group = one(history, placement)?;
    execute(
        history,
        &history_command(history.py(), "MakeUniqueCommand", (group, rename))?,
    )
}

#[pyfunction]
fn rename_placement<'py>(history: &Obj<'py>, placement: &Obj<'py>, name: &Obj<'py>) -> Reply<'py> {
    let group = one(history, placement)?;
    let class = history
        .py()
        .import("built_core")?
        .getattr("NativeCommand")?;
    execute(
        history,
        &class.call_method1("rename", (group, name.str()?))?,
    )
}

#[pyfunction]
fn rename_placements<'py>(
    history: &Obj<'py>,
    placements: &Obj<'py>,
    names: &Obj<'py>,
) -> Reply<'py> {
    let py = history.py();
    let names = names
        .try_iter()?
        .map(|n| n?.str())
        .collect::<PyResult<Vec<_>>>()?;
    let groups = resolve_placements(history, placements)?;
    if groups.len() != names.len() {
        return Err(PyValueError::new_err(
            "rename_placements needs one name per distinct placement",
        ));
    }
    let changed = PyList::empty(py);
    let labels = PyList::empty(py);
    for (group, name) in groups.iter().zip(names) {
        if group.getattr("name")?.ne(&name)? {
            changed.append(group)?;
            labels.append(name)?;
        }
    }
    let before = history.getattr("revision")?;
    if !changed.is_empty() {
        let class = py.import("built_core")?.getattr("NativeCommand")?;
        history.call_method1(
            "execute",
            (class.call_method1("rename_each", (changed, labels))?,),
        )?;
    }
    answer(history, &before, true)
}

#[pyfunction]
fn set_tag<'py>(history: &Obj<'py>, placements: &Obj<'py>, tag: &Obj<'py>) -> Reply<'py> {
    let groups = resolve_placements(history, placements)?;
    let layer = history.getattr("scene")?.call_method1("layer", (tag,))?;
    if layer.is_none() {
        return Err(PyKeyError::new_err(format!(
            "no tag with key {}",
            tag.repr()?
        )));
    }
    execute(
        history,
        &history_command(
            history.py(),
            "AssignLayerCommand",
            (groups, layer.getattr("key")?),
        )?,
    )
}

pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(resolve_placements, m)?)?;
    m.add_function(wrap_pyfunction!(hide, m)?)?;
    m.add_function(wrap_pyfunction!(unhide, m)?)?;
    m.add_function(wrap_pyfunction!(lock, m)?)?;
    m.add_function(wrap_pyfunction!(make_unique, m)?)?;
    m.add_function(wrap_pyfunction!(rename_placement, m)?)?;
    m.add_function(wrap_pyfunction!(rename_placements, m)?)?;
    m.add_function(wrap_pyfunction!(set_tag, m)?)?;
    Ok(())
}
