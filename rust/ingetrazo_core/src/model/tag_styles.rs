// SPDX-License-Identifier: GPL-3.0-or-later
//! A tag's styles (`core.layers`): its colour, its hatch's ground and ink,
//! the paint and look it gives its objects, the line styles a sheet and the
//! model ink with, a sheet legend's rows, the hatched cut's patterns,
//! grounds, inks and forms, the line types' own colours, and the tags left
//! out of the sheets.
//!
//! Read from the tags and the scene as the Python read them, in its order;
//! the hatch catalogue (`core.hatches`) and the line type tables
//! (`core.layers.LINETYPES`, the dash helpers) are called back.
//! `tests/test_tag_styles_parity.py`.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyString, PyTuple};

use super::tag_edits::{effective_visible, path_label, DEFAULT_LAYER};

type Obj<'py> = Bound<'py, PyAny>;

const DEFAULT_LAYER_COLOR: (f64, f64, f64) = (0.96, 0.95, 0.925);
const GROUND_LAYER: &str = "layer";
const LOCKED_FADE: f64 = 0.55;
const CUSTOM_LINETYPE: &str = "lin:";

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn opt<'py>(o: &Obj<'py>, name: &str) -> PyResult<Obj<'py>> {
    Ok(o.getattr_opt(name)?.unwrap_or_else(|| none(o.py())))
}

fn layers<'py>(scene: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    match scene.getattr_opt("layers")? {
        Some(l) if !l.is_none() => l.try_iter()?.collect(),
        _ => Ok(Vec::new()),
    }
}

/// `getattr(o, name, None) or {}`.
fn table<'py>(o: &Obj<'py>, name: &str) -> PyResult<Obj<'py>> {
    let v = opt(o, name)?;
    Ok(if v.is_truthy()? { v } else { PyDict::new(o.py()).into_any() })
}

fn hatches(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("core.hatches")
}

fn layers_module(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("core.layers")
}

fn custom_name(key: &Obj<'_>) -> Option<String> {
    key.cast::<PyString>().ok()?.to_str().ok()?.strip_prefix(CUSTOM_LINETYPE).map(str::to_string)
}

fn tuple_of<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    Ok(v.py().get_type::<PyTuple>().call1((v,))?)
}

fn round<'py>(py: Python<'py>, v: &Obj<'py>, n: i32) -> PyResult<Obj<'py>> {
    py.import("builtins")?.getattr("round")?.call1((v, n))
}

fn rounded_list<'py>(py: Python<'py>, rgb: &Obj<'py>, n: i32) -> PyResult<Bound<'py, PyList>> {
    let l = PyList::empty(py);
    for c in rgb.try_iter()? {
        l.append(round(py, &c?, n)?)?;
    }
    Ok(l)
}

fn floats(v: &Obj<'_>) -> PyResult<Vec<f64>> {
    v.try_iter()?.map(|c| c?.extract::<f64>()).collect()
}

// ---- Colours -----------------------------------------------------------------

/// `layer_color(layer)`: the picked colour, the default tag's white, or the
/// automatic one by key.
#[pyfunction]
pub fn layer_color<'py>(py: Python<'py>, layer: &Obj<'py>) -> PyResult<Obj<'py>> {
    if layer.is_none() {
        return Ok(DEFAULT_LAYER_COLOR.into_pyobject(py)?.into_any());
    }
    let color = layer.getattr("color")?;
    if !color.is_none() {
        return tuple_of(&color.get_item(pyo3::types::PySlice::new(py, 0, 3, 1))?);
    }
    let key = layer.getattr("key")?;
    if key.eq(DEFAULT_LAYER)? {
        return Ok(DEFAULT_LAYER_COLOR.into_pyobject(py)?.into_any());
    }
    py.import("ingetrazo_core")?.call_method1("automatic_tag_color", (key,))
}

