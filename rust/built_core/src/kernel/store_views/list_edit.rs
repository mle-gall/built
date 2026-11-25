//! Apply list operations to the current incidence, then refresh the held list.
use super::Obj;
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList, PySlice, PyTuple},
};

#[pyfunction]
pub(super) fn mutate_incidence<'py>(
    held: &Obj<'py>,
    operation: &str,
    args: &Bound<'py, PyTuple>,
    kwargs: &Bound<'py, PyDict>,
) -> PyResult<Obj<'py>> {
    let py = held.py();
    let owner = held.getattr("_owner")?;
    let path = held.getattr("_path")?;
    let field = path.get_item(0)?.extract::<String>()?;
    let current = owner.getattr(&field)?;
    let index = if path.len()? > 1 {
        Some(path.get_item(1)?.extract::<usize>()?)
    } else {
        None
    };
    let ring = if let Some(i) = index {
        if i >= current.len()? || !current.get_item(i)?.eq(held)? {
            return Err(PyValueError::new_err(
                "the held hole ring has been replaced",
            ));
        }
        current.get_item(i)?
    } else {
        current.clone()
    };
    let list = py.get_type::<PyList>();
    let target = list.call1((ring,))?;
    let result = target.call_method(operation, args, Some(kwargs))?;
    if let Some(i) = index {
        let rings = list.call1((current,))?;
        rings.set_item(i, &target)?;
        owner.setattr(&field, rings)?;
    } else {
        owner.setattr(&field, &target)?;
    }
    let refreshed = if index.is_none() && field == "hole_loops" {
        owner.getattr(&field)?
    } else {
        target
    };
    list.call_method1("__setitem__", (held, PySlice::full(py), refreshed))?;
    Ok(result)
}
