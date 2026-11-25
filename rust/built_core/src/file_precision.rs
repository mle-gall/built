// SPDX-License-Identifier: GPL-3.0-or-later
//! Built binary64 placements and legacy IGZ projection.
use glam::DMat4;
use numpy::{ndarray::Array2, IntoPyArray};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList},
};
type Obj<'py> = Bound<'py, PyAny>;
#[pyfunction]
fn built_position<'py>(vertex: &Obj<'py>) -> PyResult<Obj<'py>> {
    if vertex.hasattr("position_tuple")? {
        vertex.call_method0("position_tuple")
    } else {
        vertex.getattr("position")?.call_method0("toTuple")
    }
}
#[pyfunction]
fn built_placement(py: Python<'_>, values: &Obj<'_>) -> PyResult<crate::math::Matrix4> {
    let float = py.get_type::<pyo3::types::PyFloat>();
    let nums = values
        .try_iter()?
        .map(|v| float.call1((v?,))?.extract::<f64>())
        .collect::<PyResult<Vec<_>>>()?;
    let m: [f64; 16] = nums
        .try_into()
        .map_err(|_| PyValueError::new_err("Invalid Built placement transform"))?;
    if !m.iter().all(|v| v.is_finite()) {
        return Err(PyValueError::new_err("Invalid Built placement transform"));
    }
    Ok(DMat4::from_cols_array(&m).into())
}
#[pyfunction]
fn built_placement_values<'py>(matrix: &Obj<'py>) -> PyResult<Obj<'py>> {
    let current = matrix.call_method0("data")?;
    if let Ok(saved) = matrix.getattr("_built_source") {
        if saved.is_truthy()? {
            let out = PyList::empty(matrix.py());
            let (values, projected): (Obj<'_>, Obj<'_>) = saved.extract()?;
            for ((v, p), c) in values
                .try_iter()?
                .zip(projected.try_iter()?)
                .zip(current.try_iter()?)
            {
                let v = v?;
                let c = c?;
                out.append(if p?.eq(&c)? { v } else { c })?;
            }
            return Ok(out.into_any());
        }
    }
    matrix.py().get_type::<PyList>().call1((current,))
}
#[pyfunction]
fn built_compact_values<'py>(values: &Obj<'py>) -> PyResult<Obj<'py>> {
    values
        .py()
        .import("numpy")?
        .call_method1("asarray", (values, "float32"))?
        .call_method0("tolist")
}
#[pyfunction]
fn built_point_array<'py>(points: &Obj<'py>, sink: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = points.py();
    let np = py.import("numpy")?;
    let coords = np
        .call_method1("asarray", (points,))?
        .call_method1("reshape", (-1, 3))?;
    let dtype = coords
        .getattr("dtype")?
        .getattr("str")?
        .extract::<String>()?;
    let coords = if dtype == "<f4" || dtype == "<f8" {
        np.call_method1("ascontiguousarray", (coords,))?
    } else {
        np.call_method1("ascontiguousarray", (coords, "float64"))?
    };
    let flag = |k| sink.getattr(k).map_or(Ok(false), |v| v.is_truthy());
    let report = flag("compatibility")?;
    let placements = if report {
        sink.getattr("current_placements")?.extract()?
    } else {
        Vec::new()
    };
    let (result, mm) = crate::file_geometry::igz_points(py, &coords, flag("built")?, placements)
        .map_err(|e| {
            if e.is_instance_of::<PyValueError>(py) {
                match py
                    .import("built_core.formats.igz")
                    .and_then(|m| m.call_method1("NonFiniteDocumentError", ("geometry",)))
                {
                    Ok(v) => PyErr::from_value(v),
                    Err(e) => e,
                }
            } else {
                e
            }
        })?;
    if report {
        sink.setattr(
            "displacement_mm",
            mm.max(sink.getattr("displacement_mm")?.extract()?),
        )?;
    }
    Ok(result)
}
fn array(py: Python<'_>, m: DMat4) -> Obj<'_> {
    let a = m.to_cols_array();
    Array2::from_shape_fn((4, 4), |(r, c)| a[c * 4 + r])
        .into_pyarray(py)
        .into_any()
}
#[pyfunction]
fn built_export_placements<'py>(scene: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let py = scene.py();
    let out = PyDict::new(py);
    let identity = DMat4::IDENTITY;
    let mesh = scene.getattr("mesh").unwrap_or_else(|_| scene.clone());
    let placements = |mesh: &Obj<'py>, a: DMat4, b: DMat4| -> PyResult<()> {
        out.call_method1("setdefault", (mesh.as_ptr() as usize, PyList::empty(py)))?
            .call_method1("append", ((array(py, a), array(py, b)),))?;
        Ok(())
    };
    placements(&mesh, identity, identity)?;
    let placeholder = crate::model::module(py, "built_core")?.getattr("link_save_as_placeholder")?;
    for key in ["groups", "kept_components"] {
        let Ok(groups) = scene.getattr(key) else {
            continue;
        };
        if groups.is_none() {
            continue;
        }
        let mut stack = vec![(groups.try_iter()?, identity, identity)];
        while let Some((groups, world, exported)) = stack.last_mut() {
            let Some(g) = groups.next() else {
                stack.pop();
                continue;
            };
            let g = g?;
            let xf = g.getattr("xform")?;
            let (mut a, mut b) = (*world, *exported);
            if !xf.is_none() {
                if placeholder.call1((&g, scene))?.is_truthy()? {
                    continue;
                }
                a *= DMat4::from_cols_array(&built_placement_values(&xf)?.extract()?);
                let raw = xf.call_method0("data")?.extract::<[f64; 16]>()?;
                b *= DMat4::from_cols_array(&raw.map(|v| v as f32 as f64));
            }
            placements(&g.getattr("mesh")?, a, b)?;
            stack.push((g.getattr("children")?.try_iter()?, a, b));
        }
    }
    Ok(out)
}

#[path = "file_facade.rs"]
mod facade;
#[pyfunction]
fn igz_nonfinite_error(owner: &Obj<'_>, r#where: &Obj<'_>) -> PyResult<()> {
    let py = owner.py();
    owner.setattr("where", r#where)?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("where", r#where)?;
    let message = py.import("built_core.i18n")?.call_method(
        "tr",
        (concat!(
            "The model has a point with invalid coordinates (not a number), ",
            "so it was not saved and the file on disk keeps its last good ",
            "version. Undo the last step and save again. Where: {where}"
        ),),
        Some(&kwargs),
    )?;
    owner
        .py()
        .get_type::<pyo3::exceptions::PyValueError>()
        .call_method1("__init__", (owner, message))?;
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(igz_nonfinite_error, m)?)?;
    facade::register(m)?;
    m.add_function(wrap_pyfunction!(built_position, m)?)?;
    m.add_function(wrap_pyfunction!(built_placement, m)?)?;
    m.add_function(wrap_pyfunction!(built_placement_values, m)?)?;
    m.add_function(wrap_pyfunction!(built_point_array, m)?)?;
    m.add_function(wrap_pyfunction!(built_compact_values, m)?)?;
    m.add_function(wrap_pyfunction!(built_export_placements, m)?)?;
    Ok(())
}
