// SPDX-License-Identifier: GPL-3.0-or-later
//! Admission of a push, from the entry solid to the reached opposite wall.
use crate::{math::xyz, store::commands::cell};

use pyo3::prelude::*;

type Shape = (usize, usize, Vec<[u64; 3]>);

fn object_shape(mesh: &Bound<'_, PyAny>) -> PyResult<Shape> {
    let mut cells = Vec::new();
    for v in mesh.getattr("vertices")?.try_iter()? {
        cells.push(cell(xyz(&v?.getattr("position")?)?.to_array()));
    }
    cells.sort_unstable();
    Ok((
        mesh.getattr("faces")?.len()?,
        mesh.getattr("edges")?.len()?,
        cells,
    ))
}

fn snapshot_shape(snap: &Bound<'_, PyAny>) -> PyResult<Shape> {
    let (_, edges, faces): (usize, usize, usize) = snap.call_method0("counts")?.extract()?;
    let mut cells = Vec::new();
    for key in snap.call_method1("keys", ("vertex",))?.try_iter()? {
        cells.push(cell(
            snap.call_method1("vertex_position", (key?,))?.extract()?,
        ));
    }
    cells.sort_unstable();
    Ok((faces, edges, cells))
}

/// A closed volume may flatten, but may not open. Save through the facade:
/// IFC data and detached Python handles must survive restoration as well.
pub(crate) fn guarded_push(
    mesh: &Bound<'_, PyAny>,
    apply: impl FnOnce() -> PyResult<()>,
    check_motion: bool,
    closed: &Bound<'_, PyAny>,
    flat: &Bound<'_, PyAny>,
    store_of: &Bound<'_, PyAny>,
) -> PyResult<bool> {
    if !closed.call1((mesh,))?.is_truthy()? || flat.call1((mesh,))?.is_truthy()? {
        apply()?;
        return Ok(false);
    }
    let before = mesh.call_method0("capture_state")?;
    let snap = before.getattr_opt("snap")?;
    let shape = if check_motion && snap.is_none() {
        Some(object_shape(mesh)?)
    } else {
        None
    };
    apply()?;
    if !closed.call1((mesh,))?.is_truthy()? && !flat.call1((mesh,))?.is_truthy()? {
        mesh.call_method1("restore_state", (before,))?;
        return Ok(true);
    }
    if !check_motion {
        return Ok(false);
    }
    if let Some(snap) = snap {
        let store = store_of.call1((mesh,))?;
        if !store.is_none() {
            return store.call_method1("push_shape_matches", (snap,))?.extract();
        }
        return Ok(snapshot_shape(&snap)? == object_shape(mesh)?);
    }
    Ok(shape == Some(object_shape(mesh)?))
}
