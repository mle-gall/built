// SPDX-License-Identifier: GPL-3.0-or-later
//! Model clipboard serialization and its system-clipboard offer: the platform
//! pasteboard of the native shell, or this process alone without one.
use crate::math::{xyz, Vec3};
use pyo3::{
    exceptions::{PyKeyError, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict, PyList, PyTuple},
};
use std::collections::HashSet;
type Obj<'p> = Bound<'p, PyAny>;
fn val<'p>(d: &Obj<'p>, k: &str) -> PyResult<Obj<'p>> {
    d.call_method1("get", (k,))
}
fn seq<'p>(d: &Obj<'p>, k: &str) -> PyResult<Obj<'p>> {
    let v = val(d, k)?;
    if v.is_none() {
        Ok(PyList::empty(d.py()).into_any())
    } else {
        Ok(v)
    }
}
fn scene_file<'p, T>(py: Python<'p>, work: impl FnOnce(&Obj<'p>) -> PyResult<T>) -> PyResult<T> {
    let kw = PyDict::new(py);
    kw.set_item("prefix", "ingetrazo-clip-")?;
    let temp = py
        .import("tempfile")?
        .call_method("TemporaryDirectory", (), Some(&kw))?;
    let path = py
        .import("pathlib")?
        .call_method1("Path", (temp.getattr("name")?, "scene.igz"))?;
    let r = work(&path);
    temp.call_method0("cleanup")?;
    r
}
fn include(registry: &Obj<'_>, mats: &Obj<'_>, attrs: &Obj<'_>) -> PyResult<()> {
    let mut d = attrs.clone();
    let mut seen = HashSet::new();
    while let Ok(item) = d.cast::<PyDict>() {
        if !seen.insert(d.as_ptr()) {
            break;
        }
        let key = val(item, "mat")?;
        if registry.contains(&key)? {
            mats.set_item(&key, registry.get_item(&key)?)?;
        }
        d = val(item, "back")?;
    }
    Ok(())
}
#[pyfunction]
fn clip_encode<'p>(py: Python<'p>, clip: &Obj<'p>) -> PyResult<Obj<'p>> {
    let scene = py.import("core.scene")?.call_method0("Scene")?;
    let mesh = scene.getattr("mesh")?;
    for row in seq(clip, "faces")?.try_iter()? {
        let row = row?;
        let holes = row.get_item(1)?;
        let f = match mesh.call_method1(
            "add_face",
            (row.get_item(0)?, holes.is_truthy()?.then_some(holes)),
        ) {
            Err(e) if e.is_instance_of::<PyValueError>(py) => continue,
            other => other?,
        };
        if row.len()? > 2 {
            let attrs = row.get_item(2)?;
            if attrs.is_truthy()? {
                f.setattr("attrs", py.get_type::<PyDict>().call1((attrs,))?)?;
            }
        }
    }
    let groups = scene.getattr("groups")?;
    groups.call_method1("extend", (seq(clip, "groups")?,))?;
    let mut registry = val(clip, "registry")?;
    if !registry.is_truthy()? {
        registry = val(clip, "materials")?;
    }
    if registry.is_truthy()? {
        let mats = scene.getattr("materials")?;
        let placements = py.import("core.group")?.getattr("iter_placements")?;
        let face_attrs = py.import("core.mesh")?.getattr("face_attrs")?;
        let mut seen = HashSet::new();
        for root in groups.try_iter()? {
            for row in placements.call1((root?,))?.try_iter()? {
                let g = row?.get_item(0)?;
                include(&registry, &mats, &g.getattr("material")?)?;
                let mesh = g.getattr("mesh")?;
                if seen.insert(mesh.as_ptr()) {
                    for a in face_attrs.call1((mesh,))?.try_iter()? {
                        include(&registry, &mats, &a?)?;
                    }
                }
            }
        }
        for f in mesh.getattr("faces")?.try_iter()? {
            include(&registry, &mats, &f?.getattr("attrs")?)?;
        }
    }
    py.import("core.layers")?
        .call_method1("add_missing", (&scene, val(clip, "layers")?))?;
    let header = PyDict::new(py);
    header.set_item("format", 1)?;
    header.set_item("ref", xyz(&clip.get_item("ref")?)?.to_array().to_vec())?;
    let edges = PyList::empty(py);
    for row in seq(clip, "edges")?.try_iter()? {
        let row = row?;
        let (a, b, s, c) = (
            row.get_item(0)?,
            row.get_item(1)?,
            row.get_item(2)?,
            row.get_item(3)?,
        );
        edges.append((
            xyz(&a)?.to_array().to_vec(),
            xyz(&b)?.to_array().to_vec(),
            s.is_truthy()?,
            c,
        ))?;
    }
    header.set_item("edges", edges)?;
    header.set_item(
        "edge_layers",
        py.get_type::<PyList>()
            .call1((seq(clip, "edge_layers")?,))?,
    )?;
    let doc = scene_file(py, |p| {
        py.import("formats.igz")?
            .call_method1("save_scene", (&scene, p))?;
        p.call_method0("read_bytes")
    })?;
    let out = py.import("io")?.call_method0("BytesIO")?;
    let zip = py
        .import("zipfile")?
        .call_method1("ZipFile", (&out, "w", 0))?;
    let written = (|| -> PyResult<()> {
        zip.call_method1(
            "writestr",
            (
                "clip.json",
                py.import("json")?.call_method1("dumps", (header,))?,
            ),
        )?;
        zip.call_method1("writestr", ("scene.igz", doc))?;
        Ok(())
    })();
    zip.call_method0("close")?;
    written?;
    out.call_method0("getvalue")
}
fn points<'p>(py: Python<'p>, points: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let l = PyList::empty(py);
    for p in points.try_iter()? {
        l.append(Py::new(py, Vec3::from(xyz(&p?)?))?)?;
    }
    Ok(l)
}
#[pyfunction]
fn clip_decode<'p>(py: Python<'p>, data: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    let module = py.import("zipfile")?;
    let unpack = (|| -> PyResult<(Obj, Obj)> {
        let buf = py
            .import("io")?
            .call_method1("BytesIO", (py.get_type::<PyBytes>().call1((data,))?,))?;
        let zip = module.call_method1("ZipFile", (buf,))?;
        let result = (|| {
            py.import("core.zip_limits")?
                .call_method1("check", (&zip, "the clipboard"))?;
            Ok((
                py.import("json")?
                    .call_method1("loads", (zip.call_method1("read", ("clip.json",))?,))?,
                zip.call_method1("read", ("scene.igz",))?,
            ))
        })();
        zip.call_method0("close")?;
        result
    })();
    let (head, doc) = match unpack {
        Err(e)
            if e.is_instance_of::<PyValueError>(py)
                || e.is_instance_of::<PyKeyError>(py)
                || e.value(py).is_instance(&module.getattr("BadZipFile")?)? =>
        {
            return Ok(None)
        }
        v => v?,
    };
    if !val(&head, "format")?.eq(1)? {
        return Ok(None);
    }
    let scene = py.import("core.scene")?.call_method0("Scene")?;
    scene_file(py, |path| {
        path.call_method1("write_bytes", (doc,))?;
        py.import("formats.igz")?
            .call_method1("load_into", (&scene, path))
    })?;
    let faces = PyList::empty(py);
    for f in scene.getattr("mesh")?.getattr("faces")?.try_iter()? {
        let f = f?;
        let holes = PyList::empty(py);
        for h in f.getattr("holes")?.try_iter()? {
            holes.append(points(py, &h?)?)?;
        }
        let attrs = f.getattr("attrs")?;
        let attrs = if attrs.is_truthy()? {
            py.get_type::<PyDict>().call1((attrs,))?
        } else {
            PyDict::new(py).into_any()
        };
        faces.append((points(py, &f.getattr("vertices")?)?, holes, attrs))?;
    }
    let point = |p: &Obj<'p>| -> PyResult<_> { Py::new(py, Vec3::from(xyz(p)?)) };
    let edges = PyList::empty(py);
    for row in seq(&head, "edges")?.try_iter()? {
        let row = row?;
        let (a, b, s, c) = (
            row.get_item(0)?,
            row.get_item(1)?,
            row.get_item(2)?,
            row.get_item(3)?,
        );
        edges.append((point(&a)?, point(&b)?, s, c))?;
    }
    let groups = scene.getattr("groups")?;
    for g in groups.try_iter()? {
        let g = g?;
        if g.getattr_opt("xform")?.is_none_or(|v| v.is_none()) {
            g.setattr("xform", py.get_type::<crate::math::Matrix4>().call0()?)?;
            g.setattr("component", false)?;
        }
    }
    let out = PyDict::new(py);
    out.set_item("faces", faces)?;
    out.set_item("edges", edges)?;
    out.set_item("groups", py.get_type::<PyList>().call1((groups,))?)?;
    out.set_item(
        "edge_layers",
        py.get_type::<PyList>()
            .call1((seq(&head, "edge_layers")?,))?,
    )?;
    out.set_item(
        "layers",
        py.import("core.layers")?
            .call_method1("template_of", (&scene,))?,
    )?;
    out.set_item(
        "materials",
        py.get_type::<PyDict>()
            .call1((scene.getattr("materials")?,))?,
    )?;
    out.set_item("ref", point(&head.get_item("ref")?)?)?;
    Ok(Some(out.into_any()))
}
const MIME: &str = "application/x-ingetrazo-clipboard";

