// SPDX-License-Identifier: GPL-3.0-or-later
//! Placement geometry is eager for picking. Material and silhouette streams
//! retain a shared transform and are evaluated when the renderer requests them.
use super::*;
use glam::DMat3;
use numpy::{IntoPyArray, PyArrayMethods, PyReadonlyArray2};
use pyo3::types::PyString;
use std::collections::BTreeMap;

fn f32s(raw: &Obj, stride: usize) -> PyResult<Vec<f32>> {
    let b = raw.cast::<PyBytes>()?.as_bytes();
    if b.len() % (stride * 4) != 0 {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "invalid packed vertex stride",
        ));
    }
    Ok(b.chunks_exact(4)
        .map(|c| f32::from_ne_bytes(c.try_into().unwrap()))
        .collect())
}
fn bytes<'py>(py: Python<'py>, rows: &[f32]) -> Obj<'py> {
    PyBytes::new(py, bytemuck::cast_slice(rows)).into_any()
}
fn pt(row: &[f32]) -> DVec3 {
    DVec3::new(row[0] as f64, row[1] as f64, row[2] as f64)
}
fn store(row: &mut [f32], p: DVec3) {
    for (i, n) in p.to_array().into_iter().enumerate() {
        row[i] = n as f32
    }
}
fn shifted<'py>(raw: &Obj<'py>, stride: usize, d: DVec3) -> PyResult<Obj<'py>> {
    let mut floats = f32s(raw, stride)?;
    for row in floats.chunks_exact_mut(stride) {
        store(row, pt(row) + d)
    }
    Ok(bytes(raw.py(), &floats))
}
#[pyfunction]
fn shift_obb(entry: &Obj, d: &Obj) -> PyResult<()> {
    let b = get(entry, "obb")?;
    if b.is_none() {
        return Ok(());
    }
    let frame = b.get_item(0)?;
    let lo = DVec3::from_array(b.get_item(1)?.extract()?);
    let hi = DVec3::from_array(b.get_item(2)?.extract()?);
    let delta = xyz(d)?;
    let off = DVec3::new(
        xyz(&frame.get_item(0)?)?.dot(delta),
        xyz(&frame.get_item(1)?)?.dot(delta),
        xyz(&frame.get_item(2)?)?.dot(delta),
    );
    entry.set_item("obb", (frame, coords(lo + off), coords(hi + off)))
}
fn shift_streams(view: &Obj, entry: &Obj, d: &Obj) -> PyResult<()> {
    let py = view.py();
    view.call_method1("_shift_obb", (entry, d))?;
    let off = xyz(d)?;
    let bb = get(entry, "bbox")?;
    if !bb.is_none() {
        let (a, b): ([f64; 3], [f64; 3]) = bb.extract()?;
        entry.set_item(
            "bbox",
            (
                coords(DVec3::from_array(a) + off),
                coords(DVec3::from_array(b) + off),
            ),
        )?
    }
    for (key, stride, required) in [
        ("edges", 3, true),
        ("dback", 3, false),
        ("vcol", 6, true),
        ("back_vcol", 6, false),
        ("fvcol", 6, false),
    ] {
        let src = if required {
            entry.get_item(key)?
        } else {
            get(entry, key)?
        };
        if required || src.is_truthy()? {
            entry.set_item(key, shifted(&src, stride, off)?)?
        }
    }
    for (key, stride) in [
        ("by_texture", 5),
        ("tcol", 6),
        ("ttex", 5),
        ("back_tex", 5),
        ("back_tcol", 6),
        ("back_ttex", 5),
    ] {
        let out = PyDict::new(py);
        let runs = get(entry, key)?;
        if !runs.is_none() {
            for (k, r) in runs.cast::<PyDict>()?.iter() {
                out.set_item(k, shifted(&r, stride, off)?)?
            }
        }
        entry.set_item(key, out)?
    }
    let delta = off.to_array();
    for key in ["v0", "soft_c0", "soft_c1"] {
        let a = entry.get_item(key)?;
        if !a.is_none() && a.len()? > 0 {
            entry.set_item(key, a.add(delta)?)?
        }
    }
    let soft = entry.get_item("soft_pts")?;
    if !soft.is_none() {
        entry.set_item(
            "soft_pts",
            soft.call_method1("reshape", (-1, 3))?
                .add(delta)?
                .call_method1("astype", ("float32",))?
                .call_method1("reshape", (-1, 6))?,
        )?
    }
    Ok(())
}
#[pyfunction]
fn shift_instance_entry(view: &Obj, entry: &Obj, d: &Obj) -> PyResult<()> {
    shift_streams(view, entry, d)
}
#[pyfunction]
fn shift_chunk(view: &Obj, entry: &Obj, d: &Obj, mesh: &Obj) -> PyResult<()> {
    shift_streams(view, entry, d)?;
    let py = view.py();
    let verts = mesh.getattr("vertices")?;
    let samples = PyList::empty(py);
    for old in entry.get_item("samples")?.try_iter()? {
        let i = old?.get_item(0)?;
        let p = xyz(&verts.get_item(&i)?.getattr("position")?)?;
        samples.append((i, coords(p)))?
    }
    entry.set_item("samples", samples)?;
    let sum: f64 = entry.get_item("coordsum")?.extract()?;
    let sum = sum + entry.get_item("nv")?.extract::<f64>()? * super::fingerprint::weight(xyz(d)?);
    entry.set_item("coordsum", sum)?;
    let fp = entry.get_item("fp")?;
    let mut fields = fp.try_iter()?.collect::<PyResult<Vec<_>>>()?;
    fields[3] = rounded(py, sum, 4)?;
    entry.set_item("fp", PyTuple::new(py, fields)?)?;
    entry.set_item("fp_approx", true)
}
struct Transform {
    m: [f64; 16],
    normal: DMat3,
    det: f64,
    turned: bool,
    light: DVec3,
}
impl Transform {
    fn new(m: [f64; 16], light: DVec3) -> Self {
        let linear =
            DMat3::from_cols_array(&[m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10]]);
        let det = linear.determinant();
        let normal = if det == 0. {
            DMat3::IDENTITY
        } else {
            linear.inverse().transpose()
        };
        let turned = !(m[0] > 0.
            && [m[1], m[2], m[4], m[6], m[8], m[9]]
                .iter()
                .all(|x| x.abs() <= 1e-9)
            && [m[5], m[10]]
                .iter()
                .all(|x| (x - m[0]).abs() <= 1e-8 + 1e-5 * m[0].abs()));
        Self {
            m,
            normal,
            det,
            turned,
            light,
        }
    }
    fn map(&self, p: DVec3, translate: bool) -> DVec3 {
        let m = &self.m;
        // Match the order used by the native unbaked picker.
        let mut q = DVec3::new(
            p.x * m[0] + p.y * m[4] + p.z * m[8],
            p.x * m[1] + p.y * m[5] + p.z * m[9],
            p.x * m[2] + p.y * m[6] + p.z * m[10],
        );
        if translate {
            q += DVec3::new(m[12], m[13], m[14])
        }
        q
    }
    fn normal(&self, n: DVec3) -> DVec3 {
        let n = self.normal * n;
        let len = n.length();
        if len > 1e-12 {
            n / len
        } else {
            n
        }
    }
    fn matte(&self, n: DVec3) -> f64 {
        0.62 + 0.38 * n.dot(self.light).abs()
    }
    fn flat(&self, tri: &[f32], stride: usize) -> f64 {
        let a = pt(tri);
        let n = (pt(&tri[stride..]) - a).cross(pt(&tri[2 * stride..]) - a);
        if n.length() > 1e-9 {
            (self.matte(n / n.length()) * 32.).round_ties_even() / 32.
        } else {
            0.84375
        }
    }
    fn array<'py>(&self, a: &Obj<'py>, mode: &str) -> PyResult<Obj<'py>> {
        if a.is_none() || a.len()? == 0 {
            return Ok(a.clone());
        }
        let ar = a.extract::<PyReadonlyArray2<f64>>()?;
        let rows = ar.as_array();
        if rows.ncols() != 3 {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "expected XYZ columns",
            ));
        }
        let mut out = Vec::with_capacity(rows.len());
        for r in rows.rows() {
            let v = DVec3::new(r[0], r[1], r[2]);
            let v = if mode == "normal" {
                self.normal(v)
            } else {
                self.map(v, mode == "point")
            };
            out.extend(v.to_array());
        }
        Ok(out
            .into_pyarray(a.py())
            .reshape([rows.nrows(), 3])?
            .into_any())
    }
    fn packed(
        &self,
        raw: &Obj,
        stride: usize,
        tris: bool,
        normals: Option<&Obj>,
    ) -> PyResult<Vec<f32>> {
        let src = f32s(raw, stride)?;
        let mut out = src.clone();
        for row in out.chunks_exact_mut(stride) {
            store(row, self.map(pt(row), true))
        }
        if !tris || out.len() % (3 * stride) != 0 {
            return Ok(out);
        }
        let smooth = normals
            .filter(|n| n.is_truthy().unwrap_or(false))
            .map(|n| f32s(n, 3))
            .transpose()?;
        if smooth
            .as_ref()
            .is_some_and(|n| n.len() / 3 != out.len() / stride)
        {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "smooth normals must match vertices",
            ));
        }
        for (i, t) in out.chunks_exact_mut(stride * 3).enumerate() {
            if self.det < 0. {
                for j in 0..stride {
                    t.swap(stride + j, 2 * stride + j)
                }
            }
            if self.turned && stride == 6 {
                let old = &src[i * stride * 3..];
                let gain = self.flat(t, stride) / self.flat(old, stride);
                for (vertex, r) in t.chunks_exact_mut(stride).enumerate() {
                    if let Some(n) = &smooth {
                        let k = if self.det < 0. && vertex != 0 {
                            3 - vertex
                        } else {
                            vertex
                        };
                        let local = pt(&n[(i * 3 + k) * 3..]);
                        let gain = self.matte(self.normal(local)) / self.matte(local);
                        for c in &mut r[3..] {
                            *c = ((*c as f64 * gain).min(1.)) as f32
                        }
                    } else {
                        for c in &mut r[3..] {
                            *c = (((*c as f64 * gain).min(1.) * 64.).round_ties_even() / 64.) as f32
                        }
                    }
                }
            }
        }
        Ok(out)
    }
}
fn shade_key<'py>(key: &Obj<'py>, shade: f64) -> PyResult<Obj<'py>> {
    let mut parts = key.try_iter()?.collect::<PyResult<Vec<_>>>()?;
    if parts.len() < 2 {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "invalid texture run key",
        ));
    }
    if parts[0].is_instance_of::<PyString>() {
        parts[1] = shade.into_pyobject(key.py())?.into_any()
    } else {
        parts[0] = (parts[0].get_item(0)?, shade)
            .into_pyobject(key.py())?
            .into_any()
    }
    Ok(PyTuple::new(key.py(), parts)?.into_any())
}
#[pyclass]
struct PlacedStreams {
    base: Py<PyAny>,
    xf: Transform,
}
#[pymethods]
impl PlacedStreams {
    fn __call__<'py>(&self, py: Python<'py>, field: &str) -> PyResult<Obj<'py>> {
        let b = self.base.bind(py);
        let a = get(b, field)?;
        let xf = &self.xf;
        match field {
            "soft_n0" | "soft_n1" => xf.array(&a, "normal"),
            "soft_c0" | "soft_c1" => xf.array(&a, "point"),
            "soft_pts" => {
                if a.is_none() {
                    return Ok(a);
                }
                let raw = a.call_method0("tobytes")?;
                let flat = xf.packed(&raw, 3, false, None)?;
                Ok(flat.into_pyarray(py).reshape([a.len()?, 6])?.into_any())
            }
            "vcol" | "fvcol" | "back_vcol" | "dback" => {
                if a.is_none() {
                    return Ok(bytes(py, &[]));
                }
                let n = if field == "vcol" {
                    Some(get(b, "smooth_vcol")?)
                } else {
                    None
                };
                Ok(bytes(
                    py,
                    &xf.packed(&a, if field == "dback" { 3 } else { 6 }, true, n.as_ref())?,
                ))
            }
            _ => {
                let out = PyDict::new(py);
                if !a.is_none() {
                    let stride = if matches!(field, "tcol" | "back_tcol") {
                        6
                    } else {
                        5
                    };
                    for (key, raw) in a.cast::<PyDict>()?.iter() {
                        let stream = xf.packed(&raw, stride, true, None)?;
                        if stride == 5 && xf.turned && !stream.is_empty() && stream.len() % 15 == 0
                        {
                            let mut runs = BTreeMap::<i32, Vec<f32>>::new();
                            for t in stream.chunks_exact(15) {
                                let q = (xf.flat(t, 5) * 32.) as i32;
                                runs.entry(q).or_default().extend(t)
                            }
                            for (q, run) in runs {
                                append(&out, shade_key(&key, q as f64 / 32.)?, bytes(py, &run))?
                            }
                        } else {
                            append(&out, key, bytes(py, &stream))?
                        }
                    }
                }
                Ok(out.into_any())
            }
        }
    }
}
fn append<'py>(d: &Bound<'py, PyDict>, k: Obj<'py>, v: Obj<'py>) -> PyResult<()> {
    if let Some(old) = d.get_item(&k)? {
        d.set_item(k, old.add(v)?)
    } else {
        d.set_item(k, v)
    }
}

