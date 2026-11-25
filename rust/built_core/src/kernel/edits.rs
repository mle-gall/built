// SPDX-License-Identifier: GPL-3.0-or-later
//! Turn an ordered stroke into history commands. The simulated graph is private
//! to the plan. Store and materialised edits keep their original element handles.
use crate::{
    draw_plan,
    math::{xyz, Vec3},
    object_planes,
};
use pyo3::{
    exceptions::PyAttributeError,
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
type Obj<'p> = Bound<'p, PyAny>;
type List<'p> = Bound<'p, PyList>;
type Plane<'p> = (Obj<'p>, Obj<'p>);

fn scan<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    scene
        .py()
        .import("built_core.scan_index")?
        .call_method1("scan", (scene.getattr("mesh")?,))
}
fn objects<'p>(py: Python<'p>) -> PyResult<Obj<'p>> {
    py.import("built_core")?.getattr("object_drawing")
}
fn list_copy<'p>(items: &Obj<'p>) -> PyResult<List<'p>> {
    Ok(items
        .py()
        .get_type::<PyList>()
        .call1((items,))?
        .cast_into()?)
}
#[pyclass(name = "_PlanEdges", module = "built_core.edits")]
struct PlanEdges {
    rows: Py<PyList>,
    #[pyo3(get, set)]
    key: Option<Py<PyAny>>,
    graph: Option<Py<PyAny>>,
}
#[pymethods]
impl PlanEdges {
    #[new]
    fn new(edges: &Obj<'_>) -> PyResult<Self> {
        Ok(Self {
            rows: list_copy(edges)?.unbind(),
            key: None,
            graph: None,
        })
    }
    fn __iter__<'p>(&self, py: Python<'p>) -> PyResult<Obj<'p>> {
        self.rows.bind(py).call_method0("__iter__")
    }
    fn __len__(&self, py: Python<'_>) -> usize {
        self.rows.bind(py).len()
    }
    fn __getitem__<'p>(&self, i: Obj<'p>) -> PyResult<Obj<'p>> {
        self.rows.bind(i.py()).as_any().get_item(i)
    }
    fn __contains__(&self, e: &Obj<'_>) -> PyResult<bool> {
        self.rows.bind(e.py()).contains(e)
    }
    fn graph<'p>(&mut self, py: Python<'p>) -> PyResult<Obj<'p>> {
        if self.graph.is_none() {
            let top = py.import("built_core.topology")?;
            let key = match &self.key {
                Some(k) => k.bind(py).clone(),
                None => top.getattr("_key")?,
            };
            self.graph = Some(
                top.call_method1("CycleGraph", (self.rows.bind(py), key))?
                    .unbind(),
            );
        }
        Ok(self.graph.as_ref().unwrap().bind(py).clone())
    }
    fn append(&self, e: &Obj<'_>) -> PyResult<()> {
        self.rows.bind(e.py()).append(e)?;
        if let Some(g) = &self.graph {
            g.bind(e.py()).call_method1("add", (e,))?;
        }
        Ok(())
    }
    fn remove(&mut self, e: &Obj<'_>) -> PyResult<()> {
        self.rows.bind(e.py()).call_method1("remove", (e,))?;
        if let Some(g) = &self.graph {
            if !g.bind(e.py()).call_method1("remove", (e,))?.is_truthy()? {
                self.graph = None;
            }
        }
        Ok(())
    }
}
#[pyfunction(name = "_ScanEdges")]
fn scan_edges<'p>(scan: &Obj<'p>) -> PyResult<Obj<'p>> {
    objects(scan.py())?.call_method1("ScanOverlay", (scan, true))
}
#[pyfunction(name = "_ScanFaces")]
fn scan_faces<'p>(scan: &Obj<'p>) -> PyResult<Obj<'p>> {
    objects(scan.py())?.call_method1("ScanOverlay", (scan, false))
}

