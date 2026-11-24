// SPDX-License-Identifier: GPL-3.0-or-later
//! Transient tool records and callbacks of the document host.
use crate::document_host::*;
use pyo3::exceptions::{PyKeyError, PyRuntimeError};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

#[pyclass(get_all, set_all)]
struct SnapEdge {
    a: Py<PyAny>,
    b: Py<PyAny>,
    center: bool,
    component_origin: bool,
    arc_midpoint: bool,
    guide: bool,
    in_group: bool,
    context: Option<Py<PyAny>>,
    group: Option<Py<PyAny>>,
    figure: bool,
}
#[pymethods]
impl SnapEdge {
    #[new]
    #[pyo3(signature=(a,b,center=false,component_origin=false,arc_midpoint=false,guide=false,in_group=false,context=None,group=None,figure=false))]
    fn new(
        a: Py<PyAny>,
        b: Py<PyAny>,
        center: bool,
        component_origin: bool,
        arc_midpoint: bool,
        guide: bool,
        in_group: bool,
        context: Option<Py<PyAny>>,
        group: Option<Py<PyAny>>,
        figure: bool,
    ) -> Self {
        Self {
            a,
            b,
            center,
            component_origin,
            arc_midpoint,
            guide,
            in_group,
            context,
            group,
            figure,
        }
    }
}
#[pyfunction]
fn copy_snap_edge<'py>(e: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = e.py();
    let h = host(py)?;
    let cls = h.getattr("QVector3D")?;
    let kw = PyDict::new(py);
    for k in [
        "a",
        "b",
        "center",
        "component_origin",
        "arc_midpoint",
        "guide",
        "in_group",
        "context",
        "group",
        "figure",
    ] {
        let val = e.getattr(k)?;
        kw.set_item(
            k,
            if val.is_instance(&cls)? {
                cls.call1((val,))?
            } else {
                val
            },
        )?;
    }
    h.getattr("_SnapEdge")?.call((), Some(&kw))
}
#[pyfunction]
fn lazy_chunk_value<'py>(d: &Obj<'py>, k: &Obj<'py>) -> PyResult<Obj<'py>> {
    let make = d
        .getattr("_later")?
        .call_method1("pop", (k, d.py().None()))?;
    if make.is_none() {
        return Err(PyKeyError::new_err(k.clone().unbind()));
    }
    let value = make.call0()?;
    d.cast::<PyDict>()?.set_item(k, &value)?;
    Ok(value)
}
#[pyfunction]
fn lazy_chunk_fill<'py>(d: &Obj<'py>) -> PyResult<Obj<'py>> {
    let later = d.getattr("_later")?.cast_into::<PyDict>()?;
    for k in later.keys() {
        d.get_item(k)?;
    }
    Ok(d.clone())
}
#[pyfunction]
fn inst_proto_of<'py>(engine: &Obj<'py>, base: &Obj<'py>) -> PyResult<Obj<'py>> {
    let np = engine.py().import("numpy")?;
    let zero = np.call_method1("zeros", ((0, 3),))?;
    let v0 = base.call_method1("get", ("v0",))?;
    let mut rows = [zero.clone(), zero.clone(), zero];
    if !v0.is_none() {
        for (i, k) in ["v0", "e1", "e2"].into_iter().enumerate() {
            rows[i] = np.call_method1("ascontiguousarray", (base.get_item(k)?,))?;
        }
    }
    let e = base.call_method1("get", ("edges",))?;
    let edges = if e.is_truthy()? {
        np.call_method1("frombuffer", (e, "float32"))?
            .call_method1("reshape", (-1, 6))?
            .call_method1("astype", ("float64",))?
    } else {
        np.call_method1("zeros", ((0, 6),))?
    };
    let [a, b, c] = rows;
    engine.call_method1("InstanceProto", (a, b, c, edges))
}
#[pyfunction]
fn fp_winding(fp: &Obj) -> PyResult<i64> {
    if fp.len()? > 7 {
        let term = fp.get_item(7)?;
        if term.get_item(0)?.eq("winding")? {
            return term.get_item(1)?.extract();
        }
    }
    Ok(0)
}
#[pyfunction]
fn finite_point(p: &Obj) -> PyResult<bool> {
    Ok(!p.is_none() && xyz(p)?.is_finite())
}
#[pyfunction]
fn occlusion_native(py: Python) -> PyResult<Obj> {
    if host(py)?.getattr("_OCCL_NATIVE")?.is_truthy()? {
        let m = py.import("core._native")?.getattr("native")?;
        if m.hasattr("occlusion_span")? {
            return Ok(m);
        }
    }
    Ok(none(py))
}
#[pyfunction]
fn alt_filter(w: &Obj, event: &Obj) -> PyResult<bool> {
    let py = w.py();
    let h = host(py)?;
    let cls = h.getattr("QEvent")?;
    let t = event.call_method0("type")?;
    let down = t.eq(cls.getattr("KeyPress")?)?;
    if t.eq(cls.getattr("MouseButtonPress")?)? {
        w.setattr("_clean", false)?;
        return Ok(false);
    }
    if (!down && !t.eq(cls.getattr("KeyRelease")?)?)
        || event.call_method0("isAutoRepeat")?.is_truthy()?
    {
        return Ok(false);
    }
    if !event
        .call_method0("key")?
        .eq(h.getattr("Qt")?.getattr("Key_Alt")?)?
    {
        if down {
            w.setattr("_clean", false)?;
        }
        return Ok(false);
    }
    let mut tap = !down && w.getattr("_clean")?.is_truthy()?;
    w.setattr("_clean", down)?;
    let views = PyList::new(
        py,
        w.getattr("_viewports")?
            .try_iter()?
            .collect::<PyResult<Vec<_>>>()?,
    )?;
    for v in views.iter() {
        let apply = || -> PyResult<bool> {
            if v.getattr("_alt_down")?.is_truthy()? != down {
                v.call_method1("_apply_alt_cursor", (down,))?;
            }
            Ok(tap && v.call_method0("_alt_tapped")?.is_truthy()?)
        };
        match apply() {
            Ok(true) => tap = false,
            Err(e) if !e.is_instance_of::<PyRuntimeError>(py) => return Err(e),
            _ => {}
        }
    }
    Ok(false)
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_class::<SnapEdge>()?;
    m.add_function(wrap_pyfunction!(alt_filter, m)?)?;
    m.add_function(wrap_pyfunction!(lazy_chunk_fill, m)?)?;
    m.add_function(wrap_pyfunction!(lazy_chunk_value, m)?)?;

    m.add_function(wrap_pyfunction!(inst_proto_of, m)?)?;
    m.add_function(wrap_pyfunction!(copy_snap_edge, m)?)?;
    m.add_function(wrap_pyfunction!(fp_winding, m)?)?;
    m.add_function(wrap_pyfunction!(finite_point, m)?)?;
    m.add_function(wrap_pyfunction!(occlusion_native, m)?)?;
    Ok(())
}
