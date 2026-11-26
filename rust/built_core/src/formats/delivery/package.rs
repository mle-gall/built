// SPDX-License-Identifier: GPL-3.0-or-later
//! Issue one dated folder. An individual writer may fail without discarding its siblings.
use super::{delivery_scoped, delivery_validate, write::destination_path, Obj};
use crate::delivery_form::PACKAGE_OUTPUTS;
use pyo3::{
    exceptions::{PyInterruptedError, PyValueError},
    prelude::*,
    types::PyDict,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

mod bcf;

fn decode(v: &Obj<'_>) -> PyResult<Value> {
    let s: String = v
        .py()
        .import("json")?
        .call_method1("dumps", (v,))?
        .extract()?;
    serde_json::from_str(&s).map_err(|e| PyValueError::new_err(e.to_string()))
}
fn encode<'p>(py: Python<'p>, v: &Value) -> PyResult<Obj<'p>> {
    py.import("json")?.call_method1("loads", (v.to_string(),))
}
fn label(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or("").to_owned()
}
pub(super) fn checked_status(ids: &Value) -> String {
    let mut failed = 0;
    let mut error = None;
    for file in ids.as_array().into_iter().flatten() {
        if file["status"] != "checked" {
            error = Some(format!("IDS {}", label(file, "status")));
        }
        for rule in file["specifications"].as_array().into_iter().flatten() {
            if rule["status"] == false || rule["failed"].as_u64().unwrap_or(0) > 0 {
                failed += 1;
            }
        }
    }
    match (failed, error) {
        (0, Some(e)) => e,
        (0, None) if ids.as_array().is_none_or(Vec::is_empty) => "No IDS selected".into(),
        (0, None) => "Passed".into(),
        (1, Some(e)) => format!("1 failed rule · {e}"),
        (1, None) => "1 failed rule".into(),
        (n, Some(e)) => format!("{n} failed rules · {e}"),
        (n, None) => format!("{n} failed rules"),
    }
}

#[pyclass]
struct Package {
    scene: Py<PyAny>,
    context: Py<PyDict>,
    record: Value,
    outputs: Vec<Value>,
    directory: PathBuf,
    stage: Option<tempfile::TempDir>,
    index: usize,
    version: i64,
    summary: Value,
}
impl Package {
    fn write(&mut self, py: Python<'_>, out: &Value, root: &Path) -> PyResult<String> {
        let scene = self.scene.bind(py);
        let format = label(out, "format");
        let filename = PACKAGE_OUTPUTS.iter().find(|v| v.0 == format).unwrap().2;
        let path = root.join(filename);
        let mut r = out.clone();
        r["name"] = self.record["name"].clone();
        r["destination"] = json!(path);
        if r["node"].is_null() {
            r["node"] = self.record["node"].clone();
        }
        let record = encode(py, &r)?;
        match format.as_str() {
            "bcf" => bcf::write(scene, &path, &out["options"]["names"])?,
            "csv" => {
                let model = delivery_scoped(py, scene, &record)?;
                let text: String = py
                    .import("core.bim")?
                    .call_method1("quantities_csv", (model,))?
                    .extract()?;
                fs::write(path, text)?;
            }
            "manifest" => self.manifest(root)?,
            _ => {
                let single = py.import("built_core.deliveries")?.call_method(
                    "_write_single",
                    (scene, &record),
                    Some(self.context.bind(py)),
                )?;
                for step in single.try_iter()? {
                    step?;
                }
                if format == "ifc" {
                    let report = path.with_extension("ids-report.json");
                    let ids = if report.exists() {
                        serde_json::from_slice::<Value>(&fs::read(&report)?)
                            .map_err(|e| PyValueError::new_err(e.to_string()))?
                    } else {
                        json!([])
                    };
                    let status = checked_status(&ids);
                    fs::write(report, serde_json::to_vec_pretty(&ids).unwrap())?;
                    self.summary["ids"] = ids;
                    self.summary["ids_result"] = json!(status);
                    if status != "Passed" && status != "No IDS selected" {
                        return Ok(status);
                    }
                }
            }
        }
        Ok("Delivered".into())
    }
    fn manifest(&self, root: &Path) -> PyResult<()> {
        use sha2::{Digest, Sha256};
        let mut files = Vec::new();
        let mut text = format!(
            "{}\nModel revision {} · {}\nIDS: {}\n",
            label(&self.record, "name"),
            self.summary["revision"],
            label(&self.summary, "date"),
            label(&self.summary, "ids_result")
        );
        for result in self.summary["outputs"].as_array().into_iter().flatten() {
            text.push_str(&format!(
                "{}: {}\n",
                label(result, "format"),
                label(result, "result")
            ));
        }
        let mut entries = fs::read_dir(root)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let mut hash = Sha256::new();
            let size = std::io::copy(&mut fs::File::open(entry.path())?, &mut hash)?;
            let digest = format!("{:x}", hash.finalize());
            let name = entry.file_name().to_string_lossy().into_owned();
            text.push_str(&format!("{name}  {size} bytes  SHA-256 {digest}\n"));
            files.push(json!({"path":name,"size":size,"sha256":digest}));
        }
        let mut manifest = self.summary.clone();
        manifest["name"] = self.record["name"].clone();
        manifest["scope"] = self.record["scope"].clone();
        manifest["setup"] = self.record["outputs"].clone();
        manifest["files"] = json!(files);
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )?;
        fs::write(root.join("manifest.txt"), text)?;
        Ok(())
    }
}
#[pymethods]
impl Package {
    #[getter]
    fn summary<'p>(&self, py: Python<'p>) -> PyResult<Obj<'p>> {
        encode(py, &self.summary)
    }
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn close(&mut self) {
        self.stage.take();
    }
    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<(f64, String)>> {
        let Some(stage) = &self.stage else {
            return Ok(None);
        };
        let root = stage.path().to_owned();
        if self
            .scene
            .bind(py)
            .getattr("content_version")?
            .extract::<i64>()?
            != self.version
        {
            self.close();
            return Err(PyValueError::new_err(
                "Model changed during delivery. Run again.",
            ));
        }
        if self.index > 0 {
            let out = self.outputs[self.index - 1].clone();
            let result = match self.write(py, &out, &root) {
                Ok(status) => status,
                Err(e) if e.is_instance_of::<PyInterruptedError>(py) => {
                    self.close();
                    return Err(e);
                }
                Err(e) => format!("Failed: {}", e.value(py)),
            };
            if result != "Delivered" {
                self.summary["result"] = json!("Needs review");
            }
            self.summary["outputs"]
                .as_array_mut()
                .unwrap()
                .push(json!({"format":out["format"], "result":result}));
        }
        if let Some(next) = self.outputs.get(self.index) {
            let label = format!("Writing {}", label(next, "format").to_uppercase());
            self.index += 1;
            return Ok(Some((
                (self.index - 1) as f64 / self.outputs.len() as f64,
                label,
            )));
        }
        let name: String = label(&self.record, "name")
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || " -_".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let base = format!("{}_{}", label(&self.summary, "date"), name.trim());
        for i in 1.. {
            let dest = self.directory.join(if i == 1 {
                base.clone()
            } else {
                format!("{base}_{i}")
            });
            if dest.exists() {
                continue;
            }
            match fs::rename(&root, &dest) {
                Ok(()) => {
                    self.summary["folder"] = json!(dest);
                    break;
                }
                Err(_) if dest.exists() => continue,
                Err(e) => return Err(e.into()),
            }
        }
        self.close();
        Ok(None)
    }
}

