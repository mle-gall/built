// SPDX-License-Identifier: GPL-3.0-or-later
//! Collection manifests reference immutable snapshots. Publishing a manifest is
//! the last step, so browsing and placement can continue during an import.
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};

static JOB: Mutex<(bool, String)> = Mutex::new((false, String::new()));
pub(crate) static REVISION: AtomicU64 = AtomicU64::new(0);

fn progress(active: bool, msg: String) {
    *JOB.lock().unwrap() = (active, msg);
    REVISION.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn status() -> String {
    JOB.lock().unwrap().1.clone()
}

fn root() -> PyResult<PathBuf> {
    let profile = std::env::var_os("BUILT_SETTINGS_DIR").map(PathBuf::from);
    Ok(crate::platform_paths::local_data_directory(profile)
        .ok_or_else(|| PyRuntimeError::new_err("Library directory unavailable"))?
        .join("Built")
        .join("collections"))
}

pub(crate) fn manifests() -> PyResult<Vec<Value>> {
    let mut result = Vec::new();
    let dir = root()?;
    if !dir.exists() {
        return Ok(result);
    }
    for entry in fs::read_dir(dir)? {
        let path = entry?.path().join("collection.json");
        if path.is_file() {
            result.push(
                serde_json::from_slice(&fs::read(path)?)
                    .map_err(|e| PyValueError::new_err(e.to_string()))?,
            );
        }
    }
    result.sort_by_key(|v: &Value| v["name"].as_str().unwrap_or("").to_lowercase());
    Ok(result)
}

fn digest(py: Python<'_>, text: &str) -> PyResult<String> {
    py.import("hashlib")?
        .call_method1("sha256", (pyo3::types::PyBytes::new(py, text.as_bytes()),))?
        .call_method0("hexdigest")?
        .extract()
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect(&path, files)?;
        } else if kind.is_file()
            && path
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("rfa"))
        {
            files.push(path);
        }
    }
    Ok(())
}

// Catalog rows describe parameters. A new row without saved type geometry uses
// the reader's dimension-based placeholder instead of another type's mesh.
fn merge_catalog(py: Python<'_>, path: &Path, data: &mut Value) -> PyResult<()> {
    let path = path.with_extension("txt");
    if !path.is_file() {
        return Ok(());
    }
    let bytes = fs::read(path)?;
    let encoding = if bytes.starts_with(&[255, 254]) || bytes.starts_with(&[254, 255]) {
        "utf-16"
    } else {
        "utf-8-sig"
    };
    let text: String = pyo3::types::PyBytes::new(py, &bytes)
        .call_method1("decode", (encoding,))?
        .extract()?;
    let delimiter = text
        .chars()
        .next()
        .filter(|c| matches!(c, ',' | ';' | '\t'))
        .ok_or_else(|| PyValueError::new_err("Unknown type catalog delimiter"))?;
    let kw = pyo3::types::PyDict::new(py);
    kw.set_item("delimiter", delimiter.to_string())?;
    let input = py.import("io")?.call_method1("StringIO", (text,))?;
    let table = py
        .import("csv")?
        .getattr("reader")?
        .call((input,), Some(&kw))?;
    let mut rows = table.try_iter()?;
    let header: Vec<String> = rows
        .next()
        .transpose()?
        .ok_or_else(|| PyValueError::new_err("Empty type catalog"))?
        .extract()?;
    let types = data["types"]
        .as_array_mut()
        .ok_or_else(|| PyValueError::new_err("Invalid family types"))?;
    for row in rows {
        let row: Vec<String> = row?.extract()?;
        if row.is_empty() {
            continue;
        }
        if row.len() != header.len() || row[0].trim().is_empty() {
            return Err(PyValueError::new_err("Incomplete catalog row"));
        }
        let name = row[0].trim();
        let index = types
            .iter()
            .position(|t| t["name"] == name)
            .unwrap_or_else(|| {
                types.push(
                    json!({"name":name,"geometry_source":"none","components":[],"parameters":[]}),
                );
                types.len() - 1
            });
        let params = types[index]["parameters"]
            .as_array_mut()
            .ok_or_else(|| PyValueError::new_err("Invalid type parameters"))?;
        for (key, value) in header.iter().zip(row.iter()).skip(1) {
            let mut parts = key.split("##");
            let param = parts.next().unwrap_or("").trim();
            let kind = parts.next().unwrap_or("Text");
            let units = match parts.next().unwrap_or("").to_ascii_lowercase().as_str() {
                "millimeters" => "mm".into(),
                "centimeters" => "cm".into(),
                "meters" => "m".into(),
                "inches" => "in".into(),
                "feet" => "ft".into(),
                other => other.to_owned(),
            };
            params.retain(|p| p["name"] != param);
            params.push(json!({"name":param,"kind":kind,"units":units,"value":value}));
        }
    }
    Ok(())
}

