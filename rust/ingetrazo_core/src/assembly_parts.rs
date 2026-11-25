// SPDX-License-Identifier: GPL-3.0-or-later
//! Measure assembly parts without building editable topology for deferred meshes.
use numpy::PyReadonlyArray2;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
type Obj<'p> = Bound<'p, PyAny>;
type Cache<'p> = Option<&'p Bound<'p, PyDict>>;

fn plan<'p>(mesh: &Obj<'p>) -> PyResult<Obj<'p>> {
    mesh.py()
        .import("ingetrazo_core")?
        .call_method1("plan_of", (mesh,))
}
#[pyfunction(signature=(part, cache=None))]
fn part_points<'p>(part: &Obj<'p>, cache: Cache<'p>) -> PyResult<Obj<'p>> {
    let py = part.py();
    let np = py.import("numpy")?;
    let groups = py.import("core.group")?;
    let chunks = PyList::empty(py);
    for row in groups
        .call_method1("iter_placements", (part,))?
        .try_iter()?
    {
        let row = row?;
        let mesh = row.get_item(0)?.getattr("mesh")?;
        let pose = row.get_item(1)?;
        let key = ("points", mesh.as_ptr() as usize);
        let pos = match cache.map(|c| c.get_item(key)).transpose()?.flatten() {
            Some(hit) => hit,
            None => {
                let plan = plan(&mesh)?;
                let pos = if plan.is_none() {
                    let points = mesh
                        .getattr("vertices")?
                        .try_iter()?
                        .map(|v| v?.getattr("position")?.call_method0("toTuple"))
                        .collect::<PyResult<Vec<_>>>()?;
                    np.call_method1("array", (points, "float64"))?
                        .call_method1("reshape", (-1, 3))?
                } else {
                    plan.getattr("pos")?
                };
                if let Some(c) = cache {
                    c.set_item(key, &pos)?;
                }
                pos
            }
        };
        if pos.is_empty()? {
            continue;
        }
        if pose.is_none() {
            chunks.append(pos)?;
        } else {
            let matrix = crate::model::module(groups.py(), "ingetrazo_core")?.call_method1("group_np_affine", (pose,))?;
            let placed = pos
                .call_method1("__matmul__", (matrix.get_item(0)?.getattr("T")?,))?
                .add(matrix.get_item(1)?)?;
            chunks.append(placed)?;
        }
    }
    if chunks.is_empty() {
        np.call_method1("empty", ((0, 3),))
    } else {
        np.call_method1("concatenate", (chunks,))
    }
}

fn extent(points: &[glam::DVec3]) -> glam::DVec3 {
    let mut min = points[0];
    let mut max = min;
    for &p in &points[1..] {
        min = min.min(p);
        max = max.max(p);
    }
    max - min
}
#[pyfunction]
fn part_size(points: PyReadonlyArray2<'_, f64>) -> PyResult<(f64, f64, f64)> {
    use glam::{DMat3, DVec3};
    let py = points.py();
    let array = points.as_array();
    if array.ncols() != 3 {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "expected a three-coordinate point array",
        ));
    }
    let points: Vec<_> = array
        .rows()
        .into_iter()
        .map(|row| DVec3::new(row[0], row[1], row[2]))
        .collect();
    if points.is_empty() {
        return Ok((0., 0., 0.));
    }
    let mut best = extent(&points);
    if points.len() >= 4 {
        let centre = points.iter().sum::<DVec3>() / points.len() as f64;
        let mut covariance = [[0.; 3]; 3];
        for p in &points {
            let v = (*p - centre).to_array();
            for i in 0..3 {
                for j in 0..3 {
                    covariance[i][j] += v[i] * v[j];
                }
            }
        }
        // Use the same symmetric eigensolver as the other matrix measurements.
        let pair = py
            .import("numpy.linalg")?
            .call_method1("eigh", (covariance,))?;
        let weights: [f64; 3] = pair.get_item(0)?.extract()?;
        let rows: [[f64; 3]; 3] = pair.get_item(1)?.extract()?;
        let basis = DMat3::from_cols(rows[0].into(), rows[1].into(), rows[2].into());
        let local: Vec<_> = points.iter().map(|&p| basis * (p - centre)).collect();
        let mut own = extent(&local);
        if weights[1] > 0.
            && weights[2] > 1.01 * weights[1]
            && (weights[1] - weights[0]).abs() <= 1e-10 * weights[1]
        {
            // A square section has no unique transverse eigenvectors. Fit its hull.
            let flat = local.iter().map(|v| [v.x, v.y]).collect::<Vec<_>>();
            if let Some([a, b]) = crate::model::group_frame::frame_axis(&flat) {
                let aligned = local
                    .iter()
                    .map(|v| DVec3::new(a * v.x + b * v.y, -b * v.x + a * v.y, v.z))
                    .collect::<Vec<_>>();
                own = extent(&aligned);
            }
        }
        if own.element_product() < 0.8 * best.element_product() {
            best = own;
        }
    }
    let mut dims = best.to_array();
    dims.sort_by(|a, b| b.total_cmp(a));
    Ok((dims[0], dims[1], dims[2]))
}

