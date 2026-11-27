// SPDX-License-Identifier: GPL-3.0-or-later
//! Layer and material archive fields, in historical writer order.
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyList, PyString, PyTuple};

fn number(v: &Bound<'_, PyAny>) -> PyResult<f64> {
    v.py().get_type::<PyFloat>().call1((v,))?.extract()
}

#[pyfunction]
pub fn igz_load_layer<'py>(
    raw: &Bound<'py, PyDict>,
    class: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let py = raw.py();
    let kwargs = PyDict::new(py);
    kwargs.set_item("name", "Layer 0")?;
    for (key, value) in raw {
        let Some(name) = key.cast::<PyString>().ok().and_then(|s| s.to_str().ok()) else {
            continue;
        };
        match name {
            "name" | "uid" | "visible" | "locked" | "linetype" | "hatch_color" | "hatch_ground"
            | "hatch_offset" | "hatch_scale" | "hatch_angle" | "brick" | "joints" => {
                kwargs.set_item(&key, value)?
            }
            "hatch_on_paper" if value.is_instance_of::<PyBool>() => kwargs.set_item(&key, value)?,
            "folder" | "key" | "hatch" if value.is_instance_of::<PyString>() => {
                kwargs.set_item(&key, value)?
            }
            "hatch_cross" | "mortar" => kwargs.set_item(&key, value.is(&PyBool::new(py, true)))?,
            "printable" => kwargs.set_item(&key, !value.is(&PyBool::new(py, false)))?,
            "opacity" => kwargs.set_item(&key, number(&value).unwrap_or(1.))?,
            "lineweight" | "cut_weight" => {
                if let Ok(n) = number(&value) {
                    kwargs.set_item(&key, n.max(0.05).min(5.))?;
                }
            }
            "color"
                if (value.is_instance_of::<PyList>() || value.is_instance_of::<PyTuple>())
                    && value.len()? >= 3 =>
            {
                let rgb = (0..3)
                    .map(|i| Ok(number(&value.get_item(i)?)?.max(0.).min(1.)))
                    .collect::<PyResult<Vec<_>>>();
                if let Ok(rgb) = rgb {
                    kwargs.set_item(&key, PyTuple::new(py, rgb)?)?;
                }
            }
            _ => (),
        }
    }
    class.call((), Some(&kwargs))
}

#[pyfunction]
pub fn igz_archive_layer<'py>(layer: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyDict>> {
    let py = layer.py();
    let out = PyDict::new(py);
    let round = py.import("builtins")?.getattr("round")?;
    let rounded = |v: &Bound<'py, PyAny>, n| round.call1((number(v)?, n));
    let rounded_list = |v: &Bound<'py, PyAny>, n, limit| -> PyResult<Bound<'py, PyList>> {
        let l = PyList::empty(py);
        for c in v.try_iter()?.take(limit) {
            l.append(rounded(&c?, n)?)?;
        }
        Ok(l)
    };
    let name = layer.getattr("name")?;
    out.set_item("name", &name)?;
    out.set_item("uid", layer.getattr("uid")?)?;
    let key = layer.getattr("key")?;
    if !key.eq(name)? {
        out.set_item("key", key)?;
    }
    for k in [
        "visible",
        "locked",
        "color",
        "folder",
        "linetype",
        "lineweight",
        "cut_weight",
        "hatch",
        "hatch_angle",
        "hatch_scale",
        "hatch_offset",
        "hatch_on_paper",
        "brick",
        "hatch_cross",
        "joints",
        "mortar",
        "hatch_ground",
        "hatch_color",
        "opacity",
        "printable",
    ] {
        let v = layer.getattr(k)?;
        match k {
            "visible" | "printable" if !v.is_truthy()? => out.set_item(k, false)?,
            "locked" | "hatch_cross" | "mortar" if v.is_truthy()? => out.set_item(k, true)?,
            "color" if !v.is_none() => out.set_item(k, rounded_list(&v, 4, 3)?)?,
            "folder" | "hatch" if v.is_truthy()? => out.set_item(k, v)?,
            "linetype" if !v.eq("continuous")? => out.set_item(k, v)?,
            "lineweight" | "cut_weight" if !v.is_none() => out.set_item(k, rounded(&v, 3)?)?,
            "hatch_angle" if v.is_truthy()? => out.set_item(k, rounded(&v, 3)?)?,
            "hatch_scale" if (number(&v)? - 1.).abs() > 1e-9 => out.set_item(k, rounded(&v, 4)?)?,
            "hatch_offset" | "brick" if !v.is_none() => {
                out.set_item(k, rounded_list(&v, 2, usize::MAX)?)?
            }
            "hatch_on_paper" | "joints" if !v.is_none() => out.set_item(k, v)?,
            "hatch_ground" | "hatch_color" if !v.is_none() => {
                if v.eq("layer")? {
                    out.set_item(k, v)?;
                } else {
                    out.set_item(k, rounded_list(&v, 4, usize::MAX)?)?;
                }
            }
            "opacity" if number(&v)? < 1. => out.set_item(k, rounded(&v, 3)?)?,
            _ => (),
        }
    }
    Ok(out)
}

#[pyfunction]
pub fn igz_load_material<'py>(
    raw: &Bound<'py, PyDict>,
    class: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let py = raw.py();
    let kwargs = PyDict::new(py);
    kwargs.set_item("name", "")?;
    for key in ["name", "color", "texture", "opacity", "finish", "pbr"] {
        if let Some(v) = raw.get_item(key)? {
            let value = match key {
                "color" if !v.is_none() => py.get_type::<PyTuple>().call1((v,))?,
                "texture" if v.is_truthy()? => py.get_type::<PyDict>().call1((v,))?,
                "texture" | "finish" if !v.is_truthy()? => continue,
                "pbr" => crate::doc::value::Value::from_py(&v)?.to_py(py)?,
                _ => v,
            };
            kwargs.set_item(key, value)?;
        }
    }
    let m = class.call((), Some(&kwargs))?;
    // Format 2 names materials; older files derive a stable uid from the name.
    let uid = match raw.get_item("uid")?.and_then(|v| v.extract::<String>().ok()) {
        Some(u) => u,
        None => crate::element_ids::derived_uid(&format!("built:legacy:v1:material:{}", m.getattr("name")?.str()?)),
    };
    if m.hasattr("uid")? {
        m.setattr("uid", uid)?;
    }
    Ok(m)
}

#[pyfunction]
pub fn igz_archive_material<'py>(mat: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyDict>> {
    let py = mat.py();
    let out = PyDict::new(py);
    out.set_item("name", mat.getattr("name")?)?;
    if mat.hasattr("uid")? {
        out.set_item("uid", crate::element_ids::material_uid(mat)?)?;
    }
    for key in ["color", "texture", "opacity", "finish", "pbr"] {
        let v = mat.getattr(key)?;
        if !v.is_none() {
            let value = match key {
                "color" => py.get_type::<PyList>().call1((v,))?,
                "texture" => py.get_type::<PyDict>().call1((v,))?,
                "opacity" => py.get_type::<PyFloat>().call1((v,))?,
                "pbr" => crate::doc::value::Value::from_py(&v)?.to_py(py)?,
                _ => v,
            };
            out.set_item(key, value)?;
        }
    }
    Ok(out)
}
