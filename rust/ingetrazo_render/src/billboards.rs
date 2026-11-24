//! Face-me placements (formerly views/wgpu_billboards.py): where each figure
//! stands and which way it faces, measured once per scene edit, and its
//! world matrix turned about the vertical toward the eye on each camera
//! move. The sprite mapping that fits a figure's image to its outline.
use numpy::{IntoPyArray, PyArray3, PyReadonlyArray2, PyReadonlyArray3};
use pyo3::{exceptions::PyValueError, prelude::*};

type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm(a: V3) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

/// A figure's stance: the bottom centre of its world box and the
/// horizontal direction of its largest triangle's normal, or None for an
/// empty or degenerate figure.
pub fn stance(points: impl Iterator<Item = V3>) -> Option<(V3, [f64; 2])> {
    let world: Vec<V3> = points.collect();
    if world.is_empty() {
        return None;
    }
    let mut low = world[0];
    let mut high = world[0];
    for p in &world {
        for k in 0..3 {
            low[k] = low[k].min(p[k]);
            high[k] = high[k].max(p[k]);
        }
    }
    let anchor = [(low[0] + high[0]) * 0.5, (low[1] + high[1]) * 0.5, low[2]];
    // The first of the largest normals, as numpy's argmax picks it.
    let mut best: Option<(f64, V3)> = None;
    for t in world.chunks_exact(3) {
        let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
        let length = norm(n);
        if best.is_none_or(|(l, _)| length > l) {
            best = Some((length, n));
        }
    }
    let (_, n) = best?;
    let length = (n[0] * n[0] + n[1] * n[1]).sqrt();
    (length > 1e-9).then(|| (anchor, [n[0] / length, n[1] / length]))
}

/// The stance of each figure, `(anchor, normal)` or None, from its
/// prototype's vertex range ``(start, count)`` in ``positions`` and its world
/// matrix (row-vector convention: ``p @ m[:3, :3] + m[3, :3]``).
#[pyfunction]
pub fn billboard_stances(
    positions: PyReadonlyArray2<'_, f32>,
    ranges: Vec<(usize, usize)>,
    worlds: PyReadonlyArray3<'_, f32>,
) -> PyResult<Vec<Option<(V3, [f64; 2])>>> {
    // No figures: an empty scene's arrays are never viewed.
    if ranges.is_empty() {
        return Ok(Vec::new());
    }
    let pos = positions.as_array();
    let worlds = worlds.as_array();
    if pos.ncols() != 3 || worlds.shape()[1..] != [4, 4] || worlds.shape()[0] != ranges.len() {
        return Err(PyValueError::new_err("expected (N, 3) positions and one 4 x 4 world per range"));
    }
    ranges
        .iter()
        .enumerate()
        .map(|(j, &(start, count))| {
            if start + count > pos.nrows() {
                return Err(PyValueError::new_err("vertex range outside the positions"));
            }
            let m = worlds.index_axis(numpy::ndarray::Axis(0), j);
            Ok(stance((start..start + count).map(|i| {
                let p = [f64::from(pos[[i, 0]]), f64::from(pos[[i, 1]]), f64::from(pos[[i, 2]])];
                std::array::from_fn(|c| {
                    p[0] * f64::from(m[[0, c]]) + p[1] * f64::from(m[[1, c]]) + p[2] * f64::from(m[[2, c]])
                        + f64::from(m[[3, c]])
                })
            })))
        })
        .collect()
}

