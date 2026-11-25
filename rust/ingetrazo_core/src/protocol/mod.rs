// SPDX-License-Identifier: GPL-3.0-or-later
//! Key resolution and execution of public History verbs.
use pyo3::{
    exceptions::{PyKeyError, PyValueError},
    prelude::*,
    types::{PyDict, PyList, PyModule, PyTuple},
};
pub(crate) mod api;
mod placements;
mod transforms;
mod inspectors;
mod solids;
mod paint_place;
mod guides;
mod paste;
mod profile_tools;
mod mesh_tools;
mod document;
mod drawing;
mod materials;
pub(crate) mod keys;
mod queries;
mod reference;
mod annotations;

type Obj<'py> = Bound<'py, PyAny>;
type Reply<'py> = PyResult<Bound<'py, PyDict>>;

#[pyfunction]
#[pyo3(signature=(history, before, ok=true))]
fn answer<'py>(history: &Obj<'py>, before: &Obj<'py>, ok: bool) -> Reply<'py> {
    let py = history.py();
    let step = if history.getattr("revision")?.ne(before)? {
        history.getattr("feed")?.getattr("last")?
    } else {
        py.None().into_bound(py)
    };
    let result = PyDict::new(py);
    result.set_item("ok", ok && !step.is_none())?;
    result.set_item("step", step)?;
    result.set_item("error", history.getattr("last_error")?)?;
    result.set_item("rejected", history.getattr_opt("last_rejected")?)?;
    Ok(result)
}

#[pyfunction]
fn execute<'py>(history: &Obj<'py>, command: &Obj<'py>) -> Reply<'py> {
    let before = history.getattr("revision")?;
    if !command.is_none() {
        history.call_method1("execute", (command,))?;
    }
    answer(history, &before, true)
}

fn history_command<'py>(
    py: Python<'py>,
    name: &str,
    args: impl pyo3::call::PyCallArgs<'py>,
) -> PyResult<Obj<'py>> {
    py.import("ingetrazo_core.history")?.call_method1(name, args)
}

#[pyfunction]
fn need<'py>(mesh: &Obj<'py>, kind: &str, keys: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    use crate::store::keys::Kind;
    let py = mesh.py();
    let kind_enum =
        Kind::parse(kind).ok_or_else(|| PyValueError::new_err("expected vertex, edge or face"))?;
    // Parse the complete request before resolving it, including a later malformed key.
    let keys = keys
        .try_iter()?
        .map(|k| keys::from_json_key(&k?))
        .collect::<PyResult<Vec<_>>>()?;
    let result = PyList::empty(py);
    let store = mesh.getattr("_store")?;
    let class = py.import("core.store_mesh")?.getattr(match kind_enum {
        Kind::Vertex => "VertexView",
        Kind::Edge => "EdgeView",
        Kind::Face => "FaceView",
    })?;
    for key in keys {
        if key
            .extract::<u128>()
            .ok()
            .and_then(crate::store::keys::split_key)
            .is_some_and(|(_, k, _, _)| k == kind_enum)
            && store.call_method1("is_live", (&key,))?.is_truthy()?
        {
            result.append(class.call1((mesh, key))?)?;
        } else {
            return Err(PyKeyError::new_err(format!(
                "no live {kind} with key {key} in the open mesh"
            )));
        }
    }
    Ok(result)
}

#[pyfunction]
fn move_vertices<'py>(history: &Obj<'py>, vertices: &Obj<'py>, delta: &Obj<'py>) -> Reply<'py> {
    let py = history.py();
    let mesh = history.getattr("scene")?.getattr("mesh")?;
    let selected = need(&mesh, "vertex", vertices)?;
    let points = PyList::empty(py);
    for v in selected.iter() {
        points.append(v.call_method0("position_tuple")?)?;
    }
    let xyz = delta
        .try_iter()?
        .map(|v| py.get_type::<pyo3::types::PyFloat>().call1((v?,)))
        .collect::<PyResult<Vec<_>>>()?;
    if xyz.len() != 3 {
        return Err(PyValueError::new_err(
            "expected three displacement coordinates",
        ));
    }
    let cmd = history_command(py, "MoveVerticesCommand", (points, PyTuple::new(py, xyz)?))?;
    execute(history, &cmd)
}

#[pyfunction]
fn undo<'py>(history: &Obj<'py>) -> Reply<'py> {
    replay(history, "undo")
}

#[pyfunction]
fn redo<'py>(history: &Obj<'py>) -> Reply<'py> {
    replay(history, "redo")
}

fn replay<'py>(history: &Obj<'py>, method: &str) -> Reply<'py> {
    let before = history.getattr("revision")?;
    history.call_method0(method)?;
    answer(history, &before, true)
}

pub(crate) fn register(root: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = root.py();
    let m = PyModule::new(py, "ingetrazo_core.command_protocol")?;
    m.add_function(wrap_pyfunction!(api::run, &m)?)?;
    m.add_function(wrap_pyfunction!(answer, &m)?)?;
    m.add_function(wrap_pyfunction!(execute, &m)?)?;
    m.add_function(wrap_pyfunction!(need, &m)?)?;
    m.add_function(wrap_pyfunction!(move_vertices, &m)?)?;
    m.add_function(wrap_pyfunction!(undo, &m)?)?;
    m.add_function(wrap_pyfunction!(redo, &m)?)?;
    paint_place::register(&m)?;
    solids::register(&m)?;
    guides::register(&m)?;
    profile_tools::register(&m)?;
    inspectors::register(&m)?;
    mesh_tools::register(&m)?;
    transforms::register(&m)?;
    drawing::register(&m)?;
    keys::register(&m)?;
    queries::register(&m)?;
    reference::register(&m)?;
    annotations::register(&m)?;
    materials::register(&m)?;
    document::register(&m)?;
    placements::register(&m)?;
    py.import("sys")?
        .getattr("modules")?
        .set_item("ingetrazo_core.command_protocol", &m)?;
    root.add("command_protocol", m)
}
