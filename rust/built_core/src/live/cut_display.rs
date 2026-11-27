// SPDX-License-Identifier: GPL-3.0-or-later
//! World geometry shared by the cutter overlay and through-picking.
use super::*;
use cut::Cutter;
use serde_json::{json, Value};

/// Cache resolved outlines until either the document or target selection changes.
#[pyfunction]
pub(crate) fn live_cut_display<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = scene.py();
    let selection = scene.getattr("selection")?;
    let all = scene
        .getattr_opt("show_all_cutters")?
        .is_some_and(|v| v.is_truthy().unwrap_or(false));
    if (!all && selection.len()? == 0) || !scene.hasattr("_root")? {
        return Ok(pyo3::types::PyList::empty(py).into_any());
    }
    let key = (
        scene.getattr("content_version")?,
        selection.getattr("revision")?,
        all,
    )
        .into_pyobject(py)?;
    if let Some(cache) = scene.getattr_opt("_cut_display_cache")? {
        if cache.get_item(0)?.eq(&key)? {
            return cache.get_item(1);
        }
    }
    let mut items = Vec::new();
    if let Some(t) = current(py, root(scene)?) {
        for uid in &t.cut_targets {
            let target = &t.rows[uid];
            let Some(g) = group(py, &t, &target.owner) else {
                continue;
            };
            if !all && !selection.contains(&g)? {
                continue;
            }
            let world = reference::frame(t.groups[&target.owner]);
            for id in &target.cuts {
                let Some(row) = t.rows.get(id) else {
                    continue;
                };
                if let Some(item) = outline(scene, &t, target, row, world)? {
                    items.push(item);
                }
            }
        }
    }
    let items = to_py(py, &items)?;
    scene.setattr("_cut_display_cache", (&key, &items))?;
    Ok(items)
}

fn outline(
    scene: &Obj<'_>,
    t: &LiveTable,
    target: &Row,
    row: &Row,
    world: DMat4,
) -> PyResult<Option<Value>> {
    let mut lines = Vec::<[DVec3; 2]>::new();
    let mut grips = Vec::new();
    let (mut centre, mut normal) = (DVec3::ZERO, DVec3::Z);
    let mut radius = 0.14;
    let Some(cutter) = row.cutter.as_ref() else {
        return Ok(None);
    };
    match cutter {
        Cutter::Plane(_) | Cutter::Bisector(_) => {
            let mut single = target.clone();
            single.cuts = vec![row.uid.clone()];
            let path = if let Some(s) = target.sweep.as_ref() {
                reference::sweep_path(scene, t, target, &s.path)?.unwrap_or_default()
            } else {
                vec![[0.; 3], [0., 0., 1.]]
            };
            let Ok(planes) = cut::end_planes(scene, t, &single, &path, &mut None)? else {
                return Ok(None);
            };
            let Some(p) = planes.first() else {
                return Ok(None);
            };
            centre = p.origin;
            normal = p.normal;
            if target.sweep.is_some() {
                let a = DVec3::from(path[p.span]);
                let axis = DVec3::from(path[(p.span + 1) % path.len()]) - a;
                let den = normal.dot(axis);
                if den.abs() > 1e-9 {
                    centre = a + axis * (centre - a).dot(normal) / den;
                }
            }
            let x = normal.any_orthonormal_vector() * radius;
            let y = normal.cross(x);
            let corners = [
                centre - x - y,
                centre + x - y,
                centre + x + y,
                centre - x + y,
            ];
            for i in 0..4 {
                lines.push([corners[i], corners[(i + 1) % 4]]);
            }
            grips.push(("offset", centre));
            if matches!(cutter, Cutter::Plane(_)) {
                grips.push(("angle", centre + y));
            }
        }
        Cutter::Section(uid) => {
            let Some(section) = t.rows.get(uid) else {
                return Ok(None);
            };
            let Some(pose) = cut::section_frame(scene, t, target, row, section)? else {
                return Ok(None);
            };
            centre = pose.w_axis.truncate();
            normal = pose.z_axis.truncate().normalize_or_zero();
            let profile = profile::Profile {
                library: String::new(),
                drawing: Some(json!(section)),
                sketch: None,
                params: Default::default(),
            };
            let Ok(section) = profile.outline(48) else {
                return Ok(None);
            };
            let Ok(origin) = section.attachment("origin") else {
                return Ok(None);
            };
            radius = section.width.max(section.height) / 2000.;
            for ring in section.rings {
                for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                    let local = |node: &profile::Node| {
                        pose.transform_point3(((node.p - origin) / 1000.).extend(0.))
                    };
                    lines.push([local(a), local(b)]);
                }
            }
            grips.push(("offset", centre));
        }
        Cutter::Body(uid) | Cutter::Opening(cut::Opening { fill: uid, .. }) => {
            let opening = if let Cutter::Opening(opening) = cutter {
                Some(opening)
            } else {
                None
            };
            let mut frame = if let Some(opening) = opening {
                opening.pose()
            } else if let Some(&placement) = t.groups.get(uid) {
                world.inverse() * reference::frame(placement)
            } else {
                return Ok(None);
            };
            if let Some(a) = &row.anchor {
                let Some(p) = reference::origin(scene, t, &a.reference, row)? else {
                    return Ok(None);
                };
                normal = frame.z_axis.truncate().normalize_or_zero();
                centre = p + normal * a.offset / 1000.;
                frame.w_axis = centre.extend(1.);
                grips.push(("offset", centre));
            }
            if opening.is_some() {
                for corner in 0..8 {
                    let point = |index: usize| {
                        let coords = std::array::from_fn(|axis| {
                            if index & (1 << axis) == 0 {
                                0.
                            } else {
                                row.size[axis]
                            }
                        });
                        frame.transform_point3(DVec3::from_array(coords))
                    };
                    for axis in 0..3 {
                        let bit = 1 << axis;
                        if corner & bit == 0 {
                            lines.push([point(corner), point(corner | bit)]);
                        }
                    }
                }
            } else if let Some(g) = group(scene.py(), t, uid) {
                for edge in g.getattr("mesh")?.getattr("edges")?.try_iter()? {
                    let edge = edge?;
                    let point = |k| -> PyResult<DVec3> {
                        Ok(frame.transform_point3(crate::math::xyz(&edge.getattr(k)?)?))
                    };
                    lines.push([point("a")?, point("b")?]);
                }
            }
        }
    }
    let lines: Vec<_> = lines
        .into_iter()
        .map(|ab| ab.map(|p| world.transform_point3(p)))
        .collect();
    let grips: Vec<_> = grips
        .into_iter()
        .map(|(kind, p)| json!({"kind":kind,"at":world.transform_point3(p)}))
        .collect();
    Ok(Some(
        json!({"row":row,"centre":world.transform_point3(centre),"axis":world.transform_vector3(normal),"radius":radius,"grips":grips,"lines":lines}),
    ))
}
