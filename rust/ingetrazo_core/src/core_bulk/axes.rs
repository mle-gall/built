// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::types::{PyDict, PyTuple};
fn vector(py: Python<'_>) -> PyResult<Obj<'_>> {
    py.import("ingetrazo_core")?.getattr("Vec3")
}
#[pyfunction(pass_module)]
fn is_world(m: &Module<'_>) -> PyResult<bool> {
    Ok(m.getattr("_current")?.is_none())
}
#[pyfunction(pass_module)]
fn origin<'p>(m: &Module<'p>) -> PyResult<Obj<'p>> {
    vector(m.py())?.call1((m.getattr("ORIGIN")?,))
}
#[pyfunction(pass_module)]
fn axis<'p>(m: &Module<'p>, name: &str) -> PyResult<Obj<'p>> {
    vector(m.py())?.call1((m.getattr("AXES")?.get_item(name)?,))
}
#[pyfunction(pass_module)]
fn as_tuple<'p>(m: &Module<'p>, name: &str) -> PyResult<Obj<'p>> {
    m.getattr("AXES")?.get_item(name)?.call_method0("toTuple")
}
#[pyfunction(pass_module)]
fn sync(m: &Module<'_>, frame: &Obj<'_>) -> PyResult<()> {
    let py = m.py();
    let current = m.getattr("_current")?;
    if frame.is(&current) || (!frame.is_none() && !current.is_none() && frame.eq(&current)?) {
        return Ok(());
    }
    let axes = m.getattr("AXES")?;
    let o = m.getattr("ORIGIN")?;
    if frame.is_none() {
        for (name, xyz) in basis() {
            axes.set_item(name, vector(py)?.call1(xyz)?)?;
        }
        for f in ["setX", "setY", "setZ"] {
            o.call_method1(f, (0.,))?;
        }
        m.setattr("_current", py.None())
    } else {
        let row = py
            .import("core.group")?
            .call_method1("frame_axes", (frame,))?;
        for (i, n) in ["x", "y", "z"].iter().enumerate() {
            axes.set_item(*n, row.get_item(i + 1)?)?;
        }
        let point = row.get_item(0)?;
        for (f, g) in [("setX", "x"), ("setY", "y"), ("setZ", "z")] {
            o.call_method1(f, (point.call_method0(g)?,))?;
        }
        m.setattr(
            "_current",
            py.import("ingetrazo_core")?
                .call_method1("Matrix4", (frame,))?,
        )
    }
}
#[pyfunction(pass_module)]
fn to_local<'p>(m: &Module<'p>, v: &Obj<'p>) -> PyResult<Obj<'p>> {
    let vec = vector(m.py())?;
    let axes = m.getattr("AXES")?;
    let args = ["x", "y", "z"]
        .map(|n| vec.call_method1("dotProduct", (v, axes.get_item(n)?)))
        .into_iter()
        .collect::<PyResult<Vec<_>>>()?;
    vec.call(PyTuple::new(m.py(), args)?, None)
}
#[pyfunction(pass_module)]
fn to_world<'p>(m: &Module<'p>, v: &Obj<'p>) -> PyResult<Obj<'p>> {
    let axes = m.getattr("AXES")?;
    let term = |n: &str| axes.get_item(n)?.mul(v.call_method0(n)?);
    term("x")?.add(term("y")?)?.add(term("z")?)
}
#[pyfunction(pass_module)]
fn frame<'p>(m: &Module<'p>) -> PyResult<Obj<'p>> {
    let a = m.getattr("AXES")?;
    Ok((
        m.getattr("ORIGIN")?,
        a.get_item("x")?,
        a.get_item("y")?,
        a.get_item("z")?,
    )
        .into_pyobject(m.py())?
        .into_any())
}
#[pyfunction(pass_module)]
fn plane_axes<'p>(m: &Module<'p>, normal: &Obj<'p>) -> PyResult<Obj<'p>> {
    m.py().import("ingetrazo_core")?.call_method1(
        "drawing_plane_axes",
        (normal, if is_world(m)? { None } else { Some(frame(m)?) }),
    )
}
#[pyfunction]
fn frame_matrix<'p>(o: &Obj<'p>, x: &Obj<'p>, y: &Obj<'p>, z: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = o.py();
    let mut args = Vec::new();
    for n in ["x", "y", "z"] {
        for v in [x, y, z, o] {
            args.push(v.call_method0(n)?.extract::<f64>()?);
        }
    }
    args.extend([0., 0., 0., 1.]);
    py.import("ingetrazo_core")?
        .getattr("Matrix4")?
        .call(PyTuple::new(py, args)?, None)
}
fn basis() -> [(&'static str, (f64, f64, f64)); 3] {
    [
        ("x", (1., 0., 0.)),
        ("y", (0., 1., 0.)),
        ("z", (0., 0., 1.)),
    ]
}
pub(super) fn install(m: &Module<'_>, core: &Module<'_>) -> PyResult<()> {
    let axes = PyDict::new(m.py());
    let v = core.getattr("Vec3")?;
    for (n, p) in basis() {
        axes.set_item(n, v.call1(p)?)?;
    }
    m.add("AXES", axes)?;
    m.add("ORIGIN", v.call0()?)?;
    m.add("_current", m.py().None())?;
    for f in [
        wrap_pyfunction!(is_world, m)?,
        wrap_pyfunction!(origin, m)?,
        wrap_pyfunction!(axis, m)?,
        wrap_pyfunction!(as_tuple, m)?,
        wrap_pyfunction!(sync, m)?,
        wrap_pyfunction!(to_local, m)?,
        wrap_pyfunction!(to_world, m)?,
        wrap_pyfunction!(frame, m)?,
        wrap_pyfunction!(plane_axes, m)?,
        wrap_pyfunction!(frame_matrix, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
