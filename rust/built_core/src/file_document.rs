// SPDX-License-Identifier: GPL-3.0-or-later
//! Load a document from the archive into the staged scene. Keep the ZIP
//! descriptor alive until all images and geometry members have been read.
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyInt, PyList, PyTuple},
};
use std::time::{Duration, Instant};
type Obj<'p> = Bound<'p, PyAny>;
fn api(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("built_core.formats.igz")
}
fn seq<'p>(payload: &Obj<'p>, k: &str) -> PyResult<Obj<'p>> {
    let rows = payload.call_method1("get", (k,))?;
    Ok(if rows.is_truthy()? {
        rows
    } else {
        PyList::empty(payload.py()).into_any()
    })
}
fn tick(cb: Option<&Obj<'_>>, f: f64, s: &str) -> PyResult<()> {
    if let Some(cb) = cb {
        cb.call1((f, s))?;
    }
    Ok(())
}
#[pyclass]
struct Meter {
    cb: Py<PyAny>,
    base: f64,
    span: f64,
    stage: &'static str,
    reader: Option<Py<PyAny>>,
}
#[pymethods]
impl Meter {
    #[pyo3(signature=(fraction=None))]
    fn __call__(&self, py: Python<'_>, fraction: Option<f64>) -> PyResult<()> {
        let f = if let Some(r) = &self.reader {
            r.bind(py).call_method0("fraction")?.extract()?
        } else {
            fraction.unwrap_or(0.)
        };
        tick(
            Some(self.cb.bind(py)),
            self.base + self.span * f,
            self.stage,
        )
    }
}
fn meter<'p>(
    py: Python<'p>,
    cb: Option<&Obj<'p>>,
    base: f64,
    span: f64,
    stage: &'static str,
    reader: Option<&Obj<'p>>,
) -> PyResult<Option<Py<Meter>>> {
    cb.map(|cb| {
        Py::new(
            py,
            Meter {
                cb: cb.clone().unbind(),
                base,
                span,
                stage,
                reader: reader.map(|r| r.clone().unbind()),
            },
        )
    })
    .transpose()
}
#[pyclass]
struct ReadMesh {
    geom: Py<PyAny>,
    links: Option<Py<PyAny>>,
}
#[pymethods]
impl ReadMesh {
    fn __call__<'p>(
        &self,
        mesh: &Obj<'p>,
        block: &Obj<'p>,
        identity: &Obj<'p>,
    ) -> PyResult<Obj<'p>> {
        let py = mesh.py();
        let geom = self.geom.bind(py);
        if let Some(links) = &self.links {
            return links
                .bind(py)
                .call_method1("read", (mesh, block, geom, identity));
        }
        api(py)?.call_method1("_load_mesh", (mesh, block, geom))?;
        Ok(mesh.clone())
    }
}
fn materials<'p>(payload: &Obj<'p>, links: Option<&Obj<'p>>) -> PyResult<Bound<'p, PyDict>> {
    let py = payload.py();
    let m = py.import("built_core.materials")?.getattr("Material")?;
    let decode = api(py)?
        .call_method0("native_io")?
        .getattr("igz_load_material")
        .ok();
    let out = PyDict::new(py);
    for raw in seq(payload, "materials")?.try_iter()? {
        let raw = raw?;
        let item = if let Some(d) = &decode {
            d.call1((raw, &m))?
        } else {
            m.call_method1("from_dict", (raw,))?
        };
        let name = item.getattr("name")?;
        if name.is_truthy()? {
            out.set_item(name, item)?
        }
    }
    if let Some(l) = links {
        l.call_method1("prepare", (&out, payload))?;
    }
    Ok(out)
}
#[pyfunction]
#[pyo3(signature=(scene,path,progress=None,link_meshes=None))]
fn igz_load_into_inner<'p>(
    scene: &Obj<'p>,
    path: &Obj<'p>,
    progress: Option<&Obj<'p>>,
    link_meshes: Option<&Obj<'p>>,
) -> PyResult<()> {
    let py = scene.py();
    let codec = api(py)?;
    let repairs = codec.getattr("_repair_count")?.call_method0("get")?;
    repairs.set_item(0, 0)?;
    codec
        .getattr("_finite_flag")?
        .call_method0("get")?
        .set_item(0, false)?;
    let reading = "Reading the document…";
    let geometry = "Reading geometry…";
    let placing = "Placing groups…";
    let finishing = "Finishing…";
    tick(progress, 0.05, reading)?;
    let kw = PyDict::new(py);
    kw.set_item("pump", meter(py, progress, 0.1, 0., reading, None)?)?;
    kw.set_item(
        "read_progress",
        meter(py, progress, 0.05, 0.14, reading, None)?,
    )?;
    let root = scene.getattr("_root").ok();
    kw.set_item(
        "placements",
        root.is_some_and(|r| r.is_truthy().unwrap_or(false))
            && py
                .import("core._native")?
                .call_method0("wanted")?
                .is_truthy()?,
    )?;
    let (data, archive): (Obj<'p>, Obj<'p>) = codec
        .call_method("_read_document", (path,), Some(&kw))?
        .extract()?;
    let built = data.contains("built_format")?;
    let payload = data.call_method1("get", ("scene", PyDict::new(py)))?;
    let partial = py.import("functools")?.getattr("partial")?;
    let read = (|| -> PyResult<_> {
        if built
            || py
                .import("built_core.formats.built")?
                .call_method1("is_built", (path,))?
                .is_truthy()?
        {
            let version = data.call_method1("get", ("built_format",))?;
            if !version.is_exact_instance_of::<PyInt>()
                || version.ne(1)?
                || archive.is_none()
                || data.contains("igz_format")?
            {
                return Err(PyValueError::new_err(
                    "Invalid or unsupported Built document format",
                ));
            }
        }
        if archive.is_none() {
            return Ok((
                materials(&payload, link_meshes)?,
                PyDict::new(py).into_any(),
                py.None().into_bound(py),
            ));
        }
        if archive
            .call_method0("namelist")?
            .contains("thumbnail.png")?
        {
            let info = archive.call_method1("getinfo", ("thumbnail.png",))?;
            if info.getattr("file_size")?.le(1024 * 1024)? {
                scene.setattr(
                    "_home_thumbnail_png",
                    archive.call_method1("read", (info,))?,
                )?
            }
        }
        tick(progress, 0.2, "Unpacking textures…")?;
        codec.call_method1("_unpack_textures", (&payload, &archive))?;
        let mat = materials(&payload, link_meshes)?;
        let skip = if let Some(links) = link_meshes {
            links.call_method1("unchanged_members", (&archive, &payload))?
        } else {
            PyTuple::empty(py).into_any()
        };
        tick(progress, 0.25, geometry)?;
        let opt = PyDict::new(py);
        opt.set_item("built_format", built)?;
        let work = partial.call(
            (
                codec.getattr("_read_geometry_members")?,
                &archive,
                path,
                &payload,
                skip,
            ),
            Some(&opt),
        )?;
        let opt = PyDict::new(py);
        opt.set_item(
            "background",
            archive.is_instance(&codec.getattr("DocumentArchive")?)?,
        )?;
        let geom = codec.call_method(
            "_in_worker",
            (work, meter(py, progress, 0.25, 0., geometry, None)?),
            Some(&opt),
        )?;
        let survey = codec.call_method1("_unpack_photo_mesh", (&payload, &archive))?;
        Ok((mat, geom, survey))
    })();
    if !archive.is_none() {
        archive.call_method0("close")?;
    }
    let (mat, geom, survey) = read?;
    let version = data.call_method1("get", ("igz_format", 1))?;
    let max = codec.getattr("CURRENT_FORMAT")?;
    if version.gt(&max)? {
        return Err(PyValueError::new_err(format!(
            "Document format v{version} is newer than this build (v{max})."
        )));
    }
    codec.call_method1("_normalize_text_curve_ids", (&payload, &geom))?;
    tick(progress, 0.35, "Building geometry…")?;
    codec.call_method1("igz_reset_scene", (scene, &payload, survey))?;
    scene.setattr("materials", mat)?;
    let read = Py::new(
        py,
        ReadMesh {
            geom: geom.unbind(),
            links: link_meshes.map(|v| v.clone().unbind()),
        },
    )?;
    scene.setattr(
        "mesh",
        read.call1(py, (scene.getattr("mesh")?, &payload, ("loose",)))?,
    )?;
    codec.call_method1("igz_restore_views", (scene, &payload))?;
    let protos = PyList::empty(py);
    let raw = seq(&payload, "protos")?;
    let n = raw.len()?.max(1);
    let mut last = Instant::now();
    let mesh = codec.getattr("Mesh")?;
    for (k, p) in raw.try_iter()?.enumerate() {
        protos.append(read.call1(py, (mesh.call0()?, p?, ("proto", k)))?)?;
        if last.elapsed() > Duration::from_millis(250) {
            tick(
                progress,
                0.35 + 0.25 * k as f64 / n as f64,
                "Building geometry…",
            )?;
            last = Instant::now()
        }
    }
    let group_api = py.import("core.group")?;
    let groups = py.import("built_core")?.getattr("FileGroups")?;
    let names = PyList::empty(py);
    for r in groups.call_method1("records", (&payload,))?.try_iter()? {
        let name = r?.call_method1("get", ("name",))?;
        if name.is_truthy()? {
            names.append(name)?
        }
    }
    let raw = seq(&payload, "groups")?;
    let kept = seq(&payload, "kept_components")?;
    let n = raw.len()? + kept.len()?;
    let scope = group_api.call_method1("document_name_scope", (&names,))?;
    scope.call_method0("__enter__")?;
    let assemble = (|| -> PyResult<()> {
        let reader = groups.call1((n,))?;
        if n > 0 {
            tick(progress, 0.6, placing)?
        }
        let work = partial.call1((reader.getattr("build")?, raw, kept, protos, read, built))?;
        let opt = PyDict::new(py);
        opt.set_item("background", true)?;
        opt.set_item("cancel", reader.getattr("cancel")?)?;
        let result = codec.call_method(
            "_in_worker",
            (
                work,
                meter(py, progress, 0.6, 0.35, placing, Some(&reader))?,
            ),
            Some(&opt),
        )?;
        scene.setattr("groups", result.get_item(0)?)?;
        scene.setattr("kept_components", result.get_item(1)?)?;
        Ok(())
    })();
    match &assemble {
        Ok(()) => scope.call_method1("__exit__", (py.None(), py.None(), py.None()))?,
        Err(e) => scope.call_method1("__exit__", (e.get_type(py), e.value(py), e.traceback(py)))?,
    };
    assemble?;
    tick(progress, 0.96, finishing)?;
    if !names.is_empty() {
        group_api.call_method1("reserve_group_names", (names,))?;
    }
    let new = codec.call_method1("igz_finish_scene", (scene, &payload))?;
    let total = repairs.get_item(0)?.add(new)?;
    repairs.set_item(0, &total)?;
    scene.setattr("load_repairs", total)?;
    if progress.is_some() {
        codec.call_method1(
            "_release_in_steps",
            (data, meter(py, progress, 0.98, 0., finishing, None)?),
        )?;
    }
    scene.setattr("version", scene.getattr("version")?.add(1)?)?;
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(igz_load_into_inner, m)?)
}
