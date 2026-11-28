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

fn mesh_outline(group: &Obj<'_>, frame: DMat4, lines: &mut Vec<[DVec3; 2]>) -> PyResult<()> {
    for edge in group.getattr("mesh")?.getattr("edges")?.try_iter()? {
        let edge = edge?;
        let point = |key| -> PyResult<DVec3> {
            Ok(frame.transform_point3(crate::math::xyz(&edge.getattr(key)?)?))
        };
        lines.push([point("a")?, point("b")?]);
    }
    Ok(())
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
    let mut repeat_source = None;
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
            } else if let Some(source) = cut::source(scene, target.source.as_deref().unwrap_or(""))?
            {
                cut::solid_axis(&source.getattr("mesh")?)?
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
        Cutter::Hole(h) => {
            let Some(m) = h.pose(scene, t, row)? else {
                return Ok(None);
            };
            centre = m.w_axis.truncate();
            normal = m.z_axis.truncate().normalize();
            radius = h.d / 2000.;
            let ring = h.ring(radius, 0., m);
            lines.extend(
                ring.iter()
                    .zip(ring.iter().cycle().skip(1))
                    .map(|(a, b)| [(*a).into(), (*b).into()]),
            );
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
                family: String::new(),
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
        Cutter::Repeat(uid) => {
            let Some(repeat) = t.rows.get(uid) else {
                return Ok(None);
            };
            let Some(owner) = group(scene.py(), t, &repeat.owner) else {
                return Ok(None);
            };
            let Some(source) = repeat
                .source
                .as_ref()
                .and_then(|id| group(scene.py(), t, id))
            else {
                return Ok(None);
            };
            repeat_source = repeat.source.as_ref();
            let inverse = world.inverse();
            for station in std::iter::once(Ok(source)).chain(owner.getattr("children")?.try_iter()?)
            {
                let station = station?;
                let frame = inverse * reference::frame(station.getattr("_doc_key")?.extract()?);
                mesh_outline(&station, frame, &mut lines)?;
            }
        }
        Cutter::Block(b) => {
            let Some(Reference::Frame { junction, frame }) = row.anchor.as_ref().map(|a| &a.reference)
            else {
                return Ok(None);
            };
            let Some(base) = super::junction::local_frame(t, &row.owner, junction, frame) else {
                return Ok(None);
            };
            let frame = base
                * DMat4::from_translation(DVec3::from(b.at) / 1000.)
                * DMat4::from_scale(DVec3::from(b.size) / 1000.);
            centre = frame.transform_point3(DVec3::splat(0.5));
            normal = base.z_axis.truncate().normalize_or_zero();
            for corner in 0..8usize {
                let point = |index: usize| {
                    frame.transform_point3(DVec3::from_array(std::array::from_fn(|axis| {
                        f64::from(u8::from(index & (1 << axis) != 0))
                    })))
                };
                for axis in 0..3 {
                    let bit = 1 << axis;
                    if corner & bit == 0 {
                        lines.push([point(corner), point(corner | bit)]);
                    }
                }
            }
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
                mesh_outline(&g, frame, &mut lines)?;
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
        json!({"row":row,"repeat_source":repeat_source,"centre":world.transform_point3(centre),"axis":world.transform_vector3(normal),"radius":radius,"grips":grips,"lines":lines}),
    ))
}
