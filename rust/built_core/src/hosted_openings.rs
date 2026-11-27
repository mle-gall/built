// SPDX-License-Identifier: GPL-3.0-or-later
//! Rectangular prismatic walls. Cut dimensions are normalized to the host frame,
//! so a cut on the reverse side shares the same overlap and regeneration rules.
use crate::math::{mat, xyz, Vec3};
use glam::{DMat4, DVec3};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList},
};
type Obj<'p> = Bound<'p, PyAny>;
type Rect = [f64; 4]; // left, sill, right, head
type Face = (usize, Vec<DVec3>, Vec<Vec<DVec3>>);
const EPS: f64 = 1e-7;
pub(crate) const INVALID: &str = "Opening overlaps another opening or the wall edge.";
fn err() -> PyErr {
    PyValueError::new_err(INVALID)
}
fn valid(r: Rect, others: &[Rect]) -> bool {
    let [l, b, h, t] = r;
    r.iter().all(|x| x.is_finite())
        && l > EPS
        && h < 1. - EPS
        && b >= -EPS
        && t < 1. - EPS
        && h - l > EPS
        && t - b > EPS
        && others
            .iter()
            .all(|&[a, z, c, d]| h < a - EPS || l > c + EPS || t < z - EPS || b > d + EPS)
}
fn ring(points: &[(f64, f64)], y: f64) -> Vec<DVec3> {
    points.iter().map(|&(x, z)| DVec3::new(x, y, z)).collect()
}
fn outline([l, b, r, t]: Rect) -> [(f64, f64); 4] {
    [(l, b), (r, b), (r, t), (l, t)]
}
fn prism(r: Rect) -> Vec<Vec<DVec3>> {
    let corners = outline(r);
    let a = ring(&corners, 0.);
    let b = ring(&corners, 1.);
    let mut faces = vec![a.clone(), b.iter().rev().copied().collect()];
    for i in 0..4 {
        let j = (i + 1) % 4;
        faces.push(vec![a[j], a[i], b[i], b[j]]);
    }
    faces
}
fn skin(cuts: &[Rect]) -> Vec<Face> {
    let mut flush: Vec<_> = cuts.iter().copied().filter(|r| r[1] <= EPS).collect();
    flush.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut outer = vec![(0., 0.)];
    let base = prism([0., 0., 1., 1.]);
    let mut out = vec![];
    let mut x = 0.;
    for [l, _, r, t] in flush {
        outer.extend([(l, 0.), (l, t), (r, t), (r, 0.)]);
        out.push((2, prism([x, 0., l, 1.]).remove(2), vec![]));
        x = r;
    }
    out.push((2, prism([x, 0., 1., 1.]).remove(2), vec![]));
    outer.extend([(1., 0.), (1., 1.), (0., 1.)]);
    for side in 0..2 {
        let mut rim = ring(&outer, side as f64);
        let mut holes = vec![];
        for r in cuts.iter().filter(|r| r[1] > EPS) {
            let mut h = ring(&outline(*r), side as f64);
            if side == 0 {
                h.reverse();
            }
            holes.push(h);
        }
        if side == 1 {
            rim.reverse();
        }
        out.push((side, rim, holes));
    }
    for (i, p) in base.into_iter().enumerate().skip(3) {
        out.push((i, p, vec![]));
    }
    for &r in cuts {
        for (i, mut p) in prism(r).into_iter().enumerate().skip(2) {
            if i == 2 && r[1] <= EPS {
                continue;
            }
            p.reverse();
            out.push((6, p, vec![]));
        }
    }
    out
}
fn pose(g: &Obj) -> PyResult<DMat4> {
    let m = g.getattr("xform")?;
    if m.is_none() {
        Ok(DMat4::IDENTITY)
    } else {
        mat(&m)
    }
}
fn optional<'p>(dict: &Obj<'p>, key: &str) -> PyResult<Option<Obj<'p>>> {
    if dict.is_none() {
        return Ok(None);
    }
    Ok(Some(dict.call_method1("get", (key,))?).filter(|v| !v.is_none()))
}
// Measure the wall envelope in the selected face frame.
fn initial_frame(wall: &Obj, n: DVec3) -> PyResult<DMat4> {
    let mesh = wall.getattr("mesh")?;
    let faces = mesh.getattr("faces")?;
    if !faces.is_truthy()? || wall.getattr("children")?.is_truthy()? {
        return Err(err());
    }
    let w = pose(wall)?;
    let u = DVec3::Z.cross(n).normalize_or_zero();
    let v = -n;
    let axes = DMat4::from_cols(
        u.extend(0.),
        v.extend(0.),
        DVec3::Z.extend(0.),
        DVec3::ZERO.extend(1.),
    );
    let to_frame = axes.inverse() * w;
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = -lo;
    for p in mesh.getattr("vertices")?.try_iter()? {
        let p = to_frame.transform_point3(xyz(&p?.getattr("position")?)?);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    let frame = w.inverse() * axes * DMat4::from_translation(lo) * DMat4::from_scale(hi - lo);
    if !frame.inverse().is_finite() {
        return Err(err());
    }
    Ok(frame)
}
fn vertices(r: &[DVec3], m: DMat4) -> Vec<Vec3> {
    r.iter().map(|&p| m.transform_point3(p).into()).collect()
}
// Compare rings independent of their first vertex and winding. Definition
// copies have fresh element keys, including after a save/load round trip.
fn same_ring(a: &[DVec3], b: &[DVec3]) -> bool {
    a.len() == b.len()
        && b.iter().enumerate().any(|(j, p)| {
            p.abs_diff_eq(a[0], EPS)
                && [false, true].iter().any(|&reverse| {
                    a.iter().enumerate().all(|(i, q)| {
                        let k = if reverse { j + b.len() - i } else { j + i };
                        q.abs_diff_eq(b[k % b.len()], EPS)
                    })
                })
        })
}
fn same_face(a: &Face, b: &Face) -> bool {
    same_ring(&a.1, &b.1)
        && a.2.len() == b.2.len()
        && a.2.iter().all(|r| b.2.iter().any(|h| same_ring(r, h)))
}
fn mesh_for<'p>(wall: &Obj<'p>, frame: DMat4, before: &[Rect], cuts: &[Rect]) -> PyResult<Obj<'p>> {
    let py = wall.py();
    let mesh = py
        .import("built_core.group")?
        .call_method1("_copy_definition_mesh", (wall.getattr("mesh")?,))?;
    let old = skin(before);
    let mut desired = skin(cuts);
    let read = |ring: &Obj| {
        ring.try_iter()?
            .map(|p| Ok(frame.inverse().transform_point3(xyz(&p?)?)))
            .collect::<PyResult<Vec<_>>>()
    };
    let mut attrs: Vec<Option<Obj>> = vec![None; 6];
    let mut drop_faces = vec![];
    let mut obsolete_edges = std::collections::HashSet::new();
    for e in mesh.getattr("edges")?.try_iter()? {
        let e = e?;
        if e.getattr("faces")?.len()? > 0 {
            obsolete_edges.insert(e.getattr("key")?.extract::<u128>()?);
        }
    }
    let mut matched = 0;
    for f in mesh.getattr("faces")?.try_iter()? {
        let f = f?;
        let shape = (
            0,
            read(&f.getattr("vertices")?)?,
            f.getattr("holes")?
                .try_iter()?
                .map(|r| read(&r?))
                .collect::<PyResult<_>>()?,
        );
        let Some(source) = old.iter().find(|shape0| same_face(shape0, &shape)) else {
            continue;
        };
        matched += 1;
        if source.0 < 6 {
            attrs[source.0] = Some(f.getattr("attrs")?);
        }
        if let Some(i) = desired.iter().position(|next| same_face(next, &shape)) {
            desired.remove(i);
        } else {
            drop_faces.push(f);
        }
    }
    if matched != old.len() {
        return Err(PyValueError::new_err(
            "Wall faces have changed since the opening was placed",
        ));
    }
    for f in drop_faces {
        mesh.call_method1("remove_face", (f,))?;
    }
    for (role, outer, holes) in desired {
        let hs: Vec<_> = holes.iter().map(|r| vertices(r, frame)).collect();
        let f = mesh.call_method1("add_face", (vertices(&outer, frame), hs))?;
        if let Some(Some(a)) = attrs.get(role) {
            f.getattr("attrs")?.call_method1("update", (a,))?;
        }
    }
    let edges = mesh
        .getattr("edges")?
        .try_iter()?
        .collect::<PyResult<Vec<_>>>()?;
    for e in edges {
        if obsolete_edges.contains(&e.getattr("key")?.extract::<u128>()?)
            && e.getattr("faces")?.len()? == 0
        {
            mesh.call_method1("remove_edge", (e,))?;
        }
    }
    mesh.call_method0("purge_debris")?;
    Ok(mesh)
}
fn frame(wall: &Obj, normal: DVec3) -> PyResult<DMat4> {
    match optional(&wall.getattr("ext")?, "opening_frame")? {
        Some(v) => Ok(DMat4::from_cols_array(&v.extract()?)),
        None => initial_frame(wall, normal),
    }
}
fn rect(points: [DVec3; 4], world: DMat4) -> PyResult<Rect> {
    let q = points.map(|p| world.inverse().transform_point3(p));
    let mut lo = q[0];
    let mut hi = q[0];
    for p in q {
        lo = lo.min(p);
        hi = hi.max(p);
    }
    if hi.y - lo.y > EPS
        || q.iter().any(|p| {
            (p.x - lo.x).abs().min((p.x - hi.x).abs()) > EPS
                || (p.z - lo.z).abs().min((p.z - hi.z).abs()) > EPS
        })
    {
        return Err(err());
    }
    if lo.z < -EPS {
        return Err(err());
    }
    Ok([lo.x, lo.z.max(0.), hi.x, hi.z])
}
fn update(scene: &Obj, g: &Obj, remove: bool) -> PyResult<()> {
    let Some(host) = optional(&g.getattr("ext")?, "wall_host")? else {
        return Ok(());
    };
    let Some(local) = optional(&host, "profile")? else {
        return Ok(());
    }; // earlier documents used a placement-only link
    let wall = scene.call_method1(
        "resolve_occurrence",
        (vec![host.get_item("uid")?.extract::<String>()?],),
    )?;
    if wall.is_none() {
        return Err(PyValueError::new_err("Host wall is unavailable"));
    }
    let py = g.py();
    let f = frame(
        &wall,
        DVec3::from_array(host.get_item("normal")?.extract()?),
    )?;
    let world = pose(&wall)? * f;
    let corners = local
        .extract::<[[f64; 3]; 4]>()?
        .map(|p| DVec3::from_array(p));
    let placement = pose(g)?;
    let p = corners.map(|p| placement.transform_point3(p));
    let ifc = wall.getattr("ifc")?;
    let entries = PyList::empty(py);
    let mut cuts = Vec::new();
    let uid = g.getattr("uid")?;
    let mut guid = None;
    if let Some(old) = optional(&ifc, "openings")? {
        for o in old.try_iter()? {
            let o = o?;
            if optional(&o, "fill_uid")?.is_some_and(|v| v.eq(&uid).unwrap_or(false)) {
                guid = Some(o.get_item("guid")?);
                continue;
            }
            if let Some(r) = optional(&o, "wall_rect")? {
                cuts.push(r.extract()?);
            }
            entries.append(o)?;
        }
    }
    if !remove {
        let candidate = rect(p, world).ok().filter(|r| valid(*r, &cuts));
        let changed = guid.is_some() && crate::live::wall_faces_changed(&wall)?;
        if candidate.is_none() || changed {
            let reason = if changed {
                "wall_faces"
            } else {
                "opening_overlap"
            };
            if let Some(relative) = optional(&host, "relative")? {
                if crate::live::opening_conflict(scene, &wall, g, reason)? {
                    let last = pose(&wall)? * DMat4::from_cols_array(&relative.extract()?);
                    g.setattr("xform", crate::math::Matrix4::from(last))?;
                    return Ok(());
                }
            }
            return Err(err());
        }
        let r = candidate.unwrap();
        let entry = PyDict::new(py);
        if let Some(id) = guid {
            entry.set_item("guid", id)?;
        }
        let id = py
            .import("core.ifc_identity")?
            .call_method1("ensure_guid", (&entry,))?;
        let cut =
            crate::live::hosted(scene, &wall, g, Some((id.extract()?, f.to_cols_array(), r)))?;
        entry.set_item("cut", cut)?;
        host.set_item(
            "relative",
            (pose(&wall)?.inverse() * placement).to_cols_array(),
        )?;
        entry.set_item("wall_rect", r)?;
        entry.set_item("fill_uid", uid)?;
        entries.append(entry)?;
        cuts.push(r);
        g.getattr("ifc")?.set_item("fills", id)?;
        host.set_item("sill", r[1] * world.z_axis.truncate().length())?;
        let front = host.get_item("normal")?.extract::<[f64; 3]>()?;
        let depth = world.y_axis.truncate();
        let anchor = DVec3::from_array(host.get_item("anchor")?.extract()?);
        let y = world
            .inverse()
            .transform_point3(placement.transform_point3(anchor))
            .y;
        host.set_item(
            "offset",
            if depth.dot(front.into()) < 0. {
                y
            } else {
                1. - y
            } * depth.length(),
        )?;
        let facing = DVec3::from_array(host.get_item("facing")?.extract()?);
        host.set_item(
            "flipped",
            placement.transform_vector3(facing).dot(front.into()) < 0.,
        )?;
    }
    if remove {
        crate::live::hosted(scene, &wall, g, None)?;
    }
    if wall.getattr("ext")?.is_none() {
        wall.setattr("ext", PyDict::new(py))?;
    }
    let ext = wall.getattr("ext")?;
    if cuts.is_empty() {
        ext.del_item("opening_frame")?;
    } else {
        ext.set_item("opening_frame", f.to_cols_array())?;
    }
    ifc.set_item("openings", entries)?;
    Ok(())
}
pub(crate) fn place(scene: &Obj, g: &Obj) -> PyResult<()> {
    update(scene, g, false)
}

