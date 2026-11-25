// SPDX-License-Identifier: GPL-3.0-or-later
//! Command hints supplement the store and document diffs for paint and copies.
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
type Obj<'py> = Bound<'py, PyAny>;
fn visit<'py>(cmd: &Obj<'py>, mut leaf: impl FnMut(&Obj<'py>) -> PyResult<()>) -> PyResult<()> {
    let compound = cmd
        .py()
        .import("built_core.history")?
        .getattr("CompoundCommand")?;
    let mut stack = vec![cmd.clone()];
    while let Some(c) = stack.pop() {
        if c.is_instance(&compound)? {
            for part in c.getattr("commands")?.try_iter()? {
                stack.push(part?);
            }
        } else {
            leaf(&c)?;
        }
    }
    Ok(())
}
pub(super) fn painted<'py>(cmd: &Obj<'py>, default: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let py = cmd.py();
    let out = PyDict::new(py);
    visit(cmd, |c| {
        if !c.get_type().name()?.to_str()?.starts_with("SetFace") {
            return Ok(());
        }
        let Some(faces) = c.getattr_opt("_faces")? else {
            return Ok(());
        };
        let Ok(faces) = faces.cast::<PyList>() else {
            return Ok(());
        };
        for face in faces.iter() {
            let Some(key) = face.getattr_opt("key")? else {
                continue;
            };
            let mesh = face
                .getattr_opt("_m")?
                .filter(|m| !m.is_none())
                .unwrap_or_else(|| default.clone());
            let owner = mesh
                .getattr("__dict__")?
                .call_method1("get", ("_instance_definition", &mesh))?;
            let dest = match out.get_item(&owner)? {
                Some(v) => v,
                None => {
                    let v = PyList::empty(py);
                    out.set_item(owner, &v)?;
                    v.into_any()
                }
            };
            dest.call_method1("append", (key,))?;
        }
        Ok(())
    })?;
    Ok(out)
}
pub(super) fn copies<'py>(cmd: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let py = cmd.py();
    let out = PyList::empty(py);
    visit(cmd, |c| {
        let Some(entries) = c.getattr_opt("copies")?.filter(|v| !v.is_none()) else {
            return Ok(());
        };
        for entry in entries.try_iter()? {
            let entry = entry?;
            let row = PyDict::new(py);
            for (name, i) in [("source", 0), ("target", 1)] {
                row.set_item(name, crate::protocol::keys::mesh_key(&entry.get_item(i)?)?)?;
            }
            let keys = entry.get_item(2)?;
            let map = PyDict::new(py);
            for k in keys.try_iter()? {
                let k = k?;
                map.set_item(&k, py.get_type::<PyList>().call1((keys.get_item(&k)?,))?)?;
            }
            row.set_item("keys", map)?;
            out.append(row)?;
        }
        Ok(())
    })?;
    Ok(out)
}
