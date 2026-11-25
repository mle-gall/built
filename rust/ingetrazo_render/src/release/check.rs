// SPDX-License-Identifier: GPL-3.0-or-later
//! Installed-build update policy and the bounded GitHub release response.
use pyo3::{exceptions::PyException, prelude::*, types::PyDict};

type Obj<'p> = Bound<'p, PyAny>;
const PAGE: &str = "https://github.com/mle-gall/built/releases";
const API: &str = "https://api.github.com/repos/mle-gall/built/releases/latest";
const LAST: &str = "updates/last_check";
const AUTO: &str = "updates/check_at_start";
const SKIPPED: &str = "updates/skipped_version";
const LIMIT: usize = 2_000_000;
pyo3::create_exception!(release_check, UpdateCheckError, PyException);

#[pyclass(frozen, get_all, eq, module = "ingetrazo_render.release_check")]
#[derive(PartialEq)]
struct Release {
    version: String,
    date: String,
    notes: String,
    url: String,
}
#[pymethods]
impl Release {
    #[new]
    fn new(version: String, date: String, notes: String, url: String) -> Self {
        Self {
            version,
            date,
            notes,
            url,
        }
    }
}

// Compare each decimal field by significant digits. Leading zeroes have no
// weight, trailing zero fields are optional and no machine integer can overflow.
fn version(text: &str) -> Option<Vec<&str>> {
    let text = text.trim();
    let text = text.strip_prefix('v').unwrap_or(text);
    text.split('.')
        .map(|field| {
            (!field.is_empty() && field.bytes().all(|b| b.is_ascii_digit()))
                .then(|| field.trim_start_matches('0'))
        })
        .collect()
}
#[pyfunction]
#[pyo3(signature=(remote,local=None))]
fn is_newer(py: Python<'_>, remote: &str, local: Option<&str>) -> PyResult<bool> {
    let installed;
    let local = match local {
        Some(s) => s,
        None => {
            installed = py
                .import("core.version")?
                .getattr("__version__")?
                .extract::<String>()?;
            &installed
        }
    };
    let (Some(a), Some(b)) = (version(remote), version(local)) else {
        return Ok(false);
    };
    for i in 0..a.len().max(b.len()) {
        let a = a.get(i).copied().unwrap_or("");
        let b = b.get(i).copied().unwrap_or("");
        let cmp = (a.len(), a).cmp(&(b.len(), b));
        if !cmp.is_eq() {
            return Ok(cmp.is_gt());
        }
    }
    Ok(false)
}
#[pyfunction]
fn parse_release(payload: &Obj<'_>) -> PyResult<Release> {
    let tag = payload
        .get_item("tag_name")
        .map_err(|_| UpdateCheckError::new_err("no tag in the answer"))?;
    let tag = tag.str()?.to_string();
    let number = tag.trim_start_matches('v');
    if version(number).is_none() {
        return Err(UpdateCheckError::new_err(format!(
            "unreadable version: {tag:?}"
        )));
    }
    let field = |key: &str| -> PyResult<String> {
        let v = payload.call_method1("get", (key,))?;
        Ok(if v.is_truthy()? {
            v.str()?.to_string()
        } else {
            String::new()
        })
    };
    let url = field("html_url")?;
    Ok(Release {
        version: number.into(),
        date: field("published_at")?.chars().take(10).collect(),
        notes: field("body")?,
        url: if url.starts_with(&format!("{PAGE}/")) {
            url
        } else {
            format!("{PAGE}/latest")
        },
    })
}
#[pyfunction]
#[pyo3(signature=(timeout=8.0))]
fn fetch_latest(py: Python<'_>, timeout: f64) -> PyResult<Release> {
    let read = (|| -> PyResult<Obj<'_>> {
        let request = py.import("urllib.request")?;
        let headers = PyDict::new(py);
        headers.set_item(
            "User-Agent",
            py.import("core.version")?.getattr("USER_AGENT")?,
        )?;
        headers.set_item("Accept", "application/vnd.github+json")?;
        let kw = PyDict::new(py);
        kw.set_item("headers", headers)?;
        let req = request.call_method("Request", (API,), Some(&kw))?;
        let kw = PyDict::new(py);
        kw.set_item("timeout", timeout)?;
        kw.set_item(
            "context",
            py.import("ingetrazo_core.python_tls")?.call_method0("https_context")?,
        )?;
        let response = request.call_method("urlopen", (req,), Some(&kw))?;
        let raw = response.call_method1("read", (LIMIT + 1,));
        response.call_method0("close")?;
        let raw = raw?;
        if raw.len()? > LIMIT {
            return Err(UpdateCheckError::new_err("answer too large"));
        }
        py.import("json")?
            .call_method1("loads", (raw.call_method1("decode", ("utf-8",))?,))
    })();
    match read {
        Ok(payload) => parse_release(&payload),
        Err(e) if e.is_instance_of::<PyException>(py) => {
            Err(UpdateCheckError::new_err(e.value(py).to_string()))
        }
        Err(e) => Err(e),
    }
}
#[pyfunction]
#[pyo3(signature=(environ=None))]
fn store_managed(environ: Option<&Obj<'_>>) -> PyResult<bool> {
    match environ {
        Some(env) => Ok(env.call_method1("get", ("FLATPAK_ID",))?.is_truthy()?
            || env.call_method1("get", ("SNAP",))?.is_truthy()?),
        None => Ok(["FLATPAK_ID", "SNAP"]
            .iter()
            .any(|key| std::env::var_os(key).is_some_and(|v| !v.is_empty()))),
    }
}
fn seconds() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
#[pyfunction]
#[pyo3(signature=(settings,now=None,frozen=None,environ=None))]
fn should_check_at_start(
    settings: &Obj<'_>,
    now: Option<f64>,
    frozen: Option<bool>,
    environ: Option<&Obj<'_>>,
) -> PyResult<bool> {
    let py = settings.py();
    let frozen = match frozen {
        Some(v) => v,
        None => py
            .import("sys")?
            .getattr_opt("frozen")?
            .is_some_and(|v| v.is_truthy().unwrap_or(false)),
    };
    if !frozen
        || store_managed(environ)?
        || settings.call_method1("value", (AUTO, "1"))?.str()?.eq("0")
    {
        return Ok(false);
    }
    let time = settings.call_method1("value", (LAST, 0))?;
    let last = py
        .import("builtins")?
        .call_method1("float", (time,))
        .and_then(|x| x.extract::<f64>())
        .unwrap_or(0.);
    Ok(!(0.0..86400.0).contains(&(now.unwrap_or_else(seconds) - last)))
}
#[pyfunction]
#[pyo3(signature=(settings,now=None))]
fn mark_checked(settings: &Obj<'_>, now: Option<f64>) -> PyResult<()> {
    settings.call_method1("setValue", (LAST, now.unwrap_or_else(seconds)))?;
    Ok(())
}
#[pyfunction]
fn skip_version(settings: &Obj<'_>, version: &str) -> PyResult<()> {
    settings.call_method1("setValue", (SKIPPED, version))?;
    Ok(())
}
#[pyfunction]
fn is_skipped(settings: &Obj<'_>, version: &str) -> PyResult<bool> {
    Ok(settings
        .call_method1("value", (SKIPPED, ""))?
        .str()?
        .eq(version))
}

pub(crate) fn register(parent: &Bound<PyModule>) -> PyResult<()> {
    let py = parent.py();
    let m = PyModule::new(py, "ingetrazo_render.release_check")?;
    m.add_class::<Release>()?;
    m.add("RELEASES_PAGE", PAGE)?;
    m.add("KEY_LAST", LAST)?;
    m.add("KEY_AUTO", AUTO)?;
    m.add("UpdateCheckError", py.get_type::<UpdateCheckError>())?;
    for f in [
        wrap_pyfunction!(is_newer, &m)?,
        wrap_pyfunction!(parse_release, &m)?,
        wrap_pyfunction!(fetch_latest, &m)?,
        wrap_pyfunction!(store_managed, &m)?,
        wrap_pyfunction!(should_check_at_start, &m)?,
        wrap_pyfunction!(mark_checked, &m)?,
        wrap_pyfunction!(skip_version, &m)?,
        wrap_pyfunction!(is_skipped, &m)?,
    ] {
        m.add_function(f)?;
    }
    py.import("sys")?
        .getattr("modules")?
        .set_item("ingetrazo_render.release_check", &m)?;
    parent.add("release_check", m)
}