#[pyfunction]
pub fn light_layer_color<'py>(py: Python<'py>, ly: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    let c = floats(&layer_color(py, ly)?)?;
    PyTuple::new(py, c.iter().map(|&c| c + (1.0 - c) * 0.75))
}

#[pyfunction]
pub fn hatch_ground_rgb<'py>(py: Python<'py>, ly: &Obj<'py>) -> PyResult<Obj<'py>> {
    let ground = opt(ly, "hatch_ground")?;
    if ground.eq(GROUND_LAYER)? {
        return Ok(light_layer_color(py, ly)?.into_any());
    }
    if ground.is_none() { Ok(ground) } else { tuple_of(&ground) }
}

#[pyfunction]
pub fn hatch_ink_rgb<'py>(py: Python<'py>, ly: &Obj<'py>) -> PyResult<Obj<'py>> {
    let ink = opt(ly, "hatch_color")?;
    if ink.eq(GROUND_LAYER)? {
        let c = floats(&layer_color(py, ly)?)?;
        return Ok(PyTuple::new(py, c.iter().map(|&c| c * 0.7))?.into_any());
    }
    if ink.is_none() { Ok(ink) } else { tuple_of(&ink) }
}

#[pyfunction]
pub fn is_link_untagged(name: &Obj<'_>) -> PyResult<bool> {
    Ok(name.is_truthy()? && name.call_method1("endswith", (" / Untagged",))?.is_truthy()?)
}

/// `object_layer(group)`: its own tag, else its owner's, else the default.
#[pyfunction]
pub fn object_layer<'py>(py: Python<'py>, group: &Obj<'py>) -> PyResult<Obj<'py>> {
    let owner = opt(group, "owner")?;
    for g in [group, &owner] {
        let name = if g.is_none() { none(py) } else { opt(g, "layer")? };
        if name.is_truthy()? && !is_link_untagged(&name)? {
            return Ok(name);
        }
    }
    Ok(PyString::new(py, DEFAULT_LAYER).into_any())
}

#[pyfunction]
#[pyo3(signature = (scene, group=None))]
pub fn layer_paint<'py>(py: Python<'py>, scene: &Obj<'py>, group: Option<&Obj<'py>>) -> PyResult<Bound<'py, PyDict>> {
    let by = PyDict::new(py);
    for ly in layers(scene)? {
        by.set_item(ly.getattr("key")?, layer_color(py, &ly)?)?;
    }
    let group = group.cloned().unwrap_or_else(|| none(py));
    let color = by.call_method1("get", (object_layer(py, &group)?, DEFAULT_LAYER_COLOR))?;
    let out = PyDict::new(py);
    out.set_item("by_layer", by)?;
    out.set_item("color", color)?;
    Ok(out)
}

#[pyfunction]
pub fn layer_fx<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    for ly in layers(scene)? {
        let opacity = ly.getattr("opacity")?;
        let locked = ly.getattr("locked")?.is_truthy()?;
        if opacity.lt(1.0)? || locked {
            out.set_item(ly.getattr("key")?, (opacity, if locked { LOCKED_FADE } else { 0.0 }))?;
        }
    }
    Ok(out)
}

#[pyfunction]
#[pyo3(signature = (scene, group, base, fx=None))]
pub fn layer_look<'py>(py: Python<'py>, scene: &Obj<'py>, group: &Obj<'py>, base: &Obj<'py>,
                       fx: Option<&Obj<'py>>) -> PyResult<Obj<'py>> {
    let fx = match fx.filter(|f| !f.is_none()) {
        Some(f) => f.clone(),
        None => layer_fx(py, scene)?.into_any(),
    };
    if !fx.is_truthy()? {
        return Ok(base.clone());
    }
    let out = PyDict::new(py);
    out.set_item("layer_fx", &fx)?;
    out.set_item("fx_self", fx.call_method1("get", (object_layer(py, group)?,))?)?;
    out.set_item("base", base)?;
    Ok(out.into_any())
}

