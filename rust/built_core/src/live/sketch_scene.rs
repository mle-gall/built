// SPDX-License-Identifier: GPL-3.0-or-later
//! What the Sketch workspace reads from the model: the plane of a picked
//! face (with its outline, for a profile from the face), and the context
//! around a sketch plane: the section the plane cuts through neighbours
//! (ghosted, snappable), what lies behind it (sliced), and nearby edges in
//! world space for the live 3D inset. Sketch units are millimetres.
use super::*;
use crate::store::Store;

const MM: f64 = 1000.0;

fn store_points(s: &Store, ring: &[u32]) -> Vec<DVec3> {
    ring.iter()
        .map(|v| DVec3::from(s.state.frame.definition_point(*s.state.vpos.get(*v as usize))))
        .collect()
}

fn newell(p: &[DVec3]) -> DVec3 {
    let mut n = DVec3::ZERO;
    for i in 0..p.len() {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        n += DVec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
    }
    n.normalize_or_zero()
}

/// The uids from the root down to the group being edited.
fn edit_path(scene: &Obj<'_>) -> PyResult<(Vec<String>, Option<doc::Key>)> {
    let g = scene.getattr("edit_group")?;
    if g.is_none() {
        return Ok((vec![], None));
    }
    let k: doc::Key = g.getattr("_doc_key")?.extract()?;
    let d = doc::lock();
    let mut path = Vec::new();
    let mut cur = Some(k);
    while let Some(key) = cur {
        let Some(n) = d.node(key) else { break };
        let Data::Placement(p) = &n.data else { break };
        path.push(p.uid.clone().unwrap_or_default());
        cur = n.owners.first().copied();
    }
    path.reverse();
    Ok((path, Some(k)))
}

/// A sketch plane on a face of the geometry being edited: origin, axes and
/// normal in world metres, the face's element reference (for the sketch's
/// plane anchor) and its loops in sketch millimetres. Vertical faces get X
/// level so the sketch's Y points up; level faces keep world X.
/// `path`: the placement uids from the root to the face's group (a
/// sketch row's plane reference); None takes the geometry being edited.
#[pyfunction]
#[pyo3(signature=(scene, face, path=None))]
pub(crate) fn sketch_plane<'p>(scene: &Obj<'p>, face: &str, path: Option<Vec<String>>) -> PyResult<Obj<'p>> {
    let py = scene.py();
    let Ok((letter, id)) = eid::from_text(face) else {
        return Ok(py.None().into_bound(py));
    };
    if letter != 'f' {
        return Ok(py.None().into_bound(py));
    }
    let (path, group, mesh) = match path.filter(|p| !p.is_empty()) {
        Some(p) => {
            let g = scene.call_method1("resolve_occurrence", (p.clone(),))?;
            if g.is_none() {
                return Ok(py.None().into_bound(py));
            }
            let k: doc::Key = g.getattr("_doc_key")?.extract()?;
            (p, Some(k), g.getattr("mesh")?)
        }
        None => {
            let (p, k) = edit_path(scene)?;
            (p, k, scene.getattr("mesh")?)
        }
    };
    let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
    let borrow = store.try_borrow()?;
    let s = &borrow.inner;
    let Some(slot) = s.slot_of_pid(Kind::Face, id) else {
        return Ok(py.None().into_bound(py));
    };
    let world = group.map(reference::frame).unwrap_or(DMat4::IDENTITY);
    let rings: Vec<Vec<DVec3>> = s.face_rings(slot).iter().map(|r| store_points(s, r).into_iter().map(|p| world.transform_point3(p)).collect()).collect();
    let Some(outer) = rings.first().filter(|r| r.len() >= 3) else {
        return Ok(py.None().into_bound(py));
    };
    let n = newell(outer);
    if n == DVec3::ZERO {
        return Ok(py.None().into_bound(py));
    }
    let x = if n.z.abs() < 0.99 { DVec3::Z.cross(n).normalize() } else { (DVec3::X - n * n.x).normalize() };
    let y = n.cross(x);
    let o = outer[0];
    let local = |p: DVec3| [((p - o).dot(x)) * MM, ((p - o).dot(y)) * MM];
    // Put the origin at the outline's lower-left corner.
    let (mut lo_x, mut lo_y) = (f64::MAX, f64::MAX);
    for p in outer {
        let [a, b] = local(*p);
        lo_x = lo_x.min(a);
        lo_y = lo_y.min(b);
    }
    let origin = o + x * (lo_x / MM) + y * (lo_y / MM);
    let loops: Vec<Vec<[f64; 2]>> = rings
        .iter()
        .map(|r| r.iter().map(|p| [((*p - origin).dot(x)) * MM, ((*p - origin).dot(y)) * MM]).collect())
        .collect();
    to_py(
        py,
        &serde_json::json!({
            "origin": origin.to_array(), "x": x.to_array(), "y": y.to_array(), "n": n.to_array(),
            "ref": {"path": path, "id": face},
            "loops": loops,
        }),
    )
}

