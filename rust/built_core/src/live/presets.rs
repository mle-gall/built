// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::LazyLock};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preset {
    pub uid: String,
    pub name: String,
    pub pack: String,
    pub primitive: String,
    pub values: BTreeMap<String, Value>,
    pub record: BTreeMap<String, Value>,
    #[serde(default)]
    pub naming: String,
    #[serde(default)]
    pub node: String,
    pub locks: BTreeMap<String, String>,
    pub rev: u64,
}
#[derive(Deserialize)]
struct Pack {
    rows: Vec<Preset>,
}
static BUILTINS: LazyLock<Pack> = LazyLock::new(|| {
    serde_json::from_str(include_str!(
        "../../../../resources/presets/starter.builtpreset"
    ))
    .expect("bundled presets")
});
fn error(msg: impl ToString) -> PyErr {
    PyValueError::new_err(msg.to_string())
}
impl Preset {
    pub(super) fn validate(&self) -> PyResult<()> {
        if self.uid.len() != 16
            || !self.uid.bytes().all(|c| c.is_ascii_hexdigit())
            || self.name.trim().is_empty()
            || !matches!(self.primitive.as_str(), "sweep" | "cut" | "repeat")
            || self.locks.values().any(|v| v != "free" && v != "fixed")
        {
            return Err(error("Invalid preset"));
        }
        if self.primitive == "sweep" {
            if self
                .record
                .get("class")
                .and_then(Value::as_str)
                .is_none_or(|class| !class.starts_with("Ifc"))
            {
                return Err(error("Choose an IFC class"));
            }
            let probe = self.apply_values(json!({"path":{"points":[]}}), false)?;
            let section = probe.profile.outline(probe.segments).map_err(error)?;
            section.attachment(&probe.attachment).map_err(error)?;
        }
        Ok(())
    }
    pub(super) fn apply_values(&self, mut spec: Value, fixed_only: bool) -> PyResult<sweep::Sweep> {
        for (key, val) in &self.values {
            if !matches!(
                key.as_str(),
                "profile" | "attachment" | "rotation" | "mirror" | "segments" | "corner"
            ) {
                return Err(error("Unknown sweep preset field"));
            }
            if !fixed_only || self.locks.get(key).is_some_and(|lock| lock == "fixed") {
                spec[key] = val.clone();
            }
        }
        spec["preset"] = json!(self.name);
        spec["preset_uid"] = json!(self.uid);
        spec["preset_rev"] = json!(self.rev);
        serde_json::from_value(spec).map_err(error)
    }
    pub fn record(&self, g: &Obj<'_>, fixed_only: bool) -> PyResult<()> {
        let old = g.getattr("ifc")?;
        let record = if old.is_none() {
            PyDict::new(g.py())
        } else {
            old.cast::<PyDict>()?.copy()?
        };
        for (key, val) in &self.record {
            if !fixed_only || self.locks.get(key).is_some_and(|lock| lock == "fixed") {
                record.set_item(key, to_py(g.py(), val)?)?;
            }
        }
        g.setattr("ifc", record)
    }
}
pub fn lookup(t: &LiveTable, uid: &str) -> Option<Preset> {
    t.presets
        .get(uid)
        .cloned()
        .or_else(|| BUILTINS.rows.iter().find(|p| p.uid == uid).cloned())
}
pub fn link(t: &mut LiveTable, spec: &mut sweep::Sweep) -> PyResult<Option<Preset>> {
    let Some(uid) = &spec.preset_uid else {
        return Ok(None);
    };
    let p = match lookup(t, uid) {
        Some(p) => p,
        None => spec
            .preset_definition
            .take()
            .filter(|p| &p.uid == uid)
            .ok_or_else(|| error("Missing preset"))?,
    };
    p.validate()?;
    spec.preset_definition = None;
    if p.primitive != "sweep" {
        return Err(error("Expected a Sweep preset"));
    }
    spec.preset = Some(p.name.clone());
    spec.preset_rev = p.rev;
    Arc::make_mut(&mut t.presets).insert(uid.clone(), p.clone());
    Ok(Some(p))
}
pub fn detach(old: &sweep::Sweep, new: &mut sweep::Sweep, p: &Preset) -> Option<String> {
    if new.preset_uid != old.preset_uid {
        return None;
    }
    let (a, b) = (json!(old), json!(new));
    let field = p
        .values
        .keys()
        .find(|k| p.locks.get(*k).is_some_and(|v| v == "fixed") && a[*k] != b[*k])?;
    new.preset_uid = None;
    new.preset = None;
    new.preset_rev = 0;
    Some(format!("Detached from {}: {}", p.name, field))
}
pub fn update(scene: &Obj<'_>, t: &mut LiveTable, presets: &[Preset]) -> PyResult<()> {
    let mut seen = HashSet::new();
    for p in presets {
        p.validate()?;
        if !seen.insert(&p.uid) {
            return Err(error("Duplicate preset ID"));
        }
        let mut p = p.clone();
        p.rev = lookup(t, &p.uid).map_or(1, |v| v.rev.saturating_add(1));
        let followers: Vec<_> = t
            .rows
            .values()
            .filter(|r| {
                r.sweep
                    .as_ref()
                    .is_some_and(|s| s.preset_uid.as_ref() == Some(&p.uid))
            })
            .cloned()
            .collect();
        let mut shared_profile = None;
        for row in followers {
            if p.primitive != "sweep" || row.state == "stale" {
                return Err(error("Preset has an incompatible or stale follower"));
            }
            let mut row = (*row).clone();
            let mut sweep = p.apply_values(json!(row.sweep), true)?;
            if p.locks.get("profile").is_some_and(|lock| lock == "fixed")
                && sweep.profile.drawing.is_some()
            {
                if let Some(profile) = &shared_profile {
                    sweep.profile = profile::Profile::clone(profile);
                } else {
                    bind_drawing(t, &mut sweep.profile)?;
                    shared_profile = Some(sweep.profile.clone());
                }
            }
            row.sweep = Some(Box::new(sweep));
            if let Some(g) = group(scene.py(), t, &row.owner) {
                p.record(&g, true)?;
            }
            t.pending.insert(row.uid.clone());
            t.insert(row).map_err(error)?;
        }
        Arc::make_mut(&mut t.presets).insert(p.uid.clone(), p);
    }
    Ok(())
}
#[pyfunction]
#[pyo3(signature=(scene=None))]
fn preset_catalog<'p>(py: Python<'p>, scene: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    let table = scene
        .map(root)
        .transpose()?
        .and_then(|k| current(py, k))
        .unwrap_or_default();
    let mut rows = Vec::new();
    for p in &BUILTINS.rows {
        if !table.presets.contains_key(&p.uid) {
            rows.push((p, "bundled"));
        }
    }
    rows.extend(table.presets.values().map(|p| (p, "model")));
    to_py(py, &rows)
}
#[pyfunction]
fn preset_update<'p>(history: &Obj<'p>, rows: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    edit(history, Edit::Presets(json(rows)?))
}
#[pyfunction]
fn preset_delete<'p>(history: &Obj<'p>, uid: String) -> PyResult<Bound<'p, PyDict>> {
    edit(history, Edit::DeletePreset(uid))
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(preset_catalog, m)?)?;
    m.add_function(wrap_pyfunction!(preset_update, m)?)?;
    m.add_function(wrap_pyfunction!(preset_delete, m)?)
}

pub fn starter() -> Value {
    json!(BUILTINS.rows[0])
}
