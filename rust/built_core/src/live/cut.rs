// SPDX-License-Identifier: GPL-3.0-or-later
//! Resolve a cutter in the target definition's coordinates before generating the sweep.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cutter {
    Plane(Plane),
    Hole(hole::Hole),
    Bisector(String),
    Body(String),
    Repeat(String),
    Section(String),
    Opening(Opening),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Opening {
    pub fill: String,
    pub guid: String,
    pub frame: [f64; 16],
    pub rect: [f64; 4],
}

impl Opening {
    pub(super) fn pose(&self) -> DMat4 {
        let [left, bottom, right, top] = self.rect;
        DMat4::from_cols_array(&self.frame)
            * DMat4::from_translation(DVec3::new(left, 0., bottom))
            * DMat4::from_scale(DVec3::new(right - left, 1., top - bottom))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plane {
    #[serde(default)]
    pub origin: [f64; 3],
    #[serde(default)]
    pub normal: Option<[f64; 3]>,
    #[serde(default)]
    pub angle: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Anchor {
    #[serde(rename = "ref")]
    pub reference: Reference,
    #[serde(default)]
    pub offset: f64,
}

fn tilted(n: DVec3, angle: f64) -> DVec3 {
    glam::DQuat::from_axis_angle(n.any_orthonormal_vector(), angle.to_radians()) * n
}
pub struct EndPlane {
    pub uid: String,
    pub end: bool,
    pub span: usize,
    pub origin: DVec3,
    pub normal: DVec3,
}

pub(super) fn keep_source(scene: &Obj<'_>, mesh: &Obj<'_>) -> PyResult<String> {
    let py = scene.py();
    let source = py
        .import("built_core.group")?
        .getattr("Group")?
        .call1((mesh,))?;
    source.setattr("xform", py.import("built_core")?.call_method0("Matrix4")?)?;
    source.setattr("name", "Cut source")?;
    source.setattr("component", false)?;
    let ext = metadata(&source)?;
    ext.set_item("built.cut_source", true)?;
    source.setattr("ext", ext)?;
    scene
        .getattr("kept_components")?
        .call_method1("append", (&source,))?;
    source.getattr("uid")?.extract()
}

pub(super) fn source<'p>(scene: &Obj<'p>, uid: &str) -> PyResult<Option<Obj<'p>>> {
    for source in scene.getattr("kept_components")?.try_iter()? {
        let source = source?;
        if source.getattr("uid")?.eq(uid)? {
            return Ok(Some(source));
        }
    }
    Ok(None)
}

/// A classic group moves by rewriting vertices. Once it participates in a
/// relation its uncut definition and its placement must move independently.
pub(super) fn bind_placements(scene: &Obj<'_>, t: &LiveTable, row: &Row) -> PyResult<()> {
    let cutter = match &row.cutter {
        Some(Cutter::Body(id) | Cutter::Bisector(id)) => Some(id),
        _ => None,
    };
    let face_owner = row.anchor.as_ref().and_then(|a| match &a.reference {
        Reference::Element { path, .. } => path.last(),
        _ => None,
    });
    for id in std::iter::once(&row.owner).chain(cutter).chain(face_owner) {
        if let Some(g) = group(scene.py(), t, id) {
            if g.getattr("xform")?.is_none() {
                g.setattr("component", false)?;
                g.setattr(
                    "xform",
                    scene.py().import("built_core")?.call_method0("Matrix4")?,
                )?;
            }
        }
    }
    Ok(())
}

pub(super) fn attach(scene: &Obj<'_>, t: &mut LiveTable, row: &mut Row) -> PyResult<()> {
    let mut target = if let Some(id) = t.owners.get(&row.owner) {
        (*t.rows[id]).clone()
    } else {
        let g = group(scene.py(), t, &row.owner)
            .ok_or_else(|| PyValueError::new_err("Unknown cut target"))?;
        if g.getattr("children")?.is_truthy()?
            || !scene
                .py()
                .import("built_core.solids")?
                .call_method1("is_solid", (&g,))?
                .is_truthy()?
        {
            return Err(PyValueError::new_err("Select a solid target"));
        }
        let uid = keep_source(scene, &g.getattr("mesh")?)?;
        serde_json::from_value(serde_json::json!({"uid":eid::new_uid(),"kind":"cut_stack","owner":row.owner,"source":uid})).map_err(|e|PyValueError::new_err(e.to_string()))?
    };
    if !matches!(target.kind.as_str(), "sweep" | "cut_stack") {
        return Err(PyValueError::new_err("Select a sweep or solid target"));
    }
    if !row.cuts.is_empty() {
        return Err(PyValueError::new_err("A cut cannot own cuts"));
    }
    row.inputs.clear();
    let op = row.operation.as_deref().unwrap_or("subtract");
    match &row.cutter {
        Some(Cutter::Body(uid)) if matches!(op, "union" | "subtract" | "trim" | "intersect") => {
            row.inputs.push(Reference::Placement {
                placement: uid.clone(),
            });
        }
        Some(Cutter::Repeat(uid)) if matches!(op, "union" | "subtract" | "trim" | "intersect") => {
            if !t.rows.get(uid).is_some_and(|r| r.kind == "repeat") {
                return Err(PyValueError::new_err("Unknown Repeat cutter"));
            }
            row.inputs.push(Reference::Row { row: uid.clone() });
        }
        Some(Cutter::Hole(h)) if op == "subtract" && h.valid() => {
            if let Some(uid) = &h.repeat {
                if !t.repeats.contains(uid) {
                    return Err(PyValueError::new_err("Missing hole Repeat"));
                }
                row.inputs.push(Reference::Row { row: uid.clone() });
            }
        }
        Some(Cutter::Opening(o)) if op == "subtract" => {
            row.inputs.push(Reference::Placement {
                placement: o.fill.clone(),
            });
        }
        Some(Cutter::Section(uid)) if matches!(op, "union" | "subtract" | "trim" | "intersect") => {
            let source = t
                .rows
                .get(uid)
                .filter(|s| s.kind == "sketch")
                .ok_or_else(|| PyValueError::new_err("Select a sketch section"))?;
            row.inputs.push(Reference::Row { row: uid.clone() });
            if let Some(v) = source.sketch_fields.plane.get("ref") {
                let plane: Reference = serde_json::from_value(v.clone())
                    .map_err(|e| PyValueError::new_err(e.to_string()))?;
                if matches!(&plane, Reference::Element{path,..} if path.last()==Some(&row.owner)) {
                    if row.sketch_fields.plane.get("local").is_none() {
                        let pose = section_pose(scene, t, &row.owner, v)?
                            .ok_or_else(|| PyValueError::new_err("Sketch plane is unavailable"))?;
                        row.sketch_fields.plane = serde_json::json!({"local":pose.to_cols_array()});
                    }
                } else {
                    row.inputs.push(plane);
                }
            }
        }
        Some(Cutter::Bisector(uid)) if matches!(op, "trim" | "subtract") => {
            row.inputs.push(Reference::Placement {
                placement: uid.clone(),
            });
        }
        Some(Cutter::Plane(_)) if matches!(op, "trim" | "subtract") => (),
        _ => return Err(PyValueError::new_err("Invalid cutter or operation")),
    }
    if !matches!(row.end.as_deref(), None | Some("start" | "end")) {
        return Err(PyValueError::new_err("Unknown member end"));
    }
    if !target.cuts.contains(&row.uid) {
        if let Some(sweep) = &mut target.sweep {
            if !sweep.path.definition_space {
                let g = group(scene.py(), t, &row.owner)
                    .ok_or_else(|| PyValueError::new_err("Missing cut target"))?;
                let xf = g.getattr("xform")?;
                let inverse = if xf.is_none() {
                    DMat4::IDENTITY
                } else {
                    crate::math::mat(&xf)?.inverse()
                };
                for p in &mut sweep.path.points {
                    *p = inverse.transform_point3((*p).into()).to_array();
                }
                for end in sweep.path.ends.iter_mut().flatten() {
                    if let sweep::End::Point(p) = end {
                        *p = inverse.transform_point3((*p).into()).to_array();
                    }
                }
                sweep.path.definition_space = true;
            }
        }
        target.cuts.push(row.uid.clone());
        t.insert(target).map_err(PyValueError::new_err)?;
    }
    bind_placements(scene, t, row)
}

/// End centres along the longest definition axis, as used for box junction roles.
pub(crate) fn solid_axis(mesh: &Obj<'_>) -> PyResult<Vec<[f64; 3]>> {
    let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
    let store = store.try_borrow()?;
    let s = &store.inner;
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = -lo;
    for v in s.live_slots(Kind::Vertex) {
        let p = DVec3::from(
            s.state
                .frame
                .definition_point(*s.state.vpos.get(v as usize)),
        );
        lo = lo.min(p);
        hi = hi.max(p);
    }
    if !lo.is_finite() {
        return Ok(Vec::new());
    }
    let size = hi - lo;
    let axis = (0..3).max_by(|&a, &b| size[a].total_cmp(&size[b])).unwrap();
    let mut start = (lo + hi) * 0.5;
    let mut end = start;
    start[axis] = lo[axis];
    end[axis] = hi[axis];
    Ok(vec![start.to_array(), end.to_array()])
}

pub(super) fn end_planes(
    scene: &Obj<'_>,
    t: &LiveTable,
    target: &Row,
    path: &[[f64; 3]],
    active: &mut Option<String>,
) -> PyResult<Result<Vec<EndPlane>, &'static str>> {
    let mut result = Vec::new();
    for id in &target.cuts {
        let Some(row) = t.rows.get(id) else {
            return Ok(Err("ref_lost"));
        };
        if row.on == Some(false)
            || matches!(
                row.cutter,
                Some(
                    Cutter::Hole(_)
                        | Cutter::Body(_)
                        | Cutter::Section(_)
                        | Cutter::Opening(_)
                        | Cutter::Repeat(_)
                )
            )
        {
            continue;
        }
        *active = Some(id.clone());
        if path.len() < 2 {
            return Ok(Err("unsupported_cut_path"));
        }
        let end = row.end.as_deref() != Some("start");
        let closed = target.sweep.as_ref().is_some_and(|s| s.path.closed);
        let count = if closed { path.len() } else { path.len() - 1 };
        let span = row.span.unwrap_or(if end { count - 1 } else { 0 });
        if span >= count {
            return Ok(Err("end_unresolved"));
        }
        let a = DVec3::from(path[span]);
        let b = DVec3::from(path[(span + 1) % path.len()]);
        let tip = if end { b } else { a };
        let along = (b - a).normalize_or_zero();
        let (mut origin, mut normal) = match &row.cutter {
            Some(Cutter::Plane(p)) => (
                DVec3::from(p.origin),
                p.normal.map(DVec3::from).unwrap_or(along),
            ),
            Some(Cutter::Bisector(uid)) => {
                let Some(host) = group(scene.py(), t, uid) else {
                    return Ok(Err("ref_lost"));
                };
                let points = match metadata(&host)?
                    .get_item(EXT)?
                    .and_then(|m| m.get_item("path").ok())
                    .and_then(|p| p.extract::<Vec<[f64; 3]>>().ok())
                    .filter(|p| p.len() >= 2)
                {
                    Some(path) => path,
                    None => {
                        let Some(mesh) = junction::uncut_mesh(scene, t, uid)? else {
                            return Ok(Err("ref_lost"));
                        };
                        solid_axis(&mesh)?
                    }
                };
                if points.len() < 2 {
                    return Ok(Err("end_unresolved"));
                }
                let m = reference::frame(t.groups[&target.owner]).inverse()
                    * reference::frame(t.groups[uid]);
                let (_, pair) = points
                    .windows(2)
                    .enumerate()
                    .min_by(|(_, a), (_, b)| {
                        let distance = |s: &[[f64; 3]]| {
                            s.iter()
                                .map(|p| m.transform_point3((*p).into()).distance_squared(tip))
                                .fold(f64::INFINITY, f64::min)
                        };
                        distance(a).total_cmp(&distance(b))
                    })
                    .unwrap();
                let a = m.transform_point3(pair[0].into());
                let b = m.transform_point3(pair[1].into());
                let (near, far) = if tip.distance_squared(a) <= tip.distance_squared(b) {
                    (a, b)
                } else {
                    (b, a)
                };
                let outward = if end { along } else { -along };
                let partner = (far - near).normalize_or_zero();
                // A post can meet the beam anywhere along its length. Intersect
                // their axes, limiting the cut to the extent of ordinary stock.
                let cross = outward.cross(partner);
                if cross.length_squared() < 1e-12 {
                    return Ok(Err("end_unresolved"));
                }
                let station = (near - tip).cross(outward).dot(cross) / cross.length_squared();
                let meeting = near + partner * station.clamp(0., near.distance(far));
                let origin = if target.kind == "cut_stack" {
                    let start = DVec3::from(path[span]);
                    let finish = DVec3::from(path[(span + 1) % path.len()]);
                    start
                        + along
                            * (meeting - start)
                                .dot(along)
                                .clamp(0., start.distance(finish))
                } else {
                    meeting
                };
                (origin, outward + partner)
            }
            _ => return Ok(Err("unsupported")),
        };
        if let Some(anchor) = &row.anchor {
            let Some(p) = reference::origin(scene, t, &anchor.reference, row)? else {
                return Ok(Err("ref_lost"));
            };
            origin = p;
            if let Reference::Frame { junction, frame } = &anchor.reference {
                let Some(m) = super::junction::local_frame(t, &row.owner, junction, frame) else {
                    return Ok(Err("ref_lost"));
                };
                normal = m.z_axis.truncate();
            }
            if let Some(face) =
                reference::element_points(scene, t, row, &anchor.reference, Kind::Face)?
            {
                let Some((_, n)) =
                    crate::snap_planes::face_plane(face.iter().map(|p| p.to_array()).collect())
                else {
                    return Ok(Err("end_unresolved"));
                };
                normal = n.into();
            }
        }
        let Some(normal) = normal.try_normalize() else {
            return Ok(Err("end_unresolved"));
        };
        let offset = row.anchor.as_ref().map(|a| a.offset).unwrap_or_else(|| {
            if matches!(row.cutter, Some(Cutter::Bisector(_))) {
                row.clearance.unwrap_or(0.)
            } else {
                row.offset.unwrap_or(0.)
            }
        });
        let normal = if let Some(Cutter::Plane(p)) = &row.cutter {
            tilted(normal, p.angle)
        } else {
            normal
        };
        origin += normal * (offset / 1000.);
        result.push(EndPlane {
            uid: id.clone(),
            span,
            end,
            origin,
            normal,
        });
    }
    Ok(Ok(result))
}

