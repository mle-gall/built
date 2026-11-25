// SPDX-License-Identifier: GPL-3.0-or-later
//! Ordinary and mesh POLYLINE records carry their vertices in separate entities.
use super::{curves, prune, Entity, Shape, Tags};
use glam::{DMat4, DVec3};

pub(super) fn path(points: Vec<DVec3>, bulges: &[f64], closed: bool, basis: DMat4) -> Shape {
    let map = |p| basis.transform_point3(p);
    if bulges.iter().all(|&b| b == 0.) {
        Shape::Polyline(points.into_iter().map(map).collect(), closed)
    } else {
        let lengths = points.windows(2).map(|w| w[0].distance(w[1])).collect();
        Shape::Curve(
            curves::polyline(&points, bulges, closed)
                .into_iter()
                .map(|c| c.map(map))
                .collect(),
            lengths,
        )
    }
}
pub(super) fn classic(head: &Tags<'_>, verts: &[Tags<'_>]) -> Option<Entity> {
    let flags = head.integer(70, 0)?;
    let mut pts = vec![];
    let mut faces = vec![];
    let mut bulges = vec![];
    for v in verts {
        if flags & 64 != 0 && v.integer(70, 0)? & 192 == 128 {
            let mut ring = vec![];
            for code in 71..=74 {
                let index = v.integer(code, 0)?.unsigned_abs();
                if index > 0 {
                    ring.push(*pts.get(index as usize - 1)?);
                }
            }
            faces.push(prune(ring, true));
        } else {
            pts.push(v.point(10)?);
            bulges.push(v.number(42, 0.)?);
        }
    }
    let shape = if flags & 80 != 0 {
        if flags & 16 != 0 {
            let m = usize::try_from(head.integer(71, 0)?).ok()?;
            let n = usize::try_from(head.integer(72, 0)?).ok()?;
            if m.checked_mul(n)? != pts.len() {
                return None;
            }
            for x in 0..m.saturating_sub(1) {
                for y in 0..n.saturating_sub(1) {
                    let k = x * n + y;
                    faces.push(vec![pts[k], pts[k + 1], pts[k + n + 1], pts[k + n]]);
                }
            }
        }
        faces.retain(|r| r.len() >= 3);
        Shape::Part(faces)
    } else {
        let basis = if flags & 8 != 0 {
            DMat4::IDENTITY
        } else {
            let z = head.number(30, 0.)?;
            for p in &mut pts {
                p.z = z;
            }
            head.basis()?
        };
        path(pts, &bulges, flags & 1 != 0, basis)
    };
    Some(Entity {
        layer: head.get(8).unwrap_or("0").to_owned(),
        shape,
    })
}
pub(super) fn mesh(tags: &Tags<'_>) -> Option<Shape> {
    let verts = tags.points(10)?;
    let count = usize::try_from(tags.integer(93, 0)?).ok()?;
    let start = tags.0.iter().position(|(c, _)| *c == 93)? + 1;
    let mut indices = tags.0.get(start..start.checked_add(count)?)?.iter();
    let mut faces = vec![];
    let index = |(c, s): &(i32, &str)| {
        if *c == 90 {
            s.trim().parse::<usize>().ok()
        } else {
            None
        }
    };
    while let Some(n) = indices.next() {
        let n = index(n)?;
        if n > indices.len() {
            return None;
        }
        let face = indices
            .by_ref()
            .take(n)
            .map(|tag| verts.get(index(tag)?).copied())
            .collect::<Option<Vec<_>>>()?;
        let ring = prune(face, true);
        if ring.len() >= 3 {
            faces.push(ring);
        }
    }
    Some(Shape::Part(faces))
}
