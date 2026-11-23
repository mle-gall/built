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
