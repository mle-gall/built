// SPDX-License-Identifier: GPL-3.0-or-later
//! Enlist the stores touched by every command, including opaque compound members.
use pyo3::{prelude::*, types::PyList};
type Obj<'py> = Bound<'py, PyAny>;

pub(super) fn uses_transaction(cmd: &Obj<'_>, scene: &Obj<'_>) -> PyResult<bool> {
    if let Some(flag) = cmd.getattr_opt("_journal_geometry")? {
        if flag.is_truthy()? {
            return Ok(true);
        }
    }
    let Some(flag) = cmd.getattr_opt("doc_undo")? else {
        return Ok(false);
    };
    if flag.is_callable() {
        return match flag.call1((scene,)).and_then(|v| v.is_truthy()) {
            Err(e) if e.is_instance_of::<pyo3::exceptions::PyException>(cmd.py()) => Ok(false),
            r => r,
        };
    }
    flag.is_truthy()
}
pub(super) fn enlist(
    api: &Bound<'_, PyModule>,
    cmd: &Obj<'_>,
    scene: &Obj<'_>,
    targets: &Bound<'_, PyList>,
) -> PyResult<bool> {
    let compound = api.getattr("CompoundCommand")?;
    if cmd.is_instance(&compound)? {
        let mut covered = true;
        for part in cmd.getattr("commands")?.try_iter()? {
            covered &= enlist(api, &part?, scene, targets)?;
        }
        return Ok(covered
            && (cmd.get_type().is(&compound) || cmd.getattr("journal_parts")?.is_truthy()?));
    }
    for (class, field) in [
        ("GroupInContextCommand", "inner"),
        ("MakeComponentOfCommand", "_nest"),
        ("MakeNestedGroupCommand", "_inner"),
    ] {
        if cmd.is_instance(&api.getattr(class)?)? {
            let inner = cmd.getattr(field)?;
            if !inner.is_none() {
                return enlist(api, &inner, scene, targets);
            }
        }
    }
    if !cmd.is_instance(&api.getattr("MeshCommand")?)? {
        return uses_transaction(cmd, scene);
    }
    let meshes = cmd.call_method1("geometry_meshes", (scene,))?;
    let native = cmd.py().import("built_core.store_mesh")?.getattr("StoreMesh")?;
    let stores = PyList::empty(cmd.py());
    for mesh in meshes.try_iter()? {
        let mesh = mesh?;
        if mesh.get_type().is(&native) {
            stores.append(mesh)?;
        }
    }
    if !stores.is_empty() {
        targets.append((cmd, &stores))?;
    }
    Ok(meshes.len()? == stores.len())
}
