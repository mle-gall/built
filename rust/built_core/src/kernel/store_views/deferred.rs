// SPDX-License-Identifier: GPL-3.0-or-later
//! First-touch construction of a native block or plugin-supplied callback mesh.
use super::Obj;
use pyo3::{prelude::*, types::PyDict};

#[pyfunction]
#[pyo3(signature=(mesh,build,nfaces,source=None))]
fn defer(
    mesh: &Obj<'_>,
    build: &Obj<'_>,
    nfaces: &Obj<'_>,
    source: Option<&Obj<'_>>,
) -> PyResult<bool> {
    let py = mesh.py();
    let api = py.import("built_core.store_mesh")?;
    if !mesh.get_type().is(&api.getattr("StoreMesh")?)
        || mesh
            .getattr("_store")?
            .call_method0("counts")?
            .extract::<(usize, usize, usize)>()?
            != (0, 0, 0)
    {
        return Ok(false);
    }
    let state = mesh.getattr("__dict__")?.cast_into::<PyDict>()?;
    for name in api.getattr("STORE_SIDE")?.try_iter()? {
        let key = name?;
        if state.contains(&key)? {
            state.del_item(key)?;
        }
    }
    state.set_item("_deferred", build)?;
    state.set_item("_deferred_nfaces", nfaces)?;
    state.set_item("_deferred_source", source)?;
    mesh.setattr("__class__", api.getattr("DeferredMesh")?)?;
    Ok(true)
}

#[pyfunction]
fn materialize(mesh: &Obj<'_>) -> PyResult<()> {
    let py = mesh.py();
    let api = py.import("built_core.store_mesh")?;
    let state = mesh.getattr("__dict__")?.cast_into::<PyDict>()?;
    // Publish only after first-use validation succeeds. A damaged trim
    // leaves the original deferred definition available for diagnostics.
    if state.contains("_deferred_exact")? {
        let staged = api.getattr("StoreMesh")?.call0()?;
        state.as_any().get_item("_deferred")?.call1((&staged,))?;
        for k in [
            "_deferred_exact",
            "_deferred",
            "_deferred_nfaces",
            "_deferred_source",
        ] {
            state.del_item(k)?;
        }
        super::lifecycle::become_store(
            mesh,
            &staged.getattr("_store")?,
            &staged.getattr("_attr_table")?,
            Some(&staged.getattr("_attr_own")?),
        )?;
        let observe = py
            .import("built_core.mesh")?
            .getattr("on_mesh_materialized")?
            .call_method0("get")?;
        if !observe.is_none() {
            observe.call1((mesh,))?;
        }
        return Ok(());
    }
    let source = state.as_any().get_item("_deferred_source")?;
    let mut block = !source.is_none();
    if block {
        for k in ["pos", "edges", "rings", "sizes", "counts"] {
            if source.call_method1("get", (k,))?.is_none() {
                block = false;
                break;
            }
        }
    }
    let built = if block {
        Some(super::block::store_from_source(&source, false, None)?)
    } else {
        None
    };
    let build = state.as_any().get_item("_deferred")?;
    for k in [
        "_deferred",
        "_deferred_nfaces",
        "_deferred_source",
        "_deferred_geom",
        "_deferred_plan",
        "_drawing_staged",
        "_drawing_source_key",
        "_drawing_texture_paths",
    ] {
        if state.contains(k)? {
            state.del_item(k)?;
        }
    }
    if let Some((st, table)) = built {
        super::lifecycle::become_store(mesh, &st, table.as_any(), None)?;
    } else {
        mesh.setattr("__class__", api.getattr("StoreMesh")?)?;
        super::lifecycle::initialise_store(mesh)?;
        build.call1((mesh,))?;
    }
    let observe = py
        .import("built_core.mesh")?
        .getattr("on_mesh_materialized")?
        .call_method0("get")?;
    if !observe.is_none() {
        observe.call1((mesh,))?;
    }
    Ok(())
}

pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(defer, m)?)?;
    m.add_function(wrap_pyfunction!(materialize, m)?)?;
    Ok(())
}
