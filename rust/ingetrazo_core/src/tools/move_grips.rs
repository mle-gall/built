//! Visible Move rotation handles and their screen-space hover region.
use super::P3;
use glam::{DVec2, DVec3};

pub fn rotation_grips(axes: [P3; 3], lo: P3, hi: P3, eye: P3) -> Vec<(P3, P3, P3)> {
    let axes = axes.map(DVec3::from);
    let eye = DVec3::from(eye);
    let mid = (DVec3::from(lo) + DVec3::from(hi)) * 0.5;
    let ext = DVec3::from(hi) - DVec3::from(lo);
    let center = axes[0] * mid.x + axes[1] * mid.y + axes[2] * mid.z;
    let mut grips = Vec::new();
    for i in 0..3 {
        for sign in [1., -1.] {
            let n = axes[i] * sign;
            let face = center + n * (ext[i] * 0.5);
            if n.dot(eye - face) <= 0. {
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
