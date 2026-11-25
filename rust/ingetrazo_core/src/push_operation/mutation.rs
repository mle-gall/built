// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::push_command::{push_secondary_faces, push_top_out};
#[pyfunction]
pub(crate) fn paint_of<'p>(face: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let py = face.py();
    let out = PyDict::new(py);
    let attrs = face.getattr("attrs")?;
    if !attrs.is_truthy()? {
        return Ok(out);
    }
    for key in py.import("ingetrazo_core.mesh")?.getattr("PAINT_KEYS")?.try_iter()? {
        let key = key?;
        if attrs.contains(&key)? {
            out.set_item(&key, attrs.get_item(&key)?)?;
        }
    }
    if let Some(tex) = out
        .get_item("texture")?
        .filter(|t| t.is_truthy().unwrap_or(false))
    {
        let copy = py.get_type::<PyDict>().call1((tex,))?;
        copy.call_method1("pop", ("uvw", py.None()))?;
        copy.set_item("planar", true)?;
        out.set_item("texture", copy)?;
    }
    Ok(out)
}
fn face_shape<'p>(face: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = face.py();
    let canon = core(py)?.getattr("plane_cycle")?;
    let ring = |p: Obj<'p>| -> PyResult<Obj<'p>> {
        let pts = py.get_type::<PyList>().call1((p,))?;
        let a = canon.call1((&pts,))?;
        pts.call_method0("reverse")?;
        let b = canon.call1((pts,))?;
        Ok(if b.lt(&a)? { b } else { a })
    };
    let holes = face
        .getattr("holes")?
        .try_iter()?
        .map(|h| ring(h?))
        .collect::<PyResult<Vec<_>>>()?;
    (
        ring(face.getattr("vertices")?)?,
        pyo3::types::PyFrozenSet::new(py, holes)?,
        face.getattr("interior")?,
    )
        .into_bound_py_any(py)
}
#[pyfunction]
pub(super) fn mutate_many(op: &Obj, scene: &Obj) -> PyResult<()> {
    let wanted = number(op, "extrusion")?;
    op.call_method1("_mutate_or_top_out", (scene,))?;
    let state = [
        "base_face",
        "_normal",
        "_anchor",
        "_attached",
        "_prism_cap",
        "_cap_positions",
        "_limit_in",
        "_drag_pre_oriented",
        "_refused",
        "_topped_out",
        "extrusion",
    ];
    let lead = state
        .iter()
        .map(|name| op.getattr(*name))
        .collect::<PyResult<Vec<_>>>()?;
    let mesh = scene.getattr("mesh")?;
    let extras = op.getattr("_extra_faces")?;
    let shapes = extras
        .try_iter()?
        .map(|f| face_shape(&f?))
        .collect::<PyResult<Vec<_>>>()?;
    let prepare = |index: usize| -> PyResult<Option<f64>> {
        let faces = mesh.getattr("faces")?;
        let mut face = extras.get_item(index)?;
        if !faces.contains(&face)? {
            face = none(op.py());
            for f in faces.try_iter()? {
                let f = f?;
                if face_shape(&f)?.eq(&shapes[index])? {
                    face = f;
                    break;
                }
            }
        }
        if face.is_none() {
            return Ok(None);
        }
        let kw = PyDict::new(op.py());
        kw.set_item("only", (&face,))?;
        op.py()
            .import("core.orient")?
            .call_method("orient_outward", (&mesh,), Some(&kw))?;
        op.setattr("base_face", &face)?;
        op.setattr("_drag_pre_oriented", true)?;
        op.setattr("_anchor", face.call_method0("centroid")?)?;
        op.setattr(
            "_normal",
            op.call_method1("_outward_normal", (&face, &mesh))?,
        )?;
        let (attached, cap): (bool, bool) = classify_base(op, scene)?.extract()?;
        op.setattr("_attached", attached)?;
        op.setattr("_prism_cap", cap)?;
        op.setattr("_cap_positions", cap_loop_positions(&face)?)?;
        op.setattr("extrusion", wanted)?;
        compute_inward_limit(op, scene)?;
        clamp_extrusion(op, None)?;
        Ok(Some(number(op, "extrusion")?))
    };
    let apply = || {
        op.call_method1("_mutate_or_top_out", (scene,))?;
        Ok((flag(op, "_refused")?, flag(op, "_topped_out")?))
    };
    let result = push_secondary_faces(
        &mesh,
        shapes.len(),
        wanted,
        MIN_EXTRUDE,
        flag(op, "_refused")?,
        prepare,
        apply,
    );
    for (k, v) in state.iter().zip(lead) {
        op.setattr(*k, v)?;
    }
    let (refused, missed) = result?;
    op.setattr("_refused", refused)?;
    op.setattr("_multi_missed", missed)?;
    op.setattr("_topped_out", false)?;
    op.setattr("extrusion", wanted)
}
#[pyfunction]
pub(super) fn mutate_or_top_out(op: &Obj, scene: &Obj) -> PyResult<()> {
    let attempt = |distance| {
        op.setattr("extrusion", distance)?;
        op.call_method1("_mutate", (scene,))?;
        if !op.getattr("_store_refusal")?.is_none() {
            return Ok(None);
        }
        Ok(Some((number(op, "extrusion")?, flag(op, "_refused")?)))
    };
    let (distance, refused, partial) = push_top_out(
        &target_mesh(op, scene)?,
        attempt,
        number(op, "extrusion")?,
        MIN_EXTRUDE,
        op.getattr("_CLAMP_TRIES")?.extract()?,
    )?;
    op.setattr("extrusion", distance)?;
    op.setattr("_refused", refused)?;
    if partial {
        op.setattr("_topped_out", true)?;
    }
    Ok(())
}
#[pyfunction(signature=(op,scene,preview=false))]
pub(super) fn mutate(op: &Obj, scene: &Obj, preview: bool) -> PyResult<()> {
    let py = op.py();
    let store = py.import("ingetrazo_core.store_mesh")?;
    op.setattr("_store_refusal", py.None())?;
    let mesh = target_mesh(op, scene)?;
    let snapshot = if mesh.get_type().is(&store.getattr("StoreMesh")?) {
        Some(mesh.call_method0("capture_state")?)
    } else {
        None
    };
    let strict = store.call_method0("strict")?;
    strict.call_method0("__enter__")?;
    let result = op.call_method1("_mutate_guarded", (scene, preview));
    match &result {
        Err(err) => strict.call_method1(
            "__exit__",
            (err.get_type(py), err.value(py), err.traceback(py)),
        )?,
        Ok(_) => strict.call_method1("__exit__", (py.None(), py.None(), py.None()))?,
    };
    if let Err(err) = result {
        let refusal = store.getattr("StoreRefusal")?;
        let refused = err.matches(py, &refusal)?;
        if let Some(before) = snapshot.filter(|_| refused) {
            mesh.call_method1("restore_state", (before,))?;
            op.setattr("_refused", true)?;
            let reason = err.value(py).str()?.to_string();
            op.setattr("_store_refusal", &reason)?;
            super::refusal::write(&mesh, op, &reason)?;
        } else {
            return Err(err);
        }
    }
    Ok(())
}
#[pyfunction(signature=(op,scene,preview=false))]
pub(super) fn mutate_guarded(op: &Obj, scene: &Obj, preview: bool) -> PyResult<()> {
    let py = op.py();
    op.setattr("_refused", false)?;
    let mesh = target_mesh(op, scene)?;
    let before = mesh.getattr("_store")?.call_method0("snapshot")?;
    let base = op.getattr("base_face")?;
    let mut tag = None;
    let paint = if base.is_none() {
        PyDict::new(py)
    } else {
        let v = base.getattr("attrs")?.call_method1("get", ("ifc",))?;
        if v.is_truthy()? {
            tag = Some(py.get_type::<PyDict>().call1((v,))?);
        }
        paint_of(&base)?
    };
    let refused = crate::push_guard::guarded_push(
        &mesh,
        || {
            op.call_method1("_mutate_inner", (scene, preview))
                .map(|_| ())
        },
        number(op, "extrusion")?.abs() >= MIN_EXTRUDE,
        &py.import("core.orient")?.getattr("is_closed")?,
        &py.import("core.topology")?.getattr("_mesh_is_flat")?,
        &py.import("ingetrazo_core.store_mesh")?.getattr("_store_of")?,
    )?;
    op.setattr("_refused", refused)?;
    if refused {
        return Ok(());
    }
    let n = op.getattr("_normal")?;
    let shift = if n.is_none() {
        n
    } else {
        n.mul(number(op, "extrusion")?)?.call_method0("toTuple")?
    };
    let key = if base.is_none() {
        base
    } else {
        base.getattr("key")?
    };
    let fresh = mesh.call_method1(
        "finish_push",
        (
            before,
            key,
            shift,
            op.getattr("_CURVE_FACET_COS")?,
            tag.is_some() || !paint.is_empty(),
        ),
    )?;
    for f in fresh.try_iter()? {
        let attrs = f?.getattr("attrs")?;
        if let Some(tag) = &tag {
            if !attrs.call_method1("get", ("ifc",))?.is_truthy()? {
                attrs.set_item("ifc", tag.call_method0("copy")?)?;
            }
        }
        if !paint.is_empty() {
            let mut has = false;
            for k in py.import("ingetrazo_core.mesh")?.getattr("PAINT_KEYS")?.try_iter()? {
                if attrs.contains(k?)? {
                    has = true;
                    break;
                }
            }
            if !has {
                attrs.call_method1("update", (&paint,))?;
            }
        }
    }
    Ok(())
}
#[pyfunction(signature=(op,scene,preview=false))]
pub(super) fn mutate_inner(op: &Obj, scene: &Obj, preview: bool) -> PyResult<()> {
    let py = op.py();
    let scene = target_scene(op, scene)?;
    let mesh = scene.getattr("mesh")?;
    let face = op.getattr("base_face")?;
    let d = number(op, "extrusion")?;
    if flag(op, "_prism_cap")?
        && !flag(op, "_keep_base")?
        && !op
            .call_method1("_cap_passes_a_wall_vertex", (&scene, &face, d))?
            .is_truthy()?
    {
        let normal = crate::math::xyz(&or_face(op, "_normal", &face, "normal")?)?;
        let kw = PyDict::new(py);
        kw.set_item("removing", d < 0.)?;
        kw.set_item("preview", preview)?;
        let step = (normal * d).to_array();
        mesh.call_method(
            "push_translate",
            (op.getattr("_cap_positions")?, (step[0], step[1], step[2])),
            Some(&kw),
        )?;
        bump(&scene)
    } else {
        let n = op.getattr("_normal")?;
        let n = if n.is_none() {
            n
        } else {
            n.call_method0("toTuple")?
        };
        mesh.call_method1(
            "push_extrude",
            (
                &scene,
                face,
                d,
                n,
                flag(op, "_keep_base")?,
                flag(op, "_drag_pre_oriented")?,
                preview,
            ),
        )?;
        Ok(())
    }
}
#[pyfunction]
pub(super) fn passes_wall(op: &Obj, scene: &Obj, face: &Obj, d: f64) -> PyResult<bool> {
    if d >= 0. {
        return Ok(false);
    }
    scans(op.py())?
        .call_method1(
            "cap_passes_wall",
            (
                scene.getattr("mesh")?,
                face,
                op.getattr("_cap_positions")?,
                or_face(op, "_normal", face, "normal")?,
                d,
            ),
        )?
        .extract()
}
