//! Move and Rotate grips on a group's box: the bounding-box grips (corners,
//! edge midpoints, side centres, the centre), the red rotation handles and
//! the screen-space hover region. Boxes are `(axes, lo, hi)`: a point is
//! `Σ axes[i] · c[i]` with `lo[i] ≤ c[i] ≤ hi[i]`, in the box's own axes.
use super::P3;
use glam::{DVec2, DVec3};

pub type Obb = ([P3; 3], P3, P3);

/// The grip sets the modifier key cycles through, in order.
pub const GRIP_SETS: [&str; 4] = ["corners", "edges", "sides", "centre"];

/// A side's rotation handles show while its outward normal lies within this
/// angle of the direction to the eye: a cone of 150° apex about the view.
pub const ROTATION_CONE_DEG: f64 = 75.;

fn point(axes: &[DVec3; 3], c: DVec3) -> DVec3 {
    axes[0] * c.x + axes[1] * c.y + axes[2] * c.z
}

/// The visible grips of one set on the box, without repeats: a flat box
/// (a face in a group) has four corners, not eight twice over.
pub fn box_grips(obb: &Obb, set: usize) -> Vec<P3> {
    let axes = obb.0.map(DVec3::from);
    let (lo, hi) = (DVec3::from(obb.1), DVec3::from(obb.2));
    let at = |t: [f64; 3]| point(&axes, lo + (hi - lo) * DVec3::from(t));
    let mut out: Vec<DVec3> = Vec::new();
    let mut push = |p: DVec3| {
        let tol = 1e-9 * (1. + (hi - lo).length());
        if !out.iter().any(|q| q.distance(p) <= tol) {
            out.push(p);
        }
    };
    match set % GRIP_SETS.len() {
        0 => {
            for i in 0..8 {
                push(at([(i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64]));
            }
        }
        1 => {
            // Each edge runs along one axis at the midpoint of it.
            for axis in 0..3 {
                // A flat box's edges across its thickness are its corners.
                if (hi - lo)[axis].abs() <= 1e-9 * (1. + (hi - lo).length()) {
                    continue;
                }
                for k in 0..4 {
                    let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
                    let mut t = [0.; 3];
                    t[axis] = 0.5;
                    t[a] = (k & 1) as f64;
                    t[b] = ((k >> 1) & 1) as f64;
                    push(at(t));
                }
            }
        }
        2 => {
            for axis in 0..3 {
                for side in [0., 1.] {
                    let mut t = [0.5; 3];
                    t[axis] = side;
                    push(at(t));
                }
            }
        }
        _ => push(at([0.5; 3])),
    }
    out.into_iter().map(|p| p.to_array()).collect()
}

/// One box around several: in their shared axes when every box has the
/// same ones (copies of one component turned alike), otherwise in the
/// model's axes.
pub fn union_box(boxes: &[Obb]) -> Option<Obb> {
    let first = boxes.first()?;
    let same = boxes.iter().all(|b| {
        (0..3).all(|i| DVec3::from(b.0[i]).dot(DVec3::from(first.0[i])) > 1. - 1e-9)
    });
    if boxes.len() == 1 {
        return Some(*first);
    }
    let frame: [DVec3; 3] = if same {
        first.0.map(DVec3::from)
    } else {
        [DVec3::X, DVec3::Y, DVec3::Z]
    };
    let (mut lo, mut hi) = (DVec3::INFINITY, DVec3::NEG_INFINITY);
    for b in boxes {
        for p in box_grips(b, 0) {
            let p = DVec3::from(p);
            let c = DVec3::new(frame[0].dot(p), frame[1].dot(p), frame[2].dot(p));
            lo = lo.min(c);
            hi = hi.max(c);
        }
    }
    Some((frame.map(|a| a.to_array()), lo.to_array(), hi.to_array()))
}

/// The red rotation handles, as `(position, side centre, side normal)`:
/// four on every side of the box that faces the eye within the cone.
pub fn rotation_grips(axes: [P3; 3], lo: P3, hi: P3, eye: P3) -> Vec<(P3, P3, P3)> {
    let axes = axes.map(DVec3::from);
    let eye = DVec3::from(eye);
    let mid = (DVec3::from(lo) + DVec3::from(hi)) * 0.5;
    let ext = DVec3::from(hi) - DVec3::from(lo);
    let center = point(&axes, mid);
    let cos = ROTATION_CONE_DEG.to_radians().cos();
    let mut grips = Vec::new();
    for i in 0..3 {
        for sign in [1., -1.] {
            let n = axes[i] * sign;
            let face = center + n * (ext[i] * 0.5);
            let toward = (eye - face).normalize_or_zero();
            // A flat box keeps its one face (both of its sides coincide).
            if n.dot(toward) <= cos {
                continue;
            }
            for j in 0..3 {
                if i == j || ext[j] <= 1e-6 {
                    continue;
                }
                for side in [1., -1.] {
                    grips.push((
                        (face + axes[j] * (side * 0.3 * ext[j])).into(),
                        face.into(),
                        n.into(),
                    ));
                }
            }
        }
    }
    grips
}

/// Rotation handles yield to bounding-box grips: any within `radius` pixels
/// of a box grip on screen goes, so the move grip is the one taken there.
pub fn yield_to_box_grips(handles: &[Option<[f64; 2]>], grips: &[Option<[f64; 2]>], radius: f64) -> Vec<bool> {
    handles
        .iter()
        .map(|h| match h {
            Some([x, y]) => !grips
                .iter()
                .flatten()
                .any(|[u, v]| (x - u).hypot(y - v) < radius),
            None => false,
        })
        .collect()
}

/// The nearest pixel within `radius` of the cursor, if any.
pub fn nearest(pixels: &[Option<[f64; 2]>], cursor: [f64; 2], radius: f64) -> Option<usize> {
    let mut best = None;
    let mut distance = radius;
    for (i, p) in pixels.iter().enumerate() {
        if let Some([x, y]) = p {
            let d = (x - cursor[0]).hypot(y - cursor[1]);
            if d <= distance {
                best = Some(i);
                distance = d;
            }
        }
    }
    best
}

/// Whether the carried box, posed by `pose` (column-major), stands between
/// the eye and `target`: the sight line enters it before it reaches the
/// target. A target on the box's near side (a front corner carried) is not
/// hidden; one behind it (a back corner, a snap past the object) is.
pub fn box_hides(eye: P3, target: P3, obb: &Obb, pose: &[f64; 16]) -> bool {
    let frame = glam::DMat3::from_cols(obb.0[0].into(), obb.0[1].into(), obb.0[2].into());
    let placed = glam::DMat4::from_cols_array(pose) * glam::DMat4::from_mat3(frame);
    if placed.determinant().abs() < 1e-18 {
        return false;
    }
    let inverse = placed.inverse();
    let o = inverse.transform_point3(DVec3::from(eye));
    let d = inverse.transform_point3(DVec3::from(target)) - o;
    let (lo, hi) = (DVec3::from(obb.1), DVec3::from(obb.2));
    let (mut enter, mut leave) = (f64::NEG_INFINITY, f64::INFINITY);
    for i in 0..3 {
        if d[i].abs() < 1e-15 {
            if o[i] < lo[i] || o[i] > hi[i] {
                return false;
            }
            continue;
        }
        let (a, b) = ((lo[i] - o[i]) / d[i], (hi[i] - o[i]) / d[i]);
        enter = enter.max(a.min(b));
        leave = leave.min(a.max(b));
    }
    // The near side within a hair of the target is the target's own face.
    leave > enter.max(0.) && enter < 1. - 1e-4 && leave > 0.
}

pub(super) fn convex_hull(mut pts: Vec<DVec2>) -> Vec<DVec2> {
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then_with(|| a.y.total_cmp(&b.y)));
    pts.dedup();
    if pts.len() <= 1 { return pts; }
    fn chain(points: impl Iterator<Item = DVec2>) -> Vec<DVec2> {
        let mut out: Vec<DVec2> = Vec::new();
        for p in points {
            while out.len() >= 2 {
                let b = out[out.len() - 1];
                let a = out[out.len() - 2];
                if (b - a).perp_dot(p - a) > 0. {
                    break;
                }
                out.pop();
            }
            out.push(p);
        }
        out.pop();
        out
    }
    let mut hull = chain(pts.iter().copied());
    hull.extend(chain(pts.iter().copied().rev()));
    hull
}

