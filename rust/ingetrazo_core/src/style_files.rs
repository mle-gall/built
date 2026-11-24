//! Interchange of the document's drafting catalogues.
use pyo3::{prelude::*, types::PyList};
use std::{
    fs::File,
    hash::{BuildHasher, RandomState},
    io::{self, Write},
    path::{Path, PathBuf},
};

fn publish(path: &Path, text: &str) -> io::Result<()> {
    let (temp, mut file) = loop {
        let token = RandomState::new().hash_one(path);
        let temp = path.with_file_name(format!(".{token:016x}.part"));
        match File::create_new(&temp) {
            Ok(file) => break (temp, file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };
    let result = file
        .write_all(text.as_bytes())
        .and_then(|_| file.sync_all());
    drop(file);
    let result = result.and_then(|_| std::fs::rename(&temp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}

#[pyfunction]
pub fn export_style_file(scene: &Bound<'_, PyAny>, path: PathBuf, format: &str) -> PyResult<usize> {
    let py = scene.py();
    let rows = PyList::empty(py);
    let (backend, table, writer) = match format {
        "pat" => {
            let h = py.import("core.hatches")?;
            let info = h.getattr("HATCH_INFO")?;
            let labels = h.getattr("HATCH_LABELS")?;
            for key in h.getattr("HATCH_ORDER")?.try_iter()? {
                let key = key?;
                let families = h.call_method1("line_families", (&key,))?;
                if families.is_truthy()? {
                    let alias = info.get_item(&key)?.get_item(0)?.extract::<String>()?;
                    rows.append((format!("IT-{alias}"), labels.get_item(&key)?, families))?;
                }
            }
            (h, "hatch_patterns", "to_pat")
        }
        "lin" => (py.import("core.layers")?, "line_types", "to_lin"),
        _ => {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "Unknown pattern format",
            ))
        }
    };
    let entries = scene.getattr(table)?;
    let keys = py.import("builtins")?.call_method1("sorted", (&entries,))?;
    for name in keys.try_iter()? {
        let name = name?;
        let entry = entries.get_item(&name)?;
        let label = entry.call_method1("get", ("label", &name))?;
        if format == "pat" {
            let lines = entry.call_method1("get", ("families",))?;
            if lines.is_truthy()? {
                rows.append((name, label, lines))?;
            }
        } else {
            let pattern = entry.get_item("pattern")?;
            let spec = backend.call_method1("clean_line_text", (&entry, &pattern))?;
            rows.append((name, label, pattern, spec))?;
        }
    }
    let text: String = backend.call_method1(writer, (&rows,))?.extract()?;
    py.detach(|| publish(&path, &text))?;
    Ok(rows.len())
}

/// Round-trip the office tag catalogue using the same stable identities as copy/paste.
#[pyfunction]
pub fn tag_template_file(scene: &Bound<'_, PyAny>, path: PathBuf, save: bool) -> PyResult<String> {
    let py = scene.py();
    let backend = py.import("core.layers")?;
    let json = py.import("json")?;
    if save {
        let data = backend.call_method1("template_of", (scene,))?;
        let options = pyo3::types::PyDict::new(py);
        options.set_item("ensure_ascii", false)?;
        options.set_item("indent", 2)?;
        let text: String = json
            .call_method("dumps", (data,), Some(&options))?
            .extract()?;
        py.detach(|| publish(&path, &text))?;
        return Ok("Layer template saved".into());
    }
    if scene.hasattr("worksession_read_only")?
        && scene.getattr("worksession_read_only")?.is_truthy()?
    {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "This document is read-only",
        ));
    }
    let text = py.detach(|| std::fs::read_to_string(path))?;
    let data = json.call_method1("loads", (text,))?;
    // The existing importer applies folders and tables before it reaches individual tags.
    // Validate the entire input on an empty document before updating the open one.
    let scratch = py.import("core.scene")?.call_method0("Scene")?;
    backend
        .call_method1("apply_template", (&scratch, &data))
        .map_err(|e| {
            pyo3::exceptions::PyValueError::new_err(format!("Invalid layer template: {e}"))
        })?;
    let (added, updated): (usize, usize) = backend
        .call_method1("apply_template", (scene, data))?
        .extract()?;
    py.import("core.history")?
        .call_method1("_dirty_group_chunks", (scene,))?;
    scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
    Ok(format!("{added} layers added, {updated} updated"))
}
