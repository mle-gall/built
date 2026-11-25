// SPDX-License-Identifier: GPL-3.0-or-later
//! Session mesh identities are weak in both directions. Occurrence views use
//! their shared definition's key. All mutation of the maps occurs under the GIL.
use super::*;
use pyo3::types::{PyInt, PyString};
const KINDS: [&str; 3] = ["vertex", "edge", "face"];
struct Registry {
    by_mesh: Py<PyAny>,
    by_key: Py<PyAny>,
}
static REGISTRY: pyo3::sync::PyOnceLock<Registry> = pyo3::sync::PyOnceLock::new();
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
fn registry(py: Python<'_>) -> PyResult<&Registry> {
    REGISTRY.get_or_try_init(py, || {
        let weak = py.import("weakref")?;
        Ok(Registry {
            by_mesh: weak.call_method0("WeakKeyDictionary")?.unbind(),
            by_key: weak.call_method0("WeakValueDictionary")?.unbind(),
        })
    })
}
#[pyfunction]
pub(crate) fn mesh_key<'py>(mesh: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let owner = mesh
        .getattr_opt("_instance_definition")?
        .unwrap_or_else(|| mesh.clone());
    let reg = registry(py)?;
    let by_mesh = reg.by_mesh.bind(py);
    let k = by_mesh.call_method1("get", (&owner,))?;
    if !k.is_none() {
        return Ok(k);
    }
    let k = format!(
        "m{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    by_mesh.set_item(&owner, &k)?;
    reg.by_key.bind(py).set_item(&k, owner)?;
    Ok(PyString::new(py, &k).into_any())
}
#[pyfunction]
fn known_mesh<'py>(key: &Obj<'py>) -> PyResult<Obj<'py>> {
    registry(key.py())?
        .by_key
        .bind(key.py())
        .call_method1("get", (key,))
}
#[pyfunction]
#[pyo3(signature=(scene,r#ref=None))]
pub(crate) fn resolve_mesh<'py>(scene: &Obj<'py>, r#ref: Option<&Obj<'py>>) -> PyResult<Obj<'py>> {
    let Some(key) = r#ref else {
        return scene.getattr("mesh");
    };
    if key.eq("active")? {
        return scene.getattr("mesh");
    }
    if key.eq("loose")? {
        return Ok(scene
            .getattr_opt("loose_mesh")?
            .unwrap_or(scene.getattr("mesh")?));
    }
    if key.is_instance_of::<PyTuple>() || key.is_instance_of::<PyList>() {
        let g = scene.call_method1("resolve_occurrence", (key,))?;
        if !g.is_none() {
            return g.getattr("mesh");
        }
    }
    if key.is_instance_of::<PyString>() {
        let m = known_mesh(key)?;
        if !m.is_none() {
            return Ok(m);
        }
        let placements = scene
            .py()
            .import("core.group")?
            .getattr("iter_placements")?;
        for root in scene.getattr("groups")?.try_iter()? {
            for pair in placements.call1((root?,))?.try_iter()? {
                let g = pair?.get_item(0)?;
                if g.getattr("uid")?.eq(key)? {
                    return g.getattr("mesh");
                }
            }
        }
    }
    Err(PyKeyError::new_err(key.clone().unbind()))
}
#[pyfunction]
pub(crate) fn kind_of(element: &Obj<'_>) -> PyResult<Option<&'static str>> {
    let module = element.py().import("built_core.mesh")?;
    for (cls, kind) in [("Face", "face"), ("Edge", "edge"), ("Vertex", "vertex")] {
        if element.is_instance(&module.getattr(cls)?)? {
            return Ok(Some(kind));
        }
    }
    Ok(None)
}
#[pyfunction]
pub(crate) fn store_of<'py>(mesh: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    Ok(mesh
        .getattr("__dict__")?
        .cast::<PyDict>()?
        .get_item("_store")?
        .filter(|s| !s.is_none()))
}
fn kind_id(kind: &str) -> PyResult<usize> {
    KINDS
        .iter()
        .position(|k| *k == kind)
        .ok_or_else(|| PyValueError::new_err("kind must be one of ('vertex', 'edge', 'face')"))
}
fn collection(kind: usize) -> &'static str {
    ["vertices", "edges", "faces"][kind]
}
fn from_store<'py>(
    m: &Obj<'py>,
    st: &Obj<'py>,
    kind: usize,
    k: &Obj<'py>,
) -> PyResult<Option<Obj<'py>>> {
    let Ok(value) = k.extract::<u128>() else {
        return Ok(None);
    };
    if ((value >> 38) & 3) as usize != kind || !st.call_method1("is_live", (k,))?.is_truthy()? {
        return Ok(None);
    }
    Ok(Some(m.py().import("built_core.store_mesh")?.call_method1(
        ["VertexView", "EdgeView", "FaceView"][kind],
        (m, k),
    )?))
}
#[pyfunction]
pub(crate) fn element<'py>(
    mesh: &Obj<'py>,
    kind: &str,
    key: &Obj<'py>,
) -> PyResult<Option<Obj<'py>>> {
    let kind = kind_id(kind)?;
    if let Some(st) = store_of(mesh)? {
        return from_store(mesh, &st, kind, key);
    }
    for item in mesh.getattr(collection(kind))?.try_iter()? {
        let item = item?;
        if item.getattr("key")?.eq(key)? {
            return Ok(Some(item));
        }
    }
    Ok(None)
}
#[pyfunction]
pub(crate) fn elements<'py>(
    mesh: &Obj<'py>,
    kind: &str,
    keys: &Obj<'py>,
) -> PyResult<Bound<'py, PyDict>> {
    let py = mesh.py();
    let int = py.get_type::<PyInt>();
    let ks = keys
        .try_iter()?
        .map(|k| int.call1((k?,)))
        .collect::<PyResult<Vec<_>>>()?;
    let out = PyDict::new(py);
    let kind = kind_id(kind)?;
    if let Some(st) = store_of(mesh)? {
        for k in ks {
            if let Some(v) = from_store(mesh, &st, kind, &k)? {
                out.set_item(k, v)?;
            }
        }
    } else {
        let wanted = pyo3::types::PySet::new(py, ks)?;
        for v in mesh.getattr(collection(kind))?.try_iter()? {
            let v = v?;
            let k = v.getattr("key")?;
            if wanted.contains(&k)? {
                out.set_item(k, v)?;
            }
        }
    }
    Ok(out)
}
#[pyfunction]
pub(crate) fn to_json_key<'py>(key: &Obj<'py>) -> PyResult<Bound<'py, PyString>> {
    key.py().get_type::<PyInt>().call1((key,))?.str()
}
#[pyfunction]
pub(crate) fn from_json_key<'py>(value: &Obj<'py>) -> PyResult<Obj<'py>> {
    if value.is_exact_instance_of::<PyInt>() {
        return Ok(value.clone());
    }
    if value.is_instance_of::<PyString>()
        && value
            .call_method1("lstrip", ("-",))?
            .call_method0("isdigit")?
            .is_truthy()?
    {
        return value.py().get_type::<PyInt>().call1((value,));
    }
    Err(PyValueError::new_err(format!(
        "element key {}: a decimal string",
        value.repr()?
    )))
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(mesh_key, m)?)?;
    m.add_function(wrap_pyfunction!(known_mesh, m)?)?;
    m.add_function(wrap_pyfunction!(resolve_mesh, m)?)?;
    m.add_function(wrap_pyfunction!(kind_of, m)?)?;
    m.add_function(wrap_pyfunction!(store_of, m)?)?;
    m.add_function(wrap_pyfunction!(element, m)?)?;
    m.add_function(wrap_pyfunction!(elements, m)?)?;
    m.add_function(wrap_pyfunction!(to_json_key, m)?)?;
    m.add_function(wrap_pyfunction!(from_json_key, m)?)?;
    Ok(())
}
