// SPDX-License-Identifier: GPL-3.0-or-later
//! Document visibility queries against fixed geometry, lazy instances and section cuts.
use super::*;
use crate::math::Vec3;
use crate::pick::P;
use glam::DVec3;
use numpy::{PyReadonlyArray1, PyReadonlyArray2};
use pyo3::types::PySlice;
fn host(py: Python) -> PyResult<Bound<PyModule>> {
    py.import("views.document_interaction")
}
fn kw<'py>(py: Python<'py>, s: &str, x: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let kw = PyDict::new(py);
    kw.set_item(s, x)?;
    Ok(kw)
}
fn index<'py>(v: &Obj<'py>, skip: bool) -> PyResult<Obj<'py>> {
    let py = v.py();
    let n = if skip {
        "skip".into_pyobject(py)?.into_any()
    } else {
        none(py)
    };
    v.call_method("_pick_index", (), Some(&kw(py, "near", &n)?))
}
fn lazy<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    match v.getattr_opt("_lazy_native")? {
        Some(f) => f.call0(),
        None => Ok(none(v.py())),
    }
}
fn cut<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    host(v.py())?.call_method1("_active_cut", (v.getattr("scene")?,))
}
fn transparent(v: &Obj) -> PyResult<bool> {
    let mode = v.call_method0("_effective_style")?.getattr("face_mode")?;
    Ok(mode.eq("xray")? || mode.eq("wireframe")?)
}
fn array_points(a: &Obj) -> PyResult<Vec<P>> {
    let arr = a
        .py()
        .import("numpy")?
        .call_method1("asarray", (a, "float64"))?
        .call_method1("reshape", (-1, 3))?
        .extract::<PyReadonlyArray2<f64>>()?;
    Ok(arr
        .as_array()
        .rows()
        .into_iter()
        .map(|r| [r[0], r[1], r[2]])
        .collect())
}
fn coords(worlds: &Obj) -> PyResult<Vec<P>> {
    if worlds.is_instance(&worlds.py().import("numpy")?.getattr("ndarray")?)? {
        array_points(worlds)
    } else {
        worlds
            .try_iter()?
            .map(|w| Ok(xyz(&w?)?.to_array()))
            .collect()
    }
}
fn matrix3<'py>(py: Python<'py>, rows: &[P]) -> PyResult<Obj<'py>> {
    matrix(py, rows.iter().flatten().copied().collect())
}
fn vec_handle<'py>(py: Python<'py>, x: DVec3) -> PyResult<Obj<'py>> {
    Ok(Bound::new(py, Vec3::from(x))?.into_any())
}
#[pyfunction]
fn occluded_on_screen(v: &Obj, world: &Obj) -> PyResult<bool> {
    let snap = attr(v, "_depth_snap")?;
    if !snap.is_none() {
        return Ok(!crate::annotation_geometry::snapshot_point(
            &snap,
            xyz(world)?,
        )?);
    }
    let cam = v.getattr("camera")?;
    if !cam.getattr("perspective")?.is_truthy()? && cut(v)?.is_none() && !transparent(v)? {
        let f = cam.call_method0("forward")?;
        let ray = v.py().import("ingetrazo_render")?.call_method1(
            "parallel_annotation_ray",
            (
                cam.call_method0("eye")?.call_method0("toTuple")?,
                f.call_method0("toTuple")?,
                world.call_method0("toTuple")?,
            ),
        )?;
        if ray.is_none() {
            return Ok(false);
        }
        let o = vec_handle(v.py(), DVec3::from_array(ray.get_item(0)?.extract()?))?;
        return Ok(!v
            .call_method1("ray_distance", (o, f, ray.get_item(1)?))?
            .is_none());
    }
    v.call_method1("_is_occluded", (world,))?.extract()
}
#[pyfunction]
fn is_occluded(v: &Obj, world: &Obj) -> PyResult<bool> {
    let section = cut(v)?;
    if !section.is_none() && section.call_method1("side", (world,))?.extract::<f64>()? > 1e-6 {
        return Ok(true);
    }
    if transparent(v)? {
        return Ok(false);
    }
    if section.is_none() && !lazy(v)?.is_none() {
        return v
            .call_method1("_occluded_many", (PyList::new(v.py(), [world])?,))?
            .get_item(0)?
            .extract();
    }
    let idx = v.call_method0("_pick_index")?;
    if attr(&idx, "tri_v0")?.is_none() && !attr(&idx, "own_spans")?.is_truthy()? {
        return Ok(false);
    }
    let o = v.getattr("camera")?.call_method0("eye")?;
    let origin = xyz(&o)?;
    let delta = xyz(world)? - origin;
    let len = delta.length();
    if len < 1e-9 {
        return Ok(false);
    }
    let dir = delta / len;
    let args = (&idx, &o, vec_handle(v.py(), dir)?, idx.getattr("ent_vis")?);
    let section = cut(v)?;
    let limit = len - 0.001;
    if section.is_none() {
        let kw = PyDict::new(v.py());
        kw.set_item("reduce_global", true)?;
        return Ok(v
            .call_method("_ray_hits", args, Some(&kw))?
            .extract::<Option<f64>>()?
            .is_some_and(|n| n < limit));
    }
    let depths = v.call_method1("_ray_hits", args)?;
    if depths.is_none() {
        return Ok(false);
    }
    let n = xyz(&section.getattr("normal")?)?;
    let c = n.dot(xyz(&section.getattr("point")?)?);
    Ok(depths
        .extract::<PyReadonlyArray1<f64>>()?
        .as_array()
        .iter()
        .any(|&t| t.is_finite() && t < limit && (origin + dir * t).dot(n) - c <= 1e-6))
}
#[pyfunction]
fn occluded_memo_key<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let m = host(py)?;
    if !m.getattr("_OCCLUSION_MEMO")?.is_truthy()? || !cut(v)?.is_none() {
        return Ok(none(py));
    }
    let nat = lazy(v)?;
    if nat.is_none() || nat.get_item(2)?.len()? == 0 {
        return Ok(none(py));
    }
    let idx = index(v, true)?;
    let e = xyz(&v.getattr("camera")?.call_method0("eye")?)?;
    let sc = v.getattr("scene")?;
    let key = PyList::empty(py);
    for n in [e.x, e.y, e.z] {
        key.append(n)?
    }
    for o in [&idx, &nat.get_item(0)?, &sc] {
        key.append(o.as_ptr() as usize)?
    }
    key.append(sc.getattr("version")?)?;
    for name in ["_cache_ver", "_content_ver"] {
        key.append(m.call_method1(name, (v,))?)?
    }
    key.append(v.call_method0("_placements_epoch")?)?;
    key.append(sc.getattr("edit_group")?.as_ptr() as usize)?;
    key.append(v.call_method0("_rest_is_hidden")?)?;
    key.append(v.call_method0("_effective_style")?.getattr("face_mode")?)?;
    Ok(key.to_tuple().into_any())
}
#[pyfunction]
fn occluded_many<'py>(v: &Obj<'py>, worlds: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    if worlds.len()? == 0 {
        return Ok(PyList::empty(py).into_any());
    }
    let key = v.call_method0("_occluded_memo_key")?;
    if key.is_none() {
        return v.call_method1("_occluded_many_now", (worlds,));
    }
    let memo = attr(v, "_occluded_memo")?;
    let known =
        if memo.is_none() || !memo.get_item(0)?.eq(&key)? || memo.get_item(1)?.len()? > 400000 {
            let d = PyDict::new(py);
            v.setattr("_occluded_memo", (key, &d))?;
            d.into_any()
        } else {
            memo.get_item(1)?
        };
    let points = coords(worlds)?;
    let mut keys = Vec::new();
    let mut ask = Vec::new();
    let mut rows = Vec::new();
    for (i, &p) in points.iter().enumerate() {
        let k = triple(p).into_pyobject(py)?;
        if !known.contains(&k)? {
            ask.push(i);
            rows.push(p)
        }
        keys.push(k)
    }
    if !ask.is_empty() {
        let values = v.call_method1("_occluded_many_now", (matrix3(py, &rows)?,))?;
        for (i, h) in ask.into_iter().zip(values.try_iter()?) {
            known.set_item(&keys[i], h?.is_truthy()?)?
        }
    }
    Ok(PyList::new(
        py,
        keys.iter()
            .map(|k| known.get_item(k))
            .collect::<PyResult<Vec<_>>>()?,
    )?
    .into_any())
}
#[pyfunction]
fn first_clear<'py>(v: &Obj<'py>, pts: &Obj<'py>) -> PyResult<Option<isize>> {
    let py = v.py();
    let m = host(py)?;
    if !m.getattr("_FIRST_CLEAR")?.is_truthy()? || pts.len()? == 0 {
        return Ok(None);
    }
    let nat = lazy(v)?;
    if nat.is_none() || !nat.get_item(0)?.hasattr("first_clear")? || !cut(v)?.is_none() {
        return Ok(None);
    }
    let hypot = m.call_method0("_qv_hypot3")?;
    if !hypot.is_truthy()? {
        return Ok(None);
    }
    if transparent(v)? {
        return Ok(Some(0));
    }
    let idx = index(v, true)?;
    let mut set = none(py);
    let mut stat_rows = none(py);
    if !attr(&idx, "tri_v0")?.is_none() {
        let stat = v.call_method1("_static_occluders", (idx,))?;
        if stat.is_none() {
            return Ok(None);
        }
        set = stat.get_item(0)?;
        stat_rows = stat.get_item(1)?
    }
    let eye = v
        .getattr("camera")?
        .call_method0("eye")?
        .call_method0("toTuple")?;
    let np = py.import("numpy")?;
    let pts = np
        .call_method1("ascontiguousarray", (pts, "float64"))?
        .call_method1("reshape", (-1, 3))?;
    let (first, doubt): (isize, Obj) = nat
        .get_item(0)?
        .call_method1(
            "first_clear",
            (
                np.call_method1("ascontiguousarray", (nat.get_item(2)?, "int64"))?,
                eye,
                &pts,
                set,
                stat_rows,
                hypot,
            ),
        )?
        .extract()?;
    for j in doubt.extract::<Vec<isize>>()? {
        if !v
            .call_method1(
                "_occluded_many_now",
                (pts.get_item(PySlice::new(py, j, j + 1, 1))?,),
            )?
            .get_item(0)?
            .is_truthy()?
        {
            return Ok(Some(j));
        }
    }
    Ok(Some(first))
}
mod batch;

pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(occluded_on_screen, m)?)?;
    m.add_function(wrap_pyfunction!(is_occluded, m)?)?;
    m.add_function(wrap_pyfunction!(occluded_many, m)?)?;
    m.add_function(wrap_pyfunction!(occluded_memo_key, m)?)?;
    m.add_function(wrap_pyfunction!(first_clear, m)?)?;
    batch::register(m)
}
