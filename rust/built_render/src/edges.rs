//! Pack 48-byte edge records; preserve the 28-byte Python face ABI.
pub fn expand(
    vertices: &[u8],
    indices: &[u8],
    commands: &[u8],
    prototypes: usize,
) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), &'static str> {
    expand_with_memos(vertices, indices, commands, prototypes, None)
}

/// Reclaim edit gaps if the unpadded scene fits a smaller GPU. Edge draws
/// are in physical record order and use six indices per record.
pub(super) fn remove_slack(records: &mut Vec<u8>, commands: &mut [u8], prototypes: usize) {
    let mut used = 0;
    for row in commands.chunks_exact_mut(20).skip(prototypes) {
        let mut cmd: crate::Command = bytemuck::pod_read_unaligned(row);
        let start = cmd.first as usize / 6 * 48;
        let bytes = cmd.count as usize / 6 * 48;
        records.copy_within(start..start + bytes, used);
        cmd.first = (used / 48 * 6) as u32;
        row.copy_from_slice(bytemuck::bytes_of(&cmd));
        used += bytes;
    }
    records.truncate(used);
}

pub(super) fn expand_with_memos(
    vertices: &[u8], indices: &[u8], commands: &[u8], prototypes: usize,
    mut memos: Option<&mut Vec<(usize, crate::edge_memo::Memo)>>,
) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), &'static str> {
    let mut v = Vec::new();
    let ix = indices.to_vec();
    let mut cmd = commands.to_vec();
    for (prototype, row) in cmd.chunks_exact_mut(20).skip(prototypes).enumerate() {
        let face: crate::Command =
            bytemuck::pod_read_unaligned(&commands[prototype * 20..prototype * 20 + 20]);
        let mut c: crate::Command = bytemuck::pod_read_unaligned(row);
        if c.count % 2 != 0 {
            return Err("edge command must contain endpoint pairs");
        }
        let first = v.len() / 48;
        // Prepare the same adjacency state used by interactive replacements
        // during scene upload. Indexed or disjoint ranges use normal expansion.
        let faces = face.count as usize;
        if let Some(cache) = memos.as_deref_mut() {
            let count = faces + c.count as usize;
            if cache.len() < 2 && count <= crate::edge_memo::MAX_VERTICES && faces / 3 >= crate::edge_memo::KEEP_PAST && faces % 3 == 0
                && c.base == face.base && face.base >= 0 && c.first as u64 == face.first as u64 + faces as u64
                && indices[face.first as usize * 4..(face.first as usize + count) * 4]
                    .chunks_exact(4).enumerate().all(|(i, word)| word == (i as u32).to_le_bytes())
            {
                let start = face.base as usize * 28;
                let memo = crate::edge_memo::Memo::build(&vertices[start..start + count * 28], faces);
                let records = memo.records.len() / 48;
                v.extend_from_slice(&memo.records);
                c.first = u32::try_from(first * 6).map_err(|_| "edge buffer too large")?;
                c.count = u32::try_from(records * 6).map_err(|_| "edge buffer too large")?;
                c.base = c.start as i32;
                c.start = u32::try_from(memo.hard * 6).map_err(|_| "edge buffer too large")?;
                row.copy_from_slice(bytemuck::bytes_of(&c));
                if memo.retained_bytes() + cache.iter().map(|m| m.1.retained_bytes()).sum::<usize>() <= crate::edge_memo::CACHE_BYTES {
                    v.resize(v.len() + crate::edit_slack(records) * 48, 0);
                    cache.push((prototype, memo));
                }
                continue;
            }
        }
        let endpoint = |pair: &[u8], offset: usize| -> [u8; 12] {
            let index = u32::from_le_bytes(pair[offset..offset + 4].try_into().unwrap());
            let start = (i64::from(index) + i64::from(c.base)) as usize * 28;
            vertices[start..start + 12].try_into().unwrap()
        };
        let span = &indices[c.first as usize * 4..(c.first + c.count) as usize * 4];
        let hard: Vec<([u8; 12], [u8; 12])> =
            span.chunks_exact(8).map(|pair| (endpoint(pair, 0), endpoint(pair, 4))).collect();
        // The hard edges are marked, and their entries found, on the shard
        // threads: one lookup each instead of two on this one.
        let (sides, at) = adjacency(vertices, indices, face, &hard);
        let hard_count = hard.len();
        // Soft and profile candidates in the order the triangles met them.
        let mut soft: Vec<(&[u8; 24], [[f32; 3]; 2])> = Vec::new();
        for (k, n, hard) in sides.entries() {
            if n[0] != [0.; 3] && !hard
                && (n[1] == [0.; 3] || glam::Vec3::from(n[0]).dot(glam::Vec3::from(n[1])) < 0.99999)
            {
                soft.push((k, *n));
            }
        }
        let count = hard_count + soft.len();
        let start = v.len();
        v.resize(start + count * 48, 0);
        // A power-of-two normal scale encodes the profile-only flag without
        // quantization. Decoding by 0.5 restores the original float bits.
        let record = |out: &mut [u8], a: &[u8], b: &[u8], normals: [[f32; 3]; 2], soft: bool| {
            out[..12].copy_from_slice(a);
            out[12..24].copy_from_slice(b);
            let n0 = normals[0].map(|x| if soft { x * 2. } else { x });
            out[24..36].copy_from_slice(bytemuck::cast_slice(&n0));
            out[36..48].copy_from_slice(bytemuck::cast_slice(&normals[1]));
        };
        let (hard_out, soft_out) = v[start..].split_at_mut(hard_count * 48);
        let write_hard = |out: &mut [u8], from: usize| {
            for (j, rec) in out.chunks_exact_mut(48).enumerate() {
                let (a, b) = &hard[from + j];
                record(rec, a, b, sides.at(at[from + j]), false);
            }
        };
        if hard_count >= 1 << 16 {
            let per = hard_count.div_ceil(8);
            std::thread::scope(|scope| {
                for (k, out) in hard_out.chunks_mut(per * 48).enumerate() {
                    let write_hard = &write_hard;
                    scope.spawn(move || write_hard(out, k * per));
                }
            });
        } else {
            write_hard(hard_out, 0);
        }
        for (rec, (k, n)) in soft_out.chunks_exact_mut(48).zip(&soft) {
            record(rec, &k[..12], &k[12..], *n, true);
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
pub(crate) struct Words(pub(crate) [u64; 3]);
impl Words {
    pub(crate) fn bytes(self) -> [u8;24] {
        let mut out = [0;24];
        for (dst, w) in out.chunks_exact_mut(8).zip(self.0) { dst.copy_from_slice(&w.to_le_bytes()); }
        out
    }
}
impl std::hash::Hash for Words {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        for word in self.0 {
            state.write_u64(word);
        }
    }
}
pub(crate) fn words(k: &[u8; 24]) -> Words {
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
pub(crate) fn shard_of(k: &Words, shards: usize) -> usize {
    if shards == 1 {
        return 0;
    }
    let mixed = (k.0[0] ^ k.0[1].rotate_left(17) ^ k.0[2].rotate_left(31)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    ((mixed >> 40) as usize) % shards
}
/// No entry: an edge no triangle has.
const NO_ENTRY: u64 = u64::MAX;
impl Sides {
    /// The normals of the entry `at` (from `adjacency`'s hard list).
    fn at(&self, at: u64) -> [[f32; 3]; 2] {
        if at == NO_ENTRY {
            return [[0.; 3]; 2];
        }
        self.shards[(at >> 32) as usize].entries[at as u32 as usize].1
    }
    fn entries(&self) -> impl Iterator<Item = &([u8; 24], [[f32; 3]; 2], bool)> {
        self.shards.iter().flat_map(|shard| shard.entries.iter())
    }
}
/// The sides of the face triangles, with each of `hard`'s edges marked
/// and its entry (`Sides::at`) in order.
fn adjacency(vertices: &[u8], indices: &[u8], c: crate::Command, hard: &[([u8; 12], [u8; 12])]) -> (Sides, Vec<u64>) {
    let triangles = (c.count / 3) as usize;
    let shards = if triangles + hard.len() / 2 >= 1 << 16 {
        std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 8)
    } else {
        1
    };
    adjacency_in(vertices, indices, c, hard, shards)
}
fn adjacency_in(vertices: &[u8], indices: &[u8], c: crate::Command, hard: &[([u8; 12], [u8; 12])],
                shards: usize) -> (Sides, Vec<u64>) {
    let span = &indices[c.first as usize * 4..(c.first + c.count) as usize * 4];
    let triangles = span.len() / 12;
    let corner = |tri: &[u8], i: usize| -> [u8; 12] {
        let index = u32::from_le_bytes(tri[i * 4..i * 4 + 4].try_into().unwrap());
        let offset = (i64::from(index) + i64::from(c.base)) as usize * 28;
        vertices[offset..offset + 12].try_into().unwrap()
    };
    let build = |shard: usize| -> (Shard, Vec<(u32, u32)>) {
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
        let mut found = Vec::new();
        for (i, (a, b)) in hard.iter().enumerate() {
            let w = words(&key(a, b));
            if shard_of(&w, shards) != shard {
                continue;
            }
            if let Some(&at) = own.lookup.get(&w) {
                own.entries[at as usize].2 = true;
                found.push((i as u32, at));
            }
        }
        (own, found)
    };
    let built: Vec<(Shard, Vec<(u32, u32)>)> = if shards == 1 {
        vec![build(0)]
    } else {
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..shards).map(|shard| scope.spawn(move || build(shard))).collect();
            handles.into_iter().map(|h| h.join().expect("an edge shard")).collect()
        })
    };
    let mut at = vec![NO_ENTRY; hard.len()];
    let mut out = Vec::with_capacity(built.len());
    for (shard, (own, found)) in built.into_iter().enumerate() {
        for (i, entry) in found {
            at[i as usize] = (shard as u64) << 32 | u64::from(entry);
        }
        out.push(own);
    }
    (Sides { shards: out }, at)
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
        // Every quad's first side as a hard edge, and one edge on no triangle.
        let point = |v: usize| -> [u8; 12] { vertices[v * 28..v * 28 + 12].try_into().unwrap() };
        let mut hard: Vec<_> = (0..vertices.len() / 28).step_by(4).map(|v| (point(v), point(v + 1))).collect();
        hard.push(([1; 12], [2; 12]));
        let (one, one_at) = super::adjacency_in(&vertices, &indices, c, &hard, 1);
        let (many, many_at) = super::adjacency_in(&vertices, &indices, c, &hard, 5);
        let mut a: Vec<_> = one.entries().map(|e| (e.0, e.1.map(|n| n.map(f32::to_bits)), e.2)).collect();
        let mut b: Vec<_> = many.entries().map(|e| (e.0, e.1.map(|n| n.map(f32::to_bits)), e.2)).collect();
        assert!(a.len() > 100_000);
        assert_eq!(a.iter().filter(|e| e.2).count(), hard.len() - 1);
        a.sort();
        b.sort();
        assert_eq!(a, b);
        assert_eq!(*many_at.last().unwrap(), super::NO_ENTRY);
        for (i, (x, y)) in hard.iter().enumerate() {
            assert_eq!(one.at(one_at[i]), many.at(many_at[i]));
            if i + 1 < hard.len() {
                let k = super::key(x, y);
                assert_eq!(one.entries().find(|e| e.0 == k).unwrap().1, many.at(many_at[i]));
            }
        }
    }
}
