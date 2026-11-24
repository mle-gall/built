// SPDX-License-Identifier: GPL-3.0-or-later
//! Build a transient mesh from detached clipboard placements. Use the resident
//! bake's visible index ranges, two-sided materials and nested placements.
//! Only the translation uniform changes during the gesture.
use glam::{Mat3, Mat4, Vec3};
use numpy::{IntoPyArray, PyArrayMethods};
use pyo3::{exceptions::PyValueError, prelude::*, types::PyBytes};
use std::collections::BTreeMap;

fn invalid() -> PyErr {
    PyValueError::new_err("invalid clipboard scene pack")
}

fn bytes<'py>(pack: &Bound<'py, PyAny>, name: &str) -> PyResult<Bound<'py, PyBytes>> {
    Ok(pack.getattr(name)?.call_method0("tobytes")?.cast_into()?)
}
fn record<const N: usize>(data: &[u8], index: usize) -> PyResult<&[u8; N]> {
    data.chunks_exact(N)
        .nth(index)
        .and_then(|r| r.try_into().ok())
        .ok_or_else(invalid)
}
fn word(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(data[at..at + 4].try_into().unwrap())
}
fn vec3(data: &[u8]) -> Vec3 {
    Vec3::from_array(bytemuck::pod_read_unaligned(&data[..12]))
}

struct Geometry<'a> {
    vertices: &'a [u8],
    indices: &'a [u8],
    materials: &'a [u8],
    textures: Vec<String>,
}
#[derive(Clone, Copy, Default)]
struct Corner {
    p: Vec3,
    n: Vec3,
    color: [[u8; 4]; 2],
    uv: [[f32; 2]; 2],
    texture: [u32; 2],
}
struct Placement {
    world: Mat4,
    normals: Mat3,
    paint: [u8; 4],
    mirrored: bool,
    billboard: bool,
}
impl Geometry<'_> {
    fn vertex(&self, command: &[u8; 20], corner: usize) -> PyResult<(usize, &[u8; 28])> {
        let index = record::<4>(self.indices, word(command, 8) as usize + corner)?;
        let vertex = i64::from(u32::from_le_bytes(*index)) + i64::from(word(command, 12) as i32);
        let vertex = usize::try_from(vertex).map_err(|_| invalid())?;
        Ok((vertex, record(self.vertices, vertex)?))
    }
    fn face_corner(
        &self,
        placement: &Placement,
        header: &[u8; 16],
        command: &[u8; 20],
        i: usize,
    ) -> PyResult<Corner> {
        let (index, vertex) = self.vertex(command, i)?;
        let front = u32::from_le_bytes(vertex[24..28].try_into().unwrap()).to_le_bytes();
        let mut c = Corner {
            p: placement.world.transform_point3(vec3(vertex)),
            n: (placement.normals * vec3(&vertex[12..])).normalize_or_zero(),
            color: [front, [0; 4]],
            ..Corner::default()
        };
        let count = word(header, 8) as usize;
        if count > 0 {
            let offset = index
                .checked_sub(word(header, 0) as usize)
                .filter(|v| *v < count)
                .ok_or_else(invalid)?;
            let at = word(header, 4) as usize + offset * 2;
            c.uv = bytemuck::pod_read_unaligned(record::<16>(self.materials, at)?);
            let mat = record::<16>(self.materials, at + 1)?;
            c.color[1] = word(mat, 0).to_le_bytes();
            c.texture = [word(mat, 4), word(mat, 8)];
        }
        if word(header, 12) & 1 != 0 {
            c.color[1] = front;
        }
        if front[3] == 0 && placement.paint[3] != 0 {
            c.color[0] = placement.paint;
            if c.color[1][3] == 0 {
                c.color[1] = placement.paint;
            }
        }
        // Alpha zero requests the current style's default front/back colour.
        if c.color[0][3] == 0 {
            c.color[0] = [0; 4];
        }
        if c.color[1][3] == 0 {
            c.color[1] = [255, 0, 0, 0];
        }
        Ok(c)
    }
}

