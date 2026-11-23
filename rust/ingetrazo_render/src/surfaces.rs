//! View models for the shared sheet. Host snapshots carry document measurements,
//! diagnostics and Python plugin state; the renderer owns the presentation.
use pyo3::{exceptions::PyValueError, prelude::*};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn button(id: &str, label: &str) -> Value {
    json!({"id":id,"label":label})
}
fn string(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}
fn about(data: &Value) -> Value {
    let mut blocks = vec![
        json!({"kind":"text","text":"Built is based on IngeTrazo and distributed under GPL-3.0-or-later."}),
        json!({"kind":"buttons","align":"left","buttons":[button("license","Read licence ↗"),button("copy","Copy system info")]}),
        json!({"kind":"caption","text":"System information"}),
        json!({"kind":"kv","rows":data["facts"]}),
        json!({"kind":"separator"}),
        json!({"kind":"caption","text":"Credits"}),
        json!({"kind":"buttons","align":"left","buttons":[button("upstream","IngeTrazo ↗")]}),
    ];
    for paragraph in include_str!("../../../AUTHORS").split("\n\n").skip(1) {
        let mut lines = paragraph
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.chars().all(|c| c == '-'));
        if let Some(name) = lines.next() {
            blocks.push(json!({"kind":"caption","text":name.trim()}));
            let description = lines.map(str::trim).collect::<Vec<_>>().join(" ");
            if !description.is_empty() {
                blocks.push(json!({"kind":"text","text":description}));
            }
        }
    }
    json!({"id":"about","title":"About Built","width":900,"content":{"title":"Built","sub":data["build"],"blocks":blocks}})
}

#[derive(Deserialize)]
struct Part {
    uid: String,
    name: String,
    material: String,
    size: [f64; 3],
    dimensions: [String; 3],
}

fn cutlist(d: &Value) -> PyResult<Value> {
    let parts: Vec<Part> = serde_json::from_value(d["parts"].clone())
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let match_material = d["match_material"] == true;
    let mut groups = BTreeMap::new();
    for part in &parts {
        let millimetres = part.size.map(|v| (v * 1000.).round_ties_even() as i64);
        let material = if match_material {
            part.material.as_str()
        } else {
            ""
        };
        groups
            .entry((millimetres, material))
            .or_insert_with(Vec::new)
            .push(part);
    }
    let mut table = vec![
        vec!["Qty", "Part", "Material", "Length", "Width", "Thickness"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    ];
    let mut members = Vec::new();
    for (_, mut group) in groups.into_iter().rev() {
        let first = group[0];
        group.sort_by(|a, b| a.name.cmp(&b.name));
        let names = group.iter().map(|p| p.name.as_str()).collect::<Vec<_>>();
        let materials: std::collections::BTreeSet<_> = group
            .iter()
            .map(|p| p.material.as_str())
            .filter(|m| !m.is_empty())
            .collect();
        let mut row = vec![
            group.len().to_string(),
            names.join(", "),
            materials.into_iter().collect::<Vec<_>>().join(", "),
        ];
        row.extend(first.dimensions.clone());
        if first.size[2] < 0.0005 {
            row[5] = "surface".into();
        }
        table.push(row);
        members.push(group.iter().map(|p| p.uid.as_str()).collect::<Vec<_>>());
    }
    let mut blocks = Vec::new();
    if !parts.is_empty() {
        blocks.push(json!({"kind":"table","cells":table,"right":d["selected"]}));
    }
    Ok(
        json!({"id":"cutlist","title":"Cut list","content":{"title":d["assembly"],
        "sub":format!("{} parts · revision {}", parts.len(), d["revision"]), "blocks":blocks,
        "empty":"Select an assembly containing parts to see its cut list."},
        "members":members,
        "verbs":[button("group", if match_material {"✓ Match material"}else{"Match material"}),
        {"id":"copy","label":"Copy table","enabled":!parts.is_empty()},{"id":"csv","label":"Export CSV…","enabled":!parts.is_empty()}]}),
    )
}

fn origin(p: &Value) -> &'static str {
    if p["included"] == true {
        "Included with Built"
    } else if p["manual"] == true {
        "Manual install"
    } else if p["bundled"] == true {
        "Bundled example"
    } else if p["available"] == true {
        "Catalog"
    } else {
        "Managed install"
    }
}

