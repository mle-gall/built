// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::store::{
    drawing_plane::covers,
    geometry::{Hypot, V3},
};
fn vertices<'p>(c: &Cluster<'p>) -> PyResult<Vec<Obj<'p>>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for e in &c.edges {
        for k in ["v0", "v1"] {
            let v = e.getattr(k)?;
            if seen.insert(id(&v)?) {
                out.push(v);
            }
        }
    }
    Ok(out)
}
pub(super) fn project_vertices(mesh: &Obj, c: &Cluster) -> PyResult<()> {
    for v in vertices(c)? {
        let p = xyz(&v.getattr("position")?)?;
        let d = (p - c.o).dot(c.n);
        if WELD * 0.5 < d.abs() && d.abs() <= TOL && !v.call_method0("faces")?.is_truthy()? {
            mesh.call_method1("place_vertex", (v, Vec3::from(p - c.n * d)))?;
        }
    }
    Ok(())
}
fn cell(p: DVec3) -> [i64; 3] {
    p.to_array().map(|v| (v / TOL).floor() as i64)
}
type PointKey = [u64; 3];
type RingKey = BTreeSet<PointKey>;
fn key(p: DVec3) -> PointKey {
    p.to_array().map(|v| {
        let n = (v * 10000.).round_ties_even();
        if n == 0. {
            0
        } else {
            n.to_bits()
        }
    })
}
pub(super) fn ring_key(r: &Ring) -> RingKey {
    r.iter().map(|&p| key(p)).collect()
}
pub(super) fn holes_key(h: &[Ring]) -> BTreeSet<RingKey> {
    h.iter().map(ring_key).collect()
}
pub(super) fn snap_and_split(mesh: &Obj, c: &Cluster, regions: &mut [Region]) -> PyResult<()> {
    let mut grid = HashMap::<_, Vec<DVec3>>::new();
    for v in vertices(c)? {
        let p = xyz(&v.getattr("position")?)?;
        grid.entry(cell(p)).or_default().push(p);
    }
    let mut corners = vec![];
    let mut lookup = HashMap::new();
    for (outer, holes) in regions {
        for p in std::iter::once(outer).chain(holes.iter_mut()).flatten() {
            let [x, y, z] = cell(*p);
            let mut dist = TOL;
            let mut best = *p;
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        for &q in grid.get(&[x + dx, y + dy, z + dz]).into_iter().flatten() {
                            let d = q.distance(*p);
                            if d <= dist {
                                dist = d;
                                best = q;
                            }
                        }
                    }
                }
            }
            *p = best;
            let i = *lookup.entry(key(best)).or_insert_with(|| {
                corners.push(best);
                corners.len() - 1
            });
            corners[i] = best;
        }
    }
    // Search only the narrowest coordinate slab. Preserve its order when two
    // split candidates lie on the same edge, as each split rekeys the edge.
    let sorted: [Vec<usize>; 3] = std::array::from_fn(|i| {
        let mut v = (0..corners.len()).collect::<Vec<_>>();
        v.sort_by(|a, b| corners[*a][i].total_cmp(&corners[*b][i]));
        v
    });
    let mut work = c.edges.clone();
    while let Some(e) = work.pop() {
        let [a, b] = ends(&e)?;
        let d = b - a;
        let l2 = d.length_squared();
        if l2 < 1e-18 {
            continue;
        }
        let lo = a.min(b) - WELD;
        let hi = a.max(b) + WELD;
        let near = (0..3)
            .map(|i| {
                let v = &sorted[i];
                let l = v.partition_point(|j| corners[*j][i] < lo[i]);
                let h = v.partition_point(|j| corners[*j][i] <= hi[i]);
                &v[l..h]
            })
            .min_by_key(|s| s.len())
            .unwrap();
        for &idx in near {
            let p = corners[idx];
            if !p.cmpge(lo).all() || !p.cmple(hi).all() {
                continue;
            }
            let t = (p - a).dot(d) / l2;
            if t <= 0.
                || t >= 1.
                || p.distance(a + t * d) > WELD
                || p.distance(a) <= WELD
                || p.distance(b) <= WELD
            {
                continue;
            }
            let (first, second): (Obj, Obj) = mesh
                .call_method1("split_edge", (&e, Vec3::from(p)))?
                .extract()?;
            if !first.eq(&e)? {
                work.extend([first, second]);
            }
            break;
        }
    }
    Ok(())
}
pub(super) struct Existing<'p> {
    pub face: Obj<'p>,
    pub rim: Ring,
    pub holes: Vec<Ring>,
    lo: DVec3,
    hi: DVec3,
}
pub(super) fn existing<'p>(mesh: &Obj<'p>, o: DVec3, n: DVec3) -> PyResult<Vec<Existing<'p>>> {
    let mut result = Vec::new();
    for f in mesh.getattr("faces")?.try_iter()? {
        let f = f?;
        if xyz(&f.call_method0("normal")?)?.dot(n).abs() < 0.999 {
            continue;
        }
        let rim = points(&f.getattr("vertices")?)?;
        if rim.is_empty() || (rim[0] - o).dot(n).abs() > TOL {
            continue;
        }
        let lo = rim
            .iter()
            .copied()
            .fold(DVec3::splat(f64::INFINITY), DVec3::min);
        let hi = rim
            .iter()
            .copied()
            .fold(DVec3::splat(f64::NEG_INFINITY), DVec3::max);
        let holes = f
            .getattr("holes")?
            .try_iter()?
            .map(|r| points(&r?))
            .collect::<PyResult<_>>()?;
        result.push(Existing {
            face: f,
            rim,
            holes,
            lo: lo - TOL,
            hi: hi + TOL,
        });
    }
    Ok(result)
}
pub(super) fn has_neighbours(mesh: &Obj, f: &Obj) -> PyResult<bool> {
    let mut rings = vec![f.getattr("loop")?];
    rings.extend(
        f.getattr("hole_loops")?
            .try_iter()?
            .collect::<PyResult<Vec<_>>>()?,
    );
    for r in rings {
        let vs = r.try_iter()?.collect::<PyResult<Vec<_>>>()?;
        for (i, v) in vs.iter().enumerate() {
            let e = mesh.call_method1("find_edge", (v, &vs[(i + 1) % vs.len()]))?;
            if !e.is_none() {
                for other in e.getattr("faces")?.try_iter()? {
                    if !other?.eq(f)? {
                        return Ok(true);
                    }
                }
            }
        }
    }
    Ok(false)
}
pub(super) fn probe(py: Python, o: &Ring, h: &[Ring], n: DVec3) -> PyResult<Option<DVec3>> {
    let objs = |r: &Ring| {
        r.iter()
            .map(|&p| Vec3::from(p).into_bound_py_any(py))
            .collect::<PyResult<Vec<_>>>()
    };
    let ts = crate::polygon::polygon_triangles(
        py,
        objs(o)?,
        Some(h.iter().map(objs).collect::<PyResult<_>>()?),
        Some(Vec3::from(n).into_bound_py_any(py)?),
    )?;
    let mut result = None;
    let mut best = -1.;
    for (a, b, c) in ts {
        let (a, b, c) = (xyz(&a)?, xyz(&b)?, xyz(&c)?);
        let area = (b - a).cross(c - a).length_squared();
        if area > best {
            best = area;
            result = Some((a + b + c) / 3.);
        }
    }
    Ok(result)
}
pub(super) fn covered(existing: &[Existing], p: DVec3) -> PyResult<bool> {
    let v = |x: DVec3| V3(x.x, x.y, x.z);
    for f in existing {
        if !p.cmpge(f.lo).all() || !p.cmple(f.hi).all() {
            continue;
        }
        for tri in f.face.call_method0("triangulate")?.try_iter()? {
            let [a, b, c]: [Obj; 3] = tri?.extract()?;
            if covers(
                [v(xyz(&a)?), v(xyz(&b)?), v(xyz(&c)?)],
                v(p),
                Hypot::LibStdCxx,
            ) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