struct Frame {
    o: DVec3,
    x: DVec3,
    y: DVec3,
    n: DVec3,
}
impl Frame {
    fn mm(&self, p: DVec3) -> [f64; 2] {
        [(p - self.o).dot(self.x) * MM, (p - self.o).dot(self.y) * MM]
    }
    fn height(&self, p: DVec3) -> f64 {
        (p - self.o).dot(self.n)
    }
}

/// Context around a sketch plane (world metres): `ghost` is the section
/// the plane cuts plus edges lying on it, `behind` the edges behind it
/// within `reach` projected onto it, `inset` the edges within `reach` of
/// the plane origin in world space. Each list stops at `limit` segments.
#[pyfunction]
#[pyo3(signature=(scene, origin, x, y, reach=3.0, limit=20000))]
pub(crate) fn sketch_context<'p>(scene: &Obj<'p>, origin: [f64; 3], x: [f64; 3], y: [f64; 3], reach: f64, limit: usize) -> PyResult<Obj<'p>> {
    let py = scene.py();
    let (x, y) = (DVec3::from(x).normalize_or_zero(), DVec3::from(y).normalize_or_zero());
    let f = Frame { o: DVec3::from(origin), x, y, n: x.cross(y).normalize_or_zero() };
    // Every mesh with its world matrix: the loose geometry, then placements.
    let mut parts: Vec<(Py<PyAny>, DMat4)> = vec![(loose_mesh(scene)?.unbind(), DMat4::IDENTITY)];
    {
        let d = doc::lock();
        let root = root(scene)?;
        let mut todo: Vec<(doc::Key, DMat4)> = vec![(root, DMat4::IDENTITY)];
        while let Some((k, m)) = todo.pop() {
            let Some(node) = d.node(k) else { continue };
            for &c in &node.children {
                let Some(cn) = d.node(c) else { continue };
                let Data::Placement(p) = &cn.data else { continue };
                if p.hidden {
                    continue;
                }
                let w = m * p.xform.as_ref().map(|x| DMat4::from_cols_array(&x.doubles())).unwrap_or(DMat4::IDENTITY);
                if let Some(def) = &p.definition {
                    parts.push((def.0.get().geometry.clone_ref(py), w));
                }
                todo.push((c, w));
            }
        }
    }
    let mut ghost: Vec<[f64; 4]> = Vec::new();
    let mut behind: Vec<[f64; 4]> = Vec::new();
    let mut inset: Vec<[f64; 6]> = Vec::new();
    let eps = 1e-6;
    let seg = |a: [f64; 2], b: [f64; 2]| [a[0], a[1], b[0], b[1]];
    for (geometry, w) in parts {
        if ghost.len() >= limit && behind.len() >= limit && inset.len() >= limit {
            break;
        }
        let Ok(store) = geometry.bind(py).getattr("_store").and_then(|s| Ok(s.cast_into::<PyMeshStore>()?)) else { continue };
        let Ok(borrow) = store.try_borrow() else { continue };
        let s = &borrow.inner;
        let pos = |v: u32| w.transform_point3(DVec3::from(s.state.frame.definition_point(*s.state.vpos.get(v as usize))));
        for e in s.live_slots(Kind::Edge) {
            let [a, b] = s.state.ends.get(e as usize).map(pos);
            let (ha, hb) = (f.height(a), f.height(b));
            let near = |p: DVec3| (p - f.o).length() <= reach;
            if inset.len() < limit && near(a) && near(b) {
                inset.push([a.x, a.y, a.z, b.x, b.y, b.z]);
            }
            if ha.abs() <= eps && hb.abs() <= eps {
                if ghost.len() < limit {
                    ghost.push(seg(f.mm(a), f.mm(b)));
                }
            } else if ha <= eps && hb <= eps && ha >= -reach && hb >= -reach && behind.len() < limit {
                behind.push(seg(f.mm(a), f.mm(b)));
            }
        }
        // The section through faces: each face's crossings with the plane, paired along the cut.
        for face in s.live_slots(Kind::Face) {
            if ghost.len() >= limit {
                break;
            }
            let rings = s.face_rings(face);
            let mut hits: Vec<DVec3> = Vec::new();
            for r in &rings {
                let pts: Vec<DVec3> = r.iter().map(|v| pos(*v)).collect();
                for i in 0..pts.len() {
                    let (p, q) = (pts[i], pts[(i + 1) % pts.len()]);
                    let (hp, hq) = (f.height(p), f.height(q));
                    if (hp > eps && hq < -eps) || (hp < -eps && hq > eps) {
                        hits.push(p + (q - p) * (hp / (hp - hq)));
                    }
                }
            }
            if hits.len() < 2 {
                continue;
            }
            // Order along the cut line (the face plane meets the sketch plane in a line).
            let (h0, dir) = (hits[0], (hits[1] - hits[0]).normalize_or_zero());
            hits.sort_by(|a, b| (*a - h0).dot(dir).total_cmp(&(*b - h0).dot(dir)));
            for pair in hits.chunks_exact(2) {
                ghost.push(seg(f.mm(pair[0]), f.mm(pair[1])));
            }
        }
    }
    to_py(py, &serde_json::json!({"ghost": ghost, "behind": behind, "inset": inset}))
}
