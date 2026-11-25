// SPDX-License-Identifier: GPL-3.0-or-later
//! Material registry verbs retain the same History steps as the inspector.
use super::*;
use pyo3::types::{PyFloat, PySlice, PyString};
type Value<'py> = Option<&'py Obj<'py>>;

#[pyfunction]
fn material<'py>(h: &Obj<'py>, name: &Obj<'py>) -> PyResult<Obj<'py>> {
    if let Some(mats) = h.getattr("scene")?.getattr_opt("materials")? {
        if mats.is_truthy()? {
            let m = mats.call_method1("get", (name,))?;
            if !m.is_none() {
                return Ok(m);
            }
        }
    }
    Err(PyKeyError::new_err(format!(
        "no material called {}",
        name.repr()?
    )))
}
fn collision(name: &Obj<'_>) -> PyResult<()> {
    Err(PyValueError::new_err(format!(
        "there is already a material called {}",
        name.repr()?
    )))
}
fn replace<'py>(m: &Obj<'py>, changes: &Bound<'py, PyDict>) -> PyResult<Obj<'py>> {
    m.py()
        .import("dataclasses")?
        .call_method("replace", (m,), Some(changes))
}
fn fields<'py>(m: &Obj<'py>, field: &str, value: &Obj<'py>) -> PyResult<Obj<'py>> {
    let changes = PyDict::new(m.py());
    changes.set_item(field, value)?;
    replace(m, &changes)
}
fn restamp<'py>(h: &Obj<'py>, name: &Obj<'py>, m: &Obj<'py>) -> PyResult<()> {
    h.call_method1(
        "execute",
        (history_command(
            h.py(),
            "RestampMaterialCommand",
            (name, m),
        )?,),
    )?;
    Ok(())
}
fn number(v: &Obj<'_>) -> PyResult<f64> {
    v.py().get_type::<PyFloat>().call1((v,))?.extract()
}
fn rgb<'py>(input: &Obj<'py>) -> PyResult<Obj<'py>> {
    let parts = input.get_item(PySlice::new(input.py(), 0, 3, 1))?;
    let nums = parts
        .try_iter()?
        .map(|v| number(&v?))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(PyTuple::new(input.py(), nums)?.into_any())
}
fn optional_dict<'py>(py: Python<'py>, value: Value<'py>) -> PyResult<Obj<'py>> {
    match value {
        Some(v) if v.is_truthy()? => py.get_type::<PyDict>().call1((v,)),
        _ => Ok(py.None().into_bound(py)),
    }
}

