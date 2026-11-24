// SPDX-License-Identifier: GPL-3.0-or-later
//! History command entry points. The caller enlists the facade in the document
//! transaction. Going through its store also publishes edits to an open definition.
use crate::math::Vec3;
use pyo3::{prelude::*, types::PyDict};

pub(crate) struct DrawnEdge {
    pub(crate) key: Option<u128>,
    pub(crate) flags_changed: bool,
    pub(crate) dirty: u32,
}

fn store_hypot(mesh: &Bound<'_, PyAny>) -> PyResult<Option<String>> {
    let api = mesh.py().import("core.store_mesh")?;
    if !mesh.get_type().is(&api.getattr("StoreMesh")?)
        || mesh
            .getattr("__dict__")?
            .cast::<PyDict>()?
            .contains("_planned_points")?
    {
        return Ok(None);
    }
    api.call_method0("_hypot")?.extract()
}

fn input_point(mesh: &Bound<'_, PyAny>, point: [f64; 3]) -> PyResult<[f64; 3]> {
    mesh.call_method1(
        "_input_point",
        (Vec3::from(glam::DVec3::from_array(point)),),
    )?
    .extract()
}

pub(crate) fn command_draw_edge(
    mesh: &Bound<'_, PyAny>,
    a: [f64; 3],
    b: [f64; 3],
    soft: Option<bool>,
    curve: Option<i64>,
    layer: Option<&str>,
) -> PyResult<Option<DrawnEdge>> {
    if store_hypot(mesh)?.is_none() {
        return Ok(None);
    }
    let default_layer = mesh.py().import("core.layers")?.getattr("DEFAULT_LAYER")?;
    let a = input_point(mesh, a)?;
    let b = input_point(mesh, b)?;
    let key: Option<u128> = mesh
        .getattr("_store")?
        .call_method1(
            "draw_command_edge",
            (a, b, soft, curve, layer, default_layer),
        )?
        .extract()?;
    let dirty = u32::from(key.is_some());
    let flags_changed = key.is_none() && (soft.is_some() || curve.is_some() || layer.is_some());
    if dirty != 0 {
        mesh.call_method1("_dirty", (dirty,))?;
    } else if flags_changed {
        mesh.call_method0("mark_modified")?;
    }
    Ok(Some(DrawnEdge {
        key,
        flags_changed,
        dirty,
    }))
}
