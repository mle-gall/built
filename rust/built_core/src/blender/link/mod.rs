// SPDX-License-Identifier: GPL-3.0-or-later
//! The two-way link with Blender (the Render drawer's "Open in Blender").
//! Built sends geometry (GLB per top-level placement, then only what the
//! change feed says changed); the Built add-on in Blender sends back
//! materials, lights, cameras and placement transforms, which wait in the
//! drawer for Apply / Ignore (or Keep Built / Take Blender on a conflict).
//! Transport: `server` (loopback socket, token), `wire` (frames).
//! The add-on lives in `integrations/blender/built_link/`.
pub mod review;
pub mod scene;
pub mod server;
pub mod wire;

use pyo3::{
    exceptions::{PyOSError, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict, PyList},
};
use serde_json::{Map, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

type Obj<'py> = Bound<'py, PyAny>;

fn to_json(o: &Obj<'_>) -> PyResult<Value> {
    let s: String = o.py().import("json")?.call_method1("dumps", (o,))?.extract()?;
    serde_json::from_str(&s).map_err(|e| PyValueError::new_err(e.to_string()))
}
fn to_map(o: &Obj<'_>) -> PyResult<Map<String, Value>> {
    match to_json(o)? {
        Value::Object(m) => Ok(m),
        _ => Err(PyValueError::new_err("expected a dict")),
    }
}
fn to_py<'py>(py: Python<'py>, v: &Value) -> PyResult<Obj<'py>> {
    py.import("json")?.call_method1("loads", (v.to_string(),))
}

fn message_dict<'py>(py: Python<'py>, m: &wire::Message) -> PyResult<Obj<'py>> {
    let d = to_py(py, &Value::Object(m.header.clone()))?;
    d.set_item("_payload", PyBytes::new(py, &m.payload))?;
    Ok(d)
}

/// Built's listener for the add-on (see `server`). One per document view.
#[pyclass(module = "built_core.render_blender", unsendable)]
pub struct LinkServer {
    inner: Option<server::Server>,
    token: String,
}

#[pymethods]
impl LinkServer {
    #[new]
    #[pyo3(signature=(token=None))]
    fn new(token: Option<String>) -> Self {
        Self { inner: None, token: token.unwrap_or_else(wire::new_token) }
    }
    /// Listen on loopback; returns the port (`port` 0 or taken: any free).
    #[pyo3(signature=(port=0))]
    fn start(&mut self, port: u16) -> PyResult<u16> {
        if let Some(s) = &self.inner {
            return Ok(s.port());
        }
        let s = server::Server::start(self.token.clone(), port).map_err(|e| PyOSError::new_err(e.to_string()))?;
        let p = s.port();
        self.inner = Some(s);
        Ok(p)
    }
    #[getter]
    fn running(&self) -> bool {
        self.inner.is_some()
    }
    #[getter]
    fn port(&self) -> u16 {
        self.inner.as_ref().map_or(0, |s| s.port())
    }
    #[getter]
    fn token(&self) -> &str {
        &self.token
    }
    /// What the user pastes in the add-on: `built:<port>:<token>`.
    #[getter]
    fn pairing(&self) -> String {
        format!("built:{}:{}", self.port(), self.token)
    }
    #[getter]
    fn linked(&self) -> bool {
        self.inner.as_ref().is_some_and(|s| s.linked())
    }
    #[getter]
    fn queued(&self) -> usize {
        self.inner.as_ref().map_or(0, |s| s.queued())
    }
    fn set_document(&self, id: &str, name: &str) {
        if let Some(s) = &self.inner {
            s.set_document(id, name);
        }
    }
    /// Drain what happened: messages from the add-on (their header plus
    /// `_payload` bytes) and `_linked` / `_refused` / `_closed` events.
    fn poll<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let out = PyList::empty(py);
        let Some(s) = &self.inner else { return Ok(out) };
        for e in s.poll() {
            match e {
                server::Event::Message(m) => out.append(message_dict(py, &m)?)?,
                server::Event::Linked(h) => {
                    let d = to_py(py, &Value::Object(h))?;
                    d.del_item("token")?;
                    d.set_item("type", "_linked")?;
                    out.append(d)?;
                }
                server::Event::Refused(reason, h) => {
                    let d = PyDict::new(py);
                    d.set_item("type", "_refused")?;
                    d.set_item("reason", reason)?;
                    d.set_item("file", h.get("file").and_then(Value::as_str).unwrap_or(""))?;
                    out.append(d)?;
                }
                server::Event::Closed(reason) => {
                    let d = PyDict::new(py);
                    d.set_item("type", "_closed")?;
                    d.set_item("reason", reason)?;
                    out.append(d)?;
                }
            }
        }
        Ok(out)
    }
    /// Queue a message; false when no add-on is linked.
    #[pyo3(signature=(header, payload=None))]
    fn send(&self, header: &Obj<'_>, payload: Option<&[u8]>) -> PyResult<bool> {
        let Some(s) = &self.inner else { return Ok(false) };
        Ok(s.send(&wire::Message::new(to_map(header)?, payload.map(<[u8]>::to_vec).unwrap_or_default())))
    }
    #[pyo3(signature=(reason="unlinked"))]
    fn disconnect(&self, reason: &str) {
        if let Some(s) = &self.inner {
            s.disconnect(reason);
        }
    }
    fn stop(&mut self) {
        if let Some(mut s) = self.inner.take() {
            s.stop();
        }
    }
}