#[pyfunction(signature=(scene,record,**context))]
fn write_steps<'p>(
    py: Python<'p>,
    scene: &Obj<'p>,
    record: &Obj<'p>,
    context: Option<&Bound<'p, PyDict>>,
) -> PyResult<Obj<'p>> {
    let ctx = context
        .map(|v| v.copy())
        .transpose()?
        .unwrap_or_else(|| PyDict::new(py));
    if !record
        .call_method1("get", ("format", "package"))?
        .eq("package")?
    {
        return py.import("built_core.deliveries")?.call_method(
            "_write_single",
            (scene, record),
            Some(&ctx),
        );
    }
    delivery_validate(record)?;
    let directory = ctx
        .get_item("project_directory")?
        .map(|p| p.extract::<Option<PathBuf>>())
        .transpose()?
        .flatten();
    let (directory, project) = destination_path(py, scene, record, directory.as_deref())?;
    fs::create_dir_all(&directory)?;
    let directory = directory.canonicalize()?;
    if let Some(project) = project {
        ctx.set_item("project_directory", project)?;
    }
    let version = scene.getattr("content_version")?.extract::<i64>()?;
    let revision = if let Some(v) = ctx.get_item("revision")? {
        ctx.del_item("revision")?;
        v.extract()?
    } else {
        version
    };
    let date = py
        .import("datetime")?
        .getattr("date")?
        .call_method0("today")?
        .str()?
        .to_string();
    let record = decode(record)?;
    let mut outputs = record["outputs"].as_array().unwrap().clone();
    outputs.sort_by_key(|o| o["format"] == "manifest");
    let stage = tempfile::Builder::new()
        .prefix(".handover-")
        .tempdir_in(&directory)?;
    Ok(Bound::new(py, Package {scene:scene.clone().unbind(),context:ctx.unbind(),record, outputs, directory,stage:Some(stage),index:0,version,
        summary:json!({"date":date,"revision":revision,"outputs":[],"ids":[],"ids_result":"Not checked","result":"Delivered"})})?.into_any())
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    functions!(m;write_steps);
    Ok(())
}
