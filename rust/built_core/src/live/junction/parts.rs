// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

/// Construct the ordinary placement or sweep that the template item owns.
pub(super) fn prepare(
    scene: &Obj<'_>,
    t: &mut LiveTable,
    j: &Row,
    key: &str,
    spec: &Value,
    bindings: &mut Vec<Value>,
) -> PyResult<Value> {
    // A captured part keeps its own uids (§5); later uses derive them.
    let (uid, owner) = content_ids(j, key);
    let py = scene.py();
    let mut item = json!({"key":key,"placement":owner,"at":spec.get("at").cloned().unwrap_or_else(||json!({"frame":"@meet"}))});
    for kind in ["fastener", "weld", "glue"] {
        if let Some(feature) = spec.get(kind) {
            if !t.groups.contains_key(&owner) {
                let g = py.import("built_core.group")?.getattr("Group")?.call0()?;
                g.setattr("uid", &owner)?;
                g.setattr("name", key)?;
                let parent =
                    group(py, t, &j.owner).ok_or_else(|| err("Missing junction assembly"))?;
                parent.getattr("children")?.call_method1("append", (&g,))?;
                Arc::make_mut(&mut t.groups).insert(owner, g.getattr("_doc_key")?.extract()?);
            }
            item[kind] = feature.clone();
            return Ok(item);
        }
    }
    if let Some(source) = spec["component"].as_str() {
        let definition = match group(py, t, source) {
            Some(g) => g,
            None => {
                cut::source(scene, source)?.ok_or_else(|| err("Missing component definition"))?
            }
        };
        let assembly = group(py, t, &j.owner).ok_or_else(|| err("Missing junction assembly"))?;
        let copy = py
            .import("built_core.group")?
            .getattr("copy_group")?
            .call1((&definition, py.None(), true, py.None(), true))?;
        copy.setattr("uid", &owner)?;
        repeat::freeze_children(&copy, &definition, &owner)?;
        let children = assembly.getattr("children")?;
        if let Some(old) = group(py, t, &owner) {
            children.call_method1("remove", (old,))?;
        }
        children.call_method1("append", (&copy,))?;
        Arc::make_mut(&mut t.groups).insert(owner, copy.getattr("_doc_key")?.extract()?);
        item["component"] = json!(source);
        return Ok(item);
    }
    // A part without a preset carries its section and record inline.
    let preset = match spec["preset"].as_str() {
        Some(p) => presets::lookup(t, p)
            .filter(|p| p.primitive == "sweep")
            .ok_or_else(|| err("Missing sweep part preset"))?,
        None if spec["sweep"].is_object() => presets::Preset {
            uid: String::new(),
            name: key.into(),
            pack: String::new(),
            primitive: "sweep".into(),
            values: spec["sweep"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(k, _)| matches!(k.as_str(), "profile" | "segments" | "attachment"))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            record: spec["record"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            naming: String::new(),
            node: String::new(),
            locks: BTreeMap::new(),
        source: None,
            rev: 0,
        },
        None => return Err(err("Missing sweep part preset")),
    };
    let path = spec
        .get("path")
        .cloned()
        .unwrap_or_else(|| json!({"points":[[0,0,0],[0,0,0.01]]}));
    let mut sweep = json!(preset.apply_values(json!({"path":path}), false)?);
    sweep["path"]["definition_space"] = json!(true);
    if preset.uid.is_empty() {
        for k in ["preset", "preset_uid", "preset_rev"] {
            sweep.as_object_mut().map(|o| o.remove(k));
        }
    }
    // A captured part keeps its section's turn, mirror and attachment.
    for k in ["rotation", "mirror", "attachment"] {
        if let Some(v) = spec.get(k) {
            sweep[k] = v.clone();
        }
    }
    for (name, value) in spec["fields"].as_object().into_iter().flatten() {
        let path = if name == "thickness"
            || (name == "profile.params.flange"
                && preset.record.get("class") == Some(&json!("IfcPlate")))
        {
            "/path/points/1/2".to_owned()
        } else if name == "path.x" || name == "path.y" {
            format!("/path/points/1/{}", usize::from(name == "path.y"))
        } else if let Some(field) = name.strip_prefix("profile.params.") {
            if field.is_empty() || field.contains('/') {
                return Err(err("Invalid part dimension"));
            }
            sweep["profile"]["params"][field] = json!(0.);
            format!("/profile/params/{field}")
        } else {
            return Err(err("Unknown part field"));
        };
        let dest = sweep
            .pointer_mut(&path)
            .ok_or_else(|| err("Missing part field"))?;
        let scale = if path.starts_with("/path/points/1/") {
            0.001
        } else {
            1.
        };
        if let Some(n) = value.as_f64() {
            *dest = json!(n * scale);
        } else {
            bindings.push(json!({"row":uid,"field":path,"param":value,"scale":scale}));
        }
    }
    sweep["kind"] = json!("sweep");
    sweep["uid"] = json!(uid);
    sweep["owner"] = json!(owner);
    sweep["junction"] = json!(j.uid);
    sweep["key"] = json!(key);
    let mut row: Row = serde_json::from_value(sweep).map_err(err)?;
    if let Some(old) = t.rows.get(&uid) {
        row.cuts = old.cuts.clone();
    }
    t.insert(j.clone()).map_err(err)?;
    let new_part = !t.groups.contains_key(&owner);
    place_owned_sweep(scene, t, &row)?;
    let part = group(py, t, &owner).ok_or_else(|| err("Missing part placement"))?;
    preset.record(&part, false)?;
    if new_part {
        crate::structure::file_sweep(scene, &part, &preset.node)?;
    }
    if !preset.uid.is_empty() {
        Arc::make_mut(&mut t.presets).insert(preset.uid.clone(), preset);
    }
    t.insert(row).map_err(err)?;
    t.pending.insert(uid.clone());
    item["row"] = json!(uid);
    Ok(item)
}

pub(super) fn place(
    scene: &Obj<'_>,
    t: &LiveTable,
    j: &Row,
    params: &mut expr::Values,
    dims: &expr::Values,
) -> PyResult<bool> {
    let mut changed = false;
    for item in j.joint.contents.as_array().into_iter().flatten() {
        let Some(at) = item.get("at") else { continue };
        let frame = at["frame"]
            .as_str()
            .and_then(|s| s.strip_prefix('@'))
            .and_then(|s| j.frames.get(s))
            .ok_or_else(|| err("Missing part frame"))?;
        let mut offset = [0.; 3];
        if let Some(values) = at.get("offset") {
            let values = values
                .as_array()
                .filter(|a| a.len() == 3)
                .ok_or_else(|| err("Expected a three-axis part offset"))?;
            for (dst, v) in offset.iter_mut().zip(values) {
                *dst = expr::field(v, params, dims).map_err(err)? / 1000.;
            }
        }
        // A part's own Z (a plate's thickness, a fastener's shank) follows
        // the frame's Z unless `axis` names another frame axis.
        let turn = match at["axis"].as_str().unwrap_or("z") {
            "z" => DMat4::IDENTITY,
            "-z" => DMat4::from_rotation_x(std::f64::consts::PI),
            "x" => DMat4::from_rotation_y(std::f64::consts::FRAC_PI_2),
            "-x" => DMat4::from_rotation_y(-std::f64::consts::FRAC_PI_2),
            "y" => DMat4::from_rotation_x(-std::f64::consts::FRAC_PI_2),
            "-y" => DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2),
            _ => return Err(err("A part axis is x, y or z, optionally negated")),
        };
        let mut m =
            DMat4::from_cols_array(frame) * DMat4::from_translation(offset.into()) * turn;
        // A captured part's own axes in the frame (column-major 3 × 3).
        if let Some(b) = at.get("basis").and_then(Value::as_array).filter(|b| b.len() == 9) {
            let v: Vec<f64> = b.iter().map(|x| x.as_f64().unwrap_or(0.)).collect();
            m *= DMat4::from_mat3(glam::DMat3::from_cols_slice(&v));
        }
        let g = item["placement"]
            .as_str()
            .and_then(|s| group(scene.py(), t, s))
            .ok_or_else(|| err("Missing part placement"))?;
        if let Some(spec) = item.get("fastener") {
            fastener::update(scene, &g, spec, params, dims)?;
        }
        for kind in ["weld", "glue"] {
            if let Some(spec) = item.get(kind) {
                let glue = kind == "glue";
                let field = if glue { "area" } else { "length" };
                if j.joint.overrides.get(field).is_some() {
                    return Err(err(format!("{field} is derived from the contact path")));
                }
                let measurement = seam::update(scene, t, j, &g, spec, glue, params, dims)?;
                params.insert(field.into(), measurement);
                m = DMat4::IDENTITY;
            }
        }
        let old = g.getattr("xform")?;
        if old.is_none() || crate::math::mat(&old)? != m {
            g.setattr("xform", crate::math::Matrix4::from(m))?;
            changed = true;
        }
    }
    Ok(changed)
}
