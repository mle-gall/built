//! Native picking kernels (R2.5 slice 2, `docs/rust-tools.md` §15).
//!
//! What lies under the cursor and what a selection box catches, answered
//! from the store's arrays and the packed instance rows: the ray against
//! triangles, the projection to pixels, the segment/rectangle tests, the
//! loose mesh's box catch and the placements' box catch. Every function is
//! plain Rust over slices; the `#[pyfunction]`s at the end take NumPy
//! arrays. Projection follows the views' `_project_px`: column-major
//! `mvp`, `ok` when clip w > 0, y down.

use numpy::{AllowTypeChange, PyArrayLike1, PyArrayLikeDyn, IntoPyArray, PyArray1, PyReadonlyArray1, PyReadonlyArray2, PyUntypedArrayMethods};
use pyo3::prelude::*;

pub type P = [f64; 3];

/// Barycentric slack of the GL picker (`_BARY_EPS`).
pub const BARY_EPS: f64 = 1e-14;

fn sub(a: P, b: P) -> P { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn dot(a: P, b: P) -> f64 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn cross(a: P, b: P) -> P { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }
fn norm(a: P) -> f64 { dot(a, a).sqrt() }

/// Möller–Trumbore of `o + t·d` against one triangle `(v0, e1, e2)` with
/// the thresholds of `views.document_interaction._moller_trumbore`:
/// `Some(t)` on a hit.
pub fn ray_triangle(o: P, d: P, v0: P, e1: P, e2: P) -> Option<f64> {
    let p = cross(d, e2);
    let det = dot(e1, p);
    let mut ok = det.abs() > 1e-6;
    if !ok && det != 0.0 {
        // A tiny triangle met head-on: read the angle, not the area (#1294).
        ok = det.abs() > 1e-6 * norm(cross(e1, e2)) * norm(d);
    }
    if !ok {
        return None;
    }
    let inv = 1.0 / det;
    let s = sub(o, v0);
    let u = dot(s, p) * inv;
    let q = cross(s, e1);
    let v = dot(q, d) * inv;
    let t = dot(e2, q) * inv;
    (u >= -BARY_EPS && u <= 1.0 + BARY_EPS && v >= -BARY_EPS && u + v <= 1.0 + BARY_EPS && t > 1e-6).then_some(t)
}

/// A camera's projection to view pixels.
#[derive(Clone, Copy, Debug)]
pub struct Projector {
    /// Row-major rows 0, 1 and 3 of the clip transform.
    r0: [f64; 4],
    r1: [f64; 4],
    r3: [f64; 4],
    w: f64,
    h: f64,
}

impl Projector {
    /// `mvp` column-major (`Matrix4.data()`), view `w` × `h` pixels.
    pub fn new(mvp: &[f64; 16], w: f64, h: f64) -> Self {
        let row = |r: usize| [mvp[r], mvp[4 + r], mvp[8 + r], mvp[12 + r]];
        Projector { r0: row(0), r1: row(1), r3: row(3), w, h }
    }

    /// `(x, y, ok)`: pixels, and whether the point is in front of the eye.
    pub fn px(&self, p: P) -> (f64, f64, bool) {
        let c = |r: &[f64; 4]| p[0] * r[0] + p[1] * r[1] + p[2] * r[2] + r[3];
        let cw = c(&self.r3);
        let ok = cw > 0.0;
        let w = if ok { cw } else { 1.0 };
        ((c(&self.r0) / w * 0.5 + 0.5) * self.w, (1.0 - (c(&self.r1) / w * 0.5 + 0.5)) * self.h, ok)
    }
}

/// `[x0, y0, x1, y1]`, normalised.
pub type Rect = [f64; 4];

pub fn inside(rect: &Rect, x: f64, y: f64) -> bool { x >= rect[0] && x <= rect[2] && y >= rect[1] && y <= rect[3] }

/// Window: both ends inside. Crossing: an end inside, or the segment
/// through one of the four sides (`tools.select._seg_rect_mask`).
pub fn seg_rect(a: (f64, f64), b: (f64, f64), ok: bool, rect: &Rect, crossing: bool) -> bool {
    if !ok {
        return false;
    }
    let (ia, ib) = (inside(rect, a.0, a.1), inside(rect, b.0, b.1));
    if !crossing {
        return ia && ib;
    }
    if ia || ib {
        return true;
    }
    let [x0, y0, x1, y1] = *rect;
    for (px, py, qx, qy) in [(x0, y0, x1, y0), (x1, y0, x1, y1), (x1, y1, x0, y1), (x0, y1, x0, y0)] {
        let d1 = (qx - px) * (a.1 - py) - (qy - py) * (a.0 - px);
        let d2 = (qx - px) * (b.1 - py) - (qy - py) * (b.0 - px);
        let d3 = (b.0 - a.0) * (py - a.1) - (b.1 - a.1) * (px - a.0);
        let d4 = (b.0 - a.0) * (qy - a.1) - (b.1 - a.1) * (qx - a.0);
        if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
            return true;
        }
    }
    false
}

