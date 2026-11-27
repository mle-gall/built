// SPDX-License-Identifier: GPL-3.0-or-later
//! Piecewise planar solid representations. Coordinates at this boundary are metres.
use glam::DVec3;
use std::collections::{HashMap, HashSet};
mod author;
use step_io::build::{Frame, Part};
use step_io::{
    scene::{CurveKind, Solid, SurfaceKind},
    Orientation, StepBuilder,
};

#[derive(Default, Debug)]
pub struct Body {
    pub points: Vec<DVec3>,
    /// Outer loop followed by holes. Vertex indices run in boundary-use order.
    pub faces: Vec<Vec<Vec<usize>>>,
}

impl Body {
    pub fn plane(&self, rings: &[Vec<usize>]) -> Result<(Frame, f64), String> {
        if rings.is_empty() {
            return Err("face without bounds".into());
        }
        let mut area = DVec3::ZERO;
        let mut axes = None;
        let mut seen = HashSet::new();
        let mut origin = DVec3::ZERO;
        let mut volume = 0.;
        for ring in rings {
            if ring.len() < 3 {
                return Err("short planar boundary".into());
            }
            for &i in ring {
                if self.points.get(i).is_none_or(|p| !p.is_finite()) || !seen.insert(i) {
                    return Err("invalid or repeated boundary vertex".into());
                }
            }
            let p = self.points[ring[0]];
            if axes.is_none() {
                origin = p;
            }
            let mut normal = DVec3::ZERO;
            for pair in ring[1..].windows(2) {
                normal += (self.points[pair[0]] - p).cross(self.points[pair[1]] - p);
            }
            if axes.is_none() {
                let z = normal.try_normalize().ok_or("zero area face")?;
                let x = (self.points[ring[1]] - p)
                    .try_normalize()
                    .ok_or("zero length edge")?;
                axes = Some((z, x));
            } else if normal.dot(axes.unwrap().0) >= 0. {
                return Err("incorrect hole winding".into());
            }
            area += normal;
            volume += (p - self.points[0]).dot(normal) / 6.;
        }
        let (z, x) = axes.unwrap();
        if area.dot(z) <= 0.
            || seen
                .iter()
                .any(|&i| (self.points[i] - origin).dot(z).abs() > 1e-9)
        {
            return Err("invalid planar face".into());
        }
        Ok((
            Frame {
                origin: (origin * 1000.).to_array(),
                axis: z.to_array(),
                ref_dir: x.to_array(),
            },
            volume,
        ))
    }

