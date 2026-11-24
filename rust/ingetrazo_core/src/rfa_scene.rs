// SPDX-License-Identifier: GPL-3.0-or-later
//! Install Revit family geometry and materials. The IFC record belongs to the host.
use glam::DVec3;
use pyo3::{
    exceptions::{PyImportError, PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
type Obj<'p> = Bound<'p, PyAny>;
fn val<'p>(d: &Obj<'p>, k: &str) -> PyResult<Obj<'p>> {
    d.call_method1("get", (k,))
}
fn rows<'p>(d: &Obj<'p>, k: &str) -> PyResult<Obj<'p>> {
    let v = val(d, k)?;
    Ok(if v.is_none() {
        PyList::empty(d.py()).into_any()
    } else {
        v
    })
}
fn dict<'p>(d: &Obj<'p>, k: &str) -> PyResult<Bound<'p, PyDict>> {
    let v = val(d, k)?;
    if v.is_none() {
        Ok(PyDict::new(d.py()))
    } else {
        Ok(v.cast_into()?)
    }
}
fn tr<'p>(py: Python<'p>, msg: &str) -> PyResult<Obj<'p>> {
    py.import("core.i18n")?.call_method1("tr", (msg,))
}
fn invalid(py: Python<'_>, msg: &str) -> PyErr {
    tr(py, msg)
        .and_then(|s| Ok(PyValueError::new_err(s.str()?.to_string())))
        .unwrap_or_else(|e| e)
}
#[pyfunction]
fn rfa_read<'p>(py: Python<'p>, path: &Obj<'p>) -> PyResult<Obj<'p>> {
    let reader = match py.import("ingetrazo_rfa") {
        Err(e) if e.is_instance_of::<PyImportError>(py) => {
            let err = PyRuntimeError::new_err(
                tr(
                    py,
                    "The Revit family reader is not built. Run scripts/build_native.sh.",
                )?
                .str()?
                .to_string(),
            );
            err.set_cause(py, Some(e));
            return Err(err);
        }
        other => other?,
    };
    let text = reader.call_method1("read_family", (path.str()?,))?;
    py.import("json")?.call_method1("loads", (text,))
}
#[pyfunction]
fn rfa_triangle_mesh<'p>(
    py: Python<'p>,
    positions: &Obj<'p>,
    indices: &Obj<'p>,
) -> PyResult<Obj<'p>> {
    let np = py.import("numpy")?;
    let flat = |o: &Obj<'p>, dtype: &str| -> PyResult<Obj<'p>> {
        np.call_method1("asarray", (o, np.getattr(dtype)?))?
            .call_method1("reshape", (-1, 3))
    };
    let bad = || invalid(py, "Invalid Revit triangle mesh.");
    let positions = flat(positions, "float64").map_err(|_| bad())?;
    let indices = flat(indices, "int64").map_err(|_| bad())?;
    let pos = positions
        .extract::<numpy::PyReadonlyArray2<f64>>()
        .map_err(|_| bad())?;
    let idx = indices
        .extract::<numpy::PyReadonlyArray2<i64>>()
        .map_err(|_| bad())?;
    let pts = pos.as_array();
    if !pts.iter().all(|v| v.is_finite()) {
        return Err(bad());
    }
    let mut verts = vec![];
    for row in idx.as_array().outer_iter() {
        let mut tri = [DVec3::ZERO; 3];
        for (i, &j) in row.iter().enumerate() {
            let j = usize::try_from(j)
                .ok()
                .filter(|&j| j < pts.nrows())
                .ok_or_else(|| invalid(py, "Invalid Revit triangle index."))?;
            tri[i] = DVec3::new(pts[[j, 0]], pts[[j, 1]], pts[[j, 2]]);
        }
        if (tri[1] - tri[0]).cross(tri[2] - tri[0]).length() > 1e-12 {
            verts.extend(tri.into_iter().map(|v| v.to_array()));
        }
    }
    let mesh = crate::file_model::new_mesh(py)?;
    let n = verts.len() / 3;
    if n > 0 {
        use numpy::IntoPyArray;
        let arr = numpy::ndarray::Array2::from_shape_vec(
            (verts.len(), 3),
            verts.into_iter().flatten().collect(),
        )
        .unwrap()
        .into_pyarray(py);
        let (v, inv) = mesh
            .call_method1("bulk_weld", (arr,))?
            .extract::<(Obj, Obj)>()?;
        mesh.call_method1(
            "add_faces_welded",
            (v, inv, vec![3; n], vec![1; n], vec![None::<i32>; n]),
        )?;
        py.import("formats.fuse")?
            .call_method1("soften_smooth_edges", (&mesh,))?;
    }
    Ok(mesh)
}
fn group<'p>(name: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = name.py();
    let kw = PyDict::new(py);
    kw.set_item("name", name)?;
    py.import("core.group")?.call_method("Group", (), Some(&kw))
}
fn placeholder<'p>(
    parameters: &Obj<'p>,
    warnings: &Bound<'p, PyList>,
    name: &Obj<'p>,
) -> PyResult<Obj<'p>> {
    let py = name.py();
    let mut dims = [1.; 3];
    let translator = py.import("core.i18n")?.getattr("tr")?;
    let kw = PyDict::new(py);
    kw.set_item("name", name)?;
    for (i, axis) in ["Width", "Depth", "Height"].into_iter().enumerate() {
        let mut found = None;
        for p in parameters.try_iter()? {
            let p = p?;
            if p.get_item("name")?.eq(axis)? {
                found = p
                    .get_item("value")
                    .and_then(|v| {
                        py.get_type::<pyo3::types::PyFloat>()
                            .call1((v,))?
                            .extract::<f64>()
                    })
                    .ok()
                    .map(|v| v / 1000.)
                    .filter(|v| v.is_finite() && *v > 0.);
                break;
            }
        }
        if let Some(v) = found {
            dims[i] = v;
        } else {
            kw.set_item("axis", axis)?;
            warnings.append(translator.call(
                ("{name}: missing or invalid {axis}; using 1 m.",),
                Some(&kw),
            )?)?;
        }
    }
    let g = group(&tr(py, "Placeholder")?)?;
    let mesh = g.getattr("mesh")?;
    let points: Vec<_> = (0..8)
        .map(|i| {
            DVec3::new(
                if i & 1 != 0 { dims[0] } else { 0. },
                if i & 2 != 0 { dims[1] } else { 0. },
                if i & 4 != 0 { dims[2] } else { 0. },
            )
        })
        .collect();
    for face in [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ] {
        mesh.call_method1(
            "add_face",
            (crate::file_model::verts(py, &face.map(|i| points[i]))?,),
        )?;
    }
    g.setattr("component", false)?;
    kw.del_item("axis").ok();
    warnings.append(translator.call(
        ("{name}: no decoded mesh; imported a placeholder box.",),
        Some(&kw),
    )?)?;
    Ok(g)
}
fn material<'p>(
    registry: &Obj<'p>,
    name: &Obj<'p>,
    color: &Obj<'p>,
    opacity: Option<f64>,
) -> PyResult<Obj<'p>> {
    let py = name.py();
    let m = py.import("core.materials")?;
    let kw = PyDict::new(py);
    kw.set_item("opacity", opacity)?;
    let mat = m.call_method("Material", (name, color), Some(&kw))?;
    registry.get_item(m.call_method1("register", (registry, mat))?)
}
fn clearance<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = scene.py();
    let key = "rfa-clearance-zones";
    let layers = scene.getattr("layers")?;
    for l in layers.try_iter()? {
        let l = l?;
        if l.getattr("key")?.eq(key)? {
            return l.getattr("key");
        }
    }
    let kw = PyDict::new(py);
    kw.set_item("key", key)?;
    let l =
        py.import("core.layers")?
            .call_method("Layer", (tr(py, "Clearance zones")?,), Some(&kw))?;
    layers.call_method1("append", (&l,))?;
    l.getattr("key")
}
#[pyfunction]
fn rfa_apply<'p>(
    py: Python<'p>,
    scene: &Obj<'p>,
    payload: &Obj<'p>,
) -> PyResult<Bound<'p, PyList>> {
    if !val(payload, "schema")?.eq("ingetrazo.rfa/1")? {
        return Err(invalid(py, "Unsupported Revit family JSON schema."));
    }
    let types = rows(payload, "types")?;
    if !types.is_truthy()? {
        return Err(invalid(py, "The Revit family contains no types."));
    }
    let warnings = py
        .get_type::<PyList>()
        .call1((rows(payload, "warnings")?,))?
        .cast_into::<PyList>()?;
    let family = dict(payload, "family")?;
    let source = dict(payload, "source")?;
    let styles = dict(payload, "styles")?;
    let registry = scene.getattr("materials")?;
    let materials = PyDict::new(py);
    for (key, raw) in dict(payload, "materials")?.iter() {
        let rgb = val(&raw, "rgb")?;
        let rgb = if rgb.is_truthy()? {
            rgb.extract::<Vec<f64>>()?
        } else {
            vec![180.; 3]
        };
        let color = PyTuple::new(py, rgb.into_iter().map(|v| v / 255.))?;
        let alpha = val(&raw, "transparency")?;
        let opacity = if alpha.is_truthy()? {
            Some(
                1. - py
                    .get_type::<pyo3::types::PyFloat>()
                    .call1((alpha,))?
                    .extract::<f64>()?,
            )
        } else {
            None
        };
        materials.set_item(
            key,
            material(&registry, &raw.get_item("name")?, color.as_any(), opacity)?,
        )?;
    }
    let matrix = py.get_type::<crate::math::Matrix4>();
    let defs = PyList::empty(py);
    let copy = py.import("copy")?.getattr("deepcopy")?;
    let mut zone_layer = None;
    for typ in types.try_iter()? {
        let typ = typ?;
        let name = typ.get_item("name")?;
        let def = group(&name)?;
        def.setattr("xform", matrix.call0()?)?;
        let params = rows(&typ, "parameters")?;
        let source_type = typ.get_item("geometry_source")?;
        let meta = PyDict::new(py);
        meta.set_item("family", &family)?;
        meta.set_item("source", &source)?;
        meta.set_item("parameters", &params)?;
        meta.set_item("geometry_source", &source_type)?;
        meta.set_item("preview_png_base64", val(payload, "preview_png_base64")?)?;
        let ext = PyDict::new(py);
        ext.set_item("rfa", copy.call1((meta,))?)?;
        def.setattr("ext", ext)?;
        def.setattr(
            "ifc",
            py.import("formats.rfa")?
                .call_method1("_family_record", (&typ, &family, &source, &params))?,
        )?;
        let solids = PyList::empty(py);
        if source_type.eq("none")? {
            solids.append(placeholder(&params, &warnings, &name)?)?;
        }
        for comp in rows(&typ, "components")?.try_iter()? {
            let comp = comp?;
            let key = comp.get_item("geometry")?;
            let geometry = payload.get_item("geometries")?.get_item(&key)?;
            let values = comp.get_item("transform")?.extract::<Vec<f64>>()?;
            if values.len() != 16 || !values.iter().all(|v| v.is_finite()) {
                return Err(invalid(py, "Invalid Revit component transform."));
            }
            let grouped = PyDict::new(py);
            for part in geometry.get_item("parts")?.try_iter()? {
                let p = part?;
                let zone = val(&p, "zone")?.is_truthy()?;
                let mut mat = materials.get_item(val(&p, "material")?)?;
                if zone {
                    let (name, col) = if let Some(m) = &mat {
                        (m.getattr("name")?.str()?.to_string(), m.getattr("color")?)
                    } else {
                        (
                            "Revit".into(),
                            (0.3, 0.6, 0.9).into_pyobject(py)?.into_any(),
                        )
                    };
                    mat = Some(material(
                        &registry,
                        &format!("{name} — Clearance").into_pyobject(py)?.into_any(),
                        &col,
                        Some(0.2),
                    )?);
                }
                let name = match &mat {
                    Some(m) => m.getattr("name")?,
                    None => geometry.call_method1("get", ("label", "Solid"))?,
                };
                let g = group(&name)?;
                g.setattr("component", false)?;
                g.setattr("hidden", val(&p, "hidden")?.is_truthy()?)?;
                g.setattr(
                    "material",
                    mat.map(|m| m.call_method0("face_attrs")).transpose()?,
                )?;
                if zone {
                    if zone_layer.is_none() {
                        zone_layer = Some(clearance(scene)?);
                    }
                    g.setattr("layer", zone_layer.as_ref())?;
                }
                let id = p.get_item("solid")?;
                let detail = PyDict::new(py);
                detail.set_item("solid", &id)?;
                detail.set_item("style", copy.call1((styles.get_item(val(&p, "style")?)?,))?)?;
                let ext = PyDict::new(py);
                ext.set_item("rfa", detail)?;
                g.setattr("ext", ext)?;
                g.setattr(
                    "mesh",
                    rfa_triangle_mesh(py, &p.get_item("positions")?, &p.get_item("indices")?)?,
                )?;
                grouped
                    .call_method1("setdefault", (id, PyList::empty(py)))?
                    .call_method1("append", (g,))?;
            }
            for (id, parts) in grouped.iter() {
                let first = parts.get_item(0)?;
                let solid = group(&first.getattr("name")?)?;
                solid.call_method1("adopt", (&parts,))?;
                solid.setattr("xform", matrix.call1(PyTuple::new(py, &values)?)?)?;
                solid.setattr("component", false)?;
                let (mut hidden, mut tagged) = (true, true);
                for p in parts.try_iter()? {
                    let p = p?;
                    hidden &= p.getattr("hidden")?.is_truthy()?;
                    tagged &= p.getattr("layer")?.is_truthy()?;
                }
                solid.setattr("hidden", hidden)?;
                if tagged {
                    solid.setattr("layer", first.getattr("layer")?)?;
                }
                let detail = PyDict::new(py);
                detail.set_item("solid", id)?;
                detail.set_item("geometry", &key)?;
                let ext = PyDict::new(py);
                ext.set_item("rfa", detail)?;
                solid.setattr("ext", ext)?;
                solids.append(solid)?;
            }
        }
        def.call_method1("adopt", (solids,))?;
        defs.append(def)?;
    }
    scene
        .getattr("kept_components")?
        .call_method1("extend", (&defs,))?;
    scene.getattr("groups")?.call_method1(
        "append",
        (py.import("core.kept_components")?
            .call_method1("place_copy", (defs.get_item(0)?,))?,),
    )?;
    scene.setattr("version", scene.getattr("version")?.add(1)?)?;
    Ok(warnings)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(rfa_read, m)?)?;
    m.add_function(wrap_pyfunction!(rfa_triangle_mesh, m)?)?;
    m.add_function(wrap_pyfunction!(rfa_apply, m)?)
}
