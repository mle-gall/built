// SPDX-License-Identifier: GPL-3.0-or-later
//! Functions exported for legacy tool callers.
use super::*;

fn retype_open<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let cmd = a.get_item(1)?;
    let viewport = a.get_item(0)?;
    if cmd.is_none() {
        return false.into_bound_py_any(py);
    }
    let stack = optional(&optional(&viewport, "history")?, "undo_stack")?;
    Ok({
        let value = (stack.is_truthy()?).into_bound_py_any(py)?;
        if !value.is_truthy()? {
            value
        } else {
            (stack.get_item(1.into_bound_py_any(py)?.neg()?)?.is(&cmd)).into_bound_py_any(py)?
        }
    })
}

fn face_the_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let corners = a.get_item(0)?;
    let plane_normal = a.get_item(1)?;
    let native = imported(py, "ingetrazo_core")?.getattr("face_the_plane")?;
    native.call1((
        py.get_type::<pyo3::types::PyList>().call1((&corners,))?,
        &plane_normal,
    ))
}

fn ctrl_clause<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let active = a.get_item(0)?;
    let mut off = a.get_item(2)?;
    let mut on = a.get_item(1)?;
    let tr = imported(py, "ingetrazo_core.i18n")?.getattr("tr")?;
    {
        let [part0, part1] =
            unpack::<2>(&(tr.call1((&on,))?, tr.call1((&off,))?).into_bound_py_any(py)?)?;
        on = part0;
        off = part1;
    }
    ("Ctrl = ")
        .into_bound_py_any(py)?
        .add(if active.is_truthy()? {
            [
                "[".to_owned(),
                on.call_method1("__format__", ("",))?.extract::<String>()?,
                "] / ".to_owned(),
                off.call_method1("__format__", ("",))?.extract::<String>()?,
            ]
            .concat()
            .into_bound_py_any(py)?
        } else {
            [
                on.call_method1("__format__", ("",))?.extract::<String>()?,
                " / [".to_owned(),
                off.call_method1("__format__", ("",))?.extract::<String>()?,
                "]".to_owned(),
            ]
            .concat()
            .into_bound_py_any(py)?
        })
}

fn loop_normal<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let corners = a.get_item(0)?;
    let native = imported(py, "ingetrazo_core")?.getattr("loop_normal")?;
    native.call1((py.get_type::<pyo3::types::PyList>().call1((&corners,))?,))
}

#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("loop_normal", Mode::Function, loop_normal, &["corners"], "QVector3D", "Newell normal of a closed loop of points (``ingetrazo_core``)."),
    ("ctrl_clause", Mode::Function, ctrl_clause, &["active:bool", "on:str", "off:str"], "str", "The classic two-state modifier clause: «Ctrl = [copy] / original».\n\nThe state in force is the bracketed one, exactly the way the Tape and\nthe Protractor spell out their cycles, so the status bar says which way\nthe modifier is set RIGHT NOW and not just what it can do (Marco,\n2026-09-17). ``on``/``off`` are source strings handed to ``tr``."),
    ("face_the_plane", Mode::Function, face_the_plane, &["corners", "plane_normal"], "", "Wind ``corners`` so the face looks the way its plane does.\n\nA shape built by walking «along u, then along v» inherits the SIGN of\nthat walk: drag the other diagonal and the face came out backwards\n(Marco, 2026-09-18), and the back side travels into the .skp and the\nBIM tagging. A loop lying square to the plane is left as it came\n(``ingetrazo_core.face_the_plane``)."),
    ("retype_open", Mode::Function, retype_open, &["viewport", "cmd"], "bool", "Whether a hot retype may still redo ``cmd``: the command a tool\njust committed is still the top of the undo stack.\n\nThe retype (type a new measurement right after finishing, #414) undoes\nthat command and commits it again with the new value, so the model\nkeeps ONE undo step however many times the number is retyped. Anything\ndone in between — an undo, another operation — closes the window,\nbecause undoing then would take away the wrong thing."),
];
