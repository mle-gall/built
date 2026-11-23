// SPDX-License-Identifier: GPL-3.0-or-later
//! Admission of a push, from the entry solid to the reached opposite wall.
use crate::{
    math::{xyz, Vec3},
    store::commands::cell,
};
use glam::DVec3;
use pyo3::{prelude::*, types::PyList};

type Shape = (usize, usize, Vec<[u64; 3]>);

fn object_shape(mesh: &Bound<'_, PyAny>) -> PyResult<Shape> {
    let mut cells = Vec::new();
    for v in mesh.getattr("vertices")?.try_iter()? {
        cells.push(cell(xyz(&v?.getattr("position")?)?.to_array()));
    }
    cells.sort_unstable();
    Ok((
        mesh.getattr("faces")?.len()?,
        mesh.getattr("edges")?.len()?,
        cells,
    ))
}

fn snapshot_shape(snap: &Bound<'_, PyAny>) -> PyResult<Shape> {
    let (_, edges, faces): (usize, usize, usize) = snap.call_method0("counts")?.extract()?;
    let mut cells = Vec::new();
    for key in snap.call_method1("keys", ("vertex",))?.try_iter()? {
        cells.push(cell(
            snap.call_method1("vertex_position", (key?,))?.extract()?,
        ));
    }
    cells.sort_unstable();
    Ok((faces, edges, cells))
}

/// A closed volume may flatten, but may not open. Save through the facade:
/// IFC data and detached Python handles must survive restoration as well.
#[pyfunction]
pub fn guarded_push(
    mesh: &Bound<'_, PyAny>,
    apply: &Bound<'_, PyAny>,
    check_motion: bool,
    closed: &Bound<'_, PyAny>,
    flat: &Bound<'_, PyAny>,
    store_of: &Bound<'_, PyAny>,
) -> PyResult<bool> {
    if !closed.call1((mesh,))?.is_truthy()? || flat.call1((mesh,))?.is_truthy()? {
        apply.call0()?;
        return Ok(false);
    }
    let before = mesh.call_method0("capture_state")?;
    let snap = before.getattr_opt("snap")?;
    let shape = if check_motion && snap.is_none() {
        Some(object_shape(mesh)?)
    } else {
        None
    };
    apply.call0()?;
    if !closed.call1((mesh,))?.is_truthy()? && !flat.call1((mesh,))?.is_truthy()? {
        mesh.call_method1("restore_state", (before,))?;
        return Ok(true);
    }
    if !check_motion {
        return Ok(false);
    }
    if let Some(snap) = snap {
        let store = store_of.call1((mesh,))?;
        if !store.is_none() {
            return store.call_method1("push_shape_matches", (snap,))?.extract();
        }
        return Ok(snapshot_shape(&snap)? == object_shape(mesh)?);
    }
    Ok(shape == Some(object_shape(mesh)?))
}

#[pyfunction]
pub fn translate_prism_cap(prism: bool, keep: bool, crosses: &Bound<'_, PyAny>) -> PyResult<bool> {
    Ok(prism && !keep && !crosses.call0()?.is_truthy()?)
}

/// Check hole rims and component closure only when all outer edges suggest a
/// sheet. Both queries can be large on a mixed sheet-and-solid model.
#[pyfunction]
pub fn push_region<'py>(
    outer: &Bound<'py, PyAny>,
    holes: &Bound<'py, PyAny>,
    keep: bool,
    closed: bool,
    classify: &Bound<'py, PyAny>,
    through: &Bound<'py, PyAny>,
    component_closed: &Bound<'py, PyAny>,
    flat: &Bound<'py, PyAny>,
) -> PyResult<(bool, bool, bool, bool, Option<Bound<'py, PyAny>>)> {
    let mut backed = true;
    let mut sheet = !closed;
    let n = outer.len()?;
    for i in 0..n {
        let kind: String = classify
            .call1((outer.get_item(i)?, outer.get_item((i + 1) % n)?))?
            .extract()?;
        backed &= kind != "free";
        sheet &= kind == "coplanar";
    }
    let attached = backed && !keep;
    let far = if attached {
        Some(through.call0()?)
    } else {
        None
    };
    if sheet {
        'rims: for hole in holes.try_iter()? {
            let ring = hole?;
            let n = ring.len()?;
            for i in 0..n {
                if !classify
                    .call1((ring.get_item(i)?, ring.get_item((i + 1) % n)?))?
                    .eq("coplanar")?
                {
                    sheet = false;
                    break 'rims;
                }
            }
        }
    }
    sheet = sheet && !component_closed.call0()?.is_truthy()?;
    let solid = !flat.call0()?.is_truthy()? && !sheet;
    Ok((backed, attached, sheet, solid, far))
}