/// The drawer's list of changes from Blender (see `review`).
#[pyclass(module = "built_core.render_blender")]
#[derive(Default)]
pub struct LinkReview {
    inner: review::Review,
}

#[pymethods]
impl LinkReview {
    #[new]
    fn new() -> Self {
        Self::default()
    }
    fn __len__(&self) -> usize {
        self.inner.len()
    }
    #[getter]
    fn conflicts(&self) -> usize {
        self.inner.conflicts()
    }
    /// Take a batch (`items` from the add-on, `current`: Built's values by
    /// key). Returns `{"added": n, "same": [keys Built already matches]}`.
    fn receive<'py>(&mut self, items: &Obj<'py>, current: &Obj<'py>) -> PyResult<Obj<'py>> {
        let Value::Array(items) = to_json(items)? else {
            return Err(PyValueError::new_err("expected a list of changes"));
        };
        let got = self.inner.receive(&items, &to_map(current)?);
        to_py(current.py(), &serde_json::json!({"added": got.added, "same": got.same}))
    }
    /// Compare again with Built's values; returns keys that dropped out.
    fn revalidate(&mut self, current: &Obj<'_>) -> PyResult<Vec<String>> {
        Ok(self.inner.revalidate(&to_map(current)?))
    }
    fn keys(&self) -> Vec<String> {
        self.inner.items().iter().map(|i| i.key.clone()).collect()
    }
    fn rows<'py>(&self, py: Python<'py>) -> PyResult<Obj<'py>> {
        to_py(py, &Value::Array(self.inner.rows()))
    }
    fn items<'py>(&self, py: Python<'py>) -> PyResult<Obj<'py>> {
        to_py(py, &Value::Array(self.inner.items().iter().map(review::Item::to_json).collect()))
    }
    /// Remove and return items (all when `keys` is None).
    #[pyo3(signature=(keys=None))]
    fn take<'py>(&mut self, py: Python<'py>, keys: Option<Vec<String>>) -> PyResult<Obj<'py>> {
        let taken = self.inner.take(keys.as_deref());
        to_py(py, &Value::Array(taken.iter().map(review::Item::to_json).collect()))
    }
    fn clear(&mut self) {
        self.inner.clear();
    }
}

/// Frames of one message, as the add-on would read them (tests, tools).
#[pyfunction]
#[pyo3(signature=(header, payload=None, msg=1, chunk=wire::CHUNK))]
fn link_encode<'py>(py: Python<'py>, header: &Obj<'py>, payload: Option<&[u8]>, msg: u64, chunk: usize) -> PyResult<Bound<'py, PyBytes>> {
    let m = wire::Message::new(to_map(header)?, payload.map(<[u8]>::to_vec).unwrap_or_default());
    Ok(PyBytes::new(py, &wire::encode(&m, msg, chunk).concat()))
}

/// Whole messages from a byte stream (tests, tools).
#[pyfunction]
fn link_decode<'py>(py: Python<'py>, data: &[u8]) -> PyResult<Bound<'py, PyList>> {
    let out = PyList::empty(py);
    for m in wire::Assembler::new().feed(data).map_err(|e| PyValueError::new_err(e.to_string()))? {
        out.append(message_dict(py, &m)?)?;
    }
    Ok(out)
}

