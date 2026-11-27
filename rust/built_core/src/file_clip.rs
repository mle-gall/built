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
fn scene_file<'p, T>(
    py: Python<'p>,
    member: &str,
    work: impl FnOnce(&Obj<'p>) -> PyResult<T>,
) -> PyResult<T> {
    let kw = PyDict::new(py);
    kw.set_item("prefix", "built-clip-")?;
    let temp = py
        .import("tempfile")?
        .call_method("TemporaryDirectory", (), Some(&kw))?;
    let path = py
        .import("pathlib")?
        .call_method1("Path", (temp.getattr("name")?, member))?;
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
    let scene = py.import("built_core.scene")?.call_method0("Scene")?;
    let mesh = scene.getattr("mesh")?;
    let face_planes = val(clip, "face_planes")?;
    for (i, row) in seq(clip, "faces")?.try_iter()?.enumerate() {
        let row = row?;
        let holes = row.get_item(1)?;
        let f = match mesh.call_method1(
            "add_face",
            (row.get_item(0)?, holes.is_truthy()?.then_some(holes)),
        ) {
            Err(e) if e.is_instance_of::<PyValueError>(py) => continue,
            other => other?,
        };
        if let Ok(planes) = face_planes.cast::<PyDict>() {
            if let Some(frame) = planes.get_item(i.to_string())? {
                mesh.getattr("_store")?
                    .call_method1("bind_exact_plane", (f.getattr("key")?, frame))?;
            }
        }
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
        let placements = py.import("built_core.group")?.getattr("iter_placements")?;
        let face_attrs = py.import("built_core.mesh")?.getattr("face_attrs")?;
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
    crate::model::module(py, "built_core")?
        .getattr("tag_edits")?
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
    let mut exact = face_planes.is_truthy()?;
    if !exact {
        let placements = py.import("built_core.group")?.getattr("iter_placements")?;
        for root in groups.try_iter()? {
            for pair in placements.call1((root?,))?.try_iter()? {
                let m = pair?.get_item(0)?.getattr("mesh")?;
                let md = m.getattr("__dict__")?;
                if let Ok(st) = md.get_item("_store") {
                    if st
                        .getattr_opt("has_exact_bindings")?
                        .is_some_and(|p| p.is_truthy().unwrap_or(false))
                    {
                        exact = true;
                        break;
                    }
                }
            }
            if exact {
                break;
            }
        }
    }
    let member = if exact { "scene.built" } else { "scene.igz" };
    if exact {
        header.set_item("format", 2)?;
    }
    let doc = scene_file(py, member, |p| {
        py.import("built_core.formats.igz")?
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
        zip.call_method1("writestr", (member, doc))?;
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
            crate::archive_limits::check_archive(&zip, "the clipboard")?;
            let header = py
                .import("json")?
                .call_method1("loads", (zip.call_method1("read", ("clip.json",))?,))?;
            let member = if val(&header, "format")?.eq(2)? {
                "scene.built"
            } else {
                "scene.igz"
            };
            Ok((header, zip.call_method1("read", (member,))?))
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
    let exact = val(&head, "format")?.eq(2)?;
    if !exact && !val(&head, "format")?.eq(1)? {
        return Ok(None);
    }
    let scene = py.import("built_core.scene")?.call_method0("Scene")?;
    scene_file(
        py,
        if exact { "scene.built" } else { "scene.igz" },
        |path| {
            path.call_method1("write_bytes", (doc,))?;
            py.import("built_core.formats.igz")?
                .call_method1("load_into", (&scene, path))
        },
    )?;
    let faces = PyList::empty(py);
    let planes = PyDict::new(py);
    let mesh = scene.getattr("mesh")?;
    for f in mesh.getattr("faces")?.try_iter()? {
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
        if exact {
            let frame = mesh
                .getattr("_store")?
                .call_method1("exact_plane", (f.getattr("key")?,))?;
            if !frame.is_none() {
                planes.set_item(faces.len().to_string(), frame)?;
            }
        }
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
    if !planes.is_empty() {
        out.set_item("face_planes", planes)?;
    }
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
        crate::model::module(py, "built_core")?
            .getattr("tag_edits")?
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
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    macro_rules! add{($($f:ident),*)=>{$(m.add_function(wrap_pyfunction!($f,m)?)?;)*}}
    add!(clip_encode, clip_decode);
    Ok(())
}