// ---- Line types --------------------------------------------------------------

/// `clean_line_colour(raw)`: `(r, g, b)` kept to 0–1, or `None`.
#[pyfunction]
pub fn clean_line_colour<'py>(py: Python<'py>, raw: &Obj<'py>) -> PyResult<Obj<'py>> {
    let ok = (raw.is_instance_of::<PyList>() || raw.is_instance_of::<PyTuple>()) && raw.len()? == 3;
    if !ok {
        return Ok(none(py));
    }
    let float = py.import("builtins")?.getattr("float")?;
    let mut rgb = Vec::with_capacity(3);
    for c in raw.try_iter()? {
        let Ok(v) = float.call1((c?,)).and_then(|v| v.extract::<f64>()) else { return Ok(none(py)) };
        // `min(1.0, max(0.0, v))` as Python's: a NaN is never greater.
        let v = if v > 0.0 { v } else { 0.0 };
        rgb.push(if v < 1.0 { v } else { 1.0 });
    }
    Ok(PyTuple::new(py, rgb)?.into_any())
}

fn linetypes<'py>(py: Python<'py>) -> PyResult<Obj<'py>> {
    layers_module(py)?.getattr("LINETYPES")
}

fn custom_entry<'py>(scene: &Obj<'py>, name: &str) -> PyResult<Obj<'py>> {
    let scene = if scene.is_none() { none(scene.py()) } else { scene.clone() };
    table(&scene, "line_types")?.call_method1("get", (name,))
}

/// `linetype_pattern(key, scene=None)`: the dashes as drawn, paper mm.
#[pyfunction]
#[pyo3(signature = (key, scene=None))]
pub fn linetype_pattern<'py>(py: Python<'py>, key: &Obj<'py>, scene: Option<&Obj<'py>>) -> PyResult<Obj<'py>> {
    let table = linetypes(py)?;
    if table.contains(key).unwrap_or(false) {
        return tuple_of(&table.get_item(key)?);
    }
    if let Some(name) = custom_name(key) {
        let entry = custom_entry(&scene.cloned().unwrap_or_else(|| none(py)), &name)?;
        if entry.is_instance_of::<PyDict>() {
            let pattern = entry.call_method1("get", ("pattern",))?;
            let pattern = if pattern.is_truthy()? { pattern } else { PyTuple::empty(py).into_any() };
            return layers_module(py)?.call_method1("drawn_dashes", (pattern,));
        }
    }
    Ok(PyTuple::empty(py).into_any())
}

/// `linetype_text(key, scene=None)`: `(text, height, middle, period)` or
/// `None`.
#[pyfunction]
#[pyo3(signature = (key, scene=None))]
pub fn linetype_text<'py>(py: Python<'py>, key: &Obj<'py>, scene: Option<&Obj<'py>>) -> PyResult<Obj<'py>> {
    let Some(name) = custom_name(key) else { return Ok(none(py)) };
    let module = layers_module(py)?;
    let entry = custom_entry(&scene.cloned().unwrap_or_else(|| none(py)), &name)?;
    let pattern = if entry.is_instance_of::<PyDict>() { entry.call_method1("get", ("pattern",))? } else { none(py) };
    let spec = module.call_method1("clean_line_text", (&entry, &pattern))?;
    if spec.is_none() {
        return Ok(spec);
    }
    let period = py.import("builtins")?.call_method1("sum", (module.call_method1("drawn_dashes", (&pattern,))?,))?;
    let at = module.call_method1("text_offset", (&pattern, spec.get_item("text_at")?))?;
    Ok(PyTuple::new(py, [spec.get_item("text")?, spec.get_item("text_size")?, at, period])?.into_any())
}

