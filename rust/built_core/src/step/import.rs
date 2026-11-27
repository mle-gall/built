// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::{exact::Frame as Plane, math::Matrix4, store::edit::Change};
use built_step::step_io::{model as m, scene::ProductDef};
use pyo3::types::{PyDict, PyList};
use std::collections::HashSet;

fn make_mesh<'p>(py: Python<'p>, body: &Body, exact: bool) -> PyResult<Bound<'p, PyAny>> {
    let mesh = py.import("built_core.mesh")?.call_method0("Mesh")?;
    let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
    let mut borrow = store.try_borrow_mut()?;
    let s = &mut borrow.inner;
    let mut change = Change::default();
    for indices in &body.faces {
        let rings: Vec<Vec<_>> = indices
            .iter()
            .map(|r| r.iter().map(|&v| body.points[v].to_array()).collect())
            .collect();
        let f = s
            .add_face(&rings, &mut change)
            .map_err(|e| bad(format!("invalid imported face: {e:?}")))?;
        if exact {
            let (frame, _) = body.plane(indices).map_err(bad)?;
            let plane =
                Plane::from_array([frame.origin.map(|x| x / 1000.), frame.axis, frame.ref_dir]);
            s.bind_plane(f, plane).map_err(bad)?;
        }
    }
    s.finish(change);
    drop(borrow);
    mesh.call_method0("_bump")?;
    Ok(mesh)
}

struct Node<'p> {
    name: String,
    meshes: Vec<Bound<'p, PyAny>>,
    children: Vec<(m::EntityKey, DMat4)>,
}
fn instance<'p>(
    py: Python<'p>,
    nodes: &HashMap<m::EntityKey, Node<'p>>,
    key: m::EntityKey,
    xf: DMat4,
    path: &mut HashSet<m::EntityKey>,
) -> PyResult<Bound<'p, PyAny>> {
    if !path.insert(key) || path.len() > 256 {
        return Err(bad("invalid STEP assembly hierarchy"));
    }
    let n = nodes
        .get(&key)
        .ok_or_else(|| bad("missing STEP definition"))?;
    let group = py.import("built_core.group")?.getattr("Group")?;
    let mesh = if n.meshes.len() == 1 {
        n.meshes[0].clone()
    } else {
        py.import("built_core.mesh")?.call_method0("Mesh")?
    };
    let g = group.call1((mesh, &n.name))?;
    g.setattr("component", true)?;
    g.setattr("xform", Matrix4::from(xf))?;
    let children = g.getattr("children")?;
    if n.meshes.len() > 1 {
        for mesh in &n.meshes {
            children.call_method1("append", (group.call1((mesh, &n.name))?,))?;
        }
    }
    for &(child, at) in &n.children {
        children.call_method1("append", (instance(py, nodes, child, at, path)?,))?;
    }
    path.remove(&key);
    Ok(g)
}
fn label(d: ProductDef<'_>) -> String {
    d.product()
        .map(|p| p.name().to_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| d.id().to_owned())
}

