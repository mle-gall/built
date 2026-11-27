//! Batch triangulation across prototypes, then fill disjoint final buffer slices.
//! Only triangle indices/normals survive the count pass; packed vertices are never
//! concatenated. NumPy takes ownership of the final Vec allocations.
use crate::facegeom::{self, Hypot, V3};
use numpy::{PyArray1, PyReadonlyArray1, PyReadonlyArray2};
use pyo3::{exceptions::PyValueError, prelude::*};
use rayon::prelude::*;
use std::collections::HashMap;

struct Prepared {
    triangles: Vec<([u32; 3], [V3; 3], [u8; 4])>,
    faces: Vec<usize>,
    visible_triangles: usize,
    visible_edges: usize,
    edges: Vec<[u32; 2]>,
    bounds: [f32; 8],
}

/// A multiply-rotate hash for the vertex-pair keys (SipHash was a large
/// part of baking a 200k-face prototype).
#[derive(Default, Clone, Copy)]
struct FastHash(u64);
impl std::hash::Hasher for FastHash {
    fn finish(&self) -> u64 {
        self.0 ^ (self.0 >> 29)
    }
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.write_u64(u64::from_le_bytes(word));
        }
    }
    fn write_u32(&mut self, word: u32) {
        self.write_u64(u64::from(word));
    }
    fn write_u64(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn prepare(
    p: &[V3],
    c: &[u32],
    sizes: &[i64],
    counts: &[i64],
    colors: &[[u8; 4]],
    hidden: &[bool],
    edges: &[[u32; 2]],
    flags: &[u8],
    h: Hypot,
    retain_hidden: bool,
) -> Prepared {
    prepare_with(p, c, sizes, counts, colors, hidden, edges, flags, h, retain_hidden, None)
}

/// `exact`: per ring corner, the exact surface normal of the face at that
/// corner (NaN: none). Where given it replaces the soft-fan average, so a
/// curved support shades from its own normals (store/exact_display.rs).
#[allow(clippy::too_many_arguments)]
fn prepare_with(
    p: &[V3],
    c: &[u32],
    sizes: &[i64],
    counts: &[i64],
    colors: &[[u8; 4]],
    hidden: &[bool],
    edges: &[[u32; 2]],
    flags: &[u8],
    h: Hypot,
    retain_hidden: bool,
    exact: Option<&[[f32; 3]]>,
) -> Prepared {
    let mut bounds = [0.; 8];
    if !p.is_empty() {
        bounds = [
            f32::INFINITY,
            f32::INFINITY,
            f32::INFINITY,
            0.,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
            0.,
        ];
        for v in p {
            for (k, x) in [v.0, v.1, v.2].into_iter().enumerate() {
                bounds[k] = bounds[k].min(x);
                bounds[k + 4] = bounds[k + 4].max(x);
            }
        }
    }
    let (edge_list, edge_flags, sides, face_rings) = edge_table(p.len(), edges, flags, c, sizes, counts);
    // Triangulation is per face: in parallel for a large prototype.
    let geometry = |f: usize| {
        let (mut r, mut k) = face_rings[f];
        let mut rings = Vec::with_capacity(counts[f] as usize);
        for _ in 0..counts[f] {
            let n = sizes[r] as usize;
            rings.push(&c[k..k + n]);
            k += n;
            r += 1;
        }
        facegeom::resident_geometry(p, &rings, h)
    };
    let per_face: Vec<_> = if counts.len() >= 4096 {
        use rayon::prelude::*;
        (0..counts.len()).into_par_iter().map(geometry).collect()
    } else {
        (0..counts.len()).map(geometry).collect()
    };
    let mut triangles = Vec::new();
    let mut normals = Vec::with_capacity(counts.len());
    for (f, (tris, normal)) in per_face.into_iter().enumerate() {
        normals.push(normal);
        if retain_hidden || !hidden[f] {
            triangles.extend(tris.into_iter().map(|t| (t, f, colors[f])));
        }
    }
    let fans = if edge_flags.iter().any(|f| f & 1 != 0 && f & 4 == 0) {
        let mut starts = vec![0i64; edge_list.len() + 1];
        for &(edge, _) in &sides {
            starts[edge as usize + 1] += 1;
        }
        for e in 0..edge_list.len() {
            starts[e + 1] += starts[e];
        }
        let mut adjacent = vec![0i64; sides.len()];
        let mut fill_at: Vec<i64> = starts[..edge_list.len()].to_vec();
        for &(edge, face) in &sides {
            adjacent[fill_at[edge as usize] as usize] = i64::from(face);
            fill_at[edge as usize] += 1;
        }
        let smooth_edges: Vec<_> = edge_list
            .iter()
            .map(|e| [e[0] as i64, e[1] as i64])
            .collect();
        // Soft edges shade smooth, unless flat (soft only: «Smooth normals» off).
        let soft: Vec<_> = edge_flags.iter().map(|f| f & 1 != 0 && f & 4 == 0).collect();
        let corners: Vec<_> = c.iter().map(|&v| v as i64).collect();
        let model_vectors = |vs: &[V3]| vs.iter().map(|v|
            crate::store::geometry::V3(v.0 as f64, v.1 as f64, v.2 as f64)).collect::<Vec<_>>();
        crate::smooth::corners(
            &model_vectors(p),
            &model_vectors(&normals),
            &corners,
            sizes,
            counts,
            &smooth_edges,
            &soft,
            &starts,
            &adjacent,
            h,
        )
    } else { Default::default() };
    if retain_hidden && hidden.iter().any(|v| *v) {
        triangles.sort_by_key(|t| hidden[t.1]);
    }
    let visible_triangles = triangles.iter().filter(|t| !hidden[t.1]).count();
    let faces = triangles.iter().map(|t| t.1).collect();
    let exact_at = |v: u32, f: usize| -> Option<V3> {
        let exact = exact?;
        let (mut r, mut k) = face_rings[f];
        for _ in 0..counts[f] {
            let n = sizes[r] as usize;
            if let Some(i) = c[k..k + n].iter().position(|&x| x == v) {
                let [x, y, z] = *exact.get(k + i)?;
                return x.is_finite().then_some(V3(x, y, z));
            }
            k += n;
            r += 1;
        }
        None
    };
    let triangles = triangles
        .into_iter()
        .map(|(t, f, color)| {
            let ns = t.map(|v| {
                exact_at(v, f).unwrap_or_else(|| {
                    fans.get(&(v as usize, f)).map(|n| V3(n.0 as f32, n.1 as f32, n.2 as f32)).unwrap_or(normals[f])
                })
            });
            (t, ns, color)
        })
        .collect();
    let mut edges: Vec<_> = edge_list
        .into_iter()
        .zip(edge_flags)
        .filter(|(_, flags)| flags & 8 == 0 && (retain_hidden || flags & 3 == 0))
        .collect();
    let visible_edges = edges.iter().filter(|(_, flags)| flags & 3 == 0).count();
    if visible_edges < edges.len() {
        // The reveal pass uses the suffix for hidden edges and softened seams.
        edges.sort_by_key(|(_, flags)| flags & 3 != 0);
    }
    let edges = edges.into_iter().map(|(e, _)| e).collect();
    Prepared {
        triangles,
        faces,
        visible_triangles,
        visible_edges,
        edges,
        bounds,
    }
}

/// What the bake reads off the edges and rings: the distinct vertex pairs
/// (the edges first, in order, then ring sides no edge has, as first met)
/// with their flags or-ed over repeats, each ring side as `(pair, face)`
/// in face order, and where each face's rings start. Vertex pairs are
/// matched through buckets by lower vertex, not a hash table (the table's
/// misses were most of a 200k-face prototype's bake).
#[allow(clippy::type_complexity)]
fn edge_table(n_vertices: usize, edges: &[[u32; 2]], flags: &[u8], c: &[u32], sizes: &[i64], counts: &[i64])
    -> (Vec<[u32; 2]>, Vec<u8>, Vec<(u32, u32)>, Vec<(usize, usize)>) {
    const SKIP: u32 = u32::MAX;
    // The stream of pairs: every edge, then every ring side.
    let m = edges.len() + c.len();
    let mut ends: Vec<[u32; 2]> = Vec::with_capacity(m);
    ends.extend_from_slice(edges);
    let mut face_of: Vec<u32> = Vec::with_capacity(c.len());
    let mut face_rings: Vec<(usize, usize)> = Vec::with_capacity(counts.len());
    let (mut r, mut k) = (0, 0);
    for (f, &count) in counts.iter().enumerate() {
        face_rings.push((r, k));
        for _ in 0..count {
            let n = sizes[r] as usize;
            let ring = &c[k..k + n];
            for i in 0..n {
                ends.push([ring[i], ring[(i + 1) % n]]);
                face_of.push(f as u32);
            }
            k += n;
            r += 1;
        }
    }
    // Bucket the stream by lower vertex, in stream order.
    let mut start = vec![0u32; n_vertices + 1];
    for &[a, b] in &ends {
        if a != b {
            start[a.min(b) as usize + 1] += 1;
        }
    }
    for v in 0..n_vertices {
        start[v + 1] += start[v];
    }
    let mut fill = start.clone();
    let mut bucket = vec![0u32; start[n_vertices] as usize];
    for (i, &[a, b]) in ends.iter().enumerate() {
        if a != b {
            let lo = a.min(b) as usize;
            bucket[fill[lo] as usize] = i as u32;
            fill[lo] += 1;
        }
    }
    // Each item's first occurrence of its pair.
    let mut first = vec![SKIP; m];
    let hi = |i: u32| ends[i as usize][0].max(ends[i as usize][1]);
    let mut wide: HashMap<u32, u32, std::hash::BuildHasherDefault<FastHash>> = HashMap::default();
    for v in 0..n_vertices {
        let items = &bucket[start[v] as usize..start[v + 1] as usize];
        if items.len() > 64 {
            // A vertex many sides meet at (a fan's apex): by table.
            wide.clear();
            for &i in items {
                first[i as usize] = *wide.entry(hi(i)).or_insert(i);
            }
            continue;
        }
        for (j, &i) in items.iter().enumerate() {
            let h = hi(i);
            let earlier = items[..j].iter().find(|&&e| hi(e) == h);
            first[i as usize] = earlier.map_or(i, |&e| first[e as usize]);
        }
    }
    // Pairs numbered as first met; flags or-ed onto the first.
    let mut id = vec![SKIP; m];
    let mut edge_list = Vec::new();
    let mut edge_flags: Vec<u8> = Vec::new();
    for i in 0..m {
        let f = first[i];
        if f == SKIP {
            continue;
        }
        if f as usize == i {
            id[i] = edge_list.len() as u32;
            edge_list.push(ends[i]);
            edge_flags.push(flags.get(i).copied().unwrap_or(0));
        } else {
            id[i] = id[f as usize];
            if let Some(&flag) = flags.get(i).filter(|_| i < edges.len()) {
                edge_flags[id[i] as usize] |= flag;
            }
        }
    }
    let sides = (edges.len()..m)
        .filter(|&i| id[i] != SKIP)
        .map(|i| (id[i], face_of[i - edges.len()]))
        .collect();
    (edge_list, edge_flags, sides, face_rings)
}

fn fill(p: &[V3], prepared: &Prepared, bytes: &mut [u8], indices: &mut [u32]) {
    let mut cursor = 0;
    let mut vertex = |i: u32, n: V3, color: [u8; 4]| {
        let v = p[i as usize];
        let dst = &mut bytes[cursor * 28..(cursor + 1) * 28];
        for (slot, x) in dst[..24]
            .chunks_exact_mut(4)
            .zip([v.0, v.1, v.2, n.0, n.1, n.2])
        {
            slot.copy_from_slice(&x.to_le_bytes());
        }
        dst[24..].copy_from_slice(&color);
        indices[cursor] = cursor as u32;
        cursor += 1;
    };
    for hidden in [false, true] {
        let ts = if hidden {
            &prepared.triangles[prepared.visible_triangles..]
        } else {
            &prepared.triangles[..prepared.visible_triangles]
        };
        let es = if hidden {
            &prepared.edges[prepared.visible_edges..]
        } else {
            &prepared.edges[..prepared.visible_edges]
        };
        for &(t, n, color) in ts {
            for (i, normal) in t.into_iter().zip(n) {
                vertex(i, normal, color);
            }
        }
        for &e in es {
            for i in e {
                vertex(i, V3(0., 0., 1.), [0; 4]);
            }
        }
    }
}

#[pyfunction]
#[pyo3(signature = (pos, corners, sizes, counts, colors, hidden, edges, flags, offsets, attributes, hypot, material_maps, material_paints, retain_hidden=false, corner_normals=None, edit_room=false))]
pub fn bake_scene<'py>(
    py: Python<'py>,
    pos: PyReadonlyArray2<'_, f32>,
    corners: PyReadonlyArray1<'_, i64>,
    sizes: PyReadonlyArray1<'_, i64>,
    counts: PyReadonlyArray1<'_, i64>,
    colors: PyReadonlyArray2<'_, u8>,
    hidden: PyReadonlyArray1<'_, bool>,
    edges: PyReadonlyArray2<'_, i64>,
    flags: PyReadonlyArray1<'_, u8>,
    offsets: PyReadonlyArray2<'_, i64>,
    attributes: PyReadonlyArray1<'_, i64>,
    hypot: &str,
    material_maps: PyReadonlyArray2<'_, f32>,
    material_paints: PyReadonlyArray2<'_, u32>,
    retain_hidden: bool,
    corner_normals: Option<PyReadonlyArray2<'_, f32>>,
    edit_room: bool,
) -> PyResult<(
    Bound<'py, PyArray1<u8>>,
    Bound<'py, PyArray1<u32>>,
    Bound<'py, PyArray1<f32>>,
    Bound<'py, PyArray1<u32>>,
    Bound<'py, PyArray1<u8>>,
)> {
    let invalid = || PyValueError::new_err("invalid scene array shape or offsets");
    let p = pos.as_array();
    let e = edges.as_array();
    let col = colors.as_array();
    let off = offsets.as_array();
    let c = corners.as_slice()?;
    let s = sizes.as_slice()?;
    let f = counts.as_slice()?;
    let hidden = hidden.as_slice()?;
    let flags = flags.as_slice()?;
    if p.shape()[1] != 3
        || e.shape()[1] != 2
        || col.shape()[1] != 4
        || attributes.len()? != f.len()
        || hidden.len() != f.len()
        || flags.len() != e.shape()[0]
        || off.shape()[1] != 5
        || off.shape()[0] == 0
    {
        return Err(invalid());
    }
    let ends = [p.shape()[0], c.len(), s.len(), f.len(), e.shape()[0]];
    let offsets: Vec<[usize; 5]> = off
        .rows()
        .into_iter()
        .map(|r| std::array::from_fn(|k| r[k] as usize))
        .collect();
    if offsets[0] != [0; 5] || *offsets.last().unwrap() != ends {
        return Err(invalid());
    }
    for pair in offsets.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if (0..5).any(|k| a[k] > b[k] || b[k] > ends[k]) {
            return Err(invalid());
        }
        if s[a[2]..b[2]].iter().any(|&v| v < 3)
            || f[a[3]..b[3]].iter().any(|&v| v < 1)
            || s[a[2]..b[2]].iter().sum::<i64>() != (b[1] - a[1]) as i64
            || f[a[3]..b[3]].iter().sum::<i64>() != (b[2] - a[2]) as i64
            || c[a[1]..b[1]]
                .iter()
                .chain(e.slice(numpy::ndarray::s![a[4]..b[4], ..]).iter())
                .any(|&v| v < 0 || v as usize >= b[0] - a[0])
        {
            return Err(invalid());
        }
    }
    let pts: Vec<_> = p.rows().into_iter().map(|r| V3(r[0], r[1], r[2])).collect();
    let corners: Vec<_> = c.iter().map(|&v| v as u32).collect();
    let edges: Vec<_> = e
        .rows()
        .into_iter()
        .map(|r| [r[0] as u32, r[1] as u32])
        .collect();
    let attributes = attributes.as_slice()?;
    if attributes
        .iter()
        .any(|&a| a < 0 || a as usize >= col.shape()[0])
    {
        return Err(invalid());
    }
    let colors: Vec<_> = attributes
        .iter()
        .map(|&a| {
            let r = col.row(a as usize);
            [r[0], r[1], r[2], r[3]]
        })
        .collect();
    let maps = material_maps.as_array();
    let paints = material_paints.as_array();
    if maps.shape() != [f.len(), 16] || paints.shape() != [f.len(), 3] {
        return Err(invalid());
    }
    // Borrow the read-only mapping view throughout the synchronous detached
    // bake; copying 16 floats per source face costs hundreds of MB on imports.
    let mut paints: Vec<[u32; 3]> = paints
        .rows()
        .into_iter()
        .map(|r| [r[0], r[1], r[2]])
        .collect();
    let h = crate::hypot_of(hypot)?;
    let exact: Option<Vec<[f32; 3]>> = match &corner_normals {
        Some(a) => {
            let a = a.as_array();
            if a.shape() != [c.len(), 3] {
                return Err(invalid());
            }
            Some(a.rows().into_iter().map(|r| [r[0], r[1], r[2]]).collect())
        }
        None => None,
    };
    let exact = exact.as_deref();
    // Borrowed read-only NumPy slices stay alive throughout this synchronous call.
    let (bytes, indices, bounds, table, materials) = py.detach(|| {
        // One prototype (a rebake) runs here, not on a pool thread: under a
        // busy machine each hand-off cost more than the work it moved.
        let one = offsets.len() == 2;
        let prepare_pair = |pair: &[[usize; 5]]| {
            let (a, b) = (pair[0], pair[1]);
            prepare_with(
                &pts[a[0]..b[0]],
                &corners[a[1]..b[1]],
                &s[a[2]..b[2]],
                &f[a[3]..b[3]],
                &colors[a[3]..b[3]],
                &hidden[a[3]..b[3]],
                &edges[a[4]..b[4]],
                &flags[a[4]..b[4]],
                h,
                retain_hidden,
                exact.map(|e| &e[a[1]..b[1]]),
            )
        };
        let prepared: Vec<_> = if one { vec![prepare_pair(&offsets)] } else { offsets
            .par_windows(2)
            .map(|pair| {
                let (a, b) = (pair[0], pair[1]);
                let result = prepare_with(
                    &pts[a[0]..b[0]],
                    &corners[a[1]..b[1]],
                    &s[a[2]..b[2]],
                    &f[a[3]..b[3]],
                    &colors[a[3]..b[3]],
                    &hidden[a[3]..b[3]],
                    &edges[a[4]..b[4]],
                    &flags[a[4]..b[4]],
                    h,
                    retain_hidden,
                    exact.map(|e| &e[a[1]..b[1]]),
                );
                result
            })
            .collect() };
        let span = |n: usize| if edit_room && n >= 196608 {
            n + (n / 64).min(16384)
        } else { n };
        let total: usize = prepared
            .iter()
            .map(|p| span(p.triangles.len() * 3 + p.edges.len() * 2))
            .sum();
        let mut bytes = vec![0u8; total * 28];
        let mut indices = vec![0u32; total];
        let mut bounds = Vec::with_capacity(prepared.len() * 8);
        let stride = if retain_hidden { 5 } else { 3 };
        let mut table = Vec::with_capacity(prepared.len() * stride);
        let mut jobs = Vec::new();
        let mut byte_tail = bytes.as_mut_slice();
        let mut index_tail = indices.as_mut_slice();
        let mut first = 0;
        for (i, p) in prepared.iter().enumerate() {
            let nf = p.triangles.len() * 3;
            let n = nf + p.edges.len() * 2;
            bounds.extend_from_slice(&p.bounds);
            table.extend_from_slice(&[
                first as u32,
                (p.visible_triangles * 3) as u32,
                (p.visible_edges * 2) as u32,
            ]);
            if retain_hidden {
                table.extend_from_slice(&[
                    ((p.triangles.len() - p.visible_triangles) * 3) as u32,
                    ((p.edges.len() - p.visible_edges) * 2) as u32,
                ]);
            }
            let (dst, rest) = byte_tail.split_at_mut(span(n) * 28);
            byte_tail = rest;
            let (idx, rest) = index_tail.split_at_mut(span(n));
            index_tail = rest;
            jobs.push((i, p, dst, idx));
            first += span(n);
        }
        if one {
            for (i, p, dst, idx) in jobs {
                fill(&pts[offsets[i][0]..offsets[i + 1][0]], p, dst, idx);
            }
        } else {
            jobs.into_par_iter()
                .for_each(|(i, p, dst, idx)| fill(&pts[offsets[i][0]..offsets[i + 1][0]], p, dst, idx));
        }
        // Legacy callers reserve alpha 254 for matching backs. The parity
        // path keeps opacity in alpha and carries all backs in material rows.
        if !retain_hidden {
            for (i, mesh) in prepared.iter().enumerate() {
                for (triangle, face) in mesh.faces.iter().enumerate() {
                    let f = offsets[i][3] + face;
                    let vertex = table[i * 3] as usize + triangle * 3;
                    let color = u32::from_le_bytes(
                        bytes[vertex * 28 + 24..vertex * 28 + 28]
                            .try_into()
                            .unwrap(),
                    );
                    if paints[f][0] != 0 && paints[f][0] == color && paints[f][1..] == [0, 0] {
                        for corner in 0..3 {
                            bytes[(vertex + corner) * 28 + 27] = 254;
                        }
                    }
                }
                // Clear after marking every triangle of a face, not during it.
                for face in &mesh.faces {
                    let f = offsets[i][3] + face;
                    let color = u32::from_le_bytes(colors[f]);
                    if paints[f][0] == color && paints[f][1..] == [0, 0] {
                        paints[f][0] = 0;
                    }
                }
            }
        }
        let mut materials = vec![0u8; prepared.len() * 16];
        for (i, mesh) in prepared.iter().enumerate() {
            let first_face = offsets[i][3];
            let special = mesh.faces.iter().any(|f| paints[first_face + f] != [0; 3]);
            if !special {
                continue;
            }
            // Matching backs need no 32-byte record per corner. Header bit 0
            // reuses the front vertex RGBA, retaining the full opacity byte.
            let matching = retain_hidden && mesh.triangles.iter().zip(&mesh.faces).all(|((_, _, color), face)| {
                let paint = paints[first_face + face];
                paint[1..] == [0, 0] && paint[0] == u32::from_le_bytes(*color)
            });
            if matching {
                materials[i * 16 + 12..i * 16 + 16].copy_from_slice(&1u32.to_le_bytes());
                continue;
            }
            let length = mesh.triangles.len() * 3
                + if mesh.triangles.len() > mesh.visible_triangles {
                    mesh.visible_edges * 2
                } else {
                    0
                };
            let header = [
                table[i * stride],
                (materials.len() / 16) as u32,
                length as u32,
                0,
            ];
            materials[i * 16..i * 16 + 16].copy_from_slice(
                &header
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
            );
            let points = &pts[offsets[i][0]..offsets[i + 1][0]];
            for (j, ((t, _, _), face)) in mesh.triangles.iter().zip(&mesh.faces).enumerate() {
                if j == mesh.visible_triangles {
                    materials.resize(materials.len() + mesh.visible_edges * 2 * 32, 0);
                }
                let f = first_face + face;
                let mapping: [f32; 16] = std::array::from_fn(|k| maps[[f, k]]);
                for v in t {
                    materials.extend_from_slice(&material_vertex(
                        points[*v as usize],
                        &mapping,
                        paints[f],
                    ));
                }
            }
        }
        (bytes, indices, bounds, table, materials)
    });
    Ok((
        PyArray1::from_vec(py, bytes),
        PyArray1::from_vec(py, indices),
        PyArray1::from_vec(py, bounds),
        PyArray1::from_vec(py, table),
        PyArray1::from_vec(py, materials),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hash-table edge table `edge_table` replaced, as a reference.
    #[allow(clippy::type_complexity)]
    fn edge_table_by_hash(edges: &[[u32; 2]], flags: &[u8], c: &[u32], sizes: &[i64], counts: &[i64])
        -> (Vec<[u32; 2]>, Vec<u8>, Vec<(u32, u32)>, Vec<(usize, usize)>) {
        let mut edge_list = Vec::new();
        let mut edge_flags: Vec<u8> = Vec::new();
        let mut seen: HashMap<(u32, u32), usize> = HashMap::new();
        let mut insert = |a: u32, b: u32, flag: u8| -> Option<usize> {
            if a == b {
                return None;
            }
            let key = (a.min(b), a.max(b));
            Some(match seen.get(&key) {
                Some(&i) => {
                    edge_flags[i] |= flag;
                    i
                }
                None => {
                    seen.insert(key, edge_list.len());
                    edge_list.push([a, b]);
                    edge_flags.push(flag);
                    edge_list.len() - 1
                }
            })
        };
        for (e, f) in edges.iter().zip(flags) {
            insert(e[0], e[1], *f);
        }
        let mut sides = Vec::new();
        let mut face_rings = Vec::new();
        let (mut r, mut k) = (0, 0);
        for (f, &count) in counts.iter().enumerate() {
            face_rings.push((r, k));
            for _ in 0..count {
                let n = sizes[r] as usize;
                let ring = &c[k..k + n];
                for i in 0..n {
                    if let Some(edge) = insert(ring[i], ring[(i + 1) % n], 0) {
                        sides.push((edge as u32, f as u32));
                    }
                }
                k += n;
                r += 1;
            }
        }
        (edge_list, edge_flags, sides, face_rings)
    }

    #[test]
    fn the_bucketed_edge_table_is_the_hashed_one() {
        // Pseudo-random rings over 50 vertices, repeated and reversed
        // edges, degenerate sides, rings with holes.
        let mut x = 12345u64;
        let mut next = |n: u32| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((x >> 33) % n as u64) as u32
        };
        for _ in 0..50 {
            let edges: Vec<[u32; 2]> = (0..40).map(|_| [next(50), next(50)]).collect();
            let flags: Vec<u8> = (0..40).map(|_| next(16) as u8).collect();
            let (mut c, mut sizes, mut counts) = (Vec::new(), Vec::new(), Vec::new());
            for _ in 0..30 {
                let rings = 1 + next(2) as i64;
                counts.push(rings);
                for _ in 0..rings {
                    let n = 3 + next(4) as i64;
                    sizes.push(n);
                    c.extend((0..n).map(|_| next(50)));
                }
            }
            assert_eq!(edge_table(50, &edges, &flags, &c, &sizes, &counts),
                       edge_table_by_hash(&edges, &flags, &c, &sizes, &counts));
        }
        // A fan of 300 triangles around vertex 0.
        let c: Vec<u32> = (1..=300).flat_map(|i| [0, i, i % 300 + 1]).collect();
        let (sizes, counts) = (vec![3i64; 300], vec![1i64; 300]);
        let edges: Vec<[u32; 2]> = (1..=300).map(|i| [0, i]).collect();
        let flags = vec![1u8; 300];
        assert_eq!(edge_table(301, &edges, &flags, &c, &sizes, &counts),
                   edge_table_by_hash(&edges, &flags, &c, &sizes, &counts));
    }
    #[test]
    fn only_soft_shared_corners_receive_fan_normals() {
        let pts = [
            V3(0., 0., 0.),
            V3(1., 0., 0.),
            V3(0., 1., 0.),
            V3(0., 0., 1.),
        ];
        let bake = |flag| {
            prepare(
                &pts,
                &[0, 1, 2, 1, 0, 3],
                &[3, 3],
                &[1, 1],
                &[[255; 4]; 2],
                &[false; 2],
                &[[0, 1]],
                &[flag],
                Hypot::LibCxxFma,
                false,
            )
        };
        let hard = bake(0);
        let soft = bake(1);
        for k in 0..2 {
            let (_, hn, _) = hard.triangles[k];
            let (ids, sn, _) = soft.triangles[k];
            for j in 0..3 {
                let delta = (hn[j].0 - sn[j].0).abs()
                    + (hn[j].1 - sn[j].1).abs()
                    + (hn[j].2 - sn[j].2).abs();
                if ids[j] <= 1 {
                    assert!(delta > 0.1);
                } else {
                    assert!(delta < 1e-6);
                }
            }
        }
        // Hidden edges alone do not request smoothing.
        let hidden = bake(2);
        assert_eq!(hard.triangles[0].1[0].2, hidden.triangles[0].1[0].2);
        // Soft but flat («Smooth normals» off): no line, and no smoothing.
        let flat = bake(1 | 4);
        assert_eq!(flat.edges.len() + 1, hard.edges.len());
        assert_eq!(soft.edges.len(), flat.edges.len());
        for k in 0..2 {
            let (hn, fl) = (hard.triangles[k].1, flat.triangles[k].1);
            for j in 0..3 {
                assert_eq!((hn[j].0, hn[j].1, hn[j].2), (fl[j].0, fl[j].1, fl[j].2));
            }
        }
    }
    #[test]
    fn multiple_mesh_offsets_bounds() {
        let pts = [V3(0., -2., 1.), V3(2., 0., 1.), V3(0., 3., 1.)];
        let p = prepare(
            &pts,
            &[0, 1, 2],
            &[3],
            &[1],
            &[[255; 4]],
            &[false],
            &[],
            &[],
            Hypot::LibCxxFma,
            false,
        );
        assert_eq!(p.bounds, [0., -2., 1., 0., 2., 3., 1., 0.]);
        let n = p.triangles.len() * 3 + p.edges.len() * 2;
        let mut bytes = vec![0; n * 28 * 2];
        let mut indices = vec![0; n * 2];
        for (b, i) in bytes.chunks_mut(n * 28).zip(indices.chunks_mut(n)) {
            fill(&pts, &p, b, i);
        }
        assert_eq!(&bytes[..n * 28], &bytes[n * 28..]);
        assert_eq!(
            indices,
            (0..n as u32).cycle().take(n * 2).collect::<Vec<_>>()
        );
    }
}

