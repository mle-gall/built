// SPDX-License-Identifier: GPL-3.0-or-later
//! A single VERTEX_LOOP is a complete sphere. The native chart uses one
//! meridian twice and a collapsed side at each pole.
use crate::exact::{
    patch::{Coedge, Edge, Face, Topology},
    Curve, Frame, Rows, Surface,
};
use built_step::{
    step_io::{build::Part, model as m, AuthorError, StepBuilder},
    Authored, Measure,
};
use glam::{DVec2, DVec3};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

pub(super) fn rows(
    frame: Frame,
    r: f64,
    vertex: DVec3,
    tol: f64,
    reversed: bool,
) -> Result<Rows, &'static str> {
    let points = [frame.origin - frame.axis * r, frame.origin + frame.axis * r];
    if !points.iter().any(|&p| p.distance(vertex) <= tol) {
        return Err("sphere bound is not a pole");
    }
    let mut t = Topology::default();
    t.tolerance = tol;
    t.display_tolerance = Some(tol.max(super::DISPLAY_DEFLECTION));
    t.vertices
        .extend(points.into_iter().enumerate().map(|(i, p)| (i as u64, p)));
    t.edges.push(Edge {
        id: 0,
        curve: 0,
        vertices: [0, 1],
        interval: [-FRAC_PI_2, FRAC_PI_2],
    });
    let corners = [
        DVec2::new(0., -FRAC_PI_2),
        DVec2::new(TAU, -FRAC_PI_2),
        DVec2::new(TAU, FRAC_PI_2),
        DVec2::new(0., FRAC_PI_2),
    ];
    let ring = (0..4)
        .map(|i| Coedge {
            edge: (i % 2 == 1).then_some(0),
            pole: (i % 2 == 0).then_some((i / 2) as u64),
            projected: false,
            reversed: i == 3,
            uv: [corners[i], corners[(i + 1) % 4]],
            tolerance: tol,
        })
        .collect();
    t.faces.push(Face {
        id: 0,
        surface: 0,
        reversed,
        loops: vec![ring],
    });
    Ok(Rows {
        curves: vec![Curve::Circle {
            frame: Frame {
                origin: frame.origin,
                axis: -frame.y(),
                reference: frame.reference,
            },
            radius: r,
        }],
        surfaces: vec![Surface::Sphere { frame, radius: r }],
        topology: Some(t),
    })
}

pub(super) fn full(rows: &Rows) -> Option<(&Frame, f64)> {
    let t = rows.topology.as_ref()?;
    let [f] = t.faces.as_slice() else { return None };
    let Surface::Sphere { frame, radius } = &rows.surfaces[f.surface as usize] else {
        return None;
    };
    let [ring] = f.loops.as_slice() else {
        return None;
    };
    if f.reversed || t.edges.len() != 1 || t.vertices.len() != 2 || ring.len() != 4 {
        return None;
    }
    let poles: Vec<_> = ring
        .iter()
        .filter(|c| c.pole.is_some() && ((c.uv[1].x - c.uv[0].x).abs() - TAU).abs() < 1e-12)
        .collect();
    if poles.len() != 2 || (poles[0].uv[0].y - poles[1].uv[0].y).abs() != PI {
        return None;
    }
    Some((frame, *radius))
}
pub(super) fn volume(r: f64) -> f64 {
    4. * PI * r.powi(3) / 3.
}
pub(super) fn add(
    b: &mut StepBuilder,
    part: Part,
    name: &str,
    f: &Frame,
    r: f64,
) -> Result<Authored, AuthorError> {
    let a = b.author();
    let at = built_step::placement_frame(a, step_io_frame(f))?;
    let sphere =
        a.add_spherical_surface("", m::Axis2Placement3dRef::Axis2Placement3d(at), r * 1000.)?;
    let p = a.add_cartesian_point("", ((f.origin - f.axis * r) * 1000.).to_array().into())?;
    let v = a.add_vertex_point("", m::PointRef::CartesianPoint(p))?;
    let l = a.add_vertex_loop("", m::VertexRef::VertexPoint(v))?;
    let bound = a.add_face_bound("", m::LoopRef::VertexLoop(l), true)?;
    let face = a.add_advanced_face(
        "",
        vec![m::FaceBoundRef::FaceBound(bound)],
        m::Ap242AdvancedFaceGeometrySelectRef::SphericalSurface(sphere),
        true,
    )?;
    Ok(Authored(
        Measure::Volume(volume(r)),
        m::RepresentationItemRef::ManifoldSolidBrep(b.solid(part, name, vec![face])?),
    ))
}
fn step_io_frame(f: &Frame) -> built_step::step_io::build::Frame {
    built_step::step_io::build::Frame {
        origin: (f.origin * 1000.).to_array(),
        axis: f.axis.to_array(),
        ref_dir: f.reference.to_array(),
    }
}