#[pyfunction]
fn rename_material<'py>(history: &Obj<'py>, name: &Obj<'py>, new_name: &Obj<'py>) -> Reply<'py> {
    let m = material(history, name)?;
    let new = new_name.str()?.call_method0("strip")?;
    if !new.is_truthy()? {
        return Err(PyValueError::new_err("a material needs a name"));
    }
    let before = history.getattr("revision")?;
    if new.eq(name)? {
        return answer(history, &before, false);
    }
    if history
        .getattr("scene")?
        .getattr("materials")?
        .contains(&new)?
    {
        collision(&new)?;
    }
    restamp(history, name, &fields(&m, "name", &new)?)?;
    answer(history, &before, true)
}
#[pyfunction]
#[pyo3(signature=(history,name,color=None,opacity=None,tile=None))]
fn edit_material<'py>(
    history: &Obj<'py>,
    name: &Obj<'py>,
    color: Value<'py>,
    opacity: Value<'py>,
    tile: Value<'py>,
) -> Reply<'py> {
    let py = history.py();
    let m = material(history, name)?;
    let tex = m.getattr("texture")?;
    let before = history.getattr("revision")?;
    if tex.is_truthy()? && (color.is_some() || tile.is_some()) && opacity.is_none() {
        let kw = PyDict::new(py);
        let size = if let Some(t) = tile {
            Some((
                number(&t.get_item(0)?)?,
                number(&t.get_item(1)?)?,
                number(&tex.call_method1("get", ("rot", 0.0))?)?,
            ))
        } else {
            None
        };
        kw.set_item("size", size)?;
        let tint = if let Some(c) = color {
            py.get_type::<PyTuple>().call1((c,))?
        } else {
            tex.call_method1("get", ("tint",))?
        };
        let step = py.import("built_core.history")?.call_method(
            "EditTexturedMaterialCommand",
            (name, tint),
            Some(&kw),
        )?;
        history.call_method1("execute", (step,))?;
    } else {
        if tile.is_some() && !tex.is_truthy()? {
            return Err(PyValueError::new_err(
                "only a textured material has a tile size",
            ));
        }
        let changes = PyDict::new(py);
        if let Some(c) = color {
            changes.set_item("color", rgb(c)?)?;
        }
        if let Some(o) = opacity {
            let v = number(o)?;
            changes.set_item("opacity", if v >= 0.999 { None } else { Some(v.max(0.0)) })?;
        }
        if changes.is_empty() {
            return answer(history, &before, false);
        }
        restamp(history, name, &replace(&m, &changes)?)?;
    }
    answer(history, &before, true)
}
#[pyfunction]
#[pyo3(signature=(history,name,new_name=None))]
fn duplicate_material<'py>(
    history: &Obj<'py>,
    name: &Obj<'py>,
    new_name: Value<'py>,
) -> Reply<'py> {
    let py = history.py();
    let m = material(history, name)?;
    let taken = history.getattr("scene")?.getattr("materials")?;
    let new = if let Some(n) = new_name {
        if taken.contains(n)? {
            collision(n)?;
        }
        n.clone()
    } else {
        let base = format!("{name} copy");
        let mut n = base.clone();
        let mut i = 2;
        while taken.contains(n.as_str())? {
            n = format!("{base} {i}");
            i += 1;
        }
        PyString::new(py, &n).into_any()
    };
    let before = history.getattr("revision")?;
    let copy = py.import("copy")?.call_method1("deepcopy", (m,))?;
    let step = py
        .import("core.material_commands")?
        .call_method1("AddMaterialCommand", (fields(&copy, "name", &new)?,))?;
    history.call_method1("execute", (step,))?;
    let out = answer(history, &before, true)?;
    out.set_item("name", new)?;
    Ok(out)
}
#[pyfunction]
fn delete_material<'py>(history: &Obj<'py>, name: &Obj<'py>) -> Reply<'py> {
    material(history, name)?;
    execute(
        history,
        &history_command(history.py(), "DeleteMaterialCommand", (name,))?,
    )
}
#[pyfunction]
#[pyo3(signature=(history,name=None,color=None,texture=None,opacity=None,pbr=None,finish=None,reuse=true))]
#[allow(clippy::too_many_arguments)]
fn create_material<'py>(
    history: &Obj<'py>,
    name: Value<'py>,
    color: Value<'py>,
    texture: Value<'py>,
    opacity: Value<'py>,
    pbr: Value<'py>,
    finish: Value<'py>,
    #[pyo3(from_py_with=PyAnyMethods::is_truthy)] reuse: bool,
) -> Reply<'py> {
    let py = history.py();
    if color.is_none() == texture.is_none() {
        return Err(PyValueError::new_err(
            "a material has one base: a colour or an image",
        ));
    }
    let api = py.import("core.material_commands")?;
    let scene = history.getattr("scene")?;
    let mats = scene.getattr("materials")?;
    let kw = PyDict::new(py);
    kw.set_item("color", color.map(rgb).transpose()?)?;
    kw.set_item("texture", texture)?;
    kw.set_item("opacity", opacity)?;
    let before = history.getattr("revision")?;
    if reuse && pbr.is_none() {
        let found = api.call_method("find_material", (&mats,), Some(&kw))?;
        if !found.is_none() && (name.is_none() || found.eq(name.unwrap())?) {
            let out = PyDict::new(py);
            out.set_item("ok", true)?;
            for k in ["step", "error", "rejected"] {
                out.set_item(k, py.None())?;
            }
            out.set_item("name", found)?;
            out.set_item("reused", true)?;
            return Ok(out);
        }
    }
    let name = if let Some(n) = name {
        if mats.contains(n)? {
            collision(n)?;
        }
        n.clone()
    } else {
        let hint = api
            .call_method("material_for_paint", (scene,), Some(&kw))?
            .getattr("name")?;
        api.call_method1("fresh_name", (mats, hint))?
    };
    kw.set_item("texture", optional_dict(py, texture)?)?;
    kw.set_item(
        "opacity",
        opacity.map(number).transpose()?.filter(|v| !(*v >= 0.999)),
    )?;
    kw.set_item("pbr", optional_dict(py, pbr)?)?;
    kw.set_item("finish", finish)?;
    let m = py
        .import("built_core.materials")?
        .call_method("Material", (&name,), Some(&kw))?;
    history.call_method1("execute", (api.call_method1("AddMaterialCommand", (m,))?,))?;
    let out = answer(history, &before, true)?;
    out.set_item("name", name)?;
    out.set_item("reused", false)?;
    Ok(out)
}

