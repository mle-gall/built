// SPDX-License-Identifier: GPL-3.0-or-later
//! The source tab draws host edges in the source frame. The extra pass has
//! no scene IDs and never contributes to selection, snapping or saving.
use crate::{ui::tokens, Command};
use glam::{DMat4, DVec3, Mat4, Vec3};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyBytes, PyDict},
};
use std::collections::HashSet;

pub type Tag = (String, String, bool);

fn bytes<'py>(packed: &Bound<'py, PyAny>, field: &str) -> PyResult<Bound<'py, PyBytes>> {
    Ok(packed
        .getattr(field)?
        .call_method0("tobytes")?
        .cast_into()?)
}

pub fn snapshot(
    scene: &Bound<'_, PyAny>,
    mount: &Bound<'_, PyAny>,
    origin: [f64; 3],
) -> PyResult<(Vec<u8>, Vec<Tag>)> {
    let py = scene.py();
    let matrix = py
        .import("core.outliner")?
        .getattr("world_matrix")?
        .call1((scene.getattr("groups")?, mount))?;
    let world = if matrix.is_none() {
        DMat4::IDENTITY
    } else {
        DMat4::from_cols_array(&matrix.call_method0("data")?.extract()?)
    };
    if world.determinant().abs() < f64::EPSILON {
        return Err(PyValueError::new_err("Singular linked placement"));
    }
    let mut omitted = HashSet::new();
    let mut stack = vec![mount.clone()];
    while let Some(group) = stack.pop() {
        if omitted.insert(group.as_ptr()) {
            stack.extend(
                group
                    .getattr("children")?
                    .extract::<Vec<Bound<'_, PyAny>>>()?,
            );
        }
    }

    let kwargs = PyDict::new(py);
    kwargs.set_item("texture_images", false)?;
    let packed = py
        .import("ingetrazo_core")?
        .getattr("pack_scene")?
        .call((scene,), Some(&kwargs))?;
    let vertices = bytes(&packed, "vertices")?;
    let indices = bytes(&packed, "indices")?;
    let commands = bytes(&packed, "edge_commands")?;
    let instances = bytes(&packed, "instances")?;
    let groups = packed.getattr("groups")?;
    let offset = DVec3::from_array(packed.getattr("origin")?.extract()?);
    let transform = DMat4::from_translation(-DVec3::from_array(origin))
        * world.inverse()
        * DMat4::from_translation(offset);
    let mut edges = Vec::new();
    for (i, row) in instances.as_bytes().chunks_exact(80).enumerate() {
        if row[72] & 1 != 0 || omitted.contains(&groups.get_item(i)?.as_ptr()) {
            continue;
        }
        let prototype = u32::from_le_bytes(row[64..68].try_into().unwrap()) as usize;
        let fail = || PyValueError::new_err("Invalid packed host geometry");
        let cmd: Command = bytemuck::pod_read_unaligned(
            commands
                .as_bytes()
                .get(prototype * 20..prototype * 20 + 20)
                .ok_or_else(fail)?,
        );
        let count = cmd.count as usize;
        let start = cmd.first as usize;
        let matrix =
            transform * Mat4::from_cols_array(&bytemuck::pod_read_unaligned(&row[..64])).as_dmat4();
        let span = indices
            .as_bytes()
            .get(start * 4..(start + count) * 4)
            .ok_or_else(fail)?;
        for word in span.chunks_exact(4) {
            let index =
                i64::from(u32::from_le_bytes(word.try_into().unwrap())) + i64::from(cmd.base);
            let at = usize::try_from(index).map_err(|_| fail())? * 28;
            let point = Vec3::from_array(bytemuck::pod_read_unaligned(
                vertices.as_bytes().get(at..at + 12).ok_or_else(fail)?,
            ));
            let projected = matrix.transform_point3(point.as_dvec3()).as_vec3();
            edges.extend_from_slice(bytemuck::bytes_of(&projected.to_array()));
            edges.extend_from_slice(bytemuck::bytes_of(&[0f32, 0., 1.]));
            edges.extend_from_slice(&tokens::OFF.to_array());
        }
    }
    let mut tags = Vec::new();
    let visible = crate::host_context::core_module(py)?.getattr("tag_edits")?.getattr("effective_visible")?;
    for tag in scene.getattr("layers")?.try_iter()? {
        let tag = tag?;
        let key: String = tag.getattr("key")?.extract()?;
        tags.push((
            key,
            tag.getattr("name")?.extract()?,
            visible.call1((scene, &tag))?.extract()?,
        ));
    }
    Ok((edges, tags))
}

/// `ingetrazo_core` from `sys.modules`: `import` runs the import hooks on
/// every call (over a microsecond under PySide's), and the model rules this
/// reaches run per placement (`tests/test_no_import_in_hot_calls.py`).
pub(crate) fn core_module(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    static CORE: pyo3::sync::PyOnceLock<Py<PyAny>> = pyo3::sync::PyOnceLock::new();
    Ok(CORE.get_or_try_init(py, || py.import("ingetrazo_core").map(|m| m.into_any().unbind()))?.bind(py).clone())
}