fn onto_rim(p: DVec3, rim: &[DVec3]) -> DVec3 {
    const SNAP: f64 = 5e-4;
    let mut near = rim[0];
    for &v in &rim[1..] {
        if v.distance_squared(p) < near.distance_squared(p) {
            near = v;
        }
    }
    if near.distance(p) <= SNAP {
        return near;
    }
    let (mut best, mut distance) = (p, SNAP);
    for (&a, &b) in rim.iter().zip(rim.iter().cycle().skip(1)) {
        let ab = b - a;
        if ab.length_squared() < 1e-18 {
            continue;
        }
        let t = (p - a).dot(ab) / ab.length_squared();
        if t <= 0. || t >= 1. {
            continue;
        }
        let q = a + ab * t;
        let d = q.distance(p);
        if d < distance {
            (best, distance) = (q, d);
        }
    }
    best
}

fn points(face: &Bound<'_, PyAny>) -> PyResult<Vec<DVec3>> {
    face.getattr("vertices")?
        .try_iter()?
        .map(|p| xyz(&p?))
        .collect()
}

/// Candidates arrive in mesh order. Preserve the first equally distant face
/// and snap a projected opening to rim corners before considering rim edges.
#[pyfunction]
pub fn push_through_face<'py>(
    face: &Bound<'py, PyAny>,
    distance: f64,
    faces: &Bound<'py, PyAny>,
    mesh: &Bound<'py, PyAny>,
    near: &Bound<'py, PyAny>,
    inside: &Bound<'py, PyAny>,
    subtract: &Bound<'py, PyAny>,
) -> PyResult<Option<(Bound<'py, PyAny>, Bound<'py, PyList>, Bound<'py, PyAny>)>> {
    let py = face.py();
    let base = points(face)?;
    let normal = xyz(&face.call_method0("normal")?)?.normalize_or_zero();
    let origin = xyz(&face.call_method0("centroid")?)?;
    let push = if distance > 0. { normal } else { -normal };
    let travel = distance.abs();
    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for &v in &base {
        let end = v + push * travel;
        lo = lo.min(v).min(end);
        hi = hi.max(v).max(end);
    }
    let margin = 0.01 + 1e-6 * lo.abs().max(hi.abs()).max_element();
    lo -= DVec3::splat(margin);
    hi += DVec3::splat(margin);
    let candidates = near.call1((mesh, lo.to_array(), hi.to_array()))?;
    let mut best = None;
    let mut best_depth = f64::INFINITY;
    for far in (if candidates.is_none() {
        faces
    } else {
        &candidates
    })
    .try_iter()?
    {
        let far = far?;
        if far.eq(face)? {
            continue;
        }
        let ring = points(&far)?;
        if ring.is_empty()
            || (0..3)
                .any(|k| ring.iter().all(|p| p[k] < lo[k]) || ring.iter().all(|p| p[k] > hi[k]))
        {
            continue;
        }
        let n = xyz(&far.call_method0("normal")?)?.normalize_or_zero();
        let toward = push.dot(n);
        if toward.abs() < 0.999 {
            continue;
        }
        let centre = xyz(&far.call_method0("centroid")?)?;
        let depth = (centre - origin).dot(push);
        if depth <= 1e-4 || depth >= best_depth {
            continue;
        }
        let (mut first, mut last) = (f64::INFINITY, f64::NEG_INFINITY);
        for &v in &base {
            let reach = (centre - v).dot(n) / toward;
            first = first.min(reach);
            last = last.max(reach);
        }
        if last - first > 2e-3 {
            (first, last) = (depth, depth);
        }
        if first > travel + 1e-4 {
            continue;
        }
        let back = PyList::new(
            py,
            base.iter().map(|&v| {
                let p = v + push * depth;
                Vec3::from(onto_rim(p - n * (p - centre).dot(n), &ring))
            }),
        )?;
        let remainder = if inside.call1((&far, &back))?.is_truthy()? {
            py.None().into_bound(py)
        } else {
            if toward <= 0. && (travel < first - 1e-4 || travel > last + 1e-4) {
                continue;
            }
            let trim = subtract.call1((&far, &back))?;
            if trim.is_none() {
                continue;
            }
            trim
        };
        best_depth = depth;
        best = Some((far, back, remainder));
    }
    Ok(best)
}
