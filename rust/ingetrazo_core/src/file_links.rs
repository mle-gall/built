// SPDX-License-Identifier: GPL-3.0-or-later
//! Restore linked documents with a fresh repair counter for each nested load.
use pyo3::{
    exceptions::{PyBaseException, PyException, PyKeyError, PyOSError, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict, PyList},
};
use std::time::{Duration, Instant};
type Obj<'p> = Bound<'p, PyAny>;
fn api(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("ingetrazo_core.formats.igz")
}
pyo3::create_exception!(ingetrazo_core, LinkInterrupted, PyBaseException);
#[pyclass]
struct Report {
    cb: Py<PyAny>,
    base: f64,
    span: f64,
    scope: Option<Py<PyAny>>,
}
#[pymethods]
impl Report {
    fn __call__(&self, fraction: f64, phase: &Obj<'_>) -> PyResult<()> {
        let py = phase.py();
        match self.cb.call1(py, (self.base + fraction * self.span, phase)) {
            Err(e) if self.scope.is_some() && e.is_instance_of::<PyException>(py) => {
                let stop = LinkInterrupted::new_err(());
                stop.value(py)
                    .setattr("scope", self.scope.as_ref().unwrap().bind(py))?;
                stop.set_cause(py, Some(e));
                Err(stop)
            }
            other => other.map(|_| ()),
        }
    }
}
fn report<'p>(
    py: Python<'p>,
    cb: Option<&Obj<'p>>,
    base: f64,
    span: f64,
    scope: Option<&Obj<'p>>,
) -> PyResult<Option<Py<Report>>> {
    cb.map(|cb| {
        Py::new(
            py,
            Report {
                cb: cb.clone().unbind(),
                base,
                span,
                scope: scope.map(|s| s.clone().unbind()),
            },
        )
    })
    .transpose()
}
#[pyfunction]
#[pyo3(signature=(scene,path,progress=None,*,include_links=true,link_meshes=None))]
fn igz_load_into<'p>(
    scene: &Obj<'p>,
    path: &Obj<'p>,
    progress: Option<&Obj<'p>>,
    include_links: bool,
    link_meshes: Option<&Obj<'p>>,
) -> PyResult<()> {
    let py = scene.py();
    let codec = api(py)?;
    let gc = py.import("gc")?;
    let native = py.import("core._native")?.getattr("native")?;
    let staging = native
        .getattr("DocumentOpen")
        .and_then(|d| d.getattr("staging"))
        .ok();
    let suspend_gc = gc.call_method0("isenabled")?.is_truthy()?
        && !staging
            .map(|f| f.call0()?.is_truthy())
            .transpose()?
            .unwrap_or(false);
    if suspend_gc {
        gc.call_method0("disable")?;
    }
    let finite = codec.getattr("_finite_flag")?;
    let count = codec.getattr("_repair_count")?;
    let finite_token = finite.call_method1("set", (vec![false],))?;
    let count_token = count.call_method1("set", (vec![0],))?;
    let result = (|| -> PyResult<()> {
        let cb = progress.filter(|c| c.is_truthy().unwrap_or(false));
        codec.call_method1(
            "_load_into_inner",
            (scene, path, report(py, cb, 0., 0.8, None)?, link_meshes),
        )?;
        codec.call_method1(
            "_load_links",
            (scene, path, include_links, report(py, cb, 0.8, 0.2, None)?),
        )?;
        if let Some(root) = scene.getattr("_root").ok().filter(|r| !r.is_none()) {
            root.call_method0("_index_assemblies")?;
        }
        if let Some(cb) = cb {
            cb.call1((1., "Finishing…"))?;
        }
        Ok(())
    })();
    finite.call_method1("reset", (finite_token,))?;
    count.call_method1("reset", (count_token,))?;
    if suspend_gc {
        gc.call_method0("enable")?;
    }
    result
}
#[pyfunction]
#[pyo3(signature=(scene,path,include_links,progress=None))]
fn igz_load_links<'p>(
    scene: &Obj<'p>,
    path: &Obj<'p>,
    include_links: bool,
    progress: Option<&Obj<'p>>,
) -> PyResult<()> {
    let py = scene.py();
    let links = py.import("core.links")?;
    let path = py.import("pathlib")?.call_method1("Path", (path,))?;
    scene.setattr("_project_host_path", path.call_method0("resolve")?.str()?)?;
    let read = (|| -> PyResult<()> {
        let shared = py.import("ingetrazo_core.project")?.call_method1("read", (scene,))?;
        if shared.is_truthy()? {
            scene
                .getattr("plugin_data")?
                .set_item("ifc_identity", shared.get_item("ifc_identity")?)?
        }
        Ok(())
    })();
    match read {
        Err(e) if e.is_instance_of::<PyOSError>(py) || e.is_instance_of::<PyValueError>(py) => {
            scene.setattr("project_warning", e.value(py).str()?)?
        }
        other => other?,
    }
    scene.setattr("link_errors", PyDict::new(py))?;
    let linked = links.call_method1("linked_groups", (scene,))?;
    if !include_links {
        for g in linked.try_iter()? {
            g?.call_method0("_detach")?;
        }
        let keep = PyList::empty(py);
        for g in scene.getattr("groups")?.try_iter()? {
            let g = g?;
            if !links.call_method1("is_linked", (&g,))?.is_truthy()? {
                keep.append(g)?
            }
        }
        scene.setattr("groups", keep)?;
    } else if linked.is_truthy()? {
        let kw = PyDict::new(py);
        // Only the link load that wrapped the caller's exception can unwrap
        // it. An inner load must let it cross the recoverable-source handler.
        let scope = PyDict::new(py).into_any();
        if let Some(cb) = progress {
            kw.set_item("progress", report(py, Some(cb), 0., 1., Some(&scope))?)?
        }
        match crate::model::module(py, "ingetrazo_core")?.call_method("link_resolve", (scene, path), Some(&kw)) {
            Ok(_) => (),
            Err(e) if e.is_instance_of::<LinkInterrupted>(py) => {
                if e.value(py).getattr("scope")?.is(&scope) {
                    return Err(e.cause(py).unwrap_or(e));
                }
                return Err(e);
            }
            Err(e) if e.is_instance_of::<PyException>(py) => {
                let errors = PyDict::new(py);
                errors.set_item("", e.value(py).str()?)?;
                scene.setattr("link_errors", errors)?;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
#[pyclass]
struct PhotoBlob {
    archive: Py<PyAny>,
}
#[pymethods]
impl PhotoBlob {
    fn __call__<'p>(&self, name: &Obj<'p>) -> PyResult<Obj<'p>> {
        let py = name.py();
        match self.archive.bind(py).call_method1("read", (name,)) {
            Err(e) if e.is_instance_of::<PyKeyError>(py) => Ok(PyBytes::new(py, b"").into_any()),
            other => other,
        }
    }
}
#[pyfunction]
fn igz_unpack_photo_mesh<'p>(payload: &Obj<'p>, archive: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = payload.py();
    let photo = payload.call_method1("get", ("photo_mesh",))?;
    if !photo.is_truthy()? {
        return Ok(py.None().into_bound(py));
    }
    py.import("ingetrazo_core.georef.photomesh")?.call_method1(
        "unpack_mesh",
        (
            photo,
            Py::new(
                py,
                PhotoBlob {
                    archive: archive.clone().unbind(),
                },
            )?,
        ),
    )
}
#[pyfunction]
fn igz_bulky(node: &Obj<'_>) -> PyResult<bool> {
    if let Ok(l) = node.cast::<PyList>() {
        return Ok(l.len() > 64);
    }
    if let Ok(d) = node.cast::<PyDict>() {
        for (_, v) in d.iter() {
            if (v.is_instance_of::<PyList>() || v.is_instance_of::<PyDict>()) && v.len()? > 64 {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
#[pyfunction]
fn igz_release_in_steps(data: &Obj<'_>, pump: &Obj<'_>) -> PyResult<()> {
    let mut stack = vec![data.clone()];
    let mut last = Instant::now();
    let mut tick = || -> PyResult<()> {
        if last.elapsed() > Duration::from_millis(250) {
            pump.call0()?;
            last = Instant::now()
        }
        Ok(())
    };
    while let Some(node) = stack.pop() {
        if let Ok(d) = node.cast::<PyDict>() {
            for k in d.keys().iter() {
                let child = d.call_method1("pop", (k,))?;
                if igz_bulky(&child)? {
                    stack.push(child)
                }
            }
        } else if let Ok(l) = node.cast::<PyList>() {
            while !l.is_empty() {
                let child = l.call_method0("pop")?;
                if igz_bulky(&child)? {
                    stack.push(child)
                }
                tick()?
            }
        }
        tick()?;
    }
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(igz_load_into, m)?)?;
    m.add_function(wrap_pyfunction!(igz_load_links, m)?)?;
    m.add_function(wrap_pyfunction!(igz_unpack_photo_mesh, m)?)?;
    m.add_function(wrap_pyfunction!(igz_bulky, m)?)?;
    m.add_function(wrap_pyfunction!(igz_release_in_steps, m)?)
}
