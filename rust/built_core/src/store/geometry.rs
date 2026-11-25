// SPDX-License-Identifier: GPL-3.0-or-later
//! Double precision geometry for model operations. The viewport keeps its
//! own float32 vector type and the two share the projected polygon kernel.
pub(crate) use crate::facegeom::{earcut, is_convex, py_sum, Hypot};

#[derive(Clone, Copy)]
pub struct V3(pub f64, pub f64, pub f64);

impl V3 {
    #[inline]
    pub(crate) fn add(self, o: V3) -> V3 {
        V3(self.0 + o.0, self.1 + o.1, self.2 + o.2)
    }
    #[inline]
    pub fn sub(self, o: V3) -> V3 {
        V3(self.0 - o.0, self.1 - o.1, self.2 - o.2)
    }
    #[inline]
    pub(crate) fn mul(self, s: f64) -> V3 {
        V3(self.0 * s, self.1 * s, self.2 * s)
    }
    #[inline]
    pub(crate) fn div(self, s: f64) -> V3 {
        V3(self.0 / s, self.1 / s, self.2 / s)
    }
    #[inline]
    pub fn dot(self, other: V3, _h: Hypot) -> f64 {
        self.vector().dot(other.vector())
    }

    pub fn cross(self, other: V3, _h: Hypot) -> V3 {
        V3::from_vector(self.vector().cross(other.vector()))
    }

    fn vector(self) -> glam::DVec3 {
        glam::DVec3::new(self.0, self.1, self.2)
    }

    fn from_vector(v: glam::DVec3) -> Self {
        Self(v.x, v.y, v.z)
    }
}

#[inline]
pub fn length(v: V3, _h: Hypot) -> f64 {
    v.vector().length()
}

pub fn normalized(v: V3, _h: Hypot) -> V3 {
    V3::from_vector(v.vector().normalize_or_zero())
}

/// `built_core.triangulate.plane_axes`.
pub fn plane_axes(normal: V3, h: Hypot) -> (V3, V3) {
    let n = normalized(normal, h);
    let mut r = V3(1.0, 0.0, 0.0);
    let mut u = r.sub(n.mul(r.dot(n, h)));
    if length(u, h) < 0.1 {
        r = V3(0.0, 1.0, 0.0);
        u = r.sub(n.mul(r.dot(n, h)));
    }
    let u = normalized(u, h);
    let v = normalized(n.cross(u, h), h);
    (u, v)
}

pub fn face_normal(pos: &[V3], outer: &[u32], h: Hypot) -> V3 {
    face_normal_length(pos, outer, h).0
}

pub(crate) fn face_normal_length(pos: &[V3], outer: &[u32], h: Hypot) -> (V3, f64) {
    let raw = newell(pos, outer);
    let len = length(raw, h);
    if len < 1e-9 {
        (V3(0.0, 0.0, 1.0), len)
    } else {
        (raw.div(len), len)
    }
}

/// Arithmetic mean of the outer ring in model coordinates.
pub fn face_centroid(pos: &[V3], outer: &[u32], compensated: bool) -> V3 {
    let n = outer.len();
    if n == 0 {
        return V3(0.0, 0.0, 0.0);
    }
    let c = n as f64;
    let x = py_sum(outer.iter().map(|&i| pos[i as usize].0), compensated) / c;
    let y = py_sum(outer.iter().map(|&i| pos[i as usize].1), compensated) / c;
    let z = py_sum(outer.iter().map(|&i| pos[i as usize].2), compensated) / c;
    V3(x, y, z)
}

pub(crate) fn plane_basis(normal: V3, h: Hypot) -> (V3, V3) {
    let n = normalized(normal, h);
    let mut r = V3(1.0, 0.0, 0.0);
    if (r.dot(n, h)).abs() > 0.9 {
        r = V3(0.0, 1.0, 0.0);
    }
    let u = normalized(r.sub(n.mul(r.dot(n, h))), h);
    let v = normalized(n.cross(u, h), h);
    (u, v)
}

pub(crate) fn triangles(pos: &[V3], rings: &[&[u32]], normal: V3, h: Hypot) -> Vec<[u32; 3]> {
    let outer = rings[0];
    let m = outer.len();
    let holes: Vec<&[u32]> = rings[1..]
        .iter()
        .copied()
        .filter(|r| r.len() >= 3)
        .collect();
    let (u, v) = plane_axes(normal, h);
    let origin = pos[outer[0] as usize];
    let proj = |i: u32| {
        let rel = pos[i as usize].sub(origin);
        (rel.dot(u, h), rel.dot(v, h))
    };
    let mut tris = Vec::new();
    if holes.is_empty() {
        let poly: Vec<(f64, f64)> = outer.iter().map(|&i| proj(i)).collect();
        if is_convex(&poly) {
            for k in 1..m - 1 {
                tris.push([outer[0], outer[k], outer[k + 1]]);
            }
            return tris;
        }
    }
    let mut master: Vec<u32> = outer.to_vec();
    let mut pts: Vec<(f64, f64)> = outer.iter().map(|&i| proj(i)).collect();
    let mut hole_idx = Vec::new();
    for hr in &holes {
        hole_idx.push(master.len());
        master.extend_from_slice(hr);
        pts.extend(hr.iter().map(|&i| proj(i)));
    }
    let cut = earcut(&pts, &hole_idx);
    for t in cut {
        tris.push([master[t[0]], master[t[1]], master[t[2]]]);
    }
    tris
}

fn newell(pos: &[V3], outer: &[u32]) -> V3 {
    let m = outer.len();
    let (mut nx, mut ny, mut nz) = (0.0f64, 0.0f64, 0.0f64);
    for k in 0..m {
        let c = pos[outer[k] as usize];
        let n = pos[outer[(k + 1) % m] as usize];
        let (x0, y0, z0) = (c.0, c.1, c.2);
        let (x1, y1, z1) = (n.0, n.1, n.2);
        nx += (y0 - y1) * (z0 + z1);
        ny += (z0 - z1) * (x0 + x1);
        nz += (x0 - x1) * (y0 + y1);
    }
    V3(nx, ny, nz)
}

impl From<V3> for [f64; 3] {
    fn from(v: V3) -> Self {
        [v.0, v.1, v.2]
    }
}

pub(crate) fn face_geometry(
    pos: &[V3],
    rings: &[&[u32]],
    h: Hypot,
    compensated: bool,
) -> crate::facegeom::FaceGeom<V3, f64> {
    let outer = rings[0];
    let raw = newell(pos, outer);
    let ln = length(raw, h);
    let normal = if ln > 1e-9 {
        raw.div(ln)
    } else {
        V3(0., 0., 1.)
    };
    let unit = if ln > 1e-9 {
        normalized(raw, h)
    } else {
        normal
    };
    let valid = outer.len() >= 3;
    let tris = if valid {
        triangles(pos, rings, normal, h)
    } else {
        Vec::new()
    };
    let tris_unit =
        (valid && unit.vector() != normal.vector()).then(|| triangles(pos, rings, unit, h));
    crate::facegeom::FaceGeom {
        normal,
        unit,
        ln,
        tris,
        tris_unit,
        centroid: valid.then(|| face_centroid(pos, outer, compensated)),
    }
}
