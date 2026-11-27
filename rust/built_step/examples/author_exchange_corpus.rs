// SPDX-License-Identifier: GPL-3.0-or-later
//! Rebuild the authored cavity and display-mesh regression inputs.
use built_step::step_io::{
    build::{CurveInput, FaceBoundInput, Frame, MeshInput, Part, SurfaceInput, VoidShellNormals},
    model::AdvancedFaceId,
    Orientation, StepBuilder,
};
use glam::DVec3;
use std::{collections::HashMap, path::Path};

fn tetra(b: &mut StepBuilder, base: DVec3, size: f64) -> Vec<AdvancedFaceId> {
    let points = [DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::Z].map(|p| base + size * p);
    let vertices = points.map(|p| b.vertex(p.to_array()).unwrap());
    let mut edges = HashMap::new();
    [[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 3]]
        .map(|vs| {
            let x = (points[vs[1]] - points[vs[0]]).normalize();
            let z = x.cross(points[vs[2]] - points[vs[0]]).normalize();
            let uses = vs
                .iter()
                .zip(vs.iter().cycle().skip(1))
                .map(|(&a, &c)| {
                    let key = (a.min(c), a.max(c));
                    let edge = *edges.entry(key).or_insert_with(|| {
                        b.edge(vertices[key.0], vertices[key.1], CurveInput::Line)
                            .unwrap()
                    });
                    (
                        edge,
                        if a < c {
                            Orientation::Forward
                        } else {
                            Orientation::Reversed
                        },
                    )
                })
                .collect();
            let f = Frame {
                origin: points[vs[0]].to_array(),
                axis: z.to_array(),
                ref_dir: x.to_array(),
            };
            b.face(
                SurfaceInput::Plane(f),
                Orientation::Forward,
                vec![FaceBoundInput::outer(uses)],
            )
            .unwrap()
        })
        .to_vec()
}

fn mesh(b: &mut StepBuilder, p: Part) {
    b.mesh(
        p,
        "Display triangle",
        &MeshInput {
            points: vec![[0., 0., 0.], [10., 0., 0.], [0., 10., 0.]],
            triangles: vec![[0, 1, 2]],
            normals: Default::default(),
        },
        None,
    )
    .unwrap();
}
fn main() {
    let dir = std::env::args().nth(1).expect("fixture directory");
    for name in ["cavity", "display-mesh", "mixed-parts"] {
        let mut b = StepBuilder::new();
        let p = b.part(name);
        if name == "display-mesh" {
            mesh(&mut b, p)
        } else {
            let outer = tetra(&mut b, DVec3::ZERO, 20.);
            let inner = tetra(&mut b, DVec3::splat(2.), 3.);
            b.solid_with_voids(
                p,
                "Tetrahedral cavity",
                outer,
                vec![inner],
                VoidShellNormals::TowardMaterial,
            )
            .unwrap();
            if name == "mixed-parts" {
                let second = b.part("Facets");
                mesh(&mut b, second);
                b.place(
                    p,
                    second,
                    Frame {
                        origin: [25., 0., 0.],
                        axis: [0., 0., 1.],
                        ref_dir: [1., 0., 0.],
                    },
                )
                .unwrap();
            }
        }
        b.header_mut().time_stamp = "2026-01-01T00:00:00".into();
        built_step::step_io::write(
            &b.finish(),
            std::fs::File::create(Path::new(&dir).join(format!("{name}.step"))).unwrap(),
        )
        .unwrap();
    }
}
