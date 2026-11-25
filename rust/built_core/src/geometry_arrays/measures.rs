// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use std::ops::Range;
struct Indexed {
    positions: Vec<[f64; 3]>,
    corners: Vec<usize>,
    rings: Vec<Range<usize>>,
    faces: Vec<Range<usize>>,
}
fn ranges(sizes: Vec<i64>, length: usize) -> PyResult<Vec<Range<usize>>> {
    let mut start = 0usize;
    let mut ranges = Vec::with_capacity(sizes.len());
    for n in sizes {
        let end = usize::try_from(n)
            .ok()
            .and_then(|n| start.checked_add(n))
            .filter(|end| *end <= length && *end > start)
            .ok_or_else(|| PyValueError::new_err("invalid ring counts"))?;
        ranges.push(start..end);
        start = end;
    }
    if start != length {
        return Err(PyValueError::new_err("incomplete ring counts"));
    }
    Ok(ranges)
}
impl Indexed {
    fn read(a: &Bundle) -> PyResult<Self> {
        let positions = row_values(&a.getattr("pos")?)?;
        let corners = indices(&a.getattr("corners")?)?
            .into_iter()
            .map(|i| {
                usize::try_from(i)
                    .ok()
                    .filter(|i| *i < positions.len())
                    .ok_or_else(|| PyValueError::new_err("corner outside vertex array"))
            })
            .collect::<PyResult<Vec<_>>>()?;
        let rings = ranges(indices(&a.getattr("ring_sizes")?)?, corners.len())?;
        let faces = ranges(indices(&a.getattr("ring_counts")?)?, rings.len())?;
        Ok(Self {
            positions,
            corners,
            rings,
            faces,
        })
    }
    fn counts(&self) -> Vec<i64> {
        let mut by_edge = BTreeMap::<_, i64>::new();
        for r in &self.rings {
            let loop_ = &self.corners[r.clone()];
            for (&a, &b) in loop_.iter().zip(loop_.iter().cycle().skip(1)) {
                *by_edge.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        by_edge.into_values().collect()
    }
}
pub(super) fn edge_counts<'p>(a: &Bundle<'p>) -> PyResult<Obj<'p>> {
    let geo = Indexed::read(a)?;
    Ok(col(a.py(), a.py().detach(|| geo.counts())))
}
struct Stats {
    gross: Vec<f64>,
    net: Vec<f64>,
    normals: Vec<[f64; 3]>,
    starts: Vec<i64>,
    signed: f64,
    perimeter: f64,
    balanced: bool,
    lo: [f64; 3],
    hi: [f64; 3],
    closed: bool,
}
fn compute(g: &Indexed, counts: Option<Vec<i64>>) -> Stats {
    let counts = counts.unwrap_or_else(|| g.counts());
    let closed = counts.iter().any(|n| *n != 0) && counts.iter().all(|n| matches!(*n, 0 | 2));
    let origin = DVec3::from(g.positions[g.corners[0]]);
    let (mut lo, mut hi) = (origin, origin);
    let mut gross = vec![];
    let mut net = vec![];
    let mut normals = vec![];
    let mut starts = vec![];
    let (mut perimeter, mut signed, mut area) = (0., 0., 0.);
    let mut balance = DVec3::ZERO;
    for face in &g.faces {
        let mut outer = DVec3::ZERO;
        let mut face_area = 0.;
        for i in face.clone() {
            let corners = &g.corners[g.rings[i].clone()];
            let mut area2 = DVec3::ZERO;
            for (&a, &b) in corners.iter().zip(corners.iter().cycle().skip(1)) {
                let p = DVec3::from(g.positions[a]);
                let q = DVec3::from(g.positions[b]);
                lo = lo.min(p);
                hi = hi.max(p);
                area2 += (p - origin).cross(q - origin);
                perimeter += p.distance(q);
            }
            let vec = area2 * 0.5;
            let length = vec.length();
            area += length;
            if i == face.start {
                outer = vec;
                gross.push(length);
                normals.push((vec / length.max(1e-300)).to_array());
                starts.push(g.rings[i].start as i64);
                face_area = length;
            } else {
                face_area -= length;
            }
            let corrected = if i != face.start && outer.dot(vec) > 0. {
                -vec
            } else {
                vec
            };
            balance += corrected;
            signed += (DVec3::from(g.positions[corners[0]]) - origin).dot(corrected);
        }
        net.push(face_area.max(0.));
    }
    Stats {
        gross,
        net,
        normals,
        starts,
        signed: signed / 3.,
        perimeter,
        balanced: balance.length() <= 1e-10 * area,
        lo: lo.to_array(),
        hi: hi.to_array(),
        closed,
    }
}
#[pyfunction]
pub(super) fn measurements<'p>(a: &Bundle<'p>) -> PyResult<Obj<'p>> {
    let py = a.py();
    if let Some(x) = &a.borrow()._measurements {
        return Ok(x.bind(py).clone());
    }
    if a.getattr("ring_counts")?.len()? == 0 {
        return Ok(none(py));
    }
    let g = Indexed::read(a)?;
    let counts = a
        .borrow()
        .edge_face_counts
        .as_ref()
        .map(|v| indices(v.bind(py)))
        .transpose()?;
    let s = py.detach(|| compute(&g, counts));
    let data = (
        s.gross.into_pyarray(py),
        s.net.into_pyarray(py),
        rows(py, s.normals)?,
        s.closed.then_some(s.signed.abs()),
        s.closed,
        col(py, s.starts),
    )
        .into_bound_py_any(py)?;
    let mut a = a.borrow_mut();
    a._balanced = s.balanced;
    a._perimeter = s.perimeter;
    a._signed_volume = if s.closed { s.signed } else { 0. };
    a._bounds = Some((s.lo, s.hi));
    a._measurements = Some(data.clone().unbind());
    Ok(data)
}
pub(super) type Totals = (f64, f64, Option<f64>, bool, usize);
#[pyfunction]
pub(super) fn quantities(arrays: &Obj) -> PyResult<Totals> {
    let a = match arrays.cast::<Arrays>() {
        Ok(a) => a.clone(),
        Err(_) => super::placements::assembled(arrays)?,
    };
    let d = measurements(&a)?;
    if d.is_none() {
        return Ok((0., 0., None, false, 0));
    }
    Ok((
        sum(&d, 0)?,
        sum(&d, 1)?,
        d.get_item(3)?.extract()?,
        d.get_item(4)?.is_truthy()?,
        d.get_item(0)?.len()?,
    ))
}
pub(super) fn sum(d: &Obj, index: usize) -> PyResult<f64> {
    d.get_item(index)?.call_method0("sum")?.extract()
}
