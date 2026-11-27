// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{
    exact::{Curve, Frame, Rows, Surface},
    store::native::NativePatch,
};
use built_step::step_io::model::RepresentationItemRef as Item;
use built_step::{
    planar::Body,
    step_io::{
        build::{
            CurveInput, FaceBoundInput, Frame as OutputFrame, Part, ProfileInput, SurfaceInput,
        },
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
fn profile(c: &Curve) -> ProfileInput {
    match c {
        Curve::Line { origin, direction } => {
            ProfileInput::Line((*origin * 1000.).to_array(), direction.to_array())
        }
        Curve::Circle { frame: f, radius } => ProfileInput::Circle(frame(f), radius * 1000.),
        Curve::Ellipse {
            frame: f,
            major,
            minor,
        } => ProfileInput::Ellipse(frame(f), major * 1000., minor * 1000.),
        Curve::Spline { axis, controls } => {
            ProfileInput::Nurbs(super::splines::output_curve(axis, controls))
        }
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
    t.display_tolerance = Some(tolerance.max(super::DISPLAY_DEFLECTION));
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
    ) -> Result<built_step::Authored, String> {
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
                Curve::Spline { axis, controls } => {
                    CurveInput::Nurbs(super::splines::output_curve(axis, controls))
                }
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
                Surface::Sphere { frame: f, radius } => {
                    SurfaceInput::Sphere(frame(f), radius * 1000.)
                }
                Surface::Torus {
                    frame: f,
                    major,
                    minor,
                } => SurfaceInput::Torus(frame(f), major * 1000., minor * 1000.),
                Surface::Spline {
                    u,
                    v,
                    cols,
                    controls,
                    ..
                } => SurfaceInput::Nurbs(super::splines::output_surface(u, v, *cols, controls)),
                Surface::Extrusion { generator, vector } => SurfaceInput::LinearExtrusion(
                    profile(&p.rows.curves[*generator as usize]),
                    (*vector * 1000.).to_array(),
                ),
                Surface::Revolution {
                    generator,
                    origin,
                    axis,
                } => SurfaceInput::Revolution(
                    profile(&p.rows.curves[*generator as usize]),
                    (*origin * 1000.).to_array(),
                    axis.to_array(),
                ),
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
                b.face(
                    surface,
                    orient(
                        f.reversed
                            ^ matches!(
                                p.rows.surfaces[f.surface as usize],
                                Surface::Revolution { .. }
                            ),
                    ),
                    bounds,
                )
                .map_err(|e| e.to_string())?,
            );
        }
        if uses.len() != edges.len()
            || uses.values().any(|u| match u.as_slice() {
                [_] => false,
                [a, b] => a == b,
                _ => true,
            })
        {
            return Err("nonmanifold or inconsistently oriented STEP shell".into());
        }
        if uses.values().any(|u| u.len() == 1) {
            if !t.void_starts.is_empty() {
                return Err("open boundary in a void solid".into());
            }
            let item =
                built_step::sheet::item(b.author(), name, faces).map_err(|e| e.to_string())?;
            return Ok(built_step::Authored(
                built_step::Measure::Area(area(p, tol)?),
                Item::ShellBasedSurfaceModel(item),
            ));
        }
        let item = if t.void_starts.is_empty() {
            Item::ManifoldSolidBrep(b.solid(part, name, faces).map_err(|e| e.to_string())?)
        } else {
            let mut cavities = Vec::new();
            for &first in t.void_starts.iter().rev() {
                cavities.push(faces.split_off(first));
            }
            cavities.reverse();
            Item::BrepWithVoids(
                b.solid_with_voids(
                    part,
                    name,
                    faces,
                    cavities,
                    built_step::step_io::build::VoidShellNormals::AwayFromMaterial,
                )
                .map_err(|e| e.to_string())?,
            )
        };
        Ok(built_step::Authored(
            built_step::Measure::Volume(volume(&p.rows, tol)?),
            item,
        ))
    }
}
pub(super) fn reopened(model: &StepModel, s: Solid<'_>, m: f64, tol: f64) -> Result<f64, String> {
    if let Ok(body) = Body::from_solid(s, m) {
        return body.volume();
    }
    let rows = super::exact_read::rows(model, s, m, tol, Some(1.))?;
    let patch = NativePatch::new(rows)?;
    volume(&patch.rows, tol)
}

fn area(p: &NativePatch, tol: f64) -> Result<f64, String> {
    let mut t = p.topology().clone();
    t.display_tolerance = Some(tol.max(super::DISPLAY_DEFLECTION));
    let mesh = t.mesh(&p.rows)?;
    let area = mesh
        .triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| DVec3::from(mesh.points[i]));
            (b - a).cross(c - a).length() * 0.5
        })
        .sum::<f64>();
    if !area.is_finite() || area <= 0. {
        return Err("invalid trimmed sheet area".into());
    }
    Ok(area)
}
pub(super) fn reopened_sheet(
    model: &StepModel,
    s: built_step::step_io::scene::Shell<'_>,
    sense: Orientation,
    m: f64,
    tol: f64,
) -> Result<f64, String> {
    if let Ok(body) = Body::from_shell(s, sense, m) {
        return body.area();
    }
    let rows = super::exact_read::shell_rows(model, s, sense, m, tol, Some(1.))?;
    area(&NativePatch::new(rows)?, tol)
}
