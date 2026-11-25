// SPDX-License-Identifier: GPL-3.0-or-later
//! Restore import collection membership without invalidating scripting references.
use super::*;
use pyo3::types::{PyList, PySlice};
#[pyclass(subclass, module = "built_core")]
pub(super) struct ImportStep;

fn tracked(step: &Obj<'_>) -> PyResult<Vec<(String, String, String)>> {
    step.getattr("_TRACKED")?.extract()
}
fn apply_members(step: &Obj<'_>, scene: &Obj<'_>, undo: bool) -> PyResult<()> {
    for (name, added, removed) in tracked(step)? {
        let items = scene.getattr(&name)?;
        let added = step.getattr(added)?;
        let removed = step.getattr(removed)?;
        let remove = |x: &Obj<'_>| -> PyResult<()> {
            if items.contains(x)? {
                items.call_method1("remove", (x,))?;
            }
            if name == "groups" {
                scene.getattr("selection")?.call_method1("discard", (x,))?;
            }
            Ok(())
        };
        if undo {
            for x in added.try_iter()? {
                remove(&x?)?;
            }
            for pair in removed.try_iter()? {
                let p = pair?;
                let x = p.get_item(1)?;
                if !items.contains(&x)? {
                    let i = p.get_item(0)?.extract::<usize>()?.min(items.len()?);
                    items.call_method1("insert", (i, x))?;
                }
            }
        } else {
            for pair in removed.try_iter()? {
                remove(&pair?.get_item(1)?)?;
            }
            for x in added.try_iter()? {
                let x = x?;
                if !items.contains(&x)? {
                    items.call_method1("append", (x,))?;
                }
            }
        }
    }
    Ok(())
}

#[pymethods]
impl ImportStep {
    #[new]
    #[pyo3(signature=(*_args,**_kw))]
    fn new(_args: &Bound<'_, PyTuple>, _kw: Option<&Bound<'_, PyDict>>) -> Self {
        Self
    }
    #[getter]
    fn changed(slf: &Bound<'_, Self>) -> PyResult<bool> {
        if slf.getattr("before")?.ne(slf.getattr("after")?)? {
            return Ok(true);
        }
        for (_, a, r) in tracked(slf.as_any())? {
            if slf.getattr(a)?.is_truthy()? || slf.getattr(r)?.is_truthy()? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn r#do(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<()> {
        let py = scene.py();
        let s = slf.as_any();
        let mesh = scene.getattr("mesh")?;
        let after = s.getattr("after")?;
        if after.is_none() {
            let mut before = Vec::new();
            for (name, added, removed) in tracked(s)? {
                let list = py
                    .get_type::<PyList>()
                    .call1((scene.getattr(&name)?,))?
                    .cast_into::<PyList>()?;
                before.push((name, added, removed, list));
            }
            s.setattr("before", mesh.call_method0("capture_state")?)?;
            if let Err(err) = s.call_method1("mutate", (scene,)) {
                let all = PySlice::full(py);
                for (name, _, _, items) in before {
                    scene.getattr(name)?.set_item(&all, items)?;
                }
                return Err(err);
            }
            s.setattr("after", mesh.call_method0("capture_state")?)?;
            for (name, added, removed, old) in before {
                let now = scene.getattr(name)?;
                let additions = PyList::empty(py);
                let removals = PyList::empty(py);
                for x in now.try_iter()? {
                    let x = x?;
                    if !old.contains(&x)? {
                        additions.append(x)?;
                    }
                }
                for (index, x) in old.iter().enumerate() {
                    if !now.contains(&x)? {
                        removals.append((index, x))?;
                    }
                }
                s.setattr(added, additions)?;
                s.setattr(removed, removals)?;
            }
        } else {
            mesh.call_method1("restore_state", (after,))?;
            apply_members(s, scene, false)?;
        }
        crate::doc::steps::version(scene)
    }
    fn undo(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<()> {
        let before = slf.getattr("before")?;
        if !before.is_none() {
            scene
                .getattr("mesh")?
                .call_method1("restore_state", (before,))?;
        }
        apply_members(slf.as_any(), scene, true)?;
        crate::doc::steps::version(scene)
    }
}