fn label(attrs: &Obj) -> PyResult<Option<String>> {
    if !attrs.is_truthy()? {
        return Ok(None);
    }
    let get = |key: &str| attrs.call_method1("get", (key,));
    let mat = get("mat")?;
    if mat.is_truthy()? {
        return Ok(Some(mat.str()?.to_string()));
    }
    let tex = get("texture")?;
    if tex.is_truthy()? {
        let path = tex.call_method1("get", ("path",))?;
        if path.is_truthy()? {
            let stem = attrs
                .py()
                .import("pathlib")?
                .call_method1("PurePath", (path.str()?,))?
                .getattr("stem")?
                .extract()?;
            return Ok(Some(stem));
        }
    }
    let col = get("color")?;
    if col.is_none() {
        return Ok(None);
    }
    let rgb = col
        .try_iter()?
        .take(3)
        .map(|c| Ok((c?.extract::<f64>()? * 255.).round_ties_even() as i64))
        .collect::<PyResult<Vec<_>>>()?;
    let [r, g, b] = rgb[..] else {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "expected RGB colour",
        ));
    };
    Ok(Some(format!("#{r:02x}{g:02x}{b:02x}")))
}
fn add_area(areas: &Bound<PyDict>, name: String, area: f64) -> PyResult<()> {
    let old = areas
        .get_item(&name)?
        .map(|n| n.extract::<f64>())
        .transpose()?
        .unwrap_or(0.);
    areas.set_item(name, old + area)
}
fn paint_areas<'p>(mesh: &Obj<'p>, cache: Cache<'p>) -> PyResult<Obj<'p>> {
    let key = ("paint", mesh.as_ptr() as usize);
    if let Some(hit) = cache.map(|c| c.get_item(key)).transpose()?.flatten() {
        return Ok(hit);
    }
    let py = mesh.py();
    let areas = PyDict::new(py);
    let mut default = 0.;
    let painted = py.import("core.materials")?.getattr("has_own_material")?;
    let mut count = |attrs: Obj<'p>, area: f64| -> PyResult<()> {
        if painted.call1((&attrs,))?.is_truthy()? {
            if let Some(name) = label(&attrs)? {
                add_area(&areas, name, area)?;
            }
        } else {
            default += area;
        }
        Ok(())
    };
    let plan = plan(mesh)?;
    if plan.is_none() {
        for face in mesh.getattr("faces")?.try_iter()? {
            let face = face?;
            count(
                face.getattr("attrs")?,
                face.call_method0("net_area")?.extract()?,
            )?;
        }
    } else {
        let source = mesh.getattr("__dict__")?.get_item("_deferred_source")?;
        let decode = py.import("formats.igz")?.getattr("_face_attrs_from_json")?;
        let table = PyList::empty(py);
        for row in source
            .get_item("meta")?
            .call_method1("get", ("face_attrs", PyList::empty(py)))?
            .try_iter()?
        {
            table.append(decode.call1((row?,))?)?;
        }
        let sizes = py
            .import("core.lazy_mesh")?
            .call_method1("face_net_areas", (plan,))?;
        let empty = PyDict::new(py).into_any();
        for (key, size) in source
            .get_item("face_attr")?
            .try_iter()?
            .zip(sizes.try_iter()?)
        {
            let index = key?.extract::<isize>()?;
            let attrs = if index < 0 {
                empty.clone()
            } else {
                table.get_item(index as usize)?
            };
            count(attrs, size?.extract()?)?;
        }
    }
    let hit = (areas, default).into_pyobject(py)?.into_any();
    if let Some(c) = cache {
        c.set_item(key, &hit)?;
    }
    Ok(hit)
}
#[pyfunction(signature=(part,cache=None))]
fn part_material<'p>(part: &Obj<'p>, cache: Cache<'p>) -> PyResult<String> {
    let py = part.py();
    let area = PyDict::new(py);
    for item in py
        .import("ingetrazo_core")?
        .call_method1("group_iter_painted_placements", (part,))?
        .try_iter()?
    {
        let row = item?;
        let paint = paint_areas(&row.get_item(0)?.getattr("mesh")?, cache)?;
        for (label, size) in paint.get_item(0)?.cast::<PyDict>()? {
            add_area(&area, label.extract()?, size.extract()?)?;
        }
        let default = paint.get_item(1)?.extract::<f64>()?;
        if default != 0. {
            if let Some(name) = label(&row.get_item(2)?)? {
                add_area(&area, name, default)?;
            }
        }
    }
    let mut best = String::new();
    let mut largest = f64::NEG_INFINITY;
    for (label, amount) in area {
        let amount = amount.extract()?;
        if amount > largest {
            best = label.extract()?;
            largest = amount;
        }
    }
    Ok(best)
}
#[pyfunction(signature=(part,cache=None))]
fn part_row<'p>(part: &Obj<'p>, cache: Cache<'p>) -> PyResult<Bound<'p, PyDict>> {
    let py = part.py();
    let out = PyDict::new(py);
    out.set_item("part", part)?;
    out.set_item("name", part.getattr("name")?)?;
    out.set_item(
        "hidden",
        part.getattr_opt("hidden")?
            .is_some_and(|v| v.is_truthy().unwrap_or(false)),
    )?;
    out.set_item("material", part_material(part, cache)?)?;
    out.set_item("size", part_size(part_points(part, cache)?.extract()?)?)?;
    let face_count = py.import("ingetrazo_core.mesh")?.getattr("face_count")?;
    let mut faces = 0usize;
    for row in py
        .import("core.group")?
        .call_method1("iter_placements", (part,))?
        .try_iter()?
    {
        faces += face_count
            .call1((row?.get_item(0)?.getattr("mesh")?,))?
            .extract::<usize>()?;
    }
    out.set_item("faces", faces)?;
    Ok(out)
}
#[pyfunction]
fn part_rows<'p>(container: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let py = container.py();
    let rows = PyList::empty(py);
    if let Some(children) = container.getattr_opt("children")?.filter(|v| !v.is_none()) {
        let cache = PyDict::new(py);
        for part in children.try_iter()? {
            rows.append(part_row(&part?, Some(&cache))?)?;
        }
    }
    Ok(rows)
}
#[pyfunction]
fn is_surface(size: [f64; 3]) -> bool {
    size[2] < 0.0005
}
pub(crate) fn register(root: &Bound<PyModule>) -> PyResult<()> {
    let py = root.py();
    let name = "ingetrazo_core.parts";
    let m = PyModule::new(py, name)?;
    for f in [
        wrap_pyfunction!(part_points, &m)?,
        wrap_pyfunction!(part_size, &m)?,
        wrap_pyfunction!(part_material, &m)?,
        wrap_pyfunction!(part_row, &m)?,
        wrap_pyfunction!(part_rows, &m)?,
        wrap_pyfunction!(is_surface, &m)?,
    ] {
        m.add_function(f)?;
    }
    py.import("sys")?.getattr("modules")?.set_item(name, &m)?;
    root.add("parts", m)
}
