// SPDX-License-Identifier: GPL-3.0-or-later
//! Repeat placements are ordinary children of one journalled assembly.
mod notch;
use super::*;
use crate::math::Matrix4;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Fields {
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub run: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub rule: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub ends: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measure: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orientation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapt: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skips: Vec<Value>,
}

pub(super) fn prepare(scene: &Obj<'_>, t: &mut LiveTable, row: &mut Row) -> PyResult<()> {
    let py = scene.py();
    let source = row
        .source
        .as_ref()
        .ok_or_else(|| PyValueError::new_err("Select a Repeat source"))?;
    let k = *t
        .groups
        .get(source)
        .ok_or_else(|| PyValueError::new_err("Unknown Repeat source"))?;
    if let Some(existing) = t.rows.get(&row.uid) {
        if !row.owner.is_empty() && row.owner != existing.owner {
            return Err(PyValueError::new_err("recipe owner cannot change"));
        }
        if row.owner.is_empty() {
            row.owner.clone_from(&existing.owner);
        }
    } else {
        if !row.owner.is_empty() {
            return Err(PyValueError::new_err("A new Repeat creates its own owner"));
        }
        let assembly = py.import("built_core.group")?.getattr("Group")?.call0()?;
        assembly.setattr(
            "uid",
            eid::derived_uid(&format!("built:repeat:v1:{}:owner", row.uid)),
        )?;
        assembly.setattr("name", "Repeat")?;
        assembly.setattr("xform", Matrix4::from(DMat4::IDENTITY))?;
        assembly.setattr(
            "ifc",
            to_py(py, &json!({"class":"IfcElementAssembly","name":"Repeat"}))?,
        )?;
        let parent = doc::lock().node(k).and_then(|n| n.owners.first().copied());
        let parent = parent.and_then(|key| doc::lock().handle_ref(py, key));
        let siblings = match parent.filter(|g| g.bind(py).hasattr("children").unwrap_or(false)) {
            Some(g) => g.bind(py).getattr("children")?,
            None => scene.getattr("groups")?,
        };
        siblings.call_method1("append", (&assembly,))?;
        crate::structure::file_sweep(scene, &assembly, "")?;
        row.owner = assembly.getattr("uid")?.extract()?;
        Arc::make_mut(&mut t.groups)
            .insert(row.owner.clone(), assembly.getattr("_doc_key")?.extract()?);
    }
    // A generated subtree may depend on its owner through an occurrence even
    // when that occurrence has no recipe of its own.
    let owner_key = t.groups[&row.owner];
    let mut ancestor = Some(k);
    while let Some(key) = ancestor {
        if key == owner_key {
            return Err(PyValueError::new_err("cycle"));
        }
        ancestor = doc::lock()
            .node(key)
            .and_then(|n| n.owners.first().copied());
    }
    row.inputs = vec![Reference::Placement {
        placement: source.clone(),
    }];
    if let Some(source_group) = group(py, t, source) {
        let mut children = source_group
            .getattr("children")?
            .try_iter()?
            .collect::<PyResult<Vec<_>>>()?;
        while let Some(g) = children.pop() {
            row.inputs.push(Reference::Placement {
                placement: g.getattr("uid")?.extract()?,
            });
            children.extend(
                g.getattr("children")?
                    .try_iter()?
                    .collect::<PyResult<Vec<_>>>()?,
            );
        }
    }
    let run = &row.repeat.run;
    for edge in run
        .get("edge")
        .into_iter()
        .chain(run["chain"].as_array().into_iter().flatten())
    {
        row.inputs.push(
            serde_json::from_value(edge.clone())
                .map_err(|e| PyValueError::new_err(e.to_string()))?,
        );
    }
    if let Some(id) = run["between"].as_str() {
        row.inputs.push(Reference::Row { row: id.into() });
    }
    Ok(())
}

fn stations(rule: &Value, span: f64, polar: bool) -> Option<(usize, f64)> {
    if !span.is_finite() || span <= 1e-12 {
        return None;
    }
    let units = if polar { 1. } else { 0.001 };
    let (n, step) = if let Some(n) = rule["count"].as_u64() {
        (n as f64, rule["step"].as_f64().map_or(span, |s| s * units))
    } else if let Some(n) = rule["divide"].as_u64() {
        (n as f64, span / n as f64)
    } else if let Some(s) = rule["spacing"].as_f64() {
        ((span / (s * units) + 1e-10).floor(), s * units)
    } else {
        let n = (span / (rule["fit_max"].as_f64()? * units) - 1e-12).ceil();
        (n, span / n)
    };
    (n >= 0. && n <= 10000. && step.is_finite() && step > 0.).then_some((n as usize, step))
}