/// This process's last Copy: the clipboard dict, its encoded payload and
/// the pasteboard change count it was written at (None without a shell:
/// the offer then lives in this process only).
struct Published {
    clip: Py<PyAny>,
    data: Py<PyBytes>,
    count: Option<i64>,
}

static PUBLISHED: std::sync::Mutex<Option<Published>> = std::sync::Mutex::new(None);

fn native(py: Python<'_>) -> PyResult<Option<Obj<'_>>> {
    let Ok(module) = py.import("app_shell.native") else {
        return Ok(None);
    };
    let module = module.into_any();
    Ok(module.call_method0("available")?.is_truthy()?.then_some(module))
}

fn payload(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("formats.clip_payload")
}

/// What the clipboard holds of ours, as a Copy offers it: `clip` is the
/// dict when this process published it (None for another process's), and
/// `data(MIME).data()` gives the payload bytes, read on demand.
#[pyclass(module = "ingetrazo_core", name = "ClipOffer", frozen)]
pub struct ClipOffer {
    #[pyo3(get)]
    clip: Option<Py<PyAny>>,
    bytes: std::sync::Mutex<Option<Py<PyBytes>>>,
}

/// The payload of an offer (`.data()` as a byte array answered).
#[pyclass(module = "ingetrazo_core", name = "ClipData", frozen)]
pub struct ClipData {
    bytes: Py<PyBytes>,
}

#[pymethods]
impl ClipData {
    fn data(&self, py: Python<'_>) -> Py<PyBytes> {
        self.bytes.clone_ref(py)
    }
    fn __len__(&self, py: Python<'_>) -> usize {
        self.bytes.bind(py).as_bytes().len()
    }
}

impl ClipOffer {
    fn bytes<'p>(&self, py: Python<'p>) -> PyResult<Option<Bound<'p, PyBytes>>> {
        if let Some(b) = self.bytes.lock().unwrap().as_ref() {
            return Ok(Some(b.bind(py).clone()));
        }
        let read = match native(py)? {
            Some(shell) => shell.call_method1("pasteboard_read", (MIME,))?,
            None => return Ok(None),
        };
        if read.is_none() {
            return Ok(None);
        }
        let b = read.cast_into::<PyBytes>()?;
        *self.bytes.lock().unwrap() = Some(b.clone().unbind());
        Ok(Some(b))
    }
}