fn import_file(
    py: Python<'_>,
    path: &Path,
    dir: &Path,
    source: &str,
    relative: &str,
) -> PyResult<(Vec<Value>, Vec<String>)> {
    let raw: String = py
        .import("built_rfa")?
        .call_method1("read_family", (path,))?
        .extract()?;
    let mut data: Value =
        serde_json::from_str(&raw).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let mut catalog_data = data.clone();
    let catalog_warning = match merge_catalog(py, path, &mut catalog_data) {
        Ok(()) => {
            data = catalog_data;
            None
        }
        Err(e) => Some(format!("Catalog: {e}")),
    };
    let raw = data.to_string();
    let types = data["types"]
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or_else(|| PyValueError::new_err("No family types"))?;
    let file = dir.join(format!("{}.json", digest(py, relative)?));
    fs::write(&file, &raw)?;
    let payload = py.import("json")?.call_method1("loads", (raw,))?;
    // Draw type geometry for each row, even if the family has a shared cover image.
    payload.set_item("preview_png_base64", py.None())?;
    let defs = py
        .import("built_core")?
        .call_method1("rfa_library_definitions", (&payload,))?;
    let mut rows = Vec::new();
    let mut warnings: Vec<String> =
        serde_json::from_value(data["warnings"].clone()).unwrap_or_default();
    warnings.extend(catalog_warning);
    let family = data["family"]["name"].as_str().unwrap_or("");
    let category = data["family"]["category"].as_str().unwrap_or("");
    for (index, ty) in types.iter().enumerate() {
        let name = ty["name"].as_str().unwrap_or("");
        let image = file.with_extension(format!("{index}.png"));
        let raster = py
            .import("built_render")?
            .call_method1("component_raster", (defs.get_item(index)?, 192))?;
        if !raster.call_method1("save", (&image,))?.is_truthy()? {
            return Err(PyRuntimeError::new_err("Cannot save preview"));
        }
        let hosted = matches!(
            category.to_ascii_lowercase().as_str(),
            "door" | "doors" | "window" | "windows"
        );
        let mut defaults = vec![
            json!(["Family", family, ""]),
            json!(["Category", category, ""]),
            json!(["Host", if hosted { "Wall" } else { "Free placement" }, ""]),
        ];
        for p in ty["parameters"].as_array().into_iter().flatten() {
            defaults.push(json!([
                p["name"].as_str().unwrap_or(""),
                format!(
                    "{} {}",
                    p["value"].as_str().unwrap_or(""),
                    p["units"].as_str().unwrap_or("")
                )
                .trim(),
                ""
            ]));
        }
        if ty["geometry_source"] == "none" {
            warnings.push(format!(
                "{name}: no saved geometry, using a size placeholder"
            ));
        }
        rows.push(
            json!({"key":format!("family:{source}:{}",digest(py, &format!("{relative}\0{name}"))?),
            "name": name, "label": name, "category": family, "source":source, "defaults":defaults,
            "thumb":image,"file":file,"index":index,"family_category":category,"verbs":["apply"]}),
        );
    }
    Ok((rows, warnings))
}

fn import_folder(folder: &Path, name: &str) -> PyResult<String> {
    let id = Python::attach(|py| digest(py, &folder.to_string_lossy()))?;
    let source = format!("collection:{id}");
    let dir = root()?.join(id);
    fs::create_dir_all(&dir)?;
    let snapshot: PathBuf = Python::attach(|py| {
        py.import("tempfile")?
            .call_method1("mkdtemp", ("", "assets-", &dir))?
            .extract()
    })?;
    let mut files = Vec::new();
    collect(folder, &mut files)?;
    files.sort();
    let mut rows = Vec::new();
    let mut reports = Vec::new();
    for (i, path) in files.iter().enumerate() {
        let relative = path.strip_prefix(folder).unwrap().to_string_lossy();
        progress(
            true,
            format!("{name} · {} of {} · {relative}", i + 1, files.len()),
        );
        match Python::attach(|py| import_file(py, path, &snapshot, &source, &relative)) {
            Ok((items, warnings)) => {
                rows.extend(items);
                reports.extend(warnings.into_iter().map(|w| format!("{relative}: {w}")));
            }
            Err(e) => reports.push(format!("{relative}: {e}")),
        }
    }
    if files.is_empty() {
        reports.push("No .rfa files in this folder".into());
    }
    let summary = format!("{name} · {} types · {} messages", rows.len(), reports.len());
    let manifest = json!({"name":name,"source":source,"items":rows,"reports":reports});
    let pending = snapshot.join("manifest.tmp");
    fs::write(&pending, manifest.to_string())?;
    fs::rename(pending, dir.join("collection.json"))?;
    Ok(summary)
}

#[pyfunction]
#[pyo3(signature=(folder,name=None))]
pub(crate) fn import_collection(folder: PathBuf, name: Option<String>) -> PyResult<()> {
    let folder = folder.canonicalize()?;
    if !folder.is_dir() {
        return Err(PyValueError::new_err("Choose a folder"));
    }
    let name = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| {
        folder
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    });
    let mut job = JOB.lock().unwrap();
    if job.0 {
        return Err(PyRuntimeError::new_err("A collection is already importing"));
    }
    *job = (true, format!("Reading {name}…"));
    REVISION.fetch_add(1, Ordering::Relaxed);
    drop(job);
    std::thread::spawn(move || {
        let result = import_folder(&folder, &name);
        progress(false, result.unwrap_or_else(|e| format!("{name}: {e}")));
    });
    Ok(())
}
