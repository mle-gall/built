// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
struct Selected<'py> {
    faces: Vec<Obj<'py>>,
    edges: Vec<Obj<'py>>,
    groups: Vec<Obj<'py>>,
}
impl<'py> Selected<'py> {
    fn read(scene: &Obj<'py>) -> PyResult<Self> {
        let m = host(scene.py())?;
        let f = m.getattr("Face")?;
        let e = m.getattr("Edge")?;
        let g = m.getattr("Group")?;
        let mut s = Self {
            faces: vec![],
            edges: vec![],
            groups: vec![],
        };
        for item in scene.getattr("selection")?.try_iter()? {
            let item = item?;
            if item.is_instance(&f)? {
                s.faces.push(item.clone())
            }
            if item.is_instance(&e)? {
                s.edges.push(item.clone())
            }
            if item.is_instance(&g)? {
                s.groups.push(item)
            }
        }
        Ok(s)
    }
}
#[pyfunction]
fn copy_selection(v: &Obj, publish: bool) -> PyResult<bool> {
    let py = v.py();
    let scene = v.getattr("scene")?;
    let selected = Selected::read(&scene)?;
    if selected.faces.is_empty() && selected.edges.is_empty() && selected.groups.is_empty() {
        return Ok(false);
    }
    let m = host(py)?;
    let data = PyDict::new(py);
    let faces = PyList::empty(py);
    let edges = PyList::empty(py);
    let groups = PyList::empty(py);
    let attrs = PyList::empty(py);
    let mut lo = None;
    let mut copy_point = |p: &Obj| -> PyResult<Obj> {
        let w = xyz(p)?;
        lo = Some(lo.map_or(w, |cur: DVec3| cur.min(w)));
        vec3(py, w)
    };
    for f in selected.faces {
        let ring = PyList::empty(py);
        for p in f.getattr("vertices")?.try_iter()? {
            ring.append(copy_point(&p?)?)?
        }
        let holes = PyList::empty(py);
        for hole in f.getattr("holes")?.try_iter()? {
            let ring = PyList::empty(py);
            for p in hole?.try_iter()? {
                ring.append(copy_point(&p?)?)?
            }
            holes.append(ring)?
        }
        let a = f.getattr("attrs")?;
        let a = if a.is_truthy()? {
            py.import("copy")?.call_method1("deepcopy", (&a,))?
        } else {
            PyDict::new(py).into_any()
        };
        attrs.append(&a)?;
        faces.append((ring, holes, a))?;
    }
    let layers = py.import("core.layers")?;
    let edge_layers = PyList::empty(py);
    for e in selected.edges {
        edges.append((
            copy_point(&e.getattr("a")?)?,
            copy_point(&e.getattr("b")?)?,
            e.getattr("soft")?,
            e.getattr("curve")?,
        ))?;
        edge_layers.append(layers.call_method1("layer_of", (e,))?)?
    }
    let seed = opt(v, "_seed_proto_chunk")?;
    let old = opt(v, "clipboard")?;
    let copy = m.getattr("copy_group")?;
    let group_api = py.import("core.group")?;
    for src in selected.groups {
        let g = copy.call1((&src,))?;
        if g.getattr("xform")?.is_none() {
            g.setattr(
                "xform",
                crate::host_context::core_module(py)?.call_method0("Matrix4")?,
            )?;
            g.setattr("component", false)?;
            if seed.is_callable() {
                seed.call1((g.getattr("mesh")?, src))?;
            }
        }
        let pts = group_api.call_method1("placement_points", (&g,))?;
        if pts.len()? > 0 {
            let corner = DVec3::from_array(pts.call_method1("min", (0,))?.extract()?);
            lo = Some(lo.map_or(corner, |p| p.min(corner)))
        }
        groups.append(g)?
    }
    let Some(lo) = lo else { return Ok(false) };
    if old.is_truthy()? {
        let drop = opt(v, "_drop_clip_protos")?;
        if drop.is_callable() {
            drop.call1((old,))?;
        }
    }
    let names = py
        .import("core.insert")?
        .call_method1("used_layers", (attrs, &edge_layers, &groups))?;
    let registry = PyDict::new(py);
    let materials = opt(&scene, "materials")?;
    if !materials.is_none() {
        for kv in materials.call_method0("items")?.try_iter()? {
            let (k, mat): (Obj, Obj) = kv?.extract()?;
            if !mat.getattr("pbr")?.is_none() {
                registry.set_item(k, mat)?
            }
        }
    }
    data.set_item("faces", faces)?;
    data.set_item("edges", edges)?;
    data.set_item("registry", registry)?;
    data.set_item("edge_layers", edge_layers)?;
    data.set_item(
        "layers",
        crate::host_context::core_module(layers.py())?.getattr("tag_edits")?.call_method1("template_for", (&scene, names))?,
    )?;
    data.set_item("groups", groups)?;
    data.set_item("ref", vec3(py, lo)?)?;
    v.setattr("clipboard", &data)?;
    if publish {
        py.import("app_shell.files")?
            .call_method1("publish_clipboard", (v.call_method0("window")?, data))?;
    }
    Ok(true)
}
#[pyfunction]
fn cut_selection(v: &Obj, publish: bool) -> PyResult<bool> {
    let py = v.py();
    let copied = if publish {
        v.call_method0("copy_selection")?
    } else {
        let kw = PyDict::new(py);
        kw.set_item("publish", false)?;
        v.call_method("copy_selection", (), Some(&kw))?
    };
    if !copied.is_truthy()? {
        return Ok(false);
    }
    let scene = v.getattr("scene")?;
    let s = Selected::read(&scene)?;
    let history = py.import("core.history")?;
    let commands = PyList::empty(py);
    if !s.edges.is_empty() || !s.faces.is_empty() {
        commands.append(host(py)?.call_method1("EraseSelectionCommand", (s.edges, s.faces))?)?
    }
    for g in s.groups {
        commands.append(history.call_method1("DeleteGroupCommand", (g,))?)?
    }
    if !commands.is_empty() {
        let cmd = if commands.len() == 1 {
            commands.get_item(0)?
        } else {
            history.call_method1("CompoundCommand", (commands,))?
        };
        v.getattr("history")?.call_method1("execute", (cmd,))?;
    }
    v.call_method0("update")?;
    Ok(true)
}
#[pyfunction]
fn seed_proto_chunk(v: &Obj, mesh: &Obj, src: &Obj) -> PyResult<()> {
    let py = v.py();
    let chunks = opt(v, "_group_chunks")?;
    if !chunks.is_truthy()? || !opt(src, "xform")?.is_none() {
        return Ok(());
    }
    let base = chunks.call_method1("get", (id(src),))?;
    if base.is_none() {
        return Ok(());
    }
    let mut wrappers = opt(v, "_proto_wrappers")?;
    if wrappers.is_none() {
        wrappers = PyDict::new(py).into_any();
        v.setattr("_proto_wrappers", &wrappers)?
    }
    let mut w = wrappers.call_method1("get", (id(mesh),))?;
    if w.is_none() {
        let kw = PyDict::new(py);
        kw.set_item("mesh", mesh)?;
        kw.set_item("xform", py.None())?;
        w = py
            .import("types")?
            .call_method("SimpleNamespace", (), Some(&kw))?;
        wrappers.set_item(id(mesh), &w)?
    }
    chunks.set_item(
        id(&w),
        py.import("builtins")?.call_method1("dict", (base,))?,
    )
}
#[pyfunction]
fn drop_clip_protos(v: &Obj, clip: &Obj) -> PyResult<()> {
    let py = v.py();
    let wrappers = opt(v, "_proto_wrappers")?;
    if !wrappers.is_truthy()? {
        return Ok(());
    }
    let cache = opt(v, "_group_chunks")?;
    let all = py
        .import("core.purge")?
        .call_method1("iter_groups", (v.getattr("scene")?.getattr("groups")?,))?;
    let mut live = std::collections::HashSet::new();
    for g in all.try_iter()? {
        live.insert(id(&g?.getattr("mesh")?));
    }
    for g in clip
        .call_method1("get", ("groups", PyTuple::empty(py)))?
        .try_iter()?
    {
        let key = id(&g?.getattr("mesh")?);
        if !live.contains(&key) {
            let w = wrappers.call_method1("pop", (key, py.None()))?;
            if !w.is_none() && cache.is_truthy()? {
                cache.call_method1("pop", (id(&w), py.None()))?;
            }
        }
    }
    Ok(())
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(copy_selection, m)?)?;
    m.add_function(wrap_pyfunction!(cut_selection, m)?)?;
    m.add_function(wrap_pyfunction!(seed_proto_chunk, m)?)?;
    m.add_function(wrap_pyfunction!(drop_clip_protos, m)?)?;
    Ok(())
}