/// `linetype_color(key, scene=None)`: the line type's own colour, or `None`.
#[pyfunction]
#[pyo3(signature = (key, scene=None))]
pub fn linetype_color<'py>(py: Python<'py>, key: &Obj<'py>, scene: Option<&Obj<'py>>) -> PyResult<Obj<'py>> {
    let scene = scene.cloned().unwrap_or_else(|| none(py));
    if linetypes(py)?.contains(key).unwrap_or(false) {
        let colours = if scene.is_none() { PyDict::new(py).into_any() } else { table(&scene, "linetype_colors")? };
        return clean_line_colour(py, &colours.call_method1("get", (key,))?);
    }
    let Some(name) = custom_name(key) else { return Ok(none(py)) };
    let entry = custom_entry(&scene, &name)?;
    if entry.is_instance_of::<PyDict>() {
        clean_line_colour(py, &entry.call_method1("get", ("color",))?)
    } else {
        Ok(none(py))
    }
}

/// `set_linetype_color(scene, key, rgb)`: whether anything changed.
#[pyfunction]
pub fn set_linetype_color(py: Python<'_>, scene: &Obj<'_>, key: &Obj<'_>, rgb: &Obj<'_>) -> PyResult<bool> {
    let rgb = clean_line_colour(py, rgb)?;
    if linetype_color(py, key, Some(scene))?.eq(&rgb)? {
        return Ok(false);
    }
    let dict = py.get_type::<PyDict>();
    if linetypes(py)?.contains(key).unwrap_or(false) {
        let colours = dict.call1((table(scene, "linetype_colors")?,))?;
        if rgb.is_none() {
            colours.call_method1("pop", (key, none(py)))?;
        } else {
            colours.set_item(key, &rgb)?;
        }
        scene.setattr("linetype_colors", colours)?;
        return Ok(true);
    }
    let Some(name) = custom_name(key) else { return Ok(false) };
    let own = dict.call1((table(scene, "line_types")?,))?;
    let entry = own.call_method1("get", (&name,))?;
    if !entry.is_instance_of::<PyDict>() {
        return Ok(false);
    }
    let entry = dict.call1((entry,))?;
    if rgb.is_none() {
        entry.call_method1("pop", ("color", none(py)))?;
    } else {
        entry.set_item("color", &rgb)?;
    }
    own.set_item(&name, entry)?;
    scene.setattr("line_types", own)?;
    Ok(true)
}

#[pyfunction]
pub fn load_linetype_colors(py: Python<'_>, scene: &Obj<'_>, raw: &Obj<'_>) -> PyResult<()> {
    if !raw.is_instance_of::<PyDict>() {
        return Ok(());
    }
    let colours = py.get_type::<PyDict>().call1((table(scene, "linetype_colors")?,))?;
    let known = linetypes(py)?;
    for item in raw.call_method0("items")?.try_iter()? {
        let (key, value): (Obj, Obj) = item?.extract()?;
        let rgb = clean_line_colour(py, &value)?;
        if known.contains(&key).unwrap_or(false) && !rgb.is_none() {
            colours.set_item(key, rgb)?;
        }
    }
    scene.setattr("linetype_colors", colours)
}

/// `model_line_styles(scene)`: `{key: (rgb or None, pattern)}` of the tags
/// the model draws with their line type.
#[pyfunction]
pub fn model_line_styles<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    for ly in layers(scene)? {
        let lt = ly.getattr("linetype")?;
        let rgb = linetype_color(py, &lt, Some(scene))?;
        let pattern = tuple_of(&linetype_pattern(py, &lt, Some(scene))?)?;
        if !rgb.is_none() || pattern.is_truthy()? {
            out.set_item(ly.getattr("key")?, (rgb, pattern))?;
        }
    }
    Ok(out)
}

