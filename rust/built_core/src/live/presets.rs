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
    /// Where an imported detail came from: `{"pack", "name", "rev", "hash"}`,
    /// kept with the document so the Library can offer its updates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepeatPreset {
    pub uid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub rev: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<Box<Preset>>,
}

pub(super) fn follower(row: &Row) -> Option<&str> {
    row.sweep
        .as_ref()
        .and_then(|s| s.preset_uid.as_deref())
        .or_else(|| row.joint.detail["uid"].as_str())
        .or_else(|| row.repeat.repeat_preset.as_ref().map(|p| p.uid.as_str()))
}

const REPEAT_FIELDS: &[&str] = &["rule", "ends", "measure", "orientation", "adapt"];

fn apply_repeat(row: &mut Row, p: &Preset, fixed_only: bool) -> PyResult<()> {
    let mut fields = json!(row.repeat);
    for (key, val) in &p.values {
        if !fixed_only || p.locks.get(key).is_some_and(|v| v == "fixed") {
            fields[key] = val.clone();
        }
    }
    row.repeat = serde_json::from_value(fields).map_err(error)?;
    row.repeat.repeat_preset = Some(RepeatPreset {
        uid: p.uid.clone(),
        name: p.name.clone(),
        rev: p.rev,
        definition: None,
    });
    Ok(())
}

pub(super) fn bind_repeat(scene: &Obj<'_>, t: &mut LiveTable, row: &mut Row) -> PyResult<()> {
    let Some(link) = &mut row.repeat.repeat_preset else {
        return Ok(());
    };
    let explicit = link.definition.take();
    let p = lookup(t, &link.uid)
        .or_else(|| explicit.as_deref().cloned())
        .filter(|p| p.uid == link.uid && p.primitive == "repeat")
        .ok_or_else(|| error("Missing Repeat preset"))?;
    let owner = group(scene.py(), t, &row.owner).ok_or_else(|| error("Unknown Repeat owner"))?;
    if explicit.is_some() || t.rows.get(&row.uid).and_then(|r| follower(r)) != Some(p.uid.as_str())
    {
        p.validate()?;
        apply_repeat(row, &p, false)?;
        p.record(&owner, false)?;
        Arc::make_mut(&mut t.presets).insert(p.uid.clone(), p);
    } else {
        let old = json!(t.rows[&row.uid].repeat);
        let new = json!(row.repeat);
        if let Some(key) = p
            .values
            .keys()
            .find(|k| p.locks.get(*k).is_some_and(|v| v == "fixed") && old[*k] != new[*k])
        {
            row.repeat.repeat_preset = None;
            owner
                .getattr("ifc")?
                .set_item("preset_note", format!("Detached from {}: {key}", p.name))?;
        }
    }
    Ok(())
}
#[derive(Deserialize)]
struct Pack {
    rows: Vec<Preset>,
}
/// Built's bundled presets: the starter set and the starter detail packs
/// (each a format 2 pack with its rows inline).
pub const PACK_FILES: &[&str] = &[
    include_str!("../../../../resources/presets/starter.builtpreset"),
    include_str!("../../../../resources/presets/packs/steel-connections-en.builtpreset"),
    include_str!("../../../../resources/presets/packs/timber-joints.builtpreset"),
    include_str!("../../../../resources/presets/packs/purlin-connections-en.builtpreset"),
    include_str!("../../../../resources/presets/packs/mep-penetrations.builtpreset"),
    include_str!("../../../../resources/presets/packs/generic.builtpreset"),
];
#[derive(Deserialize)]
struct PackFile {
    rows: Vec<Value>,
    #[serde(default)]
    pack: Value,
}
static BUILTINS: LazyLock<Pack> = LazyLock::new(|| load_packs(PACK_FILES));
static IMPERIAL: LazyLock<Pack> = LazyLock::new(|| load_packs(&[
    include_str!("../../../../resources/presets/packs/steel-connections-aisc.builtpreset"),
]));
fn load_packs(files: &[&str]) -> Pack {
    let mut rows = Vec::new();
    for text in files {
        let file: PackFile = serde_json::from_str(text).expect("bundled presets");
        for row in file.rows {
            let mut p: Preset = serde_json::from_value(row.clone()).expect("bundled preset");
            // A format 2 pack names where its rows come from, with whom.
            if let Some(id) = file.pack["id"].as_str() {
                p.source = Some(json!({"pack": id, "name": file.pack["name"], "version": file.pack["version"],
                    "rev": p.rev, "hash": row["hash"], "author": file.pack["author"], "licence": file.pack["licence"]}));
            }
            if let Some(pref) = file.pack.get("preference") {
                p.source.as_mut().expect("pack metadata")["preference"] = pref.clone();
            }
            rows.push(p);
        }
    }
    Pack { rows }
}
/// A bundled preset or detail by uid.
pub fn builtin(uid: &str) -> Option<Preset> {
    let pack = if uid.starts_with("a15c") { &*IMPERIAL } else { &*BUILTINS };
    pack.rows.iter().find(|p| p.uid == uid).cloned()
}

