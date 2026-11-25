// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use numpy::{IntoPyArray, PyArrayMethods};
#[pyfunction]
fn face_geometry<'py>(
    f: &Obj<'py>,
    cache: &Obj<'py>,
    triangulate: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = f.py();
    let positions = |ring: &Obj<'py>| -> PyResult<Bound<'py, PyList>> {
        let l = PyList::empty(py);
        for v in ring.try_iter()? {
            l.append(v?.getattr("position")?)?
        }
        Ok(l)
    };
    let ring = positions(&f.getattr("loop")?)?;
    let holes = PyList::empty(py);
    for h in f.getattr("hole_loops")?.try_iter()? {
        holes.append(positions(&h?)?)?
    }
    let (signature, identity) = if cache.is_none() {
        (none(py), none(py))
    } else {
        let word = |l: &Obj<'py>| -> PyResult<Obj<'py>> {
            let mut out = Vec::new();
            for p in l.try_iter()? {
                out.push(coords(xyz(&p?)?))
            }
            Ok(PyTuple::new(py, out)?.into_any())
        };
        let ws = PyTuple::new(
            py,
            holes
                .iter()
                .map(|h| word(&h))
                .collect::<PyResult<Vec<_>>>()?,
        )?;
        let key = (word(ring.as_any())?, ws).into_pyobject(py)?.into_any();
        let id = host(py)?.call_method1("element_identity", (f,))?;
        let old = cache.call_method1("get", (&id,))?;
        if !old.is_none() && old.get_item(0)?.eq(&key)? {
            return Ok(old);
        }
        (key, id)
    };
    let tris = PyList::empty(py);
    let flat = PyList::empty(py);
    let (area, normal, center, props) = if ring.len() < 3 {
        (0., vector(py, DVec3::Z)?, none(py), none(py))
    } else {
        let n = xyz(&f.call_method0("_newell")?)?;
        let l = n.length();
        let normal = vector(py, if l > 1e-9 { n / l } else { DVec3::Z })?;
        let out = triangulate.call1((ring, holes, &normal))?;
        for tri in out.try_iter()? {
            let mut corners = Vec::new();
            for p in tri?.try_iter()? {
                let p = xyz(&p?)?;
                corners.push(coords(p));
                for x in p.to_array() {
                    flat.append(x)?
                }
            }
            tris.append(PyTuple::new(py, corners)?)?
        }
        let c = f.call_method0("centroid")?;
        let props = (coords(xyz(&normal)?), coords(xyz(&c)?))
            .into_pyobject(py)?
            .into_any();
        (0.5 * l, normal, c, props)
    };
    let value = (
        signature,
        normal,
        area,
        tris,
        center,
        PyDict::new(py),
        flat.to_tuple(),
        props,
    )
        .into_pyobject(py)?
        .into_any();
    if !cache.is_none() {
        cache.set_item(identity, &value)?
    }
    Ok(value)
}
fn geom_memo<'py>(v: &Obj<'py>, name: &str) -> PyResult<Obj<'py>> {
    let py = v.py();
    let version = host(py)?.call_method1("_cache_ver", (v,))?;
    let memo = opt(v, name)?;
    if !memo.is_none() && memo.get_item(0)?.eq(&version)? {
        return memo.get_item(1);
    }
    let map = PyDict::new(py);
    v.setattr(name, (version, &map))?;
    Ok(map.into_any())
}
fn cached_face<'py>(v: &Obj<'py>, f: &Obj<'py>, newell: bool) -> PyResult<Obj<'py>> {
    let memo = geom_memo(v, if newell { "_newell_memo" } else { "_tri_memo" })?;
    let key = host(v.py())?.call_method1("element_identity", (f,))?;
    let old = memo.call_method1("get", (&key,))?;
    if !old.is_none() && old.get_item(0)?.eq(f)? {
        return old.get_item(1);
    }
    let value = if newell {
        f.call_method0("_newell")?
    } else {
        f.call_method1("triangulate", (v.call_method1("_normal_of", (f,))?,))?
    };
    memo.set_item(key, (f, &value))?;
    Ok(value)
}
#[pyfunction]
fn newell_of<'py>(v: &Obj<'py>, f: &Obj<'py>) -> PyResult<Obj<'py>> {
    cached_face(v, f, true)
}
#[pyfunction]
fn tris_of<'py>(v: &Obj<'py>, f: &Obj<'py>) -> PyResult<Obj<'py>> {
    if f.hasattr("loop")? {
        cached_face(v, f, false)
    } else {
        f.call_method0("triangulate")
    }
}
#[pyfunction]
fn normal_of<'py>(v: &Obj<'py>, f: &Obj<'py>) -> PyResult<Obj<'py>> {
    let Some(r) = f.getattr_opt("loop")? else {
        return f.call_method0("normal");
    };
    let mut n = DVec3::Z;
    if r.len()? >= 3 {
        let raw = xyz(&v.call_method1("_newell_of", (f,))?)?;
        if raw.length() > 1e-9 {
            n = raw.normalize()
        }
    }
    vector(v.py(), n)
}
#[pyfunction]
fn area_of(v: &Obj, f: &Obj) -> PyResult<f64> {
    let Some(r) = f.getattr_opt("loop")? else {
        return f.call_method0("area")?.extract();
    };
    if r.len()? < 3 {
        Ok(0.)
    } else {
        Ok(0.5 * xyz(&v.call_method1("_newell_of", (f,))?)?.length())
    }
}
#[pyfunction]
fn seed_tris_native(v: &Obj, faces: &Obj) -> PyResult<()> {
    let py = v.py();
    let api = py.import("core._native")?;
    let nat = api.getattr("native")?;
    let mode = api.call_method0("qt_hypot")?;
    if nat.is_none() || mode.is_none() {
        return Ok(());
    }
    let memo = geom_memo(v, "_tri_memo")?;
    let identity = host(py)?.getattr("element_identity")?;
    let mut ids = Vec::new();
    let mut pos = Vec::new();
    let mut vertices = Vec::new();
    let mut sizes = Vec::new();
    let mut counts = Vec::new();
    for f in faces.try_iter()? {
        let f = f?;
        if !f.hasattr("loop")? || f.getattr("loop")?.len()? < 3 {
            continue;
        }
        let key = identity.call1((&f,))?;
        let old = memo.call_method1("get", (&key,))?;
        if !old.is_none() && old.get_item(0)?.eq(&f)? {
            continue;
        }
        let mut loops = vec![f.getattr("vertices")?];
        for h in f.getattr("holes")?.try_iter()? {
            let h = h?;
            if h.len()? >= 3 {
                loops.push(h)
            }
        }
        let n = loops
            .iter()
            .map(|r| r.len())
            .collect::<PyResult<Vec<_>>>()?
            .into_iter()
            .sum::<usize>();
        if n < v.getattr("NATIVE_TRIS_MIN_CORNERS")?.extract::<usize>()? {
            continue;
        }
        ids.push((key, f));
        counts.push(loops.len() as i64);
        for l in loops {
            sizes.push(l.len()? as i64);
            for p in l.try_iter()? {
                let p = p?;
                pos.extend(xyz(&p)?.to_array());
                vertices.push(p)
            }
        }
    }
    if ids.is_empty() {
        return Ok(());
    }
    let vertex_ids = (0..vertices.len() as i64)
        .collect::<Vec<_>>()
        .into_pyarray(py);
    let pos = pos.into_pyarray(py).reshape([vertices.len(), 3])?;
    let version = py.import("sys")?.getattr("version_info")?.ge((3, 12))?;
    let out = match nat.call_method1(
        "face_geometry",
        (
            pos,
            vertex_ids,
            sizes.into_pyarray(py),
            counts.into_pyarray(py),
            mode,
            version,
        ),
    ) {
        Ok(v) => v,
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyException>(py) => return Ok(()),
        Err(e) => return Err(e),
    };
    let tris = out
        .get_item(6)?
        .call_method0("tolist")?
        .extract::<Vec<[usize; 3]>>()?;
    let fi = out
        .get_item(7)?
        .call_method0("tolist")?
        .extract::<Vec<usize>>()?;
    let lists = ids.iter().map(|_| PyList::empty(py)).collect::<Vec<_>>();
    for ([a, b, c], i) in tris.into_iter().zip(fi) {
        lists[i].append((&vertices[a], &vertices[b], &vertices[c]))?
    }
    for ((key, face), list) in ids.into_iter().zip(lists) {
        memo.set_item(key, (face, list))?
    }
    Ok(())
}
#[pyfunction]
fn append_textured_face(v: &Obj, runs: &Obj, f: &Obj, tex: &Obj) -> PyResult<()> {
    let py = v.py();
    let key = (
        tex.get_item("path")?,
        v.call_method1("_shade_factor", (v.call_method1("_normal_of", (f,))?,))?,
    )
        .into_pyobject(py)?;
    let mut buf = runs.call_method1("get", (&key,))?;
    if buf.is_none() {
        buf = py.import("array")?.call_method1("array", ("f",))?;
        runs.set_item(key, &buf)?
    }
    let (gu, cu, gv, cv): (Obj, f64, Obj, f64) = host(py)?
        .call_method1("face_uv_axes", (tex, f.call_method0("normal")?))?
        .extract()?;
    let (gu, gv) = (xyz(&gu)?, xyz(&gv)?);
    let mut words = Vec::<f32>::new();
    for tri in v.call_method1("_tris_of", (f,))?.try_iter()? {
        for p in tri?.try_iter()? {
            let p = xyz(&p?)?;
            words.extend([p.x, p.y, p.z, gu.dot(p) + cu, gv.dot(p) + cv].map(|n| n as f32));
        }
    }
    buf.call_method1(
        "frombytes",
        (PyBytes::new(py, bytemuck::cast_slice(&words)),),
    )?;
    Ok(())
}
#[pyfunction]
fn bucket_back_face<'py>(v: &Obj<'py>, face: &Obj<'py>, back: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let opacity = back
        .call_method1("get", ("opacity", 1.))?
        .extract::<f64>()?;
    let tex = get(back, "texture")?;
    let no = (py.None(), py.None()).into_pyobject(py)?.into_any();
    if !tex.is_none() && get(&tex, "path")?.is_truthy()? {
        let tmp = PyDict::new(py);
        v.call_method1("_append_textured_face", (&tmp, face, tex))?;
        if let Some((k, buf)) = tmp.iter().next() {
            let bytes = buf.call_method0("tobytes")?;
            return Ok(if opacity < 0.999 {
                ("bttex", ((k, rounded(py, opacity, 3)?), bytes))
                    .into_pyobject(py)?
                    .into_any()
            } else {
                ("btex", (k, bytes)).into_pyobject(py)?.into_any()
            });
        }
        return Ok(no);
    }
    let col = get(back, "color")?;
    if col.is_none() {
        return Ok(no);
    }
    let rgb = v
        .call_method1(
            "_shaded_color",
            (tuple(&col)?, v.call_method1("_normal_of", (face,))?),
        )?
        .extract::<[f64; 3]>()?
        .map(|c| c as f32);
    let mut data = Vec::new();
    for t in v.call_method1("_tris_of", (face,))?.try_iter()? {
        for p in t?.try_iter()? {
            data.extend(xyz(&p?)?.to_array().map(|x| x as f32));
            data.extend(rgb)
        }
    }
    let bytes = PyBytes::new(py, bytemuck::cast_slice(&data));
    Ok(if opacity < 0.999 {
        ("btcol", (rounded(py, opacity, 3)?, bytes))
            .into_pyobject(py)?
            .into_any()
    } else {
        ("bvcol", bytes).into_pyobject(py)?.into_any()
    })
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(face_geometry, m)?)?;

    m.add_function(wrap_pyfunction!(newell_of, m)?)?;
    m.add_function(wrap_pyfunction!(tris_of, m)?)?;
    m.add_function(wrap_pyfunction!(normal_of, m)?)?;
    m.add_function(wrap_pyfunction!(area_of, m)?)?;
    m.add_function(wrap_pyfunction!(seed_tris_native, m)?)?;
    m.add_function(wrap_pyfunction!(append_textured_face, m)?)?;
    m.add_function(wrap_pyfunction!(bucket_back_face, m)?)?;
    Ok(())
}
