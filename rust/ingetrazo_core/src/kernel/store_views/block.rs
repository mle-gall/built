// SPDX-License-Identifier: GPL-3.0-or-later
//! Decode a deferred document block, applying link hooks once per attribute row.
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

struct Labels<'py> {
    table: Bound<'py, PyList>,
    attr: Obj<'py>,
    names: Bound<'py, PyList>,
    layers: Obj<'py>,
}

impl<'py> Labels<'py> {
    fn relabel(
        self,
        hooks: &Obj<'py>,
        n_edges: usize,
        np: &Bound<'py, PyModule>,
    ) -> PyResult<Self> {
        let py = np.py();
        let namespace = py.import("types")?.getattr("SimpleNamespace")?;
        let standin = namespace.call0()?;
        let faces = PyList::empty(py);
        let tags = PyList::empty(py);
        for attrs in self
            .table
            .iter()
            .chain(std::iter::once(PyDict::new(py).into_any()))
        {
            let face = namespace.call0()?;
            face.setattr("attrs", py.get_type::<PyDict>().call1((attrs,))?)?;
            faces.append(face)?;
        }
        for name in self
            .names
            .iter()
            .chain(std::iter::once(py.None().into_bound(py)))
        {
            let edge = namespace.call0()?;
            edge.setattr("layer", name)?;
            tags.append(edge)?;
        }
        standin.setattr("faces", &faces)?;
        standin.setattr("edges", &tags)?;
        for hook in hooks.try_iter()? {
            hook?.call1((&standin,))?;
        }
        let table = PyList::empty(py);
        for face in &faces {
            table.append(face.getattr("attrs")?)?;
        }
        let attr = np.call_method1(
            "where",
            (
                self.attr.call_method1("__lt__", (0,))?,
                table.len() - 1,
                &self.attr,
            ),
        )?;
        let ix = PyDict::new(py);
        let names = PyList::empty(py);
        let remap = PyList::empty(py);
        for tag in &tags {
            let layer = tag.getattr("layer")?;
            if layer.is_none() {
                remap.append(-1)?;
            } else {
                let index = if let Some(v) = ix.get_item(&layer)? {
                    v.extract()?
                } else {
                    let i = names.len();
                    ix.set_item(&layer, i)?;
                    names.append(layer)?;
                    i
                };
                remap.append(index)?;
            }
        }
        let int = np.getattr("int64")?;
        let layers = if self.layers.is_none() {
            np.call_method1("full", (n_edges, -1, &int))?
        } else {
            self.layers
        };
        let indices = np.call_method1(
            "where",
            (
                layers.call_method1("__lt__", (0,))?,
                tags.len() - 1,
                &layers,
            ),
        )?;
        let layers = np
            .call_method1("asarray", (remap, int))?
            .get_item(indices)?;
        Ok(Self {
            table,
            attr,
            names,
            layers,
        })
    }
}
#[pyfunction]
#[pyo3(signature=(src,hooks=None,*,precise=false))]
pub(crate) fn store_from_source<'py>(
    src: &Obj<'py>,
    hooks: Option<&Obj<'py>>,
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
    let labels = Labels {
        table,
        attr,
        names: rows(&meta, "edge_layers")?,
        layers: optional("layers")?,
    };
    let labels = match hooks {
        Some(h) if h.is_truthy()? => labels.relabel(h, edges.len()?, &np)?,
        _ => labels,
    };
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
            labels.layers,
            labels.names,
            indices(&src.get_item("rings")?)?,
            indices(&src.get_item("sizes")?)?,
            counts,
            labels.attr,
        ),
        Some(&kw),
    )?;
    Ok((store, labels.table))
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(store_from_source, m)?)?;
    Ok(())
}
