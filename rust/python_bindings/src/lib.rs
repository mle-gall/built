// SPDX-License-Identifier: GPL-3.0-or-later
//! Register native callbacks on existing Python classes with their plugin-facing signatures.
use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString, PyTuple};
pub type Obj<'py> = Bound<'py, PyAny>;

/// Python name, native entry point, parameters, return annotation and documentation.
/// Parameter declarations retain the source names independently of native argument names.
pub type Entry = (
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static str,
    &'static str,
);

pub type Adapter = for<'py> fn(&Bound<'py, PyTuple>) -> PyResult<Obj<'py>>;
pub type AdapterEntry = (
    &'static str,
    Adapter,
    &'static [&'static str],
    &'static str,
    &'static str,
);

pub enum Endpoint {
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
pub enum Mode {
    Method,
    Static,
    Class,
    Property,
    Setter,
    Function,
}

struct Parameter {
    name: &'static str,
    default: Option<Py<PyAny>>,
}

#[pyclass(frozen, dict, weakref, module = "built_core")]
struct NativeBinding {
    endpoint: Endpoint,
    method_type: Py<PyAny>,
    name: &'static str,
    params: Vec<Parameter>,
    positional: usize,
}

#[pymethods]
impl NativeBinding {
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
    fn __signature__<'py>(slf: &Bound<'py, Self>) -> PyResult<Obj<'py>> {
        // Introspection is needed by plugins and tooling, never by dispatch.
        let py = slf.py();
        let binding = slf.get();
        let annotations = slf.getattr("__annotations__")?.cast_into::<PyDict>()?;
        let inspect = py.import("inspect")?;
        let param = inspect.getattr("Parameter")?;
        let pos = param.getattr("POSITIONAL_OR_KEYWORD")?;
        let kw = param.getattr("KEYWORD_ONLY")?;
        let mut params = Vec::with_capacity(binding.params.len());
        for (i, p) in binding.params.iter().enumerate() {
            let opts = PyDict::new(py);
            if let Some(value) = &p.default {
                opts.set_item("default", value)?;
            }
            if let Some(annotation) = annotations.get_item(p.name)? {
                opts.set_item("annotation", annotation)?;
            }
            params.push(param.call(
                (p.name, if i < binding.positional { &pos } else { &kw }),
                Some(&opts),
            )?);
        }
        let opts = PyDict::new(py);
        if let Some(result) = annotations.get_item("return")? {
            opts.set_item("return_annotation", result)?;
        }
        inspect.getattr("Signature")?.call((params,), Some(&opts))
    }
}

pub struct Registrar<'py> {
    method: Obj<'py>,
    plugin_method: Option<Obj<'py>>,
    annotation_globals: Option<Bound<'py, PyDict>>,
    builtins: Bound<'py, PyModule>,
    defaults: Bound<'py, PyDict>,
}

impl<'py> Registrar<'py> {
    pub fn new(py: Python<'py>) -> PyResult<Self> {
        Ok(Self {
            plugin_method: None,
            annotation_globals: None,
            method: py.import("types")?.getattr("MethodType")?,
            builtins: py.import("builtins")?,
            defaults: PyDict::new(py),
        })
    }

    /// Some plugin hosts inspect a bound slot with CPython's PyFunction API.
    /// Give them a real Python function while dispatching the body natively.
    pub fn with_plugin_method(mut self, wrap: Obj<'py>) -> Self {
        self.plugin_method = Some(wrap);
        self
    }

    pub fn with_module_annotations(mut self, namespace: &Bound<'py, PyDict>) -> PyResult<Self> {
        let future = namespace
            .py()
            .import("__future__")?
            .getattr("annotations")?;
        if !namespace
            .get_item("annotations")?
            .is_some_and(|flag| flag.is(&future))
        {
            self.annotation_globals = Some(namespace.clone());
        }
        Ok(self)
    }

    fn annotation(&self, declaration: &str) -> PyResult<Obj<'py>> {
        let py = self.builtins.py();
        let Some(globals) = &self.annotation_globals else {
            return Ok(PyString::new(py, declaration).into_any());
        };
        let resolve = |name: &str| -> PyResult<Obj<'py>> {
            if name == "None" {
                return Ok(py.None().into_bound(py));
            }
            match globals.get_item(name)? {
                Some(value) => Ok(value),
                None => self.builtins.getattr(name),
            }
        };
        let mut names = declaration.split(" | ");
        let mut value = resolve(names.next().unwrap_or(declaration))?;
        for name in names {
            value = value.bitor(resolve(name)?)?;
        }
        Ok(value)
    }

    pub fn bind_default(&self, name: &str, value: &Obj<'py>) -> PyResult<()> {
        self.defaults.set_item(name, value)
    }

    pub fn install(
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

    pub fn define(
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
                annotations.set_item(name, self.annotation(annotation)?)?;
            }
            params.push(Parameter { name, default });
        }
        if !result.is_empty() {
            annotations.set_item("return", self.annotation(result)?)?;
        }
        let binding = Bound::new(
            py,
            NativeBinding {
                endpoint,
                method_type: self.method.clone().unbind(),
                name,
                positional: limit.unwrap_or(params.len()),
                params,
            },
        )?;
        binding.setattr("__name__", name)?;
        let qualname = if matches!(mode, Mode::Function) {
            name.to_owned()
        } else {
            format!(
                "{}.{name}",
                owner.getattr("__qualname__")?.extract::<String>()?
            )
        };
        binding.setattr("__qualname__", qualname)?;
        let module = if matches!(mode, Mode::Function) {
            owner.get_item("__name__")?
        } else {
            owner.getattr("__module__")?
        };
        binding.setattr("__module__", module)?;
        binding.setattr("__annotations__", annotations)?;
        binding.setattr("__doc__", (!doc.is_empty()).then_some(doc))?;
        let binding = match (&self.plugin_method, mode) {
            (Some(wrap), Mode::Method | Mode::Class) => wrap.call1((binding,))?,
            _ => binding.into_any(),
        };
        let binding = match mode {
            Mode::Method | Mode::Function => binding,
            Mode::Static => self.builtins.call_method1("staticmethod", (binding,))?,
            Mode::Class => self.builtins.call_method1("classmethod", (binding,))?,
            Mode::Property => self.builtins.call_method1("property", (binding,))?,
            Mode::Setter => owner.getattr(name)?.call_method1("setter", (binding,))?,
        };
        if matches!(mode, Mode::Function) {
            owner.cast::<PyDict>()?.set_item(name, binding)?;
        } else {
            owner.setattr(name, binding)?;
        }
        Ok(())
    }
}
