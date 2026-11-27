// SPDX-License-Identifier: GPL-3.0-or-later
//! Capture only the meshes touched by the outermost History transaction.
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
use std::collections::HashSet;
type Obj<'py> = Bound<'py, PyAny>;
fn none<'py>(py: Python<'py>) -> Obj<'py> {
    py.None().into_bound(py)
}
fn serials<'py>(m: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    (m.getattr("_mut_serial")?, m.getattr_opt("revision")?).into_pyobject(m.py())
}
pub(in crate::doc) fn tokens<'py>(
    cmd: &Obj<'py>,
    scene: &Obj<'py>,
    phase: &str,
    attrs_copy_max: isize,
) -> PyResult<Bound<'py, PyList>> {
    let py = cmd.py();
    let history = py.import("built_core.history")?;
    let compound = history.getattr("CompoundCommand")?;
    let mesh_cmd = history.getattr("MeshCommand")?;
    let active = scene.getattr("mesh")?;
    let mut meshes = vec![if phase == "execute" {
        active
    } else {
        cmd.getattr_opt("_history_mesh")?
            .filter(|v| !v.is_none())
            .unwrap_or(active)
    }];
    let mut stack = vec![cmd.clone()];
    while let Some(c) = stack.pop() {
        if phase != "execute" {
            if let Some(m) = c.getattr_opt("_history_mesh")? {
                meshes.push(m);
            }
        }
        if c.is_instance(&compound)? {
            for p in c.getattr("commands")?.try_iter()? {
                stack.push(p?);
            }
        } else if c.is_instance(&mesh_cmd)? {
            for m in c.call_method1("geometry_meshes", (scene,))?.try_iter()? {
                meshes.push(m?);
            }
        }
        for attr in ["reference_instance", "reference_assembly"] {
            if c.getattr_opt(attr)?
                .map(|v| v.is_truthy())
                .transpose()?
                .unwrap_or(false)
            {
                meshes.push(c.getattr("proto")?);
                break;
            }
        }
        if matches!(
            c.get_type().name()?.to_str()?,
            "SoftenByAngleCommand" | "FlipFacesCommand" | "FlipGroupsCommand"
        ) {
            if let Some(op) = c.getattr_opt("meshes")?.filter(|v| v.is_callable()) {
                for m in op.call0()?.try_iter()? {
                    meshes.push(m?);
                }
            }
        }
    }
    let out = PyList::empty(py);
    let mut seen = HashSet::new();
    for m in meshes {
        if m.is_none() {
            continue;
        }
        let d = m.getattr("__dict__")?;
        let m = d.call_method1("get", ("_instance_definition", m))?;
        if !seen.insert(m.as_ptr()) || m.is_none() {
            continue;
        }
        let serials = serials(&m)?;
        let d = m.getattr("__dict__")?;
        let snapshot = if let Some(store) = crate::protocol::keys::store_of(&m)? {
            let owned = d.call_method1("get", ("_attr_own",))?;
            if (if owned.is_none() {
                0
            } else {
                owned.len()? as isize
            }) > attrs_copy_max
            {
                store.call_method0("snapshot")?
            } else {
                m.call_method0("capture_state")?
            }
        } else {
            none(py)
        };
        out.append((m, (snapshot, serials)))?;
    }
    Ok(out)
}
/// The persistent id of every key the record lists, and the step's
/// succession lineage (`doc/history/dispatch.rs`).
fn element_ids<'py>(kw: &Bound<'py, PyDict>, store: &Obj<'py>, mesh: &Obj<'py>) -> PyResult<()> {
    let py = kw.py();
    if !store.hasattr("ids_of")? {
        return Ok(());
    }
    let ids = PyDict::new(py);
    for (kind, letter) in [("vertex", 'v'), ("edge", 'e'), ("face", 'f')] {
        let mut keys: Vec<u128> = Vec::new();
        for name in ["created", "removed", "modified"] {
            if let Some(d) = kw.get_item(name)? {
                if let Some(l) = d.get_item(kind).ok() {
                    keys.extend(l.extract::<Vec<u128>>()?);
                }
            }
        }
        if kind == "vertex" {
            if let Some(m) = kw.get_item("moved")? {
                keys.extend(m.extract::<Vec<u128>>()?);
            }
        }
        let out = PyDict::new(py);
        for k in keys {
            if let Ok(id) = store.call_method1("id_of", (kind, k)) {
                out.set_item(crate::protocol::keys::to_json_key(&k.into_pyobject(py)?.into_any())?, crate::element_ids::to_text(letter, id.extract()?))?;
            }
        }
        ids.set_item(kind, out)?;
    }
    kw.set_item("ids", ids)?;
    if let Some(l) = mesh.getattr("__dict__")?.cast::<PyDict>()?.get_item("_id_lineage")? {
        if l.get_item(0)?.eq(store.getattr("revision")?)? {
            let pair = l.get_item(1)?;
            let rows = |v: Obj<'py>| -> PyResult<Bound<'py, PyList>> {
                let out = PyList::empty(py);
                for t in v.try_iter()? {
                    let t = t?;
                    let row = PyDict::new(py);
                    row.set_item("kind", t.get_item(0)?)?;
                    row.set_item("id", t.get_item(1)?)?;
                    row.set_item("parent", t.get_item(2)?)?;
                    out.append(row)?;
                }
                Ok(out)
            };
            kw.set_item("derived", rows(pair.get_item(0)?)?)?;
            kw.set_item("merged", rows(pair.get_item(1)?)?)?;
        }
    }
    Ok(())
}
pub(super) fn mesh_change<'py>(
    class: &Obj<'py>,
    m: &Obj<'py>,
    token: &Obj<'py>,
    painted: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = m.py();
    let before = token.get_item(0)?;
    let old = token.get_item(1)?;
    let store = crate::protocol::keys::store_of(m)?;
    let kw = PyDict::new(py);
    let backing = if let Some(st) = &store {
        kw.set_item("revision", st.getattr("revision")?)?;
        if !before.is_none() {
            let state = py.import("built_core.store_mesh")?.getattr("StoreState")?;
            let snap = st.call_method0("snapshot")?;
            let change = if before.is_instance(&state)? {
                let d = m.getattr("__dict__")?;
                state
                    .call1((
                        snap,
                        d.get_item("_attr_own")?,
                        d.get_item("_attr_table")?,
                        m,
                    ))?
                    .call_method1("changes_since", (&before,))
            } else {
                snap.call_method1("changes_since", (&before, PyList::empty(py)))
            };
            match change {
                Ok(ch) => {
                    for name in ["created", "removed", "modified", "moved"] {
                        kw.set_item(name, ch.getattr(name)?)?;
                    }
                    element_ids(&kw, st, m)?;
                }
                Err(e) if e.is_instance_of::<PyValueError>(py) => kw.set_item("replaced", true)?,
                Err(e) => return Err(e),
            }
            return class.call((m, "store"), Some(&kw));
        }
        "store"
    } else {
        "python"
    };
    let now = serials(m)?;
    let replaced = before.is_none() != store.is_none()
        || !now.get_item(0)?.eq(old.get_item(0)?)?
        || (!now.get_item(1)?.eq(old.get_item(1)?)? && !painted.is_truthy()?);
    kw.set_item("replaced", replaced)?;
    class.call((m, backing), Some(&kw))
}
