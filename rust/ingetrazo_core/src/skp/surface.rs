// SPDX-License-Identifier: GPL-3.0-or-later
//! Coedge connectivity, per-side paint, and texture coordinates in a
//! definition's local frame. The packed world rows can still share storage.
use glam::{DMat3, DVec3};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySlice},
};

#[pyfunction]
pub fn skp_ring<'py>(
    defn: &Bound<'py, PyAny>,
    coedges: &Bound<'py, PyAny>,
) -> PyResult<Option<Bound<'py, PyList>>> {
    let table = defn.getattr("edges")?;
    let vertices = defn.getattr("vertices")?;
    let mut ends = Vec::with_capacity(coedges.len()?);
    for u in coedges.try_iter()? {
        let u = u?;
        let e = table.call_method1("get", (u.get_item(0)?,))?;
        if e.is_none() {
            return Ok(None);
        }
        ends.push((
            e.getattr("v1_id")?,
            e.getattr("v2_id")?,
            u.get_item(1)?.is_truthy()?,
        ));
    }
    let out = PyList::empty(defn.py());
    for (i, (a, b, sense)) in ends.iter().enumerate() {
        let (next_a, next_b, _) = &ends[(i + 1) % ends.len()];
        let (shared_a, shared_b) = (
            a.eq(next_a)? || a.eq(next_b)?,
            b.eq(next_a)? || b.eq(next_b)?,
        );
        let id = match (shared_a, shared_b) {
            (true, false) => a,
            (false, true) => b,
            _ if *sense => a,
            _ => b,
        };
        let v = vertices.call_method1("get", (id,))?;
        if v.is_none() {
            return Ok(None);
        }
        out.append((v.getattr("x")?, v.getattr("y")?, v.getattr("z")?))?;
    }
    Ok(Some(out))
}

fn attr<'py>(v: &Bound<'py, PyAny>, field: &str) -> PyResult<Bound<'py, PyAny>> {
    Ok(v.getattr_opt(field)?
        .unwrap_or_else(|| v.py().None().into_bound(v.py())))
}
fn mat<'py>(
    map: &Bound<'py, PyDict>,
    key: &Bound<'py, PyAny>,
) -> PyResult<Option<Bound<'py, PyDict>>> {
    map.get_item(key)?
        .filter(|v| !v.is_none())
        .map(|v| v.cast_into::<PyDict>().map_err(Into::into))
        .transpose()
}
fn coords(
    face: &Bound<'_, PyAny>,
    ring: &Bound<'_, PyAny>,
    texture: &Bound<'_, PyDict>,
    matrix: Option<&Bound<'_, PyAny>>,
    projected: bool,
) -> PyResult<Option<Vec<(f64, f64)>>> {
    let scale = |key| -> PyResult<f64> {
        Ok(texture
            .get_item(key)?
            .map(|v| v.extract::<Option<f64>>())
            .transpose()?
            .flatten()
            .unwrap_or(0.)
            / 0.0254)
    };
    let (width, height) = (scale("sw")?, scale("sh")?);
    if width <= 0. || height <= 0. {
        return Ok(None);
    }
    let inverse = if let Some(v) = matrix {
        if v.len()? != 9 {
            return Ok(None);
        }
        let m = DMat3::from_cols_array(&v.extract()?);
        if m.determinant().abs() < 1e-15 {
            return Ok(None);
        }
        Some(m.inverse())
    } else {
        None
    };
    let (x, y) = if projected {
        (DVec3::X, DVec3::Y)
    } else {
        let n = attr(face, "normal")?;
        let normal = if n.is_truthy()? {
            n
        } else {
            (0., 0., 1.).into_pyobject(face.py())?.into_any()
        };
        let (x, y) = face
            .py()
            .import("core.texture")?
            .call_method1("projection_basis", (normal,))?
            .extract::<([f64; 3], [f64; 3])>()?;
        (DVec3::from_array(x), DVec3::from_array(y))
    };
    let corners = ring.extract::<Vec<[f64; 3]>>()?;
    Ok(face.py().detach(|| {
        corners
            .into_iter()
            .map(|v| {
                let p = DVec3::from_array(v);
                let q = DVec3::new(p.dot(x), p.dot(y), 1.);
                let q = inverse.map_or(q, |m| m * q);
                (q.z.abs() >= 1e-12).then(|| (q.x / q.z / width, q.y / q.z / height))
            })
            .collect()
    }))
}

#[pyfunction]
#[pyo3(signature=(face,raw_ring,tex,matrix=None,projected=false))]
pub fn skp_positioned_uvs(
    face: &Bound<'_, PyAny>,
    raw_ring: &Bound<'_, PyAny>,
    tex: &Bound<'_, PyDict>,
    matrix: Option<&Bound<'_, PyAny>>,
    projected: bool,
) -> PyResult<Option<Vec<(f64, f64)>>> {
    let m = matrix.cloned().unwrap_or(attr(face, "uv_transform")?);
    if m.is_none() {
        return Ok(None);
    }
    coords(face, raw_ring, tex, Some(&m), projected)
}