pub fn hover_hull(points: Vec<[f64; 2]>, cursor: [f64; 2], margin: f64) -> bool {
    let hull = convex_hull(points.into_iter().map(DVec2::from).collect());
    hull.len() >= 3
        && hull
            .iter()
            .zip(hull.iter().cycle().skip(1))
            .all(|(&a, &b)| {
                let edge = b - a;
                edge.perp_dot(DVec2::from(cursor) - a) >= -margin * edge.length()
            })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORLD: [P3; 3] = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];

    fn approx(a: P3, b: P3) -> bool {
        (DVec3::from(a) - DVec3::from(b)).length() < 1e-9
    }

    #[test]
    fn each_set_has_its_count_and_sits_on_the_box() {
        let obb = (WORLD, [0., 0., 0.], [4., 2., 1.]);
        assert_eq!(box_grips(&obb, 0).len(), 8);
        assert_eq!(box_grips(&obb, 1).len(), 12);
        assert_eq!(box_grips(&obb, 2).len(), 6);
        assert_eq!(box_grips(&obb, 3), vec![[2., 1., 0.5]]);
        assert!(box_grips(&obb, 1).contains(&[2., 0., 0.]));
        assert!(box_grips(&obb, 2).contains(&[4., 1., 0.5]));
        // The cycle wraps round.
        assert_eq!(box_grips(&obb, 4).len(), 8);
    }

    #[test]
    fn a_flat_box_has_no_repeated_grips() {
        let obb = (WORLD, [0., 0., 0.], [2., 1., 0.]);
        assert_eq!(box_grips(&obb, 0).len(), 4);
        assert_eq!(box_grips(&obb, 1).len(), 4);
        assert_eq!(box_grips(&obb, 2).len(), 5);
    }

    #[test]
    fn grips_follow_a_turned_scaled_box_in_its_own_axes() {
        // A component turned 30° about Z and scaled 2 × 3 × 1: its box is in
        // its own axes, so the corners are the turned corners, not the
        // model-axis box around them.
        let (s, c) = 30f64.to_radians().sin_cos();
        let axes = [[c, s, 0.], [-s, c, 0.], [0., 0., 1.]];
        let obb = (axes, [1., 0., 0.], [3., 3., 1.]);
        let corners = box_grips(&obb, 0);
        let far = [3. * c - 3. * s, 3. * s + 3. * c, 1.];
        assert!(corners.iter().any(|&p| approx(p, far)), "{corners:?}");
        let centre = box_grips(&obb, 3)[0];
        assert!(approx(centre, [2. * c - 1.5 * s, 2. * s + 1.5 * c, 0.5]));
    }

    #[test]
    fn a_selection_of_boxes_has_one_union_box() {
        let a = (WORLD, [0., 0., 0.], [1., 1., 1.]);
        let b = (WORLD, [3., 2., 0.], [4., 5., 2.]);
        let u = union_box(&[a, b]).unwrap();
        assert_eq!((u.1, u.2), ([0., 0., 0.], [4., 5., 2.]));
        // Turned unlike: the model's axes.
        let (s, c) = 45f64.to_radians().sin_cos();
        let t = ([[c, s, 0.], [-s, c, 0.], [0., 0., 1.]], [0., 0., 0.], [1., 1., 1.]);
        let u = union_box(&[a, t]).unwrap();
        assert_eq!(u.0, WORLD);
        assert!((u.1[0] - (-s)).abs() < 1e-9 && (u.2[1] - (s + c)).abs() < 1e-9);
        // Turned alike: their own axes.
        let t2 = (t.0, [2., 0., 0.], [3., 1., 1.]);
        let u = union_box(&[t, t2]).unwrap();
        assert_eq!(u.0, t.0);
        assert_eq!((u.1, u.2), ([0., 0., 0.], [3., 1., 1.]));
    }

    #[test]
    fn rotation_handles_show_on_sides_within_the_view_cone() {
        let obb = (WORLD, [0., 0., 0.], [4., 1., 1.]);
        let normals = |eye: P3| {
            let mut n: Vec<_> = rotation_grips(obb.0, obb.1, obb.2, eye)
                .into_iter()
                .map(|(_, _, n)| n.map(|v| v.round() as i32))
                .collect();
            n.dedup();
            n
        };
        // Looking down at 45°: front and top both inside the 75° cone.
        let n = normals([2., -50., 50.5]);
        assert!(n.contains(&[0, -1, 0]) && n.contains(&[0, 0, 1]));
        // Almost straight down: the front is 80° off the view, outside the
        // cone though it still faces the eye.
        let eye = [2., -0.5 - 50. * 10f64.to_radians().tan(), 50.5];
        assert_eq!(normals(eye), vec![[0, 0, 1]]);
    }

    #[test]
    fn a_carried_box_hides_what_lies_behind_it_only() {
        const I: [f64; 16] = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.];
        let obb = (WORLD, [0., 0., 0.], [1., 1., 1.]);
        let eye = [0.5, -10., 0.5];
        // The back of the box, seen through its front: hidden.
        assert!(box_hides(eye, [0.5, 1., 0.5], &obb, &I));
        assert!(box_hides(eye, [0.5, 5., 0.5], &obb, &I));
        // Its front face, or a point beside it: not.
        assert!(!box_hides(eye, [0.5, 0., 0.5], &obb, &I));
        assert!(!box_hides(eye, [3., 5., 0.5], &obb, &I));
        // Carried 4 m to the right, the box no longer stands in the way.
        let mut moved = I;
        moved[12] = 4.;
        assert!(!box_hides(eye, [0.5, 5., 0.5], &obb, &moved));
        assert!(box_hides([4.5, -10., 0.5], [4.5, 5., 0.5], &obb, &moved));
    }

    #[test]
    fn rotation_handles_yield_to_box_grips_nearby() {
        let keep = yield_to_box_grips(
            &[Some([0., 0.]), Some([50., 0.]), None],
            &[Some([4., 3.]), None],
            9.,
        );
        assert_eq!(keep, vec![false, true, false]);
        assert_eq!(nearest(&[Some([0., 0.]), Some([5., 0.]), None], [4., 0.], 9.), Some(1));
        assert_eq!(nearest(&[Some([0., 0.])], [40., 0.], 9.), None);
    }
}
