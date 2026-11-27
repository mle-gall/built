// SPDX-License-Identifier: GPL-3.0-or-later
//! Form one trimmed face per connected smooth ruled band. Unwrap each
//! neighbour across its common generator; the remaining period cut keeps
//! both directed uses of that shared edge.
use crate::{
    exact::{
        patch::{Coedge, Edge, Face, Topology},
        Curve, Frame, Rows, Surface,
    },
    store::{keys::Kind, native::NativePatch, source_arcs::Strip, Store},
};
use glam::{DVec2, DVec3};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    f64::consts::TAU,
};

type Result<T> = std::result::Result<T, &'static str>;
const TOL: f64 = 1e-8;
struct Quad {
    strip: Strip,
    v: [u32; 4],
    e: [u32; 4],
}
struct Builder<'a> {
    st: &'a Store,
    rows: Rows,
    t: Topology,
    edges: HashMap<u32, (u64, bool)>,
}
impl Builder<'_> {
    fn point(&self, v: u32) -> DVec3 {
        self.st
            .state
            .frame
            .definition_point(*self.st.state.vpos.get(v as usize))
            .into()
    }
    fn edge(&mut self, e: u32, from: u32) -> Result<(u64, bool)> {
        let ends = *self.st.state.ends.get(e as usize);
        if let Some(&(id, rev)) = self.edges.get(&e) {
            return Ok((id, (ends[0] != from) ^ rev));
        }
        let pure = self.st.state.exact.arcs.as_ref().and_then(|a| a.get(&e));
        let (curve, mut interval) = if let Some(a) = pure {
            (
                Curve::Circle {
                    frame: (*a.frame).clone(),
                    radius: a.radius,
                },
                a.interval,
            )
        } else {
            let p = self.point(ends[0]);
            let delta = self.point(ends[1]) - p;
            (
                Curve::Line {
                    origin: p,
                    direction: delta.try_normalize().ok_or("zero generator")?,
                },
                [0., delta.length()],
            )
        };
        let rev = interval[0] > interval[1];
        let mut vs = ends.map(u64::from);
        if rev {
            interval.swap(0, 1);
            vs.swap(0, 1);
        }
        for v in ends {
            let p = self.point(v);
            self.t.vertices.insert(v.into(), p);
        }
        let id = self.t.edges.len() as u64;
        self.t.edges.push(Edge {
            id,
            vertices: vs,
            curve: self.rows.curves.len() as u32,
            interval,
        });
        self.rows.curves.push(curve);
        self.edges.insert(e, (id, rev));
        Ok((id, (ends[0] != from) ^ rev))
    }
    fn face(&mut self, surface: Surface, loops: Vec<Vec<Coedge>>, reversed: bool) {
        self.t.faces.push(Face {
            id: self.t.faces.len() as u64,
            surface: self.rows.surfaces.len() as u32,
            loops,
            reversed,
        });
        self.rows.surfaces.push(surface);
    }
    fn plane(&mut self, f: u32) -> Result<()> {
        let rings = self.st.face_rings(f);
        let r = rings
            .first()
            .filter(|r| r.len() >= 3)
            .ok_or("invalid planar boundary")?;
        let o = self.point(r[0]);
        let axis = r[1..]
            .windows(2)
            .map(|v| (self.point(v[0]) - o).cross(self.point(v[1]) - o))
            .sum::<DVec3>()
            .try_normalize()
            .ok_or("degenerate planar face")?;
        let reference = (self.point(r[1]) - o)
            .try_normalize()
            .ok_or("short planar edge")?;
        let frame = Frame {
            origin: o,
            axis,
            reference,
        };
        let project = |p: DVec3| DVec2::new((p - o).dot(frame.reference), (p - o).dot(frame.y()));
        let mut loops = Vec::new();
        let mut curved = false;
        for r in rings {
            let mut ring = Vec::new();
            for (&v, &w) in r.iter().zip(r.iter().cycle().skip(1)) {
                let e = self.st.find_edge(v, w).ok_or("missing planar edge")?;
                let (id, reversed) = self.edge(e, v)?;
                curved |= self
                    .st
                    .state
                    .exact
                    .arcs
                    .as_ref()
                    .is_some_and(|a| a.contains_key(&e));
                ring.push(Coedge {
                    edge: Some(id),
                    pole: None,
                    reversed,
                    projected: false,
                    uv: [project(self.point(v)), project(self.point(w))],
                    tolerance: TOL,
                });
            }
            loops.push(ring);
        }
        if curved {
            for u in loops.iter_mut().flatten() {
                u.projected = true;
            }
        }
        self.face(Surface::Plane { frame }, loops, false);
        Ok(())
    }
    fn band(
        &mut self,
        root: u32,
        quads: &HashMap<u32, Quad>,
        visited: &mut HashSet<u32>,
    ) -> Result<()> {
        let strip = &quads[&root].strip;
        let mut shifts = BTreeMap::from([(root, 0_i64)]);
        let mut queue = std::collections::VecDeque::from([root]);
        while let Some(f) = queue.pop_front() {
            visited.insert(f);
            let q = &quads[&f];
            for (i, &e) in q.e.iter().enumerate() {
                for &g in self.st.edge_faces(e) {
                    let Some(other) = quads.get(&g) else { continue };
                    if shifts.contains_key(&g) || other.strip.pair != strip.pair {
                        continue;
                    }
                    let j = other
                        .v
                        .iter()
                        .position(|&v| v == q.v[i])
                        .ok_or("missing band endpoint")?;
                    let step = ((q.strip.uv[i][0] - other.strip.uv[j][0]) / TAU).round() as i64;
                    shifts.insert(g, shifts[&f] + step);
                    queue.push_back(g);
                }
            }
        }
        let chart = |f: u32, i: usize| {
            let q = &quads[&f];
            let mut uv = DVec2::from(q.strip.uv[i]);
            uv.x += shifts[&f] as f64 * TAU;
            let p = self.point(q.v[i]) - strip.frame.origin;
            let theta = p.dot(strip.frame.y()).atan2(p.dot(strip.frame.reference));
            let turn = ((uv.x - theta) / TAU).round() as i64;
            (
                (q.v[i], turn),
                DVec2::new(
                    theta + turn as f64 * TAU,
                    if strip.swept.is_some() {
                        q.strip.uv[i][1]
                    } else {
                        p.dot(strip.frame.axis) / strip.semi_angle.cos()
                    },
                ),
            )
        };
        // Collect before mutating the output pools. Store vertices at each
        // period cut have distinct UV vertices but identical spatial ids.
        let mut perimeter = BTreeMap::new();
        for (&f, _) in &shifts {
            let q = &quads[&f];
            for i in 0..4 {
                let e = q.e[i];
                let (a, uv) = chart(f, i);
                let (b, end) = chart(f, (i + 1) % 4);
                let inside = self.st.edge_faces(e).iter().any(|&g| {
                    if g == f || !shifts.contains_key(&g) {
                        return false;
                    }
                    let other = &quads[&g];
                    other
                        .e
                        .iter()
                        .position(|&x| x == e)
                        .is_some_and(|j| chart(g, j).0 == b && chart(g, (j + 1) % 4).0 == a)
                });
                if !inside && perimeter.insert(a, (b, e, [uv, end])).is_some() {
                    return Err("ambiguous ruled trim junction");
                }
            }
        }
        let mut loops = Vec::new();
        while let Some((&start, _)) = perimeter.first_key_value() {
            let mut current = start;
            let mut ring = Vec::new();
            loop {
                let (next, e, uv) = perimeter.remove(&current).ok_or("open ruled trim")?;
                let (id, reversed) = self.edge(e, current.0)?;
                ring.push(Coedge {
                    edge: Some(id),
                    pole: None,
                    reversed,
                    projected: false,
                    uv,
                    tolerance: TOL,
                });
                current = next;
                if current == start {
                    break;
                }
            }
            loops.push(ring);
        }
        let area = |r: &Vec<Coedge>| {
            r.iter()
                .map(|c| (c.uv[0] - r[0].uv[0]).perp_dot(c.uv[1] - r[0].uv[0]))
                .sum::<f64>()
        };
        loops.sort_by(|a, b| area(b).abs().total_cmp(&area(a).abs()));
        let reversed = area(loops.first().ok_or("empty ruled band")?) < 0.;
        let surface = if let Some(surface) = &strip.swept {
            surface.clone()
        } else if strip.semi_angle.abs() < 1e-13 {
            Surface::Cylinder {
                frame: strip.frame.clone(),
                radius: strip.radius,
            }
        } else {
            Surface::Cone {
                frame: strip.frame.clone(),
                radius: strip.radius,
                half_angle: strip.semi_angle,
            }
        };
        self.face(surface, loops, reversed);
        Ok(())
    }
}
pub(super) fn recover(st: &Store) -> Result<Vec<NativePatch>> {
    if st
        .live_slots(Kind::Edge)
        .into_iter()
        .any(|e| !matches!(st.edge_faces(e).len(), 1 | 2))
    {
        return Err("loose or nonmanifold edge in bound geometry");
    }
    let mut remaining: HashSet<_> = st.live_slots(Kind::Face).into_iter().collect();
    let mut patches = Vec::new();
    for root in st.live_slots(Kind::Face) {
        if !remaining.remove(&root) {
            continue;
        }
        let mut faces = vec![root];
        let mut i = 0;
        while i < faces.len() {
            for (e, _) in st.boundary_edges(faces[i]).into_iter().flatten().flatten() {
                faces.extend(st.edge_faces(e).into_iter().filter(|g| remaining.remove(g)));
            }
            i += 1;
        }
        patches.push(component(st, faces)?);
    }
    Ok(patches)
}

fn component(st: &Store, faces: Vec<u32>) -> Result<NativePatch> {
    let mut b = Builder {
        st,
        rows: Rows::default(),
        t: Topology::default(),
        edges: HashMap::new(),
    };
    b.t.tolerance = TOL;
    b.t.display_tolerance = Some(super::DISPLAY_DEFLECTION);
    let mut quads = HashMap::new();
    for &f in &faces {
        if let Some(strip) = st.source_strip(f) {
            let v: [u32; 4] = st.face_rings(f)[0]
                .as_slice()
                .try_into()
                .map_err(|_| "invalid ruled face")?;
            let mut e = [0; 4];
            for i in 0..4 {
                e[i] = st
                    .find_edge(v[i], v[(i + 1) % 4])
                    .ok_or("missing ruled edge")?;
            }
            quads.insert(f, Quad { strip, v, e });
        }
    }
    let mut visited = HashSet::new();
    for f in faces {
        if quads.contains_key(&f) {
            if !visited.contains(&f) {
                b.band(f, &quads, &mut visited)?;
            }
        } else {
            b.plane(f)?;
        }
    }
    b.rows.topology = Some(b.t);
    NativePatch::new(b.rows)
}
