// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
const CATALOG: &str =
    "https://raw.githubusercontent.com/ingelibre/ingetrazo-extensions/main/catalog.json";
const REVIEWED: &str =
    "https://raw.githubusercontent.com/ingelibre/ingetrazo-extensions/main/reviewed.toml";
const LIMIT: usize = 2 << 20;
#[pyfunction]
pub(super) fn download<'p>(py: Python<'p>, url: &str, limit: usize) -> PyResult<Obj<'p>> {
    if !url.starts_with("https://") {
        return Err(err("The download requires an HTTPS address."));
    }
    let request = py.import("urllib.request")?;
    let kw = PyDict::new(py);
    let headers = PyDict::new(py);
    headers.set_item(
        "User-Agent",
        py.import("built_core.version")?.getattr("USER_AGENT")?,
    )?;
    kw.set_item("headers", headers)?;
    let req = request.call_method("Request", (url,), Some(&kw))?;
    kw.clear();
    kw.set_item(
        "context",
        py.import("built_core.python_tls")?.call_method0("https_context")?,
    )?;
    kw.set_item("timeout", 20)?;
    let response = request
        .call_method("urlopen", (req,), Some(&kw))
        .map_err(|e| err(format!("Could not download {url}: {e}")))?;
    let read = (|| {
        if !response
            .call_method0("geturl")?
            .extract::<String>()?
            .starts_with("https://")
        {
            return Err(err("The download redirected to a non-HTTPS address."));
        }
        let bytes = response.call_method1("read", (limit + 1,))?;
        if bytes.len()? > limit {
            return Err(err("The downloaded file is too large."));
        }
        Ok(bytes)
    })();
    response.call_method0("close")?;
    read
}
fn image(s: &str) -> bool {
    !s.contains(['/', '\\', ':'])
        && Path::new(s)
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|ext| {
                matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "png" | "jpg" | "jpeg" | "webp"
                )
            })
}
#[pyfunction]
fn parse_extension<'p>(raw: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = raw.py();
    if raw.cast::<PyDict>().is_err() {
        return Err(err("Invalid extension entry in catalog."));
    }
    let id = key(raw, "id")?.extract::<String>().unwrap_or_default();
    require_id(&id)?;
    let d = PyDict::new(py);
    d.set_item("id", &id)?;
    for (a, b) in [
        ("version", "version"),
        ("author", "author"),
        ("license", "license"),
        ("repository", "repository"),
        ("download", "download"),
        ("sha256", "sha256"),
        ("ingetrazo", "minimum_version"),
    ] {
        let mut v = key(raw, a)?
            .extract::<String>()
            .ok()
            .filter(|x| !x.is_empty())
            .ok_or_else(|| err(format!("Extension '{id}' has incomplete catalog metadata.")))?;
        if a == "sha256" {
            if v.len() != 64 || !v.bytes().all(|x| x.is_ascii_hexdigit()) {
                return Err(err(format!(
                    "Extension '{id}' has an invalid SHA-256 digest."
                )));
            }
            v.make_ascii_lowercase();
        }
        if a == "download" && !v.starts_with("https://") {
            return Err(err(format!(
                "Extension '{id}' does not use an HTTPS download URL."
            )));
        }
        d.set_item(b, v)?;
    }
    for k in ["name", "summary"] {
        let v = key(raw, k)?;
        let ok = v.cast::<PyDict>().is_ok_and(|map| {
            map.iter()
                .all(|(a, b)| a.is_instance_of::<PyString>() && b.is_instance_of::<PyString>())
        });
        if !ok {
            return Err(err(format!("Invalid {k} in extension catalog.")));
        }
        d.set_item(k, v)?;
    }
    let tags = raw.call_method1("get", ("tags", PyList::empty(py)))?;
    if !tags.is_instance_of::<PyList>() {
        return Err(err("Invalid tags in extension catalog."));
    }
    d.set_item(
        "tags",
        PyTuple::new(
            py,
            tags.extract::<Vec<String>>()
                .map_err(|_| err("Invalid tags in extension catalog."))?,
        )?,
    )?;
    for k in ["updated_at", "screenshot"] {
        let v = key(raw, k)?;
        if !v.is_none()
            && !v
                .extract::<String>()
                .is_ok_and(|v| k == "updated_at" || image(&v))
        {
            return Err(err(format!("Extension '{id}' has invalid {k} metadata.")));
        }
        d.set_item(k, v)?;
    }
    d.set_item(
        "thumb",
        key(raw, "thumb")?
            .extract::<String>()
            .ok()
            .filter(|v| image(v)),
    )?;
    d.set_item(
        "screenshot_sha",
        key(raw, "screenshot_sha")?
            .extract::<String>()
            .ok()
            .filter(|v| {
                v.len() == 16
                    && v.bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            }),
    )?;
    module(py)?.getattr("Extension")?.call((), Some(&d))
}
fn snapshot<'p>(
    py: Python<'p>,
    catalog: &Obj,
    review: &str,
    stamp: &str,
    cached: bool,
) -> PyResult<Obj<'p>> {
    if !catalog.is_instance_of::<PyDict>() || !key(catalog, "format")?.eq(1)? {
        return Err(err("Unsupported extension catalog format."));
    }
    let raw = key(catalog, "extensions")?;
    let raw = raw
        .cast::<PyList>()
        .map_err(|_| err("The extension catalog has no entries."))?;
    let mut ids = HashSet::new();
    let entries = PyList::empty(py);
    for e in raw {
        let ext = parse_extension(&e)?;
        if !ids.insert(s(&ext, "id")?) {
            return Err(err("The catalog contains duplicate IDs."));
        }
        entries.append(ext)?;
    }
    let reviewed = py
        .import("tomllib")?
        .call_method1("loads", (review,))
        .map_err(|e| err(format!("The extension catalog is invalid: {e}")))?;
    module(py)?.call_method1("CatalogSnapshot", (entries, reviewed, stamp, cached))
}
#[pyfunction]
fn catalog_cache_path(py: Python) -> PyResult<Obj> {
    let root = root(py)?;
    path(
        py,
        &root
            .parent()
            .unwrap_or(Path::new("."))
            .join("catalog-cache.json"),
    )
}
fn cached_path(py: Python) -> PyResult<PathBuf> {
    module(py)?.call_method0("catalog_cache_path")?.extract()
}
#[pyfunction]
fn load_cached_catalog(py: Python) -> PyResult<Option<Obj>> {
    let p = cached_path(py)?;
    let bytes = match fs::read(&p) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            warn(py, &format!("could not read extension catalog cache: {e}"))?;
            return Ok(None);
        }
    };
    let read = (|| {
        let d = py
            .import("json")?
            .call_method1("loads", (PyBytes::new(py, &bytes),))?;
        snapshot(
            py,
            &d.get_item("catalog")?,
            &d.get_item("reviewed")?.extract::<String>()?,
            &d.get_item("fetched_at")?.extract::<String>()?,
            true,
        )
    })();
    match read {
        Ok(v) => Ok(Some(v)),
        Err(e) => {
            warn(
                py,
                &format!("invalid extension catalog cache {}: {e}", p.display()),
            )?;
            Ok(None)
        }
    }
}
pub(super) fn warn(py: Python, text: &str) -> PyResult<()> {
    py.import("logging")?
        .call_method1("getLogger", ("built.extensions",))?
        .call_method1("warning", (text,))?;
    Ok(())
}
#[pyfunction]
fn refresh_catalog(py: Python) -> PyResult<Obj> {
    let api = module(py)?;
    let load: PyResult<(Obj, String)> = (|| {
        let data = api.call_method1("download", (CATALOG, LIMIT))?;
        let review = api
            .call_method1("download", (REVIEWED, LIMIT))?
            .call_method1("decode", ("utf-8",))?
            .extract::<String>()?;
        let dt = py.import("datetime")?;
        let stamp = dt
            .getattr("datetime")?
            .call_method1("now", (dt.getattr("timezone")?.getattr("utc")?,))?
            .call_method1("isoformat", ("T", "seconds"))?
            .extract::<String>()?;
        let d = py.import("json")?.call_method1("loads", (data,))?;
        let snap = snapshot(py, &d, &review, &stamp, false)?;
        let data = PyDict::new(py);
        data.set_item("catalog", d)?;
        data.set_item("reviewed", review)?;
        data.set_item("fetched_at", stamp)?;
        let json = py
            .import("json")?
            .call_method1("dumps", (data,))?
            .extract::<String>()?;
        Ok((snap, json))
    })();
    let (snap, json) = match load {
        Ok(x) => x,
        Err(e) => {
            if let Some(v) = load_cached_catalog(py)? {
                return Ok(v);
            }
            return Err(if e.is_instance_of::<ExtensionManagerError>(py) {
                e
            } else {
                err(format!("The extension catalog is invalid: {e}"))
            });
        }
    };
    let p = cached_path(py)?;
    let write = fs::create_dir_all(p.parent().unwrap_or(Path::new(".")))
        .map_err(|e| e.to_string())
        .and_then(|()| crate::builtproject::manifest::write_atomic(&p, &json));
    if let Err(e) = write {
        warn(py, &format!("could not cache extension catalog: {e}"))?
    }
    Ok(snap)
}
fn language_label(ext: &Obj, attr: &str, lang: &str, default: String) -> PyResult<String> {
    let labels = ext.getattr(attr)?.cast_into::<PyDict>()?;
    let key = lang.split(['-', '_']).next().unwrap_or("").to_lowercase();
    for k in [&key, "en"] {
        if let Some(v) = labels.get_item(k)? {
            if v.is_truthy()? {
                return v.extract();
            }
        }
    }
    labels
        .values()
        .iter()
        .next()
        .map(|v| v.extract())
        .unwrap_or(Ok(default))
}
#[pyfunction]
fn display_name(extension: &Obj, language: &str) -> PyResult<String> {
    language_label(extension, "name", language, s(extension, "id")?)
}
#[pyfunction]
fn display_summary(extension: &Obj, language: &str) -> PyResult<String> {
    language_label(extension, "summary", language, String::new())
}
#[pyfunction]
fn is_reviewed(extension: &Obj, reviewed: &Obj) -> PyResult<bool> {
    let hash = reviewed
        .get_item(s(extension, "id")?)
        .and_then(|v| v.get_item("sha256"))
        .and_then(|h| h.extract::<String>())
        .unwrap_or_default();
    Ok(hash.eq_ignore_ascii_case(&s(extension, "sha256")?))
}
fn parts(v: &str) -> Vec<u64> {
    let v = v.trim_start();
    let end = v
        .bytes()
        .take_while(|c| c.is_ascii_digit() || *c == b'.')
        .count();
    v[..end].split('.').map_while(|n| n.parse().ok()).collect()
}
#[pyfunction]
pub(super) fn compatible_with_current(extension: &Obj) -> PyResult<bool> {
    let mut required = parts(&s(extension, "minimum_version")?);
    let mut current = parts(&version(extension.py())?);
    if current.is_empty() || required.is_empty() {
        return Ok(false);
    }
    let width = current.len().max(required.len());
    current.resize(width, 0);
    required.resize(width, 0);
    Ok(required <= current)
}
#[pyfunction]
fn tested_version_is_old(extension: &Obj) -> PyResult<bool> {
    let a = parts(&s(extension, "minimum_version")?);
    let b = parts(&version(extension.py())?);
    Ok(a.len() >= 2
        && b.len() >= 2
        && (a[0] < b[0] || a[0] == b[0] && b[1].saturating_sub(a[1]) >= 2))
}
pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add("CATALOG_URL", CATALOG)?;
    m.add("REVIEWED_URL", REVIEWED)?;
    for f in [
        wrap_pyfunction!(parse_extension, m)?,
        wrap_pyfunction!(download, m)?,
        wrap_pyfunction!(catalog_cache_path, m)?,
        wrap_pyfunction!(load_cached_catalog, m)?,
        wrap_pyfunction!(refresh_catalog, m)?,
        wrap_pyfunction!(display_name, m)?,
        wrap_pyfunction!(display_summary, m)?,
        wrap_pyfunction!(is_reviewed, m)?,
        wrap_pyfunction!(compatible_with_current, m)?,
        wrap_pyfunction!(tested_version_is_old, m)?,
    ] {
        m.add_function(f)?
    }
    Ok(())
}
