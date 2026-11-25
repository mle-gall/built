// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use std::collections::{HashMap, VecDeque};
fn pos(v: &Obj) -> PyResult<DVec3> {
    xyz(&v.getattr("position")?)
}
fn id(v: &Obj) -> PyResult<u128> {
    v.getattr("key")?.extract()
}
#[pyfunction(signature=(face,edge,toward=None))]
pub fn manual_path_start<'p>(
    face: &Obj<'p>,
    edge: &Obj<'p>,
    toward: Option<&Obj>,
) -> PyResult<(Option<Vec3>, Vec<Obj<'p>>)> {
    let c = xyz(&face.call_method0("centroid")?)?;
    let n = xyz(&face.call_method0("normal")?)?.normalize_or_zero();
    let a = edge.getattr("v0")?;
    let b = edge.getattr("v1")?;
    let (pa, pb) = (pos(&a)?, pos(&b)?);
    let (da, db) = ((pa - c).dot(n), (pb - c).dot(n));
    if da * db < -1e-12 {
        let t = da / (da - db);
        let far = match toward {
            Some(p) if pa.distance(xyz(p)?) < pb.distance(xyz(p)?) => a,
            _ => b,
        };
        return Ok((Some((pa + t * (pb - pa)).into()), vec![far]));
    }
    Ok((
        None,
        if da.abs() <= db.abs() {
            vec![a, b]
        } else {
            vec![b, a]
        },
    ))
}
#[pyfunction(signature=(chain,edge,skip=None,limit=400))]
pub fn manual_path_extend<'p>(
    chain: &Bound<'p, PyList>,
    edge: &Obj<'p>,
    skip: Option<&Obj>,
    limit: isize,
) -> PyResult<bool> {
    let skip_edge =
        |e: &Obj| -> PyResult<bool> { skip.map_or(Ok(false), |f| f.call1((e,))?.is_truthy()) };
    if chain.is_empty() || skip_edge(edge)? {
        return Ok(false);
    }
    let n = chain.len();
    let last = chain.get_item(n - 1)?;
    let v0 = edge.getattr("v0")?;
    let v1 = edge.getattr("v1")?;
    let ids = chain
        .iter()
        .enumerate()
        .map(|(i, v)| Ok((id(&v)?, i)))
        .collect::<PyResult<HashMap<_, _>>>()?;
    let (a, b) = (id(&v0)?, id(&v1)?);
    let tail = id(&last)?;
    if tail == a || tail == b {
        let other = if tail == a { v1 } else { v0 };
        if n >= 2 && other.eq(&chain.get_item(n - 2)?)? {
            chain.del_item(n - 1)?;
            return Ok(true);
        }
        if let Some(&i) = ids.get(&id(&other)?) {
            if i != 0 || n < 3 {
                return Ok(false);
            }
        }
        chain.append(other)?;
        return Ok(true);
    }
    if let (Some(i), Some(j)) = (ids.get(&a), ids.get(&b)) {
        chain.del_slice(i.max(j) + 1, n)?;
        return Ok(true);
    }
    let mut queue = VecDeque::from([last]);
    let mut prev = HashMap::from([(tail, None)]);
    for _ in 0..limit {
        let Some(v) = queue.pop_front() else { break };
        for e in v.getattr("edges")?.try_iter()? {
            let e = e?;
            if skip_edge(&e)? {
                continue;
            }
            let left = e.getattr("v0")?;
            let w = if left.eq(&v)? { e.getattr("v1")? } else { left };
            let k = id(&w)?;
            if ids.contains_key(&k) || prev.contains_key(&k) {
                continue;
            }
            prev.insert(k, Some(v.clone()));
            if k == a || k == b {
                let mut route = vec![w];
                let mut key = k;
                while let Some(parent) = &prev[&key] {
                    key = id(parent)?;
                    if key != tail {
                        route.push(parent.clone());
                    }
                }
                for vertex in route.into_iter().rev() {
                    chain.append(vertex)?;
                }
                let (other, other_key) = if k == a { (&v1, b) } else { (&v0, a) };
                if other_key != k && !ids.contains_key(&other_key) {
                    chain.append(other)?;
                }
                return Ok(true);
            }
            queue.push_back(w);
        }
    }
    Ok(false)
}
#[pyfunction]
pub fn orient_closed_path(pts: &Obj, face: &Obj) -> PyResult<Vec<Vec3>> {
    let mut pts = points(pts)?;
    if pts.len() >= 3 {
        let c = xyz(&face.call_method0("centroid")?)?;
        let n = xyz(&face.call_method0("normal")?)?.normalize_or_zero();
        let first = pts
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.distance_squared(c).total_cmp(&b.distance_squared(c)))
            .unwrap()
            .0;
        pts.rotate_left(first);
        let f = pts[1] - pts[0];
        let b = pts[pts.len() - 1] - pts[0];
        if f.length() > 1e-9
            && b.length() > 1e-9
            && b.normalize().dot(n).abs() > f.normalize().dot(n).abs() + 1e-9
        {
            pts[1..].reverse();
        }
    }
    Ok(pts.into_iter().map(Vec3::from).collect())
}
