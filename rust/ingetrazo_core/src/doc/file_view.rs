// SPDX-License-Identifier: GPL-3.0-or-later
//! Saved camera records: the archive omits defaults and uses "parallel"
//! for the inverse of the document's perspective flag.
use super::value::Value;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyString, PyTuple};

pub(super) fn load<'py>(
    raw: &Bound<'py, PyDict>,
    mut put: impl FnMut(&str, Value) -> PyResult<()>,
) -> PyResult<bool> {
    use Value::{Bool, Dict, Float, List, Str, Tuple};
    let mut values = Vec::new();
    let mut field = |name, val| values.push((name, val));
    field(
        "name",
        match raw.get_item("name")? {
            Some(v) => Value::from_py(&v)?,
            None => Str("Scene".into()),
        },
    );
    let target = match raw.get_item("target")? {
        Some(t) if t.is_truthy()? => {
            if !(t.is_instance_of::<PyList>() || t.is_instance_of::<PyTuple>()) || t.len()? < 3 {
                return Ok(false);
            }
            (0..3)
                .map(|i| Value::from_py(&t.get_item(i)?))
                .collect::<PyResult<_>>()?
        }
        _ => vec![Float(0.0); 3],
    };
    field("target", Tuple(target));
    for (key, attr, default) in [
        ("distance", "distance", 20.),
        ("yaw", "yaw", -std::f64::consts::FRAC_PI_4),
        ("pitch", "pitch", 0.5235987755982988),
        ("fov", "fov_deg", 45.),
    ] {
        let n = match raw.get_item(key)? {
            Some(v) => match v.extract::<f64>() {
                Ok(n) => n,
                Err(_) => return Ok(false),
            },
            None => default,
        };
        field(attr, Float(n));
    }
    for (key, attr, inverted) in [
        ("parallel", "perspective", true),
        ("two_point", "two_point", false),
    ] {
        let on = match raw.get_item(key)? {
            Some(v) => v.is_truthy()?,
            None => false,
        };
        field(attr, Bool(on ^ inverted));
    }
    for name in ["hidden_layers", "hidden_folders", "hidden_objects"] {
        let v = match raw.get_item(name)? {
            Some(v) if !v.is_none() => {
                if !v.is_instance_of::<PyList>() {
                    return Ok(false);
                }
                Value::from_py(&v)?
            }
            _ if name == "hidden_layers" => List(Vec::new()),
            _ => Value::None,
        };
        field(name, v);
    }
    for name in [
        "style",
        "section",
        "georef",
        "shadows",
        "hidden_shown",
        "lens",
    ] {
        let v = match raw.get_item(name)? {
            Some(v) if v.is_truthy()? && v.is_instance_of::<PyDict>() => Value::from_py(&v)?,
            Some(v) if name != "lens" && v.is_truthy()? => return Ok(false),
            _ if name == "lens" => Dict(Vec::new()),
            _ => Value::None,
        };
        field(name, v);
    }
    for name in ["navigator_view", "folder"] {
        let v = match raw.get_item(name)? {
            Some(v)
                if name == "navigator_view"
                    || (v.is_instance_of::<PyString>() && v.is_truthy()?) =>
            {
                Value::from_py(&v)?
            }
            _ => Value::None,
        };
        field(name, v);
    }
    for (name, v) in values {
        put(name, v)?
    }
    Ok(true)
}

pub(super) fn archive<'py>(
    py: Python<'py>,
    mut read: impl FnMut(&str) -> PyResult<Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    out.set_item("name", read("name")?)?;
    let target = read("target")?;
    out.set_item(
        "target",
        PyList::new(
            py,
            (0..3)
                .map(|i| target.get_item(i))
                .collect::<PyResult<Vec<_>>>()?,
        )?,
    )?;
    for name in ["distance", "yaw", "pitch"] {
        out.set_item(name, read(name)?)?;
    }
    let fov = read("fov_deg")?;
    if !fov.eq(45.0)? {
        out.set_item("fov", fov)?;
    }
    if !read("perspective")?.is_truthy()? {
        out.set_item("parallel", true)?;
    }
    if read("two_point")?.is_truthy()? {
        out.set_item("two_point", true)?;
    }
    for (name, omit_empty) in [
        ("lens", true),
        ("hidden_layers", true),
        ("hidden_folders", false),
        ("style", true),
        ("section", false),
        ("georef", false),
        ("shadows", false),
        ("hidden_objects", false),
        ("hidden_shown", false),
        ("navigator_view", false),
        ("folder", true),
    ] {
        let v = read(name)?;
        if v.is_none() || (omit_empty && !v.is_truthy()?) {
            continue;
        }
        let v = match name {
            "hidden_layers" | "hidden_folders" | "hidden_objects" => {
                py.get_type::<PyList>().call1((v,))?
            }
            "navigator_view" | "folder" => v,
            _ => py.get_type::<PyDict>().call1((v,))?,
        };
        out.set_item(name, v)?;
    }
    Ok(out)
}
