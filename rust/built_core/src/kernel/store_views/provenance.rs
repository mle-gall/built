// SPDX-License-Identifier: GPL-3.0-or-later
//! Lazy store key views and weak provenance for split and explode.
use pyo3::{
    prelude::*,
    sync::PyOnceLock,
    types::{PyDict, PyIterator, PyList, PySlice},
};
type Obj<'py> = Bound<'py, PyAny>;
static ORIGINS: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
fn origins(py: Python<'_>) -> PyResult<&Obj<'_>> {
    Ok(ORIGINS
        .get_or_try_init(py, || {
            crate::model::module(py, "weakref")?
                .call_method0("WeakKeyDictionary")
                .map(Bound::unbind)
        })?
        .bind(py))
}
pub(crate) fn remember(new: &Obj<'_>, source: &Obj<'_>, keys: CopiedKeys) -> PyResult<()> {
    let py = new.py();
    let source = crate::model::module(py, "weakref")?.call_method1("ref", (source,))?;
    origins(py)?.set_item(new, (source, Py::new(py, keys)?))
}
pub(crate) fn origin_of<'py>(mesh: &Obj<'py>) -> PyResult<(Option<Obj<'py>>, Option<Obj<'py>>)> {
    let got = origins(mesh.py())?.call_method1("get", (mesh,))?;
    if got.is_none() {
        return Ok((None, None));
    }
    let source = got.get_item(0)?.call0()?;
    Ok(if source.is_none() {
        (None, None)
    } else {
        (Some(source), Some(got.get_item(1)?))
    })
}
#[pyclass(sequence, module = "built_core", skip_from_py_object)]
pub(crate) struct PieceElements {
    pub(super) mesh: Py<PyAny>,
    pub(super) keys: Py<PyAny>,
    pub(super) class: Py<PyAny>,
}
impl PieceElements {
    pub(super) fn new(mesh: &Obj<'_>, keys: Obj<'_>, class: &Obj<'_>) -> Self {
        Self {
            mesh: mesh.clone().unbind(),
            keys: keys.unbind(),
            class: class.clone().unbind(),
        }
    }
}
#[pymethods]
impl PieceElements {
    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        self.keys.bind(py).len()
    }
    fn __getitem__<'py>(&self, py: Python<'py>, index: &Obj<'py>) -> PyResult<Obj<'py>> {
        let k = self.keys.bind(py).get_item(index)?;
        let view = |key: &Obj<'py>| self.class.bind(py).call1((self.mesh.bind(py), key));
        if index.is_instance_of::<PySlice>() {
            let out = PyList::empty(py);
            for key in k.try_iter()? {
                out.append(view(&key?)?)?;
            }
            return Ok(out.into_any());
        }
        view(&k)
    }
    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Obj<'py>> {
        let repeated =
            crate::model::module(py, "itertools")?.call_method1("repeat", (&self.mesh,))?;
        crate::model::module(py, "builtins")?
            .call_method1("map", (&self.class, repeated, &self.keys))
    }
    fn count(slf: &Bound<'_, Self>, value: &Obj<'_>) -> PyResult<usize> {
        crate::model::module(slf.py(), "collections.abc")?
            .getattr("Sequence")?
            .call_method1("count", (slf, value))?
            .extract()
    }
    #[pyo3(signature=(value,start=0,stop=None))]
    fn index(
        slf: &Bound<'_, Self>,
        value: &Obj<'_>,
        start: isize,
        stop: Option<isize>,
    ) -> PyResult<usize> {
        crate::model::module(slf.py(), "collections.abc")?
            .getattr("Sequence")?
            .call_method1("index", (slf, value, start, stop))?
            .extract()
    }
    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "Views({} {})",
            self.keys.bind(py).len()?,
            self.class.bind(py).getattr("__name__")?
        ))
    }
}
#[pyclass(mapping, module = "built_core", skip_from_py_object)]
pub(crate) struct CopiedKeys {
    columns: Py<PyDict>,
    alive: Option<Py<PyAny>>,
    cache: Py<PyDict>,
}
impl CopiedKeys {
    pub(crate) fn new(columns: Bound<'_, PyDict>) -> Self {
        let cache = PyDict::new(columns.py()).unbind();
        Self {
            columns: columns.unbind(),
            alive: None,
            cache,
        }
    }
}
#[pymethods]
impl CopiedKeys {
    fn __len__(&self, py: Python<'_>) -> usize {
        self.columns.bind(py).len()
    }
    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyIterator>> {
        self.columns.bind(py).as_any().try_iter()
    }
    fn __getitem__<'py>(&self, py: Python<'py>, kind: &Obj<'py>) -> PyResult<Obj<'py>> {
        let cache = self.cache.bind(py);
        if let Some(v) = cache.get_item(kind)? {
            return Ok(v);
        }
        let pair = self.columns.bind(py).as_any().get_item(kind)?;
        let out = PyList::empty(py);
        for (a, b) in pair
            .get_item(0)?
            .try_iter()?
            .zip(pair.get_item(1)?.try_iter()?)
        {
            let b = b?;
            let a = a?;
            if !b.is_none() {
                if let Some(op) = &self.alive {
                    if !op.bind(py).call1((&b,))?.is_truthy()? {
                        continue;
                    }
                }
                out.append((a, b))?;
            }
        }
        cache.set_item(kind, &out)?;
        Ok(out.into_any())
    }
    #[pyo3(signature=(kind,default=None))]
    fn get<'py>(
        &self,
        py: Python<'py>,
        kind: &Obj<'py>,
        default: Option<&Obj<'py>>,
    ) -> PyResult<Obj<'py>> {
        if self.columns.bind(py).contains(kind)? {
            self.__getitem__(py, kind)
        } else {
            Ok(default.cloned().unwrap_or_else(|| py.None().into_bound(py)))
        }
    }
    fn keys<'py>(slf: &Bound<'py, Self>) -> PyResult<Obj<'py>> {
        crate::model::module(slf.py(), "collections.abc")?.call_method1("KeysView", (slf,))
    }
    fn items<'py>(slf: &Bound<'py, Self>) -> PyResult<Obj<'py>> {
        crate::model::module(slf.py(), "collections.abc")?.call_method1("ItemsView", (slf,))
    }
    fn values<'py>(slf: &Bound<'py, Self>) -> PyResult<Obj<'py>> {
        crate::model::module(slf.py(), "collections.abc")?.call_method1("ValuesView", (slf,))
    }
    fn only_live(&self, py: Python<'_>, store: &Obj<'_>) -> PyResult<Self> {
        Ok(Self {
            columns: self.columns.clone_ref(py),
            alive: Some(store.getattr("is_live")?.unbind()),
            cache: PyDict::new(py).unbind(),
        })
    }
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add_class::<PieceElements>()?;
    m.add_class::<CopiedKeys>()?;
    let abc = crate::model::module(py, "collections.abc")?;
    abc.getattr("Sequence")?
        .call_method1("register", (py.get_type::<PieceElements>(),))?;
    abc.getattr("Mapping")?
        .call_method1("register", (py.get_type::<CopiedKeys>(),))?;
    Ok(())
}