#[derive(Default)]
struct Output {
    triangles: Vec<u8>,
    edges: Vec<u8>,
    images: BTreeMap<(u32, u32, u8), Vec<f32>>,
}
fn push(v: &mut Vec<u8>, p: Vec3, n: Vec3, color: [u8; 4]) {
    v.extend_from_slice(bytemuck::bytes_of(&p.to_array()));
    v.extend_from_slice(bytemuck::bytes_of(&n.to_array()));
    v.extend_from_slice(&color);
}
impl Output {
    fn faces(
        &mut self,
        geo: &Geometry<'_>,
        placement: &Placement,
        header: &[u8; 16],
        command: &[u8; 20],
    ) -> PyResult<()> {
        let count = word(command, 0) as usize;
        if count % 3 != 0 {
            return Err(invalid());
        }
        let light = Vec3::new(0.35, 0.25, 1.).normalize();
        for i in (0..count).step_by(3) {
            let tri = [
                geo.face_corner(placement, header, command, i)?,
                geo.face_corner(placement, header, command, i + 1)?,
                geo.face_corner(placement, header, command, i + 2)?,
            ];
            for side in 0..2 {
                let order = if (side == 1) ^ placement.mirrored {
                    [0, 2, 1]
                } else {
                    [0, 1, 2]
                };
                let texture = tri[0].texture[side];
                if texture == 0 {
                    for j in order {
                        let c = tri[j];
                        push(&mut self.triangles, c.p, c.n, c.color[side]);
                    }
                } else {
                    if texture as usize > geo.textures.len() {
                        return Err(invalid());
                    }
                    let shade = if placement.billboard {
                        1.
                    } else {
                        0.62 + 0.38 * tri[0].n.dot(light).abs()
                    };
                    let alpha = tri[0].color[side][3];
                    let image = self
                        .images
                        .entry((
                            texture,
                            shade.to_bits(),
                            if alpha == 0 { 255 } else { alpha },
                        ))
                        .or_default();
                    for j in order {
                        image.extend(tri[j].p.to_array());
                        image.extend(tri[j].uv[side]);
                    }
                }
            }
        }
        Ok(())
    }
}

pub fn upload(
    pack: &Bound<'_, PyAny>,
    frame: [f64; 3],
    layer: &mut crate::preview::Preview,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    samples: crate::antialias::SampleModes,
    format: wgpu::TextureFormat,
) -> PyResult<()> {
    let vertices = bytes(pack, "vertices")?;
    let indices = bytes(pack, "indices")?;
    let materials = bytes(pack, "materials")?;
    let instances = bytes(pack, "instances")?;
    let faces = bytes(pack, "commands")?;
    let edges = bytes(pack, "edge_commands")?;
    let geo = Geometry {
        vertices: vertices.as_bytes(),
        indices: indices.as_bytes(),
        materials: materials.as_bytes(),
        textures: pack.getattr("texture_paths")?.extract()?,
    };
    let origin: [f64; 3] = pack.getattr("origin")?.extract()?;
    let shift = glam::DVec3::from(origin) - glam::DVec3::from(frame);
    let mut output = Output::default();
    for row in instances.as_bytes().chunks_exact(80) {
        let flags = word(row, 72);
        if flags & 1 != 0 {
            continue;
        }
        let code = word(row, 64) as usize;
        let mut world = Mat4::from_cols_array(&bytemuck::pod_read_unaligned(&row[..64]));
        world.w_axis = (world.w_axis.truncate().as_dvec3() + shift)
            .as_vec3()
            .extend(1.);
        let m = Mat3::from_mat4(world);
        let normals = Mat3::from_cols(
            m.y_axis.cross(m.z_axis),
            m.z_axis.cross(m.x_axis),
            m.x_axis.cross(m.y_axis),
        );
        let placement = Placement {
            world,
            normals,
            paint: word(row, 68).to_le_bytes(),
            mirrored: m.determinant() < 0.,
            billboard: flags & 8 != 0,
        };
        output.faces(
            &geo,
            &placement,
            record(geo.materials, code)?,
            record(faces.as_bytes(), code)?,
        )?;
        let cmd = record(edges.as_bytes(), code)?;
        for i in 0..word(cmd, 0) as usize {
            let (_, v) = geo.vertex(cmd, i)?;
            push(
                &mut output.edges,
                world.transform_point3(vec3(v)),
                Vec3::Z,
                [33, 43, 59, 255],
            );
        }
    }
    layer
        .upload(device, &output.triangles, &output.edges)
        .map_err(PyValueError::new_err)?;
    if !output.images.is_empty() {
        let mut textures = crate::preview_texture::TexturedPreview::new(
            samples,
            device,
            format,
            &layer.layout(),
            true,
        );
        let runs = output
            .images
            .into_iter()
            .map(|((id, shade, alpha), points)| {
                (
                    geo.textures[id as usize - 1].clone(),
                    f32::from_bits(shade),
                    f32::from(alpha) / 255.,
                    points.into_pyarray(pack.py()).readonly(),
                )
            })
            .collect();
        textures.upload(pack.py(), device, queue, runs)?;
        layer.textures = Some(textures);
    }
    layer.pose_group(queue, Mat4::IDENTITY.to_cols_array());
    Ok(())
}
