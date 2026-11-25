// SPDX-License-Identifier: GPL-3.0-or-later
//! Entry placement frames for world-coordinate reference edits.
use super::*;
fn copy_matrix<'py>(py: Python<'py>, m: &Obj<'py>) -> PyResult<Obj<'py>> {
    if m.is_none() {
        return Ok(m.clone());
    }
    py.get_type::<crate::math::Matrix4>().call1((m,))
}
#[pyfunction]
fn parts_record<'py>(group: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = group.py();
    let axes = group
        .getattr_opt("axes")?
        .unwrap_or_else(|| py.None().into_bound(py));
    let parts = PyList::empty(py);
    for child in group.getattr("children")?.try_iter()? {
        let child = child?;
        parts.append((
            &child,
            copy_matrix(py, &child.getattr("xform")?)?,
            child.getattr("explode_offset")?,
        ))?;
    }
    Ok((copy_matrix(py, &axes)?, parts)
        .into_pyobject(py)?
        .into_any())
}
#[pyfunction]
fn put_parts(group: &Obj<'_>, record: &Obj<'_>) -> PyResult<()> {
    let py = group.py();
    for part in record.get_item(1)?.try_iter()? {
        let part = part?;
        let child = part.get_item(0)?;
        child.setattr("xform", copy_matrix(py, &part.get_item(1)?)?)?;
        child.setattr("explode_offset", part.get_item(2)?)?;
    }
    group.setattr("axes", copy_matrix(py, &record.get_item(0)?)?)
}
#[pyfunction]
fn bake_container_xform(group: &Obj<'_>) -> PyResult<()> {
    let py = group.py();
    let Some(matrix) = group.getattr_opt("xform")?.filter(|v| !v.is_none()) else {
        return Ok(());
    };
    let identity = py.get_type::<crate::math::Matrix4>().call0()?;
    if matrix.eq(&identity)? {
        return Ok(());
    }
    for child in group.getattr("children")?.try_iter()? {
        let child = child?;
        let m = child.getattr("xform")?;
        child.setattr(
            "xform",
            matrix.mul(if m.is_none() { &identity } else { &m })?,
        )?;
    }
    py.import("core.explode")?
        .call_method1("rotate_offsets", (group, &matrix))?;
    let mesh = group.getattr("mesh")?;
    if mesh.getattr("vertices")?.is_truthy()? {
        group.setattr(
            "mesh",
            py.import("core.group")?
                .call_method1("transformed_mesh", (mesh, &matrix))?,
        )?;
    }
    group.setattr("xform", identity)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(bake_container_xform, m)?)?;
    m.add_function(wrap_pyfunction!(parts_record, m)?)?;
    m.add_function(wrap_pyfunction!(put_parts, m)?)?;
    Ok(())
}
