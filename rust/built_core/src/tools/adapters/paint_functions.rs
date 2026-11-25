// SPDX-License-Identifier: GPL-3.0-or-later
//! Functions exported for legacy tool callers.
use super::*;
fn push_recent<'py>(a: &Args<'py>) -> Reply<'py> {
    let [entries, entry, cap] = unpack::<3>(a.as_any())?;
    let key = imported(a.py(), "tools.paint_host")?.getattr("recent_key")?;
    let ident = key.call1((&entry,))?;
    let others = PyList::empty(a.py());
    for existing in entries.try_iter()? {
        let existing = existing?;
        if key.call1((&existing,))?.ne(&ident)? {
            others.append(existing)?;
        }
    }
    let result = PyList::new(a.py(), [entry])?;
    let count = cap.extract::<isize>()?.saturating_sub(1).max(0) as usize;
    result.call_method1("extend", (others.get_slice(0, count.min(others.len())),))?;
    Ok(result.into_any())
}

fn rename_recent<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let [entries, old, stamp] = unpack::<3>(a.as_any())?;
    let changed = PyList::empty(py);
    for entry in entries.try_iter()? {
        let entry = entry?;
        if entry.call_method1("get", ("mat",))?.ne(&old)? {
            changed.append(entry)?;
            continue;
        }
        let replacement = PyDict::new(py);
        for pair in entry.call_method0("items")?.try_iter()? {
            let [key, value] = unpack::<2>(&pair?)?;
            let reserved = key.extract::<String>().is_ok_and(|k| {
                matches!(k.as_str(), "mat" | "color" | "texture" | "opacity" | "pbr")
            });
            if !reserved {
                replacement.set_item(key, value)?;
            }
        }
        for pair in stamp.call_method0("items")?.try_iter()? {
            let [key, value] = unpack::<2>(&pair?)?;
            if value.is_instance_of::<PyTuple>() {
                replacement.set_item(key, py.get_type::<PyList>().call1((value,))?)?;
            } else {
                replacement.set_item(key, value)?;
            }
        }
        changed.append(replacement)?;
    }
    Ok(changed.into_any())
}

fn recent_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let mut c: Obj<'py>;
    let entry = a.get_item(0)?;
    let tint: Obj<'py>;
    if entry.call_method1("get", ("mat",))?.is_truthy()? {
        return ("m", entry.get_item("mat")?).into_bound_py_any(py);
    }
    let tex = entry.call_method1("get", ("texture",))?;
    if tex.is_truthy()? {
        tint = tex.call_method1("get", ("tint",))?;
        return (
            "t",
            tex.call_method1("get", ("path",))?,
            tex.call_method1("get", ("sw",))?,
            tex.call_method1("get", ("sh",))?,
            tex.call_method1("get", ("rot", 0.0))?,
            (if tint.is_truthy()? {
                py.get_type::<pyo3::types::PyTuple>().call1((&tint,))?
            } else {
                none(py)
            }),
            entry.call_method1("get", ("opacity",))?,
        )
            .into_bound_py_any(py);
    }
    let col = {
        let value = entry.call_method1("get", ("color",))?;
        if value.is_truthy()? {
            value
        } else {
            PyTuple::empty(py).into_any()
        }
    };
    return Ok((
        "c",
        py.get_type::<pyo3::types::PyTuple>().call1(({
            let collected = PyList::empty(py);
            for item in col.try_iter()? {
                c = item?;
                collected.append(
                    imported(py, "builtins")?
                        .getattr("round")?
                        .call1((py.get_type::<pyo3::types::PyFloat>().call1((&c,))?, 4))?,
                )?;
            }
            collected.into_any()
        },))?,
        entry.call_method1("get", ("opacity",))?,
    )
        .into_bound_py_any(py)?);
}

#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("recent_key", Mode::Function, recent_key, &["entry:dict"], "tuple", "What makes two recent entries THE SAME material: its name when it\nhas one, else the image with its size and turn (or the colour)."),
    ("push_recent", Mode::Function, push_recent, &["entries:list", "entry:dict", "cap:int=RECENT_MAX"], "list", "``entries`` with ``entry`` moved (or added) to the front, without\nduplicates and at most ``cap`` long."),
    ("rename_recent", Mode::Function, rename_recent, &["entries:list", "old:str", "stamp:dict"], "list", "``entries`` with every one named *old* now wearing *stamp* (the\nnew material's face attrs): a material renamed or recoloured in the\nmodel is not offered back under its old name and colour."),
];
