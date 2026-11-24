// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared double-precision survey bounds for model importers.
use numpy::PyReadonlyArray2;
use pyo3::{
    prelude::*,
    types::{PyList, PyTuple},
};
pub const BEYOND: f64 = 100_000.;
#[pyfunction]
fn survey_offset(
    py: Python<'_>,
    lo: [f64; 3],
    hi: [f64; 3],
) -> PyResult<Option<Bound<'_, PyTuple>>> {
    if lo[0] > hi[0]
        || !lo.iter().chain(&hi).all(|n| n.is_finite())
        || !lo.iter().chain(&hi).any(|n| n.abs() > BEYOND)
    {
        return Ok(None);
    }
    let offset = lo.map(|n| if n.abs() < 1. { 0. } else { n.trunc() });
    Ok(Some(PyTuple::new(py, offset)?))
}
#[pyclass(name = "SurveyBounds", module = "ingetrazo_core")]
struct Bounds {
    #[pyo3(get, set)]
    lo: Py<PyList>,
    #[pyo3(get, set)]
    hi: Py<PyList>,
}
impl Bounds {
    fn absorb(&self, py: Python<'_>, lo: [f64; 3], hi: [f64; 3]) -> PyResult<()> {
        let (l, h) = (self.lo.bind(py), self.hi.bind(py));
        for k in 0..3 {
            if lo[k] < l.get_item(k)?.extract::<f64>()? {
                l.set_item(k, lo[k])?;
            }
            if hi[k] > h.get_item(k)?.extract::<f64>()? {
                h.set_item(k, hi[k])?;
            }
        }
        Ok(())
    }
}
#[pymethods]
impl Bounds {
    #[new]
    fn new(py: Python<'_>) -> PyResult<Self> {
        Ok(Self {
            lo: PyList::new(py, [f64::INFINITY; 3])?.unbind(),
            hi: PyList::new(py, [f64::NEG_INFINITY; 3])?.unbind(),
        })
    }
    fn add(&self, py: Python<'_>, x: f64, y: f64, z: f64) -> PyResult<()> {
        self.absorb(py, [x, y, z], [x, y, z])
    }
    fn add_array(&self, py: Python<'_>, pts: &Bound<'_, PyAny>) -> PyResult<()> {
        let a = py
            .import("numpy")?
            .call_method1("asarray", (pts, "float64"))?
            .call_method1("reshape", (-1, 3))?;
        let a = a.extract::<PyReadonlyArray2<f64>>()?;
        let a = a.as_array();
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        let mut nan = [false; 3];
        for row in a.rows() {
            for k in 0..3 {
                let n = row[k];
                nan[k] |= n.is_nan();
                lo[k] = lo[k].min(n);
                hi[k] = hi[k].max(n);
            }
        }
        for k in 0..3 {
            if nan[k] {
                lo[k] = f64::NAN;
                hi[k] = f64::NAN;
            }
        }
        self.absorb(py, lo, hi)
    }
    fn offset<'p>(&self, py: Python<'p>) -> PyResult<Option<Bound<'p, PyTuple>>> {
        survey_offset(py, self.lo.bind(py).extract()?, self.hi.bind(py).extract()?)
    }
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("RECENTER_BEYOND_M", BEYOND)?;
    m.add_class::<Bounds>()?;
    m.add_function(wrap_pyfunction!(survey_offset, m)?)
}
