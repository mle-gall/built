// SPDX-License-Identifier: GPL-3.0-or-later
//! Resolve tessellated items through their product-definition shape.
use super::{bad, import::make_mesh, Body};
use built_step::step_io::{model as m, scene::Mesh, StepModel};
use glam::DVec3;
use pyo3::prelude::*;
use std::collections::{HashMap, HashSet};

pub(super) fn read<'p>(
    py: Python<'p>,
    model: &StepModel,
    metres: f64,
    exact: &HashSet<m::EntityKey>,
    nodes: &mut HashMap<m::EntityKey, super::import::Node<'p>>,
    problems: &mut Vec<String>,
) -> (usize, usize) {
    use m::{CharacterizedDefinitionRef as Def, EntityKey as K, RepresentationItemRef as I};
    let scene = model.scene();
    let meshes: HashMap<_, _> = scene.all_meshes().map(|m| (m.key(), m)).collect();
    let groups: HashMap<_, _> = scene.all_mesh_groups().map(|g| (g.key(), g)).collect();
    let mut seen = HashSet::new();
    let mut placed = HashSet::new();
    let mut accepted = 0;
    let mut rejected = 0;
    for (_, sdr) in model.all::<m::ShapeDefinitionRepresentation>() {
        let m::RepresentedDefinitionRef::ProductDefinitionShape(shape) = sdr.definition else {
            continue;
        };
        let owner = match model[shape].definition {
            Def::ProductDefinition(id) => K::ProductDefinition(id),
            Def::ProductDefinitionWithAssociatedDocuments(id) => {
                K::ProductDefinitionWithAssociatedDocuments(id)
            }
            _ => continue,
        };
        let Some(node) = nodes.get_mut(&owner) else {
            continue;
        };
        let items = match sdr.used_representation {
            m::RepresentationRef::TessellatedShapeRepresentation(id) => &model[id].items,
            m::RepresentationRef::ShapeRepresentation(id) => &model[id].items,
            _ => continue,
        };
        for item in items {
            let key = match item {
                I::TessellatedSolid(id) => K::TessellatedSolid(*id),
                I::TessellatedShell(id) => K::TessellatedShell(*id),
                I::ComplexTriangulatedFace(id) => K::ComplexTriangulatedFace(*id),
                I::ComplexTriangulatedSurfaceSet(id) => K::ComplexTriangulatedSurfaceSet(*id),
                I::TessellatedFace(id) => K::TessellatedFace(*id),
                _ => continue,
            };
            if !placed.insert((owner, key)) {
                continue;
            }
            let mut parts = vec![];
            let represented = if let Some(g) = groups.get(&key) {
                parts.extend(g.meshes());
                g.solid().is_some_and(|s| exact.contains(&s.key()))
            } else if let Some(m) = meshes.get(&key) {
                parts.push(*m);
                false
            } else {
                false
            };
            seen.extend(parts.iter().map(Mesh::key));
            if represented {
                continue;
            }
            let result = (|| -> PyResult<_> {
                let mut body = Body::default();
                for m in parts {
                    let points = m.points();
                    let tris = m.triangles();
                    if tris.is_empty() || tris.iter().flatten().any(|&i| i >= points.len()) {
                        return Err(bad("invalid display triangles"));
                    }
                    let base = body.points.len();
                    body.points
                        .extend(points.into_iter().map(|p| DVec3::from(p) * metres));
                    body.faces
                        .extend(tris.into_iter().map(|t| vec![t.map(|v| base + v).to_vec()]));
                }
                if body.faces.is_empty() {
                    return Err(bad("empty tessellated body"));
                }
                make_mesh(py, &body, false)
            })();
            match result {
                Ok(mesh) => {
                    node.meshes.push(mesh);
                    accepted += 1
                }
                Err(e) => {
                    problems.push(format!("{} ({key:?}): {e}", node.name));
                    rejected += 1
                }
            }
        }
    }
    for key in meshes.keys().filter(|k| !seen.contains(*k)) {
        problems.push(format!("Unresolved tessellated item {key:?}"));
        rejected += 1;
    }
    (accepted, rejected)
}
