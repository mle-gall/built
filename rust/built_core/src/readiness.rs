// SPDX-License-Identifier: GPL-3.0-or-later
//! Join the retained record/IDS/solid check results to the selected delivery.
//! Count distinct placements rather than checks so two defects never double-count a wall.
use crate::project_tree::{row, Row};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

type Failures = BTreeMap<String, (String, Vec<Value>)>;

#[pyfunction]
pub(crate) fn export_readiness(
    scene: &Bound<'_, PyAny>,
    tree: &Bound<'_, PyAny>,
    records: &Bound<'_, PyDict>,
    preferred: &str,
    rules: &Bound<'_, PyAny>,
    expanded: HashSet<String>,
    rows: Vec<Row>,
) -> PyResult<(Vec<Row>, String)> {
    let deliveries: Vec<(String, String)> = records
        .iter()
        .map(|(k, v)| Ok((k.extract()?, v.get_item("name")?.extract()?)))
        .collect::<PyResult<_>>()?;
    let active = if records.contains(preferred)? {
        preferred
    } else {
        deliveries.first().map_or("", |r| r.0.as_str())
    };
    let delivery = records.get_item(active)?;
    let scope: String = delivery
        .as_ref()
        .and_then(|v| v.get_item("scope").ok()?.extract().ok())
        .unwrap_or_else(|| "whole".into());
    let format: Option<String> = delivery
        .as_ref()
        .and_then(|r| r.get_item("format").ok()?.extract().ok());
    let by_node = tree.getattr("nodes")?;
    let membership = tree.getattr("member")?;
    let mut eligible = HashMap::new();
    let mut notes = Vec::<String>::new();
    let mut candidates = Vec::new();
    match scope.as_str() {
        "node" => {
            let key = delivery.as_ref().unwrap().get_item("node")?;
            if by_node.contains(&key)? {
                candidates = tree.call_method1("members", (key,))?.extract()?;
            } else {
                notes.push("Delivery node is missing".into());
            }
        }
        "selection" => {
            for obj in scene.getattr("selection")?.try_iter()? {
                let obj = obj?;
                if obj.hasattr("children")? {
                    candidates.push(obj);
                } else {
                    notes.push("Loose geometry has no placement record".into());
                }
            }
        }
        "visible" | "whole" => {
            for branch in ["direct", "links"] {
                for items in tree.getattr(branch)?.call_method0("values")?.try_iter()? {
                    candidates.extend(items?.extract::<Vec<Bound<'_, PyAny>>>()?);
                }
            }
            candidates.extend(
                tree.getattr("unfiled")?
                    .extract::<Vec<Bound<'_, PyAny>>>()?,
            );
        }
        _ => notes.push(format!("Placement checks do not cover {scope}")),
    }
    let config = if format.as_deref() == Some("ifc") {
        let profiles = scene
            .py()
            .import("core.bim_registry")?
            .call_method1("get", (scene,))?
            .call_method1("get", ("export_profiles", PyDict::new(scene.py())))?;
        let profile = delivery
            .as_ref()
            .and_then(|d| d.get_item("options").ok())
            .and_then(|options| options.call_method1("get", ("profile",)).ok())
            .and_then(|key| key.extract::<String>().ok())
            .unwrap_or_default();
        if !profile.is_empty() && !profiles.contains(&profile)? {
            notes.push(format!("Missing export profile: {profile}"));
        }
        profiles.call_method1("get", (profile, PyDict::new(scene.py())))?
    } else {
        PyDict::new(scene.py()).into_any()
    };
    let predicate = config.call_method1("get", ("filter",))?;
    let allowed = if !predicate.is_none() {
        let facets = scene.py().import("built_core.facets")?;
        let index = facets.call_method1("build", (scene,))?;
        let groups = index.getattr("groups")?;
        let mut ids = HashSet::new();
        for key in facets
            .call_method1("evaluate", (&index, predicate))?
            .try_iter()?
        {
            ids.insert(groups.get_item(key?)?.as_ptr() as usize);
        }
        Some(ids)
    } else {
        None
    };
    let include_links = format.as_deref() != Some("ifc")
        || config
            .call_method1("get", ("include_links", false))?
            .is_truthy()?;
    for g in candidates {
        let id = g.as_ptr() as usize;
        if allowed.as_ref().is_some_and(|ids| !ids.contains(&id)) {
            continue;
        }
        if !include_links
            && membership
                .call_method1("get", (id,))?
                .extract::<Option<String>>()?
                .is_some_and(|k| k.starts_with("link:"))
        {
            continue;
        }
        if scope != "visible" || scene.call_method1("entity_visible", (&g,))?.is_truthy()? {
            eligible.insert(g.as_ptr() as usize, g);
        }
    }
    let mut direct_checks = BTreeMap::<String, Failures>::new();
    let mut checks = BTreeMap::<String, Failures>::new();
    let mut blockers = HashMap::<String, HashSet<usize>>::new();
    for rule in rules.try_iter()? {
        let r = rule?;
        let severity: String = r.get_item(0)?.extract()?;
        let code: String = r.get_item(1)?.extract()?;
        let label: String = r.get_item(2)?.extract()?;
        if !matches!(format.as_deref(), None | Some("ifc"))
            && !matches!(code.as_str(), "links" | "solid" | "solidcut")
        {
            continue;
        }
        if severity == "Info" {
            if code == "solidcut" || code == "noids" {
                notes.push(label);
            }
            continue;
        }
        let mut affected: Vec<Bound<'_, PyAny>> = r.get_item(3)?.extract()?;
        if code == "solid" && !affected.is_empty() {
            // Geometry checks run once per definition; each scoped instance
            // of that definition counts separately towards delivery readiness.
            let mut meshes = HashSet::new();
            for item in &affected {
                if !item.is_none() {
                    meshes.insert(item.getattr("mesh")?.as_ptr());
                }
            }
            for item in eligible.values() {
                if meshes.contains(&item.getattr("mesh")?.as_ptr()) {
                    affected.push(item.clone());
                }
            }
        }
        let mut seen = HashSet::new();
        for obj in affected {
            if !seen.insert(obj.as_ptr()) {
                continue;
            }
            if obj.is_none() {
                if scope == "whole" || scope == "visible" {
                    checks.entry("*".into()).or_default().insert(
                        "active-solid".into(),
                        (
                            label.clone(),
                            vec![json!({"name":"Loose geometry","path":null,"fix":""})],
                        ),
                    );
                }
                continue;
            }
            let path: Vec<String> = scene.call_method1("occurrence_path", (&obj,))?.extract()?;
            // A nested mesh can invalidate its filed assembly. Count the
            // scoped placement once, but reveal the exact faulty part.
            let mut owner = obj.clone();
            if !eligible.contains_key(&(owner.as_ptr() as usize)) {
                if code != "solid" {
                    continue;
                }
                let mut found = false;
                for end in (1..path.len()).rev() {
                    owner = scene.call_method1("resolve_occurrence", (path[..end].to_vec(),))?;
                    if eligible.contains_key(&(owner.as_ptr() as usize)) {
                        found = true;
                        break;
                    }
                }
                if !found {
                    continue;
                }
            }
            let id = owner.as_ptr() as usize;
            let name: String = obj
                .getattr("name")?
                .extract::<Option<String>>()?
                .unwrap_or_else(|| "Group".into());
            let node: Option<String> = membership.call_method1("get", (id,))?.extract()?;
            let mut parents = vec!["*".to_owned()];
            let mut k = node;
            if k.is_none() {
                let tag = owner
                    .getattr("layer")?
                    .extract::<Option<String>>()?
                    .unwrap_or_else(|| "Layer 0".into());
                parents.push(format!("preview-tag:{tag}"));
                parents.push("n:".into());
            }
            while let Some(key) = k {
                parents.push(format!("n:{key}"));
                k = by_node.get_item(key)?.get_item("parent")?.extract()?;
            }
            let mut fix = String::new();
            if code == "status" && !obj.getattr("locked")?.is_truthy()? {
                let rec = obj.getattr("ifc")?;
                if let Some(class) = rec
                    .call_method1("get", ("class",))?
                    .extract::<Option<String>>()?
                {
                    fix = format!("Pset_{}Common", class.trim_start_matches("Ifc"));
                }
            }
            let rule_key = format!("{code}/{fix}");
            let element = json!({"name":name,"path":path,"fix":fix,"visible":!obj.getattr("hidden")?.is_truthy()?});
            if let Some(key) = parents.get(1) {
                direct_checks
                    .entry(key.clone())
                    .or_default()
                    .entry(rule_key.clone())
                    .or_insert_with(|| (label.clone(), Vec::new()))
                    .1
                    .push(element.clone());
            }
            for key in parents {
                blockers.entry(key.clone()).or_default().insert(id);
                checks
                    .entry(key)
                    .or_default()
                    .entry(rule_key.clone())
                    .or_insert_with(|| (label.clone(), Vec::new()))
                    .1
                    .push(element.clone());
            }
        }
    }
    notes.sort();
    notes.dedup();
    let blocked = blockers.get("*").map_or(0, HashSet::len);
    let total = eligible.len();
    let mut result = Vec::new();
    let mut in_catalog = false;
    for (text, mut flags) in rows {
        let key = text[0].clone();
        if flags[0] == 0 {
            in_catalog = key.starts_with("n:");
        }
        if in_catalog && matches!(flags[4], 1..=4) {
            continue;
        }
        if (flags[4] == 0 && key.starts_with("n:")) || key.starts_with("preview-tag:") {
            flags[2] = blockers.get(&key).map_or(0, HashSet::len) as i64;
            let depth = flags[0] as usize;
            let open = flags[8] != 0 || expanded.contains(&key);
            flags[8] = open as i64;
            if flags[2] > 0 {
                flags[7] = 1;
            }
            result.push((text, flags));
            if open {
                if let Some(rules) = direct_checks.get(&key) {
                    for (id, (label, items)) in rules {
                        let check = format!("check:{}", json!([key, id]));
                        let open = expanded.contains(&check);
                        let mut head = row(
                            check,
                            format!("{label} ({})", items.len()),
                            "",
                            "",
                            depth + 1,
                            9,
                            items.len(),
                            0,
                        );
                        head.1[7] = 1;
                        head.1[8] = open as i64;
                        result.push(head);
                        if open {
                            for item in items {
                                let mut placement = row(
                                    format!("exportitem:{}", item["path"]),
                                    item["name"].as_str().unwrap_or("Group").into(),
                                    "",
                                    "group",
                                    depth + 2,
                                    10,
                                    0,
                                    0,
                                );
                                if let Some(visible) = item["visible"].as_bool() { placement.1[5] = i64::from(visible); }
                                result.push(placement);
                            }
                        }
                    }
                }
            }
        } else {
            result.push((text, flags));
        }
    }
    let report = json!({"choices":deliveries,"active":active,"ready":total-blocked,"total":total,"checks":checks,"notes":notes});
    Ok((result, report.to_string()))
}
