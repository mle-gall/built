// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::types::PySlice;

pub(super) fn weight(v: DVec3) -> f64 {
    v.x + v.y * 1.000003 + v.z * 1.000007
}
pub(super) fn coordsum(verts: &Obj) -> PyResult<f64> {
    verts.try_iter()?.try_fold(
        0.,
        |sum, v| Ok(sum + weight(xyz(&v?.getattr("position")?)?)),
    )
}
pub(super) fn tex_sig<'py>(t: &Obj<'py>) -> PyResult<Obj<'py>> {
    if !t.is_truthy()? {
        return Ok(none(t.py()));
    }
    let uv = get(t, "uvw")?;
    let uv = if uv.is_truthy()? {
        tuple(&uv)?
    } else {
        PyTuple::empty(t.py()).into_any()
    };
    Ok((
        get(t, "path")?,
        get(t, "sw")?,
        get(t, "sh")?,
        t.call_method1("get", ("rot", 0))?,
        uv,
    )
        .into_pyobject(t.py())?
        .into_any())
}
pub(super) fn back_sig<'py>(attrs: &Obj<'py>) -> PyResult<Obj<'py>> {
    let back = get(attrs, "back")?;
    if back.is_truthy()? {
        Ok(back.repr()?.into_any())
    } else {
        Ok(none(attrs.py()))
    }
}
#[pyfunction]
fn faces_paint_fp(mesh: &Obj) -> PyResult<isize> {
    let mut bits = 0;
    for (i, f) in mesh.getattr("faces")?.try_iter()?.enumerate() {
        let a = f?.getattr("attrs")?;
        if !a.is_truthy()? {
            continue;
        }
        let color = get(&a, "color")?;
        let color = if color.is_none() {
            color
        } else {
            tuple(&color)?
        };
        bits ^= (
            i,
            color,
            tex_sig(&get(&a, "texture")?)?,
            get(&a, "layer")?,
            get(&a, "opacity")?,
            back_sig(&a)?,
            get(&a, "hidden")?.is_truthy()?,
        )
            .into_pyobject(mesh.py())?
            .hash()?;
    }
    Ok(bits)
}
#[pyfunction]
fn mesh_fingerprint<'py>(mesh: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let verts = mesh.getattr("vertices")?;
    let edges = mesh.getattr("edges")?;
    let mut soft = 0usize;
    let (mut flat, mut hidden) = (0u32, 0u32);
    for (i, e) in edges.try_iter()?.enumerate() {
        let e = e?;
        let marker = (i as u32 + 1).wrapping_mul(2654435761);
        if truth(&e, "soft")? {
            soft += 1;
            if truth(&e, "flat")? {
                flat ^= marker
            }
        }
        if truth(&e, "hidden")? {
            hidden ^= marker
        }
    }
    let soft = if flat != 0 {
        (soft, flat).into_pyobject(py)?.into_any()
    } else {
        soft.into_pyobject(py)?.into_any()
    };
    let key = (
        verts.len()?,
        edges.len()?,
        mesh.getattr("faces")?.len()?,
        rounded(py, coordsum(&verts)?, 4)?,
        faces_paint_fp(mesh)?,
        soft,
        hidden,
    )
        .into_pyobject(py)?
        .into_any();
    let winding = opt(mesh, "_winding_rev")?;
    if winding.is_truthy()? {
        key.add((("winding", winding),))
    } else {
        Ok(key)
    }
}
#[pyfunction]
fn group_fp<'py>(view: &Obj<'py>, group: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    let scene = view.getattr("scene")?;
    let key = (
        id(group),
        scene.getattr("version")?,
        id(&scene.getattr("mesh")?),
    )
        .into_pyobject(py)?;
    let cache = memo(view, "_fp_memo")?;
    let old = cache.call_method1("get", (&key,))?;
    if !old.is_none() {
        return Ok(old);
    }
    if cache.len()? > 64 {
        cache.call_method0("clear")?;
    }
    let h = host(py)?;
    let start = perf(&h)?;
    let fp = view.call_method1("_mesh_fingerprint", (group.getattr("mesh")?,))?;
    cache.set_item(key, &fp)?;
    if start.is_some() {
        log(
            &h,
            start,
            "fingerprint",
            format!("nv={}", fp.get_item(0)?),
            50.,
        )?
    }
    Ok(fp)
}
pub(super) fn counts(entry: &Obj, mesh: &Obj) -> PyResult<bool> {
    for (field, n) in [("vertices", "nv"), ("edges", "ne"), ("faces", "nf")] {
        if !entry.get_item(n)?.eq(mesh.getattr(field)?.len()?)? {
            return Ok(false);
        }
    }
    Ok(true)
}
fn sample_offset(verts: &Obj, sample: &Obj) -> PyResult<DVec3> {
    Ok(
        xyz(&verts.get_item(sample.get_item(0)?)?.getattr("position")?)?
            - DVec3::from_array(sample.get_item(1)?.extract()?),
    )
}
#[pyfunction]
fn translation_probe<'py>(entry: &Obj<'py>, mesh: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    if !counts(entry, mesh)? {
        return Ok(none(py));
    }
    let samples = entry.get_item("samples")?;
    let mut samples = samples.try_iter()?;
    let Some(first) = samples.next() else {
        return Ok(none(py));
    };
    let verts = mesh.getattr("vertices")?;
    let delta = sample_offset(&verts, &first?)?;
    if delta.length() < 1e-9 {
        return Ok(none(py));
    }
    for s in samples {
        if (sample_offset(&verts, &s?)? - delta).length() > 2e-4 {
            return Ok(none(py));
        }
    }
    vector(py, delta)
}
#[pyfunction]
fn samples_match(entry: &Obj, mesh: &Obj) -> PyResult<bool> {
    let verts = mesh.getattr("vertices")?;
    if !entry.get_item("nv")?.eq(verts.len()?)? {
        return Ok(false);
    }
    for s in entry.get_item("samples")?.try_iter()? {
        if sample_offset(&verts, &s?)?.length() > 1e-6 {
            return Ok(false);
        }
    }
    Ok(true)
}
#[pyfunction]
fn chunk_cache_key<'py>(mesh: &Obj<'py>, fp: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let digest = py.import("hashlib")?.call_method0("sha1")?;
    let add_repr = |value: &Obj| -> PyResult<()> {
        digest.call_method1("update", (value.repr()?.call_method0("encode")?,))?;
        Ok(())
    };
    add_repr(
        &(
            fp.get_item(0)?,
            fp.get_item(1)?,
            fp.get_item(2)?,
            fp.get_item(3)?,
            fp.get_item(5)?,
        )
            .into_pyobject(py)?
            .into_any(),
    )?;
    let faces = mesh.getattr("faces")?;
    let mut corners = Vec::new();
    for f in faces.try_iter()? {
        for v in f?
            .getattr("loop")?
            .get_item(PySlice::new(py, 0, 2, 1))?
            .try_iter()?
        {
            for c in xyz(&v?.getattr("position")?)?.to_array() {
                corners.extend_from_slice(&c.to_ne_bytes())
            }
        }
    }
    digest.call_method1("update", (PyBytes::new(py, &corners),))?;
    if fp.get_item(5)?.is_truthy()? {
        digest.call_method1(
            "update",
            (PyBytes::new(py, b"smooth-shading-angle-weighted-v3"),),
        )?;
    }
    if fp.get_item(6)?.is_truthy()? {
        add_repr(&("ehidden", fp.get_item(6)?).into_pyobject(py)?.into_any())?
    }
    for (i, f) in faces.try_iter()?.enumerate() {
        let a = f?.getattr("attrs")?;
        if !a.is_truthy()? {
            continue;
        }
        let mut col = get(&a, "color")?;
        if col.is_truthy()? {
            col = tuple(&col)?
        }
        add_repr(
            &(
                i,
                col,
                tex_sig(&get(&a, "texture")?)?,
                get(&a, "opacity")?,
                back_sig(&a)?,
                get(&a, "layer")?,
            )
                .into_pyobject(py)?
                .into_any(),
        )?;
        if get(&a, "hidden")?.is_truthy()? {
            add_repr(&("fhidden", i).into_pyobject(py)?.into_any())?
        }
    }
    digest.call_method0("hexdigest")
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(faces_paint_fp, m)?)?;
    m.add_function(wrap_pyfunction!(mesh_fingerprint, m)?)?;
    m.add_function(wrap_pyfunction!(group_fp, m)?)?;
    m.add_function(wrap_pyfunction!(translation_probe, m)?)?;
    m.add_function(wrap_pyfunction!(samples_match, m)?)?;
    m.add_function(wrap_pyfunction!(chunk_cache_key, m)?)?;
    Ok(())
}
