// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::{
    exceptions::PyValueError,
    types::{PyDict, PyList, PySet, PySlice},
};
const FIELDS: &[&str] = &[
    "name",
    "face_mode",
    "edges",
    "profiles",
    "back_edges",
    "edge_color",
    "front_color",
    "back_color",
    "background",
    "sky",
    "sky_color",
    "ground_color",
    "section_fill",
    "section_fill_color",
    "ambient_occlusion",
    "ao_distance",
    "ao_intensity",
    "color_by_layer",
    "model_linetypes",
];
const MODES: &[&str] = &[
    "textures",
    "shaded",
    "hidden_line",
    "monochrome",
    "wireframe",
    "xray",
];
fn module(py: Python<'_>) -> PyResult<Module<'_>> {
    py.import("built_core.style")
}
fn color(n: &str) -> bool {
    n.ends_with("_color") || n == "background"
}
#[pyfunction]
fn to_dict<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    let d = PyDict::new(this.py());
    for &k in FIELDS {
        let mut v = this.getattr(k)?;
        if k == "color_by_layer" && !v.is_truthy()? || k == "model_linetypes" && v.is_truthy()? {
            continue;
        }
        if color(k) && !v.is_none() {
            v = this.py().get_type::<PyList>().call1((v,))?;
        }
        d.set_item(k, v)?;
    }
    Ok(d.into_any())
}
#[pyfunction]
fn _clamped(raw: &Obj<'_>, default: f64, bounds: (f64, f64)) -> PyResult<f64> {
    let v = raw
        .py()
        .import("builtins")?
        .call_method1("float", (raw,))
        .and_then(|v| v.extract::<f64>());
    Ok(match v {
        Ok(v) if !v.is_nan() => v.max(bounds.0).min(bounds.1),
        _ => default,
    })
}
#[pyfunction]
fn _opt_rgb(raw: &Obj<'_>) -> PyResult<Option<(f64, f64, f64)>> {
    let py = raw.py();
    let float = py.import("builtins")?.getattr("float")?;
    let parse = || -> PyResult<Vec<f64>> {
        raw.try_iter()?
            .map(|v| float.call1((v?,))?.extract())
            .collect()
    };
    Ok(match parse() {
        Ok(v) if v.len() >= 3 => Some((v[0], v[1], v[2])),
        _ => None,
    })
}
#[pyfunction]
fn from_dict<'p>(cls: &Obj<'p>, raw: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = cls.py();
    let defaults = cls.call0()?;
    let kw = PyDict::new(py);
    for &k in FIELDS {
        let d = defaults.getattr(k)?;
        let v = raw.call_method1("get", (k, &d))?;
        match k {
            "name" => kw.set_item(k, v)?,
            "face_mode" => kw.set_item(
                k,
                if MODES.iter().any(|s| v.eq(*s).unwrap_or(false)) {
                    v
                } else {
                    d
                },
            )?,
            "back_color" => kw.set_item(k, _opt_rgb(&v)?)?,
            "ao_distance" | "ao_intensity" => kw.set_item(
                k,
                _clamped(
                    &v,
                    d.extract()?,
                    if k == "ao_distance" {
                        (0.01, 50.)
                    } else {
                        (0., 2.)
                    },
                )?,
            )?,
            "model_linetypes" => kw.set_item(k, !v.is(pyo3::types::PyBool::new(py, false)))?,
            k if color(k) => kw.set_item(k, py.get_type::<PyTuple>().call1((v,))?)?,
            _ => kw.set_item(k, v.is_truthy()?)?,
        }
    }
    cls.call((), Some(&kw))
}
#[pyfunction]
fn copy<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    module(this.py())?
        .getattr("Style")?
        .call_method1("from_dict", (this.call_method0("to_dict")?,))
}
#[pyfunction(signature=(style,scene=None))]
fn effective_back_color<'p>(style: &Obj<'p>, scene: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    let py = style.py();
    let picked = style.getattr_opt("back_color")?.filter(|v| !v.is_none());
    let adopted = if picked.is_none() {
        scene
            .map(|s| s.getattr_opt("back_face_color"))
            .transpose()?
            .flatten()
            .filter(|v| !v.is_none())
    } else {
        None
    };
    if let Some(v) = picked.or(adopted) {
        py.get_type::<PyTuple>()
            .call1((v.get_item(PySlice::new(py, 0, 3, 1))?,))
    } else {
        module(py)?.getattr("DEFAULT_BACK_COLOR")
    }
}
#[pyfunction(pass_module)]
fn builtin_names<'p>(m: &Module<'p>) -> PyResult<Obj<'p>> {
    let out = PySet::empty(m.py())?;
    for p in m.getattr("BUILTIN_STYLES")?.try_iter()? {
        out.add(p?.getattr("name")?)?;
    }
    Ok(out.into_any())
}
#[pyfunction]
fn _settings(py: Python<'_>) -> PyResult<Obj<'_>> {
    py.import("built_core.settings")?
        .call_method0("settings")
}
#[pyfunction(pass_module)]
fn user_styles<'p>(m: &Module<'p>) -> PyResult<Obj<'p>> {
    let py = m.py();
    let kw = PyDict::new(py);
    kw.set_item("type", py.import("builtins")?.getattr("str")?)?;
    let raw = m
        .call_method0("_settings")?
        .call_method("value", ("styles/user", ""), Some(&kw))?;
    let out = PyList::empty(py);
    if raw.is_truthy()? {
        match py.import("json")?.call_method1("loads", (raw,)) {
            Ok(rows) => {
                if let Ok(rows) = rows.cast::<PyList>() {
                    for row in rows {
                        if row.is_instance_of::<PyDict>()
                            && row.call_method1("get", ("name",))?.is_truthy()?
                        {
                            out.append(m.getattr("Style")?.call_method1("from_dict", (row,))?)?;
                        }
                    }
                }
            }
            Err(e) if e.is_instance_of::<PyValueError>(py) => (),
            Err(e) => return Err(e),
        }
    }
    Ok(out.into_any())
}
#[pyfunction(pass_module)]
fn style_by_name<'p>(m: &Module<'p>, name: &str) -> PyResult<Option<Obj<'p>>> {
    for p in m.getattr("BUILTIN_STYLES")?.try_iter()? {
        let p = p?;
        if p.getattr("name")?.eq(name)? {
            return Ok(Some(p.call_method0("copy")?));
        }
    }
    for p in m.call_method0("user_styles")?.try_iter()? {
        let p = p?;
        if p.getattr("name")?.eq(name)? {
            return Ok(Some(p));
        }
    }
    Ok(None)
}
#[pyfunction(pass_module)]
fn _store_user_styles(m: &Module<'_>, styles: &Obj<'_>) -> PyResult<()> {
    let store = m.call_method0("_settings")?;
    let rows = styles
        .try_iter()?
        .map(|s| s?.call_method0("to_dict"))
        .collect::<PyResult<Vec<_>>>()?;
    store.call_method1(
        "setValue",
        (
            "styles/user",
            m.py().import("json")?.call_method1("dumps", (rows,))?,
        ),
    )?;
    store.call_method0("sync")?;
    Ok(())
}
fn kept<'p>(m: &Module<'p>, name: &Obj<'p>) -> PyResult<(Bound<'p, PyList>, bool)> {
    let out = PyList::empty(m.py());
    let mut changed = false;
    for s in m.call_method0("user_styles")?.try_iter()? {
        let s = s?;
        if s.getattr("name")?.eq(name)? {
            changed = true
        } else {
            out.append(s)?;
        }
    }
    Ok((out, changed))
}
#[pyfunction(pass_module)]
fn save_user_style(m: &Module<'_>, style: &Obj<'_>) -> PyResult<()> {
    let name = style.getattr("name")?;
    if builtin_names(m)?.contains(&name)? {
        return Err(PyValueError::new_err(format!(
            "'{name}' is a built-in style"
        )));
    }
    let (out, _) = kept(m, &name)?;
    out.append(style.call_method0("copy")?)?;
    m.call_method1("_store_user_styles", (out,))?;
    Ok(())
}
#[pyfunction(pass_module)]
fn delete_user_style(m: &Module<'_>, name: &Obj<'_>) -> PyResult<bool> {
    let (out, removed) = kept(m, name)?;
    if removed {
        m.call_method1("_store_user_styles", (out,))?;
    }
    Ok(removed)
}
pub(super) fn install(m: &Module<'_>) -> PyResult<()> {
    let py = m.py();
    m.add("FACE_MODES", PyTuple::new(py, MODES)?)?;
    m.add("DEFAULT_FACE_COLOR", (0.96, 0.95, 0.925))?;
    m.add("DEFAULT_BACK_COLOR", (0.62, 0.70, 0.78))?;
    m.add("MONOCHROME_GLASS_RGB", (0.5, 0.5, 0.5))?;
    m.add("AO_DISTANCE_RANGE", (0.01, 50.))?;
    m.add("AO_INTENSITY_RANGE", (0., 2.))?;
    let ns = PyDict::new(py);
    ns.set_item("name", "Default")?;
    ns.set_item("face_mode", "textures")?;
    for n in [
        "edges",
        "profiles",
        "sky",
        "section_fill",
        "model_linetypes",
    ] {
        ns.set_item(n, true)?;
    }
    for n in ["back_edges", "ambient_occlusion", "color_by_layer"] {
        ns.set_item(n, false)?;
    }
    ns.set_item("back_color", py.None())?;
    for (n, rgb) in [
        ("edge_color", (0.13, 0.17, 0.23)),
        ("front_color", (1., 1., 1.)),
        ("background", (0.90, 0.91, 0.92)),
        ("sky_color", (0.925, 0.935, 0.945)),
        ("ground_color", (0.815, 0.820, 0.815)),
        ("section_fill_color", (0.35, 0.37, 0.41)),
    ] {
        ns.set_item(n, rgb)?;
    }
    ns.set_item("ao_distance", 0.5)?;
    ns.set_item("ao_intensity", 1.)?;
    method(&ns, "to_dict", wrap_pyfunction!(to_dict, m)?.as_any())?;
    class_method(&ns, "from_dict", wrap_pyfunction!(from_dict, m)?.as_any())?;
    method(&ns, "copy", wrap_pyfunction!(copy, m)?.as_any())?;
    let cls = data_class(m, "Style", FIELDS, &ns, None)?;
    let presets = PyList::empty(py);
    for (n, mode, white) in [
        ("Default", "textures", false),
        ("Architectural", "textures", true),
        ("Shaded", "shaded", false),
        ("Hidden line", "hidden_line", true),
        ("Monochrome", "monochrome", true),
        ("Wireframe", "wireframe", true),
        ("X-ray", "xray", false),
    ] {
        let kw = PyDict::new(py);
        kw.set_item("name", n)?;
        kw.set_item("face_mode", mode)?;
        if white {
            kw.set_item("background", (1., 1., 1.))?;
            kw.set_item("sky", false)?;
        }
        presets.append(cls.call((), Some(&kw))?)?;
    }
    m.add("BUILTIN_STYLES", presets)?;
    for f in [
        wrap_pyfunction!(builtin_names, m)?,
        wrap_pyfunction!(effective_back_color, m)?,
        wrap_pyfunction!(style_by_name, m)?,
        wrap_pyfunction!(_settings, m)?,
        wrap_pyfunction!(user_styles, m)?,
        wrap_pyfunction!(_store_user_styles, m)?,
        wrap_pyfunction!(save_user_style, m)?,
        wrap_pyfunction!(delete_user_style, m)?,
        wrap_pyfunction!(_clamped, m)?,
        wrap_pyfunction!(_opt_rgb, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
