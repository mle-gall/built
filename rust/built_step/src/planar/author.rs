// SPDX-License-Identifier: GPL-3.0-or-later
//! Reuse geometric support within a body while keeping distinct topological IDs.
use glam::DVec3;
use std::collections::HashMap;
use step_io::{build::Frame, model as m, Ap242Author, AuthorError};

type Coord = [u64; 3];
fn key(p: [f64; 3]) -> Coord {
    p.map(|v| if v == 0. { 0 } else { v.to_bits() })
}
pub(super) struct PlanarAuthor<'a> {
    a: &'a mut Ap242Author,
    vertices: Vec<(m::CartesianPointId, m::VertexPointId)>,
    vectors: HashMap<Coord, m::VectorId>,
    planes: HashMap<(Coord, u64), (m::PlaneId, DVec3)>,
}
impl<'a> PlanarAuthor<'a> {
    pub fn new(a: &'a mut Ap242Author, points: &[DVec3]) -> Result<Self, AuthorError> {
        let mut vertices = Vec::with_capacity(points.len());
        for p in points {
            let point = a.add_cartesian_point("", (*p * 1000.).to_array().into())?;
            let vertex = a.add_vertex_point("", m::PointRef::CartesianPoint(point))?;
            vertices.push((point, vertex));
        }
        Ok(Self {
            a,
            vertices,
            vectors: HashMap::new(),
            planes: HashMap::new(),
        })
    }
    pub fn edge(
        &mut self,
        from: usize,
        to: usize,
        direction: DVec3,
    ) -> Result<m::EdgeCurveId, AuthorError> {
        let xyz = direction.to_array();
        let vector = match self.vectors.entry(key(xyz)) {
            std::collections::hash_map::Entry::Occupied(v) => *v.get(),
            std::collections::hash_map::Entry::Vacant(v) => {
                let dir = self.a.add_direction("", xyz.into())?;
                *v.insert(self.a.add_vector("", m::DirectionRef::Direction(dir), 1.)?)
            }
        };
        let (point, start) = self.vertices[from];
        let line = self.a.add_line(
            "",
            m::CartesianPointRef::CartesianPoint(point),
            m::VectorRef::Vector(vector),
        )?;
        self.a.add_edge_curve(
            "",
            m::VertexRef::VertexPoint(start),
            m::VertexRef::VertexPoint(self.vertices[to].1),
            m::CurveRef::Line(line),
            true,
        )
    }
    pub fn oriented(
        &mut self,
        e: m::EdgeCurveId,
        forward: bool,
    ) -> Result<m::OrientedEdgeRef, AuthorError> {
        self.a
            .add_oriented_edge(
                "",
                m::Ap242OrientedEdgeEdgeElementRef::EdgeCurve(e),
                forward,
            )
            .map(m::OrientedEdgeRef::OrientedEdge)
    }
    pub fn bound(
        &mut self,
        edges: Vec<m::OrientedEdgeRef>,
        outer: bool,
    ) -> Result<m::FaceBoundRef, AuthorError> {
        let ring = m::LoopRef::EdgeLoop(self.a.add_edge_loop("", edges)?);
        if outer {
            self.a
                .add_face_outer_bound("", ring, true)
                .map(m::FaceBoundRef::FaceOuterBound)
        } else {
            self.a
                .add_face_bound("", ring, true)
                .map(m::FaceBoundRef::FaceBound)
        }
    }
    pub fn face(
        &mut self,
        f: &Frame,
        bounds: Vec<m::FaceBoundRef>,
    ) -> Result<m::AdvancedFaceId, AuthorError> {
        let n = DVec3::from(f.axis);
        let p = DVec3::from(f.origin);
        let plane_key = (key(f.axis), key([n.dot(p), 0., 0.])[0]);
        let plane = match self.planes.get(&plane_key) {
            Some(&(id, anchor)) if (p - anchor).dot(n) == 0. => id,
            _ => {
                let frame = crate::placement_frame(self.a, *f)?;
                let id = self
                    .a
                    .add_plane("", m::Axis2Placement3dRef::Axis2Placement3d(frame))?;
                self.planes.insert(plane_key, (id, p));
                id
            }
        };
        self.a.add_advanced_face(
            "",
            bounds,
            m::Ap242AdvancedFaceGeometrySelectRef::Plane(plane),
            true,
        )
    }
}
