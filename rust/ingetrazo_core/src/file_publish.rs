// SPDX-License-Identifier: GPL-3.0-or-later
//! Publish an explicit document save and preserve the last complete version.
use crate::atomic_document::AtomicDocument;
use pyo3::{
    exceptions::PyOSError,
    prelude::*,
    types::{PyDict, PyFloat, PyList, PyString, PyTuple},
};
use std::{collections::HashSet, path::PathBuf};
type Obj<'p> = Bound<'p, PyAny>;
fn codec(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("formats.igz")
}

#[pyclass(name = "DocumentOutput")]
struct Output {
    path: PathBuf,
    stage: Option<AtomicDocument>,
    target: Option<Py<PyAny>>,
}
#[pymethods]
impl Output {
    #[new]
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            stage: None,
            target: None,
        }
    }
    fn __enter__<'p>(&mut self, py: Python<'p>) -> PyResult<Obj<'p>> {
        let stage = py.detach(|| AtomicDocument::create(self.path.clone()))?;
        let target = py
            .import("builtins")?
            .call_method1("open", (stage.pending_path()?, "wb"))?;
        self.stage = Some(stage);
        self.target = Some(target.clone().unbind());
        Ok(target)
    }
    fn __exit__(
        &mut self,
        py: Python<'_>,
        ty: &Obj<'_>,
        _value: &Obj<'_>,
        _trace: &Obj<'_>,
    ) -> PyResult<()> {
        let (Some(mut stage), Some(target)) = (self.stage.take(), self.target.take()) else {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "document output is closed",
            ));
        };
        target.bind(py).call_method0("close")?;
        if ty.is_none() {
            py.detach(|| stage.commit())?;
        }
        Ok(())
    }
}
fn writing<T>(
    py: Python<'_>,
    path: PathBuf,
    write: impl FnOnce(&Obj<'_>) -> PyResult<T>,
) -> PyResult<T> {
    let mut output = Output::new(path);
    let file = output.__enter__(py)?;
    let result = write(&file);
    let none = py.None().into_bound(py);
    match &result {
        Err(e) => output.__exit__(py, e.get_type(py).as_any(), e.value(py).as_any(), &none)?,
        Ok(_) => output.__exit__(py, &none, &none, &none)?,
    }
    result
}
fn slice<'p>(data: &Obj<'p>, fragment: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    if !data.is_instance(fragment)? {
        return Ok(None);
    }
    let file = data.getattr("file")?;
    file.call_method0("flush")?;
    Ok(Some(
        (
            file.call_method0("fileno")?,
            data.getattr("offset")?,
            data.getattr("size")?,
        )
            .into_pyobject(data.py())?
            .into_any(),
    ))
}
fn contents<'p>(data: &Obj<'p>) -> PyResult<Obj<'p>> {
    if data.is_instance_of::<PyString>() {
        data.call_method1("encode", ("utf-8",))
    } else {
        Ok(data.clone())
    }
}
#[pyfunction]
fn igz_write_atomic(py: Python<'_>, path: PathBuf, data: &Obj<'_>) -> PyResult<()> {
    let api = codec(py)?;
    let native = api.call_method0("native_io")?;
    let fragment = api.getattr("Fragment")?;
    writing(py, path, |file| {
        if let Some(writer) = native.getattr_opt("igz_write_plain")? {
            let range = slice(data, &fragment)?;
            writer.call1((
                file.call_method0("fileno")?,
                range.is_none().then_some(data),
                range,
            ))?;
        } else if data.is_instance(&fragment)? {
            for chunk in data.call_method0("chunks")?.try_iter()? {
                file.call_method1("write", (chunk?,))?;
            }
        } else {
            file.call_method1("write", (data,))?;
        }
        Ok(())
    })
}
#[pyfunction(signature=(path,doc,blobs,fast=false,deflated=None,*,meter=None))]
fn igz_write_container<'p>(
    py: Python<'p>,
    path: PathBuf,
    doc: &Obj<'p>,
    blobs: &Obj<'p>,
    fast: bool,
    deflated: Option<&Obj<'p>>,
    meter: Option<&Obj<'p>>,
) -> PyResult<()> {
    let api = codec(py)?;
    let fragment = api.getattr("Fragment")?;
    let mut entries = vec![("document.json".to_owned(), true, doc.clone())];
    for (dict, compressed) in [(deflated, true), (Some(blobs), false)] {
        if let Some(dict) = dict {
            let mut keys = dict
                .try_iter()?
                .map(|k| k?.extract::<String>())
                .collect::<PyResult<Vec<_>>>()?;
            keys.sort();
            for key in keys {
                let value = dict.get_item(&key)?;
                entries.push((key, compressed, value));
            }
        }
    }
    writing(py, path, |file| {
        let native = api.call_method0("native_io")?;
        if let Some(writer) = native.getattr_opt("igz_write_zip")? {
            let members = PyList::empty(py);
            for (key, compressed, data) in &entries {
                let range = slice(data, &fragment)?;
                let bytes = if range.is_none() {
                    Some(contents(data)?)
                } else {
                    None
                };
                members.append((key, *compressed, bytes, range))?;
            }
            let options = PyDict::new(py);
            if let Some(m) = meter {
                options.set_item("meter", m)?;
            }
            if writer
                .call(
                    (file.call_method0("fileno")?, members, fast),
                    Some(&options),
                )?
                .is_truthy()?
            {
                return Ok(());
            }
        }
        let zip = py.import("zipfile")?;
        let archive = zip.call_method1("ZipFile", (file, "w"))?;
        let write = (|| -> PyResult<()> {
            for (name, compressed, data) in &entries {
                let info = zip.call_method1("ZipInfo", (name, (1980, 1, 1, 0, 0, 0)))?;
                info.setattr("compress_type", if *compressed { 8 } else { 0 })?;
                let level = fast.then_some(1);
                if data.is_instance(&fragment)? {
                    info.setattr("file_size", data.getattr("size")?)?;
                    info.setattr("_compresslevel", level)?;
                    let dest = archive.call_method1("open", (info, "w"))?;
                    let copy = (|| -> PyResult<()> {
                        for chunk in data.call_method0("chunks")?.try_iter()? {
                            dest.call_method1("write", (chunk?,))?;
                        }
                        Ok(())
                    })();
                    dest.call_method0("close")?;
                    copy?;
                } else {
                    let kw = PyDict::new(py);
                    kw.set_item("compresslevel", level)?;
                    archive.call_method("writestr", (info, data), Some(&kw))?;
                }
            }
            Ok(())
        })();
        archive.call_method0("close")?;
        write
    })
}
#[pyfunction]
fn igz_backup_previous(py: Python<'_>, path: PathBuf) -> PyResult<bool> {
    if !path.is_file()
        || py
            .import("core.general_settings")?
            .call_method1("value", ("backup", "1"))?
            .str()?
            .to_str()?
            == "0"
    {
        return Ok(true);
    }
    let mut backup = path.as_os_str().to_owned();
    backup.push(".bak");
    let result = (|| -> PyResult<()> {
        let mut stage = AtomicDocument::create(PathBuf::from(backup))?;
        py.import("shutil")?
            .call_method1("copy2", (path, stage.pending_path()?))?;
        py.detach(|| stage.commit())?;
        Ok(())
    })();
    match result {
        Ok(()) => Ok(true),
        Err(e) if e.is_instance_of::<PyOSError>(py) => Ok(false),
        Err(e) => Err(e),
    }
}
#[pyfunction(signature=(scene,path,*,backup=true,progress=None))]
fn igz_save_scene<'p>(
    py: Python<'p>,
    scene: &Obj<'p>,
    path: &Obj<'p>,
    backup: bool,
    progress: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let gc = py.import("gc")?;
    let enabled = gc.call_method0("isenabled")?.is_truthy()?;
    let kw = PyDict::new(py);
    kw.set_item("backup", backup)?;
    kw.set_item("progress", progress)?;
    let save = codec(py)?.getattr("_save_scene")?;
    gc.call_method0("disable")?;
    let result = save.call((scene, path), Some(&kw));
    if enabled {
        gc.call_method0("enable")?;
    }
    result
}
#[pyfunction(signature=(scene,path,confirm_loss,*,progress=None))]
fn igz_export<'p>(
    py: Python<'p>,
    scene: &Obj<'p>,
    path: &Obj<'p>,
    confirm_loss: &Obj<'p>,
    progress: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let kw = PyDict::new(py);
    kw.set_item("backup", true)?;
    kw.set_item("compatibility", true)?;
    kw.set_item("confirm_loss", confirm_loss)?;
    kw.set_item("progress", progress)?;
    codec(py)?.call_method("_save_scene", (scene, path), Some(&kw))
}
#[pyfunction]
fn igz_live_element_count(scene: &Obj<'_>) -> PyResult<i64> {
    let py = scene.py();
    let count = py.import("core.lazy_mesh")?.getattr("entity_count")?;
    let mut n = count
        .call1((scene.getattr_opt("mesh")?.unwrap_or_else(|| scene.clone()),))?
        .extract::<i64>()?;
    let mut stack = vec![];
    let mut seen = HashSet::new();
    fn append<'p>(stack: &mut Vec<Obj<'p>>, obj: &Obj<'p>, key: &str) -> PyResult<()> {
        if let Some(rows) = obj.getattr_opt(key)?.filter(|r| !r.is_none()) {
            for r in rows.try_iter()? {
                stack.push(r?);
            }
        }
        Ok(())
    }
    append(&mut stack, scene, "groups")?;
    append(&mut stack, scene, "kept_components")?;
    let mut visited = HashSet::new();
    while let Some(node) = stack.pop() {
        if !visited.insert(node.as_ptr()) {
            continue;
        }
        if let Some(m) = node.getattr_opt("mesh")?.filter(|m| !m.is_none()) {
            if seen.insert(m.as_ptr()) {
                n += count.call1((m,))?.extract::<i64>()?;
            }
        }
        append(&mut stack, &node, "children")?;
    }
    Ok(n)
}
#[pyfunction]
fn igz_classify_document(scene: &Obj<'_>) -> PyResult<bool> {
    let api = codec(scene.py())?;
    let limit = api.getattr("_LARGE_DOCUMENT")?.extract::<i64>()?;
    if scene.getattr_opt("_root")?.is_some_and(|r| !r.is_none()) {
        if let Some(classify) = api
            .call_method0("native_io")?
            .getattr_opt("igz_large_document")?
        {
            return classify
                .call1((
                    scene.getattr("mesh")?,
                    scene.getattr("groups")?,
                    scene.getattr("kept_components")?,
                    limit,
                ))?
                .is_truthy();
        }
    }
    Ok(igz_live_element_count(scene)? > limit)
}
#[pyfunction]
fn igz_bulk_sized(py: Python<'_>, n_edges: i64, n_faces: i64) -> PyResult<bool> {
    Ok(i128::from(n_edges) + 4 * i128::from(n_faces)
        >= codec(py)?.getattr("_BULK_MIN")?.extract::<i128>()?)
}
#[pyfunction(signature=(node,path="scene"))]
fn igz_non_finite_where(node: &Obj<'_>, path: &str) -> PyResult<String> {
    let mut stack = vec![(node.clone(), path.to_owned())];
    let mut seen = HashSet::new();
    while let Some((v, at)) = stack.pop() {
        if v.is_instance_of::<PyFloat>() && !v.extract::<f64>()?.is_finite() {
            return Ok(at);
        }
        if let Ok(d) = v.cast::<PyDict>() {
            if seen.insert(v.as_ptr()) {
                let children: Vec<_> = d.iter().collect();
                for (k, value) in children.into_iter().rev() {
                    stack.push((value, format!("{at} › {}", k.str()?)));
                }
            }
        } else if v.is_instance_of::<PyList>() || v.is_instance_of::<PyTuple>() {
            if seen.insert(v.as_ptr()) {
                for i in (0..v.len()?).rev() {
                    stack.push((v.get_item(i)?, format!("{at}[{i}]")));
                }
            }
        }
    }
    Ok(path.into())
}
#[pyfunction]
fn igz_empty_mesh(py: Python<'_>) -> PyResult<Obj<'_>> {
    let api = codec(py)?;
    let cache = api.getattr("_EMPTY")?;
    if cache.len()? == 0 {
        cache.call_method1("append", (py.import("core.mesh")?.call_method0("Mesh")?,))?;
    }
    cache.get_item(0)
}
#[pyfunction]
fn igz_note_constant<'p>(py: Python<'p>, token: &Obj<'p>) -> PyResult<Obj<'p>> {
    codec(py)?
        .getattr("_finite_flag")?
        .call_method0("get")?
        .set_item(0, true)?;
    py.get_type::<PyFloat>().call1((token,))
}
#[pyfunction]
fn igz_mesh_blocks<'p>(py: Python<'p>, payload: &Obj<'p>) -> PyResult<Obj<'p>> {
    let out = PyList::empty(py);
    out.append(payload)?;
    if let Some(protos) = payload.cast::<PyDict>()?.get_item("protos")? {
        out.call_method1("extend", (protos,))?;
    }
    for raw in py
        .import("ingetrazo_core")?
        .getattr("FileGroups")?
        .call_method1("records", (payload,))?
        .try_iter()?
    {
        let raw = raw?;
        if raw.call_method1("get", ("xform",))?.is_none() || !raw.contains("proto")? {
            out.append(raw)?;
        }
    }
    out.into_any().call_method0("__iter__")
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Output>()?;
    macro_rules! add{($($f:ident),*)=>{$(m.add_function(wrap_pyfunction!($f,m)?)?;)*}}
    add!(
        igz_empty_mesh,
        igz_note_constant,
        igz_mesh_blocks,
        igz_write_atomic,
        igz_write_container,
        igz_backup_previous,
        igz_save_scene,
        igz_export,
        igz_bulk_sized,
        igz_classify_document,
        igz_live_element_count,
        igz_non_finite_where
    );
    Ok(())
}
