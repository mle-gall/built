//! Pack 48-byte edge records; preserve the 28-byte Python face ABI.
pub fn expand(
    vertices: &[u8],
    indices: &[u8],
    commands: &[u8],
    prototypes: usize,
) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), &'static str> {
    let mut v = Vec::new();
    let ix = indices.to_vec();
    let mut cmd = commands.to_vec();
    for (prototype, row) in cmd.chunks_exact_mut(20).skip(prototypes).enumerate() {
        let face: crate::Command =
            bytemuck::pod_read_unaligned(&commands[prototype * 20..prototype * 20 + 20]);
        let mut sides = adjacency(vertices, indices, face);
        let mut c: crate::Command = bytemuck::pod_read_unaligned(row);
        if c.count % 2 != 0 {
            return Err("edge command must contain endpoint pairs");
        }
        let first = v.len() / 48;
        let endpoint = |pair: &[u8], offset: usize| -> [u8; 12] {
            let index = u32::from_le_bytes(pair[offset..offset + 4].try_into().unwrap());
            let start = (i64::from(index) + i64::from(c.base)) as usize * 28;
            vertices[start..start + 12].try_into().unwrap()
        };
        let span = &indices[c.first as usize * 4..(c.first + c.count) as usize * 4];
        let mut pairs: Vec<([u8; 12], [u8; 12], bool)> = Vec::with_capacity(span.len() / 8);
        for pair in span.chunks_exact(8) {
            let (a, b) = (endpoint(pair, 0), endpoint(pair, 4));
            sides.mark_hard(&key(&a, &b));
            pairs.push((a, b, false));
        }
        let hard_count = pairs.len();
        // Soft and profile candidates in the order the triangles met them.
        for (k, n, hard) in sides.entries() {
            if n[0] != [0.; 3] && !hard
                && (n[1] == [0.; 3] || glam::Vec3::from(n[0]).dot(glam::Vec3::from(n[1])) < 0.99999)
            {
                pairs.push((k[..12].try_into().unwrap(), k[12..].try_into().unwrap(), true));
            }
        }
        let count = pairs.len();
        v.reserve(count * 48);
        for (a, b, soft) in pairs {
            v.extend_from_slice(&a);
            v.extend_from_slice(&b);
            let normals = sides.normals(&key(&a, &b));
            // A power-of-two normal scale encodes the profile-only flag without
            // quantization. Decoding by 0.5 restores the original float bits.
            let n0 = normals[0].map(|x| if soft { x * 2. } else { x });
            v.extend_from_slice(bytemuck::cast_slice(&n0));
            v.extend_from_slice(bytemuck::cast_slice(&normals[1]));
        }
        c.first = u32::try_from(first * 6).map_err(|_| "edge buffer too large")?;
        c.count = u32::try_from(count * 6).map_err(|_| "edge buffer too large")?;
        c.base = c.start as i32; // Non-indexed first_instance addresses survivors.
        c.start = u32::try_from(hard_count * 6).map_err(|_| "edge buffer too large")?;
        row.copy_from_slice(bytemuck::bytes_of(&c));
    }
    if v.is_empty() { v.resize(48, 0); } // Valid storage binding for face-only scenes.
    Ok((v, ix, cmd))
}

/// A multiply-rotate hash for the position keys (the default SipHash was
/// most of an edit's edge expansion on a 200k-face prototype).
#[derive(Default, Clone, Copy)]
pub(crate) struct FastHash(u64);
impl std::hash::Hasher for FastHash {
    fn finish(&self) -> u64 {
        // Fold the high bits down: the table indexes by the low ones.
        self.0 ^ (self.0 >> 29)
    }
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.write_u64(u64::from_le_bytes(word));
        }
    }
    fn write_u64(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
    fn write_usize(&mut self, word: usize) {
        self.write_u64(word as u64);
    }
}
pub(crate) type FastMap<K, V> = std::collections::HashMap<K, V, std::hash::BuildHasherDefault<FastHash>>;

#[cfg(test)]
mod tests {
    #[test]
    fn expands_endpoint_pairs_without_changing_face_commands() {
        let mut vertices = vec![0u8; 56];
        vertices[28..32].copy_from_slice(&2f32.to_le_bytes());
        let indices = bytemuck::cast_slice(&[0u32, 1]).to_vec();
        let commands = [
            crate::Command {
                count: 0,
                instances: 0,
                first: 0,
                base: 0,
                start: 0,
            },
            crate::Command {
                count: 2,
                instances: 0,
                first: 0,
                base: 0,
                start: 0,
            },
        ];
        let (v, ix, cmd) =
            super::expand(&vertices, &indices, bytemuck::cast_slice(&commands), 1).unwrap();
        assert_eq!(&cmd[..20], bytemuck::bytes_of(&commands[0]));
        assert_eq!(v.len(), 48);
        assert_eq!(ix.len(), 2 * 4);
        let edge: crate::Command = bytemuck::pod_read_unaligned(&cmd[20..]);
        assert_eq!((edge.first, edge.count, edge.base), (0, 6, 0));
        assert_eq!(&v[..12], &vertices[..12]);
        assert_eq!(&v[12..24], &vertices[28..40]);


    }
}