#[pyfunction]
#[pyo3(signature = (scene, style=None))]
pub fn layer_lines_wanted(py: Python<'_>, scene: &Obj<'_>, style: Option<&Obj<'_>>) -> PyResult<bool> {
    let style = match style.filter(|s| !s.is_none()) {
        Some(s) => s.clone(),
        None => opt(scene, "display_style")?,
    };
    if !style.is_none() && opt(&style, "color_by_layer")?.is_truthy()? {
        return Ok(true);
    }
    let _ = py;
    for ly in layers(scene)? {
        if !ly.getattr("linetype")?.eq("continuous")? || !ly.getattr("lineweight")?.is_none()
            || !opt(&ly, "cut_weight")?.is_none()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `line_styles(scene, colored)`: how each tag's lines are inked on a sheet.
#[pyfunction]
pub fn line_styles<'py>(py: Python<'py>, scene: &Obj<'py>, colored: bool) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    for ly in layers(scene)? {
        let lt = ly.getattr("linetype")?;
        let own = linetype_color(py, &lt, Some(scene))?;
        let rgb = if own.is_truthy()? {
            own
        } else if colored && !ly.getattr("key")?.eq(DEFAULT_LAYER)? {
            layer_color(py, &ly)?
        } else {
            none(py)
        };
        out.set_item(ly.getattr("key")?, PyTuple::new(py, [
            rgb, lt.clone(), ly.getattr("lineweight")?, opt(&ly, "cut_weight")?,
            linetype_pattern(py, &lt, Some(scene))?, linetype_text(py, &lt, Some(scene))?,
        ])?)?;
    }
    Ok(out)
}

// ---- Hatches -----------------------------------------------------------------

#[pyfunction]
pub fn hatch_families<'py>(py: Python<'py>, scene: &Obj<'py>, key: &Obj<'py>) -> PyResult<Obj<'py>> {
    let imported: String = hatches(py)?.getattr("IMPORTED")?.extract()?;
    let Some(name) = key.cast::<PyString>().ok().and_then(|s| s.to_str().ok()?.strip_prefix(imported.as_str()).map(str::to_string)) else {
        return Ok(none(py));
    };
    let entry = table(scene, "hatch_patterns")?.call_method1("get", (name,))?;
    if entry.is_instance_of::<PyDict>() { entry.call_method1("get", ("families",)) } else { Ok(none(py)) }
}

#[pyfunction]
pub fn layer_brick<'py>(py: Python<'py>, scene: &Obj<'py>, ly: &Obj<'py>) -> PyResult<Obj<'py>> {
    let own = opt(ly, "brick")?;
    if !own.is_none() {
        return Ok(own);
    }
    let kind = hatches(py)?.call_method1("piece_kind", (opt(ly, "hatch")?,))?;
    if !kind.is_truthy()? {
        return Ok(none(py));
    }
    table(scene, "bricks")?.call_method1("get", (kind,))
}

#[pyfunction]
pub fn load_bricks(py: Python<'_>, scene: &Obj<'_>, raw: &Obj<'_>) -> PyResult<()> {
    if !raw.is_instance_of::<PyDict>() {
        return Ok(());
    }
    let h = hatches(py)?;
    let sizes = h.getattr("PIECE_SIZES")?;
    for item in raw.call_method0("items")?.try_iter()? {
        let (kind, size): (Obj, Obj) = item?.extract()?;
        let size = if sizes.contains(&kind)? { h.call_method1("clean_brick", (size,))? } else { none(py) };
        if !size.is_none() {
            let bricks = py.get_type::<PyDict>().call1((table(scene, "bricks")?,))?;
            bricks.set_item(kind, size)?;
            scene.setattr("bricks", bricks)?;
        }
    }
    Ok(())
}

