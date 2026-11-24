// SPDX-License-Identifier: GPL-3.0-or-later
//! Membership of the current edit context and the broader snap/occlusion index.
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet},
};

#[pyfunction]
pub fn context_placements<'py>(view: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let context = view.getattr("scene")?.getattr("edit_group")?;
    let placements = view.call_method0("_placements")?;
    if context.is_none() {
        return Ok(placements);
    }
    let children = PyList::empty(view.py());
    for group in placements.try_iter()? {
        let group = group?;
        if group
            .getattr_opt("context")?
            .is_some_and(|ctx| ctx.is(&context))
            && !group
                .getattr_opt("owner")?
                .is_some_and(|owner| owner.is(&context))
        {
            children.append(group)?;
        }
    }
    Ok(children.into_any())
}

type Candidates<'py> = (Bound<'py, PySet>, Bound<'py, PyList>, Bound<'py, PyDict>);

#[pyfunction]
pub fn pick_candidates<'py>(
    view: &Bound<'py, PyAny>,
    hidden: bool,
    lazy: Option<&Bound<'py, PyAny>>,
) -> PyResult<Candidates<'py>> {
    let py = view.py();
    let editable = view.call_method0("_context_placements")?;
    let selectable = PySet::empty(py)?;
    for g in editable.try_iter()? {
        selectable.add(g?.as_ptr() as usize)?;
    }
    let context = view.getattr("scene")?.getattr("edit_group")?;
    let show_rest = !context.is_none() && !hidden;
    let candidates = if show_rest {
        view.call_method0("_placements")?
    } else {
        editable
    };
    let lazy_ids = lazy.map(|table| table.get_item(4)).transpose()?;
    let fixed = PyList::empty(py);
    let positions = PyDict::new(py);
    let mut index = 0usize;
    for group in candidates.try_iter()? {
        let group = group?;
        if show_rest
            && (group.is(&context) || view.call_method1("_owner_of", (&group,))?.is(&context))
        {
            continue;
        }
        let id = group.as_ptr() as usize;
        if lazy_ids
            .as_ref()
            .map_or(Ok(false), |ids| ids.contains(id))?
        {
            positions.set_item(id, (index, group))?;
        } else {
            fixed.append(group)?;
        }
        index += 1;
    }
    Ok((selectable, fixed, positions))
}