// Position keys join duplicated triangle vertices without changing the upload ABI.
pub(crate) fn key(a: &[u8], b: &[u8]) -> [u8; 24] {
    let mut k = [0; 24];
    let (a, b) = if a < b { (a, b) } else { (b, a) };
    k[..12].copy_from_slice(a);
    k[12..].copy_from_slice(b);
    k
}
/// Each edge of the face triangles (by its endpoints' positions): the
/// normals of the two triangles on it and whether an edge record draws it.
/// A position key as three words: hashed and compared without a length
/// prefix or a byte loop.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Words([u64; 3]);
impl std::hash::Hash for Words {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        for word in self.0 {
            state.write_u64(word);
        }
    }
}
fn words(k: &[u8; 24]) -> Words {
    Words(std::array::from_fn(|i| u64::from_le_bytes(k[i * 8..i * 8 + 8].try_into().unwrap())))
}

/// Each edge of the face triangles (by its endpoints' positions): the
/// normals of the two triangles on it and whether an edge record draws it.
/// A large prototype's table is split in shards built on threads of their
/// own (the table's cache misses were most of an edit's expansion).
struct Sides {
    shards: Vec<Shard>,
}
#[derive(Default)]
struct Shard {
    lookup: FastMap<Words, u32>,
    entries: Vec<([u8; 24], [[f32; 3]; 2], bool)>,
}
fn shard_of(k: &Words, shards: usize) -> usize {
    if shards == 1 {
        return 0;
    }
    let mixed = (k.0[0] ^ k.0[1].rotate_left(17) ^ k.0[2].rotate_left(31)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    ((mixed >> 40) as usize) % shards
}
impl Sides {
    fn find(&self, k: &[u8; 24]) -> Option<(usize, usize)> {
        let w = words(k);
        let shard = shard_of(&w, self.shards.len());
        self.shards[shard].lookup.get(&w).map(|&at| (shard, at as usize))
    }
    fn mark_hard(&mut self, k: &[u8; 24]) {
        if let Some((shard, at)) = self.find(k) {
            self.shards[shard].entries[at].2 = true;
        }
    }
    fn normals(&self, k: &[u8; 24]) -> [[f32; 3]; 2] {
        self.find(k).map_or([[0.; 3]; 2], |(shard, at)| self.shards[shard].entries[at].1)
    }
    fn entries(&self) -> impl Iterator<Item = &([u8; 24], [[f32; 3]; 2], bool)> {
        self.shards.iter().flat_map(|shard| shard.entries.iter())
    }
}
fn adjacency(vertices: &[u8], indices: &[u8], c: crate::Command) -> Sides {
    let triangles = (c.count / 3) as usize;
    let shards = if triangles >= 1 << 16 {
        std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 8)
    } else {
        1
    };
    adjacency_in(vertices, indices, c, shards)
}
fn adjacency_in(vertices: &[u8], indices: &[u8], c: crate::Command, shards: usize) -> Sides {
    let span = &indices[c.first as usize * 4..(c.first + c.count) as usize * 4];
    let triangles = span.len() / 12;
    let corner = |tri: &[u8], i: usize| -> [u8; 12] {
        let index = u32::from_le_bytes(tri[i * 4..i * 4 + 4].try_into().unwrap());
        let offset = (i64::from(index) + i64::from(c.base)) as usize * 28;
        vertices[offset..offset + 12].try_into().unwrap()
    };
    let build = |shard: usize| -> Shard {
        let mut own = Shard {
            lookup: FastMap::with_capacity_and_hasher(triangles * 2 / shards + 16, Default::default()),
            entries: Vec::with_capacity(triangles * 2 / shards + 16),
        };
        for tri in span.chunks_exact(12) {
            let points = [corner(tri, 0), corner(tri, 1), corner(tri, 2)];
            let mut normal = None;
            for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                let k = key(&points[a], &points[b]);
                let w = words(&k);
                if shard_of(&w, shards) != shard {
                    continue;
                }
                let normal = *normal.get_or_insert_with(|| {
                    let point = |i: usize| glam::Vec3::from_array(bytemuck::pod_read_unaligned::<[f32; 3]>(&points[i]));
                    (point(1) - point(0)).cross(point(2) - point(0)).normalize_or_zero().to_array()
                });
                let at = *own.lookup.entry(w).or_insert_with(|| {
                    own.entries.push((k, [[0.; 3]; 2], false));
                    (own.entries.len() - 1) as u32
                });
                let entry = &mut own.entries[at as usize].1;
                if entry[0] == [0.; 3] {
                    entry[0] = normal;
                } else {
                    entry[1] = normal;
                }
            }
        }
        own
    };
    if shards == 1 {
        return Sides { shards: vec![build(0)] };
    }
    let built = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..shards).map(|shard| scope.spawn(move || build(shard))).collect();
        handles.into_iter().map(|h| h.join().expect("an edge shard")).collect()
    });
    Sides { shards: built }
}
#[cfg(test)]
fn silhouette(a: f32, b: f32, boundary: bool) -> bool {
    boundary || (a > 0.) != (b > 0.)
}
#[cfg(test)]
mod classification_tests {
    #[test]
    fn coplanar_triangulation_does_not_create_a_profile_diagonal() {
        let mut vertices = Vec::new();
        for p in [[0f32, 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]] {
            vertices.extend_from_slice(bytemuck::cast_slice(&p));
            vertices.extend_from_slice(&[0; 16]);
        }
        let indices = [0u32, 1, 2, 0, 2, 3];
        let commands = [
            crate::Command {
                count: 6,
                instances: 0,
                first: 0,
                base: 0,
                start: 0,
            },
            crate::Command {
                count: 0,
                instances: 0,
                first: 6,
                base: 0,
                start: 0,
            },
        ];
        let (_, _, expanded) = super::expand(
            &vertices,
            bytemuck::cast_slice(&indices),
            bytemuck::cast_slice(&commands),
            1,
        )
        .unwrap();
        let edge: crate::Command = bytemuck::pod_read_unaligned(&expanded[20..]);
        assert_eq!(edge.count, 24); // Four boundaries, no interior diagonal.
    }

