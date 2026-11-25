// SPDX-License-Identifier: GPL-3.0-or-later
//! The broad phase of a dimension's vertex anchor (`core.dimension`):
//! whether a deferred mesh can hold a vertex near a point without building
//! it (`_may_hold`), and the placements worth asking (`_AnchorPlacements`),
//! with the prototypes placed 32 times or more tested by box first.
//!
//! NumPy's `(K, 3, 4) @ (4,)` is the pairwise two-lane sum
//! `(a0·p0 + a2·p2) + (a1·p1 + a3·p3)` on this Mac at every size measured
//! (`tests/test_anchor_broad_parity.py`); the port computes it that way, so
//! the candidates are the Python's exactly. The meshes, matrices and
//! placements stay Python objects: only the arithmetic and the walk moved.

use numpy::PyReadonlyArray2;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

/// `np.min` / `np.max` over axis 0: a NaN anywhere in a column wins.
fn column_box(a: &PyReadonlyArray2<'_, f64>) -> ([f64; 3], [f64; 3]) {
    let v = a.as_array();
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for row in v.rows() {
        for k in 0..3 {
            let x = row[k];
            if x.is_nan() || lo[k].is_nan() {
                lo[k] = f64::NAN;
                hi[k] = f64::NAN;
                continue;
            }
            if x < lo[k] {
                lo[k] = x;
            }
            if x > hi[k] {
                hi[k] = x;
            }
        }
    }
    (lo, hi)
}

