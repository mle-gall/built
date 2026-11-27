// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::{exact::Frame as Plane, math::Matrix4, store::edit::Change};
use built_step::step_io::{model as m, scene::ProductDef};
use pyo3::types::{PyDict, PyList};
use std::collections::HashSet;

pub(super) fn make_mesh<'p>(
    py: Python<'p>,
    body: &Body,
    exact: bool,
) -> PyResult<Bound<'p, PyAny>> {
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

fn native_mesh<'p>(py: Python<'p>, rows: crate::exact::Rows) -> PyResult<Bound<'p, PyAny>> {
    let patch = crate::store::native::NativePatch::new(rows).map_err(bad)?;
    let mesh = py.import("built_core.mesh")?.call_method0("Mesh")?;
    let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
    store
        .try_borrow_mut()?
        .inner
        .install_patch(std::sync::Arc::new(patch))
        .map_err(bad)?;
    mesh.call_method0("_bump")?;
    Ok(mesh)
}

pub(super) struct Node<'p> {
    pub(super) name: String,
    assembly_mesh: Bound<'p, PyAny>,
    pub(super) meshes: Vec<Bound<'p, PyAny>>,
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
        n.assembly_mesh.clone()
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
    let source_ids = std::cell::OnceCell::new();
    let label_entity = |key| {
        source_ids
            .get_or_init(|| built_step::SourceIds::new(&bytes, &exchange))
            .label(model, key)
    };
    let contexts = super::units::Contexts::new(model).map_err(bad)?;
    let mut problems: Vec<String> = exchange
        .report
        .dropped
        .iter()
        .map(|(id, reason)| format!("#{id}: {reason}"))
        .collect();
    let mut exact = 0;
    let mut accepted_solids = HashSet::new();
    let mut rejected = 0;
    let mut missing = 0;
    let mut nodes = HashMap::new();
    let mut owned = HashSet::new();
    for def in scene.all_product_definitions() {
        let name = label(def);
        let mut meshes = Vec::new();
        for solid in def.solids() {
            owned.insert(solid.key());
            let u = contexts.item(solid.key()).map_err(bad)?;
            let result = super::exact_read::rows(model, solid, u.metres, u.tolerance, u.angle)
                .map_err(bad)
                .and_then(|rows| native_mesh(py, rows));
            match result {
                Ok(mesh) => {
                    accepted_solids.insert(solid.key());
                    meshes.push(mesh);
                    exact += 1
                }
                Err(e) => {
                    problems.push(format!("{name} ({}): {e}", label_entity(solid.key())));
                    rejected += 1
                }
            }
        }
        for sheet in def.surface_models() {
            owned.insert(sheet.key());
            for (shell, sense) in sheet.shells() {
                let u = contexts.item(sheet.key()).map_err(bad)?;
                let result = super::exact_read::shell_rows(
                    model,
                    shell,
                    sense,
                    u.metres,
                    u.tolerance,
                    u.angle,
                )
                .map_err(bad)
                .and_then(|rows| match Body::from_shell(shell, sense, u.metres) {
                    Ok(body) => {
                        crate::store::native::NativePatch::new(rows).map_err(bad)?;
                        make_mesh(py, &body, true)
                    }
                    Err(_) => native_mesh(py, rows),
                });
                match result {
                    Ok(mesh) => {
                        meshes.push(mesh);
                        exact += 1
                    }
                    Err(e) => {
                        rejected += 1;
                        problems.push(format!("{name} ({}): {e}", label_entity(shell.key())));
                    }
                }
            }
        }
        let mut children = Vec::new();
        for occurrence in def.occurrences() {
            let Some((child, mut xf)) = occurrence.definition().zip(occurrence.transform()) else {
                missing += 1;
                problems.push(format!("{name}: unresolved occurrence"));
                continue;
            };
            let parent_unit = contexts.definition(def.key()).map_err(bad)?;
            let child_unit = contexts.definition(child.key()).map_err(bad)?;
            xf.from.origin = xf.from.origin.map(|v| v * child_unit.metres);
            xf.to.origin = xf.to.origin.map(|v| v * parent_unit.metres);
            let Some(rows) = xf.matrix() else {
                missing += 1;
                problems.push(format!("{name}: invalid occurrence frame"));
                continue;
            };
            children.push((child.key(), DMat4::from_cols_array_2d(&rows).transpose()));
        }
        nodes.insert(
            def.key(),
            Node {
                assembly_mesh: py.import("built_core.mesh")?.call_method0("Mesh")?,
                name,
                meshes,
                children,
            },
        );
    }
    for solid in scene.all_solids().filter(|s| !owned.contains(&s.key())) {
        missing += 1;
        problems.push(format!("Unresolved solid {}", label_entity(solid.key())));
    }
    for sheet in scene
        .all_surface_models()
        .filter(|s| !owned.contains(&s.key()))
    {
        rejected += 1;
        problems.push(format!("Unsupported sheet {}", label_entity(sheet.key())));
    }
    for mapped in scene.all_mapped_instances() {
        missing += 1;
        problems.push(format!(
            "Unresolved mapped placement {}",
            label_entity(mapped.key())
        ));
    }
    let (faceted, invalid_meshes) = super::display_mesh::read(
        py,
        model,
        &contexts,
        &accepted_solids,
        &mut nodes,
        &mut problems,
    );
    rejected += invalid_meshes;
    for node in nodes
        .values()
        .filter(|n| n.children.is_empty() && n.meshes.is_empty())
    {
        // A rejected solid already carries a more specific diagnostic.
        if !problems
            .iter()
            .any(|p| p.starts_with(&format!("{} (", node.name)))
        {
            missing += 1;
            problems.push(format!(
                "{}: product definition has no admitted geometry",
                node.name
            ));
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
