// SPDX-License-Identifier: GPL-3.0-or-later
//! Install the viewport API onto its Python host without imposing a native base
//! class. The same unbound methods can operate on Metal's compatible surfaces.
use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString, PyTuple};
mod actions;
mod defaults;
mod helpers;
mod host;
mod plugin;

type Obj<'py> = Bound<'py, PyAny>;

/// Python name, native entry point, parameters, return annotation and documentation.
/// Parameter declarations retain the source names independently of native argument names.
type Entry = (
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static str,
    &'static str,
);

type Adapter = for<'py> fn(&Bound<'py, PyTuple>) -> PyResult<Obj<'py>>;
type AdapterEntry = (
    &'static str,
    Adapter,
    &'static [&'static str],
    &'static str,
    &'static str,
);

enum Endpoint {
    Kernel(Py<PyAny>, Py<PyString>),
    Adapter(Adapter),
}

impl Endpoint {
    fn invoke<'py>(&self, args: &Bound<'py, PyTuple>) -> PyResult<Obj<'py>> {
        match self {
            Self::Kernel(module, name) => module
                .bind(args.py())
                .call_method1(name.bind(args.py()), args),
            Self::Adapter(callback) => callback(args),
        }
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Method,
    Static,
    Class,
    Function,
}

struct Parameter {
    name: &'static str,
    annotation: &'static str,
    default: Option<Py<PyAny>>,
}

#[pyclass(frozen, dict, weakref, module = "ingetrazo_render")]
struct DocumentBinding {
    endpoint: Endpoint,
    method_type: Py<PyAny>,
    name: &'static str,
    params: Vec<Parameter>,
    positional: usize,
    result: &'static str,
}

#[pymethods]
impl DocumentBinding {
    fn __get__<'py>(
        slf: &Bound<'py, Self>,
        obj: Option<&Obj<'py>>,
        _owner: Option<&Obj<'py>>,
    ) -> PyResult<Obj<'py>> {
        match obj {
            None => Ok(slf.clone().into_any()),
            Some(obj) => slf.get().method_type.bind(slf.py()).call1((slf, obj)),
        }
    }

    #[pyo3(signature = (*args, **kwargs))]
    fn __call__<'py>(
        &self,
        args: &Bound<'py, PyTuple>,
        kwargs: Option<&Bound<'py, PyDict>>,
    ) -> PyResult<Obj<'py>> {
        let py = args.py();
        let given = args.len();
        let keywords = kwargs.filter(|k| !k.is_empty());
        // Most calls already have exactly the positional arguments of the kernel.
        if given == self.params.len() && given == self.positional && keywords.is_none() {
            return self.endpoint.invoke(args);
        }
        if given > self.positional {
            return Err(PyTypeError::new_err(format!(
                "{}() accepts at most {} positional arguments",
                self.name, self.positional
            )));
        }
        let mut values: Vec<_> = args.iter().collect();
        let mut used = 0;
        for p in &self.params[given..] {
            let value = if let Some(v) = keywords.map(|k| k.get_item(p.name)).transpose()?.flatten()
            {
                used += 1;
                v
            } else if let Some(v) = &p.default {
                v.bind(py).clone()
            } else {
                return Err(PyTypeError::new_err(format!(
                    "{}() missing required argument '{}'",
                    self.name, p.name
                )));
            };
            values.push(value);
        }
        if let Some(kw) = keywords.filter(|k| k.len() != used) {
            for (key, _) in kw.iter() {
                let name: String = key.extract()?;
                if self.params[..given].iter().any(|p| p.name == name) {
                    return Err(PyTypeError::new_err(format!(
                        "{}() got multiple values for argument '{}'",
                        self.name, name
                    )));
                }
                if !self.params.iter().any(|p| p.name == name) {
                    return Err(PyTypeError::new_err(format!(
                        "{}() got an unexpected keyword argument '{}'",
                        self.name, name
                    )));
                }
            }
        }
        self.endpoint.invoke(&PyTuple::new(py, values)?)
    }

    // References in plugin state resolve to the same callable when reloaded.
    fn __reduce__<'py>(slf: &Bound<'py, Self>) -> PyResult<Obj<'py>> {
        slf.getattr("__qualname__")
    }

    #[getter]
    fn __signature__<'py>(&self, py: Python<'py>) -> PyResult<Obj<'py>> {
        // Introspection is needed by plugins and tooling, never by dispatch.
        let inspect = py.import("inspect")?;
        let param = inspect.getattr("Parameter")?;
        let pos = param.getattr("POSITIONAL_OR_KEYWORD")?;
        let kw = param.getattr("KEYWORD_ONLY")?;
        let mut params = Vec::with_capacity(self.params.len());
        for (i, p) in self.params.iter().enumerate() {
            let opts = PyDict::new(py);
            if let Some(value) = &p.default {
                opts.set_item("default", value)?;
            }
            if !p.annotation.is_empty() {
                opts.set_item("annotation", p.annotation)?;
            }
            params.push(param.call(
                (p.name, if i < self.positional { &pos } else { &kw }),
                Some(&opts),
            )?);
        }
        let opts = PyDict::new(py);
        if !self.result.is_empty() {
            opts.set_item("return_annotation", self.result)?;
        }
        inspect.getattr("Signature")?.call((params,), Some(&opts))
    }
}