#[pymethods]
impl ClipOffer {
    /// An offer of `clip` (encoded when first read).
    #[new]
    #[pyo3(signature = (clip = None))]
    fn new(clip: Option<Py<PyAny>>) -> Self {
        Self { clip, bytes: std::sync::Mutex::new(None) }
    }
    fn formats(&self) -> Vec<&'static str> {
        vec![MIME]
    }
    #[pyo3(name = "hasFormat")]
    fn has_format(&self, mime: &str) -> bool {
        mime == MIME
    }
    /// The payload for `mime` (None for any other type).
    fn data(&self, py: Python<'_>, mime: &str) -> PyResult<Option<ClipData>> {
        if mime != MIME {
            return Ok(None);
        }
        if self.bytes.lock().unwrap().is_none() {
            if let Some(clip) = &self.clip {
                let encoded = payload(py)?.call_method1("encode", (clip.bind(py),))?.cast_into::<PyBytes>()?;
                *self.bytes.lock().unwrap() = Some(encoded.unbind());
            }
        }
        Ok(self.bytes(py)?.map(|b| ClipData { bytes: b.unbind() }))
    }
    #[pyo3(name = "retrieveData")]
    fn retrieve_data<'p>(&self, py: Python<'p>, mime: &str, _kind: Option<Obj<'p>>) -> PyResult<Option<ClipData>> {
        self.data(py, mime)
    }
}