// Separate stream preserves the geometry ABI shared with the edge renderer.
fn material_vertex(p: V3, maps: &[f32; 16], paints: [u32; 3]) -> [u8; 32] {
    let mut out = [0; 32];
    for (k, m) in maps.chunks_exact(4).enumerate() {
        let uv = p.0 * m[0] + p.1 * m[1] + p.2 * m[2] + m[3];
        out[k * 4..k * 4 + 4].copy_from_slice(&uv.to_le_bytes());
    }
    for (k, paint) in paints.iter().enumerate() {
        out[16 + k * 4..20 + k * 4].copy_from_slice(&paint.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod material_tests {
    #[test]
    fn corner_affines_and_back_paint_survive_the_stream() {
        let maps = [
            2., 0., 0., 0.5, 0., 3., 0., -1., 0., 0., 4., 2., 1., 1., 0., 0.,
        ];
        let row = super::material_vertex(super::V3(1., 2., 3.), &maps, [0xff332211, 7, 9]);
        let floats: Vec<_> = row[..16]
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        assert_eq!(floats, [2.5, 5., 14., 3.]);
        assert_eq!(&row[16..20], &[0x11, 0x22, 0x33, 0xff]);
        assert_eq!(u32::from_le_bytes(row[20..24].try_into().unwrap()), 7);
        assert_eq!(u32::from_le_bytes(row[24..28].try_into().unwrap()), 9);
    }
}