/// What a box catches of a mesh given as the store's dense arrays: the
/// edges (both ends in front of the eye, window or crossing) and the faces
/// (every corner in front; window: every corner inside; crossing: a ring
/// side through the box). Returns the edge and face ordinals.
pub fn box_mesh(pos: &[P], edges: &[[i64; 2]], corners: &[i64], ring_sizes: &[i64], ring_counts: &[i64],
                proj: &Projector, rect: &Rect, crossing: bool) -> (Vec<i64>, Vec<i64>) {
    let px: Vec<(f64, f64, bool)> = pos.iter().map(|p| proj.px(*p)).collect();
    let at = |i: i64| px[i as usize];
    let mut out_e = Vec::new();
    for (k, e) in edges.iter().enumerate() {
        let (a, b) = (at(e[0]), at(e[1]));
        if seg_rect((a.0, a.1), (b.0, b.1), a.2 && b.2, rect, crossing) {
            out_e.push(k as i64);
        }
    }
    let mut out_f = Vec::new();
    let (mut ring, mut corner) = (0usize, 0usize);
    for (f, &count) in ring_counts.iter().enumerate() {
        let rings = &ring_sizes[ring..ring + count as usize];
        ring += count as usize;
        let total: i64 = rings.iter().sum();
        let span = &corners[corner..corner + total as usize];
        corner += total as usize;
        let mut start = 0;
        let points = rings.iter().map(|&size| {
            let ring = &span[start..start + size as usize];
            start += size as usize;
            ring.iter().map(|&i| at(i))
        });
        if face_in_box(points, rect, crossing) {
            out_f.push(f as i64);
        }
    }
    (out_e, out_f)
}

/// A hidden corner invalidates a face even if an earlier ring crossed the box.
pub(crate) fn face_in_box(
    rings: impl IntoIterator<Item = impl Iterator<Item = (f64, f64, bool)>>,
    rect: &Rect,
    crossing: bool,
) -> bool {
    let mut hit = false;
    for mut ring in rings {
        let Some(first) = ring.next() else {continue};
        let mut prev = first;
        for p in ring.chain(std::iter::once(first)) {
            if !p.2 {return false}
            if crossing {
                hit |= seg_rect((prev.0, prev.1), (p.0, p.1), true, rect, true);
            } else if inside(rect, p.0, p.1) {
                hit = true;
            } else {
                return false;
            }
            prev = p;
        }
    }
    hit
}

/// A point placed by a packed row matrix (row vector times the stored 4×4,
/// translation in the last row: `tools.select._packed_world`).
fn placed(p: P, w: &[f64; 16]) -> P {
    [p[0] * w[0] + p[1] * w[4] + p[2] * w[8] + w[12],
     p[0] * w[1] + p[1] * w[5] + p[2] * w[9] + w[13],
     p[0] * w[2] + p[1] * w[6] + p[2] * w[10] + w[14]]
}

/// One prototype's points as the box reads them: triangle corners and
/// edge ends (pairs).
pub struct Proto {
    pub tris: Vec<P>,
    pub edges: Vec<P>,
}

