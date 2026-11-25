// SPDX-License-Identifier: GPL-3.0-or-later
//! Rectangular reference images, their journalled fields and texture mapping.
use crate::math::{xyz, Vec3};
use glam::DVec3;
use pyo3::{exceptions::PyAttributeError, prelude::*, types::PyDict};
type Obj<'p> = Bound<'p, PyAny>;
const MODULE: &str = "built_core.image_plane";
const LAYER: &str = "Images";
const FIELDS: [&str; 9] = [
    "path", "origin", "u", "v", "aspect", "name", "opacity", "layer", "locked",
];
#[pyfunction(name="__init__",signature=(s,path,origin,u,v,aspect=1.,name=None,opacity=1.,layer=Some(LAYER),locked=false))]
#[allow(clippy::too_many_arguments)]
fn init(
    s: &Obj,
    path: &Obj,
    origin: &Obj,
    u: &Obj,
    v: &Obj,
    aspect: f64,
    name: Option<&str>,
    opacity: f64,
    layer: Option<&str>,
    locked: bool,
) -> PyResult<()> {
    s.setattr("path", path.str()?)?;
    for (key, vector) in [("origin", origin), ("u", u), ("v", v)] {
        s.setattr(key, Vec3::from(xyz(vector)?))?;
    }
    s.setattr("aspect", if aspect > 0. { aspect } else { 1. })?;
    s.setattr("name", name.unwrap_or(""))?;
    s.setattr("opacity", opacity)?;
    s.setattr("layer", layer)?;
    s.setattr("locked", locked)
}
fn field(s: &Obj, key: &str) -> PyResult<DVec3> {
    xyz(&s.getattr(key)?)
}
#[pyfunction]
fn corners(s: &Obj) -> PyResult<Vec<Vec3>> {
    let (o, u, v) = (field(s, "origin")?, field(s, "u")?, field(s, "v")?);
    Ok([o, o + u, o + u + v, o + v].map(Vec3::from).into())
}
#[pyfunction]
fn center(s: &Obj) -> PyResult<Vec3> {
    Ok((field(s, "origin")? + (field(s, "u")? + field(s, "v")?) * 0.5).into())
}
#[pyfunction]
fn width(s: &Obj) -> PyResult<f64> {
    Ok(field(s, "u")?.length())
}
#[pyfunction]
fn height(s: &Obj) -> PyResult<f64> {
    Ok(field(s, "v")?.length())
}
fn cross(s: &Obj) -> PyResult<DVec3> {
    Ok(field(s, "u")?.cross(field(s, "v")?))
}
#[pyfunction]
fn normal(s: &Obj) -> PyResult<Vec3> {
    let n = cross(s)?;
    Ok(if n.length() > 1e-12 {
        n.normalize()
    } else {
        DVec3::Z
    }
    .into())
}
#[pyfunction]
fn plane(s: &Obj) -> PyResult<(Vec3, Vec3)> {
    Ok((center(s)?, normal(s)?))
}
#[pyfunction]
fn border_edges(s: &Obj) -> PyResult<Vec<(Vec3, Vec3)>> {
    let c = corners(s)?;
    Ok((0..4)
        .map(|i| (c[i].clone(), c[(i + 1) % 4].clone()))
        .collect())
}
#[pyfunction]
fn contains_uv(_s: &Obj, u_frac: f64, v_frac: f64) -> bool {
    let bounds = -1e-9..=1. + 1e-9;
    bounds.contains(&u_frac) && bounds.contains(&v_frac)
}
#[pyfunction]
fn project(s: &Obj, point: &Obj) -> PyResult<(f64, f64)> {
    let d = xyz(point)? - field(s, "origin")?;
    let fraction = |v: DVec3| {
        if v.length_squared() > 1e-18 {
            d.dot(v) / v.length_squared()
        } else {
            0.
        }
    };
    Ok((fraction(field(s, "u")?), fraction(field(s, "v")?)))
}
#[pyfunction]
fn texture<'p>(s: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let py = s.py();
    let out = PyDict::new(py);
    out.set_item("path", s.getattr("path")?)?;
    out.set_item("sw", width(s)?)?;
    out.set_item("sh", height(s)?)?;
    let uv = py.import("built_core")?.call_method1(
        "texture_fit_uv_affine",
        (corners(s)?, [(0., 0.), (1., 0.), (1., 1.), (0., 1.)]),
    )?;
    if !uv.is_none() {
        out.set_item("uvw", uv)?;
    }
    Ok(out)
}
#[pyfunction]
fn as_group<'p>(s: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    if cross(s)?.length() <= 1e-12 {
        return Ok(None);
    }
    let py = s.py();
    let mesh = py.import("built_core.mesh")?.call_method0("Mesh")?;
    let attrs = mesh
        .call_method1("add_face", (corners(s)?,))?
        .getattr("attrs")?;
    attrs.set_item("texture", texture(s)?)?;
    attrs.set_item("back", true)?;
    let opacity = s.getattr("opacity")?.extract::<f64>()?;
    if opacity < 1. {
        attrs.set_item("opacity", opacity)?;
    }
    let kw = PyDict::new(py);
    let name = s.getattr("name")?;
    kw.set_item("name", name.is_truthy()?.then_some(name))?;
    let g = py
        .import("built_core.group")?
        .call_method("Group", (mesh,), Some(&kw))?;
    g.setattr("layer", s.getattr("layer")?)?;
    Ok(Some(g))
}
#[pyfunction(signature=(s,width,keep_aspect=true))]
fn scaled(s: &Obj, width: f64, keep_aspect: bool) -> PyResult<(Vec3, Vec3)> {
    let (mut u, mut v) = (field(s, "u")?, field(s, "v")?);
    let (w, h) = (u.length(), v.length());
    if w > 1e-12 && width > 0. {
        u *= width / w;
        if keep_aspect && h > 1e-12 {
            v *= width * s.getattr("aspect")?.extract::<f64>()? / h;
        }
    }
    Ok((u.into(), v.into()))
}
#[pyfunction]
fn to_dict<'p>(s: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let out = PyDict::new(s.py());
    let tex = PyDict::new(s.py());
    tex.set_item("path", s.getattr("path")?)?;
    out.set_item("texture", tex)?;
    for k in ["origin", "u", "v"] {
        out.set_item(k, field(s, k)?.to_array())?;
    }
    out.set_item("aspect", s.getattr("aspect")?)?;
    for k in ["name", "opacity", "layer", "locked"] {
        let v = s.getattr(k)?;
        let store = match k {
            "opacity" => !v.eq(1.)?,
            "layer" => v.is_none() || (v.is_truthy()? && !v.eq(LAYER)?),
            _ => v.is_truthy()?,
        };
        if store {
            out.set_item(k, v)?;
        }
    }
    Ok(out)
}
#[pyfunction]
fn from_dict<'p>(cls: &Obj<'p>, raw: &Obj<'p>) -> PyResult<Obj<'p>> {
    let d = raw.call_method1("get", ("texture",))?;
    let path = if d.is_instance_of::<PyDict>() {
        d
    } else {
        raw.clone()
    }
    .call_method1("get", ("path", ""))?;
    let kw = PyDict::new(cls.py());
    for k in ["aspect", "name", "opacity", "layer", "locked"] {
        if raw.contains(k)? {
            let value = raw.get_item(k)?;
            match k {
                "aspect" | "opacity" => kw.set_item(
                    k,
                    cls.py()
                        .get_type::<pyo3::types::PyFloat>()
                        .call1((value,))?,
                )?,
                "locked" => kw.set_item(k, value.is_truthy()?)?,
                _ => kw.set_item(k, value)?,
            }
        }
    }
    cls.call(
        (
            path,
            raw.get_item("origin")?,
            raw.get_item("u")?,
            raw.get_item("v")?,
        ),
        Some(&kw),
    )
}
#[pyfunction]
fn image_aspect(path: &Obj) -> PyResult<(f64, u32, u32)> {
    let dims = path
        .py()
        .import("built_render")?
        .call_method1("raster_dimensions", (path,))?
        .extract::<Option<(u32, u32)>>()?;
    Ok(match dims {
        Some((w, h)) if w > 0 && h > 0 => (h as f64 / w as f64, w, h),
        _ => (1., 0, 0),
    })
}
#[pyfunction(name = "__getattr__")]
fn lazy<'p>(py: Python<'p>, name: &str) -> PyResult<Obj<'p>> {
    if name != "ImagePlane" {
        return Err(PyAttributeError::new_err(name.to_owned()));
    }
    let m = py.import(MODULE)?;
    crate::record_class::build(
        &m,
        name,
        &FIELDS,
        true,
        vec![
            wrap_pyfunction!(init, &m)?,
            wrap_pyfunction!(corners, &m)?,
            wrap_pyfunction!(center, &m)?,
            wrap_pyfunction!(width, &m)?,
            wrap_pyfunction!(height, &m)?,
            wrap_pyfunction!(normal, &m)?,
            wrap_pyfunction!(plane, &m)?,
            wrap_pyfunction!(border_edges, &m)?,
            wrap_pyfunction!(contains_uv, &m)?,
            wrap_pyfunction!(project, &m)?,
            wrap_pyfunction!(texture, &m)?,
            wrap_pyfunction!(as_group, &m)?,
            wrap_pyfunction!(scaled, &m)?,
            wrap_pyfunction!(to_dict, &m)?,
            wrap_pyfunction!(from_dict, &m)?,
        ],
        &["from_dict"],
    )
}
pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), MODULE)?;
    sub.add("IMAGE_LAYER", LAYER)?;
    sub.add_function(wrap_pyfunction!(image_aspect, &sub)?)?;
    sub.add_function(wrap_pyfunction!(lazy, &sub)?)?;
    m.py()
        .import("sys")?
        .getattr("modules")?
        .set_item(MODULE, &sub)?;
    m.add("image_plane", sub)
}
