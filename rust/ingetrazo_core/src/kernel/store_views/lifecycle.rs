// SPDX-License-Identifier: GPL-3.0-or-later
//! Restore the native store lineage and rekey face attributes on foreign restores.
use super::Obj;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
const LOCAL: u128 = (1 << 40) - 1;
fn module<'py>(py: Python<'py>) -> PyResult<Obj<'py>> {
    crate::model::module(py, "ingetrazo_core.store_mesh")
}
fn namespace(py: Python<'_>) -> PyResult<u64> {
    crate::model::module(py, "ingetrazo_core.mesh")?
        .getattr("_KEY_NAMESPACES")?
        .call_method0("__next__")?
        .extract()
}
fn empty_store(py: Python<'_>) -> PyResult<Obj<'_>> {
    let kw = module(py)?
        .call_method0("_weld_args")?
        .cast_into::<PyDict>()?;
    py.get_type::<crate::store::py::PyMeshStore>()
        .call((namespace(py)?,), Some(&kw))
}
fn owner<'py>(mesh: &Obj<'py>) -> PyResult<Obj<'py>> {
    mesh.getattr("__dict__")?
        .call_method1("get", ("_instance_definition", mesh))?
        .getattr("__dict__")
}
fn install_facade(mesh: &Obj<'_>) -> PyResult<()> {
    let api = module(mesh.py())?;
    if mesh.get_type().is(api.getattr("DeferredMesh")?) {
        mesh.setattr("__class__", api.getattr("StoreMesh")?)?;
    }
    Ok(())
}
#[pyfunction]
#[pyo3(signature=(mesh,store,table,own=None))]
pub(crate) fn become_store(
    mesh: &Obj<'_>,
    store: &Obj<'_>,
    table: &Obj<'_>,
    own: Option<&Obj<'_>>,
) -> PyResult<()> {
    let py = mesh.py();
    let d = mesh.getattr("__dict__")?.cast_into::<PyDict>()?;
    for k in ["vertices", "edges", "faces", "_registry", "_displaced"] {
        if d.contains(k)? {
            d.del_item(k)?;
        }
    }
    d.set_item("_store", store)?;
    d.set_item("_attr_table", table)?;
    if let Some(own) = own {
        d.set_item("_attr_own", own)?;
    } else {
        d.set_item("_attr_own", PyDict::new(py))?;
    }
    install_facade(mesh)
}
#[pyfunction]
pub(crate) fn initialise_store(mesh: &Obj<'_>) -> PyResult<()> {
    become_store(
        mesh,
        &empty_store(mesh.py())?,
        PyList::empty(mesh.py()).as_any(),
        None,
    )
}
#[pyfunction]
#[pyo3(signature=(mesh,snapshot=None))]
fn capture_state<'py>(mesh: &Obj<'py>, snapshot: Option<&Obj<'py>>) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let d = owner(mesh)?;
    let dict = py.get_type::<PyDict>();
    let own = PyDict::new(py);
    let deep = if snapshot.is_some() {
        Some(crate::model::module(py, "copy")?.getattr("deepcopy")?)
    } else {
        None
    };
    for (k, v) in d.get_item("_attr_own")?.cast::<PyDict>()?.iter() {
        let copy = if let Some(f) = &deep {
            if v.is_truthy()? {
                f.call1((v,))?
            } else {
                PyDict::new(py).into_any()
            }
        } else {
            dict.call1((v,))?
        };
        own.set_item(k, copy)?;
    }
    let snap = if let Some(s) = snapshot {
        s.clone()
    } else {
        mesh.getattr("_store")?.call_method0("snapshot")?
    };
    module(py)?.call_method1("StoreState", (snap, own, d.get_item("_attr_table")?, mesh))
}
#[pyfunction]
fn restore_state(mesh: &Obj<'_>, state: &Obj<'_>) -> PyResult<()> {
    let py = mesh.py();
    let api = module(py)?;
    if !state.is_instance(&api.getattr("StoreState")?)? {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "restore_state requires a native mesh snapshot",
        ));
    }
    let d = mesh.getattr("__dict__")?.cast_into::<PyDict>()?;
    let current = d.get_item("_store")?.filter(|v| !v.is_none());
    let same = state.getattr("mesh")?.is(mesh);
    let mut store = if same {
        Some(state.getattr("owner")?)
    } else {
        current.clone()
    };
    if same {
        if let (Some(now), Some(st)) = (&current, &store) {
            if now.getattr("lineage")?.eq(st.getattr("lineage")?)? {
                store = current;
            }
        }
    }
    let snap = state.getattr("snap")?;
    let foreign = match &store {
        None => true,
        Some(st) => !st.getattr("lineage")?.eq(snap.getattr("lineage")?)?,
    };
    let store = match store {
        Some(v) => v,
        None => empty_store(py)?,
    };
    d.set_item("_store", &store)?;
    let mut held = None;
    let ns = if foreign {
        let ns = namespace(py)?;
        let kw = PyDict::new(py);
        kw.set_item("namespace", ns)?;
        store.call_method("restore", (&snap,), Some(&kw))?;
        Some(ns)
    } else {
        held = owner(mesh)?
            .call_method1("get", ("_attr_own",))?
            .extract::<Option<Obj>>()?;
        store.call_method1("restore", (&snap,))?;
        None
    };
    let own = PyDict::new(py);
    let deep = crate::model::module(py, "copy")?.getattr("deepcopy")?;
    for (k, v) in state.getattr("own")?.cast::<PyDict>()?.iter() {
        let val = if v.is_truthy()? {
            deep.call1((v,))?
        } else {
            PyDict::new(py).into_any()
        };
        if let Some(ns) = ns {
            own.set_item((u128::from(ns) << 40) | (k.extract::<u128>()? & LOCAL), val)?;
        } else {
            own.set_item(k, val)?;
        }
    }
    if let Some(held) = held {
        for (k, v) in held.cast::<PyDict>()?.iter() {
            if !own.contains(&k)?
                && !snap.call_method1("has", (&k,))?.is_truthy()?
                && store.call_method1("knows", (&k,))?.is_truthy()?
            {
                own.set_item(k, v)?;
            }
        }
    }
    let table = state.getattr("table")?;
    d.set_item("_attr_table", &table)?;
    d.set_item("_attr_own", &own)?;
    if let Some(def) = d.get_item("_instance_definition")?.filter(|v| !v.is_none()) {
        def.setattr("_attr_table", table)?;
        def.setattr("_attr_own", own)?;
    }
    for k in [
        "vertices",
        "edges",
        "faces",
        "_registry",
        "_displaced",
        "_bulk",
    ] {
        if d.contains(k)? {
            d.del_item(k)?;
        }
    }
    install_facade(mesh)?;
    mesh.setattr("_chunk_dirty", true)?;
    mesh.setattr("_mut_serial", mesh.getattr("_mut_serial")?.add(1)?)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(become_store, m)?)?;
    m.add_function(wrap_pyfunction!(initialise_store, m)?)?;
    m.add_function(wrap_pyfunction!(capture_state, m)?)?;
    m.add_function(wrap_pyfunction!(restore_state, m)?)?;
    Ok(())
}
