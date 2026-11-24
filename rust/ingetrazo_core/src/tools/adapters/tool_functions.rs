// SPDX-License-Identifier: GPL-3.0-or-later
//! Functions exported for legacy tool callers.
use super::*;

fn tuple<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let p = a.get_item(0)?;
    Ok(if p.is_none() {
        none(py)
    } else {
        if p.hasattr("toTuple")? {
            p.call_method0("toTuple")?
        } else {
            py.get_type::<pyo3::types::PyTuple>().call1((&p,))?
        }
    })
}

fn selection_mode<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tool_base")?;
    let modifiers = a.get_item(0)?;
    let select_mode = imported(py, "ingetrazo_core")?.getattr("select_mode")?;
    select_mode.call1((module.call_method1("_mods", (&modifiers,))?,))
}

fn mods<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let modifiers = a.get_item(0)?;
    Ok(py.get_type::<pyo3::types::PyInt>().call1((({
        let value = modifiers.getattr_opt("value")?.unwrap_or(modifiers.clone());
        if value.is_truthy()? {
            value
        } else {
            0.into_bound_py_any(py)?
        }
    }),))?)
}

#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("_mods", Mode::Function, mods, &["modifiers"], "int", ""),
    ("selection_mode", Mode::Function, selection_mode, &["modifiers"], "str", "How a click or a box with these modifiers joins the selection\n(the Rust rule: Shift toggles, Ctrl adds, Shift+Ctrl removes)."),
    ("_tuple", Mode::Function, tuple, &["p"], "", ""),
];
