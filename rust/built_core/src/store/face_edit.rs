// SPDX-License-Identifier: GPL-3.0-or-later
//! Run keyed face edits inside a snapshot, including rollback on refusal.
use pyo3::{
    prelude::*,
    sync::PyOnceLock,
    types::{PyDict, PyTuple},
};
type Obj<'p> = Bound<'p, PyAny>;

pub(crate) fn scoped<T>(mesh: &Obj, run: impl FnOnce() -> PyResult<T>) -> PyResult<T> {
    let state = mesh.getattr("__dict__")?.cast_into::<PyDict>()?;
    let previous = state.get_item("_resident_edit")?;
    state.set_item("_resident_edit", true)?;
    let result = run();
    match previous {
        Some(value) => state.set_item("_resident_edit", value)?,
        None => {
            state.call_method1("pop", ("_resident_edit", mesh.py().None()))?;
        }
    }
    result
}
#[pyfunction]
fn change<'p>(step: &Obj<'p>) -> PyResult<Obj<'p>> {
    let after = step.getattr("after")?;
    if after.is_none() {
        return Ok(after);
    }
    after.call_method1("changes_since", (step.getattr("before")?,))
}
#[pyfunction]
fn execute(step: &Obj, scene: &Obj) -> PyResult<()> {
    let mesh = step
        .call_method1("geometry_meshes", (scene,))?
        .get_item(0)?;
    scoped(&mesh, || {
        let base = step.getattr("_snapshot_class")?;
        if let Err(err) = base.call_method1("do", (step, scene)) {
            let before = step.getattr("before")?;
            if !before.is_none() {
                mesh.call_method1("restore_state", (before,))?;
            }
            return Err(err);
        }
        Ok(())
    })
}
#[pyfunction]
fn apply<'p>(step: &Obj<'p>, scene: &Obj) -> PyResult<Obj<'p>> {
    step.call_method1("do", (scene,))?;
    change(step)
}
// The history facade owns these Python bases. Resolve them on first construction
// so importing the extension never depends on a partially initialized history.
fn snapshot_class<'p>(py: Python<'p>, compound: bool) -> PyResult<&'p Obj<'p>> {
    static MUTATION: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
    static COMPOUND: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
    let (cache, name, base) = if compound {
        (&COMPOUND, "StoreFaceCompound", "SnapshotCompound")
    } else {
        (&MUTATION, "StoreFaceMutation", "SnapshotMutation")
    };
    Ok(cache
        .get_or_try_init(py, || -> PyResult<Py<PyAny>> {
            let base = py.import("built_core.history")?.getattr(base)?;
            let builtins = py.import("builtins")?;
            let method = py.import("functools")?.getattr("partialmethod")?;
            let fields = PyDict::new(py);
            fields.set_item("__module__", "built_core")?;
            fields.set_item("_snapshot_class", &base)?;
            fields.set_item("do", method.call1((wrap_pyfunction!(execute, py)?,))?)?;
            fields.set_item("apply", method.call1((wrap_pyfunction!(apply, py)?,))?)?;
            fields.set_item(
                "change",
                builtins.call_method1("property", (wrap_pyfunction!(change, py)?,))?,
            )?;
            Ok(builtins
                .call_method1("type", (name, PyTuple::new(py, [base])?, fields))?
                .unbind())
        })?
        .bind(py))
}
#[pyfunction(signature=(mutate,mesh=None))]
pub(crate) fn resident_face_mutation<'p>(
    mutate: &Obj<'p>,
    mesh: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    snapshot_class(mutate.py(), false)?.call1((mutate, mesh))
}
#[pyfunction(signature=(inner,*,heal=true))]
pub(crate) fn resident_face_compound<'p>(inner: &Obj<'p>, heal: bool) -> PyResult<Obj<'p>> {
    let kw = PyDict::new(inner.py());
    kw.set_item("heal", heal)?;
    snapshot_class(inner.py(), true)?.call((inner,), Some(&kw))
}