/// Exact box test of one placed prototype (`_packed_rows_exact` for one row).
fn row_exact(proto: &Proto, w: &[f64; 16], proj: &Projector, rect: &Rect, crossing: bool) -> bool {
    let mut any_point = false;
    let mut all_inside = true;
    for p in proto.tris.iter().chain(proto.edges.iter()) {
        any_point = true;
        let (x, y, ok) = proj.px(placed(*p, w));
        let inn = ok && inside(rect, x, y);
        if crossing && inn {
            return true;
        }
        if !crossing && !inn {
            return false;
        }
        all_inside &= inn;
    }
    if !any_point {
        return false;
    }
    if !crossing {
        return all_inside;
    }
    let seg = |a: P, b: P| {
        let (pa, pb) = (proj.px(placed(a, w)), proj.px(placed(b, w)));
        seg_rect((pa.0, pa.1), (pb.0, pb.1), pa.2 && pb.2, rect, true)
    };
    for t in proto.tris.chunks_exact(3) {
        if seg(t[0], t[1]) || seg(t[1], t[2]) || seg(t[2], t[0]) {
            return true;
        }
    }
    proto.edges.chunks_exact(2).any(|e| seg(e[0], e[1]))
}

/// The objects a box catches from the packed rows (`_box_groups_packed`):
/// each row belongs to object `owner[row]` (negative: none) and draws
/// prototype `code[row]` placed by `world[row]`. Rows whose prototype box
/// lands wholly inside or past one side decide without their points; the
/// rest take the exact test. Window: an object is caught when every row
/// is; crossing: when one row is. Returns which of `n_owners` objects.
pub fn box_rows(world: &[[f64; 16]], code: &[i64], owner: &[i64], drawn: &[bool], lo: &[P], hi: &[P],
                protos: &mut dyn FnMut(usize) -> Proto, n_owners: usize, proj: &Projector, rect: &Rect,
                crossing: bool) -> Vec<bool> {
    let mut present = vec![false; n_owners];
    // Crossing: caught by one row. Window: vetoed by one row.
    let mut decided = vec![false; n_owners];
    let mut unsure = Vec::new();
    for r in 0..world.len() {
        if owner[r] < 0 || !drawn[r] {
            continue;
        }
        let o = owner[r] as usize;
        present[o] = true;
        let c = code[r] as usize;
        let (l, h) = (lo[c], hi[c]);
        let (mut front, mut all_in) = (true, true);
        let (mut left, mut right, mut up, mut down) = (true, true, true, true);
        for k in 0..8 {
            let p = [if k & 4 != 0 { h[0] } else { l[0] }, if k & 2 != 0 { h[1] } else { l[1] },
                     if k & 1 != 0 { h[2] } else { l[2] }];
            let (x, y, ok) = proj.px(placed(p, &world[r]));
            front &= ok;
            all_in &= inside(rect, x, y);
            left &= x < rect[0];
            right &= x > rect[2];
            up &= y < rect[1];
            down &= y > rect[3];
        }
        let is_in = front && all_in;
        let is_out = front && (left || right || up || down);
        if crossing && is_in || !crossing && is_out {
            decided[o] = true;
        } else if !is_in && !is_out {
            unsure.push(r);
        }
    }
    let mut loaded: std::collections::HashMap<usize, Proto> = Default::default();
    // Prototype by prototype, as the points are fetched once each.
    unsure.sort_by_key(|&r| code[r]);
    for r in unsure {
        let o = owner[r] as usize;
        if decided[o] {
            continue;
        }
        let c = code[r] as usize;
        let proto = loaded.entry(c).or_insert_with(|| protos(c));
        if row_exact(proto, &world[r], proj, rect, crossing) == crossing {
            decided[o] = true;
        }
    }
    (0..n_owners).map(|o| if crossing { decided[o] } else { present[o] && !decided[o] }).collect()
}

/// The segments within `radius` pixels of `(x, y)`, nearest first:
/// `(index, pixels, t along the segment)`; ends behind the eye skip it.
pub(crate) fn nearest_projected(
    segments: impl Iterator<Item=((f64,f64,bool),(f64,f64,bool))>, x:f64, y:f64, radius:f64
) -> Vec<(usize, f64, f64)> {
    let mut out = Vec::new();
    for (i,(pa,pb)) in segments.enumerate() {
        if !pa.2 || !pb.2 {
            continue;
        }
        let (dx, dy) = (pb.0 - pa.0, pb.1 - pa.1);
        let t = (((x - pa.0) * dx + (y - pa.1) * dy) / (dx * dx + dy * dy).max(1e-12)).clamp(0.0, 1.0);
        let dist = (pa.0 + t * dx - x).hypot(pa.1 + t * dy - y);
        if dist < radius {
            out.push((i, dist, t));
        }
    }
    out.sort_by(|p, q| p.1.total_cmp(&q.1).then(p.0.cmp(&q.0)));
    out
}

