// SPDX-License-Identifier: GPL-3.0-or-later
//! Read-only, unwelded neighbourhood for planning a materialised mesh edit.
use super::fillet::{self, FilletMesh, RoundPlan};
use super::geometry::{Hypot, V3};
use pyo3::{exceptions::PyValueError, prelude::*};

pub(super) type Vertex = ([f64; 3], Vec<u32>, Vec<u32>); // position, disk, adjacent faces
pub(super) type Edge = ([u32; 2], Vec<u32>); // endpoints, radial
pub(super) type Face = (Vec<Vec<u32>>, [f64; 3]); // rings, normal

pub(super) struct Incidence {
    vertices: Vec<Vertex>,
    edges: Vec<Edge>,
    faces: Vec<Face>,
    compensated: bool,
}

impl Incidence {
    pub fn new(
        vertices: Vec<Vertex>,
        edges: Vec<Edge>,
        faces: Vec<Face>,
        compensated: bool,
    ) -> PyResult<Self> {
        let vertex = |&v: &u32| (v as usize) < vertices.len();
        let face = |&f: &u32| (f as usize) < faces.len();
        let edge = |&e: &u32| (e as usize) < edges.len();
        if !vertices
            .iter()
            .all(|(_, disk, fs)| disk.iter().all(edge) && fs.iter().all(face))
            || !edges
                .iter()
                .all(|(vs, fs)| vs.iter().all(vertex) && fs.iter().all(face))
            || !faces.iter().all(|(rs, _)| {
                !rs.is_empty() && rs.iter().all(|r| r.len() >= 3 && r.iter().all(vertex))
            })
        {
            return Err(PyValueError::new_err("invalid fillet incidence"));
        }
        Ok(Self {
            vertices,
            edges,
            faces,
            compensated,
        })
    }

    pub fn plan(
        &self,
        count: usize,
        size: f64,
        segments: usize,
        profile: f64,
        measure: &str,
        cut: bool,
        outline: &[(f64, f64)],
        mirror: bool,
        steps: bool,
    ) -> PyResult<RoundPlan> {
        if count > self.edges.len()
            || count > u32::MAX as usize
            || segments == 0
            || outline.len() == 1
        {
            return Err(PyValueError::new_err(
                "invalid fillet selection or segment count",
            ));
        }
        fillet::plan(
            self,
            (0..count as u32).collect(),
            size,
            segments,
            profile,
            measure,
            cut,
            outline,
            mirror,
            steps,
            Hypot::LibCxxFma,
        )
        .map_err(|(message, p)| PyValueError::new_err((message, p.map(fillet::xyz))))
    }
}

fn vector([x, y, z]: [f64; 3]) -> V3 {
    V3(x, y, z)
}

impl FilletMesh for Incidence {
    const OBJECT: bool = true;
    fn compensated_sum(&self) -> bool {
        self.compensated
    }
    fn point(&self, v: u32) -> V3 {
        vector(self.vertices[v as usize].0)
    }
    fn disk(&self, v: u32) -> Vec<u32> {
        self.vertices[v as usize].1.clone()
    }
    fn adjacent(&self, v: u32) -> Vec<u32> {
        self.vertices[v as usize].2.clone()
    }
    fn ends(&self, e: u32) -> [u32; 2] {
        self.edges[e as usize].0
    }
    fn radial(&self, e: u32) -> &[u32] {
        &self.edges[e as usize].1
    }
    fn rings(&self, f: u32) -> Vec<Vec<u32>> {
        self.faces[f as usize].0.clone()
    }
    fn normal(&self, f: u32, _h: Hypot) -> V3 {
        vector(self.faces[f as usize].1)
    }
}
