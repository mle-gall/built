// SPDX-License-Identifier: GPL-3.0-or-later
//! _Host callbacks on the live document.
use super::*;

fn cycle_status<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let count = a.get_item(2)?;
    let ident = a.get_item(3)?;
    let index = a.get_item(1)?;
    let tool = a.get_item(0)?;
    let select = imported(py, "tools.select_host")?.into_any();
    select.call_method1(
        "cycle_status",
        (
            tool.getattr("vp")?,
            &index,
            &count,
            tool.getattr("handles")?.call_method1("get", (&ident,))?,
        ),
    )?;
    Ok(none(py))
}
fn select<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let grow = a.get_item(3)?;
    let ids = a.get_item(1)?;
    let mode = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let select = imported(py, "tools.select_host")?.into_any();
    select.call_method1(
        "apply",
        (
            tool.getattr("vp")?,
            ({
                let items = PyList::empty(py);
                for item in ids.try_iter()? {
                    let i = item?;
                    items.append(tool.getattr("handles")?.call_method1("get", (&i,))?)?;
                }
                items.into_any()
            }),
            &mode,
            &grow,
        ),
    )?;
    Ok(none(py))
}
fn box_select<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let crossing = a.get_item(2)?;
    let mode = a.get_item(3)?;
    let rect = a.get_item(1)?;
    let tool = a.get_item(0)?;
    let select = imported(py, "tools.select_host")?.into_any();
    select.call_method1("box_select", (tool.getattr("vp")?, &rect, &crossing, &mode))?;
    Ok(none(py))
}
fn erase<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let selection_only = a.get_item(1)?;
    let tool = a.get_item(0)?;
    let select = imported(py, "tools.select_host")?.into_any();
    if selection_only.is_truthy()? {
        return select.call_method1(
            "erase_entities",
            (
                tool.getattr("vp")?,
                py.get_type::<pyo3::types::PyList>()
                    .call1((tool.getattr("vp")?.getattr("scene")?.getattr("selection")?,))?,
            ),
        );
    }
    select.call_method1("delete_selection_or_hover", (tool.getattr("vp")?,))
}
fn host_verb<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ident = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let verb = a.get_item(1)?;
    let select = imported(py, "tools.select_host")?.into_any();
    select.getattr("HOST_VERBS")?.get_item(&verb)?.call1((
        tool.getattr("vp")?,
        tool.getattr("handles")?.call_method1("get", (&ident,))?,
    ))?;
    Ok(none(py))
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("cycle_status", Mode::Method, cycle_status, &["self", "index", "count", "ident"], "", ""),
    ("select", Mode::Method, select, &["self", "ids", "mode", "grow"], "", ""),
    ("box_select", Mode::Method, box_select, &["self", "rect", "crossing", "mode"], "", ""),
    ("erase", Mode::Method, erase, &["self", "selection_only"], "", ""),
    ("host_verb", Mode::Method, host_verb, &["self", "verb", "ident"], "", ""),
];
