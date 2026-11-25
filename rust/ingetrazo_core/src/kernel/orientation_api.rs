// SPDX-License-Identifier: GPL-3.0-or-later
//! Apply the orientation kernel's decisions to live faces, preserving paint
//! by physical side. Detached mesh subclasses retain their mutation hooks.
use super::*;
#[pyfunction(name = "is_closed")]
fn closed(mesh: &Obj) -> PyResult<bool> {
    if !mesh.getattr("faces")?.is_truthy()? {
        return Ok(false);
    }
    if let Some(closed) = super::is_closed(mesh)? {
        return Ok(closed);
    }
    for e in mesh.getattr("edges")?.try_iter()? {
        if e?.getattr("faces")?.len()? < 2 {
            return Ok(false);
        }
    }
    Ok(true)
}
#[pyfunction]
fn signed_volume(mesh: &Obj) -> PyResult<f64> {
    let mut volume = 0.;
    for f in mesh.getattr("faces")?.try_iter()? {
        let f = f?;
        if f.getattr("interior")?.is_truthy()? {
            continue;
        }
        for tri in f.call_method0("triangulate")?.try_iter()? {
            let tri = tri?;
            let a = xyz(&tri.get_item(0)?)?;
            let b = xyz(&tri.get_item(1)?)?;
            let c = xyz(&tri.get_item(2)?)?;
            volume += a.dot(b.cross(c)) / 6.;
        }
    }
    Ok(volume)
}
#[pyfunction(signature=(mesh,seed=12345,only=None))]
fn orient_outward<'p>(mesh: &Obj<'p>, seed: u64, only: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    let py = mesh.py();
    if !mesh.getattr("faces")?.is_truthy()? {
        return Ok(PyList::empty(py).into_any());
    }
    let config = py.import("core._native")?;
    let mode = config.call_method0("qt_hypot")?;
    let native = config.getattr("native")?;
    if native.is_none() || mode.is_none() || !native.hasattr("orient_decision")? {
        return Err(pyo3::exceptions::PyRuntimeError::new_err(
            "orient_outward needs the native engine (ingetrazo_core.orient_decision)",
        ));
    }
    let scan = Scan::open(mesh, true)?.expect("a required scan is always constructed");
    let subset = only
        .map(|v| {
            v.try_iter()?
                .collect::<PyResult<Vec<_>>>()
                .and_then(|rows| PyList::new(py, rows))
        })
        .transpose()?;
    let (marks, flips) = crate::orient::orient_decision(
        py,
        scan.native.try_borrow_mut()?,
        subset,
        seed,
        7,
        3e-4,
        mode.extract()?,
        sum_mode(py),
    )?;
    let marked = scan.view.call_method1(
        "faces",
        (marks.iter().map(|(i, _)| *i).collect::<Vec<_>>(),),
    )?;
    let turned = scan.view.call_method1("faces", (flips,))?;
    let api = py.import("ingetrazo_core.mesh")?;
    if mesh.get_type().is(&py
        .import("ingetrazo_core.store_mesh")?
        .getattr("StoreMesh")?)
    {
        if !marks.is_empty() || turned.len()? > 0 {
            let flags = marked
                .try_iter()?
                .zip(marks)
                .map(|(f, (_, flag))| Ok((f?.getattr("key")?, flag)))
                .collect::<PyResult<Vec<_>>>()?;
            let keys = turned
                .try_iter()?
                .map(|f| f?.getattr("key"))
                .collect::<PyResult<Vec<_>>>()?;
            mesh.getattr("_store")?
                .call_method1("apply_orientation", (flags, keys))?;
            let swap = api.getattr("swap_orientation_paint")?;
            for f in turned.try_iter()? {
                swap.call1((f?,))?;
            }
        }
    } else {
        for (f, (_, flag)) in marked.try_iter()?.zip(marks) {
            f?.setattr("interior", flag)?;
        }
        let flip = api.getattr("turn_face_over")?;
        for f in turned.try_iter()? {
            flip.call1((f?,))?;
        }
    }
    Ok(turned)
}
pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    let name = "ingetrazo_core.orient";
    let sub = PyModule::new(py, name)?;
    for f in [
        wrap_pyfunction!(closed, &sub)?,
        wrap_pyfunction!(signed_volume, &sub)?,
        wrap_pyfunction!(orient_outward, &sub)?,
    ] {
        sub.add_function(f)?;
    }
    // Script access to the mesh operation is resolved after mesh registration.
    sub.add_function(wrap_pyfunction!(lazy, &sub)?)?;
    py.import("sys")?.getattr("modules")?.set_item(name, &sub)?;
    sub.setattr(
        "__all__",
        [
            "is_closed",
            "signed_volume",
            "orient_outward",
            "turn_face_over",
        ],
    )?;
    m.add("orient", sub)
}
#[pyfunction(name = "__getattr__")]
fn lazy<'p>(py: Python<'p>, name: &str) -> PyResult<Obj<'p>> {
    if name == "turn_face_over" {
        return py.import("ingetrazo_core.mesh")?.getattr(name);
    }
    Err(pyo3::exceptions::PyAttributeError::new_err(name.to_owned()))
}
