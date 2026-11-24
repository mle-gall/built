//! Preferences shared with the classic host. A handle has its own group stack.
mod ini;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(windows)]
mod registry;

use pyo3::{
    exceptions::{PyTypeError, PyValueError},
    prelude::*,
    types::{PyBool, PyBytes, PyList, PyString, PyType},
};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Strings(Vec<String>),
    Bytes(Vec<u8>),
}

impl Value {
    fn from_py(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(if obj.is_none() {
            Self::Null
        } else if obj.is_instance_of::<PyBool>() {
            Self::Bool(obj.extract()?)
        } else if obj.is_instance_of::<PyString>() {
            Self::Text(obj.extract()?)
        } else if let Ok(v) = obj.extract::<i64>() {
            Self::Int(v)
        } else if let Ok(v) = obj.extract::<f64>() {
            Self::Float(v)
        } else if let Ok(v) = obj.cast::<PyBytes>() {
            Self::Bytes(v.as_bytes().to_vec())
        } else if let Ok(v) = obj.extract::<Vec<String>>() {
            Self::Strings(v)
        } else if obj.hasattr("__bytes__")? {
            Self::Bytes(obj.call_method0("__bytes__")?.extract()?)
        } else if obj.get_type().name()? == "QByteArray" {
            Self::Bytes(obj.call_method0("data")?.extract()?)
        } else {
            return Err(PyTypeError::new_err("Unsupported preference value"));
        })
    }
    fn to_py<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        Ok(match self {
            Self::Null => py.None().into_bound(py),
            Self::Bool(v) => v.into_pyobject(py)?.to_owned().into_any(),
            Self::Int(v) => v.into_pyobject(py)?.into_any(),
            Self::Float(v) => v.into_pyobject(py)?.into_any(),
            Self::Text(v) => PyString::new(py, v).into_any(),
            Self::Strings(v) => PyList::new(py, v)?.into_any(),
            Self::Bytes(v) => PyBytes::new(py, v).into_any(),
        })
    }
}

#[derive(Clone)]
enum Backend {
    Ini(PathBuf),
    #[cfg(target_os = "macos")]
    Mac,
    #[cfg(windows)]
    Registry,
}
#[pyclass]
pub struct NativeSettings {
    backend: Backend,
    groups: Vec<String>,
}

impl NativeSettings {
    fn key(&self, key: &str) -> String {
        self.groups
            .iter()
            .map(String::as_str)
            .chain([key])
            .flat_map(|s| s.split('/'))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("/")
    }
    fn read(&self, key: &str) -> PyResult<Option<Value>> {
        match &self.backend {
            Backend::Ini(p) => Ok(ini::get(p, key)?),
            #[cfg(target_os = "macos")]
            Backend::Mac => mac::get(key),
            #[cfg(windows)]
            Backend::Registry => registry::get(key),
        }
    }
    fn keys(&self) -> PyResult<Vec<String>> {
        match &self.backend {
            Backend::Ini(p) => Ok(ini::keys(p)?),
            #[cfg(target_os = "macos")]
            Backend::Mac => mac::keys(),
            #[cfg(windows)]
            Backend::Registry => registry::keys(),
        }
    }
    fn children(&self, recurse: bool) -> PyResult<Vec<String>> {
        let mut group = self.key("");
        if !group.is_empty() {
            group.push('/')
        }
        let mut keys: Vec<_> = self
            .keys()?
            .into_iter()
            .filter_map(|k| {
                k.strip_prefix(&group)
                    .filter(|s| !s.is_empty() && (recurse || !s.contains('/')))
                    .map(str::to_owned)
            })
            .collect();
        keys.sort();
        Ok(keys)
    }
}

#[pymethods]
#[allow(non_snake_case)]
impl NativeSettings {
    #[new]
    #[pyo3(signature=(profile=None))]
    fn new(profile: Option<PathBuf>) -> PyResult<Self> {
        let profile = profile.or_else(|| {
            std::env::var_os("BUILT_SETTINGS_DIR")
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        });
        let backend = if let Some(p) = profile {
            if p.as_os_str().is_empty() {
                return Err(PyValueError::new_err("Empty settings directory"));
            }
            Backend::Ini(p.join("IngeTrazo/IngeTrazo.ini"))
        } else {
            #[cfg(target_os = "macos")]
            {
                Backend::Mac
            }
            #[cfg(windows)]
            {
                Backend::Registry
            }
            #[cfg(not(any(windows, target_os = "macos")))]
            {
                let dir = std::env::var_os("XDG_CONFIG_HOME")
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
                    .ok_or_else(|| PyValueError::new_err("No user configuration directory"))?;
                Backend::Ini(dir.join("IngeTrazo/IngeTrazo.conf"))
            }
        };
        Ok(Self {
            backend,
            groups: vec![],
        })
    }

    fn fresh(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            groups: vec![],
        }
    }
    fn beginGroup(&mut self, name: String) {
        self.groups.push(name)
    }
    fn endGroup(&mut self) {
        self.groups.pop();
    }
    fn allKeys(&self) -> PyResult<Vec<String>> {
        self.children(true)
    }
    fn childKeys(&self) -> PyResult<Vec<String>> {
        self.children(false)
    }
    fn contains(&self, key: &str) -> PyResult<bool> {
        Ok(self.read(&self.key(key))?.is_some())
    }
    #[pyo3(signature=(key, default=None, r#type=None))]
    fn value<'py>(
        &self,
        py: Python<'py>,
        key: &str,
        default: Option<Bound<'py, PyAny>>,
        r#type: Option<&Bound<'py, PyType>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let obj = match self.read(&self.key(key))? {
            Some(v) => v.to_py(py)?,
            None => default.unwrap_or_else(|| py.None().into_bound(py)),
        };
        let Some(ty) = r#type else { return Ok(obj) };
        if obj.is_none() {
            return ty.call0();
        }
        if ty.is(&py.get_type::<PyList>()) && obj.is_instance_of::<PyString>() {
            return Ok(PyList::new(py, [obj])?.into_any());
        }
        if ty.is(&py.get_type::<PyBool>()) && obj.is_instance_of::<PyString>() {
            let s: String = obj.extract()?;
            return Value::Bool(!matches!(
                s.to_ascii_lowercase().as_str(),
                "" | "false" | "0"
            ))
            .to_py(py);
        }
        ty.call1((obj,))
    }
    fn setValue(&self, key: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let k = self.key(key);
        if k.is_empty() {
            return Err(PyValueError::new_err("Empty preference key"));
        }
        let value = Value::from_py(value)?;
        match &self.backend {
            Backend::Ini(p) => Ok(ini::update(p, &k, Some(&value))?),
            #[cfg(target_os = "macos")]
            Backend::Mac => mac::put(&k, Some(&value)),
            #[cfg(windows)]
            Backend::Registry => registry::put(&k, Some(&value)),
        }
    }
    fn remove(&self, key: &str) -> PyResult<()> {
        let k = self.key(key);
        match &self.backend {
            Backend::Ini(p) => Ok(ini::update(p, &k, None)?),
            #[cfg(target_os = "macos")]
            Backend::Mac => mac::remove(&k),
            #[cfg(windows)]
            Backend::Registry => registry::remove(&k),
        }
    }
    // Writes publish immediately; keep the existing caller contract.
    fn sync(&self) -> PyResult<()> {
        #[cfg(target_os = "macos")]
        if matches!(self.backend, Backend::Mac) {
            return mac::sync();
        }
        Ok(())
    }
}
