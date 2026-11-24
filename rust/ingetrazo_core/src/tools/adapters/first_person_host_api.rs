// SPDX-License-Identifier: GPL-3.0-or-later
//! FirstPersonHost callbacks on the live document.
use super::*;

fn held<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(py.get_type::<pyo3::types::PySet>().call1((({
        let value = tool.call_method1("_state", ("held",))?;
        if value.is_truthy()? {
            value
        } else {
            PyTuple::empty(py).into_any()
        }
    }),))?)
}
fn moving<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (tool.getattr("held")?.is_truthy()?).into_bound_py_any(py)
}
fn claims_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.walkthrough_host")?;
    let key = a.get_item(1)?;
    let modifiers = a.get_item(2)?;
    let tool = a.get_item(0)?;
    Ok({
        let value = (!(modifiers
            .bitand(module.getattr("Qt")?.getattr("ControlModifier")?)?
            .is_truthy()?))
        .into_bound_py_any(py)?;
        if !value.is_truthy()? {
            value
        } else {
            tool.getattr("_machine")?.call_method1(
                "query",
                (
                    "claims_key",
                    py.get_type::<pyo3::types::PyInt>().call1((&key,))?,
                ),
            )?
        }
    })
}
fn on_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.walkthrough_host")?;
    let key = a.get_item(2)?;
    let modifiers = a.get_item(3)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    Ok(tool.call_method1("_send", (&viewport, fields!(py; "type" => "key", "key" => py.get_type::<pyo3::types::PyString>().call1((py.get_type::<pyo3::types::PyInt>().call1((&key,))?,))?, "mods" => module.call_method1("_mods", (&modifiers,))?).into_any(),))?.get_item("consumed")?)
}
fn on_key_release<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    Ok(tool.call_method1("_send", (&viewport, fields!(py; "type" => "key_release", "key" => py.get_type::<pyo3::types::PyString>().call1((py.get_type::<pyo3::types::PyInt>().call1((&key,))?,))?).into_any(),))?.get_item("consumed")?)
}
fn on_focus_out<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("on_cancel", (&viewport,))?;
    Ok(none(py))
}
fn step<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.walkthrough_host")?;
    let dt = a.get_item(3)?;
    let keys = a.get_item(2)?;
    let modifiers = a.get_item(4)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_send", (&viewport, fields!(py; "type" => "first_person_step", "keys" => py.get_type::<pyo3::types::PyList>().call1((py.import("builtins")?.getattr("map")?.call1((py.get_type::<pyo3::types::PyInt>().into_any(), &keys,))?,))?, "dt" => &dt, "mods" => module.call_method1("_mods", (&modifiers,))?).into_any(),))?;
    Ok(none(py))
}
fn on_look<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let dx = a.get_item(2)?;
    let dy = a.get_item(3)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1(
        "_send",
        (
            &viewport,
            fields!(py; "type" => "look_delta", "delta" => (&dx, &dy,).into_bound_py_any(py)?)
                .into_any(),
        ),
    )?;
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn host_look_rate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let walkthrough = py.import("tools.walkthrough")?.into_any();
    walkthrough.call_method0("look_deg_per_px")
}
fn context_menu<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    true.into_bound_py_any(py)
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("held", Mode::Property, held, &["self"], "", ""),
    ("moving", Mode::Property, moving, &["self"], "", ""),
    ("claims_key", Mode::Method, claims_key, &["self", "key", "modifiers"], "", ""),
    ("on_key", Mode::Method, on_key, &["self", "viewport", "key", "modifiers"], "", ""),
    ("on_key_release", Mode::Method, on_key_release, &["self", "viewport", "key"], "", ""),
    ("on_focus_out", Mode::Method, on_focus_out, &["self", "viewport"], "", ""),
    ("step", Mode::Method, step, &["self", "viewport", "keys", "dt", "modifiers=Qt.NoModifier"], "", ""),
    ("on_look", Mode::Method, on_look, &["self", "viewport", "dx", "dy"], "", ""),
    ("host_look_rate", Mode::Method, host_look_rate, &["self", "viewport", "_"], "", ""),
    ("context_menu", Mode::Method, context_menu, &["self", "viewport", "pos"], "", ""),
];
