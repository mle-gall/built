// SPDX-License-Identifier: GPL-3.0-or-later
//! Resolve selection coordinates and assemble a single undoable transform.
use super::placements::resolve_placements;
use super::*;
use crate::math::Vec3;
use glam::DVec3;
use pyo3::types::{PyFloat, PyInt};
fn floats(o: &Obj<'_>) -> PyResult<Vec<f64>> {
    let f = o.py().get_type::<PyFloat>();
    o.try_iter()?.map(|v| f.call1((v?,))?.extract()).collect()
}
pub(super) fn finite(o: &Obj<'_>, error: &str) -> PyResult<DVec3> {
    let a = floats(o)?;
    let [x, y, z] = a[..] else {
        return Err(PyValueError::new_err(error.to_owned()));
    };
    let v = DVec3::new(x, y, z);
    if !v.is_finite() {
        return Err(PyValueError::new_err(error.to_owned()));
    }
    Ok(v)
}
pub(super) fn vector(py: Python<'_>, v: DVec3) -> PyResult<Obj<'_>> {
    Ok(Bound::new(py, Vec3::from(v))?.into_any())
}
fn local_center<'py>(mesh: &Obj<'py>, v: DVec3) -> PyResult<Obj<'py>> {
    let store = mesh
        .getattr("__dict__")?
        .cast::<PyDict>()?
        .get_item("_store")?;
    let origin = match store.filter(|s| !s.is_none()) {
        Some(s) => s.getattr("frame_origin")?.extract::<[f64; 3]>()?,
        None => [0.; 3],
    };
    vector(mesh.py(), v - DVec3::from_array(origin))
}
pub(super) fn empty<'py>(py: Python<'py>, o: Option<&Obj<'py>>) -> Obj<'py> {
    o.cloned().unwrap_or_else(|| PyTuple::empty(py).into_any())
}
fn distinct<'py>(seq: &Obj<'py>) -> PyResult<Obj<'py>> {
    seq.py()
        .get_type::<PyDict>()
        .call_method1("fromkeys", (seq,))
}
pub(super) fn records<'py>(
    items: &Obj<'py>,
    indices: &Obj<'py>,
    error: &str,
) -> PyResult<Bound<'py, PyList>> {
    let out = PyList::empty(items.py());
    let mut seen = std::collections::HashSet::new();
    let count = items.len()?;
    for idx in indices.try_iter()? {
        let idx = idx?;
        let i = idx
            .extract::<usize>()
            .ok()
            .filter(|_| idx.is_exact_instance_of::<PyInt>())
            .filter(|i| *i < count)
            .ok_or_else(|| PyKeyError::new_err(error.to_owned()))?;
        if seen.insert(i) {
            out.append(items.get_item(i)?)?;
        }
    }
    Ok(out)
}
pub(super) fn apply<'py>(h: &Obj<'py>, parts: &Bound<'py, PyList>) -> Reply<'py> {
    let cmd = match parts.len() {
        0 => h.py().None().into_bound(h.py()),
        1 => parts.get_item(0)?,
        _ => history_command(h.py(), "CompoundCommand", (parts,))?,
    };
    execute(h, &cmd)
}
#[pyfunction]
fn move_placements<'py>(history: &Obj<'py>, placements: &Obj<'py>, delta: &Obj<'py>) -> Reply<'py> {
    let py = history.py();
    let groups = resolve_placements(history, placements)?;
    let d = py
        .import("ingetrazo_core")?
        .getattr("Vec3")?
        .call(PyTuple::new(py, floats(delta)?)?, None)?;
    let parts = PyList::empty(py);
    for g in groups.iter() {
        parts.append(history_command(py, "MoveGroupCommand", (g, &d))?)?;
    }
    apply(history, &parts)
}
fn copy_count(value: &Obj<'_>) -> PyResult<i64> {
    value.py().get_type::<PyInt>().call1((value,))?.extract()
}

fn copy_offsets<'py>(delta: &Obj<'py>, count: i64, mode: &str) -> PyResult<Bound<'py, PyList>> {
    if count < 1 || !matches!(mode, "x" | "/") {
        return Err(PyValueError::new_err(
            "count is at least 1 and mode is 'x' or '/'",
        ));
    }
    let offsets = PyList::empty(delta.py());
    for index in 1..=count {
        let scale = if mode == "/" {
            index as f64 / count as f64
        } else {
            index as f64
        };
        offsets.append(delta.mul(scale)?)?;
    }
    Ok(offsets)
}