pub(crate) fn detach(scene: &Obj, g: &Obj) -> PyResult<()> {
    update(scene, g, true)
}

pub(crate) fn pick(wall: &Obj, point: DVec3, normal: DVec3) -> PyResult<Option<serde_json::Value>> {
    let Ok(f) = frame(wall, normal) else {
        return Ok(None);
    };
    let world = pose(wall)? * f;
    let y = world.inverse().transform_point3(point).y;
    if y.abs().min((1. - y).abs()) > EPS
        || world.y_axis.truncate().normalize().dot(normal).abs() < 0.99999
    {
        return Ok(None);
    }
    let world_ring = |r: Rect| {
        ring(&outline(r), y)
            .into_iter()
            .map(|p| world.transform_point3(p).to_array())
            .collect::<Vec<_>>()
    };
    let mut occupied = vec![];
    if let Some(entries) = optional(&wall.getattr("ifc")?, "openings")? {
        for o in entries.try_iter()? {
            if let Some(r) = optional(&o?, "wall_rect")? {
                occupied.push(world_ring(r.extract()?));
            }
        }
    }
    Ok(Some(
        serde_json::json!({"uid":wall.getattr("uid")?.extract::<String>()?,"point":point.to_array(),"normal":normal.to_array(),"corners":world_ring([0.,0.,1.,1.]),"occupied":occupied}),
    ))
}