fn points(scene: &Obj<'_>, t: &LiveTable, row: &Row) -> PyResult<Option<Vec<DVec3>>> {
    let run = &row.repeat.run;
    if let Some(id) = run["between"].as_str() {
        let Some(host) = t
            .rows
            .get(id)
            .filter(|r| r.kind == "repeat" && r.state == "ok")
        else {
            return Ok(None);
        };
        let Some(owner) = group(scene.py(), t, &host.owner) else {
            return Ok(None);
        };
        let positions = metadata(&owner)?
            .get_item(EXT)?
            .and_then(|m| m.get_item("stations").ok());
        let Some(positions) = positions else {
            return Ok(None);
        };
        let m = reference::frame(t.groups[&row.owner]).inverse()
            * reference::frame(t.groups[&host.owner]);
        return Ok(Some(
            positions
                .extract::<Vec<[f64; 3]>>()?
                .into_iter()
                .map(|p| m.transform_point3(p.into()))
                .collect(),
        ));
    }
    let mut path = run.clone();
    if let Some(edge) = run.get("edge") {
        path = json!({"chain":[edge]});
    }
    let Ok(path) = serde_json::from_value::<sweep::Path>(path) else {
        return Ok(None);
    };
    Ok(reference::sweep_path(scene, t, row, &path)?
        .map(|p| p.into_iter().map(DVec3::from).collect()))
}

fn freeze_children(copy: &Obj<'_>, source: &Obj<'_>, parent: &str) -> PyResult<()> {
    for (c, s) in copy
        .getattr("children")?
        .try_iter()?
        .zip(source.getattr("children")?.try_iter()?)
    {
        let (c, s) = (c?, s?);
        let uid = eid::derived_uid(&format!(
            "built:repeat:v1:{parent}:{}",
            s.getattr("uid")?.extract::<String>()?
        ));
        c.setattr("uid", &uid)?;
        let ext = metadata(&c)?;
        if ext.contains(EXT)? {
            ext.del_item(EXT)?;
            c.setattr("ext", ext)?;
        }
        freeze_children(&c, &s, &uid)?;
    }
    Ok(())
}

fn vector(v: &Value) -> Option<DVec3> {
    serde_json::from_value::<[f64; 3]>(v.clone())
        .ok()
        .map(DVec3::from)
}

/// A plane's tangential origin does not change the cut. Key the resolved
/// station in definition space before allocating or rebuilding its mesh.
fn planar_part<'p>(
    scene: &Obj<'p>,
    table: &LiveTable,
    recipe: &Row,
    spec: &sweep::Sweep,
    cache: &mut HashMap<Vec<i64>, Obj<'p>>,
) -> PyResult<Result<Obj<'p>, &'static str>> {
    let Some(path) = reference::sweep_path(scene, table, recipe, &spec.path)? else {
        return Ok(Err("end_unresolved"));
    };
    let planes = match cut::end_planes(scene, table, recipe, &path, &mut None)? {
        Ok(planes) => planes,
        Err(state) => return Ok(Err(state)),
    };
    let micron = |v: f64| (v * 1e6).round() as i64;
    let mut key: Vec<_> = path.iter().flatten().copied().map(micron).collect();
    for plane in &planes {
        key.extend(plane.normal.to_array().map(micron));
        key.push(micron(plane.normal.dot(plane.origin)));
    }
    if let Some(mesh) = cache.get(&key) {
        return Ok(Ok(mesh.clone()));
    }
    let mesh = scene.py().import("built_core.mesh")?.call_method0("Mesh")?;
    let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
    if let Err(state) = sweep::generate(&mut store.try_borrow_mut()?.inner, spec, &path, &planes) {
        return Ok(Err(state));
    }
    cache.insert(key, mesh.clone());
    Ok(Ok(mesh))
}

