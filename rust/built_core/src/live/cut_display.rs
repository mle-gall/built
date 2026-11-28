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
            let show_target = all || selection.contains(&g)?;
            for id in &target.cuts {
                let Some(row) = t.rows.get(id) else {
                    continue;
                };
                if !show_target {
                    let Some(Cutter::Opening(open)) = &row.cutter else {
                        continue;
                    };
                    let assembly = row
                        .junction
                        .as_ref()
                        .and_then(|id| t.rows.get(id))
                        .map(|j| j.owner.as_str());
                    let mut show = false;
                    for owner in [Some(open.fill.as_str()), assembly].into_iter().flatten() {
                        if let Some(g) = group(py, &t, owner) {
                            show |= selection.contains(&g)?;
                        }
                    }
                    if !show {
                        continue;
                    }
                }
                if let Some(item) = outline(
                    scene,
                    &t,
                    target,
                    row,
                    reference::frame(t.groups[&target.owner]),
                )? {
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

type Segments = Vec<[DVec3; 2]>;
type Dimensions = Vec<(&'static str, [DVec3; 2])>;

struct NotchFrame {
    ends: [DVec3; 2],
    axes: [DVec3; 2],
}

fn notch_axes(path: &[[f64; 3]], span: usize, rotation: f64) -> Option<NotchFrame> {
    let start = DVec3::from(*path.get(span)?);
    let end = DVec3::from(*path.get((span + 1) % path.len())?);
    let longitudinal = (end - start).try_normalize()?;
    let across = DVec3::Z
        .cross(longitudinal)
        .try_normalize()
        .unwrap_or(DVec3::X);
    let up = longitudinal.cross(across);
    let angle = rotation.to_radians();
    Some(NotchFrame {
        ends: [start, end],
        axes: [longitudinal, up * angle.cos() - across * angle.sin()],
    })
}

fn cut_edges(
    member: &Obj<'_>,
    uid: &str,
    frame: Option<NotchFrame>,
) -> PyResult<(Segments, Dimensions)> {
    let mesh = member.getattr("mesh")?;
    let mut selected = Vec::new();
    for face in mesh.getattr("faces")?.try_iter()? {
        let face = face?;
        if face
            .getattr("attrs")?
            .call_method1("get", ("built.cut_face",))?
            .eq(uid)?
        {
            selected.push(face);
        }
    }
    if selected.is_empty() {
        // Older saved clearance faces already have persistent generated IDs,
        // even when the kernel did not attach a donor to the offset surface.
        let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
        let keys = {
            let borrow = store.try_borrow()?;
            let st = &borrow.inner;
            let generated =
                eid::intern(geometry::h64(b"built:generated:v1")).map_err(PyValueError::new_err)?;
            let faces = st.pid_index(Kind::Face);
            (0..)
                .map_while(|i| {
                    let role = format!("cut/{uid}/clearance/{i}");
                    let id = eid::pack(
                        generated,
                        geometry::h64(role.as_bytes()) & eid::COUNTER_MASK,
                    );
                    faces.get(&id).map(|&f| st.key(Kind::Face, f))
                })
                .collect::<Vec<_>>()
        };
        let face_view = member
            .py()
            .import("built_core.store_mesh")?
            .getattr("FaceView")?;
        for key in keys {
            selected.push(face_view.call1((&mesh, key))?);
        }
    }
    let mut lines = Vec::new();
    for face in selected {
        for ring in
            std::iter::once(Ok(face.getattr("vertices")?)).chain(face.getattr("holes")?.try_iter()?)
        {
            let points = ring?
                .try_iter()?
                .map(|p| crate::math::xyz(&p?))
                .collect::<PyResult<Vec<_>>>()?;
            for (&a, &b) in points.iter().zip(points.iter().cycle().skip(1)) {
                lines.push([a, b]);
            }
        }
    }
    let mut dimensions = Dimensions::new();
    if let (Some(frame), Some(first)) = (frame, lines.first()) {
        for (axis, label) in frame.axes.into_iter().zip(["Length", "Depth"]) {
            // Points level along the axis tie; the smallest coordinates win,
            // so the dimension sits in one place whatever the face order
            // (a reopened file lists its faces in another order).
            let before = |a: DVec3, b: DVec3| {
                let d = a.dot(axis) - b.dot(axis);
                if d.abs() > 1e-9 { d < 0. } else { a.to_array() < b.to_array() }
            };
            let (mut low, mut high) = (first[0], first[0]);
            for &point in lines.iter().flatten() {
                if before(point, low) {
                    low = point;
                }
                if before(high, point) && (point.dot(axis) - high.dot(axis)).abs() > 1e-9 {
                    high = point;
                }
            }
            // A flat end cut leaves a single cross-section. Measure its
            // setback from the original end of the selected sweep span.
            if label == "Length" && (high - low).dot(axis).abs() < 1e-7 {
                let [a, b] = frame.ends.map(|end| (end - low).dot(axis));
                high = low + axis * if a.abs() < b.abs() { a } else { b };
            }
            let extent = (high - low).dot(axis);
            if extent.abs() > 1e-8 {
                dimensions.push((label, [low, low + extent * axis]));
            }
        }
    }
    Ok((lines, dimensions))
}

/// Evaluate the picked pair once per revision/clearance. Detached operands
/// use the target's frame, as they do on commit.
pub(crate) fn body_preview(scene: &Obj<'_>, args: &Value) -> PyResult<Value> {
    let py = scene.py();
    let key = (scene.getattr("content_version")?, args.to_string()).into_pyobject(py)?;
    if let Some(cache) = scene.getattr_opt("_cut_body_preview")? {
        if cache.get_item(0)?.eq(&key)? {
            return Ok(
                serde_json::from_str(&cache.get_item(1)?.extract::<String>()?)
                    .unwrap_or(Value::Null),
            );
        }
    }
    let Some(t) = current(py, root(scene)?) else {
        return Ok(Value::Null);
    };
    let target = args["target"].as_str().unwrap_or_default();
    let body = args["body"].as_str().unwrap_or_default();
    let Some(row) = t.owners.get(target).and_then(|id| t.rows.get(id)) else {
        return Ok(Value::Null);
    };
    let Some(sweep) = &row.sweep else {
        return Ok(Value::Null);
    };
    let (Some(owner), Some(cutter)) = (group(py, &t, target), group(py, &t, body)) else {
        return Ok(Value::Null);
    };
    let world = reference::frame(t.groups[target]);
    let cls = py.import("built_core.group")?.getattr("Group")?;
    let tool = cls.call1((cutter.getattr("mesh")?,))?;
    tool.setattr(
        "xform",
        crate::math::Matrix4::from(world.inverse() * reference::frame(t.groups[body])),
    )?;
    let subject = cls.call1((owner.getattr("mesh")?,))?;
    let operands = pyo3::types::PyList::new(py, [tool, subject])?;
    let value = match crate::solids::operation(
        "subtract",
        operands.as_any(),
        Some("preview"),
        args["offset"].as_f64().unwrap_or_default(),
    ) {
        Ok((_, result)) => {
            let path = reference::sweep_path(scene, &t, row, &sweep.path)?.unwrap_or_default();
            let axis = notch_axes(
                &path,
                args["span"].as_u64().unwrap_or(0) as usize,
                sweep.rotation,
            );
            let (lines, dims) = cut_edges(&result.get_item(0)?, "preview", axis)?;
            let lines: Vec<_> = lines
                .into_iter()
                .map(|ab| ab.map(|p| world.transform_point3(p)))
                .collect();
            let sizes: Vec<_> = dims
                .into_iter()
                .map(|(label, [a, b])| (label, world.transform_vector3(b - a).length()))
                .collect();
            json!({"lines":lines,"sizes":sizes})
        }
        Err(e) if e.is_instance_of::<crate::solids::SolidError>(py) => Value::Null,
        Err(e) => return Err(e),
    };
    scene.setattr("_cut_body_preview", (&key, value.to_string()))?;
    Ok(value)
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
    let mut dimensions = Dimensions::new();
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
                catalogue: None,
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
            let Some(Reference::Frame { junction, frame }) =
                row.anchor.as_ref().map(|a| &a.reference)
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
                let corners = [DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Z, DVec3::Z]
                    .map(|p| frame.transform_point3(p * DVec3::from(row.size)));
                dimensions.extend([
                    ("Width", [corners[0], corners[1]]),
                    ("Height", [corners[1], corners[2]]),
                ]);
                if let Some(fill) = group(scene.py(), t, uid) {
                    let window = fill
                        .getattr("ifc")?
                        .get_item("class")
                        .is_ok_and(|v| v.eq("IfcWindow").unwrap_or(false));
                    let facing = reference::frame(t.groups[uid]).transform_vector3(-DVec3::Y);
                    let front = world.inverse().transform_vector3(facing);
                    let flipped = front.dot(frame.y_axis.truncate()) > 0.;
                    lines.extend(crate::hosted_openings::symbol(
                        corners, front, window, flipped,
                    ));
                }
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
                if let (Some(sweep), Some(member)) =
                    (&target.sweep, group(scene.py(), t, &target.owner))
                {
                    let path =
                        reference::sweep_path(scene, t, target, &sweep.path)?.unwrap_or_default();
                    let axis = notch_axes(&path, row.span.unwrap_or(0), sweep.rotation);
                    (lines, dimensions) = cut_edges(&member, &row.uid, axis)?;
                }
                if lines.is_empty() {
                    mesh_outline(&g, frame, &mut lines)?;
                }
            }
        }
    }
    let lines: Vec<_> = lines
        .into_iter()
        .map(|ab| ab.map(|p| world.transform_point3(p)))
        .collect();
    let dimensions: Vec<_> = dimensions
        .into_iter()
        .map(|(label, [a, b])| {
            (
                world.transform_point3((a + b) * 0.5),
                world.transform_vector3(b - a).length(),
                label,
            )
        })
        .collect();
    let grips: Vec<_> = grips
        .into_iter()
        .map(|(kind, p)| json!({"kind":kind,"at":world.transform_point3(p)}))
        .collect();
    Ok(Some(
        json!({"row":row,"repeat_source":repeat_source,"centre":world.transform_point3(centre),"axis":world.transform_vector3(normal),"radius":radius,"grips":grips,"lines":lines,"dimensions":dimensions}),
    ))
}
