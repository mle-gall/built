// SPDX-License-Identifier: GPL-3.0-or-later
//! Ordered plugin element lists. Reads always resolve the owner's active frame.
use super::*;
use pyo3::exceptions::PyIndexError;
use pyo3::types::{PySlice, PyTuple};

type Obj<'py> = Bound<'py, PyAny>;

#[pyclass(module = "ingetrazo_core", name = "MeshElements")]
pub(super) struct Elements {
    mesh: Py<PyAny>,
    class: Py<PyAny>,
    base: Py<PyAny>,
    kind: Kind,
}

impl Elements {
    fn mesh<'py>(&self, py: Python<'py>) -> &Obj<'py> {
        self.mesh.bind(py)
    }

    fn store<'py>(&self, py: Python<'py>) -> PyResult<Obj<'py>> {
        self.mesh(py).getattr("_store")
    }

    fn keys<'py>(&self, py: Python<'py>) -> PyResult<Obj<'py>> {
        self.store(py)?.call_method1("keys", (self._kind(),))
    }

    fn views<'py>(&self, keys: &Obj<'py>) -> PyResult<Obj<'py>> {
        let py = keys.py();
        let make = py
            .import("functools")?
            .getattr("partial")?
            .call1((self.class.bind(py), self.mesh(py)))?;
        py.import("builtins")?.call_method1("map", (make, keys))
    }

    fn write<'py>(
        &self,
        name: &str,
        args: impl pyo3::call::PyCallArgs<'py>,
        py: Python<'py>,
    ) -> PyResult<Obj<'py>> {
        let items = self.copy(py)?;
        let result = items.call_method1(name, args)?;
        self.replace(&items)?;
        Ok(result)
    }
}

#[pymethods]
impl Elements {
    #[new]
    fn new(mesh: Py<PyAny>, kind: &str, class: Py<PyAny>, base: Py<PyAny>) -> PyResult<Self> {
        Ok(Self {
            mesh,
            kind: kind_of(kind)?,
            class,
            base,
        })
    }

