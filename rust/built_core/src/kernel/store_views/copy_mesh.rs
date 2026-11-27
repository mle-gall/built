// SPDX-License-Identifier: GPL-3.0-or-later
//! Copy or archive a store without serialising transient view caches.
use super::{lifecycle::become_store, Obj};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList},
};
const LOCAL: u128 = (1u128 << 40) - 1;
fn api(py: Python<'_>) -> PyResult<Obj<'_>> {
    crate::model::module(py, "built_core.store_mesh")
}
fn namespace(py: Python<'_>) -> PyResult<u64> {
    crate::model::module(py, "built_core.mesh")?
        .getattr("_KEY_NAMESPACES")?
        .call_method0("__next__")?
        .extract()
}
#[pyfunction]
#[pyo3(signature=(mesh,matrix,origin,for_edit=false))]
pub(super) fn instance_view<'py>(
    mesh: &Obj<'py>,
    matrix: &Obj<'py>,
    origin: &Obj<'py>,
    for_edit: bool,
) -> PyResult<Option<Obj<'py>>> {
    let py = mesh.py();
    let module = api(py)?;
    if mesh.is_instance(&module.getattr("DeferredMesh")?)? {
        mesh.call_method0("_materialize")?;
    }
    let class = module.getattr("StoreMesh")?;
    if !mesh.get_type().is(&class) {
        return Ok(None);
    }
    let view = match mesh
        .getattr("_store")?
        .call_method1("_instance_view", (mesh, matrix, origin, for_edit))
    {
        Ok(v) => v,
        Err(e) if e.is_instance_of::<PyValueError>(py) => return Ok(None),
        Err(e) => return Err(e),
    };
    let result = class.call0()?;
    become_store(
        &result,
        &view,
        &mesh.getattr("_attr_table")?,
        Some(&mesh.getattr("_attr_own")?),
    )?;
    result.setattr("_instance_definition", mesh)?;
    if for_edit {
        result.setattr("_instance_edit", true)?;
    }
    Ok(Some(result))
}
fn allocate<'py>(class: &Obj<'py>) -> PyResult<Obj<'py>> {
    crate::model::module(class.py(), "builtins")?
        .getattr("object")?
        .call_method1("__new__", (class,))
}
#[pyfunction]
fn unpickle_mesh<'py>(
    version: &Obj<'py>,
    archive: &Obj<'py>,
    cls: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = cls.py();
    let mesh = allocate(cls)?;
    let store = py
        .get_type::<crate::store::py::PyMeshStore>()
        .call_method1("from_pickle_archive", (version, archive, namespace(py)?))?;
    become_store(&mesh, &store, PyList::empty(py).as_any(), None)?;
    Ok(mesh)
}
fn extra<'py>(mesh: &Obj<'py>, skip_views: bool) -> PyResult<Bound<'py, PyDict>> {
    let py = mesh.py();
    let side = api(py)?.getattr("STORE_SIDE")?;
    let out = PyDict::new(py);
    for (k, v) in mesh.getattr("__dict__")?.cast::<PyDict>()?.iter() {
        if side.contains(&k)? || k.eq("_keys")? {
            continue;
        }
        if skip_views
            && k.extract::<String>().ok().is_some_and(|s| {
                matches!(
                    s.as_str(),
                    "vertices" | "edges" | "faces" | "_registry" | "_displaced"
                )
            })
        {
            continue;
        }
        out.set_item(k, v)?;
    }
    Ok(out)
}
#[pyfunction]
fn deepcopy_mesh<'py>(mesh: &Obj<'py>, memo: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let d = mesh.getattr("__dict__")?;
    let store = mesh.getattr("_store")?;
    let deep = crate::model::module(py, "copy")?.getattr("deepcopy")?;
    let def = d.call_method1("get", ("_instance_definition",))?;
    let addr = mesh.as_ptr() as usize;
    if !def.is_none() {
        let copy = deep.call1((def, memo))?;
        let copy = instance_view(
            &copy,
            &store.getattr("frame_matrix")?,
            &store.getattr("frame_origin")?,
            d.contains("_instance_edit")?,
        )?
        .unwrap_or_else(|| py.None().into_bound(py));
        memo.set_item(addr, &copy)?;
        return Ok(copy);
    }
    let new = allocate(mesh.get_type().as_any())?;
    memo.set_item(addr, &new)?;
    let nd = new.getattr("__dict__")?;
    nd.call_method1("update", (deep.call1((extra(mesh, true)?, memo))?,))?;
    let ns = namespace(py)?;
    nd.set_item("_store", store.call_method1("copy", (ns,))?)?;
    nd.set_item("_attr_table", d.get_item("_attr_table")?)?;
    let own = PyDict::new(py);
    for (k, v) in d.get_item("_attr_own")?.cast::<PyDict>()?.iter() {
        own.set_item(
            (u128::from(ns) << 40) | (k.extract::<u128>()? & LOCAL),
            deep.call1((v, memo))?,
        )?;
    }
    nd.set_item("_attr_own", own)?;
    Ok(new)
}
#[pyfunction]
fn reduce_mesh<'py>(mesh: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let api = api(py)?;
    let store = mesh.getattr("_store")?;
    let def = mesh
        .getattr("__dict__")?
        .call_method1("get", ("_instance_definition",))?;
    if !def.is_none() {
        return Ok((
            api.getattr("instance_view")?,
            (
                def,
                store.getattr("frame_matrix")?,
                store.getattr("frame_origin")?,
            ),
        )
            .into_pyobject(py)?
            .into_any());
    }
    let attrs = PyDict::new(py);
    for (k, v) in mesh.getattr("_attr_own")?.cast::<PyDict>()?.iter() {
        attrs.set_item(k.extract::<u128>()? & LOCAL, v)?;
    }
    Ok((
        api.getattr("_unpickle_mesh")?,
        (1, store.call_method0("pickle_archive")?, mesh.get_type()),
        (extra(mesh, false)?, mesh.getattr("_attr_table")?, attrs),
    )
        .into_pyobject(py)?
        .into_any())
}
#[pyfunction]
fn set_mesh_state(mesh: &Obj<'_>, saved: &Obj<'_>) -> PyResult<()> {
    let state = saved.get_item(0)?;
    mesh.getattr("__dict__")?.call_method1("update", (state,))?;
    mesh.setattr("_attr_table", saved.get_item(1)?)?;
    let attrs = PyDict::new(mesh.py());
    let base = mesh
        .getattr("_store")?
        .getattr("key_base")?
        .extract::<u128>()?;
    for (k, v) in saved.get_item(2)?.cast::<PyDict>()?.iter() {
        attrs.set_item(base | k.extract::<u128>()?, v)?;
    }
    mesh.setattr("_attr_own", attrs)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(instance_view, m)?)?;
    m.add_function(wrap_pyfunction!(unpickle_mesh, m)?)?;
    m.add_function(wrap_pyfunction!(deepcopy_mesh, m)?)?;
    m.add_function(wrap_pyfunction!(reduce_mesh, m)?)?;
    m.add_function(wrap_pyfunction!(set_mesh_state, m)?)?;
    Ok(())
}
