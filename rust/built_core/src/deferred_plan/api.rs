// SPDX-License-Identifier: GPL-3.0-or-later
//! Read a deferred block without building its live geometry.
use super::{deferred_incidence, deferred_weld, Obj};
use crate::math::{xyz, Vec3};
use pyo3::{
    prelude::*,
    types::{PyDict, PyInt, PyList},
};
#[pyfunction]
#[pyo3(signature=(pos,edges,flags,curves,layers,layer_names,rings,sizes,counts,sequential=false))]
pub(super) fn deferred_plan<'py>(
    pos: &Obj<'py>,
    edges: &Obj<'py>,
    flags: Option<&Obj<'py>>,
    curves: Option<&Obj<'py>>,
    layers: Option<&Obj<'py>>,
    layer_names: &Obj<'py>,
    rings: &Obj<'py>,
    sizes: &Obj<'py>,
    counts: &Obj<'py>,
    sequential: bool,
) -> PyResult<Obj<'py>> {
    let py = pos.py();
    let np = crate::model::module(py, "numpy")?;
    let dtype = np.getattr("int64")?;
    let as_array = np.getattr("ascontiguousarray")?;
    let col = |v: &Obj<'py>| as_array.call1((v, &dtype));
    let optional = |v: Option<&Obj<'py>>| {
        v.map(|v| -> PyResult<_> { Ok(col(v)?.extract::<numpy::PyReadonlyArray1<i64>>()?) })
            .transpose()
    };
    let sizes = col(sizes)?;
    let counts = col(counts)?;
    let (pos, ids) = deferred_weld(pos, sequential, None)?;
    let edges = col(edges)?.call_method1("reshape", (-1, 2))?;
    let rings = col(rings)?;
    let data = deferred_incidence(
        py,
        ids.extract()?,
        edges.extract()?,
        optional(flags)?,
        optional(curves)?,
        optional(layers)?,
        layer_names,
        rings.extract()?,
        sizes.extract()?,
        counts.extract()?,
    )?
    .cast_into::<PyDict>()?;
    data.set_item("pos", pos)?;
    data.set_item("ring_sizes", sizes)?;
    data.set_item("ring_counts", counts)?;
    crate::model::module(py, "built_core.lazy_mesh")?
        .getattr("Plan")?
        .call((), Some(&data))
}
#[pyfunction]
pub(super) fn plan_of<'py>(mesh: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let py = mesh.py();
    if !mesh
        .get_type()
        .is(crate::model::module(py, "built_core.store_mesh")?.getattr("DeferredMesh")?)
    {
        return Ok(None);
    }
    let d = mesh.getattr("__dict__")?.cast_into::<PyDict>()?;
    if let Some(p) = d.get_item("_deferred_plan")?.filter(|v| !v.is_none()) {
        return Ok(Some(p));
    }
    let s = d.as_any().get_item("_deferred_source")?;
    if s.is_none() {
        return Ok(None);
    }
    let get = |k: &str| s.get_item(k);
    let opt =
        |k: &str| -> PyResult<Option<Obj<'py>>> { Ok(Some(get(k)?).filter(|v| !v.is_none())) };
    let names = s.get_item("meta")?.call_method1("get", ("edge_layers",))?;
    let names = if names.is_truthy()? {
        names
    } else {
        PyList::empty(py).into_any()
    };
    let p = deferred_plan(
        &get("pos")?,
        &get("edges")?,
        opt("flags")?.as_ref(),
        opt("curves")?.as_ref(),
        opt("layers")?.as_ref(),
        &names,
        &get("rings")?,
        &get("sizes")?,
        &get("counts")?,
        s.call_method1("get", ("sequential", false))?.is_truthy()?,
    )?;
    d.set_item("_deferred_plan", &p)?;
    Ok(Some(p))
}
#[pyfunction]
pub(super) fn entity_count(mesh: &Obj<'_>) -> PyResult<usize> {
    if let Some(p) = plan_of(mesh)? {
        Ok(p.getattr("ring_counts")?.len()? + p.getattr("edges")?.len()?)
    } else {
        Ok(mesh.getattr("faces")?.len()? + mesh.getattr("edges")?.len()?)
    }
}
#[pyclass(module = "built_core", skip_from_py_object)]
struct PlannedVertex {
    #[pyo3(get, set)]
    position: Py<Vec3>,
}
#[pyclass(module = "built_core", sequence, skip_from_py_object)]
struct PlannedVertices {
    #[pyo3(get, set)]
    pos: Py<PyAny>,
}
#[pymethods]
impl PlannedVertices {
    #[new]
    fn new(pos: Py<PyAny>) -> Self {
        Self { pos }
    }
    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        self.pos.bind(py).len()
    }
    fn __getitem__(&self, py: Python<'_>, i: &Obj<'_>) -> PyResult<PlannedVertex> {
        let p = self
            .pos
            .bind(py)
            .get_item(py.get_type::<PyInt>().call1((i,))?)?;
        Ok(PlannedVertex {
            position: Py::new(py, Vec3::from(xyz(&p)?))?,
        })
    }
}
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(deferred_plan, m)?)?;
    m.add_function(wrap_pyfunction!(entity_count, m)?)?;
    m.add_function(wrap_pyfunction!(plan_of, m)?)?;
    m.add_class::<PlannedVertices>()?;
    Ok(())
}