    #[test]
    fn boundary_and_opposed_faces_are_profiles() {
        assert!(super::silhouette(1., 1., true));
        assert!(super::silhouette(-1., 1., false));
        assert!(super::silhouette(1., -1., false));
        assert!(!super::silhouette(1., 1., false));
        assert!(!super::silhouette(-1., -1., false));
    }
}

#[cfg(test)]
mod compact_tests {
    #[test]
    fn pixel_width_is_depth_invariant() {
        // Clip-space displacement used by edge_vs; viewport conversion cancels w.
        for depth in [0.01f32, 1., 1000.] {
            for width in [1f32, 4., 7.5] {
                let clip_offset = width / 1400. * depth;
                let pixels = 2. * clip_offset / depth * 1400. * 0.5;
                assert!((pixels - width).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn packs_normals_flags_and_hard_only_count() {
        let mut vertices = Vec::new();
        for p in [[0f32,0.,0.],[1.,0.,0.],[0.,1.,0.]] {
            vertices.extend_from_slice(bytemuck::cast_slice(&p));
            vertices.extend_from_slice(&[0;16]);
        }
        let commands = [
            crate::Command { count:3, instances:0, first:0, base:0, start:7 },
            crate::Command { count:4, instances:0, first:3, base:0, start:7 },
        ];
        // Coincident source edges preserve their independent dash directions.
        let (v, _, cmd) = super::expand(&vertices, bytemuck::cast_slice(&[0u32,1,2,0,1,1,0]),
            bytemuck::cast_slice(&commands), 1).unwrap();
        assert_eq!(v.len(), 4*48);
        let edge: crate::Command = bytemuck::pod_read_unaligned(&cmd[20..]);
        assert_eq!((edge.count,edge.base,edge.start), (24,7,12));
        assert_eq!(f32::from_le_bytes(v[32..36].try_into().unwrap()),1.);
        assert_eq!(f32::from_le_bytes(v[44..48].try_into().unwrap()),0.);
        assert_eq!(f32::from_le_bytes(v[128..132].try_into().unwrap()),2.);
    }

    #[test]
    fn empty_edges_have_a_valid_storage_binding() {
        let command = crate::Command { count: 0, instances: 0, first: 0, base: 0, start: 0 };
        let (records, indices, commands) = super::expand(&[], &[],
            bytemuck::cast_slice(&[command, command]), 1).unwrap();
        assert_eq!(records.len(), 48);
        assert_eq!(indices.len(), 0);
        assert_eq!(commands.len(), 40);
    }

    #[test]
    fn flag_scale_preserves_normal_bits() {
        for normal in [glam::Vec3::Z, -glam::Vec3::Z, glam::Vec3::new(1.,2.,3.).normalize()] {
            assert_eq!((normal * 2. * 0.5).to_array(), normal.to_array());
        }
    }
}

#[cfg(test)]
mod view_direction_tests {
    #[test]
    fn local_eye_classification_matches_projected_area() {
        use glam::{Mat3, Mat4, Vec3, Vec4};
        let eye = Vec3::new(5., 7., 11.);
        let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
        let a = Vec3::new(0.2, 0.3, 0.4);
        let b = a + Vec3::Y;
        for mirror in [1., -1.] {
            let world = Mat4::from_scale(Vec3::new(mirror*2., 3., 4.));
            for perspective in [false, true] {
                let projection = if perspective { Mat4::perspective_rh(1., 1.4, 0.1, 100.) }
                    else { Mat4::orthographic_rh(-10.,10.,-10.,10.,0.1,100.) };
                let transform = projection * view * world;
                let camera = view.inverse() * if perspective { Vec4::W } else { Vec4::Z };
                let local = world.inverse() * camera;
                let direction = local.truncate() - a * local.w;
                let xyw = |p: Vec4| Vec3::new(p.x,p.y,p.w);
                let projected = |n: Vec3| Mat3::from_cols(
                    xyw(transform*a.extend(1.)), xyw(transform*b.extend(1.)),
                    xyw(transform*(a+n.cross(b-a)).extend(1.))).determinant();
                for (n0,n1) in [(Vec3::X,Vec3::Z),(Vec3::X,-Vec3::Z),(-Vec3::X,Vec3::Z)] {
                    assert_eq!((projected(n0)>0.) != (projected(n1)>0.),
                        (n0.dot(direction)>0.) != (n1.dot(direction)>0.));
                }
            }
        }
    }
}

#[cfg(test)]
mod placement_cache_tests {
    #[test]
    fn cached_clip_transform_preserves_mirrors_and_near_plane_crossings() {
        use glam::{Mat4, Vec3};
        let projection = Mat4::perspective_rh(1., 1.4, 0.1, 100.);
        for scale in [Vec3::ONE, Vec3::new(-2., 3., 0.5)] {
            let world = Mat4::from_scale_rotation_translation(scale,
                glam::Quat::from_rotation_y(0.3), Vec3::new(0., 0., -0.1));
            let cached = projection * world;
            for point in [Vec3::ZERO, Vec3::X, Vec3::Z, -Vec3::Z] {
                let expected = projection * (world * point.extend(1.));
                assert!((cached * point.extend(1.) - expected).abs().max_element() < 1e-5);
            }
        }
    }
}

#[cfg(test)]
mod screen_cull_tests {
    #[test]
    fn screen_size_cull_retains_nonzero_subpixel_coverage() {
        // WGSL direction is NDC delta times viewport, twice physical pixels.
        for (pixels, culled) in [(0.,true),(0.01,false),(0.49,false),(0.5,false),(1.,false)] {
            let direction = glam::Vec2::new(pixels*2., 0.);
            assert_eq!(direction.length() == 0., culled);
        }
    }
}

#[cfg(test)]
mod shard_tests {
    /// A grid of 300 x 120 quads (72k triangles): the threads' shards hold
    /// exactly the sides one table holds, normals included.
    #[test]
    fn shards_hold_what_one_table_holds() {
        let (nx, ny) = (300u32, 120u32);
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for i in 0..nx {
            for j in 0..ny {
                let z = ((i * 7 + j * 3) % 5) as f32 * 0.1;
                let corners = [[i, j], [i + 1, j], [i + 1, j + 1], [i, j + 1]];
                let at = (vertices.len() / 28) as u32;
                for [x, y] in corners {
                    vertices.extend_from_slice(bytemuck::cast_slice(&[x as f32, y as f32, z]));
                    vertices.extend_from_slice(&[0; 16]);
                }
                indices.extend_from_slice(&[at, at + 1, at + 2, at, at + 2, at + 3]);
            }
        }
        let indices: Vec<u8> = bytemuck::cast_slice(&indices).to_vec();
        let c = crate::Command { count: (indices.len() / 4) as u32, instances: 0, first: 0, base: 0, start: 0 };
        let one = super::adjacency_in(&vertices, &indices, c, 1);
        let many = super::adjacency_in(&vertices, &indices, c, 5);
        let mut a: Vec<_> = one.entries().map(|e| (e.0, e.1.map(|n| n.map(f32::to_bits)))).collect();
        let mut b: Vec<_> = many.entries().map(|e| (e.0, e.1.map(|n| n.map(f32::to_bits)))).collect();
        assert!(a.len() > 100_000);
        a.sort();
        b.sort();
        assert_eq!(a, b);
        for entry in one.entries().take(1000) {
            assert_eq!(many.normals(&entry.0), entry.1);
        }
    }
}