#[pyfunction]
pub(super) fn prepare_step(py: Python<'_>, path: PathBuf) -> PyResult<Bound<'_, PyDict>> {
    let bytes = std::fs::read(path)?;
    let exchange = py.detach(|| built_step::read(&bytes)).map_err(bad)?;
    let model = &exchange.model;
    let scene = model.scene();
    let metres = scene
        .units()
        .length
        .ok_or_else(|| bad("missing STEP length unit"))?
        .to_si;
    if !metres.is_finite() || metres <= 0. {
        return Err(bad("invalid STEP length unit"));
    }
    // The scene helper resolves the first context. Refuse a different length
    // unit until representation-local conversion is available.
    let contexts = model
        .all::<m::ComplexEntity>()
        .flat_map(|(_, c)| &c.parts)
        .filter_map(|p| match p {
            m::PartialEntity::GlobalUnitAssignedContext { units } => Some(units),
            _ => None,
        })
        .chain(
            model
                .all::<m::GlobalUnitAssignedContext>()
                .map(|(_, c)| &c.units),
        );
    let mut units = None;
    for context in contexts {
        if units.replace(context).is_some_and(|u| u != context) {
            return Err(bad("multiple distinct STEP unit contexts"));
        }
    }
    let mut problems: Vec<String> = exchange
        .report
        .dropped
        .iter()
        .map(|(id, reason)| format!("#{id}: {reason}"))
        .collect();
    let mut exact = 0;
    let mut faceted = 0;
    let mut rejected = 0;
    let mut missing = 0;
    let mut nodes = HashMap::new();
    let mut owned = HashSet::new();
    for def in scene.all_product_definitions() {
        let name = label(def);
        let mut meshes = Vec::new();
        for solid in def.solids() {
            owned.insert(solid.key());
            let tol = scene.units().precision.unwrap_or(1e-6) * metres;
            let result = super::exact_read::rows(model, solid, metres, tol)
                .and_then(crate::store::native::NativePatch::new)
                .map_err(bad)
                .and_then(|patch| {
                    let mesh = py.import("built_core.mesh")?.call_method0("Mesh")?;
                    let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
                    store
                        .try_borrow_mut()?
                        .inner
                        .install_patch(std::sync::Arc::new(patch))
                        .map_err(bad)?;
                    mesh.call_method0("_bump")?;
                    Ok(mesh)
                });
            match result {
                Ok(mesh) => {
                    meshes.push(mesh);
                    exact += 1
                }
                Err(e) => {
                    problems.push(format!("{name} ({:?}): {e}", solid.key()));
                    rejected += 1
                }
            }
        }
        let mut children = Vec::new();
        for occurrence in def.occurrences() {
            let Some((child, mut rows)) = occurrence
                .definition()
                .zip(occurrence.transform().and_then(|t| t.matrix()))
            else {
                missing += 1;
                problems.push(format!("{name}: unresolved occurrence"));
                continue;
            };
            for row in &mut rows[..3] {
                row[3] *= metres
            }
            children.push((child.key(), DMat4::from_cols_array_2d(&rows).transpose()));
        }
        nodes.insert(
            def.key(),
            Node {
                name,
                meshes,
                children,
            },
        );
    }
    for solid in scene.all_solids().filter(|s| !owned.contains(&s.key())) {
        missing += 1;
        problems.push(format!("Unresolved solid {:?}", solid.key()));
    }
    for sheet in scene.all_surface_models() {
        rejected += 1;
        problems.push(format!("Unsupported sheet {:?}", sheet.key()));
    }
    for mapped in scene.all_mapped_instances() {
        missing += 1;
        problems.push(format!("Unresolved mapped placement {:?}", mapped.key()));
    }
    if scene.all_solids().next().is_none() && scene.all_meshes().next().is_some() {
        if nodes.len() == 1 && scene.all_mapped_instances().next().is_none() {
            let node = nodes.values_mut().next().unwrap();
            for mesh in scene.all_meshes() {
                let points: Vec<_> = mesh
                    .points()
                    .into_iter()
                    .map(|p| DVec3::from(p) * metres)
                    .collect();
                let triangles = mesh.triangles();
                if triangles.is_empty() || triangles.iter().flatten().any(|&v| v >= points.len()) {
                    return Err(bad("invalid STEP display mesh"));
                }
                let body = Body {
                    points,
                    faces: triangles.into_iter().map(|t| vec![t.to_vec()]).collect(),
                };
                node.meshes.push(make_mesh(py, &body, false)?);
                faceted += 1;
            }
        } else {
            return Err(bad("ambiguous STEP tessellation ownership"));
        }
    }
    problems.extend(scene.warnings().iter().map(ToString::to_string));
    if exact + faceted == 0 {
        return Err(bad(format!(
            "No importable STEP bodies. {}",
            problems.join("\n")
        )));
    }
    let groups = PyList::empty(py);
    for root in scene.root_definitions() {
        groups.append(instance(
            py,
            &nodes,
            root.key(),
            DMat4::IDENTITY,
            &mut HashSet::new(),
        )?)?;
    }
    if groups.is_empty() {
        return Err(bad("no reachable STEP product roots"));
    }
    let result = PyDict::new(py);
    result.set_item("groups", groups)?;
    result.set_item("exact", exact)?;
    result.set_item("faceted", faceted)?;
    result.set_item("rejected", rejected)?;
    result.set_item("missing", missing)?;
    result.set_item("problems", problems)?;
    result.set_item("synthetic_file_name", exchange.synthetic_file_name)?;
    Ok(result)
}
#[pyfunction]
#[pyo3(signature=(scene,prepared,accept_partial=false))]
pub(super) fn apply_step(
    scene: &Bound<'_, PyAny>,
    prepared: &Bound<'_, PyDict>,
    accept_partial: bool,
) -> PyResult<()> {
    if !accept_partial
        && prepared
            .get_item("problems")?
            .ok_or_else(|| bad("missing STEP report"))?
            .is_truthy()?
    {
        return Err(bad("Partial STEP import requires confirmation"));
    }
    scene.getattr("groups")?.call_method1(
        "extend",
        (prepared
            .get_item("groups")?
            .ok_or_else(|| bad("missing STEP geometry"))?,),
    )?;
    Ok(())
}
