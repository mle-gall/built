// SPDX-License-Identifier: GPL-3.0-or-later
//! Immutable prototype acceleration plus replacement rows for incremental editing.
use super::{prepare::resident_prototype, resident_curve_features};
use numpy::{IntoPyArray, PyReadonlyArray1, PyReadonlyArray2};
use pyo3::{
    exceptions::{PyIndexError, PyValueError},
    prelude::*,
    types::{PyDict, PyList, PySet, PySlice, PyTuple},
};
type Obj<'p> = Bound<'p, PyAny>;
#[pyclass(module = "ingetrazo_core", sequence, skip_from_py_object)]
struct CurveFeatures {
    #[pyo3(get)]
    pending: Py<PyList>,
    #[pyo3(get)]
    fitted: Py<PyList>,
}
impl CurveFeatures {
    fn create(curves: &Obj<'_>) -> PyResult<Self> {
        let py = curves.py();
        let n = curves.len()?;
        Ok(Self {
            pending: curves.clone().cast_into::<PyList>()?.unbind(),
            fitted: PyList::new(py, (0..n).map(|_| py.None()))?.unbind(),
        })
    }
    fn at<'p>(&self, py: Python<'p>, i: usize) -> PyResult<Obj<'p>> {
        let fitted = self.fitted.bind(py);
        let hit = fitted.get_item(i)?;
        if !hit.is_none() {
            return Ok(hit);
        }
        let pending = self.pending.bind(py);
        let v = resident_curve_features(&pending.get_item(i)?)?.into_any();
        fitted.set_item(i, &v)?;
        pending.set_item(i, py.None())?;
        Ok(v)
    }
    fn replace(&self, py: Python<'_>, i: usize, curves: Obj<'_>) -> PyResult<()> {
        let p = self.pending.bind(py);
        let f = self.fitted.bind(py);
        while p.len() <= i {
            p.append(PyTuple::empty(py))?;
            f.append(py.None())?;
        }
        p.set_item(i, curves)?;
        f.set_item(i, py.None())
    }
}
#[pymethods]
impl CurveFeatures {
    fn __traverse__(&self, visit: pyo3::PyVisit) -> Result<(), pyo3::PyTraverseError> {
        visit.call(&self.pending)?;
        visit.call(&self.fitted)
    }
    fn __len__(&self, py: Python<'_>) -> usize {
        self.fitted.bind(py).len()
    }
    fn __getitem__<'p>(&self, py: Python<'p>, code: &Obj<'p>) -> PyResult<Obj<'p>> {
        let n = self.__len__(py) as isize;
        if let Ok(slice) = code.cast::<PySlice>() {
            let s = slice.indices(n)?;
            let result = PyList::empty(py);
            for k in 0..s.slicelength {
                result.append(self.at(py, (s.start + k as isize * s.step) as usize)?)?;
            }
            return Ok(result.into_any());
        }
        let mut i = code.extract::<isize>()?;
        if i < 0 {
            i += n;
        }
        if i < 0 {
            return Err(PyIndexError::new_err(i));
        }
        self.at(py, i as usize)
    }
}
#[pyclass(module = "ingetrazo_core", dict, weakref, skip_from_py_object)]
pub(super) struct ResidentInference {
    #[pyo3(get, set)]
    packed: Py<PyAny>,
    weak_scene: Py<PyAny>,
    #[pyo3(get)]
    features: Py<CurveFeatures>,
    #[pyo3(get, set)]
    screen_cache: Py<PyAny>,
    #[pyo3(get)]
    screen_dirty: Py<PySet>,
    #[pyo3(get)]
    epoch: u64,
    #[pyo3(get)]
    base: Py<PyAny>,
    #[pyo3(get)]
    protos: Py<PyList>,
    #[pyo3(get)]
    bounds: Py<PyList>,
    #[pyo3(get)]
    rows: Py<PyDict>,
    #[pyo3(get)]
    changed: Py<PySet>,
    #[pyo3(get)]
    local: Py<PyDict>,
    #[pyo3(get)]
    replaced: Py<PyAny>,
}
#[pymethods]
impl ResidentInference {
    fn __traverse__(&self, visit: pyo3::PyVisit) -> Result<(), pyo3::PyTraverseError> {
        visit.call(&self.packed)?;
        visit.call(&self.weak_scene)?;
        visit.call(&self.features)?;
        visit.call(&self.screen_cache)?;
        visit.call(&self.screen_dirty)?;
        visit.call(&self.base)?;
        visit.call(&self.protos)?;
        visit.call(&self.bounds)?;
        visit.call(&self.rows)?;
        visit.call(&self.changed)?;
        visit.call(&self.local)?;
        visit.call(&self.replaced)?;
        Ok(())
    }
    #[new]
    #[pyo3(signature=(base,protos,curves,packed,scene=None))]
    fn new<'p>(
        py: Python<'p>,
        base: Py<PyAny>,
        protos: Py<PyList>,
        curves: &Obj<'p>,
        packed: &Obj<'p>,
        scene: Option<&Obj<'p>>,
    ) -> PyResult<Self> {
        let weak = py.import("weakref")?.getattr("ref")?;
        let weak_scene = match scene {
            Some(s) if !s.is_instance(&weak)? => weak.call1((s,))?.unbind(),
            Some(s) => s.clone().unbind(),
            None => py.None(),
        };
        let bounds = PyList::empty(py);
        for b in packed.getattr("bounds")?.try_iter()? {
            bounds.append(b?.call_method0("copy")?)?;
        }
        Ok(Self {
            packed: packed.clone().unbind(),
            weak_scene,
            features: Py::new(py, CurveFeatures::create(curves)?)?,
            screen_cache: py.None(),
            screen_dirty: PySet::empty(py)?.unbind(),
            epoch: 0,
            base,
            protos,
            bounds: bounds.unbind(),
            rows: PyDict::new(py).unbind(),
            changed: PySet::empty(py)?.unbind(),
            local: PyDict::new(py).unbind(),
            replaced: py
                .import("numpy")?
                .call_method1("zeros", (packed.getattr("instances")?.len()?, "bool"))?
                .unbind(),
        })
    }
    #[getter]
    fn scene<'p>(&self, py: Python<'p>) -> PyResult<Obj<'p>> {
        let weak = self.weak_scene.bind(py);
        if weak.is_none() {
            Ok(weak.clone())
        } else {
            weak.call0()
        }
    }
    #[pyo3(signature=(ids,packed,changed_codes=None,*,code_rows=None))]
    fn update<'p>(
        &mut self,
        py: Python<'p>,
        ids: &Obj<'p>,
        packed: &Obj<'p>,
        changed_codes: Option<&Obj<'p>>,
        code_rows: Option<&Obj<'p>>,
    ) -> PyResult<()> {
        let np = py.import("numpy")?;
        let instances = packed.getattr("instances")?;
        self.epoch += 1;
        self.packed = packed.clone().unbind();
        let n = instances.len()?;
        let old = self.replaced.bind(py).len()?;
        if n > old {
            self.replaced = np
                .call_method1("pad", (self.replaced.bind(py), (0, n - old)))?
                .unbind();
        }
        let codes = match (code_rows, changed_codes) {
            (Some(map), _) => Some(map.clone()),
            (_, Some(seq)) => Some(py.get_type::<PyDict>().call_method1("fromkeys", (seq,))?),
            _ => None,
        };
        if let Some(codes) = codes {
            for c in codes.try_iter()? {
                let c = c?;
                let k = c.extract::<usize>()?;
                let protos = self.protos.bind(py);
                let bounds = self.bounds.bind(py);
                while protos.len() <= k {
                    protos.append(py.None())?;
                    bounds.append(py.None())?;
                }
                let affected = match code_rows {
                    Some(map) => map.get_item(&c)?,
                    None => {
                        let info = instances
                            .get_item("info")?
                            .get_item((PySlice::full(py), 0))?;
                        np.call_method1("flatnonzero", (np.call_method1("equal", (info, k))?,))?
                    }
                };
                let first = if affected.len()? > 0 {
                    affected.get_item(0)?.extract()?
                } else {
                    -1
                };
                let pair = resident_prototype(packed, k, &self.scene(py)?, first)?;
                protos.set_item(k, pair.get_item(0)?)?;
                bounds.set_item(
                    k,
                    packed
                        .getattr("bounds")?
                        .get_item(k)?
                        .call_method0("copy")?,
                )?;
                self.features
                    .bind(py)
                    .borrow()
                    .replace(py, k, pair.get_item(1)?)?;
                self.changed.bind(py).add(k)?;
                self.replaced.bind(py).set_item(&affected, true)?;
                self.screen_dirty
                    .bind(py)
                    .call_method1("update", (affected.call_method0("tolist")?,))?;
                let local = self.local.bind(py);
                for row in local.keys().iter() {
                    if local.get_item(&row)?.unwrap().get_item(0)?.eq(k)? {
                        local.del_item(row)?;
                    }
                }
            }
        }
        for r in ids.try_iter()? {
            let r = r?.extract::<i64>()?;
            self.rows
                .bind(py)
                .set_item(r, instances.get_item(r)?.call_method0("copy")?)?;
            self.local.bind(py).call_method1("pop", (r, py.None()))?;
            self.replaced.bind(py).set_item(r, true)?;
            self.screen_dirty.bind(py).add(r)?;
        }
        Ok(())
    }
    fn committed<'p>(&self, py: Python<'p>, row: i64) -> PyResult<(usize, Obj<'p>)> {
        if let Some(v) = self.rows.bind(py).get_item(row)? {
            return Ok((
                v.get_item("info")?.get_item(0)?.extract()?,
                v.get_item("world")?.call_method1("astype", ("float64",))?,
            ));
        }
        let pair = self.base.bind(py).call_method1("placement", (row,))?;
        let flat = pair.get_item(1)?;
        Ok((
            pair.get_item(0)?.extract()?,
            py.import("numpy")?
                .call_method1("asarray", (flat, "float64"))?
                .call_method1("reshape", (4, 4))?,
        ))
    }
    #[pyo3(signature=(rows,*args))]
    fn ray_cands<'p>(
        &self,
        py: Python<'p>,
        rows: &Obj<'p>,
        args: &Bound<'p, PyTuple>,
    ) -> PyResult<Obj<'p>> {
        self.query(py, "ray_cands", rows, args)
    }
    #[pyo3(signature=(rows,*args))]
    fn edges_near<'p>(
        &self,
        py: Python<'p>,
        rows: &Obj<'p>,
        args: &Bound<'p, PyTuple>,
    ) -> PyResult<Obj<'p>> {
        self.query(py, "edges_near", rows, args)
    }
}

