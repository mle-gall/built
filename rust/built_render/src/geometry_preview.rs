// SPDX-License-Identifier: GPL-3.0-or-later
//! Transient deformation of loose components and wire endpoints. Retain the
//! exact resident bytes so cancelling restores shading, bounds and geometry.
use super::{Command, Scene};
use glam::{DMat4, DVec3};
use std::collections::HashMap;

type Key = [u32; 3];
fn key(p: DVec3) -> Key {
    p.as_vec3()
        .to_array()
        .map(|v| if v == 0. { 0 } else { v.to_bits() })
}
fn vector(bytes: &[u8]) -> DVec3 {
    glam::Vec3::from_array(bytemuck::pod_read_unaligned(&bytes[..12])).as_dvec3()
}
fn put(bytes: &mut [u8], value: DVec3) {
    bytes[..12].copy_from_slice(bytemuck::bytes_of(&value.as_vec3().to_array()));
}

struct Vertex {
    offset: u64,
    bytes: [u8; 28],
    point: DVec3,
}
struct Edge {
    offset: u64,
    bytes: [u8; 48],
    points: [Option<DVec3>; 2],
}
pub(super) struct GeometryPreview {
    code: usize,
    origin: DVec3,
    bounds: [u8; 32],
    vertices: Vec<Vertex>,
    edges: Vec<Edge>,
}
impl GeometryPreview {
    pub(super) fn begin(
        s: &Scene,
        code: usize,
        vertices: &[u8],
        selected: Vec<[f64; 3]>,
        origin: [f64; 3],
    ) -> Option<Self> {
        if code >= s.p as usize
            || vertices.len() % 28 != 0
            || !origin.into_iter().all(f64::is_finite)
            || !selected.iter().flatten().all(|x| x.is_finite())
        {
            return None;
        }
        let origin = DVec3::from(origin);
        let points: HashMap<_, _> = selected
            .into_iter()
            .map(DVec3::from)
            .map(|p| (key(p - origin), p))
            .collect();
        let cmd = |i: usize| {
            bytemuck::pod_read_unaligned::<Command>(&s.template_data[i * 20..(i + 1) * 20])
        };
        let faces = cmd(code);
        if faces.count as usize != vertices.len() / 28 {
            return None;
        }
        let mut out = Self {
            code,
            origin,
            bounds: s.prototype_data[code * 32..(code + 1) * 32]
                .try_into()
                .ok()?,
            vertices: vec![],
            edges: vec![],
        };
        let face_indices = s
            .index_data
            .get(faces.first as usize * 4..(faces.first as usize + faces.count as usize) * 4)?;
        let mut locations = std::collections::HashSet::new();
        for (v, index) in vertices.chunks_exact(28).zip(face_indices.chunks_exact(4)) {
            let Some(&point) = points.get(&key(vector(v))) else {
                continue;
            };
            let location = (u32::from_le_bytes(index.try_into().ok()?) as i64 + faces.base as i64)
                .checked_mul(28)?;
            if location < 0 || location as usize + 28 > s.vertex_bytes {
                return None;
            }
            if locations.insert(location) {
                out.vertices.push(Vertex {
                    offset: location as u64,
                    bytes: v.try_into().ok()?,
                    point,
                });
            }
        }
        let edges = cmd(code + s.p as usize);
        let first = edges.first as usize / 6 * 48;
        let records = s
            .edge_data
            .get(first..first + edges.count as usize / 6 * 48)?;
        for (i, record) in records.chunks_exact(48).enumerate() {
            let ends = [0, 12].map(|j| points.get(&key(vector(&record[j..]))).copied());
            if ends.iter().any(Option::is_some) {
                out.edges.push(Edge {
                    offset: (first + i * 48) as u64,
                    bytes: record.try_into().ok()?,
                    points: ends,
                });
            }
        }
        (!out.vertices.is_empty() || !out.edges.is_empty()).then_some(out)
    }
    pub(super) fn write(&self, s: &Scene, queue: &wgpu::Queue, matrix: Option<DMat4>) {
        let matrix = matrix.filter(|m| *m != DMat4::IDENTITY);
        let mut bounds = self.bounds;
        let mut lo = vector(&bounds);
        let mut hi = vector(&bounds[16..]);
        let mut moved = |p: DVec3, m: DMat4| {
            let q = m.transform_point3(p) - self.origin;
            lo = lo.min(q);
            hi = hi.max(q);
            q
        };
        // Cofactors give the normal of the transformed winding, including a
        // reflection or a grip pulled down to zero size.
        let normal = |v: DVec3, m: DMat4| {
            let [a, b, c] = [
                m.x_axis.truncate(),
                m.y_axis.truncate(),
                m.z_axis.truncate(),
            ];
            (v.x * b.cross(c) + v.y * c.cross(a) + v.z * a.cross(b)).normalize_or_zero()
                * v.length()
        };
        let mut writes = Writes::default();
        for v in &self.vertices {
            let mut bytes = v.bytes;
            if let Some(m) = matrix {
                put(&mut bytes, moved(v.point, m));
                put(&mut bytes[12..], normal(vector(&v.bytes[12..]), m));
            }
            writes.push(queue, &s.vertex, v.offset, &bytes);
        }
        writes.flush(queue, &s.vertex);
        for e in &self.edges {
            let mut bytes = e.bytes;
            if let Some(m) = matrix {
                for (i, p) in e.points.iter().enumerate() {
                    if let Some(p) = p {
                        put(&mut bytes[i * 12..], moved(*p, m));
                    }
                }
                if e.points.iter().all(Option::is_some) {
                    for i in [24, 36] {
                        put(&mut bytes[i..], normal(vector(&e.bytes[i..]), m));
                    }
                }
            }
            writes.push(queue, &s.edge_records, e.offset, &bytes);
        }
        writes.flush(queue, &s.edge_records);
        if matrix.is_some() {
            put(&mut bounds, lo);
            put(&mut bounds[16..], hi);
        }
        queue.write_buffer(&s.protos, (self.code * 32) as u64, &bounds);
    }
}
#[derive(Default)]
struct Writes {
    offset: u64,
    bytes: Vec<u8>,
}
impl Writes {
    fn push(&mut self, queue: &wgpu::Queue, buffer: &wgpu::Buffer, at: u64, data: &[u8]) {
        if at != self.offset + self.bytes.len() as u64 {
            self.flush(queue, buffer);
        }
        if self.bytes.is_empty() {
            self.offset = at
        }
        self.bytes.extend_from_slice(data)
    }
    fn flush(&mut self, queue: &wgpu::Queue, buffer: &wgpu::Buffer) {
        if !self.bytes.is_empty() {
            queue.write_buffer(buffer, self.offset, &self.bytes);
            self.bytes.clear();
        }
    }
}
