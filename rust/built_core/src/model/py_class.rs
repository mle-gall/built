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
#[pyclass(frozen, module = "built_core", name = "NativeMethod")]
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

/// A native method the way CPython's own are: a method descriptor (a
/// `PyMethodDef` with `METH_FASTCALL`, bound to `object`), so `obj.name(…)`
/// calls straight into Rust as a Python function is called, with no bound
/// method made per access. [`NativeMethod`] costs a `MethodType` and a
/// trampoline per call, which a one-line rule (`Group.is_component`,
/// `Scene.__setattr__`) notices. The body gets the instance and the
/// positional arguments; a `_kw` one also gets the keyword names (CPython's
/// vectorcall convention: their values follow the positional ones).
pub type FastBody = for<'py> fn(&Bound<'py, PyAny>, &[Bound<'py, PyAny>]) -> PyResult<Bound<'py, PyAny>>;
pub type FastKwBody = for<'py> fn(&Bound<'py, PyAny>, &[Bound<'py, PyAny>], Option<&Bound<'py, PyTuple>>) -> PyResult<Bound<'py, PyAny>>;

/// Run a fast method's body: the GIL is held (CPython is calling), errors
/// and panics become the Python exception.
///
/// # Safety
/// `slf` and the `n` pointers at `args` are live objects CPython lends for
/// the call; `kwnames` is null or a tuple.
pub unsafe fn run_fast(
    slf: *mut pyo3::ffi::PyObject,
    args: *const *mut pyo3::ffi::PyObject,
    n: usize,
    kwnames: *mut pyo3::ffi::PyObject,
    body: Result<FastBody, FastKwBody>,
) -> *mut pyo3::ffi::PyObject {
    let py = unsafe { Python::assume_attached() };
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> PyResult<Bound<'_, PyAny>> {
        let me = unsafe { Bound::from_borrowed_ptr(py, slf) };
        let nkw = if kwnames.is_null() { 0 } else { unsafe { pyo3::ffi::PyTuple_Size(kwnames) as usize } };
        let all: Vec<Bound<'_, PyAny>> = (0..n + nkw).map(|i| unsafe { Bound::from_borrowed_ptr(py, *args.add(i)) }).collect();
        match body {
            Ok(f) => {
                if nkw != 0 {
                    return Err(pyo3::exceptions::PyTypeError::new_err("this method takes no keyword arguments"));
                }
                f(&me, &all)
            }
            Err(f) => {
                let names = if kwnames.is_null() { None } else { Some(unsafe { Bound::from_borrowed_ptr(py, kwnames) }.cast_into::<PyTuple>()?) };
                f(&me, &all, names.as_ref())
            }
        }
    }));
    match out {
        Ok(Ok(v)) => v.into_ptr(),
        Ok(Err(e)) => {
            e.restore(py);
            std::ptr::null_mut()
        }
        Err(_) => {
            pyo3::exceptions::PyRuntimeError::new_err("a native method panicked").restore(py);
            std::ptr::null_mut()
        }
    }
}

/// `extern "C"` entry points for a fast method body: `fast!(entry, body)`
/// for positional arguments, `fast!(kw entry, body)` with keywords.
#[macro_export]
macro_rules! fast {
    ($entry:ident, $body:path) => {
        unsafe extern "C" fn $entry(
            slf: *mut pyo3::ffi::PyObject,
            args: *mut *mut pyo3::ffi::PyObject,
            n: pyo3::ffi::Py_ssize_t,
        ) -> *mut pyo3::ffi::PyObject {
            unsafe { $crate::model::py_class::run_fast(slf, args as *const _, n as usize, std::ptr::null_mut(), Ok($body)) }
        }
    };
    (kw $entry:ident, $body:path) => {
        unsafe extern "C" fn $entry(
            slf: *mut pyo3::ffi::PyObject,
            args: *const *mut pyo3::ffi::PyObject,
            n: pyo3::ffi::Py_ssize_t,
            kwnames: *mut pyo3::ffi::PyObject,
        ) -> *mut pyo3::ffi::PyObject {
            // The method descriptor passes a plain count (no offset flag).
            unsafe { $crate::model::py_class::run_fast(slf, args, n as usize, kwnames, Err($body)) }
        }
    };
}

fn descriptor<'py>(py: Python<'py>, name: &'static std::ffi::CStr, meth: pyo3::ffi::PyMethodDefPointer, flags: std::ffi::c_int) -> PyResult<Obj<'py>> {
    let def = Box::leak(Box::new(pyo3::ffi::PyMethodDef { ml_name: name.as_ptr(), ml_meth: meth, ml_flags: flags, ml_doc: std::ptr::null() }));
    // SAFETY: `def` lives for ever; `object` is the type every instance is.
    unsafe {
        let d = pyo3::ffi::PyDescr_NewMethod(std::ptr::addr_of_mut!(pyo3::ffi::PyBaseObject_Type), def);
        Bound::from_owned_ptr_or_err(py, d)
    }
}

/// `ns[name]` = a fast method taking positional arguments (`fast!`).
pub fn fast_method(ns: &Bound<'_, PyDict>, name: &'static std::ffi::CStr, entry: pyo3::ffi::PyCFunctionFast) -> PyResult<()> {
    let d = descriptor(ns.py(), name, pyo3::ffi::PyMethodDefPointer { PyCFunctionFast: entry }, pyo3::ffi::METH_FASTCALL)?;
    ns.set_item(name.to_string_lossy(), d)
}

/// `ns[name]` = a fast method taking keywords too (`fast!(kw …)`).
pub fn fast_kw_method(ns: &Bound<'_, PyDict>, name: &'static std::ffi::CStr, entry: pyo3::ffi::PyCFunctionFastWithKeywords) -> PyResult<()> {
    let d = descriptor(ns.py(), name, pyo3::ffi::PyMethodDefPointer { PyCFunctionFastWithKeywords: entry }, pyo3::ffi::METH_FASTCALL | pyo3::ffi::METH_KEYWORDS)?;
    ns.set_item(name.to_string_lossy(), d)
}

/// The arguments of a fast keyword call matched to `names` (positional
/// first, then by keyword); `None` for one not given.
pub fn bind_args<'a, 'py>(args: &'a [Bound<'py, PyAny>], kwnames: Option<&Bound<'py, PyTuple>>, names: &[&str]) -> PyResult<Vec<Option<&'a Bound<'py, PyAny>>>> {
    let nkw = kwnames.map_or(0, |k| k.len());
    let npos = args.len() - nkw;
    if npos > names.len() {
        return Err(pyo3::exceptions::PyTypeError::new_err(format!("takes at most {} arguments ({npos} given)", names.len())));
    }
    let mut out: Vec<Option<&Bound<'py, PyAny>>> = (0..names.len()).map(|i| args[..npos].get(i)).collect();
    if let Some(k) = kwnames {
        for (j, key) in k.iter().enumerate() {
            let key: String = key.extract()?;
            let Some(i) = names.iter().position(|n| *n == key) else {
                return Err(pyo3::exceptions::PyTypeError::new_err(format!("unexpected keyword argument '{key}'")));
            };
            if out[i].is_some() {
                return Err(pyo3::exceptions::PyTypeError::new_err(format!("got multiple values for argument '{key}'")));
            }
            out[i] = Some(&args[npos + j]);
        }
    }
    Ok(out)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<NativeMethod>()
}
