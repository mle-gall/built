// SPDX-License-Identifier: GPL-3.0-or-later
//! Broad-phase and exact contacts of a copied or newly drawn stroke.
use crate::{
    draw_plan::Region,
    math::xyz,
    store::{
        commands::cell,
        drawing::{intersection, StrokeSegment},
        geometry::Hypot,
    },
};
use glam::DVec3;
use pyo3::prelude::*;
use std::collections::HashMap;

type Obj<'py> = Bound<'py, PyAny>;
pub(crate) type Bounds = [f64; 6];

fn bounds(mut points: impl Iterator<Item = DVec3>) -> Bounds {
    let Some(first) = points.next() else {
        return [0.; 6];
    };
    let (mut lo, mut hi) = (first, first);
    for p in points {
        lo = lo.min(p);
        hi = hi.max(p);
    }
    [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z]
}
fn padded(mut b: Bounds) -> Bounds {
    let margin = 0.01 + 1e-6 * b.iter().copied().map(f64::abs).fold(0., f64::max);
    for i in 0..3 {
        b[i] -= margin;
        b[i + 3] += margin;
    }
    b
}
pub(crate) fn meets(b: Bounds, w: Bounds) -> bool {
    (0..3).all(|i| b[i + 3] >= w[i] && b[i] <= w[i + 3])
}
fn edge_ends(e: &Obj<'_>) -> PyResult<[DVec3; 2]> {
    Ok([xyz(&e.getattr("a")?)?, xyz(&e.getattr("b")?)?])
}
pub(crate) fn points_box(pts: &Obj<'_>) -> PyResult<Bounds> {
    let pts = pts
        .try_iter()?
        .map(|p| xyz(&p?))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(bounds(pts.into_iter()))
}
pub(crate) fn item_box(item: &Obj<'_>) -> PyResult<Bounds> {
    if let Some(pts) = item.getattr_opt("vertices")? {
        points_box(&pts)
    } else {
        Ok(bounds(edge_ends(item)?.into_iter()))
    }
}
pub(crate) fn window(pts: &Obj<'_>) -> PyResult<Bounds> {
    Ok(padded(points_box(pts)?))
}

pub(crate) fn identity(item: &Obj<'_>) -> PyResult<i128> {
    match item.getattr_opt("key")? {
        Some(k) if !k.is_none() => k.extract(),
        _ => Ok(item.as_ptr() as usize as i128),
    }
}

/// Cached boxes of a running list. Retain the original references so detached
/// preview objects with address-based identity cannot be recycled mid-plan.
#[pyclass(dict)]
pub(crate) struct Nearby {
    boxes: HashMap<i128, (Py<PyAny>, Bounds)>,
}

impl Nearby {
    fn pick<'py>(&mut self, items: &Obj<'py>, w: Bounds) -> PyResult<Vec<Obj<'py>>> {
        let mut hits = Vec::new();
        for item in items.try_iter()? {
            let item = item?;
            let key = identity(&item)?;
            let b = match self.boxes.entry(key) {
                std::collections::hash_map::Entry::Occupied(e) => e.get().1,
                std::collections::hash_map::Entry::Vacant(e) => {
                    let b = item_box(&item)?;
                    e.insert((item.clone().unbind(), b));
                    b
                }
            };
            if meets(b, w) {
                hits.push(item);
            }
        }
        Ok(hits)
    }
}
#[pymethods]
impl Nearby {
    #[new]
    fn new(edges: &Obj<'_>, faces: &Obj<'_>) -> PyResult<Self> {
        let mut boxes = HashMap::new();
        for items in [edges, faces] {
            for item in items.try_iter()? {
                let item = item?;
                boxes.insert(identity(&item)?, (item.clone().unbind(), item_box(&item)?));
            }
        }
        Ok(Self { boxes })
    }
    #[staticmethod]
    fn _points_box(pts: &Obj<'_>) -> PyResult<Bounds> {
        points_box(pts)
    }
    #[staticmethod]
    fn _window(pts: &Obj<'_>) -> PyResult<Bounds> {
        window(pts)
    }
    fn edges<'py>(
        &mut self,
        edges: &Obj<'py>,
        a: &Obj<'py>,
        b: &Obj<'py>,
    ) -> PyResult<Vec<Obj<'py>>> {
        self.pick(edges, padded(bounds([xyz(a)?, xyz(b)?].into_iter())))
    }
    fn faces<'py>(
        &mut self,
        faces: &Obj<'py>,
        a: &Obj<'py>,
        b: &Obj<'py>,
    ) -> PyResult<Vec<Obj<'py>>> {
        self.edges(faces, a, b)
    }
    fn faces_around<'py>(&mut self, faces: &Obj<'py>, pts: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
        self.pick(faces, window(pts)?)
    }
}

#[pyfunction]
pub(crate) fn find_duplicate_edge<'py>(
    edges: &Obj<'py>,
    a: &Obj<'py>,
    b: &Obj<'py>,
) -> PyResult<Option<Obj<'py>>> {
    let (a, b) = (cell(xyz(a)?.to_array()), cell(xyz(b)?.to_array()));
    if a == b {
        return Ok(None);
    }
    for e in edges.try_iter()? {
        let e = e?;
        let [c, d] = edge_ends(&e)?.map(|p| cell(p.to_array()));
        if (a == c && b == d) || (a == d && b == c) {
            return Ok(Some(e));
        }
    }
    Ok(None)
}

#[pyfunction]
pub(crate) fn touching_segments<'py>(
    segments: &Obj<'py>,
    edges: &Obj<'py>,
    faces: &Obj<'py>,
) -> PyResult<Vec<(Obj<'py>, Obj<'py>)>> {
    if !segments.is_truthy()? {
        return Ok(vec![]);
    }
    let mut lines = Vec::new();
    for e in edges.try_iter()? {
        let ends = edge_ends(&e?)?;
        lines.push((
            bounds(ends.into_iter()),
            ends,
            StrokeSegment::new(ends[0], ends[1]),
        ));
    }
    let mut regions = Vec::new();
    for f in faces.try_iter()? {
        let f = f?;
        regions.push((item_box(&f)?, f, None));
    }
    let compensated = segments.py().version_info().minor >= 12;
    let mut out = Vec::new();
    for pair in segments.try_iter()? {
        let pair = pair?;
        let (a, b) = (pair.get_item(0)?, pair.get_item(1)?);
        let (pa, pb) = (xyz(&a)?, xyz(&b)?);
        let w = padded(bounds([pa, pb].into_iter()));
        let path = StrokeSegment::new(pa, pb);
        let segment = [pa.to_array(), pb.to_array()];
        if lines.iter().any(|&(bbox, ends, ref line)| {
            meets(bbox, w)
                && (intersection(
                    segment,
                    ends.map(|p| p.to_array()),
                    Hypot::LibStdCxx,
                    compensated,
                )
                .is_some()
                    || line.contains(pa, 1e-4)
                    || path.contains(ends[0], 1e-4))
        }) {
            out.push((a, b));
            continue;
        }
        for (bbox, f, cache) in &mut regions {
            if !meets(*bbox, w) {
                continue;
            }
            if cache.is_none() {
                *cache = Some(Region::new(f)?);
            }
            if cache.as_ref().unwrap().contacts(pa, pb) {
                out.push((a, b));
                break;
            }
        }
    }
    Ok(out)
}
