// SPDX-License-Identifier: GPL-3.0-or-later
//! Transient tool records and callbacks of the document host.
use crate::document_host::*;
use pyo3::exceptions::PyKeyError;
use pyo3::{prelude::*, types::PyDict};

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
pub(crate) fn lazy_chunk_fill<'py>(d: &Obj<'py>) -> PyResult<Obj<'py>> {
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
/// Observe Alt before chrome or tools consume the key. Each document owns its tap.
#[pyfunction]
fn observe_alt_key(view: &Obj, pressed: bool, key: i64, repeat: bool) -> PyResult<()> {
    const ALT: i64 = 0x01000023;
    if repeat {
        return Ok(());
    }
    if key != ALT {
        if pressed && opt(view, "_clean_alt_press")?.is_truthy()? {
            view.setattr("_clean_alt_press", false)?;
        }
        return Ok(());
    }
    let tap = !pressed && opt(view, "_clean_alt_press")?.is_truthy()?;
    view.setattr("_clean_alt_press", pressed)?;
    view.call_method1("_apply_alt_cursor", (pressed,))?;
    if tap {
        view.call_method0("_alt_tapped")?;
    }
    Ok(())
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_class::<SnapEdge>()?;
    m.add_function(wrap_pyfunction!(observe_alt_key, m)?)?;
    m.add_function(wrap_pyfunction!(lazy_chunk_fill, m)?)?;
    m.add_function(wrap_pyfunction!(lazy_chunk_value, m)?)?;

    m.add_function(wrap_pyfunction!(inst_proto_of, m)?)?;
    m.add_function(wrap_pyfunction!(copy_snap_edge, m)?)?;
    m.add_function(wrap_pyfunction!(fp_winding, m)?)?;
    m.add_function(wrap_pyfunction!(finite_point, m)?)?;
    m.add_function(wrap_pyfunction!(occlusion_native, m)?)?;
    Ok(())
}
