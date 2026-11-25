// SPDX-License-Identifier: GPL-3.0-or-later
//! Model clipboard archives, delayed process-local offers and native pasteboard data.
use crate::math::{xyz, Vec3};
use pyo3::{
    exceptions::{PyKeyError, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict, PyList},
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
    crate::model::module(py, "ingetrazo_core")?.getattr("tag_edits")?
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
        crate::model::module(py, "ingetrazo_core")?.getattr("tag_edits")?
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
const PLATFORM_MIME: &str = "com.trolltech.anymime.application--x-ingetrazo-clipboard";
fn adapter(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("formats.clip")
}

#[pyclass]
struct ClipboardBytes {
    value: Py<PyBytes>,
}
#[pymethods]
impl ClipboardBytes {
    fn data(&self, py: Python<'_>) -> Py<PyBytes> {
        self.value.clone_ref(py)
    }
    fn __bytes__(&self, py: Python<'_>) -> Py<PyBytes> {
        self.data(py)
    }
    fn __len__(&self, py: Python<'_>) -> usize {
        self.value.bind(py).as_bytes().len()
    }
}

/// A plain byte offer. A foreign offer reads the platform pasteboard on first use.
#[pyclass(subclass)]
#[derive(Default)]
struct DataOffer {
    cached: Option<Py<ClipboardBytes>>,
}

#[pyclass(name = "ClipMime", extends = DataOffer)]
struct Offer {
    #[pyo3(get)]
    clip: Py<PyAny>,
}
#[pymethods]
impl Offer {
    #[new]
    fn new(clip: Py<PyAny>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(DataOffer::default()).add_subclass(Self { clip })
    }
}

#[pymethods]
impl DataOffer {
    fn formats(&self) -> Vec<&str> {
        vec![MIME]
    }
    #[pyo3(name = "hasFormat")]
    fn has_format(&self, mime: &str) -> bool {
        mime == MIME
    }
    #[pyo3(name="retrieveData",signature=(mime,_type=None))]
    fn retrieve(
        slf: &Bound<'_, Self>,
        mime: &str,
        _type: Option<&Obj<'_>>,
    ) -> PyResult<Option<Py<ClipboardBytes>>> {
        if mime != MIME {
            return Ok(None);
        }
        let py = slf.py();
        if let Some(bytes) = &slf.borrow().cached {
            return Ok(Some(bytes.clone_ref(py)));
        }
        let result = if let Some(clip) = slf.getattr_opt("clip")? {
            adapter(py)?.call_method1("encode", (clip,))
        } else {
            py.import("app_shell.native")?
                .call_method1("pasteboard_read", (PLATFORM_MIME,))
        };
        let bytes = match result {
            Ok(v) if !v.is_none() => v.cast_into::<PyBytes>()?,
            Ok(_) => PyBytes::new(py, &[]),
            Err(e) if e.is_instance_of::<pyo3::exceptions::PyException>(py) => {
                PyBytes::new(py, &[])
            }
            Err(e) => return Err(e),
        };
        let buf = Py::new(
            py,
            ClipboardBytes {
                value: bytes.unbind(),
            },
        )?;
        slf.borrow_mut().cached = Some(buf.clone_ref(py));
        Ok(Some(buf))
    }
    fn data(slf: &Bound<'_, Self>, mime: &str) -> PyResult<Option<Py<ClipboardBytes>>> {
        Self::retrieve(slf, mime, None)
    }
}
#[pyfunction]
fn clip_current_offer(py: Python<'_>) -> PyResult<Obj<'_>> {
    let api = adapter(py)?;
    let native = py.import("app_shell.native")?;
    if native.call_method0("available")?.is_truthy()? {
        let (kinds, change): (Vec<String>, i64) =
            native.call_method0("pasteboard_types")?.extract()?;
        if !kinds.iter().any(|m| m == PLATFORM_MIME) {
            api.setattr("_offer", py.None())?;
            return Ok(py.None().into_bound(py));
        }
        if !api.getattr("_change")?.eq(change)? {
            api.setattr("_offer", Py::new(py, DataOffer::default())?)?;
            api.setattr("_change", change)?;
        }
    }
    api.getattr("_offer")
}
#[pyfunction]
fn clip_available(py: Python<'_>) -> PyResult<bool> {
    let md = adapter(py)?.call_method0("current_offer")?;
    Ok(!md.is_none() && md.call_method1("hasFormat", (MIME,))?.is_truthy()?)
}
#[pyfunction]
fn clip_publish(py: Python<'_>, clip: Py<PyAny>) -> PyResult<()> {
    let api = adapter(py)?;
    let native = py.import("app_shell.native")?;
    let offer = Py::new(py, Offer::new(clip))?;
    if native.call_method0("available")?.is_truthy()? {
        let bytes = offer
            .bind(py)
            .call_method1("data", (MIME,))?
            .call_method0("data")?;
        if bytes.len()? == 0
            || !native
                .call_method1("pasteboard_write", (vec![(PLATFORM_MIME, bytes)],))?
                .is_truthy()?
        {
            return Ok(());
        }
        api.setattr(
            "_change",
            native.call_method0("pasteboard_types")?.get_item(1)?,
        )?;
    }
    api.setattr("_offer", offer)
}
#[pyfunction]
fn clip_flush(py: Python<'_>) -> PyResult<()> {
    let api = adapter(py)?;
    let offer = clip_current_offer(py)?;
    if offer.is_instance_of::<Offer>() {
        let cached = DataOffer::data(offer.cast::<DataOffer>()?, MIME)?;
        api.setattr("_offer", Py::new(py, DataOffer { cached })?)?;
    }
    Ok(())
}
#[pyfunction]
fn clip_foreign(py: Python<'_>) -> PyResult<Obj<'_>> {
    let api = adapter(py)?;
    let md = api.call_method0("current_offer")?;
    if md.is_none()
        || md.is_instance_of::<Offer>()
        || !md.call_method1("hasFormat", (MIME,))?.is_truthy()?
    {
        return Ok(py.None().into_bound(py));
    }
    api.call_method1(
        "decode",
        (md.call_method1("data", (MIME,))?.call_method0("data")?,),
    )
}
#[pyfunction(signature=(current,adopted_from=None))]
fn clip_own<'p>(
    py: Python<'p>,
    current: &Obj<'p>,
    adopted_from: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let api = adapter(py)?;
    let md = api.call_method0("current_offer")?;
    if let Some(clip) = md.getattr_opt("clip")?.filter(|v| !v.is_none()) {
        if !clip.is(current) && !adopted_from.is_some_and(|v| v.is(&clip)) {
            return api.call_method1(
                "decode",
                (md.call_method1("data", (MIME,))?.call_method0("data")?,),
            );
        }
    }
    Ok(py.None().into_bound(py))
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    macro_rules! add{($($f:ident),*)=>{$(m.add_function(wrap_pyfunction!($f,m)?)?;)*}}
    m.add_class::<Offer>()?;
    add!(
        clip_encode,
        clip_decode,
        clip_current_offer,
        clip_available,
        clip_publish,
        clip_flush,
        clip_foreign,
        clip_own
    );
    Ok(())
}