/// A filled opening may cover the hit face or expose empty space. Intersect
/// the cursor ray with the recorded apertures to retain an invalid preview.
pub(crate) fn aperture_at(
    scene: &Obj,
    origin: DVec3,
    dir: DVec3,
) -> PyResult<Option<serde_json::Value>> {
    let mut best = None;
    let mut nearest = f64::INFINITY;
    for wall in scene.getattr("groups")?.try_iter()? {
        let wall = wall?;
        let Some(local) = optional(&wall.getattr("ext")?, "opening_frame")? else {
            continue;
        };
        if wall.getattr("locked")?.is_truthy()? || wall.getattr("hidden")?.is_truthy()? {
            continue;
        }
        let frame = pose(&wall)? * DMat4::from_cols_array(&local.extract()?);
        let from = frame.inverse().transform_point3(origin);
        let ray = frame.inverse().transform_vector3(dir);
        if ray.y.abs() < 1e-10 {
            continue;
        }
        let side = if ray.y > 0. { 0. } else { 1. };
        let t = (side - from.y) / ray.y;
        if t < 0. || t >= nearest {
            continue;
        }
        let q = from + ray * t;
        for opening in wall.getattr("ifc")?.get_item("openings")?.try_iter()? {
            let Some(r) = optional(&opening?, "wall_rect")? else {
                continue;
            };
            let [l, b, h, z] = r.extract::<Rect>()?;
            if q.x >= l && q.x <= h && q.z >= b && q.z <= z {
                let normal =
                    frame.y_axis.truncate().normalize() * if side == 0. { -1. } else { 1. };
                best = pick(&wall, origin + t * dir, normal)?;
                nearest = t;
                break;
            }
        }
    }
    Ok(best)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn volume(faces: &[Face]) -> f64 {
        let mut sum = 0.;
        for (_, rim, holes) in faces {
            for r in std::iter::once(rim).chain(holes) {
                for i in 1..r.len() - 1 {
                    sum += r[0].dot(r[i].cross(r[i + 1])) / 6.;
                }
            }
        }
        sum
    }
    #[test]
    fn closed_window_and_flush_door_regenerate() {
        let window = [0.1, 0.3, 0.3, 0.7];
        let door = [0.5, 0., 0.7, 0.8];
        let both = skin(&[window, door]);
        assert_eq!(both.iter().filter(|f| !f.2.is_empty()).count(), 2);
        assert_eq!(both.iter().filter(|f| f.0 == 6).count(), 7);
        assert_eq!(both.iter().filter(|f| f.0 == 2).count(), 2);
        assert!((volume(&both) - 0.76).abs() < 1e-10);
        assert!((volume(&skin(&[door])) - 0.84).abs() < 1e-10);
        assert!((volume(&skin(&[])) - 1.).abs() < 1e-10);
    }
    #[test]
    fn outlines_on_either_side_validate_in_one_host_frame() {
        let wall = DMat4::from_rotation_z(0.4) * DMat4::from_scale(DVec3::new(8., 0.2, 3.));
        let other = [0.1, 0.2, 0.3, 0.6];
        for side in [0., 1.] {
            let points = outline(other).map(|(x, z)| wall.transform_point3(DVec3::new(x, side, z)));
            let candidate = rect(points, wall).unwrap();
            assert!(valid(candidate, &[]));
            assert!(!valid(candidate, &[other]));
            assert!(!valid([0., 0., 0.15, 0.5], &[]));
        }
    }
}