#[pyfunction]
fn name_unnamed_colours<'py>(history: &Obj<'py>) -> Reply<'py> {
    let cmd = history
        .py()
        .import("core.material_commands")?
        .call_method0("NameColoursCommand")?;
    let before = history.getattr("revision")?;
    let n = cmd.call_method1("count", (history.getattr("scene")?,))?;
    let ran = n.is_truthy()?;
    if ran {
        history.call_method1("execute", (cmd,))?;
    }
    let out = answer(history, &before, ran)?;
    out.set_item("named", n)?;
    Ok(out)
}
#[pyfunction]
#[pyo3(signature=(history,name,base,color=None,texture=None))]
fn set_material_base<'py>(
    history: &Obj<'py>,
    name: &Obj<'py>,
    base: &str,
    color: Value<'py>,
    texture: Value<'py>,
) -> Reply<'py> {
    let py = history.py();
    let m = material(history, name)?;
    let before = history.getattr("revision")?;
    let changes = PyDict::new(py);
    match base {
        "color" => {
            let current = m.getattr("color")?;
            let value = if let Some(c) = color {
                rgb(c)?
            } else if current.is_truthy()? {
                rgb(&current)?
            } else {
                (0.8, 0.8, 0.8).into_pyobject(py)?.into_any()
            };
            changes.set_item("color", value)?;
            changes.set_item("texture", py.None())?;
        }
        "image" => {
            let current = m.getattr("texture")?;
            let source = match texture {
                Some(v) if v.is_truthy()? => v,
                _ => &current,
            };
            let tex = optional_dict(py, Some(source))?;
            if tex.is_none() || !tex.call_method1("get", ("path",))?.is_truthy()? {
                return Err(PyValueError::new_err("an image base needs an image"));
            }
            let sw = tex.call_method1("setdefault", ("sw", 1.0))?;
            tex.call_method1("setdefault", ("sh", sw))?;
            changes.set_item("texture", tex)?;
            changes.set_item(
                "color",
                if m.getattr("pbr")?.is_none() {
                    py.None().into_bound(py)
                } else {
                    m.getattr("color")?
                },
            )?;
        }
        _ => return Err(PyValueError::new_err("base is 'color' or 'image'")),
    }
    let new = replace(&m, &changes)?;
    if new.eq(&m)? {
        return answer(history, &before, false);
    }
    restamp(history, name, &new)?;
    answer(history, &before, true)
}
#[pyfunction]
#[pyo3(signature=(history,name,**channels))]
fn set_material_pbr<'py>(
    history: &Obj<'py>,
    name: &Obj<'py>,
    channels: Option<&Bound<'py, PyDict>>,
) -> Reply<'py> {
    let py = history.py();
    let m = material(history, name)?;
    let api = py.import("core.material_commands")?;
    let keys = api.getattr("PBR_KEYS")?;
    let maps = api.getattr("PBR_MAPS")?;
    let mut unknown = Vec::new();
    let current = m.getattr("pbr")?;
    let pbr = if current.is_truthy()? {
        py.get_type::<PyDict>()
            .call1((current,))?
            .cast_into::<PyDict>()?
    } else {
        PyDict::new(py)
    };
    if let Some(channels) = channels {
        for key in channels.keys() {
            if !keys.contains(&key)? {
                unknown.push(key.extract::<String>()?);
            }
        }
        if !unknown.is_empty() {
            unknown.sort();
            let allowed = py.get_type::<PyList>().call1((keys,))?;
            return Err(PyValueError::new_err(format!(
                "unknown PBR channel {}; one of {allowed}",
                PyString::new(py, &unknown[0]).repr()?
            )));
        }
        for (key, value) in channels.iter() {
            if value.is_none() {
                pbr.call_method1("pop", (key, py.None()))?;
            } else if maps.contains(&key)? {
                let map = if value.is_instance_of::<PyString>() {
                    let t = PyDict::new(py);
                    t.set_item("path", value)?;
                    t.into_any()
                } else {
                    py.get_type::<PyDict>().call1((value,))?
                };
                pbr.set_item(key, map)?;
            } else {
                let v = number(&value)?;
                let limit = if key.eq("normal_scale")? {
                    f64::INFINITY
                } else {
                    1.0
                };
                pbr.set_item(key, v.clamp(0.0, limit))?;
            }
        }
    }
    let before = history.getattr("revision")?;
    let value = if pbr.is_empty() {
        py.None().into_bound(py)
    } else {
        pbr.into_any()
    };
    let new = fields(&m, "pbr", &value)?;
    if new.eq(&m)? {
        return answer(history, &before, false);
    }
    restamp(history, name, &new)?;
    answer(history, &before, true)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(material,m)?)?;
    m.add_function(wrap_pyfunction!(set_material_pbr, m)?)?;
    m.add_function(wrap_pyfunction!(set_material_base, m)?)?;
    m.add_function(wrap_pyfunction!(rename_material, m)?)?;
    m.add_function(wrap_pyfunction!(edit_material, m)?)?;
    m.add_function(wrap_pyfunction!(duplicate_material, m)?)?;
    m.add_function(wrap_pyfunction!(delete_material, m)?)?;
    m.add_function(wrap_pyfunction!(create_material, m)?)?;
    m.add_function(wrap_pyfunction!(name_unnamed_colours, m)?)?;
    Ok(())
}