fn bake<'py>(
    face: &Bound<'py, PyAny>,
    outer: &Bound<'py, PyAny>,
    raw: &Bound<'py, PyAny>,
    paint: Option<&Bound<'py, PyDict>>,
    matrix: &Bound<'py, PyAny>,
    projected: bool,
) -> PyResult<Option<Bound<'py, PyDict>>> {
    let Some(entry) = paint else {
        return Ok(None);
    };
    let Some(tex) = entry.get_item("texture")? else {
        return Ok(Some(entry.clone()));
    };
    let tex = tex.cast::<PyDict>()?;
    let source = (!matrix.is_none()).then_some(matrix);
    let Some(uv) = coords(face, raw, tex, source, projected && source.is_some())? else {
        return Ok(Some(entry.clone()));
    };
    let fit = face
        .py()
        .import("core.texture")?
        .call_method1("fit_uv_affine", (outer, uv))?;
    if fit.is_none() {
        return Ok(Some(entry.clone()));
    }
    let baked = tex.copy()?;
    baked.set_item("uvw", fit)?;
    if source.is_none() {
        baked.set_item("planar", true)?;
    }
    let out = entry.copy()?;
    out.set_item("texture", baked)?;
    Ok(Some(out))
}

#[pyfunction]
#[pyo3(signature=(face,wl,raw_l,s0,s1,holes_sl,attr_map,inherited=None,layer=None,mirrored=false))]
pub fn skp_face_entry<'py>(
    face: &Bound<'py, PyAny>,
    wl: &Bound<'py, PyAny>,
    raw_l: &Bound<'py, PyAny>,
    s0: isize,
    s1: isize,
    holes_sl: Vec<(isize, isize)>,
    attr_map: &Bound<'py, PyDict>,
    inherited: Option<&Bound<'py, PyAny>>,
    layer: Option<&Bound<'py, PyAny>>,
    mirrored: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let py = face.py();
    let own = mat(attr_map, &attr(face, "material_id")?)?;
    let back = mat(attr_map, &attr(face, "back_material_id")?)?;
    let inherited = inherited.map(|v| mat(attr_map, v)).transpose()?.flatten();
    let flipped = own.is_none() && back.is_some();
    let slice = |points: &Bound<'py, PyAny>, a, b| -> PyResult<Bound<'py, PyAny>> {
        let ring = points.get_item(PySlice::new(py, a, b, 1))?;
        if mirrored != flipped {
            // Use the object's Python slicing contract, including packed spans.
            let reverse = py.get_type::<PySlice>().call1((py.None(), py.None(), -1))?;
            ring.get_item(reverse)
        } else {
            Ok(ring)
        }
    };
    let outer = slice(wl, s0, s1)?;
    let holes = PyList::empty(py);
    for (a, b) in holes_sl {
        holes.append(slice(wl, a, b)?)?;
    }
    let raw = slice(raw_l, s0, s1)?;
    let front = own.as_ref().or(back.as_ref()).or(inherited.as_ref());
    let flags = if flipped {
        "uv_projected_back"
    } else {
        "uv_projected"
    };
    let matrix = attr(
        face,
        if flipped {
            "uv_transform_back"
        } else {
            "uv_transform"
        },
    )?;
    let mut paint = bake(
        face,
        &outer,
        &raw,
        front,
        &matrix,
        attr(face, flags)?.is_truthy()? || attr(face, "_projected")?.is_truthy()?,
    )?;
    if let Some(other) = back.as_ref().or(inherited.as_ref()).filter(|_| !flipped) {
        let entry = match paint {
            Some(p) => p.copy()?,
            None => PyDict::new(py),
        };
        if front.is_some_and(|f| f.is(other)) {
            entry.set_item("back", true)?;
        } else {
            entry.set_item(
                "back",
                bake(
                    face,
                    &outer,
                    &raw,
                    Some(other),
                    &attr(face, "uv_transform_back")?,
                    attr(face, "uv_projected_back")?.is_truthy()?,
                )?,
            )?;
        }
        paint = Some(entry);
    }
    let tag = attr(face, "layer")?;
    if let Some(tag) = if tag.is_truthy()? { Some(&tag) } else { layer } {
        if tag.is_truthy()? {
            let entry = match paint {
                Some(p) => p.copy()?,
                None => PyDict::new(py),
            };
            entry.set_item("layer", tag)?;
            paint = Some(entry);
        }
    }
    Ok((outer, holes, paint).into_pyobject(py)?.into_any())
}
