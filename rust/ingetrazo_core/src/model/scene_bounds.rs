// SPDX-License-Identifier: GPL-3.0-or-later
//! The model's box and the selection's (`Scene.bounds`,
//! `Scene.selection_bounds`): the walk over the loose geometry and the
//! placements, one vertex array per mesh, the placements that cannot widen
//! the box skipped, a lazy mesh's placed box kept between walks, and the
//! box kept across view-only changes. The scene answers visibility through
//! its own methods; the placed boxes are `group_frame::group_placed_box`
//! (`tests/test_bounds_parity.py`, `tests/test_selection_bounds_parity.py`).

use numpy::PyReadonlyArray2;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

type P3 = [f64; 3];

fn float64_rows<'py>(py: Python<'py>, pos: &Bound<'py, PyAny>) -> PyResult<PyReadonlyArray2<'py, f64>> {
    let np = py.import("numpy")?;
    let kw = PyDict::new(py);
    kw.set_item("dtype", np.getattr("float64")?)?;
    Ok(np.getattr("ascontiguousarray")?.call((pos,), Some(&kw))?.extract::<PyReadonlyArray2<'py, f64>>()?)
}

/// A box grown point by point as NumPy's `min` / `max` over a column
/// would: a NaN stays.
struct NanBox {
    lo: P3,
    hi: P3,
    any: bool,
}

impl NanBox {
    fn new() -> Self {
        NanBox { lo: [f64::INFINITY; 3], hi: [f64::NEG_INFINITY; 3], any: false }
    }
    fn add(&mut self, p: P3) {
        self.any = true;
        for k in 0..3 {
            if p[k].is_nan() || self.lo[k].is_nan() {
                self.lo[k] = f64::NAN;
                self.hi[k] = f64::NAN;
            } else {
                if p[k] < self.lo[k] {
                    self.lo[k] = p[k];
                }
                if p[k] > self.hi[k] {
                    self.hi[k] = p[k];
                }
            }
        }
    }
}

fn vec_xyz(p: &Bound<'_, PyAny>) -> PyResult<P3> {
    Ok([p.call_method0("x")?.extract()?, p.call_method0("y")?.extract()?, p.call_method0("z")?.extract()?])
}

fn camera_point<'py>(py: Python<'py>, p: P3) -> PyResult<Bound<'py, PyAny>> {
    py.import("core.camera_point")?.getattr("CameraPoint")?.call1((p[0], p[1], p[2]))
}

fn none_pair(py: Python<'_>) -> PyResult<Bound<'_, PyTuple>> {
    PyTuple::new(py, [py.None(), py.None()])
}

/// `Scene.selection_bounds()`: the box of what is selected, or
/// `(None, None)`; a guide by its point only (a circle by 32 samples).
#[pyfunction]
fn scene_selection_bounds<'py>(py: Python<'py>, scene: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyTuple>> {
    let group = py.import("core.group")?;
    let is_guide = py.import("core.guide")?.getattr("is_guide")?;
    let mesh = py.import("core.mesh")?;
    let (face_cls, edge_cls) = (mesh.getattr("Face")?, mesh.getattr("Edge")?);
    let mut b = NanBox::new();
    for ent in scene.getattr("selection")?.try_iter()? {
        let ent = ent?;
        if is_guide.call1((&ent,))?.is_truthy()? {
            // Before Edge: it has .a / .b
            if ent.getattr_opt("is_circle")?.map(|v| v.is_truthy()).transpose()?.unwrap_or(false) {
                for p in ent.call_method1("sample_points", (32,))?.try_iter()? {
                    b.add(vec_xyz(&p?)?);
                }
            } else {
                b.add(vec_xyz(&ent.getattr("point")?)?);
            }
        } else if ent.hasattr("mesh")? {
            let arr = group.getattr("placement_points")?.call1((&ent,))?;
            if arr.len()? > 0 {
                let a = float64_rows(py, &arr)?;
                // Its own min and max rows, then both as points.
                let mut own = NanBox::new();
                for r in a.as_array().rows() {
                    own.add([r[0], r[1], r[2]]);
                }
                b.add(own.lo);
                b.add(own.hi);
            }
        } else if ent.is_instance(&face_cls)? {
            for v in ent.getattr("loop")?.try_iter()? {
                b.add(v?.call_method0("position_tuple")?.extract()?);
            }
        } else if ent.is_instance(&edge_cls)? {
            b.add(ent.getattr("v0")?.call_method0("position_tuple")?.extract()?);
            b.add(ent.getattr("v1")?.call_method0("position_tuple")?.extract()?);
        } else if ent.hasattr("vertices")? {
            for v in ent.getattr("vertices")?.try_iter()? {
                b.add(vec_xyz(&v?)?);
            }
        } else if ent.hasattr("corners")? {
            for c in ent.call_method0("corners")?.try_iter()? {
                b.add(vec_xyz(&c?)?);
            }
        } else if ent.hasattr("a")? && ent.hasattr("b")? {
            b.add(vec_xyz(&ent.getattr("a")?)?);
            b.add(vec_xyz(&ent.getattr("b")?)?);
        }
    }
    if !b.any {
        return none_pair(py);
    }
    PyTuple::new(py, [camera_point(py, b.lo)?, camera_point(py, b.hi)?])
}

/// Whether a placement `d` (column-major) of a mesh whose box is `local`
/// (centre, half size) stays strictly inside `lo`–`hi` whatever its points,
/// by more than the rounding of reading them. A NaN anywhere says no.
fn inside(local: &[f64; 6], d: &[f64], lo: P3, hi: P3) -> bool {
    let [cx, cy, cz, hx, hy, hz] = *local;
    for k in 0..3 {
        let c = d[k] * cx + d[k + 4] * cy + d[k + 8] * cz + d[k + 12];
        let r = d[k].abs() * hx + d[k + 4].abs() * hy + d[k + 8].abs() * hz;
        let slack = 1e-9 * (c.abs() + r);
        if !(c - r - slack > lo[k] && c + r + slack < hi[k]) {
            return false;
        }
    }
    true
}

/// `_inside(local, d, lo, hi)` for Python (the tests bound it).
#[pyfunction]
fn scene_bounds_inside(local: [f64; 6], d: Vec<f64>, lo: P3, hi: P3) -> bool {
    d.len() >= 16 && inside(&local, &d, lo, hi)
}

struct MeshPoints<'py> {
    arr: Bound<'py, PyAny>,
    lazy: bool,
    /// Its own box: centre and half size.
    local: Option<[f64; 6]>,
    len: usize,
}