// ---- Python ----------------------------------------------------------------

fn rows3(a: &PyReadonlyArray2<'_, f64>) -> PyResult<Vec<P>> {
    let s = a.as_slice()?;
    Ok(s.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect())
}

fn mat16(m: Vec<f64>) -> PyResult<[f64; 16]> {
    m.try_into().map_err(|_| pyo3::exceptions::PyValueError::new_err("a 4x4 matrix has 16 values"))
}

/// Preserve the row-major reshape of an incoming triangle stream. Cached f64
/// streams are read in place; sliced and Fortran-ordered arrays keep logical order.
fn triangle_stream<'a>(array: &'a PyArrayLikeDyn<'_, f64, AllowTypeChange>)
    -> PyResult<std::borrow::Cow<'a, [f64]>>
{
    if array.is_c_contiguous() {
        Ok(std::borrow::Cow::Borrowed(array.as_slice()?))
    } else {
        Ok(std::borrow::Cow::Owned(array.as_array().iter().copied().collect()))
    }
}

/// Distances and hit flags for a ray and three aligned triangle streams.
#[pyfunction]
pub fn ray_triangles<'py>(
    py: Python<'py>,
    o: PyArrayLike1<'py, f64, AllowTypeChange>,
    d: PyArrayLike1<'py, f64, AllowTypeChange>,
    v0: PyArrayLikeDyn<'py, f64, AllowTypeChange>,
    e1: PyArrayLikeDyn<'py, f64, AllowTypeChange>,
    e2: PyArrayLikeDyn<'py, f64, AllowTypeChange>,
) -> PyResult<(Bound<'py, PyArray1<f64>>, Bound<'py, PyArray1<bool>>)> {
    let (o, d) = (o.as_array(), d.as_array());
    if o.len() != 3 || d.len() != 3 {
        return Err(pyo3::exceptions::PyValueError::new_err("expected three ray coordinates"));
    }
    let (o, d) = ([o[0], o[1], o[2]], [d[0], d[1], d[2]]);
    let (v0, e1, e2) = (triangle_stream(&v0)?, triangle_stream(&e1)?, triangle_stream(&e2)?);
    if v0.len() % 3 != 0 || v0.len() != e1.len() || v0.len() != e2.len() {
        return Err(pyo3::exceptions::PyValueError::new_err("expected matching XYZ triangle streams"));
    }
    let mut times = Vec::with_capacity(v0.len() / 3);
    let mut hits = Vec::with_capacity(times.capacity());
    for ((start, a), b) in v0.chunks_exact(3).zip(e1.chunks_exact(3)).zip(e2.chunks_exact(3)) {
        let hit = ray_triangle(o, d, [start[0], start[1], start[2]],
                               [a[0], a[1], a[2]], [b[0], b[1], b[2]]);
        times.push(hit.unwrap_or(0.0));
        hits.push(hit.is_some());
    }
    Ok((times.into_pyarray(py), hits.into_pyarray(py)))
}

/// `(xs, ys, ok)` of points (N, 3) through `mvp` (column-major) into a
/// `w` × `h` view.
#[pyfunction]
pub fn project_px<'py>(py: Python<'py>, points: PyReadonlyArray2<'py, f64>, mvp: Vec<f64>, w: f64, h: f64)
    -> PyResult<(Bound<'py, PyArray1<f64>>, Bound<'py, PyArray1<f64>>, Bound<'py, PyArray1<bool>>)> {
    let proj = Projector::new(&mat16(mvp)?, w, h);
    let pts = rows3(&points)?;
    let (mut xs, mut ys, mut ok) = (Vec::with_capacity(pts.len()), Vec::with_capacity(pts.len()), Vec::with_capacity(pts.len()));
    for p in pts {
        let (x, y, k) = proj.px(p);
        xs.push(x);
        ys.push(y);
        ok.push(k);
    }
    Ok((xs.into_pyarray(py), ys.into_pyarray(py), ok.into_pyarray(py)))
}