/// Each figure's world matrix turned about the vertical through its anchor
/// so its normal points at the eye (at the target's side in a parallel
/// view). ``stances`` as `billboard_stances` returns them; ``mesh`` marks
/// figures that fall back to their own normal when the eye is right above.
#[pyfunction]
pub fn billboard_worlds<'py>(
    py: Python<'py>,
    base: PyReadonlyArray3<'py, f32>,
    stances: Vec<Option<(V3, [f64; 2])>>,
    mesh: Vec<bool>,
    eye: V3,
    target: V3,
    perspective: bool,
) -> PyResult<Bound<'py, PyArray3<f32>>> {
    if stances.is_empty() {
        return Ok(PyArray3::zeros(py, [0, 4, 4], false));
    }
    let base = base.as_array();
    if base.shape()[1..] != [4, 4] || base.shape()[0] != stances.len() || mesh.len() != stances.len() {
        return Err(PyValueError::new_err("expected one 4 x 4 base matrix and mesh flag per stance"));
    }
    let mut out = base.to_owned();
    for (j, stance) in stances.iter().enumerate() {
        let Some((anchor, normal)) = stance else { continue };
        let from = if perspective { *anchor } else { target };
        let mut d = [eye[0] - from[0], eye[1] - from[1]];
        let length = (d[0] * d[0] + d[1] * d[1]).sqrt();
        if length < 1e-6 {
            d = if mesh[j] { *normal } else { [1., 0.] };
        } else {
            d = [d[0] / length, d[1] / length];
        }
        let cosine = normal[0] * d[0] + normal[1] * d[1];
        let sine = normal[0] * d[1] - normal[1] * d[0];
        // turn = [[c, -s, 0, tx], [s, c, 0, ty], [0, 0, 1, 0], [0, 0, 0, 1]],
        // the rotation about the anchor; world' = (turn @ base.T).T.
        let tx = anchor[0] - (cosine * anchor[0] - sine * anchor[1]);
        let ty = anchor[1] - (sine * anchor[0] + cosine * anchor[1]);
        for r in 0..4 {
            let b: [f64; 4] = std::array::from_fn(|c| f64::from(base[[j, r, c]]));
            let x = cosine * b[0] - sine * b[1] + tx * b[3];
            let y = sine * b[0] + cosine * b[1] + ty * b[3];
            out[[j, r, 0]] = x as f32;
            out[[j, r, 1]] = y as f32;
            out[[j, r, 2]] = b[2] as f32;
            out[[j, r, 3]] = b[3] as f32;
        }
    }
    Ok(out.into_pyarray(py))
}

/// A sprite's image mapping: its horizontal extent along the first ring's
/// face and its height to 0..1, `[ux, uy, uz, u0, vx, vy, vz, v0]` with
/// `u = p · (ux, uy, uz) + u0`; None for a flat or empty outline.
#[pyfunction]
pub fn sprite_mapping(positions: PyReadonlyArray2<'_, f64>, corners: Vec<usize>, first_ring: usize) -> PyResult<Option<[f64; 8]>> {
    if corners.is_empty() {
        return Ok(None);
    }
    let pos = positions.as_array();
    if pos.ncols() != 3 || corners.iter().any(|&c| c >= pos.nrows()) {
        return Err(PyValueError::new_err("expected (N, 3) positions and corners inside them"));
    }
    let points: Vec<V3> = corners.iter().map(|&c| [pos[[c, 0]], pos[[c, 1]], pos[[c, 2]]]).collect();
    if points.is_empty() {
        return Ok(None);
    }
    let ring = &points[..first_ring.min(points.len())];
    let mut normal = [0.; 3];
    for (i, &p) in ring.iter().enumerate() {
        let c = cross(p, ring[(i + 1) % ring.len()]);
        for k in 0..3 {
            normal[k] += c[k];
        }
    }
    let mut right = [-normal[1], normal[0], 0.];
    let length = norm(right).max(1e-30);
    right = right.map(|v| v / length);
    let u: Vec<f64> = points.iter().map(|p| p[0] * right[0] + p[1] * right[1] + p[2] * right[2]).collect();
    let (low, high) = u.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| (a.min(v), b.max(v)));
    let (bottom, top) = points.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p[2]), b.max(p[2])));
    if high - low <= 1e-9 || top - bottom <= 1e-9 {
        return Ok(None);
    }
    let w = high - low;
    let h = top - bottom;
    Ok(Some([right[0] / w, right[1] / w, right[2] / w, -low / w, 0., 0., 1. / h, -bottom / h]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_standing_quad_stands_at_its_bottom_centre_and_faces_its_normal() {
        let quad = [[0., 0., 0.], [2., 0., 0.], [2., 0., 3.], [0., 0., 0.], [2., 0., 3.], [0., 0., 3.]];
        let (anchor, normal) = stance(quad.into_iter()).unwrap();
        assert_eq!(anchor, [1., 0., 0.]);
        assert!((normal[0]).abs() < 1e-12 && (normal[1] + 1.).abs() < 1e-12);
        assert!(stance(std::iter::empty()).is_none());
        // Flat on the ground: no horizontal normal.
        assert!(stance([[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]].into_iter()).is_none());
    }
}