#[pyfunction]
pub fn layer_hatch_families<'py>(py: Python<'py>, scene: &Obj<'py>, ly: &Obj<'py>) -> PyResult<Obj<'py>> {
    let key = opt(ly, "hatch")?;
    let families = hatch_families(py, scene, &key)?;
    let brick = layer_brick(py, scene, ly)?;
    if families.is_none() && !brick.is_none() {
        let h = hatches(py)?;
        if h.getattr("BRICKS")?.contains(&key)? {
            let out = PyList::empty(py);
            for f in h.call_method1("brick_families", (&key, &brick))?.try_iter()? {
                let f = f?;
                let row = PyList::empty(py);
                for v in f.get_item(pyo3::types::PySlice::new(py, 0, 5, 1))?.try_iter()? {
                    row.append(v?)?;
                }
                row.append(py.get_type::<PyList>().call1((f.get_item(5)?,))?)?;
                out.append(row)?;
            }
            return Ok(out.into_any());
        }
    }
    Ok(families)
}

#[pyfunction]
pub fn hatch_label<'py>(py: Python<'py>, scene: &Obj<'py>, key: &Obj<'py>) -> PyResult<Obj<'py>> {
    let _ = scene;
    let h = hatches(py)?;
    let labels = h.getattr("HATCH_LABELS")?;
    if labels.contains(key).unwrap_or(false) {
        return labels.get_item(key);
    }
    let imported: String = h.getattr("IMPORTED")?.extract()?;
    if let Some(name) = key.cast::<PyString>().ok().and_then(|s| s.to_str().ok()?.strip_prefix(imported.as_str()).map(str::to_string)) {
        return Ok(PyString::new(py, &name).into_any());
    }
    Ok(PyString::new(py, "").into_any())
}

#[pyfunction]
pub fn hatch_grows(py: Python<'_>, ly: &Obj<'_>) -> PyResult<bool> {
    let own = opt(ly, "hatch_on_paper")?;
    if !own.is_none() {
        return Ok(!own.is_truthy()?);
    }
    hatches(py)?.getattr("REAL_SIZE")?.contains(opt(ly, "hatch")?)
}

fn or_number(v: Obj<'_>, default: f64) -> PyResult<f64> {
    if v.is_truthy()? { v.extract() } else { Ok(default) }
}

/// `cut_hatch_forms(scene, scale_n=100.0)`: how each hatched tag's pattern
/// is laid on a sheet at 1:`scale_n`.
#[pyfunction]
#[pyo3(signature = (scene, scale_n=None))]
pub fn cut_hatch_forms<'py>(py: Python<'py>, scene: &Obj<'py>, scale_n: Option<&Obj<'py>>) -> PyResult<Bound<'py, PyDict>> {
    let scale_n = match scale_n {
        Some(s) => s.clone(),
        None => 100.0f64.into_pyobject(py)?.into_any(),
    };
    let n = or_number(scale_n.clone(), 100.0)?;
    let out = PyDict::new(py);
    let h = hatches(py)?;
    for ly in layers(scene)? {
        let key = opt(&ly, "hatch")?;
        if !key.is_truthy()? {
            continue;
        }
        let mut size = or_number(ly.getattr_opt("hatch_scale")?.unwrap_or_else(|| 1.0f64.into_pyobject(py).unwrap().into_any()), 1.0)?;
        if hatch_grows(py, &ly)? {
            let real = h.call_method1("real_scale", (&key, &scale_n, layer_brick(py, scene, &ly)?))?;
            size *= if real.is_none() { 100.0 / n.max(1e-6) } else { real.extract::<f64>()? };
        }
        let form = PyDict::new(py);
        form.set_item("angle", or_number(opt(&ly, "hatch_angle")?, 0.0)?)?;
        form.set_item("scale", size)?;
        form.set_item("families", layer_hatch_families(py, scene, &ly)?)?;
        form.set_item("brick", layer_brick(py, scene, &ly)?)?;
        form.set_item("cross", opt(&ly, "hatch_cross")?.is_truthy()?)?;
        form.set_item("joints", opt(&ly, "joints")?)?;
        form.set_item("mortar", opt(&ly, "mortar")?.is_truthy()?)?;
        form.set_item("hatch_scale", or_number(opt(&ly, "hatch_scale")?, 1.0)?)?;
        form.set_item("offset", opt(&ly, "hatch_offset")?)?;
        form.set_item("scale_n", n)?;
        out.set_item(ly.getattr("key")?, form)?;
    }
    Ok(out)
}

