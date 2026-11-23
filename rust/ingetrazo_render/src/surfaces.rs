//! View models for the shared sheet. Host snapshots carry document measurements,
//! diagnostics and Python plugin state; the renderer owns the presentation.
use pyo3::{exceptions::PyValueError, prelude::*};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn button(id: &str, label: &str) -> Value {
    json!({"id":id,"label":label})
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

#[pyfunction]
pub fn surface_spec(id: &str, snapshot: &str) -> PyResult<String> {
    let d: Value =
        serde_json::from_str(snapshot).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let spec = match id {
        "about" => crate::about::sheet(&d, false, |s| Ok(s.to_owned()))?,
        "cutlist" => cutlist(&d)?,
        _ => return Err(PyValueError::new_err("Unknown sheet")),
    };
    Ok(spec.to_string())
}