/// Convert legacy box walls only when their evaluated skin matches every
/// saved aperture. A hand-edited legacy wall retains its original records.
pub(crate) fn migrate(scene: &Obj<'_>) -> PyResult<()> {
    use crate::doc;
    let Some(root) = scene
        .getattr_opt("_root")?
        .and_then(|r| r.getattr("_doc_key").ok())
        .and_then(|k| k.extract().ok())
    else {
        return Ok(());
    };
    let attr = doc::attr_id("ext").map_err(PyValueError::new_err)?;
    let walls = {
        let d = doc::lock();
        let Some(root) = d.node(root) else {
            return Ok(());
        };
        root.children
            .iter()
            .filter_map(|k| {
                let doc::value::Value::Dict(items) = d.node(*k)?.attr(attr)? else {
                    return None;
                };
                items
                    .iter()
                    .any(|(k, _)| matches!(k,doc::value::Value::Str(s) if s=="opening_frame"))
                    .then(|| d.handle_ref(scene.py(), *k))
                    .flatten()
            })
            .collect::<Vec<_>>()
    };
    for wall in walls {
        let wall = wall.bind(scene.py());
        let Some(entries) = optional(&wall.getattr("ifc")?, "openings")? else {
            continue;
        };
        let frame = optional(&wall.getattr("ext")?, "opening_frame")?
            .unwrap()
            .extract()?;
        let mut saved = Vec::new();
        let mut rects = Vec::new();
        for entry in entries.try_iter()? {
            let entry = entry?;
            if optional(&entry, "cut")?.is_some() {
                continue;
            }
            let (Some(uid), Some(r), Some(guid)) = (
                optional(&entry, "fill_uid")?,
                optional(&entry, "wall_rect")?,
                optional(&entry, "guid")?,
            ) else {
                continue;
            };
            let fill =
                scene.call_method1("resolve_occurrence", (vec![uid.extract::<String>()?],))?;
            if fill.is_none() {
                continue;
            }
            let rect = r.extract()?;
            rects.push(rect);
            saved.push((entry, fill, guid.extract()?, rect));
        }
        if saved.is_empty() || saved.len() != entries.len()? {
            continue;
        }
        let baked = wall.getattr("mesh")?;
        let Ok(base) = mesh_for(wall, DMat4::from_cols_array(&frame), &rects, &[]) else {
            continue;
        };
        wall.setattr("mesh", base)?;
        crate::live::migrate_wall(scene, wall, frame)?;
        for (entry, fill, guid, rect) in saved {
            let id = crate::live::hosted(scene, wall, &fill, Some((guid, frame, rect)))?;
            entry.set_item("cut", id)?;
            if optional(&entry, "faces")?.is_some() {
                entry.del_item("faces")?;
            }
            if let Some(host) = optional(&fill.getattr("ext")?, "wall_host")? {
                host.set_item(
                    "relative",
                    (pose(wall)?.inverse() * pose(&fill)?).to_cols_array(),
                )?;
            }
        }
        wall.setattr("mesh", baked)?;
        crate::live::finish_migration(scene)?;
    }
    Ok(())
}
