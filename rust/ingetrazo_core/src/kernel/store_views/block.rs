// SPDX-License-Identifier: GPL-3.0-or-later
//! Decode a deferred document block into indexed geometry and its attribute table.
use super::Obj;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

fn rows<'py>(meta: &Obj<'py>, key: &str) -> PyResult<Bound<'py, PyList>> {
    let value = meta.call_method1("get", (key,))?;
    let py = meta.py();
    if !value.is_truthy()? {
        return Ok(PyList::empty(py));
    }
    Ok(py.get_type::<PyList>().call1((value,))?.cast_into()?)
}

#[pyfunction]
#[pyo3(signature=(src,*,precise=false))]
pub(crate) fn store_from_source<'py>(
    src: &Obj<'py>,
    precise: bool,
) -> PyResult<(Obj<'py>, Bound<'py, PyList>)> {
    let py = src.py();
    let kernel = py.import("ingetrazo_core")?;
    let np = py.import("numpy")?;
    let mut meta = src.call_method1("get", ("meta",))?;
    if !meta.is_truthy()? {
        meta = PyDict::new(py).into_any();
    }
    let table = PyList::empty(py);
    let decode = py.import("formats.igz")?.getattr("_face_attrs_from_json")?;
    for raw in rows(&meta, "face_attrs")?.iter() {
        let row = decode.call1((raw,))?;
        table.append(if row.is_truthy()? {
            row
        } else {
            PyDict::new(py).into_any()
        })?;
    }
    let direct = !kernel.call_method0("document_native_io")?.is_none();
    let i64t = np.getattr("int64")?;
    let i32t = np.getattr("int32")?;
    let indices = |value: &Obj<'py>| -> PyResult<Obj<'py>> {
        let a = np.call_method1("asarray", (value,))?;
        let dtype = a.getattr("dtype")?;
        let t = if direct && (dtype.eq(&i32t)? || dtype.eq(&i64t)?) {
            &dtype
        } else {
            &i64t
        };
        np.call_method1("ascontiguousarray", (a, t))
    };
    let optional = |key: &str| -> PyResult<Obj<'py>> {
        let v = src.call_method1("get", (key,))?;
        if v.is_none() {
            Ok(v)
        } else {
            indices(&v)
        }
    };
    let counts = indices(&src.get_item("counts")?)?;
    let fa = optional("face_attr")?;
    let attr = if fa.is_none() {
        np.call_method1("full", (counts.len()?, -1, &i64t))?
    } else {
        fa
    };
    let edges = indices(&src.get_item("edges")?)?.call_method1("reshape", (-1, 2))?;
    let points = np.call_method1("asarray", (src.get_item("pos")?,))?;
    let t = if !precise && points.getattr("dtype")?.eq(np.getattr("float32")?)? {
        "float32"
    } else {
        "float64"
    };
    let pos = np
        .call_method1("ascontiguousarray", (points, np.getattr(t)?))?
        .call_method1("reshape", (-1, 3))?;
    let flags = src.call_method1("get", ("flags",))?;
    let flags = if flags.is_none() {
        flags
    } else {
        np.call_method1("ascontiguousarray", (flags, np.getattr("uint8")?))?
    };
    let ns = py
        .import("core.mesh")?
        .getattr("_KEY_NAMESPACES")?
        .call_method0("__next__")?;
    let kw = py
        .import("core.store_mesh")?
        .call_method0("_weld_args")?
        .cast_into::<PyDict>()?;
    kw.set_item(
        "sequential",
        src.call_method1("get", ("sequential", false))?
            .is_truthy()?,
    )?;
    let store = kernel.getattr("MeshStore")?.call_method(
        "from_block",
        (
            ns,
            pos,
            edges,
            flags,
            optional("curves")?,
            optional("layers")?,
            rows(&meta, "edge_layers")?,
            indices(&src.get_item("rings")?)?,
            indices(&src.get_item("sizes")?)?,
            counts,
            attr,
        ),
        Some(&kw),
    )?;
    Ok((store, table))
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(store_from_source, m)?)?;
    Ok(())
}