struct Registrar<'py> {
    core: Bound<'py, PyModule>,
    render: Bound<'py, PyModule>,
    method: Obj<'py>,
    builtins: Bound<'py, PyModule>,
    defaults: Bound<'py, PyDict>,
}

impl<'py> Registrar<'py> {
    fn new(py: Python<'py>) -> PyResult<Self> {
        Ok(Self {
            core: py.import("ingetrazo_core")?,
            render: py.import("ingetrazo_render")?,
            method: py.import("types")?.getattr("MethodType")?,
            builtins: py.import("builtins")?,
            defaults: PyDict::new(py),
        })
    }

    fn install(
        &self,
        owner: &Obj<'py>,
        backend: &Obj<'py>,
        mode: Mode,
        entries: &[Entry],
    ) -> PyResult<()> {
        for &(name, target, declarations, result, doc) in entries {
            let kernel = Endpoint::Kernel(
                backend.clone().unbind(),
                PyString::intern(owner.py(), target).unbind(),
            );
            self.define(owner, mode, name, kernel, declarations, result, doc)?;
        }
        Ok(())
    }

    fn define(
        &self,
        owner: &Obj<'py>,
        mode: Mode,
        name: &'static str,
        endpoint: Endpoint,
        declarations: &[&'static str],
        result: &'static str,
        doc: &str,
    ) -> PyResult<()> {
        let py = owner.py();
        let mut params = Vec::new();
        let mut limit = None;
        let annotations = PyDict::new(py);
        for &decl in declarations {
            if decl == "*" {
                limit = Some(params.len());
                continue;
            }
            let (arg, default) = match decl.split_once('=') {
                Some((arg, literal)) => {
                    let value = match self.defaults.get_item(literal)? {
                        Some(value) => value,
                        None => {
                            let value =
                                py.import("ast")?.call_method1("literal_eval", (literal,))?;
                            self.defaults.set_item(literal, &value)?;
                            value
                        }
                    };
                    (arg, Some(value.unbind()))
                }
                None => (decl, None),
            };
            let (name, annotation) = arg.split_once(':').unwrap_or((arg, ""));
            if !annotation.is_empty() {
                annotations.set_item(name, annotation)?;
            }
            params.push(Parameter {
                name,
                annotation,
                default,
            });
        }
        if !result.is_empty() {
            annotations.set_item("return", result)?;
        }
        let binding = Bound::new(
            py,
            DocumentBinding {
                endpoint,
                method_type: self.method.clone().unbind(),
                name,
                positional: limit.unwrap_or(params.len()),
                params,
                result,
            },
        )?;
        binding.setattr("__name__", name)?;
        let qualname = if matches!(mode, Mode::Function) {
            name.to_owned()
        } else {
            format!("DocumentInteraction.{name}")
        };
        binding.setattr("__qualname__", qualname)?;
        binding.setattr("__module__", "views.document_interaction")?;
        binding.setattr("__annotations__", annotations)?;
        binding.setattr("__doc__", (!doc.is_empty()).then_some(doc))?;
        let binding = match mode {
            Mode::Method | Mode::Function => binding.into_any(),
            Mode::Static => self.builtins.call_method1("staticmethod", (binding,))?,
            Mode::Class => self.builtins.call_method1("classmethod", (binding,))?,
        };
        if matches!(mode, Mode::Function) {
            owner.cast::<PyDict>()?.set_item(name, binding)?;
        } else {
            owner.setattr(name, binding)?;
        }
        Ok(())
    }
}

#[pyfunction]
fn register_document_host(cls: &Obj) -> PyResult<()> {
    let r = Registrar::new(cls.py())?;
    defaults::controller(&r, cls)?;
    host::install(&r, cls)?;
    plugin::install(&r, cls)?;
    for &(name, action, params, result, doc) in actions::METHODS {
        r.define(
            cls,
            Mode::Method,
            name,
            Endpoint::Adapter(action),
            params,
            result,
            doc,
        )?;
    }
    Ok(())
}

#[pyfunction]
fn register_document_module(ns: &Bound<PyDict>) -> PyResult<()> {
    let r = Registrar::new(ns.py())?;
    defaults::module(&r, ns)?;
    helpers::install(&r, ns)
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(register_document_module, m)?)?;
    m.add_function(wrap_pyfunction!(register_document_host, m)?)?;
    Ok(())
}