#[pyfunction]
fn instance_chunk<'py>(v: &Obj<'py>, group: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let h = host(py)?;
    let mesh = group.getattr("mesh")?;
    let paint = h.call_method1("_paint_of", (v, group))?;
    let base = v.call_method1(
        "_group_chunk",
        (h.call_method1("_proto_wrapper", (v, mesh, paint))?,),
    )?;
    let matrix = group.getattr("xform")?.call_method0("data")?;
    let m: [f64; 16] = matrix.extract()?;
    let b_uid = base.get_item("uid")?;
    let b_rev = get(&base, "rev")?;
    let key = (&b_uid, tuple(&matrix)?, &b_rev).into_pyobject(py)?;
    let cache = memo(v, "_inst_chunks")?;
    let cur = cache.call_method1("get", (id(group),))?;
    let mut rev = 0;
    if !cur.is_none() {
        let old = cur.get_item("ikey")?;
        if old.eq(&key)? {
            return Ok(cur);
        }
        rev = cur.get_item("rev")?.extract::<u64>()? + 1;
        if old.get_item(0)?.eq(&b_uid)? && old.get_item(2)?.eq(&b_rev)? {
            let prev: [f64; 16] = old.get_item(1)?.extract()?;
            if (0..12).chain([15]).all(|i| (m[i] - prev[i]).abs() < 1e-12) {
                let delta = DVec3::from_slice(&m[12..]) - DVec3::from_slice(&prev[12..]);
                v.call_method1("_shift_instance_entry", (&cur, vector(py, delta)?))?;
                let xf = Transform::new(m, DVec3::ZERO);
                cur.set_item(
                    "edges",
                    bytes(py, &xf.packed(&base.get_item("edges")?, 3, false, None)?),
                )?;
                if !base.get_item("v0")?.is_none() {
                    for k in ["v0", "e1", "e2"] {
                        cur.set_item(
                            k,
                            xf.array(
                                &base.get_item(k)?,
                                if k == "v0" { "point" } else { "direction" },
                            )?,
                        )?
                    }
                }
                cur.set_item("ikey", key)?;
                cur.set_item("rev", rev)?;
                return Ok(cur);
            }
        }
    }
    let xf = Transform::new(m, xyz(&v.getattr("_LIGHT")?)?);
    let d = PyDict::new(py);
    d.set_item("ikey", key)?;
    d.set_item("uid", uid(py)?)?;
    d.set_item("rev", rev)?;
    for k in ["nv", "ne", "nf", "faces", "tri_ent", "soft_single"] {
        d.set_item(k, base.get_item(k)?)?
    }
    d.set_item(
        "edges",
        bytes(py, &xf.packed(&base.get_item("edges")?, 3, false, None)?),
    )?;
    d.set_item(
        "areas",
        base.get_item("areas")?.mul(xf.det.abs().powf(2. / 3.))?,
    )?;
    for k in ["v0", "e1", "e2"] {
        d.set_item(
            k,
            xf.array(
                &base.get_item(k)?,
                if k == "v0" { "point" } else { "direction" },
            )?,
        )?
    }
    let bb = get(&base, "bbox")?;
    if bb.is_none() {
        d.set_item("bbox", bb)?
    } else {
        let (a, b): ([f64; 3], [f64; 3]) = bb.extract()?;
        let mut lo = DVec3::splat(f64::INFINITY);
        let mut hi = -lo;
        for x in [a[0], b[0]] {
            for y in [a[1], b[1]] {
                for z in [a[2], b[2]] {
                    let p = xf.map(DVec3::new(x, y, z), true);
                    lo = lo.min(p);
                    hi = hi.max(p)
                }
            }
        }
        d.set_item("bbox", (coords(lo), coords(hi)))?
    }
    let later = PyDict::new(py);
    let make = Bound::new(
        py,
        PlacedStreams {
            base: base.unbind(),
            xf,
        },
    )?;
    let partial = py.import("functools")?.getattr("partial")?;
    for field in [
        "dback",
        "vcol",
        "by_texture",
        "tcol",
        "ttex",
        "back_vcol",
        "back_tex",
        "back_tcol",
        "back_ttex",
        "fvcol",
        "soft_pts",
        "soft_n0",
        "soft_c0",
        "soft_n1",
        "soft_c1",
    ] {
        later.set_item(field, partial.call1((&make, field))?)?
    }
    let entry = h.call_method1("_LazyChunk", (d, later))?;
    cache.set_item(id(group), &entry)?;
    Ok(entry)
}

