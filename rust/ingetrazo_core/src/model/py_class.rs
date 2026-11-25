// SPDX-License-Identifier: GPL-3.0-or-later
//! Python classes built from native functions, for the `core.*` modules
//! served from Rust (`core.doc_model`, `core.dimension`, …): plugins
//! subclass them, pickle them and check them with `isinstance`, so they
//! stay ordinary heap types made with `type(name, bases, namespace)`; the
//! methods are native functions bound by [`NativeMethod`], which binds as
//! a Python function does (a bound method per access).

use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyDict, PyTuple};

type Obj<'py> = Bound<'py, PyAny>;

static METHOD_TYPE: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

/// A native function as a method: `obj.name` is `MethodType(function, obj)`,
/// `Class.name` the function itself.
#[pyclass(frozen, module = "ingetrazo_core", name = "NativeMethod")]
pub struct NativeMethod {
    function: Py<PyAny>,
}

#[pymethods]
impl NativeMethod {
    #[pyo3(signature = (*args, **kwargs))]
    fn __call__<'py>(&self, args: &Bound<'py, PyTuple>, kwargs: Option<&Bound<'py, PyDict>>) -> PyResult<Obj<'py>> {
        self.function.bind(args.py()).call(args, kwargs)
    }

    fn __get__<'py>(&self, py: Python<'py>, instance: Option<&Obj<'py>>, _owner: Option<&Obj<'py>>) -> PyResult<Obj<'py>> {
        let func = self.function.bind(py);
        match instance {
            Some(obj) => {
                let mt = METHOD_TYPE.get_or_try_init(py, || -> PyResult<_> { Ok(py.import("types")?.getattr("MethodType")?.unbind()) })?;
                mt.bind(py).call1((func, obj))
            }
            None => Ok(func.clone()),
        }
    }

    #[getter]
    fn __doc__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(self.function.bind(py).getattr("__doc__")?.unbind())
    }

    #[getter]
    fn __wrapped__(&self, py: Python<'_>) -> Py<PyAny> {
        self.function.clone_ref(py)
    }
}

/// `ns[name]` = `f` as a method.
pub fn method<'py, T>(ns: &Bound<'py, PyDict>, name: &str, f: Bound<'py, T>) -> PyResult<()> {
    let py = ns.py();
    let function = f.into_any().unbind();
    ns.set_item(name, Bound::new(py, NativeMethod { function })?)
}

/// `f` as a [`NativeMethod`]: callable as it is, bound when kept on a
/// class.
pub fn native_method<'py, T>(f: Bound<'py, T>) -> PyResult<Bound<'py, NativeMethod>> {
    let py = f.py();
    Bound::new(py, NativeMethod { function: f.into_any().unbind() })
}

/// `ns[name]` = `property(get, set)`.
pub fn property<'py>(ns: &Bound<'py, PyDict>, name: &str, get: Obj<'py>, set: Option<Obj<'py>>) -> PyResult<()> {
    let py = ns.py();
    ns.set_item(name, py.import("builtins")?.getattr("property")?.call1((get, set))?)
}

/// `ns[name]` = `staticmethod(f)`.
pub fn static_method<'py>(ns: &Bound<'py, PyDict>, name: &str, f: Obj<'py>) -> PyResult<()> {
    let py = ns.py();
    ns.set_item(name, py.import("builtins")?.getattr("staticmethod")?.call1((f,))?)
}

/// `type(name, bases, ns)` with `__module__` = `module` and `__qualname__`
/// = `name`.
pub fn class<'py>(py: Python<'py>, module: &str, name: &str, bases: &[Obj<'py>], ns: &Bound<'py, PyDict>) -> PyResult<Obj<'py>> {
    ns.set_item("__module__", module)?;
    ns.set_item("__qualname__", name)?;
    py.import("builtins")?.getattr("type")?.call1((name, PyTuple::new(py, bases)?, ns))
}

static GETATTR: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
static MISSING: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

/// `getattr(o, name, default)` and `hasattr` the way Python runs them. Built
/// for the 3.11 stable ABI, PyO3's `getattr_opt` and `hasattr` raise and
/// catch an `AttributeError` for every missing name, and making that
/// exception costs more than the rule asking (`layer_of` on an edge, which
/// has no `attrs`). The builtin looks the name up without raising.
pub trait FastAttr<'py> {
    fn attr_opt<N: IntoPyObject<'py>>(&self, name: N) -> PyResult<Option<Obj<'py>>>;
    fn has_attr<N: IntoPyObject<'py>>(&self, name: N) -> PyResult<bool> {
        Ok(self.attr_opt(name)?.is_some())
    }
}

impl<'py, T> FastAttr<'py> for Bound<'py, T> {
    fn attr_opt<N: IntoPyObject<'py>>(&self, name: N) -> PyResult<Option<Obj<'py>>> {
        let py = self.py();
        let getattr = GETATTR.get_or_try_init(py, || -> PyResult<_> { Ok(py.import("builtins")?.getattr("getattr")?.unbind()) })?;
        let missing = MISSING.get_or_try_init(py, || -> PyResult<_> { Ok(py.import("builtins")?.getattr("object")?.call0()?.unbind()) })?;
        let name = pyo3::BoundObject::into_bound(pyo3::BoundObject::into_any(name.into_pyobject(py).map_err(Into::into)?));
        let got = getattr.bind(py).call1((self.as_any(), name, missing.bind(py)))?;
        Ok(if got.is(missing.bind(py)) { None } else { Some(got) })
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<NativeMethod>()
}