#[pyfunction]
#[pyo3(signature=(history,placements,delta,count=1,mode="x"))]
fn copy_placements<'py>(
    history: &Obj<'py>,
    placements: &Obj<'py>,
    delta: &Obj<'py>,
    #[pyo3(from_py_with=copy_count)] count: i64,
    mode: &str,
) -> Reply<'py> {
    let py = history.py();
    let groups = resolve_placements(history, placements)?;
    let d = py
        .import("ingetrazo_core")?
        .getattr("Vec3")?
        .call(PyTuple::new(py, floats(delta)?)?, None)?;
    let copier = py.import("core.group")?.getattr("copy_group")?;
    let parts = PyList::empty(py);
    let clones = PyList::empty(py);
    for offset in copy_offsets(&d, count, mode)?.iter() {
        for g in groups.iter() {
            let c = copier.call1((g, &offset))?;
            parts.append(history_command(py, "InsertGroupCommand", (&c,))?)?;
            clones.append(c)?;
        }
    }
    let reply = apply(history, &parts)?;
    let ids = PyList::empty(py);
    if reply.as_any().get_item("ok")?.is_truthy()? {
        for g in clones.iter() {
            ids.append(g.getattr("uid")?)?;
        }
    }
    reply.set_item("placements", ids)?;
    Ok(reply)
}
#[pyfunction]
fn set_group_scale<'py>(
    history: &Obj<'py>,
    placement: &Obj<'py>,
    factors: &Obj<'py>,
) -> Reply<'py> {
    let py = history.py();
    let v = finite(factors, "expected three finite scale factors")?;
    let g = resolve_placements(history, PyList::new(py, [placement])?.as_any())?.get_item(0)?;
    let change = crate::group_scale::group_scale_command(&g, v.to_array())?;
    let command = change.unwrap_or_else(|| py.None().into_bound(py));
    execute(history, &command)
}
#[pyfunction]
#[pyo3(signature=(history,anchor,factors,vertices=None,placements=None,images=None,axes=None))]
fn scale_selection<'py>(
    history: &Obj<'py>,
    anchor: &Obj<'py>,
    factors: &Obj<'py>,
    vertices: Option<&Obj<'py>>,
    placements: Option<&Obj<'py>>,
    images: Option<&Obj<'py>>,
    axes: Option<&Obj<'py>>,
) -> Reply<'py> {
    let py = history.py();
    let f = finite(factors, "three finite, nonzero scale factors required")?;
    if f.to_array().contains(&0.) {
        return Err(PyValueError::new_err(
            "three finite, nonzero scale factors required",
        ));
    }
    let a = finite(anchor, "three finite anchor coordinates required")?;
    let axes = if let Some(axes) = axes {
        let mut v = Vec::new();
        for ax in axes.try_iter()? {
            v.push(vector(py, finite(&ax?, "three finite box axes required")?)?);
        }
        if v.len() != 3 {
            return Err(PyValueError::new_err("three finite box axes required"));
        }
        Some(PyTuple::new(py, v)?)
    } else {
        None
    };
    let scene = history.getattr("scene")?;
    let mesh = scene.getattr("mesh")?;
    let c = local_center(&mesh, a)?;
    let groups = resolve_placements(history, &empty(py, placements))?;
    let vs = need(&mesh, "vertex", &distinct(&empty(py, vertices))?)?;
    let refs = records(
        &scene.getattr("image_planes")?,
        &empty(py, images),
        "unknown image plane",
    )?;
    let packed = if f.x == f.y && f.y == f.z {
        f.x.into_pyobject(py)?.into_any()
    } else {
        PyTuple::new(py, f.to_array())?.into_any()
    };
    let parts = PyList::empty(py);
    for g in groups.iter() {
        parts.append(history_command(
            py,
            "ScaleGroupCommand",
            (g, &c, &packed, &axes),
        )?)?;
    }
    if !vs.is_empty() {
        let pos = PyList::new(
            py,
            vs.iter()
                .map(|v| v.getattr("position"))
                .collect::<PyResult<Vec<_>>>()?,
        )?;
        parts.append(history_command(
            py,
            "ScaleVerticesCommand",
            (pos, &c, &packed, &axes),
        )?)?;
    }
    if !refs.is_empty() {
        parts.append(history_command(
            py,
            "ScaleImagePlanesCommand",
            (refs, &c, &packed, axes),
        )?)?;
    }
    apply(history, &parts)
}
#[pyfunction]
#[pyo3(signature=(history,center,axis,angles,copy=false,placements=None,vertices=None,edges=None,faces=None,images=None,guides=None,sections=None))]
fn rotate_selection<'py>(
    history: &Obj<'py>,
    center: &Obj<'py>,
    axis: &Obj<'py>,
    angles: &Obj<'py>,
    #[pyo3(from_py_with=PyAnyMethods::is_truthy)] copy: bool,
    placements: Option<&Obj<'py>>,
    vertices: Option<&Obj<'py>>,
    edges: Option<&Obj<'py>>,
    faces: Option<&Obj<'py>>,
    images: Option<&Obj<'py>>,
    guides: Option<&Obj<'py>>,
    sections: Option<&Obj<'py>>,
) -> Reply<'py> {
    let py = history.py();
    let error = "finite centre, axis and angles required";
    let c = finite(center, error)?;
    let a = finite(axis, error)?;
    let degrees = floats(angles)?;
    if !degrees.iter().all(|d| d.is_finite()) {
        return Err(PyValueError::new_err(error));
    }
    if a.length() < 1e-12 || degrees.is_empty() || (!copy && degrees.len() != 1) {
        return Err(PyValueError::new_err(
            "a nonzero axis and one angle (or one per copy) required",
        ));
    }
    let scene = history.getattr("scene")?;
    let mesh = scene.getattr("mesh")?;
    let gs = resolve_placements(history, &empty(py, placements))?;
    let vs = need(&mesh, "vertex", &empty(py, vertices))?;
    let pos = PyList::new(
        py,
        vs.iter()
            .map(|v| v.getattr("position"))
            .collect::<PyResult<Vec<_>>>()?,
    )?;
    let rec = |field: &str, index| {
        records(
            &scene.getattr(field)?,
            &empty(py, index),
            "unknown record index",
        )
    };
    let cmd = crate::tools::polar_history::rotation_command(
        py,
        &local_center(&mesh, c)?,
        &vector(py, a)?,
        copy,
        gs.as_any(),
        pos.as_any(),
        need(&mesh, "face", &empty(py, faces))?.as_any(),
        need(&mesh, "edge", &empty(py, edges))?.as_any(),
        rec("guides", guides)?.as_any(),
        rec("section_planes", sections)?.as_any(),
        rec("image_planes", images)?.as_any(),
        angles,
    )?
    .unwrap_or_else(|| py.None().into_bound(py));
    execute(history, &cmd)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(move_selection, m)?)?;
    m.add_function(wrap_pyfunction!(flip_selection, m)?)?;
    m.add_function(wrap_pyfunction!(move_placements, m)?)?;
    m.add_function(wrap_pyfunction!(copy_placements, m)?)?;
    m.add_function(wrap_pyfunction!(rotate_selection, m)?)?;
    m.add_function(wrap_pyfunction!(scale_selection, m)?)?;
    m.add_function(wrap_pyfunction!(set_group_scale, m)?)?;
    Ok(())
}
#[pyfunction]
#[pyo3(signature=(history,delta,placements=None,vertices=None,edges=None,faces=None,copy=false,count=None,mode="x",_plan=None))]
fn move_selection<'py>(
    history: &Obj<'py>,
    delta: &Obj<'py>,
    placements: Option<&Obj<'py>>,
    vertices: Option<&Obj<'py>>,
    edges: Option<&Obj<'py>>,
    faces: Option<&Obj<'py>>,
    #[pyo3(from_py_with=PyAnyMethods::is_truthy)] copy: bool,
    count: Option<&Obj<'py>>,
    mode: &str,
    _plan: Option<&Obj<'py>>,
) -> Reply<'py> {
    let py = history.py();
    if let Some(plan) = _plan {
        if !plan.is_instance(&py.import("core.history")?.getattr("Command")?)? {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "a prepared move must be a history command",
            ));
        }
        return execute(history, plan);
    }
    let scene = history.getattr("scene")?;
    let mesh = scene.getattr("mesh")?;
    let groups;
    let vs;
    let es;
    let fs;
    let guides = PyList::empty(py);
    let labels = PyList::empty(py);
    let dims = PyList::empty(py);
    let planes = PyList::empty(py);
    let images = PyList::empty(py);
    if [placements, vertices, edges, faces]
        .iter()
        .any(|x| x.is_some())
    {
        groups = resolve_placements(history, &empty(py, placements))?;
        vs = need(&mesh, "vertex", &empty(py, vertices))?;
        es = need(&mesh, "edge", &empty(py, edges))?;
        fs = need(&mesh, "face", &empty(py, faces))?;
    } else {
        groups = PyList::empty(py);
        vs = PyList::empty(py);
        es = PyList::empty(py);
        fs = PyList::empty(py);
        let classes = [
            ("core.group", "Group", &groups),
            ("core.mesh", "Edge", &es),
            ("core.mesh", "Face", &fs),
            ("core.dimension", "Dimension", &dims),
            ("core.section", "SectionPlane", &planes),
            ("core.textlabel", "TextLabel", &labels),
        ];
        let classes = classes
            .into_iter()
            .map(|(m, c, l)| Ok((py.import(m)?.getattr(c)?, l)))
            .collect::<PyResult<Vec<_>>>()?;
        let is_guide = py.import("core.guide")?.getattr("is_guide")?;
        for e in scene.getattr("selection")?.try_iter()? {
            let e = e?;
            for (class, list) in &classes {
                if e.is_instance(class)? {
                    list.append(&e)?;
                }
            }
            if is_guide.call1((&e,))?.is_truthy()? {
                guides.append(&e)?;
            }
            if e.get_type().name()? == "ImagePlane" {
                images.append(e)?;
            }
        }
    }
    let ctor = py.import("ingetrazo_core")?.getattr("Vec3")?;
    let d = ctor.call(PyTuple::new(py, floats(delta)?)?, None)?;
    let ops = py.import("core.move_ops")?;
    if copy {
        let n = match count {
            Some(c) => py.get_type::<PyInt>().call1((c,))?.extract()?,
            None => 1i64,
        };
        let steps = copy_offsets(&d, n, mode)?;
        let cmd = ops
            .call_method1("copy_builder", (&scene, groups, fs, es, guides))?
            .call1((steps,))?;
        return execute(history, &cmd);
    }
    let mut pos = Vec::new();
    for v in vs.iter() {
        pos.push(v.getattr("position")?);
    }
    for e in es.iter() {
        pos.extend([e.getattr("a")?, e.getattr("b")?]);
    }
    for f in fs.iter() {
        pos.extend(
            f.getattr("vertices")?
                .try_iter()?
                .collect::<PyResult<Vec<_>>>()?,
        );
    }
    let key = py.import("core.topology")?.getattr("_key")?;
    let seen = pyo3::types::PySet::empty(py)?;
    let positions = PyList::empty(py);
    let attached = PyList::empty(py);
    for p in pos {
        let k = key.call1((&p,))?;
        if seen.contains(&k)? {
            continue;
        }
        seen.add(k)?;
        let p = ctor.call1((p,))?;
        let v = mesh.call_method1("vertex_at", (&p,))?;
        if !v.is_none() {
            attached.append(v)?;
        }
        positions.append(p)?;
    }
    let dims = ops.call_method1("dimension_modes", (dims, &groups, attached))?;
    let kw = PyDict::new(py);
    kw.set_item("groups", groups)?;
    kw.set_item("positions", positions)?;
    kw.set_item("labels", labels)?;
    kw.set_item("guides", guides)?;
    kw.set_item("dims", dims)?;
    kw.set_item("splanes", planes)?;
    kw.set_item("images", images)?;
    let parts = ops
        .call_method("move_commands", (d,), Some(&kw))?
        .cast_into::<PyList>()?;
    apply(history, &parts)
}
#[pyfunction]
#[pyo3(signature=(history,center,axis,copy=false,placements=None,faces=None,edges=None))]
fn flip_selection<'py>(
    history: &Obj<'py>,
    center: &Obj<'py>,
    axis: &Obj<'py>,
    #[pyo3(from_py_with=PyAnyMethods::is_truthy)] copy: bool,
    placements: Option<&Obj<'py>>,
    faces: Option<&Obj<'py>>,
    edges: Option<&Obj<'py>>,
) -> Reply<'py> {
    let py = history.py();
    let err = "flip needs a finite centre and plane normal";
    let c = finite(center, err)?;
    let axis = finite(axis, err)?;
    if axis.length() < 1e-12 {
        return Err(PyValueError::new_err("flip needs a nonzero plane normal"));
    }
    let scene = history.getattr("scene")?;
    let mesh = scene.getattr("mesh")?;
    let n = vector(py, axis.normalize())?;
    let c = local_center(&mesh, c)?;
    let groups = resolve_placements(history, &empty(py, placements))?;
    let faces = need(&mesh, "face", &empty(py, faces))?;
    let edges = need(&mesh, "edge", &empty(py, edges))?;
    let cmds = PyList::empty(py);
    if copy {
        let group = py.import("core.group")?;
        let attrs = py
            .import("core.ifc_identity")?
            .call_method1("copied_face_attrs", (&scene,))?;
        let m = history_command(py, "mirror_matrix", (&c, &n))?;
        let ring = |points: Obj<'py>| -> PyResult<Bound<'py, PyList>> {
            let r = PyList::empty(py);
            for p in points.try_iter()? {
                r.append(m.call_method1("map", (p?,))?)?;
            }
            r.reverse()?;
            Ok(r)
        };
        for g in groups.iter() {
            let clone = group.call_method1("copy_group", (g,))?;
            cmds.append(history_command(py, "InsertGroupCommand", (&clone,))?)?;
            cmds.append(history_command(
                py,
                "FlipGroupsCommand",
                (PyList::new(py, [clone])?, &c, &n),
            )?)?;
        }
        for face in faces.iter() {
            let outer = ring(face.getattr("vertices")?)?;
            let holes = PyList::empty(py);
            for h in face.getattr("holes")?.try_iter()? {
                holes.append(ring(h?)?)?;
            }
            let kw = PyDict::new(py);
            kw.set_item("holes", if holes.is_empty() { None } else { Some(holes) })?;
            kw.set_item("auto", false)?;
            kw.set_item(
                "attrs",
                attrs.call1((
                    group.call_method1("transformed_attrs", (face.getattr("attrs")?, &m))?,
                ))?,
            )?;
            cmds.append(py.import("core.history")?.call_method(
                "AddFaceCommand",
                (outer,),
                Some(&kw),
            )?)?;
        }
        let curves = PyDict::new(py);
        let mesh_type = py.import("core.mesh")?.getattr("Mesh")?;
        for e in edges.iter() {
            let curve = e.getattr("curve")?;
            if !curve.is_none() && !curves.contains(&curve)? {
                curves.set_item(&curve, mesh_type.call_method0("next_curve_id")?)?;
            }
            let kw = PyDict::new(py);
            kw.set_item("curve", curves.get_item(curve)?)?;
            let soft = e.getattr("soft")?;
            kw.set_item("soft", if soft.is_truthy()? { Some(soft) } else { None })?;
            cmds.append(py.import("core.history")?.call_method(
                "AddEdgeCommand",
                (
                    m.call_method1("map", (e.getattr("a")?,))?,
                    m.call_method1("map", (e.getattr("b")?,))?,
                ),
                Some(&kw),
            )?)?;
        }
    } else {
        if !groups.is_empty() {
            cmds.append(history_command(py, "FlipGroupsCommand", (groups, &c, &n))?)?;
        }
        let positions = PyDict::new(py);
        let add = |v: Obj<'py>| positions.set_item(v.getattr("key")?, v.getattr("position")?);
        for f in faces.iter() {
            for v in f.getattr("loop")?.try_iter()? {
                add(v?)?;
            }
        }
        for e in edges.iter() {
            add(e.getattr("v0")?)?;
            add(e.getattr("v1")?)?;
        }
        if !positions.is_empty() {
            let kw = PyDict::new(py);
            kw.set_item("faces", faces)?;
            cmds.append(py.import("core.history")?.call_method(
                "FlipVerticesCommand",
                (positions.values(), c, n),
                Some(&kw),
            )?)?;
        }
    }
    apply(history, &cmds)
}
