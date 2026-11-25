// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use glam::DVec3;
use std::time::Instant;
#[pyfunction]
fn instanced_eligible(v: &Obj, g: &Obj) -> PyResult<bool> {
    let py = v.py();
    let module = host(py)?;
    if module.getattr("_NO_INSTANCING")?.is_truthy()?
        || opt(g, "xform")?.is_none()
        || truth(g, "billboard")?
    {
        return Ok(false);
    }
    let scene = opt(v, "scene")?;
    let active = opt(&scene, "mesh")?;
    let mesh = g.getattr("mesh")?;
    if mesh.is(&active)
        || !active.is_none()
            && mesh.is(&opt(&active, "_instance_definition")?)
            && g.getattr("uid")?
                .eq(scene.getattr("edit_group")?.getattr("uid")?)?
    {
        return Ok(false);
    }
    let paint = module.call_method1("_paint_of", (v, g))?;
    let epoch = opt(v, "_placements_epoch")?;
    let memo = if epoch.is_none() {
        None
    } else {
        let epoch = epoch.call0()?;
        let slot = opt(v, "_elig_memo")?;
        let map = if slot.is_none() || !slot.get_item(0)?.eq(&epoch)? {
            let d = PyDict::new(py);
            v.setattr("_elig_memo", (epoch, &d))?;
            d.into_any()
        } else {
            slot.get_item(1)?
        };
        let key =
            (id(&mesh), module.call_method1("_material_sig", (&paint,))?).into_pyobject(py)?;
        let hit = map.call_method1("get", (&key,))?;
        if !hit.is_none() && hit.get_item(0)?.is(&mesh) {
            return hit.get_item(1)?.extract();
        }
        Some((map, key))
    };
    let base = v.call_method1("_proto_base_chunk", (&mesh, paint))?;
    let mut ok = true;
    for name in [
        "tcol",
        "ttex",
        "back_vcol",
        "back_tex",
        "back_tcol",
        "back_ttex",
        "fvcol",
    ] {
        if get(&base, name)?.is_truthy()? {
            ok = false;
            break;
        }
    }
    if let Some((map, key)) = memo {
        map.set_item(key, (mesh, ok))?
    }
    Ok(ok)
}
#[pyfunction]
fn prepare_drawing(v: &Obj, progress: &Obj) -> PyResult<()> {
    let py = v.py();
    let groups = v.call_method0("_placements")?;
    let m = host(py)?;
    let bake = py.import("views.array_bake")?;
    let paint = m.getattr("_paint_of")?;
    let mut rows = Vec::new();
    let mut meshes = Vec::new();
    for g in groups.try_iter()? {
        let g = g?;
        let p = paint.call1((v, &g))?;
        let mesh = g.getattr("mesh")?;
        meshes.push(mesh);
        rows.push((g, p));
    }
    let notify = |at: f64| -> PyResult<()> {
        if !progress.is_none() {
            progress.call1((at, "Preparing the drawing…"))?;
        }
        Ok(())
    };
    notify(0.65)?;
    bake.call_method1("prefetch", (meshes,))?;
    let sig = py.import("built_core.materials")?.getattr("material_sig")?;
    let mut jobs = Vec::new();
    for (g, p) in &rows {
        let mesh = g.getattr("mesh")?;
        let dict = opt(&mesh, "__dict__")?;
        if !truth(g, "billboard")?
            && (dict.is_none() || !get(&dict, "_drawing_staged")?.is_truthy()?)
        {
            jobs.push((mesh, p, sig.call1((p,))?))
        }
    }
    bake.call_method1("prebake", (v, jobs))?;
    notify(0.8)?;
    let done = PyDict::new(py);
    let mut last = Instant::now();
    for (i, (g, p)) in rows.iter().enumerate() {
        if truth(g, "billboard")? {
            continue;
        }
        // A placement that cannot be prewarmed is retried by the regular draw path.
        let warm = || -> PyResult<()> {
            if !opt(g, "xform")?.is_none() {
                let mesh = g.getattr("mesh")?;
                let key = (id(&mesh), m.call_method1("_material_sig", (p,))?).into_pyobject(py)?;
                let ok = if let Some(ok) = done.get_item(&key)? {
                    ok.is_truthy()?
                } else {
                    v.call_method1("_proto_base_chunk", (mesh, p))?;
                    let ok = v.call_method1("_instanced_eligible", (g,))?.is_truthy()?;
                    done.set_item(key, ok)?;
                    ok
                };
                if ok {
                    return Ok(());
                }
            }
            v.call_method1("_group_chunk", (g,))?;
            Ok(())
        };
        if let Err(e) = warm() {
            if !e.is_instance_of::<pyo3::exceptions::PyException>(py) {
                return Err(e);
            }
        }
        if !progress.is_none() && last.elapsed().as_secs_f64() > 0.25 {
            notify(0.8 + 0.2 * (i + 1) as f64 / rows.len() as f64)?;
            last = Instant::now()
        }
    }
    v.setattr("_prebaked", py.None())?;
    notify(1.)
}
#[pyfunction]
fn placement_frame<'py>(v: &Obj<'py>, g: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let base = v.call_method1(
        "_proto_base_chunk",
        (
            g.getattr("mesh")?,
            host(py)?.call_method1("_paint_of", (v, g))?,
        ),
    )?;
    let matrix = g.getattr("xform")?;
    let key = (get(&base, "uid")?, tuple(&matrix.call_method0("data")?)?).into_pyobject(py)?;
    let frames = cache(v, "_placement_frames")?;
    let hit = frames.call_method1("get", (id(g),))?;
    if !hit.is_none() && hit.get_item(0)?.eq(&key)? {
        return hit.get_item(1);
    }
    let bbox = get(&base, "bbox")?;
    let bounds = if bbox.is_truthy()? {
        let [lo, hi] = bbox.extract::<[[f64; 3]; 2]>()?;
        let mut min = DVec3::splat(f64::INFINITY);
        let mut max = -min;
        for x in [lo[0], hi[0]] {
            for y in [lo[1], hi[1]] {
                for z in [lo[2], hi[2]] {
                    let p = xyz(&matrix.call_method1("map", (vec3(py, DVec3::new(x, y, z))?,))?)?;
                    min = min.min(p);
                    max = max.max(p)
                }
            }
        }
        ((min.x, min.y, min.z), (max.x, max.y, max.z))
            .into_pyobject(py)?
            .into_any()
    } else {
        none(py)
    };
    let (inv, ok): (Obj, bool) = matrix.call_method0("inverted")?.extract()?;
    let placements = opt(v, "_placements_memo")?;
    let n = if placements.is_none() {
        v.getattr("scene")?.getattr("groups")?.len()?
    } else {
        placements.get_item(1)?.len()?
    };
    if frames.len()? > 4 * n.max(64) {
        frames.call_method0("clear")?;
    }
    let val = (bounds, if ok { inv } else { none(py) })
        .into_pyobject(py)?
        .into_any();
    frames.set_item(id(g), (key, &val))?;
    Ok(val)
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(instanced_eligible, m)?)?;
    m.add_function(wrap_pyfunction!(prepare_drawing, m)?)?;
    m.add_function(wrap_pyfunction!(placement_frame, m)?)?;
    Ok(())
}
