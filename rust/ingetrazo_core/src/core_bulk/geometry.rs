// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::types::PyDict;
#[pyfunction]
fn length<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    this.getattr("b")?
        .sub(this.getattr("a")?)?
        .call_method0("length")
}
#[pyfunction]
fn newell<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    this.py()
        .import("ingetrazo_core")?
        .call_method1("polygon_newell", (this.getattr("vertices")?,))
}
#[pyfunction]
fn normal<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    if this.getattr("vertices")?.len()? >= 3 {
        let n = this.call_method0("_newell")?;
        if !n.call_method0("length")?.lt(1e-9)? {
            return n.call_method0("normalized");
        }
    }
    this.py()
        .import("ingetrazo_core")?
        .call_method1("Vec3", (0., 0., 1.))
}
#[pyfunction]
fn area(this: &Obj<'_>) -> PyResult<f64> {
    if this.getattr("vertices")?.len()? < 3 {
        return Ok(0.);
    }
    Ok(0.5
        * this
            .call_method0("_newell")?
            .call_method0("length")?
            .extract::<f64>()?)
}
#[pyfunction]
fn centroid<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    this.py()
        .import("ingetrazo_core")?
        .call_method1("polygon_centroid", (this.getattr("vertices")?,))
}
#[pyfunction]
fn triangulate<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    let v = this.getattr("vertices")?;
    if v.len()? < 3 {
        return Ok(pyo3::types::PyList::empty(this.py()).into_any());
    }
    this.py().import("ingetrazo_core")?.call_method1(
        "polygon_triangles",
        (v, this.getattr("holes")?, this.call_method0("normal")?),
    )
}
pub(super) fn install(m: &Module<'_>, core: &Module<'_>) -> PyResult<()> {
    let py = m.py();
    let dc = py.import("dataclasses")?;
    let kw = PyDict::new(py);
    kw.set_item("eq", false)?;
    let decorate = dc.getattr("dataclass")?.call((), Some(&kw))?;
    for (name, fields) in [
        ("Edge", &["a", "b"][..]),
        ("Face", &["vertices", "holes", "attrs"][..]),
    ] {
        let ns = PyDict::new(py);
        let ann = PyDict::new(py);
        for &f in fields {
            ann.set_item(
                f,
                if name == "Edge" {
                    "QVector3D"
                } else if f == "attrs" {
                    "dict | None"
                } else if f == "holes" {
                    "list[list[QVector3D]]"
                } else {
                    "list[QVector3D]"
                },
            )?;
            if f == "vertices" || f == "holes" {
                let kw = PyDict::new(py);
                kw.set_item("default_factory", py.get_type::<pyo3::types::PyList>())?;
                ns.set_item(f, dc.getattr("field")?.call((), Some(&kw))?)?;
            } else if f == "attrs" {
                ns.set_item(f, py.None())?;
            }
        }
        ns.set_item("__annotations__", ann)?;
        ns.set_item("__module__", m.name()?)?;
        if name == "Edge" {
            method(&ns, "length", wrap_pyfunction!(length, m)?.as_any())?;
        } else {
            for (n, f) in [
                ("_newell", wrap_pyfunction!(newell, m)?),
                ("normal", wrap_pyfunction!(normal, m)?),
                ("area", wrap_pyfunction!(area, m)?),
                ("centroid", wrap_pyfunction!(centroid, m)?),
                ("triangulate", wrap_pyfunction!(triangulate, m)?),
            ] {
                method(&ns, n, f.as_any())?;
            }
        }
        let cls = py
            .import("builtins")?
            .call_method1("type", (name, PyTuple::empty(py), ns))?;
        let cls = decorate.call1((cls,))?;
        cls.setattr("__module__", "core.geometry")?;
        m.add(name, cls)?;
    }
    m.add("QVector3D", core.getattr("Vec3")?)?;
    Ok(())
}