/// The add-on's folder in this installation.
#[pyfunction]
fn addon_source(py: Python<'_>) -> PyResult<PathBuf> {
    let root = py.import("built_core.paths")?.call_method0("app_root")?.extract::<PathBuf>()?;
    Ok(root.join("integrations/blender/built_link"))
}

fn dos_time() -> (u16, u16) {
    // 2026-01-01 00:00: a fixed stamp keeps the archive reproducible.
    (0, ((2026 - 1980) << 9 | 1 << 5 | 1) as u16)
}

/// Write `source` (the add-on folder) into a zip Blender installs from:
/// `built_link/<file>` entries, stored, `.py` and `.md` files only.
pub fn write_zip(source: &Path, dest: &Path) -> std::io::Result<usize> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(source)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "py" || e == "md"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "the add-on folder is empty"));
    }
    let folder = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "built_link".into());
    let (time, date) = dos_time();
    let mut out = Vec::new();
    let mut central = Vec::new();
    for path in &files {
        let data = std::fs::read(path)?;
        let name = format!("{folder}/{}", path.file_name().unwrap().to_string_lossy());
        let crc = crc32fast::hash(&data);
        let offset = out.len() as u32;
        let mut local = Vec::new();
        local.extend_from_slice(&0x04034b50u32.to_le_bytes());
        local.extend_from_slice(&20u16.to_le_bytes()); // version needed
        local.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
        local.extend_from_slice(&0u16.to_le_bytes()); // stored
        local.extend_from_slice(&time.to_le_bytes());
        local.extend_from_slice(&date.to_le_bytes());
        local.extend_from_slice(&crc.to_le_bytes());
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(name.len() as u16).to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&local);
        out.extend_from_slice(&data);
        central.extend_from_slice(&0x02014b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // made by
        central.extend_from_slice(&local[4..30]);
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&(0o100644u32 << 16).to_le_bytes()); // external attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let start = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x06054b50u32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = dest.with_extension("zip.part");
    std::fs::File::create(&tmp)?.write_all(&out)?;
    std::fs::rename(&tmp, dest)?;
    Ok(files.len())
}

/// Package the add-on as an installable zip at `dest`; returns the path.
#[pyfunction]
#[pyo3(signature=(dest, source=None))]
fn write_addon_zip(py: Python<'_>, dest: PathBuf, source: Option<PathBuf>) -> PyResult<PathBuf> {
    let source = match source {
        Some(s) => s,
        None => addon_source(py)?,
    };
    write_zip(&source, &dest).map_err(|e| PyOSError::new_err(format!("{}: {e}", source.display())))?;
    Ok(dest)
}

pub(crate) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    m.add("LINK_PROTOCOL", wire::PROTOCOL)?;
    m.add("LINK_CHUNK", wire::CHUNK)?;
    m.add("LINK_PORT", 47815)?;
    m.add_class::<LinkServer>()?;
    m.add_class::<LinkReview>()?;
    m.add_class::<scene::LinkTracker>()?;
    for f in [
        wrap_pyfunction!(link_encode, m)?,
        wrap_pyfunction!(link_decode, m)?,
        wrap_pyfunction!(addon_source, m)?,
        wrap_pyfunction!(write_addon_zip, m)?,
        wrap_pyfunction!(scene::export_chunks, m)?,
        wrap_pyfunction!(scene::link_values, m)?,
        wrap_pyfunction!(scene::link_apply, m)?,
        wrap_pyfunction!(scene::link_new_id, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zip_holds_the_package_folder_and_reads_back() {
        let dir = std::env::temp_dir().join(format!("built-addon-{}", std::process::id()));
        let src = dir.join("built_link");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("__init__.py"), "bl_info = {}\n").unwrap();
        std::fs::write(src.join("wire.py"), "X = 1\n").unwrap();
        std::fs::write(src.join("notes.tmp"), "skip").unwrap();
        let dest = dir.join("out/built_link.zip");
        assert_eq!(write_zip(&src, &dest).unwrap(), 2);
        let bytes = std::fs::read(&dest).unwrap();
        assert_eq!(&bytes[..4], &0x04034b50u32.to_le_bytes());
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("built_link/__init__.py") && text.contains("built_link/wire.py"));
        assert!(!text.contains("notes.tmp"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
