// SPDX-License-Identifier: GPL-3.0-or-later
//! Bake a materialized prototype into draw runs, pick triangles, and soft
//! edge silhouette records. Deferred meshes retain their array-bake provider.
use super::*;
use numpy::{IntoPyArray, PyArrayMethods};
use pyo3::types::PySlice;
fn as_bytes<'py>(py: Python<'py>, rows: &[f32]) -> Bound<'py, PyBytes> {
    PyBytes::new(py, bytemuck::cast_slice(rows))
}
fn matrix<'py>(py: Python<'py>, rows: Vec<f64>, columns: usize) -> PyResult<Obj<'py>> {
    let n = rows.len() / columns;
    Ok(rows.into_pyarray(py).reshape([n, columns])?.into_any())
}
fn append<'py>(d: &Bound<'py, PyDict>, key: &Obj<'py>, part: &Obj<'py>) -> PyResult<()> {
    let list = match d.get_item(key)? {
        Some(v) => v,
        None => {
            let l = PyList::empty(d.py());
            d.set_item(key, &l)?;
            l.into_any()
        }
    };
    list.call_method1("append", (part,))?;
    Ok(())
}
fn joined<'py>(d: &Bound<'py, PyDict>) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(d.py());
    let empty = PyBytes::new(d.py(), b"");
    for (k, v) in d.iter() {
        out.set_item(k, empty.call_method1("join", (v,))?)?
    }
    Ok(out)
}
fn texture_runs<'py>(
    v: &Obj<'py>,
    face: &Obj<'py>,
    tex: &Obj<'py>,
    bucket: &Bound<'py, PyDict>,
    opacity: Option<(&Obj<'py>, i32)>,
) -> PyResult<()> {
    let tmp = PyDict::new(v.py());
    v.call_method1("_append_textured_face", (&tmp, face, tex))?;
    for (k, a) in tmp.iter() {
        let key = if let Some((o, c)) = opacity {
            (k, o, c).into_pyobject(v.py())?.into_any()
        } else {
            k
        };
        append(bucket, &key, &a.call_method0("tobytes")?)?
    }
    Ok(())
}
fn props<'py>(
    f: &Obj<'py>,
    element: &Obj<'py>,
    built: &Obj<'py>,
    memo: &Bound<'py, PyDict>,
) -> PyResult<([f64; 3], [f64; 3])> {
    let key = element.call1((f,))?;
    if let Some(r) = memo.get_item(&key)? {
        return r.extract();
    }
    let geom = built.call_method1("get", (&key,))?;
    if !geom.is_none() {
        let r = geom.get_item(7)?;
        if !r.is_none() {
            memo.set_item(key, &r)?;
            return r.extract();
        }
    }
    let n = if geom.is_none() {
        f.call_method0("normal")?
    } else {
        geom.get_item(1)?
    };
    let result = (
        xyz(&n)?.to_array(),
        xyz(&f.call_method0("centroid")?)?.to_array(),
    );
    memo.set_item(key, result)?;
    Ok(result)
}
fn bake<'py>(
    v: &Obj<'py>,
    mesh: &Obj<'py>,
    paint: &Obj<'py>,
    fp: &Obj<'py>,
    vkey: &Obj<'py>,
    msig: &Obj<'py>,
    editing: bool,
) -> PyResult<Obj<'py>> {
    let py = v.py();
    let h = host(py)?;
    let faces = mesh.getattr("faces")?;
    let edges = mesh.getattr("edges")?;
    let verts = mesh.getattr("vertices")?;
    let md = mesh.getattr("__dict__")?;
    let mut memo = none(py);
    if editing {
        memo = get(&md, "_face_geom_memo")?;
        if memo.is_none() || memo.len()? > 2 * faces.len()? + 64 {
            memo = PyDict::new(py).into_any();
            md.set_item("_face_geom_memo", &memo)?
        }
    }
    let built = if memo.is_none() {
        PyDict::new(py).into_any()
    } else {
        memo.clone()
    };
    let geom_of = h.getattr("_face_geometry")?;
    let triangulate = py.import("core.triangulate")?.getattr("triangulate")?;
    let effective = h.getattr("effective_attrs")?;
    let back_default = py.import("core.materials")?.getattr("back_is_default")?;
    let curved = h.call_method1("_curved_faces", (mesh,))?;
    let element = h.getattr("element_identity")?;
    let smoother = py
        .import("formats.meshexport")?
        .call_method0("SmoothNormals")?;
    let out = PyDict::new(py);
    let face_handles = PyList::empty(py);
    let buckets = PyDict::new(py);
    for name in [
        "by_texture",
        "tcol",
        "ttex",
        "back_tex",
        "back_tcol",
        "back_ttex",
    ] {
        buckets.set_item(name, PyDict::new(py))?
    }
    let bucket = |n: &str| {
        buckets
            .get_item(n)?
            .unwrap()
            .cast_into::<PyDict>()
            .map_err(PyErr::from)
    };
    let mut positions = Vec::new();
    let mut edge1 = Vec::new();
    let mut edge2 = Vec::new();
    let mut tri_ent = Vec::new();
    let mut areas = Vec::new();
    let mut rgb = Vec::new();
    let mut soft_normals = Vec::new();
    let mut front = Vec::new();
    let mut backs = Vec::<u8>::new();
    let mut default_backs = Vec::new();
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = -lo;
    let mut has_bounds = false;
    for f in faces.try_iter()? {
        let f = f?;
        let attrs = f.getattr("attrs")?;
        if get(&attrs, "hidden")?.is_truthy()? {
            continue;
        }
        let attrs = effective.call1((attrs, paint))?;
        let index = face_handles.len() as i64;
        face_handles.append(&f)?;
        let g = geom_of.call1((&f, &memo, &triangulate))?;
        if memo.is_none() {
            built.set_item(element.call1((&f,))?, &g)?
        }
        let normal = g.get_item(1)?;
        let tris = g.get_item(3)?;
        let raw = tris.extract::<Vec<[[f64; 3]; 3]>>()?;
        areas.push(g.get_item(2)?.extract::<f64>()?);
        let back = get(&attrs, "back")?;
        let mut cull = 0;
        if back.is_instance_of::<PyDict>() {
            let (kind, payload): (Option<String>, Obj) =
                v.call_method1("_bucket_back_face", (&f, back))?.extract()?;
            match kind.as_deref() {
                Some("bvcol") => backs.extend_from_slice(payload.cast::<PyBytes>()?.as_bytes()),
                Some("btex" | "btcol" | "bttex") => {
                    let name = match kind.as_deref() {
                        Some("btex") => "back_tex",
                        Some("btcol") => "back_tcol",
                        _ => "back_ttex",
                    };
                    cull = i32::from(name != "back_tex");
                    append(&bucket(name)?, &payload.get_item(0)?, &payload.get_item(1)?)?
                }
                _ => (),
            }
        }
        let tex = get(&attrs, "texture")?;
        let opacity: f64 = attrs.call_method1("get", ("opacity", 1.))?.extract()?;
        if !tex.is_none() && get(&tex, "path")?.is_truthy()? {
            if opacity < 0.999 {
                let op = rounded(py, opacity, 3)?;
                texture_runs(v, &f, &tex, &bucket("ttex")?, Some((&op, cull)))?
            } else {
                texture_runs(v, &f, &tex, &bucket("by_texture")?, None)?
            }
        } else {
            let color = get(&attrs, "color")?;
            let base = if color.is_none() {
                v.getattr("DEFAULT_FACE_COLOR")?
            } else {
                tuple(&color)?
            };
            let shade = g.get_item(5)?;
            let mut col = shade.call_method1("get", (&base,))?;
            if col.is_none() {
                col = v.call_method1("_shaded_color", (&base, &normal))?;
                shade.set_item(&base, &col)?
            }
            let smooth = curved.contains(&f)?;
            let mut scratch = Vec::new();
            let dest = if opacity < 0.999 {
                &mut scratch
            } else if cull != 0 {
                &mut front
            } else {
                &mut rgb
            };
            if smooth {
                let rows = h
                    .call_method1(
                        "_smooth_vcol",
                        (&f, base, col, &smoother, &tris, v.getattr("_LIGHT")?),
                    )?
                    .extract::<Vec<f32>>()?;
                dest.extend(rows)
            } else {
                let channels: [f32; 3] = col.extract()?;
                for tri in &raw {
                    for point in tri {
                        dest.extend(point.map(|x| x as f32));
                        dest.extend(channels)
                    }
                }
            }
            if opacity < 0.999 {
                append(
                    &bucket("tcol")?,
                    &(rounded(py, opacity, 3)?, cull)
                        .into_pyobject(py)?
                        .into_any(),
                    &as_bytes(py, &scratch).into_any(),
                )?
            } else if cull == 0 && curved.is_truthy()? {
                let normals = if smooth {
                    Some(smoother.call_method1("corners", (&f,))?)
                } else {
                    None
                };
                for t in &raw {
                    for p in t {
                        let n = if let Some(ns) = &normals {
                            ns.call_method1("get", (coords(DVec3::from_array(*p)), &normal))?
                        } else {
                            normal.clone()
                        };
                        soft_normals.extend(xyz(&n)?.as_vec3().to_array())
                    }
                }
            }
        }
        let dback = !raw.is_empty() && back_default.call1((&attrs,))?.is_truthy()?;
        for tri in raw {
            let [a, b, c] = tri.map(DVec3::from_array);
            let e1 = b - a;
            let e2 = c - a;
            positions.extend(a.to_array());
            edge1.extend(e1.to_array());
            edge2.extend(e2.to_array());
            tri_ent.push(index);
            for p in [a, a + e1, a + e2] {
                lo = lo.min(p);
                hi = hi.max(p);
                has_bounds = true
            }
            if dback {
                for p in [a, b, c] {
                    default_backs.extend(p.as_vec3().to_array())
                }
            }
        }
    }
    let mut hard = Vec::new();
    let mut soft = Vec::new();
    let mut singles = Vec::new();
    let mut silhouette: [Vec<f64>; 4] = Default::default();
    let hard_layers = PyList::empty(py);
    let soft_layers = PyList::empty(py);
    let prop_memo = PyDict::new(py);
    for e in edges.try_iter()? {
        let e = e?;
        if truth(&e, "hidden")? {
            continue;
        }
        let soft_edge = truth(&e, "soft")?;
        if soft_edge {
            let ff = e.getattr("faces")?;
            if !(1..=2).contains(&ff.len()?) {
                continue;
            }
            let a = props(&ff.get_item(0)?, &element, &built, &prop_memo)?;
            let b = if ff.len()? == 2 {
                props(&ff.get_item(1)?, &element, &built, &prop_memo)?
            } else {
                a
            };
            for (col, data) in silhouette.iter_mut().zip([a.0, a.1, b.0, b.1]) {
                col.extend(data)
            }
            singles.push(ff.len()? == 1);
            soft_layers.append(opt(&e, "layer")?)?
        } else {
            hard_layers.append(opt(&e, "layer")?)?
        }
        for name in ["a", "b"] {
            let p = xyz(&e.getattr(name)?)?.as_vec3();
            if soft_edge {
                soft.extend(p.to_array())
            } else {
                hard.extend(p.to_array());
                let q = p.as_dvec3();
                lo = lo.min(q);
                hi = hi.max(q);
                has_bounds = true
            }
        }
    }
    for (k, data) in [
        ("edges", &hard),
        ("vcol", &rgb),
        ("smooth_vcol", &soft_normals),
        ("fvcol", &front),
        ("dback", &default_backs),
    ] {
        out.set_item(k, as_bytes(py, data))?
    }
    out.set_item("back_vcol", PyBytes::new(py, &backs))?;
    for (k, parts) in buckets.iter() {
        out.set_item(k, joined(parts.cast::<PyDict>()?)?)?
    }
    out.set_item(
        "soft_pts",
        if soft.is_empty() {
            none(py)
        } else {
            let n = soft.len() / 6;
            soft.into_pyarray(py).reshape([n, 6])?.into_any()
        },
    )?;
    for (k, rows) in ["soft_n0", "soft_c0", "soft_n1", "soft_c1"]
        .into_iter()
        .zip(silhouette)
    {
        out.set_item(
            k,
            if rows.is_empty() {
                rows.into_pyarray(py).into_any()
            } else {
                matrix(py, rows, 3)?
            },
        )?
    }
    out.set_item("edge_layer", hard_layers)?;
    out.set_item("soft_layer", soft_layers)?;
    out.set_item("soft_single", singles.into_pyarray(py))?;
    for (k, rows) in [("v0", positions), ("e1", edge1), ("e2", edge2)] {
        out.set_item(
            k,
            if rows.is_empty() {
                none(py)
            } else {
                matrix(py, rows, 3)?
            },
        )?
    }
    out.set_item(
        "tri_ent",
        if tri_ent.is_empty() {
            none(py)
        } else {
            tri_ent.into_pyarray(py).into_any()
        },
    )?;
    out.set_item(
        "bbox",
        if has_bounds {
            (coords(lo), coords(hi)).into_pyobject(py)?.into_any()
        } else {
            none(py)
        },
    )?;
    out.set_item("obb", py.None())?;
    out.set_item("areas", areas.into_pyarray(py))?;
    out.set_item("faces", face_handles)?;
    out.set_item("fp", fp)?;
    out.set_item("vkey", vkey)?;
    out.set_item("msig", msig)?;
    out.set_item("serial", opt(mesh, "_mut_serial")?)?;
    out.set_item("uid", uid(py)?)?;
    out.set_item("rev", 0)?;
    let nv = verts.len()?;
    out.set_item("nv", nv)?;
    out.set_item("ne", edges.len()?)?;
    out.set_item("nf", faces.len()?)?;
    out.set_item("coordsum", super::fingerprint::coordsum(&verts)?)?;
    let samples = PyList::empty(py);
    if nv > 0 {
        let mut prev = None;
        for k in 0..32 {
            let i = k * (nv - 1) / 31;
            if Some(i) != prev {
                samples.append((i, coords(xyz(&verts.get_item(i)?.getattr("position")?)?)))?;
                prev = Some(i)
            }
        }
    }
    out.set_item("samples", samples)?;
    Ok(out.into_any())
}

