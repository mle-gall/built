// SPDX-License-Identifier: GPL-3.0-or-later
//! Functions exported for legacy tool callers.
use super::*;

fn commit_arc<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let mut first: Obj<'py>;
    let mut b: Obj<'py>;
    let close_to = a.get_item(2)?;
    let mut p: Obj<'py>;
    let pts = a.get_item(1)?;
    let trim = a.get_item(3)?;
    let viewport = a.get_item(0)?;
    let arguments = fields!(py; "points" => {let items=PyList::empty(py); for item in pts.try_iter()? { p = item?;
items.append(py.get_type::<pyo3::types::PyList>().call1((module.call_method1("_tuple", (&p,))?,))?)?;
} items.into_any()}).into_any();
    if !close_to.is_none() {
        arguments.set_item(
            "center",
            py.get_type::<pyo3::types::PyList>()
                .call1((module.call_method1("_tuple", (&close_to,))?,))?,
        )?;
    }
    if trim.is_truthy()? {
        arguments.set_item("trim", {
            let items = PyList::empty(py);
            for item in trim.try_iter()? {
                {
                    let [part0, part1] = unpack::<2>(&item?)?;
                    first = part0;
                    b = part1;
                }
                items.append(
                    PyList::new(
                        py,
                        [
                            py.get_type::<pyo3::types::PyList>()
                                .call1((module.call_method1("_tuple", (&first,))?,))?,
                            py.get_type::<pyo3::types::PyList>()
                                .call1((module.call_method1("_tuple", (&b,))?,))?,
                        ],
                    )?
                    .into_any(),
                )?;
            }
            items.into_any()
        })?;
    }
    module.call_method1("_Host", (&viewport,))?.call_method1(
        "run",
        (
            "add_arc",
            module
                .getattr("json")?
                .call_method1("dumps", (&arguments,))?,
        ),
    )
}

fn degrees<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let value = a.get_item(0)?;
    let bare_number_scale = imported(py, "core.units")?.getattr("bare_number_scale")?;
    let scale = bare_number_scale.call0()?;
    Ok(if scale.is_truthy()? {
        py.get_type::<pyo3::types::PyFloat>()
            .call1((&value,))?
            .div(&scale)?
    } else {
        py.get_type::<pyo3::types::PyFloat>().call1((&value,))?
    })
}

#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("_degrees", Mode::Function, degrees, &["value"], "float", "An angle typed in the value box: the parser reads a bare field as a\nLENGTH in the document's unit (90 → 0.09 in millimetres); an angle has\nnone."),
    ("commit_arc", Mode::Function, commit_arc, &["viewport", "pts", "close_to=None", "trim=None"], "", "Draw an arc polyline (Freehand's commit): the protocol's ``add_arc``."),
];