impl ResidentInference {
    fn local_index<'p>(
        &self,
        py: Python<'p>,
        row: i64,
        code: usize,
        world: &Obj<'p>,
    ) -> PyResult<Obj<'p>> {
        let local = self.local.bind(py);
        if let Some(pair) = local.get_item(row)? {
            return pair.get_item(1);
        }
        let bounds = self
            .bounds
            .bind(py)
            .get_item(code)?
            .call_method1("astype", ("float64",))?;
        let bounds = bounds.extract::<PyReadonlyArray2<f64>>()?;
        let bb = bounds.as_array();
        let m = world.extract::<PyReadonlyArray2<f64>>()?;
        let m = m.as_array();
        if m.dim() != (4, 4) || bb.nrows() != 2 || bb.ncols() < 3 {
            return Err(PyValueError::new_err(
                "invalid placement transform or bounds",
            ));
        }
        let mut lo = [0.; 3];
        let mut hi = [0.; 3];
        for j in 0..3 {
            let mut c = 0.;
            let mut e = 0.;
            for k in 0..3 {
                c += m[[k, j]] * ((bb[[0, k]] + bb[[1, k]]) * 0.5);
                e += m[[k, j]].abs() * ((bb[[1, k]] - bb[[0, k]]) * 0.5);
            }
            c += m[[3, j]];
            lo[j] = c - e;
            hi[j] = c + e;
        }
        let array = |v: [f64; 3]| v.to_vec().into_pyarray(py).call_method1("reshape", (1, 3));
        let index = py.get_type::<crate::InstanceSet>().call1((
            PyList::new(py, [self.protos.bind(py).get_item(code)?])?,
            vec![0i64].into_pyarray(py),
            world.call_method1("reshape", (1, 16))?,
            array(lo)?,
            array(hi)?,
        ))?;
        local.set_item(row, (code, &index))?;
        Ok(index)
    }
    fn query<'p>(
        &self,
        py: Python<'p>,
        name: &str,
        rows: &Obj<'p>,
        args: &Bound<'p, PyTuple>,
    ) -> PyResult<Obj<'p>> {
        let rows = if rows.extract::<PyReadonlyArray1<i64>>().is_ok() {
            rows.clone()
        } else {
            py.import("numpy")?
                .call_method1("asarray", (rows, "int64"))?
        };
        let call = |obj: &Obj<'p>, r: &Obj<'p>| -> PyResult<Obj<'p>> {
            let mut params = Vec::with_capacity(args.len() + 1);
            params.push(r.clone());
            params.extend(args.iter());
            obj.call_method1(name, PyTuple::new(py, params)?)
        };
        let base = self.base.bind(py);
        if self.rows.bind(py).is_empty() && self.changed.bind(py).is_empty() {
            return call(base, &rows);
        }
        let replaced = self.replaced.bind(py).extract::<PyReadonlyArray1<bool>>()?;
        let flags = replaced.as_array();
        let query_rows = rows.extract::<PyReadonlyArray1<i64>>()?;
        let mut untouched = Vec::new();
        let mut edited = Vec::new();
        for &r in query_rows.as_array() {
            let i = if r < 0 { flags.len() as i64 + r } else { r };
            if *flags
                .get(i as usize)
                .ok_or_else(|| PyIndexError::new_err(r))?
            {
                edited.push(r)
            } else {
                untouched.push(r)
            }
        }
        if edited.is_empty() {
            let result = call(base, &rows)?;
            // A multi-row incremental edge query orders distance ties by row
            // key. The base BVH uses input order, which can differ here.
            if name == "edges_near" && untouched.len() > 1 {
                let cap = args.get_item(args.len() - 1)?.extract::<isize>()?;
                if cap == 0 || !untouched.windows(2).all(|pair| pair[0] < pair[1]) {
                    let columns = result.extract::<[Obj<'p>; 5]>()?;
                    return sorted_edges(py, columns.into(), cap);
                }
            }
            return Ok(result);
        }
        let mut parts = Vec::new();
        if rows.len()? != 1 {
            parts.push(call(base, untouched.into_pyarray(py).as_any())?);
        }
        let zero = vec![0i64].into_pyarray(py);
        for row in edited {
            let local = self.local.bind(py).get_item(row)?;
            let ix = match local {
                Some(pair) => pair.get_item(1)?,
                None => {
                    let (code, world) = self.committed(py, row)?;
                    self.local_index(py, row, code, &world)?
                }
            };
            let res = call(&ix, zero.as_any())?;
            let mut cols = vec![vec![row; res.get_item(0)?.len()?]
                .into_pyarray(py)
                .into_any()];
            for i in 1..5 {
                cols.push(res.get_item(i)?);
            }
            parts.push(PyTuple::new(py, cols)?.into_any());
        }
        if parts.len() == 1 {
            return Ok(parts.pop().unwrap());
        }
        let np = py.import("numpy")?;
        let mut merged = Vec::new();
        for i in 0..5 {
            let seq = parts
                .iter()
                .map(|p| p.get_item(i))
                .collect::<PyResult<Vec<_>>>()?;
            merged.push(np.call_method1("concatenate", (seq,))?);
        }
        if name == "edges_near" {
            let cap = args.get_item(args.len() - 1)?.extract()?;
            return sorted_edges(py, merged, cap);
        }
        Ok(PyTuple::new(py, merged)?.into_any())
    }
}
fn sorted_edges<'p>(py: Python<'p>, mut merged: Vec<Obj<'p>>, cap: isize) -> PyResult<Obj<'p>> {
    let np = py.import("numpy")?;
    let order = np
        .call_method1("lexsort", ((&merged[1], &merged[0], &merged[4]),))?
        .get_item(PySlice::new(py, 0, cap, 1))?;
    for a in &mut merged {
        *a = a.get_item(&order)?;
    }
    Ok(PyTuple::new(py, merged)?.into_any())
}
#[pyfunction]
fn tangent_direction<'p>(
    index: &ResidentInference,
    packed: &Obj<'p>,
    row: Option<i64>,
    point: &Obj<'p>,
) -> PyResult<Option<crate::math::Vec3>> {
    use glam::DVec3;
    let Some(row) = row else { return Ok(None) };
    let py = packed.py();
    let instances = packed.getattr("instances")?;
    if row < 0 || row as usize >= instances.len()? {
        return Ok(None);
    }
    let instance = instances.get_item(row)?;
    if instance.get_item("info")?.get_item(2)?.extract::<u32>()? & 129 != 0 {
        return Ok(None);
    }
    let v = index.rows.bind(py).get_item(row)?.unwrap_or(instance);
    let code = v.get_item("info")?.get_item(0)?.extract()?;
    let fits = index.features.bind(py).borrow().at(py, code)?;
    let tips = fits.get_item(1)?;
    if tips.is_empty()? {
        return Ok(None);
    }
    let world = v.get_item("world")?.call_method1("astype", ("float64",))?;
    let m = world.extract::<PyReadonlyArray2<f64>>()?;
    let m = m.as_array();
    if m.dim() != (4, 4) {
        return Err(PyValueError::new_err("expected 4 by 4 placement matrix"));
    }
    let rotate = |v: [f64; 3]| {
        DVec3::from_array(std::array::from_fn(|j| {
            m[[0, j]] * v[0] + m[[1, j]] * v[1] + m[[2, j]] * v[2]
        }))
    };
    let offset = DVec3::new(m[[3, 0]], m[[3, 1]], m[[3, 2]]);
    let p = crate::math::xyz(point)?;
    for tip in tips.try_iter()? {
        let tip = tip?;
        if (p - (rotate(tip.get_item(0)?.extract()?) + offset)).length() <= 1e-6 {
            let d = rotate(tip.get_item(1)?.extract()?);
            if d.length() > 1e-9 {
                return Ok(Some(crate::math::Vec3::from(d.normalize())));
            }
        }
    }
    Ok(None)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<ResidentInference>()?;
    m.add_function(wrap_pyfunction!(tangent_direction, m)?)
}
