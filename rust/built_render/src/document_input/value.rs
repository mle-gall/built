// SPDX-License-Identifier: GPL-3.0-or-later
//! The value grammar lives in core. Dispatch its result to the active tool,
//! retaining expression feedback and recognizing a replaced undo-stack head.
use super::*;
fn chord(mods: &Obj) -> PyResult<bool> {
    let codes = host(mods.py())?.getattr("Qt")?;
    let has = |name: &str| mods.bitand(codes.getattr(name)?)?.is_truthy();
    let (ctrl, alt) = (has("ControlModifier")?, has("AltModifier")?);
    Ok(!(ctrl && alt) && (ctrl || alt || has("MetaModifier")?))
}
fn accepts<'py>(
    v: &Obj<'py>,
    t: &Obj<'py>,
    text: &str,
    buffer: &str,
) -> PyResult<Bound<'py, PyDict>> {
    let kw = PyDict::new(v.py());
    let arrays = opt(t, "accepts_array")?.is_truthy()?;
    kw.set_item("arrays", arrays)?;
    kw.set_item(
        "array_pending",
        arrays
            && buffer.is_empty()
            && text.eq_ignore_ascii_case("x")
            && v.call_method0("_array_pending")?.is_truthy()?,
    )?;
    kw.set_item("profile", t.hasattr("on_profile_value")?)?;
    Ok(kw)
}
pub(super) fn claims(v: &Obj, ev: &Obj, buffer: &str) -> PyResult<bool> {
    if chord(&ev.call_method0("modifiers")?)? {
        return Ok(false);
    }
    let t = tool(v)?;
    let text = ev.call_method0("text")?.extract::<String>()?.to_lowercase();
    let kw = accepts(v, &t, &text, buffer)?;
    kw.set_item("zoom", t.is_none() && opt(v, "nav_mode")?.eq("zoom")?)?;
    v.py()
        .import("built_core")?
        .getattr("value_claims")?
        .call((buffer, text, !t.is_none()), Some(&kw))?
        .is_truthy()
}
fn error(v: &Obj, error: &Obj, word: &Obj) -> PyResult<()> {
    let py = v.py();
    let kw = PyDict::new(py);
    kw.set_item("error", error)?;
    kw.set_item("word", word)?;
    let message = py
        .import("built_core.units")?
        .getattr("Parsed")?
        .call((), Some(&kw))?
        .call_method0("message")?;
    v.call_method1("flash_status", (message, 4000))?;
    Ok(())
}
fn clear(v: &Obj) -> PyResult<()> {
    v.call_method1("_set_value_buffer", ("",))?;
    Ok(())
}
fn zoom_enter(v: &Obj, buffer: &str) -> PyResult<bool> {
    if buffer.is_empty() {
        return Ok(false);
    }
    let (typed, err, word): (Obj, Obj, Obj) = v
        .py()
        .import("built_core")?
        .call_method1("zoom_value_enter", (buffer,))?
        .extract()?;
    if typed.is_none() {
        error(v, &err, &word)?
    } else {
        v.call_method1("apply_zoom_value", (typed,))?;
        clear(v)?
    }
    Ok(true)
}
fn tagged(v: &Obj, t: &Obj, value: &Obj) -> PyResult<bool> {
    if !value.is_instance_of::<PyTuple>() || value.len()? == 0 {
        return Ok(false);
    }
    let Ok(kind) = value.get_item(0)?.extract::<String>() else {
        return Ok(false);
    };
    if kind == "ratio" {
        if opt(t, "accepts_angle_ratio")?.is_truthy()? {
            t.call_method1("on_value", (v, value.get_item(1)?))?;
        }
        return Ok(true);
    }
    let handler = opt(t, &format!("on_{kind}_value"))?;
    if handler.is_none() {
        return Ok(true);
    }
    if kind != "array" {
        handler.call1((v, value.get_item(1)?))?;
    } else if value.len()? <= 3 {
        handler.call1((v, value.get_item(1)?, value.get_item(2)?))?;
    } else if v
        .py()
        .import("inspect")?
        .call_method1("signature", (&handler,))?
        .getattr("parameters")?
        .contains("step")?
    {
        let kw = PyDict::new(v.py());
        kw.set_item("step", value.get_item(3)?)?;
        handler.call((v, value.get_item(1)?, value.get_item(2)?), Some(&kw))?;
    } else {
        v.call_method1(
            "flash_status",
            (host(v.py())?.call_method1("tr", ("Type the count alone here, e.g. 3x",))?,),
        )?;
    }
    Ok(true)
}
fn expression_text<'py>(
    units: &Bound<'py, PyModule>,
    value: &Obj<'py>,
    plain: bool,
) -> PyResult<Obj<'py>> {
    let one = |val: &Obj<'py>| {
        if plain {
            units.call_method1("localized", (val.call_method1("__format__", (".6g",))?,))
        } else {
            units.call_method1("fmt_len", (val,))
        }
    };
    if !value.is_instance_of::<PyTuple>() {
        return one(value);
    }
    if value.len()? > 0 && value.get_item(0)?.eq("abs_len")? {
        return units.call_method1("fmt_len", (value.get_item(1)?,));
    }
    let mut parts = Vec::new();
    for val in value.try_iter()? {
        parts.push(one(&val?)?.extract::<String>()?)
    }
    Ok(parts.join("; ").into_pyobject(value.py())?.into_any())
}
fn enter(v: &Obj, t: &Obj, buffer: &str) -> PyResult<bool> {
    let py = v.py();
    let units = py.import("built_core.units")?;
    let unitless = opt(t, "value_is_unitless")?;
    let plain = unitless.is_callable() && unitless.call0()?.is_truthy()?;
    let length = units.call_method1("typed_context", ("length",))?;
    let bare = if plain {
        let guard = units.call_method0("unitless_numbers")?;
        guard.call_method0("__enter__")?;
        let result = units.call_method1("typed_context", ("length",));
        guard.call_method1("__exit__", (py.None(), py.None(), py.None()))?;
        result?.get_item(0)?
    } else {
        length.get_item(0)?
    };
    let parsed = py.import("built_core")?.call_method1(
        "value_enter",
        (
            buffer,
            opt(t, "vcb_comma_lists")?.is_truthy()?,
            bare,
            length.get_item(1)?,
            length.get_item(0)?,
            opt(t, "accepts_absolute_length")?.is_truthy()?,
        ),
    )?;
    if parsed.is_none() {
        return Ok(false);
    }
    let kind = parsed.get_item(0)?;
    if kind.eq("error")? {
        error(v, &parsed.get_item(1)?, &parsed.get_item(2)?)?;
        return Ok(true);
    }
    if kind.eq("clear")? {
        clear(v)?;
        return Ok(true);
    }
    let mut value = parsed.get_item(1)?;
    if tagged(v, t, &value)? {
        clear(v)?;
        return Ok(true);
    }
    if parsed.get_item(2)?.is_truthy()? {
        value = ("abs_len", value).into_pyobject(py)?.into_any()
    }
    let history = opt(v, "history")?;
    let before = if history.is_none() {
        0
    } else {
        history.getattr("undo_stack")?.len()?
    };
    let top = if before > 0 {
        history.getattr("undo_stack")?.get_item(before - 1)?
    } else {
        none(py)
    };
    t.call_method1("on_value", (v, &value))?;
    let after = if history.is_none() {
        0
    } else {
        history.getattr("undo_stack")?.len()?
    };
    let drew = after > before;
    let replaced = before > 0
        && after == before
        && !history.getattr("undo_stack")?.get_item(after - 1)?.is(&top);
    clear(v)?;
    if drew || replaced {
        let text = if units
            .call_method1("is_expression", (buffer,))?
            .is_truthy()?
        {
            expression_text(&units, &value, plain)?
        } else if plain {
            buffer.into_pyobject(py)?.into_any()
        } else {
            units.call_method1("typed_value_text", (buffer,))?
        };
        emit(v, "measurementChanged", &text)?
    }
    let kw = PyDict::new(py);
    kw.set_item("committed", drew)?;
    v.call_method("_release_axis_lock_after_operation", (), Some(&kw))?;
    Ok(true)
}
#[pyfunction]
fn document_value_key(v: &Obj, event: &Obj) -> PyResult<bool> {
    let t = tool(v)?;
    let zoom = t.is_none() && opt(v, "nav_mode")?.eq("zoom")?;
    if t.is_none() && !zoom {
        return Ok(false);
    }
    let key: i32 = event.call_method0("key")?.extract()?;
    let buffer = v.getattr("_value_buffer")?.extract::<String>()?;
    if key == 0x1000004 || key == 0x1000005 {
        return if zoom {
            zoom_enter(v, &buffer)
        } else {
            enter(v, &t, &buffer)
        };
    }
    let text = event.call_method0("text")?.extract::<String>()?;
    let kw = if zoom {
        PyDict::new(v.py())
    } else {
        let kw = accepts(v, &t, &text, &buffer)?;
        kw.set_item("comma_lists", opt(&t, "vcb_comma_lists")?.is_truthy()?)?;
        kw
    };
    let is_chord = match event.getattr_opt("modifiers")? {
        Some(f) => chord(&f.call0()?)?,
        None => false,
    };
    kw.set_item("chord", is_chord)?;
    kw.set_item("zoom", zoom)?;
    let name = match key {
        0x1000003 => "backspace",
        0x1000000 => "escape",
        _ => "",
    };
    let typed = v
        .py()
        .import("built_core")?
        .getattr("value_key")?
        .call((buffer, name, text), Some(&kw))?;
    if typed.is_none() {
        return Ok(false);
    }
    v.call_method1("_set_value_buffer", (typed,))?;
    Ok(true)
}
/// An idle tool can accept a replacement value after its geometry anchor
/// is gone. Use the same unit grammar as a commit.
#[pyfunction]
fn pending_tool_value(tool: &Obj, buffer: &str) -> PyResult<String> {
    if buffer.is_empty() || tool.is_none() {
        return Ok(String::new());
    }
    let unitless = opt(tool, "value_is_unitless")?;
    if unitless.is_callable() && unitless.call0()?.is_truthy()? {
        return Ok(buffer.to_owned());
    }
    tool.py().import("built_core.units")?.call_method1("typed_value_text", (buffer,))?.extract()
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(pending_tool_value, m)?)?;
    m.add_function(wrap_pyfunction!(document_value_key, m)?)
}
