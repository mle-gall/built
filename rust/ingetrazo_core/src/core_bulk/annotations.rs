// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::types::{PyDict, PyList};
fn vec(py: Python<'_>) -> PyResult<Obj<'_>> {
    py.import("ingetrazo_core")?.getattr("Vec3")
}
fn copy<'p>(v: &Obj<'p>) -> PyResult<Obj<'p>> {
    vec(v.py())?.call1((v,))
}
fn coords<'p>(v: &Obj<'p>) -> PyResult<Obj<'p>> {
    v.py()
        .get_type::<PyList>()
        .call1((v.call_method0("toTuple")?,))
}
fn unpack<'p>(v: &Obj<'p>) -> PyResult<Obj<'p>> {
    let t = v.py().get_type::<PyTuple>().call1((v,))?;
    vec(v.py())?.call(t.cast::<PyTuple>()?, None)
}
fn get<'p>(d: &Obj<'p>, k: &str) -> PyResult<Obj<'p>> {
    d.call_method1("get", (k,))
}
fn unit<'p>(v: &Obj<'p>, up: bool) -> PyResult<Obj<'p>> {
    let v = copy(v)?;
    if v.call_method0("length")?.gt(1e-12)? {
        v.call_method0("normalized")
    } else if up {
        vec(v.py())?.call1((0., 0., 1.))
    } else {
        Ok(v.py().None().into_bound(v.py()))
    }
}
#[pyfunction(signature=(this,point,direction=None,origin=None,layer=None))]
fn guide_init(
    this: &Obj<'_>,
    point: &Obj<'_>,
    direction: Option<&Obj<'_>>,
    origin: Option<&Obj<'_>>,
    layer: Option<&Obj<'_>>,
) -> PyResult<()> {
    this.setattr("point", copy(point)?)?;
    let d = direction.map(|d| unit(d, false)).transpose()?;
    this.setattr("direction", &d)?;
    this.setattr(
        "origin",
        if d.as_ref().is_none_or(|d| d.is_none()) {
            origin.map(copy).transpose()?
        } else {
            None
        },
    )?;
    this.setattr("layer", layer)
}
#[pyfunction(signature=(this,center,normal,radius,layer=None))]
fn circle_init(
    this: &Obj<'_>,
    center: &Obj<'_>,
    normal: &Obj<'_>,
    radius: f64,
    layer: Option<&Obj<'_>>,
) -> PyResult<()> {
    this.setattr("center", copy(center)?)?;
    this.setattr("normal", unit(normal, true)?)?;
    this.setattr("radius", radius)?;
    this.setattr("origin", this.py().None())?;
    this.setattr("layer", layer)
}
#[pyfunction(signature=(this,anchor,offset,text,layer=None,attachment=None))]
fn label_init(
    this: &Obj<'_>,
    anchor: &Obj<'_>,
    offset: &Obj<'_>,
    text: &Obj<'_>,
    layer: Option<&Obj<'_>>,
    attachment: Option<&Obj<'_>>,
) -> PyResult<()> {
    this.setattr("_anchor", anchor)?;
    this.setattr("offset", offset)?;
    this.setattr("text", text)?;
    this.setattr("layer", layer)?;
    this.setattr("attachment", attachment)
}
#[pyfunction]
fn anchor<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    let a = this.getattr("attachment")?;
    if !a.is_none() {
        let p = a.call_method0("position")?;
        if !p.is_none() && !p.eq(this.getattr("_anchor")?)? {
            this.setattr("_anchor", p)?;
        }
    }
    this.getattr("_anchor")
}
#[pyfunction]
fn set_anchor(this: &Obj<'_>, value: &Obj<'_>) -> PyResult<()> {
    this.setattr("_anchor", value)
}
#[pyfunction]
fn position<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    this.getattr("anchor")?.add(this.getattr("offset")?)
}
#[pyfunction]
fn circle_true(_this: &Obj<'_>) -> bool {
    true
}
#[pyfunction]
fn circle_false(_this: &Obj<'_>) -> bool {
    false
}
#[pyfunction]
fn is_line(this: &Obj<'_>) -> PyResult<bool> {
    Ok(!this.getattr("direction")?.is_none())
}
#[pyfunction]
fn center<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    this.getattr("center")
}
#[pyfunction]
fn normal<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    this.getattr("normal")
}
#[pyfunction]
fn set_center(this: &Obj<'_>, value: &Obj<'_>) -> PyResult<()> {
    this.setattr("center", copy(value)?)
}
#[pyfunction]
fn set_normal(this: &Obj<'_>, value: &Obj<'_>) -> PyResult<()> {
    if !value.is_none() {
        this.setattr("normal", unit(value, true)?)?;
    }
    Ok(())
}
#[pyfunction]
fn segment<'p>(this: &Obj<'p>) -> PyResult<(Obj<'p>, Obj<'p>)> {
    let p = this.getattr("point")?;
    let dir = this.getattr("direction")?;
    if dir.is_none() {
        Ok((copy(&p)?, copy(&p)?))
    } else {
        let d = dir.mul(1e4)?;
        Ok((p.sub(&d)?, p.add(d)?))
    }
}
#[pyfunction]
fn a<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    this.call_method0("segment")?.get_item(0)
}
#[pyfunction]
fn b<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    this.call_method0("segment")?.get_item(1)
}
#[pyfunction]
fn is_finite(this: &Obj<'_>) -> PyResult<bool> {
    let circle = this
        .getattr_opt("is_circle")?
        .is_some_and(|v| v.is_truthy().unwrap_or(false));
    for field in if circle {
        &["center", "normal"][..]
    } else {
        &["point", "direction", "origin"][..]
    } {
        let v = this.getattr(*field)?;
        if !v.is_none()
            && !v
                .call_method0("toTuple")?
                .extract::<[f64; 3]>()?
                .iter()
                .all(|v| v.is_finite())
        {
            return Ok(false);
        }
    }
    Ok(!circle || this.getattr("radius")?.extract::<f64>()?.is_finite())
}
#[pyfunction(signature=(this,n=64))]
fn sample_points<'p>(this: &Obj<'p>, n: i64) -> PyResult<Obj<'p>> {
    let py = this.py();
    let basis = py
        .import("ingetrazo_core.axes")?
        .call_method1("plane_axes", (this.getattr("normal")?,))?;
    let (u, v) = (basis.get_item(0)?, basis.get_item(1)?);
    let r = this.getattr("radius")?;
    let center = this.getattr("center")?;
    let out = PyList::empty(py);
    for i in 0..n {
        let t = std::f64::consts::TAU * i as f64 / n as f64;
        out.append(
            center.add(
                u.mul(crate::model::pyfloat::cos(t))?
                    .add(v.mul(crate::model::pyfloat::sin(t))?)?
                    .mul(&r)?,
            )?,
        )?;
    }
    Ok(out.into_any())
}
#[pyfunction]
fn guide_dict<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    let d = PyDict::new(this.py());
    let circle = this.getattr_opt("is_circle")?.is_some();
    if circle {
        d.set_item("kind", "circle")?;
        d.set_item("radius", this.getattr("radius")?)?;
    }
    for field in if circle {
        &["center", "normal"][..]
    } else {
        &["point", "direction", "origin"][..]
    } {
        let v = this.getattr(*field)?;
        if !v.is_none() {
            d.set_item(*field, coords(&v)?)?;
        }
    }
    let layer = this.getattr("layer")?;
    if layer.is_truthy()? {
        d.set_item("layer", layer)?;
    }
    Ok(d.into_any())
}
fn layer<'p>(data: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    let l = get(data, "layer")?;
    Ok(if l.is_truthy()? { Some(l) } else { None })
}
#[pyfunction]
fn guide_from<'p>(cls: &Obj<'p>, data: &Obj<'p>) -> PyResult<Obj<'p>> {
    let optional = |n: &str| -> PyResult<Option<Obj<'p>>> {
        let v = get(data, n)?;
        Ok(if v.is_truthy()? {
            Some(unpack(&v)?)
        } else {
            None
        })
    };
    cls.call1((
        unpack(&data.get_item("point")?)?,
        optional("direction")?,
        optional("origin")?,
        layer(data)?,
    ))
}
#[pyfunction]
fn circle_from<'p>(cls: &Obj<'p>, data: &Obj<'p>) -> PyResult<Obj<'p>> {
    let v = |n: &str, d: (f64, f64, f64)| -> PyResult<Obj<'p>> {
        let v = get(data, n)?;
        if v.is_truthy()? {
            unpack(&v)
        } else {
            vec(cls.py())?.call1(d)
        }
    };
    cls.call1((
        v("center", (0., 0., 0.))?,
        v("normal", (0., 0., 1.))?,
        data.call_method1("get", ("radius", 0.))?,
        layer(data)?,
    ))
}
#[pyfunction]
fn label_dict<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    let d = PyDict::new(this.py());
    let a = this.getattr("attachment")?;
    let binding = if a.is_none() {
        None
    } else {
        Some(a.call_method0("archive")?)
    };
    for f in ["anchor", "offset"] {
        d.set_item(f, coords(&this.getattr(f)?)?)?;
    }
    d.set_item("text", this.getattr("text")?)?;
    if let Some(b) = binding {
        if b.is_truthy()? {
            d.set_item("attachment", b)?;
        }
    }
    let l = this.getattr("layer")?;
    if l.is_truthy()? {
        d.set_item("layer", l)?;
    }
    Ok(d.into_any())
}
#[pyfunction(signature=(cls,data,scene=None))]
fn label_from<'p>(cls: &Obj<'p>, data: &Obj<'p>, scene: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    let point = unpack(&data.call_method1("get", ("anchor", (0., 0., 0.)))?)?;
    let bind = if let Some(s) = scene {
        Some(
            cls.py()
                .import("ingetrazo_core.annotation_anchor")?
                .getattr("Attachment")?
                .call_method1("restore", (s, get(data, "attachment")?, &point))?,
        )
    } else {
        None
    };
    cls.call1((
        point,
        unpack(&data.call_method1("get", ("offset", (0., 0., 0.)))?)?,
        data.call_method1("get", ("text", ""))?,
        layer(data)?,
        bind,
    ))
}
#[pyfunction(pass_module)]
fn is_guide(m: &Module<'_>, entity: &Obj<'_>) -> PyResult<bool> {
    Ok(entity.is_instance(&m.getattr("Guide")?)?
        || entity.is_instance(&m.getattr("GuideCircle")?)?)
}
#[pyfunction(pass_module)]
fn guide_from_dict<'p>(m: &Module<'p>, data: &Obj<'p>) -> PyResult<Obj<'p>> {
    m.getattr(if get(data, "kind")?.eq("circle")? {
        "GuideCircle"
    } else {
        "Guide"
    })?
    .call_method1("from_dict", (data,))
}
#[pyfunction(pass_module, name = "__getattr__")]
fn get_class<'p>(m: &Module<'p>, name: &str) -> PyResult<Obj<'p>> {
    let label = m.name()?.to_string().ends_with(".textlabel");
    if !(label && name == "TextLabel" || !label && matches!(name, "Guide" | "GuideCircle")) {
        return Err(pyo3::exceptions::PyAttributeError::new_err(name.to_owned()));
    }
    let py = m.py();
    let dm = py.import("core.doc_model")?;
    let ns = PyDict::new(py);
    let builtins = py.import("builtins")?;
    let fields = match name {
        "TextLabel" => {
            for (n, f) in [
                ("__init__", wrap_pyfunction!(label_init, m)?),
                ("position", wrap_pyfunction!(position, m)?),
                ("to_dict", wrap_pyfunction!(label_dict, m)?),
            ] {
                method(&ns, n, f.as_any())?;
            }
            ns.set_item(
                "anchor",
                builtins.call_method1(
                    "property",
                    (
                        wrap_pyfunction!(anchor, m)?,
                        wrap_pyfunction!(set_anchor, m)?,
                    ),
                )?,
            )?;
            ns.set_item(
                "from_dict",
                builtins.call_method1("classmethod", (wrap_pyfunction!(label_from, m)?,))?,
            )?;
            "_anchor offset text layer attachment"
        }
        "Guide" => {
            for (n, f) in [
                ("__init__", wrap_pyfunction!(guide_init, m)?),
                ("is_finite", wrap_pyfunction!(is_finite, m)?),
                ("segment", wrap_pyfunction!(segment, m)?),
                ("to_dict", wrap_pyfunction!(guide_dict, m)?),
            ] {
                method(&ns, n, f.as_any())?;
            }
            for (n, f) in [
                ("a", wrap_pyfunction!(a, m)?),
                ("b", wrap_pyfunction!(b, m)?),
                ("is_line", wrap_pyfunction!(is_line, m)?),
            ] {
                ns.set_item(n, builtins.call_method1("property", (f,))?)?;
            }
            ns.set_item(
                "from_dict",
                builtins.call_method1("classmethod", (wrap_pyfunction!(guide_from, m)?,))?,
            )?;
            "point direction origin layer"
        }
        _ => {
            for (n, f) in [
                ("__init__", wrap_pyfunction!(circle_init, m)?),
                ("is_finite", wrap_pyfunction!(is_finite, m)?),
                ("sample_points", wrap_pyfunction!(sample_points, m)?),
                ("to_dict", wrap_pyfunction!(guide_dict, m)?),
            ] {
                method(&ns, n, f.as_any())?;
            }
            ns.set_item(
                "is_circle",
                builtins.call_method1("property", (wrap_pyfunction!(circle_true, m)?,))?,
            )?;
            ns.set_item(
                "is_line",
                builtins.call_method1("property", (wrap_pyfunction!(circle_false, m)?,))?,
            )?;
            ns.set_item(
                "point",
                builtins.call_method1(
                    "property",
                    (
                        wrap_pyfunction!(center, m)?,
                        wrap_pyfunction!(set_center, m)?,
                    ),
                )?,
            )?;
            ns.set_item(
                "direction",
                builtins.call_method1(
                    "property",
                    (
                        wrap_pyfunction!(normal, m)?,
                        wrap_pyfunction!(set_normal, m)?,
                    ),
                )?,
            )?;
            ns.set_item(
                "from_dict",
                builtins.call_method1("classmethod", (wrap_pyfunction!(circle_from, m)?,))?,
            )?;
            "center normal radius layer origin"
        }
    };
    ns.set_item(
        "__module__",
        if label {
            "core.textlabel"
        } else {
            "core.guide"
        },
    )?;
    let cls = builtins.call_method1("type", (name, (dm.getattr("Record")?,), ns))?;
    dm.getattr("record_fields")?
        .call(
            PyTuple::new(py, fields.split_whitespace().collect::<Vec<_>>())?,
            None,
        )?
        .call1((&cls,))?;
    m.add(name, &cls)?;
    Ok(cls)
}
pub(super) fn install(m: &Module<'_>, label: bool) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(get_class, m)?)?;
    if !label {
        m.add("GUIDE_HALF_LEN", 1e4)?;
        m.add_function(wrap_pyfunction!(is_guide, m)?)?;
        m.add_function(wrap_pyfunction!(guide_from_dict, m)?)?;
    }
    Ok(())
}