#[pyfunction(signature=(scene,segments,detect_faces=true,eye=None,*,close_cycles=true,soft=None,tag=None))]
fn plan_edge_commands<'p>(
    scene: &Obj<'p>,
    segments: &Obj<'p>,
    detect_faces: bool,
    eye: Option<&Obj<'p>>,
    close_cycles: bool,
    soft: Option<&Obj<'p>>,
    tag: Option<&Obj<'p>>,
) -> PyResult<List<'p>> {
    let py = scene.py();
    let sc = scan(scene)?;
    let scanned = !sc.is_none();
    let none = py.None().into_bound(py);
    let (edges, faces, near) = if scanned {
        (scan_edges(&sc)?, scan_faces(&sc)?, none.clone())
    } else {
        let rows = Bound::new(py, PlanEdges::new(&scene.getattr("edges")?)?)?.into_any();
        let faces = list_copy(&scene.getattr("faces")?)?.into_any();
        let near = objects(py)?.call_method1("Nearby", (&rows, &faces))?;
        (rows, faces, near)
    };
    let memo = py.import("built_core.topology")?.call_method0("KeyMemo")?;
    edges.setattr("key", &memo)?;
    if !near.is_none() {
        near.setattr("key", &memo)?;
    }
    draw_plan::plan_segments(
        edges,
        faces,
        near,
        segments,
        scanned,
        detect_faces,
        close_cycles,
        eye.unwrap_or(&none).clone(),
        soft.unwrap_or(&none).clone(),
        tag.unwrap_or(&none).clone(),
    )
}
#[pyfunction]
fn _scene_has_curves(scene: &Obj<'_>) -> PyResult<bool> {
    let sc = scan(scene)?;
    if sc.is_none() {
        object_planes::has_plane_edge(&scene.getattr("edges")?, None, true)
    } else {
        sc.getattr("native")?.call_method0("any_curve")?.is_truthy()
    }
}
fn scan_plane<'p>(
    mesh: &Obj<'p>,
    sc: &Obj<'p>,
    points: &Obj<'p>,
    scoped: bool,
) -> PyResult<Option<Plane<'p>>> {
    let py = mesh.py();
    let source = if sc.getattr("store_backed")?.is_truthy()? {
        mesh.getattr("_store")?
    } else {
        sc.getattr("native")?
    };
    let coords = points
        .try_iter()?
        .map(|p| Ok(xyz(&p?)?.to_array()))
        .collect::<PyResult<Vec<_>>>()?;
    let h = py.import("core._native")?.call_method0("qt_hypot")?;
    let result = if scoped {
        source.call_method1("rebuild_plane", (coords, 400, h))?
    } else {
        source.call_method1("drawing_plane", (coords, h))?
    };
    let Some([o, n]) = result.extract::<Option<[[f64; 3]; 2]>>()? else {
        return Ok(None);
    };
    let v = |p| Ok::<_, PyErr>(Bound::new(py, Vec3::from(glam::DVec3::from_array(p)))?.into_any());
    Ok(Some((v(o)?, v(n)?)))
}
#[pyfunction]
fn _whole_drawing_plane<'p>(scene: &Obj<'p>, points: &Obj<'p>) -> PyResult<Option<Plane<'p>>> {
    let sc = scan(scene)?;
    let mesh = scene.getattr("mesh")?;
    if !sc.is_none() {
        return scan_plane(&mesh, &sc, points, false);
    }
    let result = object_planes::coplanar_plane(points, 1e-4, Some(&mesh.getattr("vertices")?))?;
    result
        .map(|(o, n)| Ok((o, Bound::new(scene.py(), n)?.into_any())))
        .transpose()
}
fn has_step(cmds: &List<'_>, names: &[&str]) -> PyResult<bool> {
    let history = cmds.py().import("built_core.history")?;
    let types = names
        .iter()
        .map(|n| history.getattr(*n))
        .collect::<PyResult<Vec<_>>>()?;
    let types = PyTuple::new(cmds.py(), types)?;
    for c in cmds {
        if c.is_instance(&types)? {
            return Ok(true);
        }
    }
    Ok(false)
}
const REBUILDS: &[&str] = &["RebuildPlanarFacesCommand", "RebuildPlaneFacesCommand"];
fn scoped(cmds: &List<'_>, plane: Plane<'_>) -> PyResult<()> {
    cmds.append(
        cmds.py()
            .import("built_core.history")?
            .call_method1("RebuildPlaneFacesCommand", plane)?,
    )
}
#[pyfunction(signature=(scene,commands,points,*,close_cycles=true))]
fn _append_flat_curve_rebuild(
    scene: &Obj<'_>,
    commands: &List<'_>,
    points: &Obj<'_>,
    close_cycles: bool,
) -> PyResult<()> {
    if has_step(commands, REBUILDS)? {
        return Ok(());
    }
    let py = scene.py();
    let curves = _scene_has_curves(scene)?;
    let mesh = scene.getattr("mesh")?;
    if _whole_drawing_plane(scene, points)?.is_some() {
        if curves && close_cycles {
            commands.append(
                py.import("built_core.history")?
                    .call_method0("RebuildPlanarFacesCommand")?,
            )?;
        } else {
            let faces = mesh.getattr("faces")?;
            if (1..=400).contains(&faces.len()?) {
                let f = faces.get_item(0)?;
                scoped(
                    commands,
                    (
                        f.getattr("vertices")?.get_item(0)?,
                        f.call_method0("normal")?,
                    ),
                )?;
            }
        }
    } else if curves {
        let Some((o, n)) = object_planes::coplanar_plane(points, 1e-4, None)? else {
            return Ok(());
        };
        let n = Bound::new(py, n)?.into_any();
        let sc = scan(scene)?;
        let edges = if sc.is_none() {
            mesh.getattr("edges")?
        } else {
            let rows = sc.getattr("native")?.call_method1(
                "edges_on_plane",
                (
                    <(f64, f64, f64)>::from(xyz(&o)?),
                    <(f64, f64, f64)>::from(xyz(&n)?),
                    1e-4,
                    true,
                ),
            )?;
            sc.call_method1("edges", (rows,))?
        };
        if object_planes::has_plane_edge(&edges, Some((o.clone(), n.clone())), true)? {
            scoped(commands, (o, n))?;
        }
    }
    Ok(())
}
#[pyfunction]
fn _append_face_plane_rebuild(
    scene: &Obj<'_>,
    commands: &List<'_>,
    points: &Obj<'_>,
) -> PyResult<()> {
    if has_step(commands, REBUILDS)? {
        return Ok(());
    }
    let sc = scan(scene)?;
    let mesh = scene.getattr("mesh")?;
    let plane = if sc.is_none() {
        object_planes::face_rebuild_plane(&mesh.getattr("faces")?, points, 400)?
    } else {
        scan_plane(&mesh, &sc, points, true)?
    };
    if let Some(p) = plane {
        scoped(commands, p)?;
    }
    Ok(())
}
#[pyfunction]
fn _append_active_tag(scene: &Obj<'_>, commands: &List<'_>) -> PyResult<()> {
    let Some(tag) = scene.getattr_opt("active_ifc")? else {
        return Ok(());
    };
    if !tag.is_truthy()? {
        return Ok(());
    }
    let h = scene.py().import("built_core.history")?;
    let cls = h.getattr("AddFaceCommand")?;
    let faces = PyList::empty(scene.py());
    for c in commands {
        if c.is_instance(&cls)? && c.getattr("auto")?.is_truthy()? {
            faces.append(c)?;
        }
    }
    if !faces.is_empty() {
        commands.append(h.call_method1("AutoTagDrawnFacesCommand", (faces, tag))?)?;
    }
    Ok(())
}
#[pyfunction(signature=(scene,a,b,detect_faces=true,eye=None,*,soft=None,tag=None))]
fn build_add_edge<'p>(
    scene: &Obj<'p>,
    a: &Obj<'p>,
    b: &Obj<'p>,
    detect_faces: bool,
    eye: Option<&Obj<'p>>,
    soft: Option<&Obj<'p>>,
    tag: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let py = scene.py();
    let points = PyList::new(py, [a, b])?;
    let seg = PyList::new(py, [&points])?;
    let cmds = plan_edge_commands(scene, &seg, detect_faces, eye, true, soft, tag)?;
    _append_active_tag(scene, &cmds)?;
    _append_flat_curve_rebuild(scene, &cmds, &points, true)?;
    if detect_faces {
        _append_face_plane_rebuild(scene, &cmds, &points)?;
    }
    if cmds.len() == 1 && soft.is_none() && tag.is_none() && !_scene_has_curves(scene)? {
        return cmds.get_item(0);
    }
    py.import("built_core.history")?
        .call_method1("SnapshotCompound", (cmds,))
}
#[pyfunction(signature=(scene,segments,detect_faces=true,extra=None,eye=None,plane_rebuild=false,*,preserve_faces=false))]
fn build_add_edges<'p>(
    scene: &Obj<'p>,
    segments: &Obj<'p>,
    detect_faces: bool,
    extra: Option<&Obj<'p>>,
    eye: Option<&Obj<'p>>,
    plane_rebuild: bool,
    preserve_faces: bool,
) -> PyResult<Obj<'p>> {
    let py = scene.py();
    let cmds = plan_edge_commands(
        scene,
        segments,
        detect_faces,
        eye,
        !preserve_faces,
        None,
        None,
    )?;
    if let Some(extra) = extra {
        cmds.call_method1("extend", (extra,))?;
    }
    _append_active_tag(scene, &cmds)?;
    if !preserve_faces
        || has_step(&cmds, &["DeleteFaceCommand", "DeleteEdgesCommand"])?
        || plane_rebuild
    {
        let points = PyList::empty(py);
        for seg in segments.try_iter()? {
            points.call_method1("extend", (seg?,))?;
        }
        _append_flat_curve_rebuild(scene, &cmds, &points, !preserve_faces)?;
        if plane_rebuild {
            _append_face_plane_rebuild(scene, &cmds, &points)?;
        }
    }
    if cmds.len() == 1 && !_scene_has_curves(scene)? {
        return cmds.get_item(0);
    }
    let kw = PyDict::new(py);
    kw.set_item("heal", !preserve_faces)?;
    py.import("built_core.history")?
        .getattr("SnapshotCompound")?
        .call((cmds,), Some(&kw))
}