fn section_pose(
    scene: &Obj<'_>,
    t: &LiveTable,
    owner: &str,
    plane: &serde_json::Value,
) -> PyResult<Option<DMat4>> {
    let Some(id) = plane["id"].as_str() else {
        return Ok(None);
    };
    let path = serde_json::from_value(plane["path"].clone())
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let world = sketch_scene::sketch_plane(scene, id, Some(path))?;
    if world.is_none() {
        return Ok(None);
    }
    let axis = |key| -> PyResult<DVec3> { Ok(world.get_item(key)?.extract::<[f64; 3]>()?.into()) };
    Ok(Some(
        reference::frame(t.groups[owner]).inverse()
            * DMat4::from_cols(
                axis("x")?.extend(0.),
                axis("y")?.extend(0.),
                axis("n")?.extend(0.),
                axis("origin")?.extend(1.),
            ),
    ))
}
pub(super) fn section_frame(
    scene: &Obj<'_>,
    t: &LiveTable,
    target: &Row,
    cut: &Row,
    section: &Row,
) -> PyResult<Option<DMat4>> {
    let mut pose = DMat4::IDENTITY;
    if let Some(plane) = section.sketch_fields.plane.get("ref") {
        if let Some(local) = cut.sketch_fields.plane.get("local") {
            pose = DMat4::from_cols_array(
                &serde_json::from_value(local.clone())
                    .map_err(|e| PyValueError::new_err(e.to_string()))?,
            );
        } else if let Some(frame) = section_pose(scene, t, &target.owner, plane)? {
            pose = frame;
        } else {
            return Ok(None);
        }
    }
    pose *= DMat4::from_translation(DVec3::Z * cut.offset.unwrap_or(0.) / 1000.);
    if let Some(anchor) = &cut.anchor {
        let Some(point) = reference::origin(scene, t, &anchor.reference, cut)? else {
            return Ok(None);
        };
        pose.w_axis =
            (point + pose.z_axis.truncate().normalize_or_zero() * anchor.offset / 1000.).extend(1.);
    }
    Ok(Some(pose))
}
fn operand<'p>(
    scene: &Obj<'p>,
    t: &LiveTable,
    target: &Row,
    cut: &Row,
    mesh: &Obj<'p>,
) -> PyResult<Result<Obj<'p>, &'static str>> {
    let py = scene.py();
    let (tool_mesh, mut pose) = match &cut.cutter {
        Some(Cutter::Plane(_) | Cutter::Bisector(_)) => {
            let mut single = target.clone();
            single.cuts = vec![cut.uid.clone()];
            let bisector = matches!(cut.cutter, Some(Cutter::Bisector(_)));
            let path = if bisector {
                let Some(stock) = source(scene, target.source.as_deref().unwrap_or(""))? else {
                    return Ok(Err("ref_lost"));
                };
                solid_axis(&stock.getattr("mesh")?)?
            } else {
                vec![[0.; 3], [0., 0., 1.]]
            };
            let mut planes = match end_planes(scene, t, &single, &path, &mut None)? {
                Ok(planes) => planes,
                Err(state) => return Ok(Err(state)),
            };
            let Some(plane) = planes.pop() else {
                return Ok(Err("end_unresolved"));
            };
            let origin = plane.origin;
            let n = if !bisector && cut.end.as_deref() == Some("start") {
                -plane.normal
            } else {
                plane.normal
            };
            let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
            let store = store.try_borrow()?;
            let s = &store.inner;
            let radius = s
                .live_slots(Kind::Vertex)
                .into_iter()
                .map(|i| {
                    DVec3::from(
                        s.state
                            .frame
                            .definition_point(*s.state.vpos.get(i as usize)),
                    )
                    .distance(origin)
                })
                .fold(0., f64::max)
                + 1.;
            let x = n.any_orthonormal_vector() * radius;
            let y = n.cross(x);
            let frame = DMat4::from_cols(
                (2. * x).extend(0.),
                (2. * y).extend(0.),
                (radius * n).extend(0.),
                (origin - x - y).extend(1.),
            );
            let m = py.import("built_core.mesh")?.call_method0("Mesh")?;
            let handle = m.getattr("_store")?.cast_into::<PyMeshStore>()?;
            geometry::generate(&mut handle.try_borrow_mut()?.inner, cut, [0.; 3])
                .map_err(PyValueError::new_err)?;
            (m, frame)
        }
        Some(Cutter::Body(uid)) => {
            let Some(body) = group(py, t, uid) else {
                return Ok(Err("ref_lost"));
            };
            (
                body.getattr("mesh")?,
                reference::frame(t.groups[&target.owner]).inverse()
                    * reference::frame(t.groups[uid]),
            )
        }
        Some(Cutter::Hole(h)) => {
            let Some(pose) = h.pose(scene, t, cut)? else {
                return Ok(Err("ref_lost"));
            };
            let tool = py.import("built_core.mesh")?.call_method0("Mesh")?;
            let dst = tool.getattr("_store")?.cast_into::<PyMeshStore>()?;
            let src = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
            if let Err(state) = h.tool(
                &mut dst.try_borrow_mut()?.inner,
                &src.try_borrow()?.inner,
                pose,
            ) {
                return Ok(Err(state));
            }
            (tool, pose)
        }
        Some(Cutter::Opening(opening)) => {
            let m = py.import("built_core.mesh")?.call_method0("Mesh")?;
            let handle = m.getattr("_store")?.cast_into::<PyMeshStore>()?;
            geometry::generate(&mut handle.try_borrow_mut()?.inner, cut, [0.; 3])
                .map_err(PyValueError::new_err)?;
            (m, opening.pose())
        }
        Some(Cutter::Section(uid)) => {
            let Some(section) = t.rows.get(uid) else {
                return Ok(Err("ref_lost"));
            };
            if section.state != "ok" {
                return Ok(Err("profile_open"));
            }
            let Some(pose) = section_frame(scene, t, target, cut, section)? else {
                return Ok(Err("ref_lost"));
            };
            let extrusion = if cut.sketch_fields.extrude.is_object() {
                &cut.sketch_fields.extrude
            } else {
                &section.sketch_fields.extrude
            };
            let depth = extrusion["depth"].as_f64().unwrap_or(100.) / 1000.;
            let (from, to) = match extrusion["extent"].as_str().unwrap_or("through") {
                "depth" => (0., depth),
                "both" => (-depth, depth),
                "through" => {
                    let inverse = pose.inverse();
                    let mut lo = f64::INFINITY;
                    let mut hi = f64::NEG_INFINITY;
                    let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
                    let borrow = store.try_borrow()?;
                    let s = &borrow.inner;
                    for slot in s.live_slots(Kind::Vertex) {
                        let p = s
                            .state
                            .frame
                            .definition_point(*s.state.vpos.get(slot as usize));
                        let z = inverse.transform_point3(p.into()).z;
                        lo = lo.min(z);
                        hi = hi.max(z);
                    }
                    (lo - 0.001, hi + 0.001)
                }
                _ => return Ok(Err("extent_unresolved")),
            };
            let path = [[0., 0., from], [0., 0., to]];
            let spec:sweep::Sweep=serde_json::from_value(serde_json::json!({"profile":{"drawing":section},"attachment":"origin","path":{"points":path}})).map_err(|e|PyValueError::new_err(e.to_string()))?;
            let mesh = py.import("built_core.mesh")?.call_method0("Mesh")?;
            let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
            if let Err(state) =
                sweep::generate(&mut store.try_borrow_mut()?.inner, &spec, &path, &[])
            {
                return Ok(Err(state));
            }
            (mesh, pose)
        }
        _ => return Ok(Err("unsupported")),
    };
    if let Some(anchor) = cut.anchor.as_ref().filter(|_| {
        !matches!(
            cut.cutter,
            Some(Cutter::Hole(_) | Cutter::Plane(_) | Cutter::Bisector(_) | Cutter::Section(_))
        )
    }) {
        let Some(point) = reference::origin(scene, t, &anchor.reference, cut)? else {
            return Ok(Err("ref_lost"));
        };
        pose.w_axis =
            (point + pose.z_axis.truncate().normalize_or_zero() * anchor.offset / 1000.).extend(1.);
    }
    let group_type = py.import("built_core.group")?.getattr("Group")?;
    let tool = group_type.call0()?;
    tool.setattr("mesh", tool_mesh)?;
    tool.setattr("xform", Py::new(py, crate::math::Matrix4::from(pose))?)?;
    Ok(Ok(tool))
}