    pub fn add(
        &self,
        b: &mut StepBuilder,
        part: Part,
        name: &str,
    ) -> Result<crate::Authored, String> {
        let mut author =
            author::PlanarAuthor::new(b.author(), &self.points).map_err(|e| e.to_string())?;
        let mut edges = HashMap::new();
        let mut faces = Vec::new();
        let mut volume = 0.;
        let mut adjacent = vec![vec![]; self.faces.len()];
        for (i, rings) in self.faces.iter().enumerate() {
            let (plane, v) = self.plane(rings)?;
            volume += v;
            let mut bounds = Vec::new();
            for (j, ring) in rings.iter().enumerate() {
                let mut uses = Vec::new();
                for (&from, &to) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                    let key = (from.min(to), from.max(to));
                    let (edge, sense) = match edges.entry(key) {
                        std::collections::hash_map::Entry::Vacant(v) => {
                            let direction = (self.points[to] - self.points[from])
                                .try_normalize()
                                .ok_or("zero length edge")?;
                            let e = author
                                .edge(from, to, direction)
                                .map_err(|e| e.to_string())?;
                            v.insert((e, from, i, false));
                            (e, Orientation::Forward)
                        }
                        std::collections::hash_map::Entry::Occupied(mut o) => {
                            let (e, start, face, closed) = o.get_mut();
                            if *closed || *start == from {
                                return Err("inconsistent edge incidence".into());
                            }
                            *closed = true;
                            adjacent[*face].push(i);
                            adjacent[i].push(*face);
                            (*e, Orientation::Reversed)
                        }
                    };
                    uses.push(
                        author
                            .oriented(edge, sense == Orientation::Forward)
                            .map_err(|e| e.to_string())?,
                    );
                }
                bounds.push(author.bound(uses, j == 0).map_err(|e| e.to_string())?);
            }
            faces.push(author.face(&plane, bounds).map_err(|e| e.to_string())?);
        }
        if faces.is_empty() {
            return Err("empty shell".into());
        }
        let closed = edges.values().all(|r| r.3);
        if closed && (!volume.is_finite() || volume <= 0.) {
            return Err("invalid closed shell orientation or volume".into());
        }
        let mut reached = HashSet::new();
        let mut stack = vec![0];
        while let Some(i) = stack.pop() {
            if reached.insert(i) {
                stack.extend(&adjacent[i]);
            }
        }
        if reached.len() != faces.len() {
            return Err("multiple shells in one planar body".into());
        }
        if closed {
            let solid = b.solid(part, name, faces).map_err(|e| e.to_string())?;
            Ok(crate::Authored(
                crate::Measure::Volume(volume),
                step_io::model::RepresentationItemRef::ManifoldSolidBrep(solid),
            ))
        } else {
            let sheet = crate::sheet::item(b.author(), name, faces).map_err(|e| e.to_string())?;
            Ok(crate::Authored(
                crate::Measure::Area(self.area()?),
                step_io::model::RepresentationItemRef::ShellBasedSurfaceModel(sheet),
            ))
        }
    }
    pub fn area(&self) -> Result<f64, String> {
        let mut area = 0.;
        for rings in &self.faces {
            let (p, _) = self.plane(rings)?;
            let z = DVec3::from(p.axis);
            for ring in rings {
                let origin = self.points[ring[0]];
                for pair in ring[1..].windows(2) {
                    area += 0.5
                        * z.dot(
                            (self.points[pair[0]] - origin).cross(self.points[pair[1]] - origin),
                        );
                }
            }
        }
        Ok(area)
    }

    pub fn from_solid(solid: Solid<'_>, metres: f64) -> Result<Self, String> {
        let (shell, sense) = solid.outer().ok_or("missing outer shell")?;
        if !shell.is_closed() || solid.voids().next().is_some() {
            return Err("unsupported shell arrangement".into());
        }
        Self::from_shell(shell, sense, metres)
    }
    pub fn from_shell(
        shell: step_io::scene::Shell<'_>,
        sense: Orientation,
        metres: f64,
    ) -> Result<Self, String> {
        let mut body = Body::default();
        let mut vertices = HashMap::new();
        for face in shell.faces() {
            if !matches!(face.surface().kind(), SurfaceKind::Plane(_)) {
                return Err("nonplanar support".into());
            }
            let mut rings = Vec::new();
            let mut outer = None;
            for bound in face.bounds() {
                let mut ring = Vec::new();
                let mut ends = Vec::new();
                for (edge, orientation) in bound.oriented_edges() {
                    let curve = edge.curve();
                    if !matches!(curve.curve_3d().unwrap_or(curve).kind(), CurveKind::Line(_)) {
                        return Err("nonlinear boundary".into());
                    }
                    let a = edge.start().ok_or("missing start vertex")?;
                    let b = edge.end().ok_or("missing end vertex")?;
                    let (a, b) = if orientation == Orientation::Forward {
                        (a, b)
                    } else {
                        (b, a)
                    };
                    let p = a.point().ok_or("missing point")?;
                    let index = *vertices.entry(a.key()).or_insert_with(|| {
                        body.points.push(DVec3::from(p.xyz()) * metres);
                        body.points.len() - 1
                    });
                    ring.push(index);
                    ends.push((a.key(), b.key()));
                }
                if ends.is_empty()
                    || ends
                        .iter()
                        .zip(ends.iter().cycle().skip(1))
                        .any(|(a, b)| a.1 != b.0)
                {
                    return Err("discontinuous boundary".into());
                }
                if bound.orientation() != sense {
                    ring.reverse();
                }
                if bound.is_outer() {
                    if outer.replace(rings.len()).is_some() {
                        return Err("multiple outer bounds".into());
                    }
                }
                rings.push(ring);
            }
            let first = outer
                .or_else(|| (rings.len() == 1).then_some(0))
                .ok_or("ambiguous outer boundary")?;
            rings.swap(0, first);
            body.plane(&rings)?;
            body.faces.push(rings);
        }
        if body.faces.is_empty() {
            return Err("empty shell".into());
        }
        Ok(body)
    }

    pub fn volume(&self) -> Result<f64, String> {
        self.faces
            .iter()
            .map(|r| self.plane(r).map(|(_, v)| v))
            .sum()
    }
}