/// The rectangle test of N screen segments (`_seg_rect_mask`).
#[pyfunction]
#[allow(clippy::too_many_arguments)]
pub fn seg_rect_mask<'py>(py: Python<'py>, ax: PyReadonlyArray1<'py, f64>, ay: PyReadonlyArray1<'py, f64>,
                         bx: PyReadonlyArray1<'py, f64>, by: PyReadonlyArray1<'py, f64>, ok: PyReadonlyArray1<'py, bool>,
                         rect: [f64; 4], crossing: bool) -> PyResult<Bound<'py, PyArray1<bool>>> {
    let (ax, ay, bx, by, ok) = (ax.as_slice()?, ay.as_slice()?, bx.as_slice()?, by.as_slice()?, ok.as_slice()?);
    let out: Vec<bool> = (0..ax.len()).map(|i| seg_rect((ax[i], ay[i]), (bx[i], by[i]), ok[i], &rect, crossing)).collect();
    Ok(out.into_pyarray(py))
}

/// The loose mesh's box catch from the store's arrays (`box_mesh`):
/// `(edge ordinals, face ordinals)`.
#[pyfunction]
#[allow(clippy::too_many_arguments)]
pub fn box_mesh_arrays<'py>(py: Python<'py>, pos: PyReadonlyArray2<'py, f64>, edges: PyReadonlyArray2<'py, i64>,
                           corners: PyReadonlyArray1<'py, i64>, ring_sizes: PyReadonlyArray1<'py, i64>,
                           ring_counts: PyReadonlyArray1<'py, i64>, mvp: Vec<f64>, w: f64, h: f64,
                           rect: [f64; 4], crossing: bool)
    -> PyResult<(Bound<'py, PyArray1<i64>>, Bound<'py, PyArray1<i64>>)> {
    let proj = Projector::new(&mat16(mvp)?, w, h);
    let pos = rows3(&pos)?;
    let edges: Vec<[i64; 2]> = edges.as_slice()?.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
    let (corners, sizes, counts) = (corners.as_slice()?.to_vec(), ring_sizes.as_slice()?.to_vec(), ring_counts.as_slice()?.to_vec());
    let (e, f) = py.detach(|| box_mesh(&pos, &edges, &corners, &sizes, &counts, &proj, &rect, crossing));
    Ok((e.into_pyarray(py), f.into_pyarray(py)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ortho() -> Projector {
        // x, y in [-1, 1] → pixels 0..100, w = 1.
        let mut m = [0.0; 16];
        m[0] = 1.0;
        m[5] = 1.0;
        m[10] = 1.0;
        m[15] = 1.0;
        Projector::new(&m, 100.0, 100.0)
    }

    #[test]
    fn projection_matches_the_views() {
        let (x, y, ok) = ortho().px([0.0, 0.0, 0.0]);
        assert_eq!((x, y, ok), (50.0, 50.0, true));
        let (x, y, _) = ortho().px([1.0, 1.0, 0.0]);
        assert_eq!((x, y), (100.0, 0.0));
    }

    #[test]
    fn window_and_crossing_boxes_on_a_loose_square() {
        let pos = [[-0.5, -0.5, 0.0], [0.5, -0.5, 0.0], [0.5, 0.5, 0.0], [-0.5, 0.5, 0.0]];
        let edges = [[0, 1], [1, 2], [2, 3], [3, 0]];
        let corners = [0, 1, 2, 3];
        let proj = ortho();
        let (e, f) = box_mesh(&pos, &edges, &corners, &[4], &[1], &proj, &[0.0, 0.0, 100.0, 100.0], false);
        assert_eq!((e.len(), f), (4, vec![0]));
        // A box over the right half: window takes the right edge only.
        let (e, f) = box_mesh(&pos, &edges, &corners, &[4], &[1], &proj, &[60.0, 0.0, 100.0, 100.0], false);
        assert_eq!((e, f.len()), (vec![1], 0));
        let (e, f) = box_mesh(&pos, &edges, &corners, &[4], &[1], &proj, &[60.0, 0.0, 100.0, 100.0], true);
        assert_eq!((e, f), (vec![0, 1, 2], vec![0]));
    }

    #[test]
    fn packed_rows_window_needs_every_row_crossing_one() {
        let tris = [[-0.1, -0.1, 0.0], [0.1, -0.1, 0.0], [0.0, 0.1, 0.0]];
        let mut protos = |_c: usize| Proto { tris: tris.to_vec(), edges: vec![] };
        let mut a = [0.0; 16];
        a[0] = 1.0; a[5] = 1.0; a[10] = 1.0; a[15] = 1.0;
        let mut b = a;
        b[12] = 0.8; // the second row of object 0 sits at x = 0.8
        let lo = [[-0.1, -0.1, 0.0]];
        let hi = [[0.1, 0.1, 0.0]];
        let proj = ortho();
        let rect = [0.0, 0.0, 70.0, 100.0];
        let world = [a, b, a];
        let hit = box_rows(&world, &[0, 0, 0], &[0, 0, 1], &[true; 3], &lo, &hi, &mut protos, 2, &proj, &rect, false);
        assert_eq!(hit, vec![false, true]);
        let hit = box_rows(&world, &[0, 0, 0], &[0, 0, 1], &[true; 3], &lo, &hi, &mut protos, 2, &proj, &rect, true);
        assert_eq!(hit, vec![true, true]);
    }

    #[test]
    fn nearest_segment_first() {
        let a = [[-1.0, 0.0, 0.0], [-1.0, 0.5, 0.0]];
        let b = [[1.0, 0.0, 0.0], [1.0, 0.5, 0.0]];
        let proj = ortho();
        let segments = a.into_iter().zip(b).map(|(a, b)| (proj.px(a), proj.px(b)));
        let found = nearest_projected(segments, 50.0, 30.0, 30.0);
        assert_eq!(found.iter().map(|f| f.0).collect::<Vec<_>>(), vec![1, 0]);
    }
}

/// Cached triangles of the visible faces, in document order. A ray returns
/// only the winning ordinal across the bridge, including while faces move.
#[pyclass(module = "built_core")]
pub struct FacePicker {
    triangles: Vec<[P; 3]>,
    faces: Vec<(i128, usize, std::ops::Range<usize>)>,
}

impl FacePicker {
    fn empty() -> Self { Self { triangles: Vec::new(), faces: Vec::new() } }
    fn append(&mut self, key:i128, ordinal:usize, triangles: impl IntoIterator<Item=[P;3]>) {
        let start = self.triangles.len();
        self.triangles.extend(triangles.into_iter().map(|[a,b,c]| [a,sub(b,a),sub(c,a)]));
        self.faces.push((key,ordinal,start..self.triangles.len()));
    }
    pub(crate) fn from_store(store:&crate::store::Store, hypot:crate::store::geometry::Hypot) -> Self {
        use crate::store::keys::Kind;
        let mut picker = Self::empty();
        for (i,slot) in store.live_slots(Kind::Face).into_iter().enumerate() {
            picker.append(store.key(Kind::Face,slot) as i128, i, store.face_triangles(slot,None,hypot));
        }
        picker
    }
}

#[pymethods]
impl FacePicker {
    #[new]
    fn new(faces:&Bound<'_,PyAny>) -> PyResult<Self> {
        let mut picker = Self::empty();
        for (i, face) in faces.try_iter()?.enumerate() {
            let face = face?;
            if face.getattr_opt("hidden")?.map(|v|v.is_truthy()).transpose()?.unwrap_or(false) {
                continue;
            }
            let mut triangles = Vec::new();
            for tri in face.call_method0("triangulate")?.try_iter()? {
                let tri = tri?;
                let [a,b,c] = [0,1,2].map(|i| crate::math::xyz(&tri.get_item(i)?).map(|v|v.to_array()));
                triangles.push([a?, b?, c?]);
            }
            picker.append(face.getattr("key")?.extract()?,i,triangles);
        }
        Ok(picker)
    }
    fn first(&self, py:Python<'_>, origin:P, direction:P, excluded:std::collections::HashSet<i128>) -> Option<usize> {
        py.detach(|| {
            let mut best = None;
            let mut distance = f64::INFINITY;
            for (key,ordinal,range) in &self.faces {
                if excluded.contains(key) {continue;}
                for &[a,e1,e2] in &self.triangles[range.clone()] {
                    if let Some(t) = ray_triangle(origin,direction,a,e1,e2) {
                        if t<distance {distance=t; best=Some(*ordinal);}
                    }
                }
            }
            best
        })
    }
}
