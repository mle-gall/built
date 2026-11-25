// SPDX-License-Identifier: GPL-3.0-or-later
//! Classify a repaired plane using the two occupancy probes in sample order.
//! Keep creases between paints and distinguish pushed seams from user edges.
use crate::math::{xyz, Vec3};
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyBool, PyDict, PyList},
};
type XY = [f64; 2];
type Ring = Vec<XY>;
type Region = (Ring, Vec<Ring>);
type Segment = (XY, XY);
type Obj<'py> = Bound<'py, PyAny>;

enum Surface {
    Boundary(bool),
    Partition,
    Sheet,
    InteriorSeam,
    Discard,
}

fn surface(
    plus: bool,
    minus: bool,
    old: bool,
    skin: u8,
    fresh: u8,
    keep: bool,
    removing: bool,
) -> Surface {
    if plus != minus {
        return Surface::Boundary(plus);
    }
    let decl = if keep { 0 } else { fresh };
    if (1..=2).contains(&decl) && plus == removing {
        return Surface::Boundary(decl == 2);
    }
    if plus {
        if !removing
            && !keep
            && ((skin & 1 != 0 && decl & 2 != 0) || (skin & 2 != 0 && decl & 1 != 0))
        {
            Surface::InteriorSeam
        } else if (!removing || decl == 0) && (old || (keep && fresh != 0)) {
            Surface::Partition
        } else {
            Surface::Discard
        }
    } else if decl == 0 && old {
        Surface::Sheet
    } else {
        Surface::Discard
    }
}

struct Frame {
    origin: DVec3,
    u: DVec3,
    v: DVec3,
    n: DVec3,
}
impl Frame {
    fn project(&self, p: DVec3) -> XY {
        let d = p - self.origin;
        [d.dot(self.u), d.dot(self.v)]
    }
    fn lift(&self, [x, y]: XY) -> DVec3 {
        self.origin + self.u * x + self.v * y
    }
    fn ring(&self, points: Ring, reverse: bool) -> Vec<Vec3> {
        let mut out: Vec<_> = points.into_iter().map(|p| self.lift(p).into()).collect();
        if reverse {
            out.reverse();
        }
        out
    }
}

fn keeps_paint(radial: &Obj<'_>) -> PyResult<bool> {
    let py = radial.py();
    let mut first: Option<Obj<'_>> = None;
    for f in radial.try_iter()? {
        let a = f?.getattr("attrs")?;
        let a = if a.is_truthy()? {
            a
        } else {
            PyDict::new(py).into_any()
        };
        if let Some(ref first) = first {
            if a.ne(first)? {
                return Ok(true);
            }
        } else {
            first = Some(a);
        }
    }
    Ok(false)
}

fn creases(
    frame: &Frame,
    edges: &Obj<'_>,
    op: &Obj<'_>,
    cache: &Obj<'_>,
    plane_tol: f64,
    seg_tol: f64,
) -> PyResult<Vec<Segment>> {
    let mut rim = Vec::new();
    if !op.is_none() {
        for pair in op.try_iter()? {
            let pair = pair?;
            let a = xyz(&pair.get_item(0)?)?;
            let b = xyz(&pair.get_item(1)?)?;
            if [a, b]
                .iter()
                .all(|p| (*p - frame.origin).dot(frame.n).abs() < plane_tol)
            {
                rim.push([frame.project(a), frame.project(b)]);
            }
        }
    }
    let mut out = Vec::new();
    for e in edges.try_iter()? {
        let e = e?;
        let fs = e.getattr("faces")?;
        if fs.is_empty()? {
            continue;
        }
        let mut crease = false;
        for f in fs.try_iter()? {
            let f = f?;
            let n = if cache.is_none() {
                xyz(&f.call_method0("normal")?)?.normalize_or_zero()
            } else {
                xyz(&cache.call_method1("normal", (&f,))?)?
            };
            if n.dot(frame.n).abs() < 0.999 {
                crease = true;
                break;
            }
        }
        if !crease && op.is_none() {
            continue;
        }
        let a = frame.project(xyz(&e.getattr("a")?)?);
        let b = frame.project(xyz(&e.getattr("b")?)?);
        if crease
            || !rim.iter().any(|&s| {
                [a, b]
                    .iter()
                    .all(|&p| super::cap_outline::on_segment(p, s, seg_tol))
            })
            || keeps_paint(&fs)?
        {
            out.push((a, b));
        }
    }
    Ok(out)
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
pub(crate) fn rebuild_region_faces(
    origin: &Obj<'_>,
    normal: &Obj<'_>,
    axes: (Obj<'_>, Obj<'_>),
    samples: Vec<(Ring, Vec<Ring>, XY)>,
    covers: Vec<(bool, u8, u8)>,
    edges: &Obj<'_>,
    keep: bool,
    removing: bool,
    op: &Obj<'_>,
    cache: &Obj<'_>,
    sample_offset: f64,
    plane_tol: f64,
    seg_tol: f64,
    ray: &Obj<'_>,
    rng: &Obj<'_>,
    packed: &Obj<'_>,
    outlines: &Obj<'_>,
) -> PyResult<Vec<(Vec<Vec3>, Vec<Vec<Vec3>>, bool)>> {
    let frame = Frame {
        origin: xyz(origin)?,
        u: xyz(&axes.0)?,
        v: xyz(&axes.1)?,
        n: xyz(normal)?,
    };
    let py = origin.py();
    let mut groups: [Vec<Region>; 4] = Default::default();
    for ((outer, holes, probe), (old, skin, fresh)) in samples.into_iter().zip(covers) {
        let p = frame.lift(probe);
        // The jitter generator advances through the positive side first, even on a tie.
        let occupied = |pos: DVec3, n: DVec3| -> PyResult<bool> {
            Ok(ray
                .call1((Vec3::from(pos), Vec3::from(n), py.None(), rng, packed))?
                .is(PyBool::new(py, false)))
        };
        let plus = occupied(p + frame.n * sample_offset, frame.n)?;
        let minus = occupied(p - frame.n * sample_offset, -frame.n)?;
        let group = match surface(plus, minus, old, skin, fresh, keep, removing) {
            Surface::Boundary(true) => 0,
            Surface::Boundary(false) => 1,
            Surface::Partition => 2,
            Surface::Sheet => 3,
            Surface::Discard => continue,
            Surface::InteriorSeam => {
                if !op.is_none() || !cache.is_none() {
                    let rim = PyList::empty(py);
                    for ring in std::iter::once(&outer).chain(&holes) {
                        for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                            rim.append((Vec3::from(frame.lift(a)), Vec3::from(frame.lift(b))))?;
                        }
                    }
                    if !op.is_none() {
                        op.call_method1("extend", (&rim,))?;
                    }
                    if !cache.is_none() {
                        cache.getattr("dropped")?.call_method1("extend", (&rim,))?;
                    }
                }
                continue;
            }
        };
        groups[group].push((outer, holes));
    }
    let seams = creases(&frame, edges, op, cache, plane_tol, seg_tol)?;
    let mut result = Vec::new();
    for (group, regions) in groups.into_iter().enumerate() {
        if regions.is_empty() {
            continue;
        }
        let nested = outlines
            .call1((regions, &seams))?
            .extract::<Vec<Region>>()?;
        for (outer, holes) in nested {
            result.push((
                frame.ring(outer, group == 0),
                holes
                    .into_iter()
                    .map(|h| frame.ring(h, group == 0))
                    .collect(),
                group == 2,
            ));
        }
    }
    Ok(result)
}