/// Opening geometry for interchange, in the target's definition frame.
/// Regenerate the tool on demand, so a moving wall never saves world faces.
#[pyfunction(signature=(scene,owner,all_operations=false,skip_holes=false))]
pub(super) fn live_openings<'p>(
    scene: &Obj<'p>,
    owner: &str,
    all_operations: bool,
    skip_holes: bool,
) -> PyResult<Bound<'p, pyo3::types::PyList>> {
    let py = scene.py();
    let result = pyo3::types::PyList::empty(py);
    let Some(t) = current(py, root(scene)?) else {
        return Ok(result);
    };
    let Some(target) = t.owners.get(owner).and_then(|id| t.rows.get(id)) else {
        return Ok(result);
    };
    if target.state != "ok" {
        return Ok(result);
    }
    let Some(g) = group(py, &t, owner) else {
        return Ok(result);
    };
    for id in &target.cuts {
        let Some(cut) = t.rows.get(id) else {
            continue;
        };
        if cut.on == Some(false)
            || cut.state != "ok"
            || (skip_holes && matches!(cut.cutter, Some(Cutter::Hole(_))))
            || (!all_operations
                && !matches!(cut.operation.as_deref(), None | Some("trim" | "subtract")))
            || !(matches!(
                cut.cutter,
                Some(
                    Cutter::Hole(_)
                        | Cutter::Body(_)
                        | Cutter::Section(_)
                        | Cutter::Opening(_)
                        | Cutter::Repeat(_)
                )
            ) || all_operations
                && target.kind == "cut_stack"
                && matches!(cut.cutter, Some(Cutter::Plane(_) | Cutter::Bisector(_))))
        {
            continue;
        }
        if let Ok(tool) = operand(scene, &t, target, cut, &g.getattr("mesh")?)? {
            let guid = if let Some(Cutter::Opening(o)) = &cut.cutter {
                Some(&o.guid)
            } else {
                None
            };
            let gap = cut.clearance.unwrap_or(0.);
            let tool = if !all_operations && gap > 0. {
                crate::solids::clearance_operand(&tool, gap)?
            } else {
                tool
            };
            result.append((id, tool, guid))?;
        }
    }
    Ok(result)
}
fn hole_stations(
    scene: &Obj<'_>,
    t: &LiveTable,
    cut: &Row,
    h: &hole::Hole,
    pose: DMat4,
) -> PyResult<Result<Vec<(String, DMat4)>, &'static str>> {
    let mut stations = vec![(cut.uid.clone(), pose)];
    stations.extend(
        h.stations
            .iter()
            .map(|(key, m)| (key.clone(), pose * DMat4::from_cols_array(m))),
    );
    if let Some(uid) = &h.repeat {
        let Some(r) = t.rows.get(uid) else {
            return Ok(Err("ref_lost"));
        };
        let loaded_layout;
        let layout = if let Some(layout) = t.stock.get(uid).and_then(|s| s.layout.as_ref()) {
            layout
        } else {
            loaded_layout = match repeat::layout(scene, t, r)? {
                Ok(layout) => layout,
                Err(state) => return Ok(Err(state)),
            };
            &loaded_layout
        };
        let into_target =
            reference::frame(t.groups[&cut.owner]).inverse() * reference::frame(t.groups[&r.owner]);
        for (i, _, copy) in &layout.poses {
            let delta = into_target * *copy * layout.base.inverse() * into_target.inverse();
            stations.push((format!("{}/{i}", cut.uid), delta * pose));
        }
    }
    Ok(Ok(stations))
}

