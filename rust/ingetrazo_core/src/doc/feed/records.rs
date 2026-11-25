// SPDX-License-Identifier: GPL-3.0-or-later
//! JSON wire records. Element identifiers travel as decimal strings.
use crate::protocol::keys::to_json_key;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet, PyTuple},
};
type Obj<'py> = Bound<'py, PyAny>;
fn json_ids<'py>(seq: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let out = PyList::empty(seq.py());
    for item in seq.try_iter()? {
        out.append(to_json_key(&item?)?)?;
    }
    Ok(out)
}
fn fields<'py>(record: &Obj<'py>, names: &[&str]) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(record.py());
    for name in names {
        d.set_item(name, record.getattr(*name)?)?;
    }
    Ok(d)
}
#[pyclass(subclass, module = "ingetrazo_core")]
pub(crate) struct FeedMeshRecord;
#[pymethods]
impl FeedMeshRecord {
    #[new]
    #[pyo3(signature=(*_args,**_kw))]
    fn new(_args: &Bound<'_, PyTuple>, _kw: Option<&Bound<'_, PyDict>>) -> Self {
        Self
    }
    #[getter]
    fn mesh<'py>(slf: &Bound<'py, Self>) -> PyResult<Obj<'py>> {
        slf.getattr("_ref")?.call0()
    }
    fn empty(slf: &Bound<'_, Self>) -> PyResult<bool> {
        for field in ["replaced", "moved"] {
            if slf.getattr(field)?.is_truthy()? {
                return Ok(false);
            }
        }
        for name in ["created", "removed", "modified"] {
            for item in slf.getattr(name)?.call_method0("values")?.try_iter()? {
                if item?.is_truthy()? {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
    fn faces_touched<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PySet>> {
        let out = PySet::empty(slf.py())?;
        for name in ["created", "modified"] {
            for key in slf.getattr(name)?.get_item("face")?.try_iter()? {
                out.add(key?)?;
            }
        }
        Ok(out)
    }
    fn to_json<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyDict>> {
        let out = fields(slf.as_any(), &["backing", "replaced", "revision"])?;
        out.set_item("mesh", slf.getattr("key")?)?;
        for name in ["created", "removed", "modified"] {
            let d = PyDict::new(slf.py());
            for (k, v) in slf.getattr(name)?.cast::<PyDict>()?.iter() {
                d.set_item(k, json_ids(&v)?)?;
            }
            out.set_item(name, d)?;
        }
        out.set_item("moved", json_ids(&slf.getattr("moved")?)?)?;
        Ok(out)
    }
}
#[pyclass(subclass, module = "ingetrazo_core")]
pub(crate) struct FeedStepRecord;
#[pymethods]
impl FeedStepRecord {
    #[new]
    #[pyo3(signature=(*_args,**_kw))]
    fn new(_args: &Bound<'_, PyTuple>, _kw: Option<&Bound<'_, PyDict>>) -> Self {
        Self
    }
    fn mesh_change<'py>(slf: &Bound<'py, Self>, mesh: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
        for v in slf.getattr("meshes")?.try_iter()? {
            let v = v?;
            if v.getattr("mesh")?.is(mesh) {
                return Ok(Some(v));
            }
        }
        Ok(None)
    }
    fn to_json<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyDict>> {
        let py = slf.py();
        let out = fields(slf.as_any(), &["revision", "phase", "command", "complete"])?;
        let meshes = PyList::empty(py);
        for m in slf.getattr("meshes")?.try_iter()? {
            meshes.append(m?.call_method0("to_json")?)?;
        }
        out.set_item("meshes", meshes)?;
        for name in ["placements", "tags", "materials"] {
            let d = PyDict::new(py);
            for (k, v) in slf.getattr(name)?.cast::<PyDict>()?.iter() {
                d.set_item(k, py.get_type::<PyList>().call1((v,))?)?;
            }
            out.set_item(name, d)?;
        }
        let copies = PyList::empty(py);
        for c in slf.getattr("copies")?.try_iter()? {
            let c = c?;
            let row = PyDict::new(py);
            for field in ["source", "target"] {
                row.set_item(field, c.get_item(field)?)?;
            }
            let ks = PyDict::new(py);
            for (kind, pairs) in c.get_item("keys")?.cast::<PyDict>()?.iter() {
                let list = PyList::empty(py);
                for pair in pairs.try_iter()? {
                    list.append(json_ids(&pair?)?)?;
                }
                ks.set_item(kind, list)?;
            }
            row.set_item("keys", ks)?;
            copies.append(row)?;
        }
        out.set_item("copies", copies)?;
        Ok(out)
    }
}
