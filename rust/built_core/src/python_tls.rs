// SPDX-License-Identifier: GPL-3.0-or-later
//! Trust roots for Python plugin and urllib HTTPS requests in a packaged interpreter.
use pyo3::{
    exceptions::{PyImportError, PyOSError},
    prelude::*,
    types::PyDict,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

type Obj<'p> = Bound<'p, PyAny>;
const SYSTEM_BUNDLES: [&str; 5] = [
    "/etc/ssl/cert.pem",
    "/etc/ssl/certs/ca-certificates.crt",
    "/etc/pki/tls/certs/ca-bundle.crt",
    "/etc/ssl/ca-bundle.pem",
    "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem",
];

fn windows(py: Python, platform: Option<&str>) -> PyResult<bool> {
    match platform {
        Some(s) => Ok(s.starts_with("win")),
        None => Ok(py
            .import("sys")?
            .getattr("platform")?
            .extract::<String>()?
            .starts_with("win")),
    }
}

fn has_certs(paths: &Obj<'_>) -> PyResult<bool> {
    for attr in ["cafile", "openssl_cafile"] {
        if paths
            .getattr(attr)?
            .extract::<Option<PathBuf>>()?
            .is_some_and(|p| p.is_file())
        {
            return Ok(true);
        }
    }
    for attr in ["capath", "openssl_capath"] {
        if let Some(p) = paths.getattr(attr)?.extract::<Option<PathBuf>>()? {
            if p.is_dir() && fs::read_dir(p)?.next().transpose()?.is_some() {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn certifi(py: Python) -> PyResult<Option<PathBuf>> {
    match py.import("certifi") {
        Ok(module) => module.call_method0("where")?.extract(),
        Err(e) if e.is_instance_of::<PyImportError>(py) => Ok(None),
        Err(e) => Err(e),
    }
}

#[pyfunction]
#[pyo3(signature=(paths=None, candidates=None, environ=None, platform=None))]
fn ensure_ca_bundle(
    py: Python,
    paths: Option<&Obj<'_>>,
    candidates: Option<Vec<PathBuf>>,
    environ: Option<&Obj<'_>>,
    platform: Option<&str>,
) -> PyResult<Option<String>> {
    if windows(py, platform)? {
        return Ok(None);
    }
    let env = match environ {
        Some(env) => env.clone(),
        None => py.import("os")?.getattr("environ")?,
    };
    let chosen = env
        .call_method1("get", ("SSL_CERT_FILE",))?
        .extract::<Option<String>>()?
        .filter(|s| !s.is_empty());
    if chosen.as_ref().is_some_and(|p| Path::new(p).is_file()) {
        return Ok(None);
    }
    if chosen.is_none() {
        let paths = match paths {
            Some(paths) => paths.clone(),
            None => py.import("ssl")?.call_method0("get_default_verify_paths")?,
        };
        if has_certs(&paths)? {
            return Ok(None);
        }
    }
    let bundles = candidates.unwrap_or_else(|| SYSTEM_BUNDLES.iter().map(PathBuf::from).collect());
    if let Some(path) = certifi(py)?
        .into_iter()
        .chain(bundles)
        .find(|p| p.is_file())
    {
        let name = path.to_string_lossy().into_owned();
        env.set_item("SSL_CERT_FILE", &name)?;
        return Ok(Some(name));
    }
    Ok(None)
}

#[pyfunction]
pub(crate) fn ensure_once(py: Python) -> PyResult<()> {
    let module = py.import("built_core.python_tls")?;
    if !module.getattr("_initialized")?.is_truthy()? {
        module.setattr("_initialized", true)?;
        if let Err(e) = ensure_ca_bundle(py, None, None, None, None) {
            if !e.is_instance_of::<PyOSError>(py) {
                return Err(e);
            }
        }
    }
    Ok(())
}

#[pyfunction]
#[pyo3(signature=(platform=None))]
fn https_context<'p>(py: Python<'p>, platform: Option<&str>) -> PyResult<Obj<'p>> {
    let ssl = py.import("ssl")?;
    if windows(py, platform)? {
        match py.import("truststore") {
            Ok(module) => {
                return module.call_method1("SSLContext", (ssl.getattr("PROTOCOL_TLS_CLIENT")?,))
            }
            Err(e) if e.is_instance_of::<PyImportError>(py) => (),
            Err(e) => return Err(e),
        }
    }
    ensure_once(py)?;
    let kwargs = PyDict::new(py);
    let file = py
        .import("os")?
        .getattr("environ")?
        .call_method1("get", ("SSL_CERT_FILE",))?;
    if file
        .extract::<Option<PathBuf>>()?
        .is_some_and(|p| p.is_file())
    {
        kwargs.set_item("cafile", file)?;
    }
    ssl.call_method("create_default_context", (), Some(&kwargs))
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let name = "built_core.python_tls";
    let sub = PyModule::new(m.py(), name)?;
    sub.add("_initialized", false)?;
    sub.add_function(wrap_pyfunction!(ensure_ca_bundle, &sub)?)?;
    sub.add_function(wrap_pyfunction!(ensure_once, &sub)?)?;
    sub.add_function(wrap_pyfunction!(https_context, &sub)?)?;
    m.py()
        .import("sys")?
        .getattr("modules")?
        .set_item(name, &sub)?;
    m.add("python_tls", sub)
}