    #[getter]
    fn owner<'py>(&self, py: Python<'py>) -> Obj<'py> {
        self.mesh(py).clone()
    }

    #[getter]
    fn _kind(&self) -> &'static str {
        match self.kind {
            Kind::Vertex => "vertex",
            Kind::Edge => "edge",
            Kind::Face => "face",
        }
    }

    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        self.store(py)?
            .call_method0("counts")?
            .get_item(self.kind as usize)?
            .extract()
    }

    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Obj<'py>> {
        self.views(&self.keys(py)?)
    }

    fn __reversed__<'py>(&self, py: Python<'py>) -> PyResult<Obj<'py>> {
        self.views(&self.keys(py)?.call_method0("__reversed__")?)
    }

    fn __getitem__<'py>(&self, index: &Obj<'py>) -> PyResult<Obj<'py>> {
        let py = index.py();
        if index.is_instance_of::<PySlice>() {
            let keys = self.keys(py)?.get_item(index)?;
            return py.get_type::<PyList>().call1((self.views(&keys)?,));
        }
        let i: isize = index.extract()?;
        let n = self.__len__(py)? as isize;
        let i = if i < 0 { i + n } else { i };
        if !(0..n).contains(&i) {
            return Err(PyIndexError::new_err("list index out of range"));
        }
        let key = self.store(py)?.call_method1("key_at", (self._kind(), i))?;
        self.class.bind(py).call1((self.mesh(py), key))
    }

    fn __contains__(&self, item: &Obj<'_>) -> PyResult<bool> {
        let py = item.py();
        if !item.is_instance(self.base.bind(py))? {
            return Ok(false);
        }
        let Ok(key) = item.getattr("key")?.extract::<u128>() else {
            return Ok(false);
        };
        self.store(py)?.call_method1("is_live", (key,))?.extract()
    }

    #[pyo3(signature = (item, *bounds))]
    fn index(&self, item: &Obj<'_>, bounds: &Bound<'_, PyTuple>) -> PyResult<usize> {
        let py = item.py();
        if bounds.is_empty() && self.__contains__(item)? {
            return self
                .store(py)?
                .call_method1("index_of", (item.getattr("key")?,))?
                .extract();
        }
        let mut args = vec![item.clone()];
        args.extend(bounds.iter());
        self.copy(py)?
            .call_method1("index", PyTuple::new(py, args)?)?
            .extract()
    }

    fn count(&self, item: &Obj<'_>) -> PyResult<usize> {
        Ok(self.__contains__(item)? as usize)
    }

    fn copy<'py>(&self, py: Python<'py>) -> PyResult<Obj<'py>> {
        py.get_type::<PyList>().call1((self.__iter__(py)?,))
    }

    fn __eq__<'py>(&self, other: &Obj<'py>) -> PyResult<Obj<'py>> {
        let py = other.py();
        if !other.is_instance_of::<PyList>() && !other.is_instance_of::<Self>() {
            return Ok(py.NotImplemented().into_bound(py));
        }
        self.copy(py)?
            .call_method1("__eq__", (py.get_type::<PyList>().call1((other,))?,))
    }

    fn __add__<'py>(&self, other: &Obj<'py>) -> PyResult<Obj<'py>> {
        let py = other.py();
        self.copy(py)?
            .call_method1("__add__", (py.get_type::<PyList>().call1((other,))?,))
    }

    fn __radd__<'py>(&self, other: &Obj<'py>) -> PyResult<Obj<'py>> {
        let py = other.py();
        py.get_type::<PyList>()
            .call1((other,))?
            .call_method1("__add__", (self.copy(py)?,))
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        self.copy(py)?.repr()?.extract()
    }

    fn replace(&self, items: &Obj<'_>) -> PyResult<()> {
        let py = items.py();
        let store = self.store(py)?;
        let mut keys = Vec::new();
        for item in items.try_iter()? {
            let item = item?;
            if item.is_instance(self.class.bind(py))? {
                let key: u128 = item.getattr("key")?.extract()?;
                if store.call_method1("knows", (key,))?.is_truthy()? {
                    keys.push(key);
                    continue;
                }
            }
            return Err(PyValueError::new_err(format!(
                "{} list requires keyed elements belonging to this mesh",
                self._kind()
            )));
        }
        store.call_method1("set_members", (self._kind(), keys))?;
        self.mesh(py).call_method0("_dirty")?;
        Ok(())
    }

    fn append(&self, item: &Obj<'_>) -> PyResult<()> {
        self.write("append", (item,), item.py())?;
        Ok(())
    }
    fn extend(&self, items: &Obj<'_>) -> PyResult<()> {
        self.write("extend", (items,), items.py())?;
        Ok(())
    }
    fn remove(&self, item: &Obj<'_>) -> PyResult<()> {
        self.write("remove", (item,), item.py())?;
        Ok(())
    }
    fn insert(&self, index: isize, item: &Obj<'_>) -> PyResult<()> {
        self.write("insert", (index, item), item.py())?;
        Ok(())
    }
    #[pyo3(signature = (index=-1))]
    fn pop<'py>(&self, py: Python<'py>, index: isize) -> PyResult<Obj<'py>> {
        self.write("pop", (index,), py)
    }
    fn clear(&self, py: Python<'_>) -> PyResult<()> {
        self.write("clear", (), py)?;
        Ok(())
    }
    fn reverse(&self, py: Python<'_>) -> PyResult<()> {
        self.write("reverse", (), py)?;
        Ok(())
    }
    #[pyo3(signature = (**kwargs))]
    fn sort(&self, py: Python<'_>, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<()> {
        let items = self.copy(py)?;
        items.call_method("sort", (), kwargs)?;
        self.replace(&items)
    }
    fn __setitem__(&self, i: &Obj<'_>, value: &Obj<'_>) -> PyResult<()> {
        self.write("__setitem__", (i, value), i.py())?;
        Ok(())
    }
    fn __delitem__(&self, i: &Obj<'_>) -> PyResult<()> {
        self.write("__delitem__", (i,), i.py())?;
        Ok(())
    }
    fn __iadd__(&self, items: &Obj<'_>) -> PyResult<()> {
        self.extend(items)
    }
    fn __imul__(&self, py: Python<'_>, n: isize) -> PyResult<()> {
        self.write("__imul__", (n,), py)?;
        Ok(())
    }
}
