// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared world-space triangles, surface normals, UVs and named materials
//! for file interchange and camera-facing exports.
mod smooth;
mod world;
use crate::math::xyz;
use pyo3::{
    prelude::*,
    types::{PyDict, PyIterator, PyList, PyTuple},
};
use std::collections::HashSet;
type Obj<'p> = Bound<'p, PyAny>;
const CREAM: (f64, f64, f64) = (0.96, 0.95, 0.925);
fn get<'p>(d: &Obj<'p>, k: &str) -> PyResult<Obj<'p>> {
    d.call_method1("get", (k,))
}
fn attr<'p>(o: &Obj<'p>, k: &str) -> PyResult<Obj<'p>> {
    match o.getattr(k) {
        Ok(v) => Ok(v),
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyAttributeError>(o.py()) => {
            Ok(o.py().None().into_bound(o.py()))
        }
        Err(e) => Err(e),
    }
}
fn truth<'p>(a: Obj<'p>, b: Obj<'p>) -> PyResult<Obj<'p>> {
    if a.is_truthy()? {
        Ok(a)
    } else {
        Ok(b)
    }
}
fn rgb<'p>(attrs: &Obj<'p>, default: (f64, f64, f64), limit: bool) -> PyResult<Bound<'p, PyTuple>> {
    let py = attrs.py();
    let c = truth(get(attrs, "color")?, default.into_pyobject(py)?.into_any())?;
    let items = c
        .try_iter()?
        .take(if limit { 3 } else { usize::MAX })
        .collect::<PyResult<Vec<_>>>()?;
    PyTuple::new(py, items)
}
fn opacity<'p>(attrs: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = attrs.py();
    let op = get(attrs, "opacity")?;
    if op.is_none() {
        return Ok(op);
    }
    let value = op
        .call_method0("__float__")
        .or_else(|_| py.import("builtins")?.call_method1("float", (op,)))?;
    if value.extract::<f64>()? >= 0.999 {
        return Ok(py.None().into_bound(py));
    }
    value.call_method1("__round__", (3,))
}
fn path<'p>(p: &Obj<'p>) -> PyResult<Obj<'p>> {
    p.py().import("pathlib")?.call_method1("Path", (p,))
}
#[pyfunction]
#[pyo3(signature=(attrs,with_opacity=true))]
fn material_key<'p>(attrs: &Obj<'p>, with_opacity: bool) -> PyResult<Bound<'p, PyTuple>> {
    let py = attrs.py();
    let attrs = truth(attrs.clone(), PyDict::new(py).into_any())?;
    let tex = get(&attrs, "texture")?;
    let src = if tex.is_none() {
        py.None().into_bound(py)
    } else {
        get(&tex, "path")?
    };
    let mut parts = if src.is_truthy()? {
        vec![
            "tex".into_pyobject(py)?.into_any(),
            path(&src)?.getattr("name")?,
        ]
    } else {
        vec![
            "color".into_pyobject(py)?.into_any(),
            rgb(&attrs, CREAM, true)?.into_any(),
        ]
    };
    let pbr = get(&attrs, "pbr")?;
    if !pbr.is_none() {
        let kw = PyDict::new(py);
        kw.set_item("sort_keys", true)?;
        let recipe = py.import("json")?.call_method("dumps", (pbr,), Some(&kw))?;
        parts.push(
            ("pbr", recipe, rgb(&attrs, (1., 1., 1.), false)?, src)
                .into_pyobject(py)?
                .into_any(),
        );
    }
    if with_opacity {
        let op = opacity(&attrs)?;
        if !op.is_none() {
            parts.push(op)
        }
    }
    let name = get(&attrs, "mat")?;
    if name.is_truthy()? {
        parts.push(("mat", name).into_pyobject(py)?.into_any())
    }
    PyTuple::new(py, parts)
}
#[pyfunction]
fn export_names<'p>(materials: &Bound<'p, PyDict>) -> PyResult<Bound<'p, PyDict>> {
    let py = materials.py();
    let names = PyDict::new(py);
    let mut used = HashSet::new();
    for (i, (key, info)) in materials.iter().enumerate() {
        let mat = get(&info, "mat")?;
        let mut base = String::new();
        if mat.is_truthy()? {
            let raw = mat.call_method0("strip")?.extract::<String>()?;
            let normalized: String = raw
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || "._-".contains(c) {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            base = normalized.trim_matches(['.', '_', '-']).into();
        }
        if base.is_empty() {
            base = format!("mat{i}")
        } else if base
            .chars()
            .next()
            .unwrap()
            .to_string()
            .into_pyobject(py)?
            .call_method0("isdigit")?
            .is_truthy()?
        {
            base = format!("m_{base}")
        }
        let mut n = 2;
        let mut name = base.clone();
        while !used.insert(name.clone()) {
            name = format!("{base}_{n}");
            n += 1
        }
        names.set_item(key, name)?;
    }
    Ok(names)
}
#[pyfunction]
fn geolocation(scene: &Obj<'_>) -> PyResult<Option<(f64, f64, f64)>> {
    let g = attr(scene, "georef")?;
    let lat = attr(&g, "lat")?;
    let lon = attr(&g, "lon")?;
    if lat.is_none() || lon.is_none() {
        return Ok(None);
    }
    let float = scene.py().import("builtins")?.getattr("float")?;
    let alt = match g.getattr("alt") {
        Ok(alt) => float.call1((alt,))?.extract()?,
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyAttributeError>(scene.py()) => 0.,
        Err(e) => return Err(e),
    };
    Ok(Some((
        float.call1((lat,))?.extract()?,
        float.call1((lon,))?.extract()?,
        alt,
    )))
}
#[pyclass]
struct Triangles {
    source: Py<PyIterator>,
}
#[pymethods]
impl Triangles {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__<'p>(&self, py: Python<'p>) -> PyResult<Option<Obj<'p>>> {
        for tri in self.source.bind(py).clone() {
            let tri = tri?;
            let a = xyz(&tri.get_item(0)?)?;
            let b = xyz(&tri.get_item(1)?)?;
            let c = xyz(&tri.get_item(2)?)?;
            if (b - a).cross(c - a).length() * 0.5 >= 1e-9 {
                return Ok(Some(tri));
            }
        }
        Ok(None)
    }
}
#[pyfunction]
fn export_triangles(face: &Obj<'_>) -> PyResult<Triangles> {
    Ok(Triangles {
        source: face.call_method0("triangulate")?.try_iter()?.unbind(),
    })
}
#[pyfunction]
fn _worn_outright<'p>(attrs: &Obj<'p>) -> PyResult<Obj<'p>> {
    let back = get(attrs, "back")?;
    if back.is_instance_of::<PyDict>() {
        let has = attrs
            .py()
            .import("core.materials")?
            .getattr("has_own_material")?;
        if has.call1((&back,))?.is_truthy()? && !has.call1((attrs,))?.is_truthy()? {
            return Ok(back);
        }
    }
    Ok(attrs.clone())
}
#[pyfunction]
#[pyo3(signature=(face,materials,prims,attrs=None,smoother=None))]
fn _add_face<'p>(
    face: &Obj<'p>,
    materials: &Bound<'p, PyDict>,
    prims: &Bound<'p, PyDict>,
    attrs: Option<&Obj<'p>>,
    smoother: Option<&Obj<'p>>,
) -> PyResult<()> {
    let py = face.py();
    let attrs = _worn_outright(&match attrs {
        Some(a) => a.clone(),
        None => face.getattr("attrs")?,
    })?;
    let smoother = match smoother {
        Some(s) => s.clone(),
        None => Py::new(py, smooth::SmoothNormals::new(py))?
            .into_bound(py)
            .into_any(),
    };
    let normal = smoother.call_method1("flat", (face,))?;
    let smooth = smoother
        .call_method1("corners", (face,))?
        .cast_into::<PyDict>()?;
    let key = material_key(&attrs, true)?;
    let recipe = get(&attrs, "pbr")?;
    let tex = truth(get(&attrs, "texture")?, PyDict::new(py).into_any())?;
    let source = get(&tex, "path")?;
    let mapped = source.is_truthy()?
        || (recipe.is_truthy()?
            && recipe.try_iter()?.any(|k| {
                k.and_then(|s| s.extract::<String>())
                    .is_ok_and(|k| k.ends_with("_map"))
            }));
    let info = if let Some(i) = materials.get_item(&key)? {
        i.cast_into::<PyDict>()?
    } else {
        let info = PyDict::new(py);
        let white = (1., 1., 1.);
        let col = if mapped && recipe.is_none() {
            white.into_pyobject(py)?
        } else {
            rgb(&attrs, if recipe.is_none() { CREAM } else { white }, true)?
        };
        info.set_item("color", col)?;
        info.set_item("map", py.None())?;
        if mapped {
            let src = if source.is_truthy()? {
                path(&source)?
            } else {
                py.None().into_bound(py)
            };
            if !src.is_none() {
                info.set_item("map", src.getattr("name")?)?
            }
            info.set_item("src", src)?;
        }
        info.set_item("pbr", &recipe)?;
        info.set_item("opacity", opacity(&attrs)?)?;
        materials.set_item(&key, &info)?;
        info
    };
    let mat = get(&attrs, "mat")?;
    if mat.is_truthy()? && !info.contains("mat")? {
        info.set_item("mat", mat)?
    }
    let uvw = get(&tex, "uvw")?;
    let options = if mapped {
        let one = 1f64.into_pyobject(py)?.into_any();
        Some((
            truth(tex.call_method1("get", ("sw", &one))?, one.clone())?,
            truth(tex.call_method1("get", ("sh", &one))?, one)?,
            py.import("builtins")?
                .call_method1("float", (tex.call_method1("get", ("rot", 0.))?,))?,
        ))
    } else {
        None
    };
    let triangles = export_triangles(face)?;
    while let Some(tri) = triangles.__next__(py)? {
        let pts = tri.try_iter()?.collect::<PyResult<Vec<_>>>()?;
        let pts = PyList::new(py, pts)?;
        let uv = if let Some((sw, sh, rot)) = &options {
            let tex = py.import("core.texture")?;
            Some(if uvw.is_truthy()? {
                tex.call_method1("affine_uv", (&uvw, &pts))?
            } else {
                tex.call_method1("planar_uv", (&normal, &pts, sw, sh, rot))?
            })
        } else {
            None
        };
        let verts = PyList::empty(py);
        for (i, p) in pts.iter().enumerate() {
            let n = if smooth.is_empty() {
                normal.clone()
            } else {
                smooth
                    .get_item(p.call_method0("toTuple")?)?
                    .unwrap_or_else(|| normal.clone())
            };
            let texcoord = if let Some(uv) = &uv {
                let coord = uv.get_item(i)?;
                (coord.get_item(0)?, coord.get_item(1)?)
                    .into_pyobject(py)?
                    .into_any()
            } else {
                py.None().into_bound(py)
            };
            verts.append((p, texcoord, n))?;
        }
        let group = if let Some(l) = prims.get_item(&key)? {
            l.cast_into::<PyList>()?
        } else {
            let l = PyList::empty(py);
            prims.set_item(&key, &l)?;
            l
        };
        group.append((&normal, verts))?;
    }
    Ok(())
}
#[pyfunction]
fn _resolve_finishes(scene: &Obj<'_>, materials: &Bound<'_, PyDict>) -> PyResult<()> {
    let py = scene.py();
    let resolve = py.import("core.finish")?.getattr("resolve")?;
    let registry = truth(attr(scene, "materials")?, PyDict::new(py).into_any())?;
    for (_, info) in materials.iter() {
        let mat = get(&info, "mat")?;
        let chosen = if mat.is_truthy()? {
            attr(&registry.call_method1("get", (&mat,))?, "finish")?
        } else {
            py.None().into_bound(py)
        };
        info.set_item(
            "finish",
            resolve.call1((chosen, mat, get(&info, "map")?, get(&info, "opacity")?))?,
        )?;
    }
    Ok(())
}
fn collect<'p>(scene: &Obj<'p>, camera: Option<&Obj<'p>>, split: bool) -> PyResult<Obj<'p>> {
    let py = scene.py();
    let materials = PyDict::new(py);
    let prims = PyDict::new(py);
    let normals = Py::new(py, smooth::SmoothNormals::new(py))?
        .into_bound(py)
        .into_any();
    let faces = Py::new(py, world::world_faces_painted(scene, camera, !split)?)?.into_bound(py);
    for pair in faces.try_iter()? {
        let pair = pair?;
        _add_face(
            &pair.get_item(0)?,
            &materials,
            &prims,
            Some(&pair.get_item(1)?),
            Some(&normals),
        )?;
    }
    let figures = PyList::empty(py);
    if split {
        let camera = camera
            .ok_or_else(|| pyo3::exceptions::PyTypeError::new_err("expected a camera direction"))?;
        for row in world::faceme_figures(scene, camera)?.iter() {
            let own = PyDict::new(py);
            for f in row.get_item(0)?.try_iter()? {
                _add_face(&f?, &materials, &own, None, Some(&normals))?
            }
            if !own.is_empty() {
                let item = PyDict::new(py);
                item.set_item("prims", own)?;
                item.set_item("feet", row.get_item(1)?)?;
                item.set_item("yaw", row.get_item(2)?)?;
                figures.append(item)?;
            }
        }
    }
    _resolve_finishes(scene, &materials)?;
    Ok(if split {
        (materials, prims, figures).into_pyobject(py)?.into_any()
    } else {
        (materials, prims).into_pyobject(py)?.into_any()
    })
}
#[pyfunction]
#[pyo3(signature=(scene,face_me=None))]
fn collect_geometry<'p>(scene: &Obj<'p>, face_me: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    collect(scene, face_me, false)
}
#[pyfunction]
fn collect_geometry_split<'p>(scene: &Obj<'p>, face_me: &Obj<'p>) -> PyResult<Obj<'p>> {
    collect(scene, Some(face_me), true)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    smooth::register(m)?;
    world::register(m)?;
    m.add_function(wrap_pyfunction!(material_key, m)?)?;
    m.add_function(wrap_pyfunction!(export_names, m)?)?;
    m.add_function(wrap_pyfunction!(geolocation, m)?)?;
    m.add_function(wrap_pyfunction!(export_triangles, m)?)?;
    m.add_function(wrap_pyfunction!(_worn_outright, m)?)?;
    m.add_function(wrap_pyfunction!(_add_face, m)?)?;
    m.add_function(wrap_pyfunction!(_resolve_finishes, m)?)?;
    m.add_function(wrap_pyfunction!(collect_geometry, m)?)?;
    m.add_function(wrap_pyfunction!(collect_geometry_split, m)?)
}
