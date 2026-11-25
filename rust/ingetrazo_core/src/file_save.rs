// SPDX-License-Identifier: GPL-3.0-or-later
//! Prepare and publish explicit saves and detached autosave snapshots.
use pyo3::{
    exceptions::{PyStopIteration, PyTypeError, PyUnicodeEncodeError, PyValueError},
    prelude::*,
    types::{PyDict, PyModule},
};
type Obj<'p> = Bound<'p, PyAny>;
fn api(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("formats.igz")
}
fn scratch<'p, T>(stream: Obj<'p>, work: impl FnOnce(&Obj<'p>) -> PyResult<T>) -> PyResult<T> {
    let py = stream.py();
    let file = stream.call_method0("__enter__")?;
    let out = work(&file);
    match &out {
        Ok(_) => {
            stream.call_method1("__exit__", (py.None(), py.None(), py.None()))?;
        }
        Err(e) => {
            stream.call_method1("__exit__", (e.get_type(py), e.value(py), e.traceback(py)))?;
        }
    }
    out
}
fn tick(progress: Option<&Obj<'_>>, f: f64, stage: &str) -> PyResult<()> {
    if let Some(cb) = progress {
        cb.call1((f, stage))?;
    }
    Ok(())
}
#[pyclass]
struct Pulse {
    progress: Py<PyAny>,
    meter: Option<Py<PyAny>>,
}
#[pymethods]
impl Pulse {
    fn __call__(&self, py: Python<'_>) -> PyResult<()> {
        let fraction = if let Some(m) = &self.meter {
            m.bind(py).call_method0("fraction")?.extract::<f64>()?
        } else {
            0.
        };
        tick(
            Some(self.progress.bind(py)),
            0.76 + 0.23 * fraction,
            "Writing the document…",
        )
    }
}
#[pyfunction]
#[pyo3(signature=(scene,path,*,backup,confirm_loss=None,compatibility=false,progress=None))]
fn igz_save_scene_inner<'p>(
    py: Python<'p>,
    scene: &Obj<'p>,
    path: &Obj<'p>,
    backup: bool,
    confirm_loss: Option<&Obj<'p>>,
    compatibility: bool,
    progress: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let api = api(py)?;
    if scene.hasattr("plugin_data")? && !compatibility {
        let project = py.import("ingetrazo_core.project")?;
        project.call_method1("refresh_reference", (scene, path))?;
        project.call_method1("sync_files", (scene, path))?;
        crate::model::link_io::refresh_paths(scene, path)?;
    }
    let native = api
        .getattr("built")?
        .call_method1("is_built", (path,))?
        .is_truthy()?
        && !compatibility;
    let prepare = |stream: Option<&Obj<'p>>| {
        igz_prepare_and_write(py, scene, path, backup, stream, confirm_loss, progress)
    };
    if native || compatibility || api.call_method1("_large_document", (scene,))?.is_truthy()? {
        let kwargs = PyDict::new(py);
        kwargs.set_item("built", native)?;
        kwargs.set_item("compatibility", compatibility)?;
        kwargs.set_item("destination", path)?;
        scratch(api.call_method("SaveStream", (), Some(&kwargs))?, |s| {
            prepare(Some(s))
        })
    } else {
        prepare(None)
    }
}
#[pyfunction]
#[pyo3(signature=(scene,path,backup,stream=None,confirm_loss=None,progress=None))]
fn igz_prepare_and_write<'p>(
    py: Python<'p>,
    scene: &Obj<'p>,
    path: &Obj<'p>,
    backup: bool,
    stream: Option<&Obj<'p>>,
    confirm_loss: Option<&Obj<'p>>,
    progress: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let api = api(py)?;
    tick(progress, 0.02, "Preparing the document…")?;
    let kw = PyDict::new(py);
    kw.set_item("stream", stream)?;
    kw.set_item("large", stream.is_some())?;
    kw.set_item("progress", progress)?;
    let iter = api.call_method("prepare_scene_steps", (scene,), Some(&kw))?;
    let prepared = loop {
        match iter.call_method0("__next__") {
            Ok(_) => (),
            Err(e) if e.is_instance_of::<PyStopIteration>(py) => {
                break e.value(py).getattr("value")?
            }
            Err(e) => return Err(e),
        }
    };
    if let (Some(cb), Some(stream)) = (confirm_loss, stream) {
        let mm = stream.getattr("displacement_mm")?;
        let limit = api.getattr("built")?.getattr("WARNING_MM")?;
        if mm.gt(limit)? && !cb.call1((mm,))?.is_truthy()? {
            return Ok(py.None().into_bound(py));
        }
    }
    let failed = backup && !api.call_method1("_backup_previous", (path,))?.is_truthy()?;
    tick(progress, 0.76, "Writing the document…")?;
    let io = api.call_method0("native_io")?;
    let meter = if progress.is_some() {
        io.getattr("ArchiveProgress")
            .ok()
            .map(|c| c.call0())
            .transpose()?
    } else {
        None
    };
    let pulse = progress
        .map(|p| {
            Py::new(
                py,
                Pulse {
                    progress: p.clone().unbind(),
                    meter: meter.clone().map(Bound::unbind),
                },
            )
        })
        .transpose()?;
    let kw = PyDict::new(py);
    kw.set_item("stream", stream)?;
    kw.set_item("meter", meter)?;
    let work = py.import("functools")?.call_method(
        "partial",
        (api.getattr("write_prepared")?, prepared, path),
        Some(&kw),
    )?;
    let options = PyDict::new(py);
    options.set_item("background", !io.is_none())?;
    let stats = api.call_method("_in_worker", (work, pulse), Some(&options))?;
    tick(progress, 1., "Saved")?;
    if failed {
        stats.set_item("backup_failed", true)?;
    }
    Ok(stats)
}
fn nonfinite<'p, T>(api: &Bound<'p, PyModule>, obj: &Obj<'p>, value: PyResult<T>) -> PyResult<T> {
    match value {
        Err(e) if e.is_instance_of::<PyValueError>(obj.py()) => {
            let loc = api.call_method1("_non_finite_where", (obj,))?;
            let error = PyErr::from_value(api.call_method1("NonFiniteDocumentError", (loc,))?);
            Err(error)
        }
        result => result,
    }
}
#[pyfunction]
pub fn igz_spool_json<'p>(value: &Obj<'p>, stream: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = value.py();
    let api = api(py)?;
    let kw = PyDict::new(py);
    kw.set_item("blobs", stream.getattr("blobs")?)?;
    kw.set_item("resolved", stream.getattr("resolved")?)?;
    let packed = api.call_method("_pack_textures", (value,), Some(&kw))?;
    stream.setattr(
        "missing",
        stream.getattr("missing")?.add(packed.get_item(1)?)?,
    )?;
    nonfinite(&api, value, stream.call_method1("json", (value,)))
}
fn encode<'p>(
    api: &Bound<'p, PyModule>,
    data: &Bound<'p, PyDict>,
    large: bool,
) -> PyResult<Obj<'p>> {
    let py = data.py();
    let io = api.call_method0("native_io")?;
    if !io.is_none() {
        let out = py.import("io")?.call_method0("BytesIO")?;
        let res = io.call_method1(
            "igz_encode_json",
            (
                data,
                out.getattr("write")?,
                api.getattr("Fragment")?,
                !large,
            ),
        );
        match res {
            Ok(_) => return out.call_method0("getvalue"),
            Err(e)
                if e.is_instance_of::<PyTypeError>(py)
                    || e.is_instance_of::<PyUnicodeEncodeError>(py) =>
            {
                ()
            }
            Err(e) => return Err(e),
        }
    }
    let opts = PyDict::new(py);
    opts.set_item("allow_nan", false)?;
    if large {
        opts.set_item("separators", (",", ":"))?;
    } else {
        opts.set_item("indent", 2)?;
    }
    py.import("json")?
        .call_method("dumps", (data,), Some(&opts))
}
#[pyfunction]
#[pyo3(signature=(prepared,path,background=false,*,stream=None,meter=None))]
fn igz_write_prepared<'p>(
    py: Python<'p>,
    prepared: &Obj<'p>,
    path: &Obj<'p>,
    background: bool,
    stream: Option<&Obj<'p>>,
    meter: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let api = api(py)?;
    if background {
        let kw = PyDict::new(py);
        kw.set_item(
            "built",
            api.getattr("built")?.call_method1("is_built", (path,))?,
        )?;
        scratch(api.call_method("SaveStream", (), Some(&kw))?, |s| {
            igz_write_prepared_inner(py, prepared, path, true, Some(s), meter)
        })
    } else {
        igz_write_prepared_inner(py, prepared, path, false, stream, meter)
    }
}
#[pyfunction]
#[pyo3(signature=(prepared,path,background,stream=None,meter=None))]
fn igz_write_prepared_inner<'p>(
    py: Python<'p>,
    prepared: &Obj<'p>,
    path: &Obj<'p>,
    background: bool,
    stream: Option<&Obj<'p>>,
    meter: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let api = api(py)?;
    let [payload, mut geom, large, mut survey]: [Obj<'p>; 4] = prepared.extract()?;
    let large = large.is_truthy()?;
    let kw = PyDict::new(py);
    if let Some(stream) = stream {
        kw.set_item("blobs", stream.getattr("blobs")?)?;
        kw.set_item("resolved", stream.getattr("resolved")?)?;
        if background {
            geom = stream.clone();
            let snapshot = py.import("ingetrazo_core")?;
            let opt = PyDict::new(py);
            opt.set_item("stream", stream)?;
            let spool = py.import("functools")?.call_method(
                "partial",
                (api.getattr("_spool_json")?,),
                Some(&opt),
            )?;
            snapshot.call_method1("resolve_geometry", (&payload, &geom, spool))?;
            if survey.is_instance(&snapshot.getattr("Survey")?)? {
                let result = py
                    .import("georef.photomesh")?
                    .call_method1("pack_mesh", (survey.getattr("mesh")?,))?;
                payload.set_item("photo_mesh", result.get_item(0)?)?;
                let blobs = result.get_item(1)?;
                if let Some(png) = survey.getattr("thumbnail").ok().filter(|v| !v.is_none()) {
                    blobs.set_item("thumbnail.png", png)?;
                }
                survey = blobs;
            }
        }
    }
    let (blobs, mut missing): (Obj<'p>, usize) = api
        .call_method("_pack_textures", (&payload,), Some(&kw))?
        .extract()?;
    if let Some(s) = stream {
        missing += s.getattr("missing")?.extract::<usize>()?;
    }
    blobs.call_method1("update", (survey,))?;
    let data = PyDict::new(py);
    let version = if geom.is_truthy()? {
        3
    } else if blobs.is_truthy()? {
        2
    } else {
        1
    };
    data.set_item("igz_format", version)?;
    data.set_item("app_version", api.getattr("__version__")?)?;
    data.set_item("scene", &payload)?;
    let mut container = version != 1;
    if let Some(s) = stream {
        if s.hasattr("built")? && s.getattr("built")?.is_truthy()? {
            data.del_item("igz_format")?;
            data.set_item("built_format", 1)?;
            container = true;
        } else if s.hasattr("compatibility")? && s.getattr("compatibility")?.is_truthy()? {
            data.set_item("igz_format", 3)?;
            container = true;
        }
    }
    let doc = nonfinite(
        &api,
        &payload,
        if let Some(s) = stream {
            s.call_method1("document", (&data,))
        } else {
            encode(&api, &data, large)
        },
    )?;
    if container {
        let kw = PyDict::new(py);
        kw.set_item("fast", large)?;
        kw.set_item("deflated", geom)?;
        if let Some(m) = meter {
            kw.set_item("meter", m)?;
        }
        api.call_method("_write_container", (path, doc, &blobs), Some(&kw))?;
    } else {
        let bytes = if doc.is_instance_of::<pyo3::types::PyString>() {
            doc.call_method1("encode", ("utf-8",))?
        } else {
            doc
        };
        api.call_method1("_write_atomic", (path, bytes))?;
    }
    let stats = PyDict::new(py);
    stats.set_item("embedded", blobs.len()?)?;
    stats.set_item("missing", missing)?;
    Ok(stats.into_any())
}
#[path = "file_publish.rs"]
mod publish;
#[path = "file_stream.rs"]
mod stream;
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    publish::register(m)?;
    stream::register(m)?;
    m.add_function(wrap_pyfunction!(igz_save_scene_inner, m)?)?;
    m.add_function(wrap_pyfunction!(igz_prepare_and_write, m)?)?;
    m.add_function(wrap_pyfunction!(igz_spool_json, m)?)?;
    m.add_function(wrap_pyfunction!(igz_write_prepared, m)?)?;
    m.add_function(wrap_pyfunction!(igz_write_prepared_inner, m)?)
}