/// `Scene.bounds()`: the box of all visible geometry, or `(None, None)`.
/// Kept per `version`, and across a view-only change by `_bounds_key()`.
#[pyfunction]
fn scene_bounds<'py>(py: Python<'py>, scene: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyTuple>> {
    let none = py.None().into_bound(py);
    let cached = scene.getattr_opt("_bounds_cache")?.unwrap_or_else(|| none.clone());
    let content = scene.call_method0("_bounds_key")?;
    let version = scene.getattr("version")?;
    if !cached.is_none() {
        let same_version = cached.get_item(0)?.eq(&version)?;
        if same_version || cached.get_item(2)?.eq(&content)? {
            if !same_version {
                scene.setattr("_bounds_cache", (&version, cached.get_item(1)?, &content))?;
            }
            let pair = cached.get_item(1)?;
            let lo = pair.get_item(0)?;
            if lo.is_none() {
                return none_pair(py);
            }
            return PyTuple::new(py, [camera_point(py, lo.extract()?)?, camera_point(py, pair.get_item(1)?.extract()?)?]);
        }
    }
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    let mut seen = false;
    let absorb = |p: P3, lo: &mut P3, hi: &mut P3| {
        for k in 0..3 {
            if p[k] < lo[k] {
                lo[k] = p[k];
            }
            if p[k] > hi[k] {
                hi[k] = p[k];
            }
        }
    };
    let loose = scene.getattr("loose_mesh")?;
    for edge in loose.getattr("edges")?.try_iter()? {
        let edge = edge?;
        if scene.call_method1("entity_visible", (&edge,))?.is_truthy()? {
            seen = true;
            absorb(edge.getattr("v0")?.call_method0("position_tuple")?.extract()?, &mut lo, &mut hi);
            absorb(edge.getattr("v1")?.call_method0("position_tuple")?.extract()?, &mut lo, &mut hi);
        }
    }
    for face in loose.getattr("faces")?.try_iter()? {
        let face = face?;
        if scene.call_method1("entity_visible", (&face,))?.is_truthy()? {
            for v in face.getattr("loop")?.try_iter()? {
                seen = true;
                absorb(v?.call_method0("position_tuple")?.extract()?, &mut lo, &mut hi);
            }
        }
    }
    // One vertex array per mesh, not per placement.
    let mut per_mesh: std::collections::HashMap<usize, MeshPoints<'py>> = std::collections::HashMap::new();
    let parts = match scene.getattr_opt("_bounds_parts")? {
        Some(p) if p.is_truthy()? => p,
        _ => PyDict::new(py).into_any(),
    };
    let kept = PyDict::new(py);
    let plan_of = py.import("core.lazy_mesh")?.getattr("plan_of")?;
    let local_points = py.import("core.group")?.getattr("_local_points")?;
    let builtins = py.import("builtins")?;
    let kw = PyDict::new(py);
    kw.set_item("precise", true)?;
    for item in scene.call_method("placements", (), Some(&kw))?.try_iter()? {
        let (g, m): (Bound<'py, PyAny>, Bound<'py, PyAny>) = item?.extract()?;
        let mesh = g.getattr("mesh")?;
        let key = mesh.as_ptr() as usize;
        if !per_mesh.contains_key(&key) {
            let plan = plan_of.call1((&mesh,))?; // a lazy mesh: no build
            let (arr, lazy) = if !plan.is_none() {
                (plan.getattr("pos")?, true)
            } else {
                let arr = local_points.call1((&mesh,))?;
                if arr.is_none() {
                    continue;
                }
                (arr, false)
            };
            let len = arr.len()?;
            let mut local = None;
            if len > 0 {
                // In the array's own precision, as NumPy works them out.
                let a = arr.call_method1("min", (0,))?;
                let b = arr.call_method1("max", (0,))?;
                let c: P3 = a.add(&b)?.div(2.0)?.call_method0("tolist")?.extract()?;
                let h: P3 = b.sub(&a)?.div(2.0)?.call_method0("tolist")?.extract()?;
                local = Some([c[0], c[1], c[2], h[0], h[1], h[2]]);
            }
            per_mesh.insert(key, MeshPoints { arr, lazy, local, len });
        }
        let mp = &per_mesh[&key];
        if mp.len == 0 {
            continue;
        }
        // Column-major.
        let d: Option<Vec<f64>> = if m.is_none() { None } else { Some(m.call_method0("ravel")?.extract()?) };
        let mut part: Option<Bound<'py, PyAny>> = None;
        let mut pkey = None;
        if mp.lazy {
            let dk = match &d {
                Some(d) => PyTuple::new(py, d)?.into_any(),
                None => none.clone(),
            };
            let k = PyTuple::new(py, [builtins.getattr("id")?.call1((&mp.arr,))?, dk])?;
            part = match kept.get_item(&k)? {
                Some(p) if p.is_truthy()? => Some(p),
                _ => parts.call_method1("get", (&k,))?.extract::<Option<Bound<'py, PyAny>>>()?,
            };
            pkey = Some(k);
        }
        let (glo, ghi): (P3, P3) = if let Some(p) = &part {
            (p.get_item(1)?.extract()?, p.get_item(2)?.extract()?)
        } else if seen && d.as_ref().is_some_and(|d| mp.local.as_ref().is_some_and(|l| inside(l, d, lo, hi))) {
            continue; // it cannot widen the box
        } else {
            // The prototype's points through the placement, and their box.
            let (a, b) = super::group_frame::group_placed_box(float64_rows(py, &mp.arr)?, d)?;
            let (glo, ghi) = ([a.0, a.1, a.2], [b.0, b.1, b.2]);
            if mp.lazy {
                part = Some(PyTuple::new(py, [mp.arr.clone(), glo.into_pyobject(py)?.into_any(),
                                              ghi.into_pyobject(py)?.into_any()])?.into_any());
            }
            (glo, ghi)
        };
        if let (Some(p), Some(k)) = (&part, &pkey) {
            kept.set_item(k, p)?;
        }
        seen = true;
        for k in 0..3 {
            if glo[k] < lo[k] {
                lo[k] = glo[k];
            }
            if ghi[k] > hi[k] {
                hi[k] = ghi[k];
            }
        }
    }
    scene.setattr("_bounds_parts", kept)?;
    if !seen {
        scene.setattr("_bounds_cache", (&version, (py.None(), py.None()), &content))?;
        return none_pair(py);
    }
    let lo_t = (lo[0], lo[1], lo[2]);
    let hi_t = (hi[0], hi[1], hi[2]);
    scene.setattr("_bounds_cache", (&version, (lo_t, hi_t), &content))?;
    PyTuple::new(py, [camera_point(py, lo)?, camera_point(py, hi)?])
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(scene_selection_bounds, m)?)?;
    m.add_function(wrap_pyfunction!(scene_bounds, m)?)?;
    m.add_function(wrap_pyfunction!(scene_bounds_inside, m)?)?;
    Ok(())
}