/// What the clipboard offers now in our format, or None.
#[pyfunction]
fn clip_current_offer(py: Python<'_>) -> PyResult<Option<ClipOffer>> {
    // Copied out first: the lock is never held across a call into Python.
    let published = PUBLISHED
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| (p.clip.clone_ref(py), p.data.clone_ref(py), p.count));
    let ours = |clip: Py<PyAny>, data: Py<PyBytes>| ClipOffer { clip: Some(clip), bytes: std::sync::Mutex::new(Some(data)) };
    match native(py)? {
        Some(shell) => {
            let (types, count): (Vec<String>, i64) = shell.call_method0("pasteboard_types")?.extract()?;
            if !types.iter().any(|t| t == MIME) {
                return Ok(None);
            }
            // Ours while nobody wrote the pasteboard since.
            Ok(Some(match published.filter(|p| p.2 == Some(count)) {
                Some((clip, data, _)) => ours(clip, data),
                None => ClipOffer { clip: None, bytes: std::sync::Mutex::new(None) },
            }))
        }
        None => Ok(published.map(|(clip, data, _)| ours(clip, data))),
    }
}

#[pyfunction]
fn clip_available(py: Python<'_>) -> PyResult<bool> {
    Ok(clip_current_offer(py)?.is_some())
}

/// Put `clip` on the clipboard: on the platform pasteboard when a shell
/// runs (other windows and processes paste it), else in this process.
#[pyfunction]
fn clip_publish<'p>(py: Python<'p>, clip: &Obj<'p>) -> PyResult<()> {
    let encoded = payload(py)?.call_method1("encode", (clip,))?;
    let data = encoded.cast_into::<PyBytes>()?;
    let mut count = None;
    if let Some(shell) = native(py)? {
        let items = PyList::new(py, [PyTuple::new(py, [pyo3::types::PyString::new(py, MIME).into_any(), data.clone().into_any()])?])?;
        if shell.call_method1("pasteboard_write", (items,))?.is_truthy()? {
            let (_, n): (Vec<String>, i64) = shell.call_method0("pasteboard_types")?.extract()?;
            count = Some(n);
        }
    }
    let previous = PUBLISHED.lock().unwrap().replace(Published { clip: clip.clone().unbind(), data: data.unbind(), count });
    drop(previous); // released outside the lock
    Ok(())
}

/// Nothing left to render at quit: the payload is written when copied.
#[pyfunction]
fn clip_flush() {}

/// Another process's Copy, decoded (None when the offer is ours or absent).
#[pyfunction]
fn clip_foreign(py: Python<'_>) -> PyResult<Obj<'_>> {
    let none = py.None().into_bound(py);
    let Some(offer) = clip_current_offer(py)? else { return Ok(none) };
    if offer.clip.is_some() {
        return Ok(none);
    }
    match offer.bytes(py)? {
        Some(b) if !b.as_bytes().is_empty() => payload(py)?.call_method1("decode", (b,)),
        _ => Ok(none),
    }
}

/// Our own offer decoded afresh, for another window of this process
/// (None when it is `current`, was adopted from, or is not ours).
#[pyfunction(signature=(current,adopted_from=None))]
fn clip_own<'p>(py: Python<'p>, current: &Obj<'p>, adopted_from: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    let none = py.None().into_bound(py);
    let Some(offer) = clip_current_offer(py)? else { return Ok(none) };
    let Some(clip) = offer.clip.as_ref().map(|c| c.bind(py).clone()) else { return Ok(none) };
    if clip.is(current) || adopted_from.is_some_and(|v| v.is(&clip)) {
        return Ok(none);
    }
    match offer.bytes(py)? {
        Some(b) if !b.as_bytes().is_empty() => payload(py)?.call_method1("decode", (b,)),
        _ => Ok(none),
    }
}

/// The payload of an older offer object (`clip`, cached in `_bytes`).
#[pyfunction]
fn clip_retrieve<'p>(py: Python<'p>, offer: &Obj<'p>, mime: &str) -> PyResult<Obj<'p>> {
    if mime != MIME {
        return Ok(py.None().into_bound(py));
    }
    let cached = offer.getattr("_bytes")?;
    if !cached.is_none() {
        return Ok(cached);
    }
    let data = match payload(py)?.call_method1("encode", (offer.getattr("clip")?,)) {
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyException>(py) => PyBytes::new(py, b"").into_any(),
        r => r?,
    };
    offer.setattr("_bytes", &data)?;
    Ok(data)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    macro_rules! add{($($f:ident),*)=>{$(m.add_function(wrap_pyfunction!($f,m)?)?;)*}}
    add!(
        clip_retrieve,
        clip_encode,
        clip_decode,
        clip_current_offer,
        clip_available,
        clip_publish,
        clip_flush,
        clip_foreign,
        clip_own
    );
    m.add_class::<ClipOffer>()?;
    m.add_class::<ClipData>()?;
    Ok(())
}