fn by_hatched<'py>(py: Python<'py>, scene: &Obj<'py>, f: fn(Python<'py>, &Obj<'py>) -> PyResult<Obj<'py>>)
    -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    for ly in layers(scene)? {
        if !opt(&ly, "hatch")?.is_truthy()? {
            continue;
        }
        let rgb = f(py, &ly)?;
        if !rgb.is_none() {
            out.set_item(ly.getattr("key")?, rgb)?;
        }
    }
    Ok(out)
}

#[pyfunction]
pub fn cut_hatches<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    for ly in layers(scene)? {
        let h = opt(&ly, "hatch")?;
        if h.is_truthy()? {
            out.set_item(ly.getattr("key")?, h)?;
        }
    }
    Ok(out)
}

#[pyfunction]
pub fn cut_hatch_grounds<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    by_hatched(py, scene, hatch_ground_rgb)
}

#[pyfunction]
pub fn cut_hatch_inks<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    by_hatched(py, scene, hatch_ink_rgb)
}

#[pyfunction]
pub fn cut_fill_colours<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    for ly in layers(scene)? {
        let key = ly.getattr("key")?;
        if !key.eq(DEFAULT_LAYER)? {
            out.set_item(key, layer_color(py, &ly)?)?;
        }
    }
    Ok(out)
}

/// `clean_offset(raw)`: `(x, y)` real mm within ±10 m, or `None`.
#[pyfunction]
pub fn clean_offset<'py>(py: Python<'py>, raw: &Obj<'py>) -> PyResult<Obj<'py>> {
    let ok = (raw.is_instance_of::<PyList>() || raw.is_instance_of::<PyTuple>()) && raw.len()? == 2;
    if !ok {
        return Ok(none(py));
    }
    let float = py.import("builtins")?.getattr("float")?;
    let mut v = [0.0; 2];
    for (i, c) in raw.try_iter()?.enumerate() {
        match float.call1((c?,)).and_then(|f| f.extract::<f64>()) {
            Ok(f) => v[i] = f,
            Err(_) => return Ok(none(py)),
        }
    }
    let [x, y] = v;
    if !(x.is_finite() && y.is_finite()) || (x == 0.0 && y == 0.0) {
        return Ok(none(py));
    }
    let clamp = |a: f64| if a > -10000.0 { if a < 10000.0 { a } else { 10000.0 } } else { -10000.0 };
    Ok((clamp(x), clamp(y)).into_pyobject(py)?.into_any())
}

// ---- Legend and print ----------------------------------------------------------

/// `legend_rows(scene)`: the visible, printed tags' labels and swatches.
#[pyfunction]
pub fn legend_rows<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<(Bound<'py, PyList>, Bound<'py, PyDict>)> {
    let rows = PyList::empty(py);
    let swatches = PyDict::new(py);
    let forms = cut_hatch_forms(py, scene, None)?;
    for ly in layers(scene)? {
        if !effective_visible(scene, &ly)? || !ly.getattr_opt("printable")?.is_none_or(|p| p.is_truthy().unwrap_or(true)) {
            continue;
        }
        let key = ly.getattr("key")?;
        let label = path_label(scene, &key, "/")?;
        rows.append(&label)?;
        let swatch = PyDict::new(py);
        swatch.set_item("color", rounded_list(py, &layer_color(py, &ly)?, 4)?)?;
        let lt = ly.getattr("linetype")?;
        swatch.set_item("linetype", &lt)?;
        swatches.set_item(&label, &swatch)?;
        if custom_name(&lt).is_some() {
            let p = linetype_pattern(py, &lt, Some(scene))?;
            swatch.set_item("pattern", py.get_type::<PyList>().call1((p,))?)?;
        }
        let hatch = opt(&ly, "hatch")?;
        if hatch.is_truthy()? {
            let form = forms.get_item(&key)?.unwrap();
            swatch.set_item("hatch_form", &form)?;
            swatch.set_item("hatch", &hatch)?;
            let angle = ly.getattr_opt("hatch_angle")?;
            if let Some(a) = angle.filter(|a| a.is_truthy().unwrap_or(false)) {
                swatch.set_item("hatch_angle", a)?;
            }
            let families = form.get_item("families")?;
            if families.is_truthy()? {
                swatch.set_item("hatch_families", families)?;
            }
            for (k, rgb) in [("hatch_ground", hatch_ground_rgb(py, &ly)?), ("hatch_color", hatch_ink_rgb(py, &ly)?)] {
                if !rgb.is_none() {
                    swatch.set_item(k, rounded_list(py, &rgb, 4)?)?;
                }
            }
        }
    }
    Ok((rows, swatches))
}

