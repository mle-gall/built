// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{
    exact::{Curve, Frame, Rows, Surface},
    store::native::NativePatch,
};
use built_step::{
    planar::Body,
    step_io::{
        build::{CurveInput, FaceBoundInput, Frame as OutputFrame, Part, SurfaceInput},
        scene::Solid,
        Orientation, StepBuilder, StepModel,
    },
};
use glam::DVec3;
use std::{collections::HashMap, sync::Arc};

pub(super) enum Geometry {
    Facets(Body),
    Exact(Arc<NativePatch>),
}

fn frame(f: &Frame) -> OutputFrame {
    OutputFrame {
        origin: (f.origin * 1000.).to_array(),
        axis: f.axis.to_array(),
        ref_dir: f.reference.to_array(),
    }
}
fn orient(rev: bool) -> Orientation {
    if rev {
        Orientation::Reversed
    } else {
        Orientation::Forward
    }
}
fn volume(rows: &Rows, tolerance: f64) -> Result<f64, String> {
    let mut t = rows.topology.clone().ok_or("missing topology")?;
    t.tolerance = tolerance;
    let mesh = t.mesh(rows)?;
    let base = DVec3::from(*mesh.points.first().ok_or("empty solid")?);
    let mut volumes = vec![0.; t.void_starts.len() + 1];
    for (tri, &face) in mesh.triangles.iter().zip(&mesh.face_ids) {
        let shell = t.void_starts.partition_point(|&i| i <= face as usize);
        let [a, b, c] = tri.map(|i| DVec3::from(mesh.points[i]) - base);
        volumes[shell] += a.dot(b.cross(c)) / 6.;
    }
    if volumes[0] <= 0. || volumes[1..].iter().any(|v| *v >= 0.) {
        return Err("inconsistent shell orientation".into());
    }
    let volume = volumes.iter().sum::<f64>();
    if !volume.is_finite() || volume <= 0. {
        return Err("nonpositive shell volume".into());
    }
    Ok(volume)
}

impl Geometry {
    pub(super) fn add(
        &self,
        b: &mut StepBuilder,
        part: Part,
        name: &str,
        tol: f64,
    ) -> Result<built_step::Measure, String> {
        let Self::Exact(p) = self else {
            let Self::Facets(body) = self else {
                unreachable!()
            };
            return body.add(b, part, name);
        };
        let t = p.topology();
        let mut vertices = HashMap::new();
        for (&id, point) in &t.vertices {
            vertices.insert(
                id,
                b.vertex((*point * 1000.).to_array())
                    .map_err(|e| e.to_string())?,
            );
        }
        let mut edges = HashMap::new();
        for e in &t.edges {
            let curve = match &p.rows.curves[e.curve as usize] {
                Curve::Line { direction, .. } => CurveInput::LineAlong(direction.to_array()),
                Curve::Circle { frame: f, radius } => CurveInput::Circle(frame(f), radius * 1000.),
                Curve::Ellipse {
                    frame: f,
                    major,
                    minor,
                } => CurveInput::Ellipse(frame(f), major * 1000., minor * 1000.),
                _ => return Err("unsupported STEP boundary curve".into()),
            };
            let [a, z] = e.vertices;
            edges.insert(
                e.id,
                b.edge(vertices[&a], vertices[&z], curve)
                    .map_err(|e| e.to_string())?,
            );
        }
        let mut faces = Vec::new();
        let mut uses = HashMap::<u64, Vec<bool>>::new();
        for f in &t.faces {
            let surface = match &p.rows.surfaces[f.surface as usize] {
                Surface::Plane { frame: f } => SurfaceInput::Plane(frame(f)),
                Surface::Cylinder { frame: f, radius } => {
                    SurfaceInput::Cylinder(frame(f), radius * 1000.)
                }
                Surface::Cone {
                    frame: f,
                    radius,
                    half_angle,
                } => SurfaceInput::Cone(frame(f), radius * 1000., *half_angle),
                _ => return Err("unsupported STEP face support".into()),
            };
            let mut bounds = Vec::new();
            for (i, ring) in f.loops.iter().enumerate() {
                let mut coedges = Vec::new();
                for c in ring {
                    let id = c.edge.ok_or("unsupported singular bound")?;
                    uses.entry(id).or_default().push(c.reversed);
                    coedges.push((edges[&id], orient(c.reversed)));
                }
                bounds.push(FaceBoundInput {
                    edges: coedges,
                    outer: i == 0,
                    orientation: Orientation::Forward,
                })
            }
            faces.push(
                b.face(surface, orient(f.reversed), bounds)
                    .map_err(|e| e.to_string())?,
            );
        }
        if uses.len() != edges.len() || uses.values().any(|v| v.len() != 2 || v[0] == v[1]) {
            return Err("STEP solid requires a closed, oriented shell".into());
        }
        if t.void_starts.is_empty() {
            b.solid(part, name, faces).map_err(|e| e.to_string())?;
        } else {
            let mut cavities = Vec::new();
            for &first in t.void_starts.iter().rev() {
                cavities.push(faces.split_off(first));
            }
            cavities.reverse();
            b.solid_with_voids(
                part,
                name,
                faces,
                cavities,
                built_step::step_io::build::VoidShellNormals::AwayFromMaterial,
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(built_step::Measure::Volume(volume(&p.rows, tol)?))
    }
}
pub(super) fn reopened(model: &StepModel, s: Solid<'_>, m: f64, tol: f64) -> Result<f64, String> {
    if let Ok(body) = Body::from_solid(s, m) {
        return body.volume();
    }
    let rows = super::exact_read::rows(model, s, m, tol)?;
    let patch = NativePatch::new(rows)?;
    volume(&patch.rows, tol)
}
