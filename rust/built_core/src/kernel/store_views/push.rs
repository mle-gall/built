// SPDX-License-Identifier: GPL-3.0-or-later
//! Publish store extrusion and stitch results in command order. Paint donors
//! are read at each transfer because a face may be both a source and a target.
use super::{
    edits::{dirty, view},
    paint_classes::paint_classes,
    properties::copy_attrs,
    Obj,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
fn hypot(py: Python<'_>) -> PyResult<Obj<'_>> {
    crate::model::module(py, "core._native")?.call_method0("qt_hypot")
}
fn compensated(py: Python<'_>) -> PyResult<bool> {
    crate::model::module(py, "sys")?
        .getattr("version_info")?
        .ge((3, 12))
}
fn painted(mesh: &Obj<'_>, pairs: &Obj<'_>, reps: &Obj<'_>) -> PyResult<()> {
    for pair in pairs.try_iter()? {
        let pair = pair?;
        view(mesh, 2, &pair.get_item(0)?)?
            .setattr("attrs", copy_attrs(&reps.get_item(pair.get_item(1)?)?)?)?;
    }
    Ok(())
}
fn flipped(mesh: &Obj<'_>, keys: &Obj<'_>) -> PyResult<()> {
    for k in keys.try_iter()? {
        super::geometry::swap_orientation_paint(&view(mesh, 2, &k?)?)?;
    }
    Ok(())
}
fn donors(mesh: &Obj<'_>, pairs: &Obj<'_>, only_empty: bool) -> PyResult<()> {
    for pair in pairs.try_iter()? {
        let pair = pair?;
        let donor = view(mesh, 2, &pair.get_item(0)?)?.getattr("attrs")?;
        let dest = view(mesh, 2, &pair.get_item(1)?)?;
        if !only_empty || (donor.is_truthy()? && !dest.getattr("attrs")?.is_truthy()?) {
            dest.setattr("attrs", mesh.py().get_type::<PyDict>().call1((donor,))?)?;
        }
    }
    Ok(())
}
fn report<'py>(mesh: &Obj<'py>, op: &str, result: PyResult<Obj<'py>>) -> PyResult<Obj<'py>> {
    if let Err(e) = &result {
        if e.is_instance_of::<crate::store::py::StoreFallback>(mesh.py()) {
            crate::model::module(mesh.py(), "built_core.store_mesh")?
                .call_method1("_note_refusal", (op, e.value(mesh.py())))?;
        }
    }
    result
}
#[pyfunction]
fn rebuild_cap_plane(mesh: &Obj<'_>, origin: &Obj<'_>, normal: &Obj<'_>) -> PyResult<bool> {
    let py = mesh.py();
    let attrs = paint_classes(mesh, false)?;
    let answer = mesh.getattr("_store")?.call_method1(
        "rebuild_single_plane",
        (
            origin.call_method0("toTuple")?,
            normal.call_method0("toTuple")?,
            attrs.get_item(0)?,
            attrs.get_item(1)?,
            hypot(py)?,
            compensated(py)?,
        ),
    )?;
    let n = answer.get_item(0)?.extract::<i64>()?;
    if n != 0 {
        dirty(mesh, n)?;
        painted(mesh, &answer.get_item(1)?, &attrs.get_item(2)?)?;
    }
    Ok(n != 0)
}
#[pyfunction]
fn push_extrude(
    mesh: &Obj<'_>,
    scene: &Obj<'_>,
    face: &Obj<'_>,
    distance: &Obj<'_>,
    normal: &Obj<'_>,
    keep: &Obj<'_>,
    oriented: &Obj<'_>,
    preview: &Obj<'_>,
) -> PyResult<()> {
    let py = mesh.py();
    let cls = paint_classes(mesh, true)?;
    let store = mesh.getattr("_store")?;
    let args = (
        face.getattr("key")?,
        distance,
        normal,
        keep,
        oriented,
        preview,
        cls.get_item(0)?,
        cls.get_item(1)?,
        cls.get_item(3)?,
        cls.get_item(4)?,
        hypot(py)?,
        compensated(py)?,
    );
    let r = report(
        mesh,
        "push_extrude",
        store.call_method1("push_extrude", args),
    )?;
    dirty(mesh, r.get_item(0)?.extract()?)?;
    painted(mesh, &r.get_item(2)?, &cls.get_item(2)?)?;
    flipped(mesh, &r.get_item(3)?)?;
    let selection = scene.getattr("selection")?;
    for (field, kind) in [(4, 2), (5, 1)] {
        for k in r.get_item(field)?.try_iter()? {
            selection.call_method1("discard", (view(mesh, kind, &k?)?,))?;
        }
    }
    scene.setattr("version", scene.getattr("version")?.add(r.get_item(1)?)?)
}
#[pyfunction]
fn push_translate(
    mesh: &Obj<'_>,
    cap: &Obj<'_>,
    delta: &Obj<'_>,
    removing: &Obj<'_>,
    preview: &Obj<'_>,
) -> PyResult<()> {
    let py = mesh.py();
    let public = crate::model::module(py, "built_core.mesh")?;
    let cls = paint_classes(mesh, false)?;
    let points = PyList::empty(py);
    for p in cap.try_iter()? {
        points.append(p?.call_method0("toTuple")?)?;
    }
    let args = (
        points,
        delta,
        removing,
        preview,
        public.getattr("_CURVE_COUNTER")?,
        cls.get_item(0)?,
        cls.get_item(1)?,
        hypot(py)?,
        compensated(py)?,
    );
    let r = report(
        mesh,
        "push_translate",
        mesh.getattr("_store")?.call_method1("push_translate", args),
    )?;
    public.setattr("_CURVE_COUNTER", r.get_item(1)?)?;
    dirty(mesh, r.get_item(0)?.extract()?)?;
    donors(mesh, &r.get_item(2)?, false)?;
    painted(mesh, &r.get_item(3)?, &cls.get_item(2)?)?;
    donors(mesh, &r.get_item(4)?, true)?;
    donors(mesh, &r.get_item(5)?, false)?;
    flipped(mesh, &r.get_item(6)?)
}
fn stitched(mesh: &Obj<'_>, answer: &Obj<'_>) -> PyResult<usize> {
    let result = answer.get_item(0)?;
    let pairs = result.get_item(0)?;
    dirty(mesh, result.get_item(1)?.extract()?)?;
    donors(mesh, &pairs, true)?;
    pairs.len()
}
#[pyfunction]
fn dedupe_faces(mesh: &Obj<'_>) -> PyResult<usize> {
    stitched(mesh, &mesh.getattr("_store")?.call_method0("dedupe_faces")?)
}
#[pyfunction]
fn stitch_connect(mesh: &Obj<'_>, seedkeys: &Obj<'_>, dedupe: bool) -> PyResult<()> {
    let py = mesh.py();
    let seeds = if seedkeys.is_truthy()? {
        crate::model::module(py, "builtins")?.call_method1("sorted", (seedkeys,))?
    } else {
        PyList::empty(py).into_any()
    };
    let args = (
        seeds,
        dedupe,
        crate::model::module(py, "built_core.mesh")?.getattr("_STITCH_TOL")?,
        hypot(py)?,
    );
    stitched(
        mesh,
        &mesh
            .getattr("_store")?
            .call_method1("stitch_connect", args)?,
    )?;
    Ok(())
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(rebuild_cap_plane, m)?)?;
    m.add_function(wrap_pyfunction!(push_extrude, m)?)?;
    m.add_function(wrap_pyfunction!(push_translate, m)?)?;
    m.add_function(wrap_pyfunction!(dedupe_faces, m)?)?;
    m.add_function(wrap_pyfunction!(stitch_connect, m)?)?;
    Ok(())
}
