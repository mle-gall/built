// SPDX-License-Identifier: GPL-3.0-or-later
//! Selection and mesh updates shared by native history steps.
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet, PyTuple},
};
type Obj<'py> = Bound<'py, PyAny>;
#[pyfunction]
pub(crate) fn dirty_group_chunks(scene: &Obj<'_>) -> PyResult<()> {
    let py = scene.py();
    scene.getattr("mesh")?.call_method0("bump_revision")?;
    let placements = py.import("built_core.group")?.getattr("iter_placements")?;
    if let Some(groups) = scene.getattr_opt("groups")? {
        for g in groups.try_iter()? {
            for pair in placements.call1((g?,))?.try_iter()? {
                let m = pair?.get_item(0)?.getattr("mesh")?;
                m.setattr("_chunk_dirty", true)?;
                m.setattr("_attrs_dirty", true)?;
            }
        }
    }
    Ok(())
}
pub(crate) fn dirty_placements(groups: &Obj<'_>) -> PyResult<()> {
    let items = groups
        .py()
        .import("built_core")?
        .call_method1("purge_iter_groups", (groups,))?;
    for g in items.try_iter()? {
        if let Some(m) = g?.getattr_opt("mesh")?.filter(|m| !m.is_none()) {
            m.call_method0("invalidate_placement_chunks")?;
        }
    }
    Ok(())
}
pub(crate) fn unselect(scene: &Obj<'_>, items: &Obj<'_>) -> PyResult<()> {
    let py = scene.py();
    let sel = scene.getattr("selection")?;
    if !sel.is_truthy()? {
        return Ok(());
    }
    if sel.len()?.saturating_mul(4) < items.len()? {
        let identity = py.import("built_core.mesh")?.getattr("element_identity")?;
        let drop = PySet::empty(py)?;
        for e in items.try_iter()? {
            drop.add(identity.call1((e?,))?)?;
        }
        let mut gone = Vec::new();
        for e in sel.try_iter()? {
            let e = e?;
            if drop.contains(identity.call1((&e,))?)? {
                gone.push(e)
            }
        }
        for e in gone {
            sel.call_method1("discard", (e,))?;
        }
    } else {
        for e in items.try_iter()? {
            sel.call_method1("discard", (e?,))?;
        }
    }
    Ok(())
}
pub(crate) fn reverse_faces<'py>(faces: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let py = faces.py();
    let batches = PyDict::new(py);
    for f in faces.try_iter()? {
        let f = f?;
        let m = f.getattr("_m")?;
        let list = match batches.get_item(&m)? {
            Some(v) => v.cast_into::<PyList>()?,
            None => {
                let list = PyList::empty(py);
                batches.set_item(m, &list)?;
                list
            }
        };
        list.append(f.getattr("key")?)?;
    }
    for (m, keys) in batches.iter() {
        m.getattr("_store")?
            .call_method1("reverse_faces", (keys,))?;
    }
    Ok(batches)
}
pub(crate) fn member_placement<'py>(scene: &Obj<'py>, g: &Obj<'py>) -> PyResult<Obj<'py>> {
    let pose = g.getattr("xform")?;
    if pose.is_none()
        && scene
            .getattr("mesh")?
            .getattr_opt("_instance_definition")?
            .is_some_and(|v| !v.is_none())
    {
        let parent = scene.getattr("edit_group")?;
        if !parent.is_none() && parent.getattr("children")?.contains(g)? {
            g.setattr("component", false)?;
            return scene.py().import("built_core")?.call_method0("Matrix4");
        }
    }
    Ok(pose)
}
pub(crate) fn edit_local_transform<'py>(
    scene: &Obj<'py>,
    g: &Obj<'py>,
    transform: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let parent = scene.getattr("edit_group")?;
    if !parent.is_none() && parent.getattr("children")?.contains(g)? {
        let mesh = scene.getattr("mesh")?;
        if mesh
            .getattr("__dict__")?
            .cast::<PyDict>()?
            .get_item("_instance_definition")?
            .is_some_and(|v| !v.is_none())
        {
            let matrix = mesh.getattr("_store")?.call_method0("tool_placement")?;
            let inv = matrix.call_method0("inverted")?;
            if inv.get_item(1)?.is_truthy()? {
                return inv.get_item(0)?.mul(transform)?.mul(matrix);
            }
        }
    }
    Ok(transform.clone())
}
pub(crate) fn discard_removed_selection(scene: &Obj<'_>) -> PyResult<()> {
    let py = scene.py();
    let sel = scene.getattr("selection")?;
    for e in py.get_type::<PyTuple>().call1((&sel,))?.try_iter()? {
        let e = e?;
        if let Some(owner) = e.getattr_opt("_m")?.filter(|m| !m.is_none()) {
            if let Some(store) = owner.getattr_opt("_store")?.filter(|s| !s.is_none()) {
                if !store
                    .call_method1("is_live", (e.getattr("key")?,))?
                    .is_truthy()?
                {
                    sel.call_method1("discard", (e,))?;
                }
            }
        }
    }
    Ok(())
}
pub(crate) fn drop_dead_elements(scene: &Obj<'_>, mesh: &Obj<'_>) -> PyResult<()> {
    let py = scene.py();
    let sel = py
        .import("built_core")?
        .call_method1("ElementSet", (scene.getattr("selection")?,))?;
    let classes = py.import("built_core.mesh")?;
    let kinds = (classes.getattr("Face")?, classes.getattr("Edge")?).into_pyobject(py)?;
    let store = mesh.getattr("_store")?;
    let keep = PySet::empty(py)?;
    for e in sel.try_iter()? {
        let e = e?;
        if !e.is_instance(&kinds)?
            || store
                .call_method1("is_live", (e.getattr("key")?,))?
                .is_truthy()?
        {
            keep.add(e)?;
        }
    }
    scene.setattr("selection", keep)
}
#[pyfunction]
#[pyo3(signature=(mesh,seedkeys,new_faces=None,coplanar_merge=true,dedupe=true))]
pub(crate) fn run_stitch(
    mesh: &Obj<'_>,
    seedkeys: &Obj<'_>,
    new_faces: Option<&Obj<'_>>,
    coplanar_merge: bool,
    dedupe: bool,
) -> PyResult<()> {
    mesh.call_method1("stitch_connect", (seedkeys, dedupe))?;
    if !coplanar_merge {
        return Ok(());
    }
    let py = mesh.py();
    let module = py.import("built_core.store_mesh")?;
    let face = module.getattr("FaceView")?;
    let store = mesh.getattr("_store")?;
    let hypot = module.call_method0("_hypot")?;
    let mut start = 0usize;
    let seeds = if seedkeys.is_truthy()? {
        py.get_type::<PyList>().call1((seedkeys,))?
    } else {
        PyList::empty(py).into_any()
    };
    loop {
        let fresh = if let Some(new) = new_faces {
            let keys = PySet::empty(py)?;
            for f in new.try_iter()? {
                keys.add(f?.getattr("key")?)?;
            }
            Some(keys)
        } else {
            None
        };
        let result = store.call_method1("stitch_region", (&seeds, fresh, start, &hypot))?;
        if result.is_none() {
            break;
        }
        start = result.get_item(0)?.extract()?;
        let component = PySet::empty(py)?;
        for k in result.get_item(1)?.try_iter()? {
            component.add(face.call1((mesh, k?))?)?;
        }
        let region = mesh.call_method1("dissolve_coplanar_region", (&component,))?;
        if !region.is_none() {
            if let Some(new) = new_faces {
                new.call_method1("difference_update", (component,))?;
                new.call_method1("add", (region,))?;
            }
            start = 0;
        }
    }
    Ok(())
}
#[pyfunction]
#[pyo3(signature=(xform,tol=1e-6))]
pub(crate) fn placement_is_identity(xform: &Obj<'_>, tol: f64) -> PyResult<bool> {
    if xform.is_none() {
        return Ok(true);
    }
    let data = xform.call_method0("data")?;
    for (i, v) in data.try_iter()?.take(16).enumerate() {
        let expected = if i % 5 == 0 { 1. } else { 0. };
        if !((v?.extract::<f64>()? - expected).abs() <= tol) {
            return Ok(false);
        }
    }
    Ok(true)
}
#[pyfunction]
pub(crate) fn placement_rotation_deg(xform: &Obj<'_>) -> PyResult<f64> {
    let col = |i| -> PyResult<[f64; 3]> {
        let c = xform.call_method1("column", (i,))?;
        Ok([
            c.call_method0("x")?.extract()?,
            c.call_method0("y")?.extract()?,
            c.call_method0("z")?.extract()?,
        ])
    };
    let x = col(0)?;
    let y = col(1)?;
    let z = col(2)?;
    if z[0].abs() > 1e-6
        || z[1].abs() > 1e-6
        || (z[2] - 1.).abs() > 1e-6
        || x[2].abs() > 1e-6
        || y[2].abs() > 1e-6
    {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "the model is tilted (its Z axis is not vertical)",
        ));
    }
    if (x[0].hypot(x[1]) - 1.).abs() > 1e-4 || (y[0].hypot(y[1]) - 1.).abs() > 1e-4 {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "the model is scaled",
        ));
    }
    Ok(x[1].atan2(x[0]).to_degrees())
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(dirty_group_chunks, m)?)?;
    m.add_function(wrap_pyfunction!(super::tag_steps::is_hidden, m)?)?;
    m.add_function(wrap_pyfunction!(super::tag_steps::set_hidden, m)?)?;
    m.add_function(wrap_pyfunction!(run_stitch, m)?)?;
    m.add_function(wrap_pyfunction!(placement_is_identity, m)?)?;
    m.add_function(wrap_pyfunction!(placement_rotation_deg, m)?)?;
    Ok(())
}
