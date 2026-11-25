// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::types::{PyDict, PyList};
const FIELDS: &[&str] = &["name", "color", "texture", "opacity", "finish", "pbr"];
fn get<'p>(o: &Obj<'p>, k: &str) -> PyResult<Obj<'p>> {
    o.call_method1("get", (k,))
}
fn builtin<'p>(v: &Obj<'p>, kind: &str) -> PyResult<Obj<'p>> {
    v.py().import("builtins")?.call_method1(kind, (v,))
}
fn dict_or_empty<'p>(v: &Obj<'p>) -> PyResult<Obj<'p>> {
    if v.is_truthy()? {
        builtin(v, "dict")
    } else {
        Ok(PyDict::new(v.py()).into_any())
    }
}
fn deep<'p>(v: &Obj<'p>) -> PyResult<Obj<'p>> {
    v.py().import("copy")?.call_method1("deepcopy", (v,))
}
fn serialize<'p>(this: &Obj<'p>, face: bool) -> PyResult<Obj<'p>> {
    let d = PyDict::new(this.py());
    d.set_item(if face { "mat" } else { "name" }, this.getattr("name")?)?;
    for &k in &FIELDS[1..] {
        let v = this.getattr(k)?;
        if v.is_none() || face && k == "finish" {
            continue;
        }
        d.set_item(
            k,
            match k {
                "color" => builtin(&v, if face { "tuple" } else { "list" })?,
                "texture" => builtin(&v, "dict")?,
                "opacity" => builtin(&v, "float")?,
                "pbr" => deep(&v)?,
                _ => v,
            },
        )?;
    }
    Ok(d.into_any())
}
#[pyfunction]
fn to_dict<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    serialize(this, false)
}
#[pyfunction]
fn face_attrs<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    serialize(this, true)
}
#[pyfunction]
fn from_dict<'p>(cls: &Obj<'p>, raw: &Obj<'p>) -> PyResult<Obj<'p>> {
    let kw = PyDict::new(cls.py());
    kw.set_item("name", raw.call_method1("get", ("name", ""))?)?;
    for &k in &FIELDS[1..] {
        let v = get(raw, k)?;
        if v.is_none() || matches!(k, "texture" | "finish") && !v.is_truthy()? {
            continue;
        }
        kw.set_item(
            k,
            match k {
                "color" => builtin(&v, "tuple")?,
                "texture" => builtin(&v, "dict")?,
                "pbr" => deep(&v)?,
                _ => v,
            },
        )?;
    }
    cls.call((), Some(&kw))
}
#[pyfunction]
fn reduce<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = this.py();
    let dm = py.import("built_core.doc_model")?;
    let state = dm.call_method1("handle_state", (this, PyTuple::new(py, FIELDS)?))?;
    Ok((dm.getattr("new_handle")?, (this.get_type(),), state)
        .into_pyobject(py)?
        .into_any())
}
#[pyfunction]
fn setstate(this: &Obj<'_>, state: &Obj<'_>) -> PyResult<()> {
    this.py()
        .import("built_core.doc_model")?
        .call_method1("set_handle_state", (this, state))?;
    Ok(())
}
#[pyfunction]
fn is_translucent(attrs: &Obj<'_>) -> PyResult<bool> {
    if !attrs.is_truthy()? {
        return Ok(false);
    }
    if builtin(&attrs.call_method1("get", ("opacity", 1.))?, "float")?.lt(0.999)? {
        return Ok(true);
    }
    let tex = get(attrs, "texture")?;
    if tex.is_truthy()? {
        let p = get(&tex, "path")?;
        if p.is_truthy()? {
            return attrs
                .py()
                .import("built_core.texture")?
                .call_method1("image_has_cutout", (p,))?
                .is_truthy();
        }
    }
    Ok(false)
}
#[pyfunction]
fn back_is_default(attrs: &Obj<'_>) -> PyResult<bool> {
    Ok(!(attrs.is_truthy()? && get(attrs, "back")?.is_truthy()? || is_translucent(attrs)?))
}
#[pyfunction]
fn has_own_material(attrs: &Obj<'_>) -> PyResult<bool> {
    if attrs.is_truthy()? {
        for k in ["texture", "pbr", "color"] {
            if !get(attrs, k)?.is_none() {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
#[pyfunction]
fn free_name(base: &str, taken: &Obj<'_>) -> PyResult<String> {
    let mut name = base.to_owned();
    let mut i = 2;
    while taken.contains(&name)? {
        name = format!("{base} ({i})");
        i += 1;
    }
    Ok(name)
}
#[pyfunction(name = "register")]
fn register_material(materials: &Obj<'_>, mat: &Obj<'_>) -> PyResult<String> {
    let original = mat.getattr("name")?;
    let base = if original.is_truthy()? {
        original.extract::<String>()?
    } else {
        "Material".to_owned()
    };
    let mut name = base.clone();
    let mut i = 2;
    while materials.contains(&name)? {
        let other = materials.get_item(&name)?;
        let mut same = true;
        for &k in &FIELDS[1..] {
            if !other.getattr(k)?.eq(mat.getattr(k)?)? {
                same = false;
                break;
            }
        }
        if same {
            return Ok(name);
        }
        name = format!("{base} ({i})");
        i += 1;
    }
    if original.eq(&name)? {
        materials.set_item(&name, mat)?;
    } else {
        let kw = PyDict::new(mat.py());
        kw.set_item("name", &name)?;
        materials.set_item(
            &name,
            mat.py()
                .import("dataclasses")?
                .getattr("replace")?
                .call((mat,), Some(&kw))?,
        )?;
    }
    Ok(name)
}
#[pyfunction]
fn effective_attrs<'p>(attrs: &Obj<'p>, material: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = attrs.py();
    if !material.is_truthy()? {
        return Ok(attrs.clone());
    }
    let fx = get(material, "layer_fx")?;
    let by = get(material, "by_layer")?;
    if !fx.is_none() || !by.is_none() {
        let own = if attrs.is_truthy()? {
            get(attrs, "layer")?
        } else {
            py.None().into_bound(py)
        };
        if !fx.is_none() {
            let merged = effective_attrs(attrs, &get(material, "base")?)?;
            let mut row = if own.is_truthy()? {
                fx.call_method1("get", (own,))?
            } else {
                py.None().into_bound(py)
            };
            if row.is_none() {
                row = get(material, "fx_self")?;
            }
            if !row.is_truthy()? {
                return Ok(merged);
            }
            let d = dict_or_empty(&merged)?;
            let op = row.get_item(0)?.extract::<f64>()?;
            let fade = row.get_item(1)?.extract::<f64>()?;
            if op < 1. {
                d.set_item(
                    "opacity",
                    builtin(&d.call_method1("get", ("opacity", 1.))?, "float")?
                        .extract::<f64>()?
                        .min(op),
                )?;
            }
            if fade > 0. && !get(&d, "texture")?.is_truthy()? {
                let mut rgb = get(&d, "color")?;
                if !rgb.is_truthy()? {
                    rgb = py.import("built_core.layers")?.getattr("DEFAULT_LAYER_COLOR")?;
                }
                let mut out = Vec::new();
                for c in rgb.try_iter()?.take(3) {
                    let c = c?.extract::<f64>()?;
                    out.push(c + (1. - c) * fade);
                }
                d.set_item("color", PyTuple::new(py, out)?)?;
            }
            return Ok(d);
        }
        let mut rgb = py.None().into_bound(py);
        if own.is_truthy()?
            && !py
                .import("built_core")?
                .getattr("tag_styles")?
                .call_method1("is_link_untagged", (&own,))?
                .is_truthy()?
        {
            rgb = by.call_method1("get", (own,))?;
        }
        if rgb.is_none() {
            rgb = get(material, "color")?;
        }
        let d = dict_or_empty(attrs)?;
        for k in ["texture", "mat", "pbr"] {
            d.call_method1("pop", (k, py.None()))?;
        }
        d.set_item(
            "color",
            if rgb.is_none() {
                rgb
            } else {
                builtin(&rgb, "tuple")?
            },
        )?;
        d.set_item("back", true)?;
        return Ok(d);
    }
    if has_own_material(attrs)? {
        return Ok(attrs.clone());
    }
    let d = dict_or_empty(attrs)?;
    for k in ["color", "texture", "opacity", "mat", "pbr"] {
        let v = get(material, k)?;
        if !v.is_none() {
            d.set_item(k, v)?;
        }
    }
    if !get(&d, "back")?.is_truthy()? && !get(material, "front_only")?.is_truthy()? {
        d.set_item("back", true)?;
    }
    Ok(d)
}
#[pyfunction]
fn material_sig<'p>(material: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = material.py();
    if !material.is_truthy()? {
        return Ok(py.None().into_bound(py));
    }
    let fx = get(material, "layer_fx")?;
    if !fx.is_none() {
        return Ok((
            "layer_fx",
            material_sig(&get(material, "base")?)?,
            builtin(&builtin(&fx.call_method0("items")?, "sorted")?, "tuple")?,
            get(material, "fx_self")?,
        )
            .into_pyobject(py)?
            .into_any());
    }
    let c = get(material, "color")?;
    let c = if c.is_none() {
        c
    } else {
        builtin(&c, "tuple")?
    };
    let by = get(material, "by_layer")?;
    if !by.is_none() {
        let rows = PyList::empty(py);
        for row in by.call_method0("items")?.try_iter()? {
            let row = row?;
            rows.append((row.get_item(0)?, builtin(&row.get_item(1)?, "tuple")?))?;
        }
        return Ok((
            "by_layer",
            c,
            builtin(&builtin(rows.as_any(), "sorted")?, "tuple")?,
        )
            .into_pyobject(py)?
            .into_any());
    }
    let tex = get(material, "texture")?;
    let t = if tex.is_instance_of::<PyDict>() {
        Some((
            get(&tex, "path")?,
            get(&tex, "sw")?,
            get(&tex, "sh")?,
            get(&tex, "rot")?,
        ))
    } else {
        None
    };
    let result = (
        c,
        t,
        get(material, "opacity")?,
        get(material, "mat")?,
        get(material, "front_only")?.is_truthy()?,
    )
        .into_pyobject(py)?
        .into_any();
    if material.contains("pbr")? {
        let kw = PyDict::new(py);
        kw.set_item("sort_keys", true)?;
        result.add((py.import("json")?.call_method(
            "dumps",
            (material.get_item("pbr")?,),
            Some(&kw),
        )?,))
    } else {
        Ok(result)
    }
}
pub(super) fn install(m: &Module<'_>, core: &Module<'_>) -> PyResult<()> {
    m.add("Optional", m.py().import("typing")?.getattr("Optional")?)?;
    let py = m.py();
    let ns = PyDict::new(py);
    for &k in &FIELDS[1..] {
        ns.set_item(k, py.None())?;
    }
    for (n, f) in [
        ("to_dict", wrap_pyfunction!(to_dict, m)?),
        ("face_attrs", wrap_pyfunction!(face_attrs, m)?),
        ("__reduce__", wrap_pyfunction!(reduce, m)?),
        ("__setstate__", wrap_pyfunction!(setstate, m)?),
    ] {
        method(&ns, n, f.as_any())?;
    }
    class_method(&ns, "from_dict", wrap_pyfunction!(from_dict, m)?.as_any())?;
    let mat = data_class(
        m,
        "Material",
        FIELDS,
        &ns,
        Some(core.getattr("MaterialData")?),
    )?;
    for &k in &FIELDS[1..] {
        mat.delattr(k)?;
    }
    m.add("MATERIAL_FIELDS", PyTuple::new(py, FIELDS)?)?;
    for f in [
        wrap_pyfunction!(register_material, m)?,
        wrap_pyfunction!(free_name, m)?,
        wrap_pyfunction!(has_own_material, m)?,
        wrap_pyfunction!(effective_attrs, m)?,
        wrap_pyfunction!(material_sig, m)?,
        wrap_pyfunction!(back_is_default, m)?,
        wrap_pyfunction!(is_translucent, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