fn error(msg: impl ToString) -> PyErr {
    PyValueError::new_err(msg.to_string())
}
impl Preset {
    /// The checks that need no geometry: identity, kind, locks, parameters.
    pub fn check(&self) -> Result<(), String> {
        if self.uid.len() != 16
            || !self.uid.bytes().all(|c| c.is_ascii_hexdigit())
            || self.name.trim().is_empty()
            || !matches!(
                self.primitive.as_str(),
                "sweep" | "cut" | "repeat" | "junction"
            )
            || self.locks.values().any(|v| v != "free" && v != "fixed")
        {
            return Err("Invalid preset".into());
        }
        if self.primitive == "junction" {
            let params = self
                .values
                .get("params")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            expr::validate(params)?;
        }
        if self.primitive == "repeat"
            && self
                .values
                .keys()
                .any(|k| !REPEAT_FIELDS.contains(&k.as_str()))
        {
            return Err("Unknown Repeat preset field".into());
        }
        Ok(())
    }
    pub(super) fn validate(&self) -> PyResult<()> {
        if self.uid.len() != 16
            || !self.uid.bytes().all(|c| c.is_ascii_hexdigit())
            || self.name.trim().is_empty()
            || !matches!(
                self.primitive.as_str(),
                "sweep" | "cut" | "repeat" | "junction"
            )
            || self.locks.values().any(|v| v != "free" && v != "fixed")
        {
            return Err(error("Invalid preset"));
        }
        if self.primitive == "junction" {
            let params = self
                .values
                .get("params")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            expr::validate(params).map_err(error)?;
        }
        if self.primitive == "repeat"
            && self
                .values
                .keys()
                .any(|k| !REPEAT_FIELDS.contains(&k.as_str()))
        {
            return Err(error("Unknown Repeat preset field"));
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
                "profile"
                    | "attachment"
                    | "rotation"
                    | "mirror"
                    | "segments"
                    | "corner"
                    | "cut_definitions"
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
        .or_else(|| builtin(uid))
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
/// Embed presets a step needs that the model does not hold yet. A preset the
/// model already has keeps the model's revision: updating it is its own,
/// explicit step (the Library's Update).
pub fn embed(t: &mut LiveTable, presets: &[Preset]) -> PyResult<()> {
    let mut seen = HashSet::new();
    for p in presets {
        p.validate()?;
        if !seen.insert(&p.uid) {
            return Err(error("Duplicate preset ID"));
        }
    }
    for p in presets {
        if !t.presets.contains_key(&p.uid) {
            Arc::make_mut(&mut t.presets).insert(p.uid.clone(), p.clone());
        }
    }
    Ok(())
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
            .filter(|r| follower(r) == Some(p.uid.as_str()))
            .cloned()
            .collect();
        let mut shared_profile = None;
        for row in followers {
            if row.kind != p.primitive || row.state == "stale" {
                return Err(error("Preset has an incompatible or stale follower"));
            }
            let mut row = (*row).clone();
            if row.kind == "junction" {
                junction::prepare(scene, t, &mut row, &p)?;
                t.pending.insert(row.uid.clone());
                t.insert(row).map_err(error)?;
                continue;
            }
            if row.kind == "repeat" {
                apply_repeat(&mut row, &p, true)?;
                if let Some(g) = group(scene.py(), t, &row.owner) {
                    p.record(&g, true)?;
                }
                t.pending.insert(row.uid.clone());
                t.insert(row).map_err(error)?;
                continue;
            }
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
    let units = py.import("built_core.units")?;
    let uses_inches = |units: Obj<'_>| -> PyResult<bool> {
        let length: String = units.get_item("length")?.extract()?;
        Ok(matches!(length.as_str(), "ft" | "in" | "ft-in" | "in-frac" | "ft-in-frac"))
    };
    let imperial = scene.map(|s| uses_inches(units.call_method1("model_units_of", (s,))?))
        .transpose()?.unwrap_or(false) || uses_inches(units.call_method0("new_document_units")?)?;
    let regional = imperial.then(|| IMPERIAL.rows.as_slice()).unwrap_or_default();
    for p in BUILTINS.rows.iter().chain(regional) {
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