#[pyfunction]
fn group_chunk<'py>(v: &Obj<'py>, group: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    if !opt(group, "xform")?.is_none() {
        return v.call_method1("_instance_chunk", (group,));
    }
    let h = host(py)?;
    let cache = memo(v, "_group_chunks")?;
    let mut entry = cache.call_method1("get", (id(group),))?;
    let scene = v.getattr("scene")?;
    let vkey = (scene.getattr("version")?, id(&scene.getattr("mesh")?))
        .into_pyobject(py)?
        .into_any();
    let mesh = group.getattr("mesh")?;
    let paint = h.call_method1("_paint_of", (v, group))?;
    let msig = h.call_method1("_material_sig", (&paint,))?;
    if !entry.is_none() && (!get(&entry, "msig")?.eq(&msig)? || !get(&entry, "mesh")?.is(&mesh)) {
        entry = none(py)
    }
    let flag_gen = mesh
        .getattr_opt("_flags_gen")?
        .unwrap_or(0i32.into_pyobject(py)?.into_any());
    let behind = !entry.is_none() && !entry.call_method1("get", ("flags_gen", 0))?.eq(flag_gen)?;
    let arrays = h.call_method0("_array_bake")?;
    let painted_fp = |fp: Obj<'py>| -> PyResult<Obj<'py>> {
        if msig.is_none() {
            Ok(fp)
        } else {
            fp.add((("paint",).into_pyobject(py)?.add(&msig)?,))
        }
    };
    if arrays.call_method1("can_bake", (&mesh,))?.is_truthy()? {
        if entry.is_none() {
            entry = arrays.call_method1("bake", (v, &mesh, &paint, &msig, &vkey, uid(py)?))?;
            if !msig.is_none() {
                entry.set_item("fp", painted_fp(entry.get_item("fp")?)?)?
            }
            cache.set_item(id(group), &entry)?
        }
        entry.set_item("vkey", vkey)?;
        return Ok(entry);
    }
    let winding = mesh
        .getattr_opt("_winding_rev")?
        .unwrap_or(0i32.into_pyobject(py)?.into_any());
    if !entry.is_none()
        && !h
            .call_method1("_fp_winding", (entry.get_item("fp")?,))?
            .eq(&winding)?
    {
        entry = none(py)
    }
    let serial = opt(&mesh, "_mut_serial")?;
    if !entry.is_none() {
        if get(&entry, "vkey")?.eq(&vkey)? {
            return Ok(entry);
        }
        if !behind && !truth(&mesh, "_attrs_dirty")? {
            let old = get(&entry, "serial")?;
            if !old.is_none() && old.eq(&serial)? && super::fingerprint::counts(&entry, &mesh)? {
                entry.set_item("vkey", vkey)?;
                return Ok(entry);
            }
            let delta = v.call_method1("_translation_probe", (&entry, &mesh))?;
            if !delta.is_none() {
                v.call_method1("_shift_chunk", (&entry, delta, &mesh))?;
                entry.set_item("vkey", vkey)?;
                entry.set_item("rev", entry.get_item("rev")?.add(1)?)?;
                entry.set_item("serial", serial)?;
                return Ok(entry);
            }
        }
        if !behind
            && mesh
                .getattr_opt("_chunk_dirty")?
                .is_some_and(|b| matches!(b.is_truthy(), Ok(false)))
            && super::fingerprint::counts(&entry, &mesh)?
            && v.call_method1("_samples_match", (&entry, &mesh))?
                .is_truthy()?
        {
            entry.set_item("vkey", vkey)?;
            entry.set_item("serial", serial)?;
            return Ok(entry);
        }
    }
    let fp = painted_fp(v.call_method1("_group_fp", (group,))?)?;
    if !entry.is_none() {
        let old = entry.get_item("fp")?;
        let matches = |a: &Obj, b: &Obj, start: isize, end: isize| -> PyResult<bool> {
            let s = PySlice::new(py, start, end, 1);
            a.get_item(&s)?.eq(b.get_item(&s)?)
        };
        let same = old.eq(&fp)?
            || (get(&entry, "fp_approx")?.is_truthy()?
                && matches(&old, &fp, 0, 3)?
                && matches(&old, &fp, 4, isize::MAX)?);
        if same
            && v.call_method1("_samples_match", (&entry, &mesh))?
                .is_truthy()?
        {
            entry.set_item("fp", &fp)?;
            entry.set_item("fp_approx", false)?;
            entry.set_item("serial", serial)?;
            entry.set_item("vkey", vkey)?;
            h.call_method1("_flags_read", (&mesh, &entry))?;
            return Ok(entry);
        }
    }
    let timer = perf(&h)?;
    let loader = opt(v, "_chunk_cache_load")?;
    if loader.is_callable() {
        let disk = loader.call1((group, &fp, &vkey))?;
        if !disk.is_none()
            && v.call_method1("_samples_match", (&disk, &mesh))?
                .is_truthy()?
        {
            disk.set_item("msig", msig)?;
            disk.set_item("serial", serial)?;
            disk.set_item("mesh", &mesh)?;
            h.call_method1("_flags_read", (&mesh, &disk))?;
            cache.set_item(id(group), &disk)?;
            if timer.is_some() {
                log(
                    &h,
                    timer,
                    "chunk_from_disk",
                    format!("faces={}", disk.get_item("nf")?),
                    0.,
                )?
            }
            return Ok(disk);
        }
    }
    let editing = opt(v, "_editing_meshes")?;
    let in_edit = editing.is_callable() && editing.call0()?.contains(id(&mesh))?;
    let fresh = bake(
        v,
        &mesh,
        &paint,
        &fp,
        &vkey,
        &msig,
        in_edit || !editing.is_callable(),
    )?;
    if timer.is_some() {
        log(
            &h,
            timer,
            "chunk_rebuild",
            format!("faces={}", fresh.get_item("faces")?.len()?),
            0.,
        )?
    }
    h.call_method1("_flags_read", (&mesh, &fresh))?;
    fresh.set_item("mesh", &mesh)?;
    cache.set_item(id(group), &fresh)?;
    if !in_edit {
        mesh.getattr("__dict__")?
            .call_method1("pop", ("_face_geom_memo", py.None()))?;
        let save = opt(v, "_chunk_cache_store")?;
        if save.is_callable() {
            save.call1((mesh, &fresh))?;
        }
    }
    Ok(fresh)
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(group_chunk, m)?)?;
    Ok(())
}
