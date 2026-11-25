// SPDX-License-Identifier: GPL-3.0-or-later
//! Plugin element properties resolve the current store on every access.
use super::Obj;
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    types::{PyBool, PyDict, PyInt, PyList, PyString},
};

fn owner<'py>(view: &Obj<'py>) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let mesh = view.getattr("_m")?;
    let store = mesh.getattr("_store")?;
    Ok((mesh, store))
}

#[pyfunction]
fn endpoint<'py>(index: usize, view: &Obj<'py>) -> PyResult<Obj<'py>> {
    let (mesh, store) = owner(view)?;
    let key = store
        .call_method1("edge_ends", (view.getattr("key")?,))?
        .get_item(index)?;
    view.py()
        .import("ingetrazo_core.store_mesh")?
        .call_method1("VertexView", (mesh, key))
}

#[pyfunction]
fn set_endpoint(index: usize, view: &Obj<'_>, vertex: &Obj<'_>) -> PyResult<()> {
    let (mesh, store) = owner(view)?;
    let key = mesh.call_method1("_incidence_key", (vertex, 0))?;
    store.call_method1("set_endpoint", (view.getattr("key")?, index, key))?;
    mesh.call_method0("_dirty")?;
    Ok(())
}

fn flag_bit(name: &str) -> Option<u8> {
    match name {
        "soft" => Some(1),
        "hidden" => Some(2),
        "flat" => Some(4),
        _ => None,
    }
}

#[pyfunction]
fn edge_flag<'py>(name: &str, view: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    let (_, store) = owner(view)?;
    let row = store.call_method1("edge", (view.getattr("key")?,))?;
    if let Some(bit) = flag_bit(name) {
        return Ok(
            PyBool::new(py, row.get_item(2)?.extract::<u8>()? & bit != 0)
                .to_owned()
                .into_any(),
        );
    }
    if name == "curve" {
        let id = row.get_item(3)?;
        return Ok(if id.lt(0)? {
            py.None().into_bound(py)
        } else {
            id
        });
    }
    row.get_item(4)
}

#[pyfunction]
fn set_edge_flag(name: &str, view: &Obj<'_>, value: &Obj<'_>) -> PyResult<()> {
    let (_, store) = owner(view)?;
    let key = view.getattr("key")?;
    let invalid = || -> PyResult<()> {
        Err(PyTypeError::new_err(format!(
            "invalid {name} value: {}",
            value.repr()?
        )))
    };
    if let Some(bit) = flag_bit(name) {
        if !value.eq(0)? && !value.eq(1)? {
            return invalid();
        }
        let flags: u8 = store
            .call_method1("edge", (&key,))?
            .get_item(2)?
            .extract()?;
        store.call_method1(
            "set_edge_flags",
            (
                key,
                if value.is_truthy()? {
                    flags | bit
                } else {
                    flags & !bit
                },
            ),
        )?;
    } else if name == "curve" {
        if !value.is_none() && !value.is_instance_of::<PyInt>() {
            return invalid();
        }
        store.call_method1("set_edge_curve", (key, value))?;
    } else {
        if !value.is_none() && !value.is_instance_of::<PyString>() {
            return invalid();
        }
        store.call_method1("set_edge_layer", (key, value))?;
    }
    Ok(())
}

#[pyfunction]
fn interior(view: &Obj<'_>) -> PyResult<bool> {
    let (_, store) = owner(view)?;
    Ok(store
        .call_method1("face_info", (view.getattr("key")?,))?
        .get_item(1)?
        .extract::<u8>()?
        & 1
        != 0)
}

#[pyfunction]
fn set_interior(view: &Obj<'_>, value: &Obj<'_>) -> PyResult<()> {
    if !value.is_instance_of::<PyBool>() {
        return Err(PyTypeError::new_err("face.interior requires a bool"));
    }
    let (_, store) = owner(view)?;
    let key = view.getattr("key")?;
    let flags = store
        .call_method1("face_info", (&key,))?
        .get_item(1)?
        .extract::<u8>()?;
    store.call_method1(
        "set_face_flags",
        (
            key,
            if value.is_truthy()? {
                flags | 1
            } else {
                flags & !1
            },
        ),
    )?;
    Ok(())
}

pub(super) fn attribute_owner<'py>(mesh: &Obj<'py>) -> PyResult<Obj<'py>> {
    Ok(mesh
        .getattr_opt("_instance_definition")?
        .unwrap_or_else(|| mesh.clone()))
}

#[pyfunction]
pub(crate) fn copy_attrs<'py>(row: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let py = row.py();
    let result = PyDict::new(py);
    if row.is_truthy()? {
        for pair in row.call_method0("items")?.try_iter()? {
            let pair = pair?;
            let value = pair.get_item(1)?;
            let value = if value.is_exact_instance_of::<PyDict>()
                || value.is_exact_instance_of::<PyList>()
            {
                value.call_method0("copy")?
            } else {
                value
            };
            result.set_item(pair.get_item(0)?, value)?;
        }
    }
    Ok(result)
}

#[pyfunction]
fn face_attrs<'py>(view: &Obj<'py>) -> PyResult<Obj<'py>> {
    let (mesh, store) = owner(view)?;
    let attrs = attribute_owner(&mesh)?;
    let own = attrs.getattr("_attr_own")?.cast_into::<PyDict>()?;
    let key = view.getattr("key")?;
    if let Some(d) = own.get_item(&key)? {
        if !d.is_none() {
            return Ok(d);
        }
    }
    let index = store
        .call_method1("face_info", (&key,))?
        .get_item(0)?
        .extract::<i64>()?;
    let row = if index < 0 {
        PyDict::new(view.py())
    } else {
        copy_attrs(&attrs.getattr("_attr_table")?.get_item(index)?)?
    };
    own.set_item(key, &row)?;
    Ok(row.into_any())
}

#[pyfunction]
fn set_face_attrs(view: &Obj<'_>, value: &Obj<'_>) -> PyResult<()> {
    attribute_owner(&view.getattr("_m")?)?
        .getattr("_attr_own")?
        .set_item(view.getattr("key")?, value)
}

pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(endpoint, m)?)?;
    m.add_function(wrap_pyfunction!(set_endpoint, m)?)?;
    m.add_function(wrap_pyfunction!(edge_flag, m)?)?;
    m.add_function(wrap_pyfunction!(set_edge_flag, m)?)?;
    m.add_function(wrap_pyfunction!(interior, m)?)?;
    m.add_function(wrap_pyfunction!(set_interior, m)?)?;
    m.add_function(wrap_pyfunction!(face_attrs, m)?)?;
    m.add_function(wrap_pyfunction!(set_face_attrs, m)?)?;
    m.add_function(wrap_pyfunction!(copy_attrs, m)?)?;
    Ok(())
}