#[pyclass(frozen)]
struct PlacementMap {
    xf: Transform,
    points: bool,
}
#[pymethods]
impl PlacementMap {
    fn __call__<'py>(&self, rows: &Obj<'py>) -> PyResult<Obj<'py>> {
        let a = rows
            .py()
            .import("numpy")?
            .call_method1("asarray", (rows, "float64"))?;
        self.xf
            .array(&a, if self.points { "point" } else { "vector" })
    }
}
#[pyfunction]
fn placed_maps<'py>(xf: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = xf.py();
    let m = xf.call_method0("data")?.extract::<[f64; 16]>()?;
    let linear = [m[0], m[4], m[8], m[1], m[5], m[9], m[2], m[6], m[10]]
        .to_vec()
        .into_pyarray(py)
        .reshape([3, 3])?;
    let mapper = |points| {
        Py::new(
            py,
            PlacementMap {
                xf: Transform::new(m, DVec3::ZERO),
                points,
            },
        )
    };
    Ok((linear, mapper(false)?, mapper(true)?)
        .into_pyobject(py)?
        .into_any())
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(placed_maps, m)?)?;

    m.add_function(wrap_pyfunction!(shift_obb, m)?)?;
    m.add_function(wrap_pyfunction!(shift_chunk, m)?)?;
    m.add_function(wrap_pyfunction!(shift_instance_entry, m)?)?;
    m.add_function(wrap_pyfunction!(instance_chunk, m)?)?;
    Ok(())
}