pub(super) fn evaluate(scene: &Obj<'_>, t: &LiveTable, row: &Row) -> PyResult<&'static str> {
    let py = scene.py();
    let Some(source) = row.source.as_deref().and_then(|s| group(py, t, s)) else {
        return Ok("ref_lost");
    };
    let Some(owner) = group(py, t, &row.owner) else {
        return Ok("ref_lost");
    };
    let source_uid = row.source.as_ref().unwrap();
    let base =
        reference::frame(t.groups[&row.owner]).inverse() * reference::frame(t.groups[source_uid]);
    let polar = row.repeat.run.get("polar");
    let mut path = Vec::new();
    let mut axis = DVec3::Z;
    let mut centre = DVec3::ZERO;
    let mut sign = 1.;
    let span = if let Some(p) = polar {
        let (Some(c), Some(a), Some(deg)) = (
            vector(&p["centre"]),
            vector(&p["axis"]).and_then(DVec3::try_normalize),
            p["angle"].as_f64(),
        ) else {
            return Ok("invalid_run");
        };
        centre = c;
        axis = a;
        sign = deg.signum();
        deg.abs()
    } else {
        let Some(p) = points(scene, t, row)? else {
            return Ok("ref_lost");
        };
        path = p;
        if path.len() < 2 || path.windows(2).any(|p| p[0].distance(p[1]) < 1e-12) {
            return Ok("invalid_run");
        }
        path.windows(2).map(|v| v[0].distance(v[1])).sum()
    };
    let unit = if polar.is_some() { 1. } else { 0.001 };
    let start = row.repeat.ends["start"].as_f64().unwrap_or(0.) * unit;
    let end = row.repeat.ends["end"].as_f64().unwrap_or(0.) * unit;
    let mut rule = row.repeat.rule.clone();
    if row.repeat.measure.as_deref() == Some("gap") && polar.is_none() {
        let dir = (path[1] - path[0]).normalize();
        let store = source
            .getattr("mesh")?
            .getattr("_store")?
            .cast_into::<PyMeshStore>()?;
        let store = store.try_borrow()?;
        let s = &store.inner;
        let (lo, hi) = s
            .live_slots(Kind::Vertex)
            .into_iter()
            .map(|v| {
                let p = s
                    .state
                    .frame
                    .definition_point(*s.state.vpos.get(v as usize));
                base.transform_point3(p.into()).dot(dir)
            })
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                (lo.min(v), hi.max(v))
            });
        if hi >= lo {
            for field in ["step", "spacing", "fit_max"] {
                if let Some(n) = rule[field].as_f64() {
                    rule[field] = json!(n + (hi - lo) / unit);
                }
            }
        }
    }
    let Some((mut count, step)) = stations(&rule, span - start - end, polar.is_some()) else {
        return Ok("invalid_rule");
    };
    let between = row.repeat.run.get("between").is_some();
    if between {
        count = path.len() - 1;
    }
    let mut source_recipe = t
        .owners
        .get(source_uid)
        .and_then(|id| t.rows.get(id))
        .filter(|r| r.kind == "sweep")
        .map(|r| r.as_ref().clone());
    if let Some(recipe) = source_recipe.as_mut() {
        let external = recipe
            .references()
            .any(|r| !matches!(r, Reference::Point(_)))
            || !recipe.slots.is_empty()
            || recipe.cuts.iter().any(|id| {
                t.rows.get(id).is_some_and(|r| {
                    r.anchor.is_some()
                        || matches!(
                            r.cutter,
                            Some(
                                cut::Cutter::Body(_)
                                    | cut::Cutter::Repeat(_)
                                    | cut::Cutter::Bisector(_)
                            )
                        )
                })
            });
        if row.repeat.adapt.as_deref() == Some("identical")
            || (!external && row.repeat.adapt.as_deref() != Some("adaptive"))
        {
            source_recipe = None;
        } else {
            let path = &mut recipe.sweep.as_mut().unwrap().path;
            if !path.definition_space {
                let xf = source.getattr("xform")?;
                let inv = if xf.is_none() {
                    DMat4::IDENTITY
                } else {
                    crate::math::mat(&xf)?.inverse()
                };
                for point in &mut path.points {
                    *point = inv.transform_point3((*point).into()).to_array();
                }
                for end in path.ends.iter_mut().flatten() {
                    if let sweep::End::Point(p) = end {
                        *p = inv.transform_point3((*p).into()).to_array();
                    }
                }
                path.definition_space = true;
            }
        }
    }
    let mut host_paths = Vec::new();
    if let Some(host) = row.repeat.run["between"]
        .as_str()
        .and_then(|id| t.rows.get(id))
    {
        let first = host.source.as_ref().and_then(|uid| group(py, t, uid));
        let assembly = group(py, t, &host.owner);
        let (Some(first), Some(assembly)) = (first, assembly) else {
            return Ok("ref_lost");
        };
        for member in std::iter::once(Ok(first)).chain(assembly.getattr("children")?.try_iter()?) {
            host_paths.push(
                scene
                    .call_method1("occurrence_path", (member?,))?
                    .extract::<Vec<String>>()?,
            );
        }
    }
    if between && host_paths.len() != path.len() {
        return Ok("ref_lost");
    }
    let slot_templates = source_recipe
        .as_ref()
        .map(|r| r.slots.clone())
        .unwrap_or_default();
    let mut ordinals = Vec::new();
    let mut offsets = vec![base.w_axis.truncate().to_array()];
    let mut parts = HashMap::new();
    let mut local = t.clone();
    if let Some(recipe) = &source_recipe {
        for cut_id in &recipe.cuts {
            if let Some(cut) = local.rows.get_mut(cut_id) {
                if !recipe.slots.is_empty() {
                    let target = Arc::make_mut(cut);
                    for (name, r) in &recipe.slots {
                        target
                            .slots
                            .entry(name.clone())
                            .or_insert_with(|| r.clone());
                    }
                }
            }
        }
    }
    let copies = pyo3::types::PyList::empty(py);
    let mut failed = false;
    let planar = source_recipe.as_ref().and_then(|recipe| {
        let spec = recipe.sweep.as_deref()?;
        let plane_cuts = recipe.cuts.iter().all(|id| {
            t.rows.get(id).is_some_and(|cut| {
                cut.on == Some(false)
                    || matches!(
                        cut.cutter,
                        Some(cut::Cutter::Plane(_) | cut::Cutter::Bisector(_))
                    )
            })
        });
        (spec.profile.sketch.is_none() && recipe.inputs.is_empty() && plane_cuts)
            .then(|| spec.clone())
    });
    let notch_tool = source_recipe.as_ref().and_then(|recipe| {
        let mut bodies = recipe
            .cuts
            .iter()
            .filter_map(|id| t.rows.get(id))
            .filter(|cut| {
                cut.on != Some(false)
                    && !matches!(
                        cut.cutter,
                        Some(cut::Cutter::Plane(_) | cut::Cutter::Bisector(_))
                    )
            });
        let cut = bodies.next()?;
        if bodies.next().is_some()
            || cut.anchor.is_some()
            || cut.clearance.unwrap_or(0.) != 0.
            || !matches!(cut.operation.as_deref(), None | Some("trim" | "subtract"))
            || !recipe.inputs.is_empty()
            || recipe.sweep.as_ref()?.profile.sketch.is_some()
        {
            return None;
        }
        let cut::Cutter::Body(uid) = cut.cutter.as_ref()? else {
            return None;
        };
        group(py, t, uid)
    });
    let mut notches = notch::Cache::default();
    let mut planar_cache = HashMap::new();
    let mut canonical = HashMap::new();
    let copier = py.import("built_core.group")?.getattr("copy_group")?;
    let mut old = HashMap::new();
    for child in owner.getattr("children")?.try_iter()? {
        let child = child?;
        old.insert(child.getattr("uid")?.extract::<String>()?, child);
    }
    let mut skipped = HashSet::new();
    for skip in &row.repeat.skips {
        if let Some(at) = skip["at"].as_f64() {
            let d = at * if polar.is_some() { 1. } else { 0.001 };
            let station = if skip["from"] == "end" {
                count as f64 * step - d
            } else {
                d
            };
            skipped.insert(((station - start) / step).round() as i64);
        }
    }
    let nested_source = t
        .owners
        .get(source_uid)
        .and_then(|id| t.rows.get(id))
        .filter(|r| r.kind == "repeat")
        .and_then(|r| r.source.as_ref())
        .and_then(|uid| group(py, t, uid));
    let leaf = source.getattr("children")?.is_empty()? && nested_source.is_none();
    let source_mesh = source.getattr("mesh")?;
    let first = if start != 0. && row.repeat.ends["at_start"] == true && !between {
        0
    } else {
        1
    };
    for i in first..=count {
        if skipped.contains(&(i as i64)) {
            continue;
        }
        let distance = start + i as f64 * step;
        if row.repeat.ends["at_end"] == false && (distance + end - span).abs() < 1e-9 {
            continue;
        }
        let transform = if polar.is_some() {
            let rot = DMat4::from_axis_angle(axis, (distance * sign).to_radians());
            let turn = DMat4::from_translation(centre) * rot * DMat4::from_translation(-centre);
            if row.repeat.orientation.as_deref() == Some("fixed") {
                DMat4::from_translation(
                    turn.transform_point3(base.w_axis.truncate()) - base.w_axis.truncate(),
                )
            } else {
                turn
            }
        } else if between {
            DMat4::from_translation((path[i - 1] + path[i]) * 0.5 - path[0])
        } else {
            let mut d = distance;
            let mut pos = path[0];
            let mut direction = path[1] - path[0];
            for (idx, p) in path.windows(2).enumerate() {
                let len = p[0].distance(p[1]);
                if d <= len || idx + 2 == path.len() {
                    direction = p[1] - p[0];
                    pos = p[0] + (p[1] - p[0]) * (d / len);
                    break;
                }
                d -= len;
            }
            let translation = DMat4::from_translation(pos - path[0]);
            if row.repeat.orientation.as_deref() == Some("follow") {
                let turn = glam::DQuat::from_rotation_arc(
                    (path[1] - path[0]).normalize(),
                    direction.normalize(),
                );
                translation
                    * DMat4::from_translation(base.w_axis.truncate())
                    * DMat4::from_quat(turn)
                    * DMat4::from_translation(-base.w_axis.truncate())
            } else {
                translation
            }
        };
        let mut pose = transform * base;
        if row.repeat.orientation.as_deref() == Some("alternate") && i % 2 == 1 {
            pose *= DMat4::from_scale(DVec3::new(-1., 1., 1.));
        }
        offsets.push(pose.w_axis.truncate().to_array());
        let uid = eid::derived_uid(&format!("built:repeat:v1:{}:{i}", row.uid));
        let copy = if let Some(existing) = old.get(&uid).filter(|_| leaf) {
            if source_recipe.is_none() {
                existing.setattr("mesh", &source_mesh)?;
            }
            existing.clone()
        } else {
            let copy = copier.call1((&source, py.None(), true, py.None(), true))?;
            copy.setattr("uid", &uid)?;
            if !leaf {
                freeze_children(&copy, &source, &uid)?;
                // The first member of a Repeat is its original, a sibling of
                // the assembly. A nested instance needs that member as well.
                if let Some(first) = &nested_source {
                    let child = copier.call1((first, py.None(), true, py.None(), true))?;
                    let id = eid::derived_uid(&format!("built:repeat:v1:{uid}:source"));
                    child.setattr("uid", &id)?;
                    let ext = metadata(&child)?;
                    if ext.contains(EXT)? {
                        ext.del_item(EXT)?;
                    }
                    child.setattr("ext", ext)?;
                    child.setattr(
                        "xform",
                        Matrix4::from(
                            reference::frame(t.groups[source_uid]).inverse()
                                * reference::frame(first.getattr("_doc_key")?.extract()?),
                        ),
                    )?;
                    freeze_children(&child, first, &id)?;
                    copy.getattr("children")?.call_method1("append", (child,))?;
                }
            }
            copy
        };
        copy.setattr("xform", Matrix4::from(pose))?;
        let ext = metadata(&copy)?;
        ext.set_item(
            "built.repeat",
            to_py(
                py,
                &json!({"row":row.uid,"at":distance*if polar.is_some(){1.}else{1000.}}),
            )?,
        )?;
        copy.setattr("ext", ext)?;
        copies.append(&copy)?;
        ordinals.push(i);
    }
    if !owner.getattr("children")?.eq(&copies)? {
        owner.setattr("children", &copies)?;
    }
    for (ordinal, copy) in ordinals.into_iter().zip(copies.try_iter()?) {
        let copy = copy?;
        let ext = metadata(&copy)?;
        if ext.contains(EXT)? {
            ext.del_item(EXT)?;
        }
        if let Some(recipe) = &mut source_recipe {
            if between {
                for (name, path) in [
                    ("start_host", &host_paths[ordinal - 1]),
                    ("end_host", &host_paths[ordinal]),
                ] {
                    let bound = match slot_templates.get(name) {
                        Some(Reference::Element { id, .. }) => Reference::Element {
                            path: path.clone(),
                            id: id.clone(),
                        },
                        _ => Reference::Placement {
                            placement: path.last().cloned().unwrap_or_default(),
                        },
                    };
                    recipe.slots.insert(name.into(), bound.clone());
                    for id in &recipe.cuts {
                        if let Some(r) = local.rows.get_mut(id) {
                            Arc::make_mut(r).slots.insert(name.into(), bound.clone());
                        }
                    }
                }
            }
            let k = copy.getattr("_doc_key")?.extract()?;
            Arc::make_mut(&mut local.groups).insert(source_uid.clone(), k);
            let result = if let Some(spec) = &planar {
                planar_part(scene, &local, recipe, spec, &mut planar_cache)?
            } else if let Some(tool) = &notch_tool {
                match planar_part(
                    scene,
                    &local,
                    recipe,
                    recipe.sweep.as_ref().unwrap(),
                    &mut planar_cache,
                )? {
                    Ok(stock) => notches.part(scene, &local, recipe, &stock, tool)?,
                    Err(state) => Err(state),
                }
            } else {
                let state = super::evaluate(scene, &local, recipe, &mut None, &mut Vec::new())?;
                if state == "ok" {
                    Ok(copy.getattr("mesh")?)
                } else {
                    Err(state)
                }
            };
            match result {
                Ok(mesh) => {
                    let shared = if let Some(shared) = canonical.get(&(mesh.as_ptr() as usize)) {
                        Obj::clone(shared)
                    } else {
                        let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
                        let key = geometry::part_key(&store.try_borrow()?.inner);
                        let shared = parts.entry(key.clone()).or_insert_with(|| mesh.clone());
                        if mesh.is(&*shared) {
                            eid::set_definition_uid(
                                &mesh,
                                &eid::derived_uid(&format!("built:repeat:part:{}:{key}", row.uid)),
                            )?;
                        }
                        if planar.is_some() {
                            canonical.insert(mesh.as_ptr() as usize, shared.clone());
                        }
                        shared.clone()
                    };
                    copy.setattr("mesh", shared)?;
                    if ext.contains("built.repeat_state")? {
                        ext.del_item("built.repeat_state")?;
                    }
                }
                Err(state) => {
                    failed = true;
                    if let Some(previous) = old.get(&copy.getattr("uid")?.extract::<String>()?) {
                        copy.setattr("mesh", previous.getattr("mesh")?)?;
                    }
                    ext.set_item("built.repeat_state", state)?;
                }
            }
        }
        if copy.getattr("ifc")?.is_none() {
            copy.setattr("ifc",to_py(py,&json!({"class":"IfcBuildingElementProxy","name":source.getattr("name")?.extract::<String>()?}))?)?;
        }
        copy.setattr("ext", ext)?;
    }
    let distinct = if source_recipe.is_some() {
        parts.len()
    } else {
        usize::from(!copies.is_empty())
    };
    let ext = metadata(&owner)?;
    let stamp = stamp(&owner)?;
    ext.set_item(EXT,to_py(py,&json!({"row":row.uid,"stamp":stamp,"copies":copies.len(),"distinct":distinct,"stations":offsets}))?)?;
    owner.setattr("ext", ext)?;
    Ok(if failed { "end_unresolved" } else { "ok" })
}

/// The assembly itself is empty. Its stamp covers the instances and their
/// transforms so a reader that edits a baked station makes the row stale.
pub(super) fn stamp(owner: &Obj<'_>) -> PyResult<String> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    let mut seen = HashMap::new();
    let mut pending = vec![owner.clone()];
    while let Some(g) = pending.pop() {
        hash.update(g.getattr("uid")?.extract::<String>()?.as_bytes());
        let xf = g.getattr("xform")?;
        if !xf.is_none() {
            for n in crate::math::mat(&xf)?.to_cols_array() {
                hash.update(n.to_le_bytes());
            }
        }
        let mesh = g.getattr("mesh")?;
        let digest = match seen.entry(mesh.as_ptr() as usize) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
                let digest = geometry::stamp(&store.try_borrow()?.inner);
                e.insert(digest)
            }
        };
        hash.update(digest.as_bytes());
        let kids = g.getattr("children")?;
        hash.update((kids.len()? as u64).to_le_bytes());
        pending.extend(kids.try_iter()?.collect::<PyResult<Vec<_>>>()?);
    }
    Ok(format!(
        "{:016x}",
        u64::from_be_bytes(hash.finalize()[..8].try_into().unwrap())
    ))
}
