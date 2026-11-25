// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
struct Example {
    file: PathBuf,
    title: String,
    blurb: String,
    tagline: String,
}
fn scan(py: Python) -> PyResult<Vec<Example>> {
    let root: PathBuf = py
        .import("ingetrazo_core.paths")?
        .call_method0("app_root")?
        .extract()?;
    let ast = py.import("ast")?;
    let mut out = vec![];
    for p in entries(&root.join("examples/extensions")) {
        let src = if p.is_file() && p.extension().is_some_and(|ext| ext == "py") {
            p.clone()
        } else if p.join("__init__.py").is_file() {
            p.join("__init__.py")
        } else {
            continue;
        };
        let doc = fs::read_to_string(src)
            .ok()
            .and_then(|body| ast.call_method1("parse", (body,)).ok())
            .and_then(|node| ast.call_method1("get_docstring", (node,)).ok())
            .and_then(|s| s.extract::<String>().ok())
            .unwrap_or_default();
        let (first, rest) = doc.split_once('\n').unwrap_or((&doc, ""));
        let (title, tagline) = first.split_once(" — ").unwrap_or((first, ""));
        let title = if title.trim().is_empty() {
            files::ident(&p)
        } else {
            title.trim()
        }
        .to_owned();
        let blurb = rest
            .trim()
            .split("\n\n")
            .next()
            .unwrap_or("")
            .replace('\n', " ");
        let tagline = tagline.trim();
        let tagline = if let Some(c) = tagline.chars().next() {
            format!("{}{}", c.to_uppercase(), &tagline[c.len_utf8()..])
        } else {
            blurb.clone()
        };
        out.push(Example {
            file: p,
            title,
            blurb,
            tagline,
        });
    }
    Ok(out)
}
#[pyfunction]
fn example_extension_files(py: Python) -> PyResult<Vec<(Obj, String, String)>> {
    scan(py)?
        .into_iter()
        .map(|e| Ok((path(py, &e.file)?, e.title, e.blurb)))
        .collect()
}
pub(super) fn bundle_hash(py: Python, p: &Path) -> PyResult<String> {
    if p.is_file() {
        return digest(py, &fs::read(p)?);
    }
    let h = py.import("hashlib")?.call_method0("sha256")?;
    for f in files::descendants(p) {
        if f.extension().is_some_and(|s| s == "pyc") {
            continue;
        }
        let rel = f
            .strip_prefix(p)
            .unwrap()
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        h.call_method1("update", (PyBytes::new(py, format!("{rel}\0").as_bytes()),))?;
        h.call_method1("update", (PyBytes::new(py, &fs::read(f)?),))?;
    }
    h.call_method0("hexdigest")?.extract()
}
#[pyfunction]
fn bundled_extensions(py: Python) -> PyResult<Vec<Obj>> {
    let i18n = py.import("core.i18n")?;
    let lang = i18n.call_method0("current_language")?.extract::<String>()?;
    let lang = lang.split(['-', '_']).next().unwrap_or("").to_lowercase();
    let version = version(py)?;
    let class = module(py)?.getattr("Extension")?;
    let mut out = vec![];
    for e in scan(py)? {
        let p = e.file;
        let id = files::ident(&p);
        let names = PyDict::new(py);
        names.set_item("en", e.title)?;
        let summary = PyDict::new(py);
        summary.set_item("en", &e.tagline)?;
        summary.set_item(&lang, i18n.call_method1("tr", (&e.tagline,))?)?;
        let image_start = if p.is_dir() {
            p.join("screenshot")
        } else {
            p.with_extension("")
        };
        let image = ["png", "jpg", "jpeg", "webp"]
            .map(|ext| image_start.with_extension(ext))
            .into_iter()
            .find(|p| p.is_file());
        let kw = PyDict::new(py);
        kw.set_item(
            "screenshot",
            image.map(|p| p.to_string_lossy().into_owned()),
        )?;
        kw.set_item("bundled", p.to_string_lossy().as_ref())?;
        match bundle_hash(py, &p) {
            Ok(hash) => out.push(class.call(
                (
                    id,
                    &version,
                    "IngeTrazo",
                    "",
                    "",
                    "",
                    hash,
                    &version,
                    names,
                    summary,
                    PyTuple::empty(py),
                ),
                Some(&kw),
            )?),
            Err(e) if e.is_instance_of::<pyo3::exceptions::PyOSError>(py) => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}
pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(example_extension_files, m)?)?;
    m.add_function(wrap_pyfunction!(bundled_extensions, m)?)
}
