// SPDX-License-Identifier: GPL-3.0-or-later
//! Gather editable edges and replay the store's angle-softening plan by stable key.
use pyo3::{
    exceptions::{PyAttributeError, PyValueError},
    prelude::*,
    types::{PyDict, PyList, PySet},
};
use std::collections::HashSet;
type Obj<'p> = Bound<'p, PyAny>;

#[pyfunction]
fn selected_edges<'p>(scene: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let py = scene.py();
    let mesh = scene.getattr("mesh")?;
    let geo = py.import("built_core.mesh")?;
    let (edge, face, group) = (
        geo.getattr("Edge")?,
        geo.getattr("Face")?,
        py.import("built_core.group")?.getattr("Group")?,
    );
    let mut places = vec![];
    let mut seen = HashSet::new();
    let keys = PySet::empty(py)?;
    let faces = PySet::empty(py)?;
    let links = py.import("built_core.links")?;
    for ent in scene.getattr("selection")?.try_iter()? {
        let e = ent?;
        if e.is_instance(&edge)? {
            keys.add(e.getattr("key")?)?;
        } else if e.is_instance(&face)? {
            faces.add(e)?;
        } else if e.is_instance(&group)?
            && !e.getattr("locked")?.is_truthy()?
            && py
                .import("built_core")?
                .call_method1("link_linked_ancestor", (&e,))?
                .is_none()
        {
            places.push(e);
        }
    }
    if !faces.is_empty() {
        for edge in mesh.getattr("edges")?.try_iter()? {
            let e = edge?;
            let adj = e.getattr("faces")?;
            if adj.len()? == 2
                && faces.contains(adj.get_item(0)?)?
                && faces.contains(adj.get_item(1)?)?
            {
                keys.add(e.getattr("key")?)?;
            }
        }
    }
    let targets = PyDict::new(py);
    if !keys.is_empty() {
        targets.set_item(&mesh, keys)?;
    }
    while let Some(g) = places.pop() {
        if !seen.insert(g.as_ptr())
            || g.getattr("locked")?.is_truthy()?
            || links.call_method1("is_linked", (&g,))?.is_truthy()?
        {
            continue;
        }
        let children = g.getattr("children")?;
        if !children.is_none() {
            for c in children.try_iter()? {
                places.push(c?);
            }
        }
        let m = g.getattr("mesh")?;
        if m.is_none() {
            continue;
        }
        let keys = targets
            .call_method1("setdefault", (&m, PySet::empty(py)?))?
            .cast_into::<PySet>()?;
        for e in m.getattr("edges")?.try_iter()? {
            keys.add(e?.getattr("key")?)?;
        }
    }
    Ok(targets)
}
#[pyfunction]
#[pyo3(signature=(cmd,targets,angle=30.0,coplanar=true,harden=false,materials=false,smooth=true))]
fn init(
    cmd: &Obj<'_>,
    targets: &Obj<'_>,
    angle: f64,
    coplanar: bool,
    harden: bool,
    materials: bool,
    smooth: bool,
) -> PyResult<()> {
    if !(0.0..=180.0).contains(&angle) {
        return Err(PyValueError::new_err(
            "Angle must be between 0 and 180 degrees",
        ));
    }
    cmd.setattr("targets", targets)?;
    cmd.setattr("angle", angle)?;
    for (k, v) in [
        ("coplanar", coplanar),
        ("harden", harden),
        ("materials", materials),
        ("smooth", smooth),
    ] {
        cmd.setattr(k, v)?;
    }
    cmd.setattr("changes", cmd.py().None())
}
#[pyfunction]
fn geometry_meshes<'p>(cmd: &Obj<'p>, _scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    cmd.getattr("targets")
}
#[pyfunction]
fn meshes<'p>(cmd: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    let changes = cmd.getattr("changes")?;
    let mut seen = HashSet::new();
    let mut meshes = vec![];
    if !changes.is_none() {
        for row in changes.try_iter()? {
            let mesh = row?.get_item(0)?;
            if seen.insert(mesh.as_ptr()) {
                meshes.push(mesh);
            }
        }
    }
    Ok(meshes)
}
#[pyfunction]
fn matching<'p>(mesh: &Obj<'p>, faces: &Obj<'p>, a: &Obj<'p>, b: &Obj<'p>) -> PyResult<bool> {
    let py = mesh.py();
    let view = py.import("built_core.store_mesh")?.getattr("FaceView")?;
    let sig = py.import("built_core.formats.fuse")?.getattr("_smooth_sig")?;
    let face_sig = |key: &Obj<'p>| -> PyResult<Obj<'p>> {
        let f = if faces.is_none() {
            view.call1((mesh, key))?
        } else {
            faces.get_item(key)?
        };
        sig.call1((f.getattr("attrs")?,))
    };
    face_sig(a)?.eq(face_sig(b)?)
}
#[pyfunction]
fn normal<'p>(faces: &Obj<'p>, k: &Obj<'p>) -> PyResult<Obj<'p>> {
    faces
        .get_item(k)?
        .call_method0("normal")?
        .call_method0("toTuple")
}
#[pyfunction]
fn prepare<'p>(cmd: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let py = cmd.py();
    let changes = PyList::empty(py);
    let api = py.import("built_core.store_mesh")?;
    let native = py.import("built_core")?.getattr("MeshStore")?;
    let bind = py.import("functools")?.getattr("partial")?;
    for (mesh, keys) in cmd.getattr("targets")?.cast::<PyDict>()?.iter() {
        let store = api.call_method1("_store_of", (&mesh,))?;
        let angle = cmd.getattr("angle")?;
        let coplanar = cmd.getattr("coplanar")?;
        let harden = cmd.getattr("harden")?;
        let smooth = cmd.getattr("smooth")?;
        let matches = |faces: &Obj<'p>| -> PyResult<Obj<'p>> {
            if cmd.getattr("materials")?.is_truthy()? {
                Ok(py.None().into_bound(py))
            } else {
                bind.call1((wrap_pyfunction!(matching, py)?, &mesh, faces))
            }
        };
        let plan = if !store.is_none() {
            let keys = py.import("builtins")?.call_method1("set", (keys,))?;
            store.call_method1(
                "plan_softening",
                (
                    keys,
                    angle,
                    coplanar,
                    harden,
                    smooth,
                    matches(&py.None().into_bound(py))?,
                    api.call_method0("_hypot")?,
                ),
            )?
        } else {
            let faces = PyDict::new(py);
            let rows = PyList::empty(py);
            for edge in mesh.getattr("edges")?.try_iter()? {
                let e = edge?;
                let k = e.getattr("key")?;
                let adj = e.getattr("faces")?;
                if !keys.contains(&k)? || adj.len()? != 2 {
                    continue;
                }
                let a = adj.get_item(0)?;
                let b = adj.get_item(1)?;
                let ka = a.getattr("key")?;
                let kb = b.getattr("key")?;
                faces.set_item(&ka, a)?;
                faces.set_item(&kb, b)?;
                let flat = e
                    .getattr_opt("flat")?
                    .is_some_and(|v| v.is_truthy().unwrap_or(false));
                rows.append((k, (e.getattr("soft")?.is_truthy()?, flat), (ka, kb)))?;
            }
            native.call_method1(
                "plan_object_softening",
                (
                    rows,
                    angle,
                    coplanar,
                    harden,
                    smooth,
                    matches(faces.as_any())?,
                    bind.call1((wrap_pyfunction!(normal, py)?, faces))?,
                ),
            )?
        };
        for row in plan.try_iter()? {
            let row = row?;
            changes.append((&mesh, row.get_item(0)?, row.get_item(1)?, row.get_item(2)?))?;
        }
    }
    Ok(changes)
}
fn apply_changes(cmd: &Obj<'_>, scene: &Obj<'_>, after: bool) -> PyResult<()> {
    let py = cmd.py();
    let by_mesh = PyDict::new(py);
    for row in cmd.getattr("changes")?.try_iter()? {
        let row = row?;
        by_mesh
            .call_method1("setdefault", (row.get_item(0)?, PyList::empty(py)))?
            .call_method1(
                "append",
                ((row.get_item(1)?, row.get_item(2)?, row.get_item(3)?),),
            )?;
    }
    for (mesh, rows) in by_mesh.iter() {
        let store = py
            .import("built_core.store_mesh")?
            .call_method1("_store_of", (&mesh,))?;
        if store.is_none() {
            py.import("built_core")?
                .getattr("MeshStore")?
                .call_method1(
                    "apply_object_softening",
                    (mesh.getattr("edges")?, rows, after),
                )?;
        } else {
            store.call_method1("apply_softening", (rows, after))?;
        }
        mesh.call_method0("mark_modified")?;
    }
    if !by_mesh.is_empty() {
        scene.setattr("version", scene.getattr("version")?.add(1)?)?;
    }
    Ok(())
}
#[pyfunction]
fn apply(cmd: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    if cmd.getattr("changes")?.is_none() {
        cmd.setattr("changes", prepare(cmd)?)?;
    }
    apply_changes(cmd, scene, true)
}
#[pyfunction]
fn undo(cmd: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    apply_changes(cmd, scene, false)
}
#[pyfunction]
#[pyo3(pass_module)]
fn __getattr__<'p>(m: &Bound<'p, PyModule>, name: &str) -> PyResult<Obj<'p>> {
    if name != "SoftenByAngleCommand" {
        return Err(PyAttributeError::new_err(name.to_owned()));
    }
    let py = m.py();
    let ns = PyDict::new(py);
    ns.set_item("__module__", m.name()?)?;
    ns.set_item("keeps_bounds", true)?;
    ns.set_item("keeps_rows", true)?;
    let bind = py.import("functools")?.getattr("partialmethod")?;
    for (name, f) in [
        ("__init__", wrap_pyfunction!(init, py)?),
        ("geometry_meshes", wrap_pyfunction!(geometry_meshes, py)?),
        ("meshes", wrap_pyfunction!(meshes, py)?),
        ("prepare", wrap_pyfunction!(prepare, py)?),
        ("do", wrap_pyfunction!(apply, py)?),
        ("undo", wrap_pyfunction!(undo, py)?),
    ] {
        ns.set_item(name, bind.call1((f,))?)?;
    }
    let cls = py.import("builtins")?.call_method1(
        "type",
        (
            name,
            (py.import("built_core.history")?.getattr("MeshCommand")?,),
            ns,
        ),
    )?;
    m.add(name, &cls)?;
    Ok(cls)
}
pub(crate) fn register(parent: &Bound<PyModule>) -> PyResult<()> {
    let py = parent.py();
    let m = PyModule::new(py, "built_core.soften")?;
    m.add_function(wrap_pyfunction!(selected_edges, &m)?)?;
    m.add_function(wrap_pyfunction!(__getattr__, &m)?)?;
    py.import("sys")?
        .getattr("modules")?
        .set_item("built_core.soften", &m)?;
    parent.add("soften", m)
}