fn next_launch(p: &Value, safe: bool) -> &'static str {
    if p["installed"] != true {
        "Not installed"
    } else if safe {
        "Safe mode"
    } else if p["enabled"] == true {
        "Enabled"
    } else {
        "Disabled"
    }
}

fn package_detail(p: &Value, safe: bool) -> Value {
    let mut buttons = Vec::new();
    if p["installed"] == true {
        buttons.push(button(
            "toggle",
            if p["enabled"] == true {
                "Disable"
            } else {
                "Enable"
            },
        ));
        if p["update"] == true {
            buttons.push(json!({"id":"install","label":"Update","enabled":p["compatible"]}));
        }
        if p["included"] != true {
            buttons.push(button("remove", "Remove…"));
        }
    } else {
        buttons.push(json!({"id":"install","label":"Install…","enabled":p["compatible"]}));
    }
    let running = if p["failure"] != "" {
        "Load error"
    } else if p["running"] == true {
        "Running"
    } else {
        "Not running"
    };
    let mut blocks = vec![
        json!({"kind":"text","text":p["summary"]}),
        json!({"kind":"kv","rows":[["Source",origin(p),""],["Version",p["version"],""],
        ["This session",running,""],["Next launch",next_launch(p,safe),""]]}),
    ];
    if p["failure"] != "" {
        blocks.push(json!({"kind":"text","text":p["failure"],"tone":"error"}));
    }
    if p["compatible"] == false {
        blocks
            .push(json!({"kind":"text","text":"Requires a newer version of Built","tone":"warn"}));
    }
    if p["source"] != "" {
        blocks.push(json!({"kind":"text","text":p["source"]}));
    }
    blocks.push(json!({"kind":"buttons","align":"fill","buttons":buttons}));
    json!({"title":p["name"],"width":340,"blocks":blocks})
}

fn extensions(d: &Value) -> Value {
    let tab = string(&d["tab"]);
    let query = string(&d["query"]).to_lowercase();
    let rows: Vec<_> = d["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| {
            let matches = query.is_empty()
                || format!("{} {} {}", string(&p["name"]), string(&p["id"]), origin(p))
                    .to_lowercase()
                    .contains(&query);
            matches
                && match tab {
                    "Updates" => p["update"] == true,
                    "Browse" => p["available"] == true,
                    _ => p["installed"] == true,
                }
        })
        .collect();
    let selected = rows.iter().find(|p| p["id"] == d["focus"]).or(rows.first());
    let items: Vec<_> = rows
        .iter()
        .map(|p| json!({"id":p["id"],"label":p["name"],"sub":origin(p),"right":next_launch(p,d["safe"]==true)}))
        .collect();
    let detail = selected.map(|p| package_detail(p, d["safe"] == true));
    let tabs: Vec<_> = ["Installed", "Browse", "Updates"]
        .iter()
        .map(|s| json!({"id":s,"label":s,"on":s==&tab}))
        .collect();
    json!({"id":"extensions","title":"Extension Manager","tabs":tabs,"search":{"placeholder":"Search extensions","key":"⌘F"},
        "verbs":[button("refresh","Refresh catalog")],"force_focus":true,"focus":selected.map(|p|&p["id"]),"detail":detail,
        "content":{"title":tab,"empty":"No matching extensions","blocks":if items.is_empty(){vec![]}else{vec![json!({"kind":"rows","items":items})]}},
        "footer":{"links":[button("safe",if d["safe"]==true {"Leave safe mode"}else{"Safe mode"}),button("reset","Reset overrides…")],"hint":d["notice"]}})
}

#[pyfunction]
pub fn surface_spec(id: &str, snapshot: &str) -> PyResult<String> {
    let d: Value =
        serde_json::from_str(snapshot).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let spec = match id {
        "about" => about(&d),
        "extensions" => extensions(&d),
        "cutlist" => cutlist(&d)?,
        _ => return Err(PyValueError::new_err("Unknown sheet")),
    };
    Ok(spec.to_string())
}