/// Measured bore extents in model coordinates, keyed by persistent station.
#[pyfunction]
pub(super) fn live_holes(
    scene: &Obj<'_>,
    uid: &str,
) -> PyResult<Vec<(String, [f64; 3], [f64; 3], f64)>> {
    let Some(t) = current(scene.py(), root(scene)?) else {
        return Ok(vec![]);
    };
    let Some(row) = t.rows.get(uid) else {
        return Ok(vec![]);
    };
    let Some(Cutter::Hole(h)) = &row.cutter else {
        return Ok(vec![]);
    };
    if row.on == Some(false) || row.state != "ok" {
        return Ok(vec![]);
    }
    let Some(pose) = h.pose(scene, &t, row)? else {
        return Ok(vec![]);
    };
    let Ok(stations) = hole_stations(scene, &t, row, h, pose)? else {
        return Ok(vec![]);
    };
    let Some(group) = group(scene.py(), &t, &row.owner) else {
        return Ok(vec![]);
    };
    let handle = group
        .getattr("mesh")?
        .getattr("_store")?
        .cast_into::<PyMeshStore>()?;
    let mesh = handle.try_borrow()?;
    let st = &mesh.inner;
    let world = reference::frame(t.groups[&row.owner]);
    let generated =
        eid::intern(geometry::h64(b"built:generated:v1")).map_err(PyValueError::new_err)?;
    let mut holes = Vec::new();
    for (key, m) in stations {
        let inv = m.inverse();
        let height = |slot: u32| {
            inv.transform_point3(
                st.state
                    .frame
                    .definition_point(*st.state.vpos.get(slot as usize))
                    .into(),
            )
            .z
        };
        let cap = |i| {
            let role = format!("hole/{key}/{i}/vertex/0");
            let id = eid::pack(
                generated,
                geometry::h64(role.as_bytes()) & eid::COUNTER_MASK,
            );
            st.slot_of_pid(Kind::Vertex, id).map(height)
        };
        let (lo, hi) = match (cap(0), cap(1)) {
            (Some(a), Some(b)) => (a, b),
            _ if h.depth.is_number() => (-h.depth.as_f64().unwrap() / 1000., 0.),
            _ => st
                .live_slots(Kind::Vertex)
                .into_iter()
                .map(height)
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), z| {
                    (a.min(z), b.max(z))
                }),
        };
        if hi > lo {
            let at = |z| (world * m).transform_point3(DVec3::Z * z).to_array();
            holes.push((key, at(lo), at(hi), h.d / 2000.));
        }
    }
    Ok(holes)
}
pub(super) fn bodies<'p>(
    scene: &Obj<'p>,
    t: &LiveTable,
    target: &Row,
    mut mesh: Obj<'p>,
    active: &mut Option<String>,
    skipped: &mut Vec<(String, &'static str)>,
) -> PyResult<Result<Obj<'p>, &'static str>> {
    let py = scene.py();
    let rows: Vec<_> = target
        .cuts
        .iter()
        .filter_map(|id| t.rows.get(id))
        .filter(|r| r.on != Some(false))
        .collect();
    let body_count = rows
        .iter()
        .filter(|r| matches!(r.cutter, Some(Cutter::Body(_))))
        .count();
    if body_count > 1
        && rows.iter().all(|r| {
            (matches!(r.cutter, Some(Cutter::Body(_)))
                || target.kind == "sweep" && matches!(r.cutter, Some(Cutter::Bisector(_))))
                || (target.kind == "sweep" && matches!(r.cutter, Some(Cutter::Plane(_))))
        })
    {
        let mut tools = Vec::with_capacity(body_count);
        for cut in rows
            .into_iter()
            .filter(|r| matches!(r.cutter, Some(Cutter::Body(_))))
        {
            *active = Some(cut.uid.clone());
            let tool = match operand(scene, t, target, cut, &mesh)? {
                Ok(t) => t,
                Err(state) => return Ok(Err(state)),
            };
            tools.push((
                cut.uid.clone(),
                cut.operation.clone().unwrap_or_else(|| "subtract".into()),
                cut.clearance.unwrap_or(0.),
                tool,
            ));
        }
        let subject = py
            .import("built_core.group")?
            .getattr("Group")?
            .call1((&mesh,))?;
        return crate::solids::body_stack(&subject, tools, active, skipped);
    }
    for id in &target.cuts {
        let Some(cut) = t.rows.get(id) else {
            return Ok(Err("ref_lost"));
        };
        if cut.on == Some(false) {
            continue;
        }
        if !(target.kind == "cut_stack"
            && matches!(cut.cutter, Some(Cutter::Plane(_) | Cutter::Bisector(_))))
            && !matches!(
                cut.cutter,
                Some(
                    Cutter::Hole(_)
                        | Cutter::Body(_)
                        | Cutter::Section(_)
                        | Cutter::Opening(_)
                        | Cutter::Repeat(_)
                )
            )
        {
            continue;
        }
        if let Some(Cutter::Hole(h)) = &cut.cutter {
            *active = Some(id.clone());
            if !h.valid() {
                return Ok(Err("invalid_hole"));
            }
            let Some(pose) = h.pose(scene, t, cut)? else {
                return Ok(Err("ref_lost"));
            };
            if target.kind == "cut_stack" {
                if let Some(source) = source(scene, target.source.as_deref().unwrap_or(""))? {
                    if mesh.is(&source.getattr("mesh")?) {
                        mesh = py.import("copy")?.call_method1("deepcopy", (&mesh,))?;
                    }
                }
            }
            let stations = match hole_stations(scene, t, cut, h, pose)? {
                Ok(stations) => stations,
                Err(state) => return Ok(Err(state)),
            };
            for (key, m) in stations {
                let dst = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
                match h.drill(&mut dst.try_borrow_mut()?.inner, m, &key) {
                    Ok(true) => continue,
                    Ok(false) => (),
                    Err(state) => return Ok(Err(state)),
                }
                let mut single = (**cut).clone();
                let mut tool_hole = h.clone();
                tool_hole.frame = Some(m.to_cols_array());
                tool_hole.repeat = None;
                single.anchor = None;
                single.cutter = Some(Cutter::Hole(tool_hole));
                let tool = match operand(scene, t, target, &single, &mesh)? {
                    Ok(tool) => tool,
                    Err(s) => return Ok(Err(s)),
                };
                let cls = py.import("built_core.group")?.getattr("Group")?;
                let subject = cls.call1((&mesh,))?;
                let mut misses = Vec::new();
                match crate::solids::body_stack(
                    &subject,
                    vec![(key, "subtract".into(), 0., tool)],
                    &mut None,
                    &mut misses,
                )? {
                    Ok(result) => mesh = result,
                    Err(s) => return Ok(Err(s)),
                }
                if let Some((_, state)) = misses.first() {
                    skipped.push((id.clone(), *state));
                }
            }
            continue;
        }
        if let Some(Cutter::Repeat(uid)) = &cut.cutter {
            let Some(repeat) = t.rows.get(uid).filter(|r| r.state == "ok") else {
                return Ok(Err("ref_lost"));
            };
            let Some(owner) = group(py, t, &repeat.owner) else {
                return Ok(Err("ref_lost"));
            };
            let Some(source) = repeat.source.as_ref().and_then(|s| group(py, t, s)) else {
                return Ok(Err("ref_lost"));
            };
            let target_inv = reference::frame(t.groups[&target.owner]).inverse();
            let cls = py.import("built_core.group")?.getattr("Group")?;
            let mut tools = Vec::new();
            for station in std::iter::once(Ok(source)).chain(owner.getattr("children")?.try_iter()?)
            {
                let station = station?;
                let tool = cls.call1((station.getattr("mesh")?,))?;
                tool.setattr(
                    "xform",
                    crate::math::Matrix4::from(
                        target_inv * reference::frame(station.getattr("_doc_key")?.extract()?),
                    ),
                )?;
                let station_uid = station.getattr("uid")?.extract::<String>()?;
                tools.push((
                    format!("{id}/{station_uid}"),
                    cut.operation.clone().unwrap_or_else(|| "subtract".into()),
                    cut.clearance.unwrap_or(0.),
                    tool,
                ));
            }
            let subject = cls.call1((&mesh,))?;
            let mut misses = Vec::new();
            match crate::solids::body_stack(&subject, tools, &mut None, &mut misses)? {
                Ok(result) => mesh = result,
                Err(state) => return Ok(Err(state)),
            }
            if let Some((_, state)) = misses.first() {
                skipped.push((id.clone(), *state));
            }
            continue;
        }
        *active = Some(id.clone());
        let tool = match operand(scene, t, target, cut, &mesh)? {
            Ok(tool) => tool,
            Err(state) => return Ok(Err(state)),
        };
        let subject = py
            .import("built_core.group")?
            .getattr("Group")?
            .call1((&mesh,))?;
        let op = cut.operation.as_deref().unwrap_or("subtract");
        let args = if matches!(op, "trim" | "subtract") {
            [tool, subject]
        } else {
            [subject, tool]
        };
        match crate::solids::operation(
            op,
            pyo3::types::PyList::new(py, args)?.as_any(),
            Some(id),
            cut.clearance.unwrap_or(0.),
        ) {
            Ok((_, results)) => mesh = results.get_item(0)?.getattr("mesh")?,
            Err(e) if e.is_instance_of::<crate::solids::SolidError>(py) => {
                skipped.push((
                    id.clone(),
                    match e.value(py).str()?.to_str()? {
                        "no overlap" => "cut_misses",
                        "empty" => "empty_cut",
                        _ => "boolean_failed",
                    },
                ));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(Ok(mesh))
}

/// A surviving donor keeps its face ID. For split donors, sort the pieces
/// spatially to distinguish them. Edges and vertices use the bounding roles.
pub(crate) fn assign_cut_roles(
    mesh: &Obj<'_>,
    ids: &Bound<'_, PyDict>,
    donors: &[(String, u64, bool)],
    uid: &str,
) -> PyResult<()> {
    let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
    let mut borrow = store.try_borrow_mut()?;
    let st = &mut borrow.inner;
    let mut pieces = std::collections::BTreeMap::<String, Vec<(u32, DVec3, Option<u64>)>>::new();
    for (key, donor) in ids.iter() {
        let Some(f) = st.slot(Kind::Face, key.extract()?) else {
            continue;
        };
        let donor = donor.extract::<usize>().ok().and_then(|i| donors.get(i));
        let role = donor
            .map(|r| r.0.clone())
            .unwrap_or_else(|| format!("cut/{uid}/clearance"));
        let ring = st.face_rings(f).remove(0);
        let centre = ring
            .iter()
            .map(|v| DVec3::from(*st.state.vpos.get(*v as usize)))
            .sum::<DVec3>()
            / (ring.len() as f64);
        pieces
            .entry(role)
            .or_default()
            .push((f, centre, donor.filter(|r| !r.2).map(|r| r.1)));
    }
    let mut roles = Vec::new();
    let mut inherited = Vec::new();
    let mut faces = HashMap::new();
    for (role, mut list) in pieces {
        list.sort_by(|a, b| {
            a.1.x
                .total_cmp(&b.1.x)
                .then(a.1.y.total_cmp(&b.1.y))
                .then(a.1.z.total_cmp(&b.1.z))
        });
        for (index, (f, _, pid)) in list.into_iter().enumerate() {
            let label = format!("{role}/{index}");
            faces.insert(f, label.clone());
            if let Some(pid) = pid.filter(|_| index == 0) {
                inherited.push((f, pid));
            } else {
                roles.push((label, Kind::Face, f));
            }
        }
    }
    let mut vertices = HashMap::new();
    for v in st.live_slots(Kind::Vertex) {
        let mut sides: Vec<_> = st
            .faces_around(&[v])
            .iter()
            .filter_map(|f| faces.get(f))
            .collect();
        sides.sort();
        let role = format!("corner/{sides:?}");
        vertices.insert(v, role.clone());
        roles.push((role, Kind::Vertex, v));
    }
    for e in st.live_slots(Kind::Edge) {
        let mut ends = st.state.ends.get(e as usize).map(|v| &vertices[&v]);
        ends.sort();
        roles.push((format!("rim/{ends:?}"), Kind::Edge, e));
    }
    geometry::assign_roles(st, roles).map_err(PyValueError::new_err)?;
    for (f, pid) in inherited {
        st.set_pid(Kind::Face, f, pid);
    }
    Ok(())
}

pub(crate) fn pushed_row<'p>(
    scene: &Obj<'p>,
    group: &Obj<'p>,
    face: &Obj<'p>,
    distance: f64,
) -> PyResult<Option<Obj<'p>>> {
    if group.is_none() || face.is_none() {
        return Ok(None);
    }
    let Some(t) = current(scene.py(), root(scene)?) else {
        return Ok(None);
    };
    let owner: String = group.getattr("uid")?.extract()?;
    let Some(row) = t.owners.get(&owner).and_then(|id| t.rows.get(id)) else {
        return Ok(None);
    };
    if row.cuts.is_empty() {
        return Ok(None);
    }
    let face_pid = group
        .getattr("mesh")?
        .getattr("_store")?
        .call_method1("id_of", ("face", face.getattr("key")?));
    let pid = face_pid.and_then(|i| i.extract::<u64>()).ok();
    let tag = face
        .getattr("attrs")?
        .call_method1("get", ("built.cut_face",))?
        .extract::<Option<String>>()?;
    for id in &row.cuts {
        let Some(cut) = t.rows.get(id) else {
            continue;
        };
        if cut.on == Some(false) {
            continue;
        }
        let plane = matches!(cut.cutter, Some(Cutter::Plane(_) | Cutter::Bisector(_)));
        if !(plane
            && pid.is_some_and(|pid| geometry::matches_role(pid, &format!("cut/{id}/plane"))))
            && tag.as_ref() != Some(id)
        {
            continue;
        }
        let mut edit = (**cut).clone();
        let delta = distance * 1000.;
        match edit.cutter.as_mut() {
            Some(Cutter::Bisector(_)) if edit.anchor.is_none() => {
                edit.clearance = Some(edit.clearance.unwrap_or(0.) + delta);
            }
            Some(Cutter::Plane(p)) if edit.anchor.is_none() => {
                let normal = crate::math::xyz(&face.call_method0("normal")?)?;
                p.origin = (DVec3::from(p.origin) + distance * normal).to_array();
            }
            Some(Cutter::Section(uid)) => {
                let original = t.rows.get(uid).map(|r| &r.sketch_fields.extrude);
                if !edit.sketch_fields.extrude.is_object() {
                    edit.sketch_fields.extrude = original.cloned().unwrap_or_default();
                }
                let depth = edit.sketch_fields.extrude["depth"].as_f64().unwrap_or(100.);
                edit.sketch_fields.extrude =
                    serde_json::json!({"extent":"depth","depth":(depth+delta).max(0.001)});
            }
            Some(Cutter::Body(_)) => {
                edit.clearance = Some((edit.clearance.unwrap_or(0.) - delta).max(0.));
            }
            _ if edit.anchor.is_some() => {
                edit.anchor.as_mut().unwrap().offset += delta;
            }
            _ => return Ok(None),
        }
        return Ok(Some(to_py(scene.py(), &edit)?));
    }
    Ok(None)
}

/// A legacy wall stored its extents as a unit-cube transform. A rectangle
/// swept along its centreline reproduces that box in definition coordinates.
pub(crate) fn migrate_wall(scene: &Obj<'_>, wall: &Obj<'_>, frame: [f64; 16]) -> PyResult<()> {
    let root = root(scene)?;
    let mut t = current(scene.py(), root).unwrap_or_else(|| LiveTable {
        groups: Arc::new(index(root)),
        ..Default::default()
    });
    let owner: String = wall.getattr("uid")?.extract()?;
    if t.owners.contains_key(&owner) {
        return Ok(());
    }
    let frame = DMat4::from_cols_array(&frame);
    let (x, y, z) = (
        frame.x_axis.truncate(),
        frame.y_axis.truncate(),
        frame.z_axis.truncate(),
    );
    let axis = x.normalize_or_zero();
    if axis.dot(y.normalize_or_zero()).abs() > 1e-8
        || axis.dot(z.normalize_or_zero()).abs() > 1e-8
        || y.normalize_or_zero().dot(z.normalize_or_zero()).abs() > 1e-8
    {
        return Ok(());
    }
    let across = DVec3::Z.cross(axis).try_normalize().unwrap_or(DVec3::X);
    let rotation = y.dot(axis.cross(across)).atan2(y.dot(across)).to_degrees();
    let a = frame.transform_point3(DVec3::new(0., 0.5, 0.5));
    let row:Row=serde_json::from_value(serde_json::json!({
        "uid":eid::derived_uid(&format!("built:wall:v1:{owner}")),"owner":owner,"kind":"sweep",
        "preset":"f76ea921edc2e55d","attachment":"centre","rotation":rotation,
        "profile":{"library":"basic/wall","params":{"width":y.length()*1000.,"height":z.length()*1000.}},
        "path":{"points":[a.to_array(),(a+x).to_array()],"definition_space":true}
    })).map_err(|e|PyValueError::new_err(e.to_string()))?;
    t.insert(row).map_err(PyValueError::new_err)?;
    put(scene, t)
}

pub(crate) fn hosted(
    scene: &Obj<'_>,
    wall: &Obj<'_>,
    fill: &Obj<'_>,
    opening: Option<(String, [f64; 16], [f64; 4])>,
) -> PyResult<String> {
    let document = root(scene)?;
    let mut t = current(scene.py(), document).unwrap_or_else(|| LiveTable {
        groups: Arc::new(index(document)),
        ..Default::default()
    });
    let owner: String = wall.getattr("uid")?.extract()?;
    let fill_uid: String = fill.getattr("uid")?.extract()?;
    let groups = Arc::make_mut(&mut t.groups);
    groups.insert(owner.clone(), wall.getattr("_doc_key")?.extract()?);
    groups.insert(fill_uid.clone(), fill.getattr("_doc_key")?.extract()?);
    let uid = eid::derived_uid(&format!("built:opening:v1:{owner}:{fill_uid}"));
    if let Some((guid, frame, rect)) = opening {
        let mut row:Row=serde_json::from_value(serde_json::json!({
            "uid":uid,"owner":owner,"kind":"cut","operation":"subtract","cutter":{"opening":{ "fill":fill_uid,"guid":guid,"frame":frame,"rect":rect }}
        })).map_err(|e|PyValueError::new_err(e.to_string()))?;
        if let Some(old) = t.rows.get(&uid) {
            row.on = old.on;
        }
        attach(scene, &mut t, &mut row)?;
        t.insert(row.clone()).map_err(PyValueError::new_err)?;
        junction::hosting::adopt(scene, &mut t, &row, &fill_uid, false)?;
        t.pending.insert(uid.clone());
    } else if let Some(old) = t.remove(&uid) {
        if let Some(j) = old.junction.as_ref().and_then(|id| t.rows.get(id)).cloned() {
            junction::remove_descendants(&mut t, &j.uid)?;
            if let Some(g) = group(scene.py(), &t, &j.owner) {
                let parent = doc::lock()
                    .node(t.groups[&j.owner])
                    .and_then(|n| n.owners.first().copied());
                let siblings = parent
                    .and_then(|k| doc::lock().handle_ref(scene.py(), k))
                    .and_then(|p| p.bind(scene.py()).getattr("children").ok())
                    .unwrap_or(scene.getattr("groups")?);
                siblings.call_method1("remove", (g,))?;
            }
            t.remove(&j.uid);
            Arc::make_mut(&mut t.groups).remove(&j.owner);
        }
        let target = t.owners[&owner].clone();
        let mut row = (*t.rows[&target]).clone();
        row.cuts.retain(|id| id != &uid);
        if row.cuts.is_empty() && row.kind == "cut_stack" {
            let source = source(scene, row.source.as_deref().unwrap_or(""))?
                .ok_or_else(|| PyValueError::new_err("Missing cut source"))?;
            wall.setattr("mesh", source.getattr("mesh")?)?;
            t.remove(&target);
            if !t.rows.values().any(|r| r.source == row.source) {
                scene
                    .getattr("kept_components")?
                    .call_method1("remove", (source,))?;
            }
            let ext = metadata(wall)?;
            if ext.contains(EXT)? {
                ext.del_item(EXT)?;
                wall.setattr("ext", ext)?;
            }
        } else {
            t.insert(row).map_err(PyValueError::new_err)?;
            t.pending.insert(target);
        }
    }
    put(scene, t)?;
    Ok(uid)
}