fn float64_rows<'py>(py: Python<'py>, pos: &Bound<'py, PyAny>) -> PyResult<PyReadonlyArray2<'py, f64>> {
    let np = super::module(py, "numpy")?;
    let kw = PyDict::new(py);
    kw.set_item("dtype", np.getattr("float64")?)?;
    Ok(np.getattr("asarray")?.call((pos,), Some(&kw))?.extract::<PyReadonlyArray2<'py, f64>>()?)
}

fn is_deferred(py: Python<'_>, mesh: &Bound<'_, PyAny>) -> PyResult<bool> {
    Ok(mesh.get_type().is(&super::module(py, "built_core.mesh")?.getattr("DeferredMesh")?))
}

fn xyz(p: &Bound<'_, PyAny>) -> PyResult<[f64; 3]> {
    Ok([p.call_method0("x")?.extract()?, p.call_method0("y")?.extract()?, p.call_method0("z")?.extract()?])
}

/// ``_may_hold(mesh, local, tol)``: whether ``mesh`` can have a vertex
/// within ``tol`` of ``local``. A deferred mesh answers from its source
/// points (their box kept in ``_anchor_box``) without being built; any
/// other mesh may.
#[pyfunction]
fn dimension_may_hold(py: Python<'_>, mesh: &Bound<'_, PyAny>, local: &Bound<'_, PyAny>, tol: f64) -> PyResult<bool> {
    if !is_deferred(py, mesh)? {
        return Ok(true);
    }
    let d = mesh.getattr("__dict__")?;
    let src = d.call_method1("get", ("_deferred_source",))?;
    if !src.is_truthy()? || !src.contains("pos")? {
        return Ok(true);
    }
    let pos = src.get_item("pos")?;
    if pos.len()? == 0 {
        return Ok(false);
    }
    let cached = d.call_method1("get", ("_anchor_box",))?;
    let (lo, hi): ([f64; 3], [f64; 3]) = if cached.is_none() {
        let b = column_box(&float64_rows(py, &pos)?);
        d.set_item("_anchor_box", b)?;
        b
    } else {
        cached.extract()?
    };
    let pad = 4.0 * tol;
    let p = xyz(local)?;
    if (0..3).any(|i| p[i] < lo[i] - pad || p[i] > hi[i] + pad) {
        return Ok(false);
    }
    let a = float64_rows(py, &pos)?;
    Ok(a.as_array().rows().into_iter().any(|r| (0..3).all(|k| (r[k] - p[k]).abs() <= pad)))
}

struct Batch {
    mesh: Py<PyAny>,
    rows: Vec<usize>,
    /// The inverse placements' top three rows.
    affine: Vec<[[f64; 4]; 3]>,
}

/// ``_AnchorPlacements(groups)``: every placement under ``groups``
/// (billboards left out) depth first as ``(root, placed, inverse)`` in
/// ``entries``, singular placements dropped; ``candidates(point, tol)``
/// those whose prototype's box, taken through the inverse, can hold
/// ``point``.
#[pyclass(module = "built_core", name = "AnchorPlacements")]
pub struct AnchorPlacements {
    #[pyo3(get)]
    entries: Py<PyList>,
    batches: Vec<Batch>,
    /// mesh → ((revision, id(store), store revision), box or None).
    boxes: Py<PyDict>,
}

#[pymethods]
impl AnchorPlacements {
    #[new]
    fn new(py: Python<'_>, groups: &Bound<'_, PyAny>) -> PyResult<Self> {
        let iter_placements = super::module(py, "core.group")?.getattr("iter_placements")?;
        let entries = PyList::empty(py);
        let by_mesh = PyDict::new(py);
        for root in groups.try_iter()? {
            let root = root?;
            if root.getattr_opt("billboard")?.map(|b| b.is_truthy()).transpose()?.unwrap_or(false) {
                continue;
            }
            for item in iter_placements.call1((&root,))?.try_iter()? {
                let item = item?;
                let placed = item.get_item(0)?;
                let matrix = item.get_item(1)?;
                let mut inverse = py.None().into_bound(py);
                if !matrix.is_none() {
                    let inv = matrix.call_method0("inverted")?;
                    if !inv.get_item(1)?.is_truthy()? {
                        continue;
                    }
                    inverse = inv.get_item(0)?;
                }
                if !inverse.is_none() && inverse.call_method0("isAffine")?.is_truthy()? {
                    let mesh = placed.getattr("mesh")?;
                    let rows = match by_mesh.get_item(&mesh)? {
                        Some(r) => r,
                        None => {
                            let r = PyList::empty(py).into_any();
                            by_mesh.set_item(&mesh, &r)?;
                            r
                        }
                    };
                    rows.call_method1("append", (entries.len(),))?;
                }
                entries.append(PyTuple::new(py, [root.clone(), placed, inverse])?)?;
            }
        }
        let mut batches = Vec::new();
        for (mesh, rows) in by_mesh.iter() {
            let rows: Vec<usize> = rows.extract()?;
            if rows.len() < 32 {
                continue;
            }
            let mut affine = Vec::with_capacity(rows.len());
            for &i in &rows {
                let data: Vec<f64> = entries.get_item(i)?.get_item(2)?.call_method0("data")?.extract()?;
                let mut a = [[0.0; 4]; 3];
                for (r, row) in a.iter_mut().enumerate() {
                    for (c, v) in row.iter_mut().enumerate() {
                        *v = data[4 * c + r];
                    }
                }
                affine.push(a);
            }
            batches.push(Batch { mesh: mesh.unbind(), rows, affine });
        }
        Ok(AnchorPlacements { entries: entries.unbind(), batches, boxes: PyDict::new(py).unbind() })
    }

    /// The placements, meshes and matrices held are Python objects that can
    /// lead back to the scene keeping this: the collector must see them.
    fn __traverse__(&self, visit: pyo3::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        visit.call(&self.entries)?;
        visit.call(&self.boxes)?;
        for b in &self.batches {
            visit.call(&b.mesh)?;
        }
        Ok(())
    }

    fn __clear__(&mut self, py: Python<'_>) {
        self.batches.clear();
        self.entries = PyList::empty(py).unbind();
        self.boxes = PyDict::new(py).unbind();
    }

    fn candidates(&self, py: Python<'_>, point: &Bound<'_, PyAny>, tol: f64) -> PyResult<Py<PyAny>> {
        let entries = self.entries.bind(py);
        if self.batches.is_empty() {
            return Ok(entries.clone().into_any().unbind());
        }
        let mut keep = vec![true; entries.len()];
        let t: [f64; 3] = point.call_method0("toTuple")?.extract()?;
        let p = [t[0], t[1], t[2], 1.0];
        let boxes = self.boxes.bind(py);
        let group = super::module(py, "core.group")?;
        let builtins = super::module(py, "builtins")?;
        for batch in &self.batches {
            let mesh = batch.mesh.bind(py);
            let d = mesh.getattr("__dict__")?;
            let store = d.call_method1("get", ("_store",))?;
            let key = PyTuple::new(py, [
                mesh.getattr("revision")?,
                builtins.getattr("id")?.call1((&store,))?,
                store.getattr_opt("revision")?.unwrap_or_else(|| py.None().into_bound(py)),
            ])?;
            let cached = boxes.get_item(mesh)?;
            let fresh = match &cached {
                Some(c) => !c.get_item(0)?.eq(&key)?,
                None => true,
            };
            let bx: Option<([f64; 3], [f64; 3])> = if fresh {
                let source = d.call_method1("get", ("_deferred_source",))?;
                let has_source = source.is_truthy()?;
                if !has_source && is_deferred(py, mesh)? {
                    continue; // a custom deferred builder without source positions
                }
                let pos = if has_source {
                    source.get_item("pos")?
                } else if !store.is_none() {
                    store.call_method0("positions")?
                } else {
                    group.getattr("_local_points")?.call1((mesh,))?
                };
                let bx = if pos.is_none() || pos.len()? == 0 {
                    None
                } else {
                    Some(column_box(&float64_rows(py, &pos)?))
                };
                boxes.set_item(mesh, (key, bx))?;
                bx
            } else {
                cached.unwrap().get_item(1)?.extract()?
            };
            let Some((lo, hi)) = bx else {
                for &i in &batch.rows {
                    keep[i] = false;
                }
                continue;
            };
            let ap = [p[0].abs(), p[1].abs(), p[2].abs(), p[3].abs()];
            // 8 · float32 eps, as NumPy keeps it (a float32 scalar).
            let eps8 = 8.0 * f32::EPSILON as f64;
            for (&i, a) in batch.rows.iter().zip(&batch.affine) {
                let mut out = false;
                for k in 0..3 {
                    let r = &a[k];
                    let local = (r[0] * p[0] + r[2] * p[2]) + (r[1] * p[1] + r[3] * p[3]);
                    let mag = (r[0].abs() * ap[0] + r[2].abs() * ap[2]) + (r[1].abs() * ap[1] + r[3].abs() * ap[3]);
                    let slack = 4.0 * tol + eps8 * mag;
                    if local < lo[k] - slack || local > hi[k] + slack {
                        out = true;
                    }
                }
                keep[i] = !out;
            }
        }
        let kept: Vec<Bound<'_, PyAny>> = keep.iter().enumerate().filter(|(_, k)| **k)
            .map(|(i, _)| entries.get_item(i)).collect::<PyResult<_>>()?;
        Ok(PyList::new(py, kept)?.into_any().unbind())
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(dimension_may_hold, m)?)?;
    m.add_class::<AnchorPlacements>()?;
    Ok(())
}
