// SPDX-License-Identifier: GPL-3.0-or-later
//! Lazily resolve columnar snap candidates into records only for rules that inspect them.
use pyo3::{
    exceptions::PyIndexError,
    prelude::*,
    types::{PyDict, PyFloat, PyInt, PyList, PySlice, PyTuple},
};
type Obj<'p> = Bound<'p, PyAny>;
const COLUMNS: [&str; 5] = ["rows", "ids", "a", "b", "distances"];
fn attrs<'p>(v: &Obj<'p>) -> PyResult<(Obj<'p>, Obj<'p>, Obj<'p>)> {
    Ok((
        v.getattr("prefix")?,
        v.getattr("rows")?,
        v.getattr("features")?,
    ))
}
#[pyclass(module = "built_core", dict, skip_from_py_object)]
pub(super) struct ResidentEdges {
    #[pyo3(get, set)]
    rows: Py<PyAny>,
    #[pyo3(get, set)]
    ids: Py<PyAny>,
    #[pyo3(get, set)]
    a: Py<PyAny>,
    #[pyo3(get, set)]
    b: Py<PyAny>,
    #[pyo3(get, set)]
    distances: Py<PyAny>,
    #[pyo3(get, set)]
    groups: Py<PyAny>,
    #[pyo3(get, set)]
    context: Py<PyAny>,
    #[pyo3(get, set)]
    prefix: Py<PyAny>,
    #[pyo3(get, set)]
    features: Py<PyAny>,
}
#[pymethods]
impl ResidentEdges {
    #[new]
    fn new(columns: &Obj<'_>, groups: Py<PyAny>, context: Py<PyAny>) -> PyResult<Self> {
        let [rows, ids, a, b, distances] = columns.extract::<[Py<PyAny>; 5]>()?;
        let empty = || PyList::empty(columns.py()).into_any().unbind();
        Ok(Self {
            rows,
            ids,
            a,
            b,
            distances,
            groups,
            context,
            prefix: empty(),
            features: empty(),
        })
    }
    fn __len__(slf: &Bound<'_, Self>) -> PyResult<usize> {
        let (p, r, f) = attrs(slf.as_any())?;
        Ok(p.len()? + r.len()? + f.len()?)
    }
    fn __getitem__<'p>(slf: &Bound<'p, Self>, index: &Obj<'p>) -> PyResult<Obj<'p>> {
        let py = slf.py();
        let n = Self::__len__(slf)? as isize;
        if let Ok(slice) = index.cast::<PySlice>() {
            let s = slice.indices(n)?;
            let list = PyList::empty(py);
            for k in 0..s.slicelength {
                list.append(Self::__getitem__(
                    slf,
                    &(s.start + k as isize * s.step)
                        .into_pyobject(py)?
                        .into_any(),
                )?)?;
            }
            return Ok(list.into_any());
        }
        let mut i = index.extract::<isize>()?;
        if i < 0 {
            i += n;
        }
        if i < 0 || i >= n {
            return Err(PyIndexError::new_err(i));
        }
        let (prefix, rows, features) = attrs(slf.as_any())?;
        let mut i = i as usize;
        if i < prefix.len()? {
            return prefix.get_item(i);
        }
        i -= prefix.len()?;
        if i >= rows.len()? {
            return features.get_item(i - rows.len()?);
        }
        let row = py.get_type::<PyInt>().call1((rows.get_item(i)?,))?;
        let groups = slf.getattr("groups")?;
        let group = if groups.is_none() {
            groups
        } else {
            groups.get_item(&row)?
        };
        let loose = group.is(slf.getattr("context")?);
        let kw = PyDict::new(py);
        for name in ["a", "b"] {
            let p = slf.getattr(name)?.get_item(i)?.extract::<[f64; 3]>()?;
            kw.set_item(name, crate::math::Vec3::from(glam::DVec3::from_array(p)))?;
        }
        kw.set_item("row", row)?;
        kw.set_item("group", &group)?;
        kw.set_item("in_group", !loose)?;
        kw.set_item(
            "context",
            if loose {
                None
            } else if !group.is_none() && group.call_method0("is_instance")?.is_truthy()? {
                Some("component")
            } else {
                Some("group")
            },
        )?;
        kw.set_item(
            "edge_id",
            py.get_type::<PyInt>()
                .call1((slf.getattr("ids")?.get_item(i)?,))?,
        )?;
        kw.set_item(
            "distance",
            py.get_type::<PyFloat>()
                .call1((slf.getattr("distances")?.get_item(i)?,))?,
        )?;
        py.import("types")?
            .getattr("SimpleNamespace")?
            .call((), Some(&kw))
    }
    fn keep<'p>(slf: &Bound<'p, Self>, mask: &Obj<'p>) -> PyResult<Obj<'p>> {
        let py = slf.py();
        let mask = py
            .import("numpy")?
            .call_method1("asarray", (mask, "bool"))?;
        let cols = PyList::empty(py);
        for name in COLUMNS {
            cols.append(slf.getattr(name)?.get_item(&mask)?)?;
        }
        let result =
            slf.get_type()
                .call1((cols, slf.getattr("groups")?, slf.getattr("context")?))?;
        for name in ["prefix", "features"] {
            result.setattr(name, slf.getattr(name)?)?;
        }
        Ok(result)
    }
    fn in_group<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        let py = slf.py();
        let groups = slf.getattr("groups")?;
        let context = slf.getattr("context")?;
        let rows = slf.getattr("rows")?;
        let np = py.import("numpy")?;
        if groups.is_none() {
            return np.call_method1("full", (rows.len()?, !context.is_none()));
        }
        let mut flags = Vec::with_capacity(rows.len()?);
        for row in rows.try_iter()? {
            flags.push(!groups.get_item(row?)?.is(&context));
        }
        np.call_method1("asarray", (flags, "bool"))
    }
    fn snap_columns<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        let py = slf.py();
        let (prefix, rows, features) = attrs(slf.as_any())?;
        let grouped = Self::in_group(slf)?.call_method0("tolist")?;
        let tags = PyList::new(py, vec![0; rows.len()?])?;
        let a = slf.getattr("a")?;
        let b = slf.getattr("b")?;
        if prefix.is_empty()? && features.is_empty()? {
            return Ok((a, b, tags, grouped).into_pyobject(py)?.into_any());
        }
        let cols = py.import("core.snap_query")?.getattr("edge_columns")?;
        let first = cols.call1((prefix,))?;
        let last = cols.call1((features,))?;
        let concat = py.import("numpy")?.getattr("concatenate")?;
        let mut out = Vec::new();
        for (i, v) in [a, b, tags.into_any(), grouped].into_iter().enumerate() {
            let f = first.get_item(i)?;
            let l = last.get_item(i)?;
            out.push(if i < 2 {
                concat.call1(((f, v, l),))?
            } else {
                f.add(v)?.add(l)?
            });
        }
        Ok(PyTuple::new(py, out)?.into_any())
    }
}