#[pyfunction]
pub fn unprinted<'py>(scene: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let mut out = Vec::new();
    for ly in layers(scene)? {
        if !ly.getattr_opt("printable")?.is_none_or(|p| p.is_truthy().unwrap_or(true)) {
            out.push(ly);
        }
    }
    Ok(out)
}

#[pyfunction]
pub fn hide_unprinted(scene: &Obj<'_>) -> PyResult<()> {
    for ly in unprinted(scene)? {
        ly.setattr("visible", false)?;
    }
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "tag_styles")?;
    for f in [
        wrap_pyfunction!(layer_color, &sub)?,
        wrap_pyfunction!(light_layer_color, &sub)?,
        wrap_pyfunction!(hatch_ground_rgb, &sub)?,
        wrap_pyfunction!(hatch_ink_rgb, &sub)?,
        wrap_pyfunction!(is_link_untagged, &sub)?,
        wrap_pyfunction!(object_layer, &sub)?,
        wrap_pyfunction!(layer_paint, &sub)?,
        wrap_pyfunction!(layer_fx, &sub)?,
        wrap_pyfunction!(layer_look, &sub)?,
        wrap_pyfunction!(clean_line_colour, &sub)?,
        wrap_pyfunction!(linetype_pattern, &sub)?,
        wrap_pyfunction!(linetype_text, &sub)?,
        wrap_pyfunction!(linetype_color, &sub)?,
        wrap_pyfunction!(set_linetype_color, &sub)?,
        wrap_pyfunction!(load_linetype_colors, &sub)?,
        wrap_pyfunction!(model_line_styles, &sub)?,
        wrap_pyfunction!(layer_lines_wanted, &sub)?,
        wrap_pyfunction!(line_styles, &sub)?,
        wrap_pyfunction!(hatch_families, &sub)?,
        wrap_pyfunction!(layer_brick, &sub)?,
        wrap_pyfunction!(load_bricks, &sub)?,
        wrap_pyfunction!(layer_hatch_families, &sub)?,
        wrap_pyfunction!(hatch_label, &sub)?,
        wrap_pyfunction!(hatch_grows, &sub)?,
        wrap_pyfunction!(cut_hatch_forms, &sub)?,
        wrap_pyfunction!(cut_hatches, &sub)?,
        wrap_pyfunction!(cut_hatch_grounds, &sub)?,
        wrap_pyfunction!(cut_hatch_inks, &sub)?,
        wrap_pyfunction!(cut_fill_colours, &sub)?,
        wrap_pyfunction!(clean_offset, &sub)?,
        wrap_pyfunction!(legend_rows, &sub)?,
        wrap_pyfunction!(unprinted, &sub)?,
        wrap_pyfunction!(hide_unprinted, &sub)?,
    ] {
        sub.add_function(f)?;
    }
    m.add_submodule(&sub)?;
    Ok(())
}
