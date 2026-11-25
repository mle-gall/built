// SPDX-License-Identifier: GPL-3.0-or-later
//! Cooperative snapshots. Each Python next() polls just one slice. Only owned
//! references survive a slice; detached arrays and metadata can then cross to
//! the save worker without exposing the live document.
use numpy::{ndarray::Array2, IntoPyArray};
use pyo3::{
    exceptions::{PyRecursionError, PyStopIteration, PyValueError},
    prelude::*,
    types::{PyDict, PyIterator, PyList, PySlice, PyTuple},
};
use std::{
    collections::HashMap,
    future::{poll_fn, Future},
    pin::Pin,
    task::{Context, Poll, Waker},
};
type Owned = Py<PyAny>;
type Obj<'p> = Bound<'p, PyAny>;
type Job = Pin<Box<dyn Future<Output = PyResult<Owned>> + Send>>;
#[pyclass]
pub(crate) struct Steps {
    work: std::sync::Mutex<Option<Job>>,
}
#[pymethods]
impl Steps {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn close(&mut self) {
        *self.work.get_mut().unwrap() = None;
    }
    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<Owned>> {
        let Some(work) = self.work.get_mut().unwrap() else {
            return Ok(None);
        };
        match work.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Pending => Ok(Some(py.None())),
            Poll::Ready(result) => {
                *self.work.get_mut().unwrap() = None;
                Err(PyStopIteration::new_err((result?,)))
            }
        }
    }
}
pub(crate) fn steps(job: impl Future<Output = PyResult<Owned>> + Send + 'static) -> Steps {
    Steps {
        work: std::sync::Mutex::new(Some(Box::pin(job))),
    }
}
pub(crate) async fn pause() {
    let mut first = true;
    poll_fn(|_| {
        if std::mem::take(&mut first) {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    })
    .await
}
pub(crate) fn iterator(o: &Obj<'_>) -> PyResult<Py<PyIterator>> {
    Ok(o.try_iter()?.unbind())
}
pub(crate) fn next(it: &Py<PyIterator>) -> PyResult<Option<Owned>> {
    Python::attach(|py| {
        it.bind(py)
            .clone()
            .next()
            .map(|v| v.map(Bound::unbind))
            .transpose()
    })
}
fn api(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("formats.igz")
}
#[pyclass(get_all, set_all, module = "ingetrazo_core")]
struct Geometry {
    arrays: Option<Owned>,
    attrs: Option<Owned>,
    layers: Option<Owned>,
    deferred: Option<Owned>,
}
#[pymethods]
impl Geometry {
    #[new]
    #[pyo3(signature=(arrays=None,attrs=None,layers=None,deferred=None))]
    fn new(
        arrays: Option<Owned>,
        attrs: Option<Owned>,
        layers: Option<Owned>,
        deferred: Option<Owned>,
    ) -> Self {
        Self {
            arrays,
            attrs,
            layers,
            deferred,
        }
    }
}
#[pyclass(get_all, set_all, module = "ingetrazo_core")]
pub(crate) struct Survey {
    mesh: Owned,
    thumbnail: Option<Owned>,
}
#[pymethods]
impl Survey {
    #[new]
    fn new(mesh: Owned) -> Self {
        Self {
            mesh,
            thumbnail: None,
        }
    }
}

enum CopyKind {
    Scalar,
    Array,
    Mapping,
    Sequence,
}
fn copy(value: Owned, depth: usize) -> Job {
    Box::pin(async move {
        if depth >= 128
            && depth
                >= Python::attach(|py| {
                    py.import("sys")?
                        .call_method0("getrecursionlimit")?
                        .extract::<usize>()
                })?
        {
            return Err(PyRecursionError::new_err(
                "snapshot metadata is nested too deeply",
            ));
        }
        let kind = Python::attach(|py| -> PyResult<_> {
            let v = value.bind(py);
            Ok(if v.is_instance_of::<PyDict>() {
                CopyKind::Mapping
            } else if v.is_instance_of::<PyList>() || v.is_instance_of::<PyTuple>() {
                CopyKind::Sequence
            } else if v.is_instance(&py.import("numpy")?.getattr("ndarray")?)? {
                CopyKind::Array
            } else {
                CopyKind::Scalar
            })
        })?;
        match kind {
            CopyKind::Scalar => Ok(value),
            CopyKind::Array => {
                let (array, length, stride) = Python::attach(|py| -> PyResult<_> {
                    let v = value.bind(py);
                    let a = py
                        .import("numpy")?
                        .call_method1("empty", (v.getattr("shape")?, v.getattr("dtype")?))?;
                    if v.getattr("ndim")?.extract::<usize>()? == 0 {
                        a.set_item(py.Ellipsis(), v)?;
                        return Ok((a.unbind(), 0, 1));
                    }
                    let bytes = v
                        .get_item(PySlice::new(py, 0, 1, 1))?
                        .getattr("nbytes")?
                        .extract::<usize>()?;
                    Ok((a.unbind(), v.len()?, (262144 / bytes.max(1)).max(1)))
                })?;
                for i in (0..length).step_by(stride) {
                    Python::attach(|py| {
                        let s = PySlice::new(py, i as isize, (i + stride).min(length) as isize, 1);
                        array.bind(py).set_item(&s, value.bind(py).get_item(&s)?)
                    })?;
                    pause().await;
                }
                Ok(array)
            }
            CopyKind::Mapping => {
                let (it, out) = Python::attach(|py| -> PyResult<_> {
                    Ok((
                        iterator(&value.bind(py).call_method0("items")?)?,
                        PyDict::new(py).unbind(),
                    ))
                })?;
                while let Some(pair) = next(&it)? {
                    let (key, v) = Python::attach(|py| -> PyResult<_> {
                        let p = pair.bind(py);
                        Ok((p.get_item(0)?.unbind(), p.get_item(1)?.unbind()))
                    })?;
                    let saved = copy(v, depth + 1).await?;
                    Python::attach(|py| out.bind(py).set_item(key, saved))?;
                    pause().await
                }
                Ok(out.into_any())
            }
            CopyKind::Sequence => {
                let (it, out) = Python::attach(|py| -> PyResult<_> {
                    Ok((iterator(value.bind(py))?, PyList::empty(py).unbind()))
                })?;
                while let Some(v) = next(&it)? {
                    let saved = copy(v, depth + 1).await?;
                    Python::attach(|py| out.bind(py).append(saved))?;
                    pause().await
                }
                Ok(out.into_any())
            }
        }
    })
}
#[pyfunction]
fn copy_steps(value: Owned) -> Steps {
    steps(copy(value, 0))
}
fn geometry_record(geometry: Geometry) -> PyResult<Owned> {
    Python::attach(|py| {
        let d = PyDict::new(py);
        d.set_item("geom", Py::new(py, geometry)?)?;
        Ok(d.into_any().unbind())
    })
}
async fn capture_geometry(mesh: Owned) -> PyResult<Owned> {
    let deferred = Python::attach(|py| -> PyResult<_> {
        let m = mesh.bind(py);
        Ok(m.get_type()
            .is(&py.import("ingetrazo_core.mesh")?.getattr("DeferredMesh")?)
            .then(|| {
                m.getattr("__dict__")?
                    .get_item("_deferred_source")
                    .map(Bound::unbind)
            })
            .transpose()?)
    })?;
    if let Some(src) = deferred {
        return geometry_record(Geometry::new(None, None, None, Some(copy(src, 0).await?)));
    }
    let empty = Python::attach(|py| -> PyResult<_> {
        let m = mesh.bind(py);
        if m.getattr("edges")?.len()? != 0 || m.getattr("faces")?.len()? != 0 {
            return Ok(None);
        }
        let d = PyDict::new(py);
        for k in ["edges", "faces"] {
            d.set_item(k, PyList::empty(py))?
        }
        Ok(Some(d.into_any().unbind()))
    })?;
    if let Some(e) = empty {
        return Ok(e);
    }
    let packed = Python::attach(|py| -> PyResult<_> {
        let p = api(py)?.call_method1("_store_columns", (mesh.bind(py), true))?;
        if p.is_none() {
            return Ok(None);
        }
        let (a, attrs, names): (Bound<'_, PyDict>, Owned, Owned) = p.extract()?;
        let np = py.import("numpy")?;
        let n = a.get_item("edges")?.unwrap().len()?;
        let layer = a.call_method1("pop", ("edge_layer", py.None()))?;
        let full = |t| np.call_method1("full", (n, -1, t));
        if !a.contains("edge_curve")? {
            a.set_item("edge_curve", full("int64")?)?
        }
        a.set_item(
            "edge_layer",
            if layer.is_none() {
                full("int32")?
            } else {
                layer
            },
        )?;
        Ok(Some((a.into_any().unbind(), attrs, names)))
    })?;
    if let Some((a, attrs, names)) = packed {
        return geometry_record(Geometry::new(
            Some(a),
            Some(copy(attrs, 0).await?),
            Some(names),
            None,
        ));
    }
    let mut vertices = crate::file_mesh::Vertices::new(true);
    let mut edges = vec![];
    let mut flags = vec![];
    let mut curves = vec![];
    let mut layers = vec![];
    let layer_names = Python::attach(|py| PyDict::new(py).unbind());
    let it = Python::attach(|py| iterator(&mesh.bind(py).getattr("edges")?))?;
    let mut i = 0;
    while let Some(e) = next(&it)? {
        Python::attach(|py| -> PyResult<()> {
            let e = e.bind(py);
            edges.extend([
                vertices.index(&e.getattr("v0")?)?,
                vertices.index(&e.getattr("v1")?)?,
            ]);
            let soft = e.getattr("soft")?.is_truthy()?;
            flags.push(
                soft as u8
                    | ((e.getattr("hidden")?.is_truthy()? as u8) << 1)
                    | (((soft && e.getattr("flat")?.is_truthy()?) as u8) << 2),
            );
            curves.push(e.getattr("curve")?.extract::<Option<i64>>()?.unwrap_or(-1));
            let tag = e.getattr("layer")?;
            let names = layer_names.bind(py);
            layers.push(if tag.is_none() {
                -1
            } else {
                names
                    .call_method1("setdefault", (tag, names.len()))?
                    .extract::<i32>()?
            });
            Ok(())
        })?;
        if i % 128 == 0 {
            pause().await
        }
        i += 1;
    }
    let mut rings = vec![];
    let mut sizes = vec![];
    let mut counts = vec![];
    let mut indices = vec![];
    let table = Python::attach(|py| PyList::empty(py).unbind());
    let mut attrs_seen = HashMap::new();
    let it = Python::attach(|py| iterator(&mesh.bind(py).getattr("faces")?))?;
    let mut i = 0;
    while let Some(f) = next(&it)? {
        let loops = Python::attach(|py| -> PyResult<_> {
            let f = f.bind(py);
            let mut loops = vec![f.getattr("loop")?];
            loops.extend(crate::file_mesh::maximal_vertex_holes(
                &f.getattr("hole_loops")?,
            )?);
            Ok(loops.into_iter().map(Bound::unbind).collect::<Vec<_>>())
        })?;
        counts.push(loops.len() as i32);
        for ring in loops {
            let (n, it) = Python::attach(|py| -> PyResult<_> {
                let r = ring.bind(py);
                Ok((r.len()?, iterator(r)?))
            })?;
            sizes.push(n as i32);
            for start in (0..n).step_by(256) {
                Python::attach(|py| -> PyResult<()> {
                    for v in it.bind(py).clone().take(256) {
                        rings.push(vertices.index(&v?)?)
                    }
                    Ok(())
                })?;
                if start + 256 < n {
                    pause().await
                }
            }
        }
        let entry = Python::attach(|py| -> PyResult<_> {
            let a = api(py)?;
            let attrs = a.call_method1("_face_attrs_json", (f.bind(py),))?;
            if attrs.is_truthy()? {
                Ok(Some((
                    a.call_method1("_attrs_key", (&attrs,))?
                        .extract::<String>()?,
                    attrs.unbind(),
                )))
            } else {
                Ok(None)
            }
        })?;
        let id = match entry {
            None => -1,
            Some((key, attrs)) => {
                if let Some(i) = attrs_seen.get(&key) {
                    *i
                } else {
                    let i = attrs_seen.len() as i32;
                    attrs_seen.insert(key, i);
                    let saved = copy(attrs, 0).await?;
                    Python::attach(|py| table.bind(py).append(saved))?;
                    i
                }
            }
        };
        indices.push(id);
        if i % 64 == 0 {
            pause().await
        }
        i += 1;
    }
    let (arrays, names) = Python::attach(|py| -> PyResult<_> {
        let a = PyDict::new(py);
        let pos = vertices.positions();
        a.set_item(
            "pos",
            Array2::from_shape_vec((pos.len() / 3, 3), pos)
                .unwrap()
                .into_pyarray(py),
        )?;
        a.set_item(
            "edges",
            Array2::from_shape_vec((edges.len() / 2, 2), edges)
                .unwrap()
                .into_pyarray(py),
        )?;
        a.set_item("rings", rings.into_pyarray(py))?;
        a.set_item("ring_sizes", sizes.into_pyarray(py))?;
        a.set_item("ring_counts", counts.into_pyarray(py))?;
        a.set_item("face_attr", indices.into_pyarray(py))?;
        a.set_item("edge_flags", flags.into_pyarray(py))?;
        a.set_item("edge_curve", curves.into_pyarray(py))?;
        a.set_item("edge_layer", layers.into_pyarray(py))?;
        Ok((
            a.into_any().unbind(),
            layer_names.bind(py).keys().into_any().unbind(),
        ))
    })?;
    geometry_record(Geometry::new(
        Some(arrays),
        Some(table.into_any()),
        Some(names),
        None,
    ))
}
#[pyfunction]
fn geometry_steps(mesh: Owned) -> Steps {
    steps(capture_geometry(mesh))
}
#[pyclass]
struct Capture {
    cache: Py<PyDict>,
}
#[pymethods]
impl Capture {
    fn __call__(&self, py: Python<'_>, mesh: Owned) -> Steps {
        let cache = self.cache.clone_ref(py);
        steps(async move {
            let id = mesh.as_ptr() as usize;
            let cached =
                Python::attach(|py| cache.bind(py).get_item(id).map(|o| o.map(Bound::unbind)))?;
            let entry = if let Some(e) = cached {
                e
            } else {
                let e = capture_geometry(mesh).await?;
                Python::attach(|py| cache.bind(py).set_item(id, e.bind(py)))?;
                e
            };
            Python::attach(|py| Ok(entry.bind(py).cast::<PyDict>()?.copy()?.into_any().unbind()))
        })
    }
}
pub(crate) async fn drive(generator: Owned) -> PyResult<Owned> {
    loop {
        let result = Python::attach(|py| -> PyResult<_> {
            match generator.bind(py).call_method0("__next__") {
                Ok(_) => Ok(None),
                Err(e) if e.is_instance_of::<PyStopIteration>(py) => {
                    Ok(Some(e.value(py).getattr("value")?.unbind()))
                }
                Err(e) => Err(e),
            }
        })?;
        if let Some(value) = result {
            return Ok(value);
        }
        pause().await
    }
}
#[pyfunction]
fn snapshot_steps(scene: Owned) -> Steps {
    steps(async move {
        let generator = Python::attach(|py| -> PyResult<_> {
            let s = scene.bind(py);
            let document = py.import("copy")?.call_method1("copy", (s,))?;
            if let Some(m) = s.getattr("_loose_mesh").ok().filter(|v| !v.is_none()) {
                document.setattr("mesh", m)?
            }
            let cb = Py::new(
                py,
                Capture {
                    cache: PyDict::new(py).unbind(),
                },
            )?;
            Ok(api(py)?
                .call_method1("prepare_scene_steps", (document, cb))?
                .unbind())
        })?;
        copy(drive(generator).await?, 0).await
    })
}
#[pyfunction]
fn survey_steps(source: Owned) -> Steps {
    steps(async move {
        let mesh = Python::attach(|py| -> PyResult<_> {
            Ok(py
                .import("types")?
                .call_method0("SimpleNamespace")?
                .unbind())
        })?;
        for name in ["vertices", "uvs", "triangles"] {
            let v = Python::attach(|py| source.bind(py).getattr(name).map(Bound::unbind))?;
            let v = copy(v, 0).await?;
            Python::attach(|py| mesh.bind(py).setattr(name, v))?;
        }
        for name in [
            "materials",
            "source",
            "anchor",
            "visible",
            "layer",
            "vertical_reference",
        ] {
            Python::attach(|py| {
                mesh.bind(py).setattr(
                    name,
                    py.import("copy")?
                        .call_method1("deepcopy", (source.bind(py).getattr(name)?,))?,
                )
            })?;
            pause().await;
        }
        Python::attach(|py| {
            let images = PyDict::new(py);
            let raster = py.import("ingetrazo_core")?.getattr("as_raster")?;
            for (k, v) in source.bind(py).getattr("images")?.cast::<PyDict>()?.iter() {
                images.set_item(k, raster.call1((v,))?)?
            }
            mesh.bind(py).setattr("images", images)?;
            Ok(Py::new(py, Survey::new(mesh))?.into_any())
        })
    })
}
fn encoded_geometry<'p>(
    d: &Geometry,
    py: Python<'p>,
    sink: &Obj<'p>,
    encode: &Obj<'p>,
) -> PyResult<Obj<'p>> {
    let codec = api(py)?;
    let (meta, ne, nf) = if let Some(s) = &d.deferred {
        let src = s.bind(py);
        let tags = src
            .get_item("meta")?
            .call_method1("get", ("edge_layers",))?;
        let names = if tags.is_truthy()? {
            tags
        } else {
            PyList::empty(py).into_any()
        };
        let plan = py.import("ingetrazo_core")?.call_method1(
            "deferred_plan",
            (
                src.get_item("pos")?,
                src.get_item("edges")?,
                src.get_item("flags")?,
                src.get_item("curves")?,
                src.get_item("layers")?,
                names,
                src.get_item("rings")?,
                src.get_item("sizes")?,
                src.get_item("counts")?,
                src.call_method1("get", ("sequential", false))?,
            ),
        )?;
        let mesh = py.import("types")?.call_method0("SimpleNamespace")?;
        mesh.setattr("_deferred_source", src)?;
        (
            codec.call_method1("_pack_plan", (mesh, &plan, sink))?,
            plan.getattr("edges")?.len()?,
            plan.getattr("nfaces")?.extract::<usize>()?,
        )
    } else {
        let arrays = d
            .arrays
            .as_ref()
            .ok_or_else(|| PyValueError::new_err("snapshot has no geometry"))?
            .bind(py)
            .cast::<PyDict>()?;
        let np = py.import("numpy")?;
        let a = PyDict::new(py);
        for (k, v) in arrays.iter() {
            a.set_item(k, np.call_method1("asarray", (v,))?)?
        }
        for (k, n) in [("pos", 3), ("edges", 2)] {
            a.set_item(
                k,
                a.get_item(k)?
                    .ok_or_else(|| PyValueError::new_err(k))?
                    .call_method1("reshape", (-1, n))?,
            )?
        }
        let pos = a.get_item("pos")?.unwrap();
        if !np
            .call_method1("isfinite", (pos,))?
            .call_method0("all")?
            .is_truthy()?
        {
            return Err(PyErr::from_value(
                codec.call_method1("NonFiniteDocumentError", ("geometry",))?,
            ));
        }
        let ne = a.get_item("edges")?.unwrap().len()?;
        let nf = a
            .get_item("ring_counts")?
            .ok_or_else(|| PyValueError::new_err("missing ring counts"))?
            .len()?;
        (
            codec.call_method1("_write_geometry", (a, &d.attrs, &d.layers, sink))?,
            ne,
            nf,
        )
    };
    if !codec.call_method1("_bulk_sized", (ne, nf))?.is_truthy()? {
        meta.set_item("sequential", true)?
    }
    encode.call1((meta,))
}
#[pyfunction]
fn resolve_geometry<'p>(payload: &Obj<'p>, sink: &Obj<'p>, encode_meta: &Obj<'p>) -> PyResult<()> {
    let py = payload.py();
    let mut stack = vec![payload.clone()];
    let mut encoded = HashMap::new();
    while let Some(node) = stack.pop() {
        if let Ok(d) = node.cast::<PyDict>() {
            if let Some(g) = d.get_item("geom")? {
                if let Ok(s) = g.cast::<Geometry>() {
                    // Retain the captured object as an identity key until all
                    // references have been substituted, including shared meshes.
                    let key = g.as_ptr() as usize;
                    let (_, v) = match encoded.entry(key) {
                        std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                        std::collections::hash_map::Entry::Vacant(e) => {
                            let out = encoded_geometry(&s.borrow(), py, sink, encode_meta)?;
                            e.insert((g, out))
                        }
                    };
                    d.set_item("geom", &*v)?;
                }
            }
            stack.extend(d.iter().map(|(_, v)| v));
        } else if let Ok(l) = node.cast::<PyList>() {
            stack.extend(l.iter());
        }
    }
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Geometry>()?;
    m.add_class::<Survey>()?;
    m.add_function(wrap_pyfunction!(copy_steps, m)?)?;
    m.add_function(wrap_pyfunction!(geometry_steps, m)?)?;
    m.add_function(wrap_pyfunction!(snapshot_steps, m)?)?;
    m.add_function(wrap_pyfunction!(survey_steps, m)?)?;
    m.add_function(wrap_pyfunction!(resolve_geometry, m)?)
}