fn store_profile<'p>(mesh: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    let api = mesh.py().import("built_core.store_mesh")?;
    if !mesh.get_type().is(&api.getattr("StoreMesh")?) {
        return Ok(None);
    }
    let h = api.call_method0("_hypot")?;
    Ok((!h.is_none()).then_some(h))
}
fn compensated(py: Python<'_>) -> PyResult<bool> {
    py.import("sys")?.getattr("version_info")?.ge((3, 12))
}
fn keys(edges: &Obj<'_>) -> PyResult<Vec<u128>> {
    edges
        .try_iter()?
        .map(|e| e?.getattr("key")?.extract())
        .collect()
}
#[pyfunction]
fn divide_edges(mesh: &Obj<'_>, edges: &Obj<'_>, n: &Obj<'_>) -> PyResult<usize> {
    let py = mesh.py();
    let n = py
        .import("builtins")?
        .call_method1("int", (n,))?
        .extract::<isize>()?;
    if n < 2 {
        return Ok(0);
    }
    let Some(h) = store_profile(mesh)? else {
        return crate::object_edge_edits::divide_edges(mesh, edges, n as usize);
    };
    let mm = py.import("built_core.mesh")?;
    let pair = mesh.getattr("_store")?.call_method1(
        "divide_edges",
        (
            keys(edges)?,
            n,
            mm.getattr("_CURVE_COUNTER")?,
            h,
            compensated(py)?,
        ),
    )?;
    let (cuts, next_id): (usize, i64) = pair.get_item(0)?.extract()?;
    mm.setattr("_CURVE_COUNTER", next_id)?;
    mesh.call_method1("_dirty", (3 * cuts,))?;
    if pair
        .get_item(1)?
        .getattr("modified")?
        .get_item("edge")?
        .is_truthy()?
    {
        mesh.setattr("_chunk_dirty", true)?;
    }
    Ok(cuts)
}
#[pyfunction(signature=(mesh,edges,verdicts=None))]
fn join_edges<'p>(
    mesh: &Obj<'p>,
    edges: &Obj<'p>,
    verdicts: Option<&Bound<'p, PyDict>>,
) -> PyResult<List<'p>> {
    let py = mesh.py();
    let pool = mesh.getattr("edges")?;
    let live = PyList::empty(py);
    for e in edges.try_iter()? {
        let e = e?;
        if pool.contains(&e)? {
            live.append(e)?;
        }
    }
    let Some(h) = store_profile(mesh)? else {
        return Ok(objects(py)?
            .call_method1("join_edges", (mesh, live, verdicts))?
            .cast_into()?);
    };
    let result = mesh
        .getattr("_store")?
        .call_method1("join_edges", (keys(&live)?, h, compensated(py)?))?
        .get_item(0)?;
    let joins = result.get_item(2)?.extract::<usize>()?;
    mesh.call_method1("_dirty", (3 * joins,))?;
    let api = py.import("built_core.store_mesh")?;
    if let Some(v) = verdicts {
        for pair in result.get_item(1)?.try_iter()? {
            let p = pair?;
            v.set_item(
                api.call_method1("VertexView", (mesh, p.get_item(0)?))?,
                p.get_item(1)?,
            )?;
        }
    }
    let out = PyList::empty(py);
    for key in result.get_item(0)?.try_iter()? {
        out.append(api.call_method1("EdgeView", (mesh, key?))?)?;
    }
    Ok(out)
}
#[pyfunction]
fn join_preview<'p>(mesh: &Obj<'p>, edges: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let snap = mesh.call_method0("capture_state")?;
    let verdicts = PyDict::new(mesh.py());
    let result = join_edges(mesh, edges, Some(&verdicts));
    mesh.call_method1("restore_state", (snap,))?;
    result?;
    Ok(verdicts)
}
#[pyfunction(signature=(r#loop,eye=None))]
fn face_up_or_toward<'p>(r#loop: &Obj<'p>, eye: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    objects(r#loop.py())?.call_method1("face_winding", (r#loop, eye))
}
#[pyfunction(pass_module)]
fn __getattr__<'p>(m: &Bound<'p, PyModule>, name: &str) -> PyResult<Obj<'p>> {
    let py = m.py();
    let source = match name {
        "AddEdgeCommand" | "AddFaceCommand" | "DeleteFaceCommand" | "DeleteEdgesCommand"
        | "Command" | "CompoundCommand" | "SnapshotCompound" => "built_core.history",
        "Face" | "Edge" => "built_core.geometry",
        "element_identity" => "built_core.mesh",
        "_point_in_face_solid"
        | "face_exists"
        | "find_chord_split"
        | "find_cycles_through"
        | "find_duplicate_edge"
        | "is_planar"
        | "plan_edge_split"
        | "split_edge_in_faces" => "built_core.topology",
        _ => return Err(PyAttributeError::new_err(name.to_owned())),
    };
    let value = py.import(source)?.getattr(name)?;
    m.setattr(name, &value)?;
    Ok(value)
}
pub(super) fn register(root: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new(root.py(), "built_core.edits")?;
    let d = root.getattr("object_drawing")?;
    for (alias, name) in [
        ("_Nearby", "Nearby"),
        ("touching_segments", "touching_segments"),
        ("_order_chain", "order_chain"),
    ] {
        m.add(alias, d.getattr(name)?)?;
    }
    m.add("_newell", root.getattr("polygon_newell")?)?;
    let vec = root.getattr("Vec3")?;
    m.add("QVector3D", &vec)?;
    m.add(
        "Segment",
        root.py().get_type::<PyTuple>().get_item((&vec, &vec))?,
    )?;
    m.add("JOIN_REASONS", ("join", "kink", "branch", "flags", "curve"))?;
    m.add_class::<PlanEdges>()?;
    macro_rules! add{($($f:ident),*)=>{$(m.add_function(wrap_pyfunction!($f,&m)?)?;)*};}
    add!(
        scan_edges,
        scan_faces,
        plan_edge_commands,
        _scene_has_curves,
        _whole_drawing_plane,
        _append_flat_curve_rebuild,
        _append_face_plane_rebuild,
        _append_active_tag,
        face_up_or_toward,
        build_add_edge,
        build_add_edges,
        divide_edges,
        join_edges,
        join_preview,
        __getattr__
    );
    root.add_submodule(&m)
}
