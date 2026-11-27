// SPDX-License-Identifier: GPL-3.0-or-later
//! Persistent maps share unchanged branches between document undo records.
use im::{HashMap, OrdSet};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap as StdMap, HashSet};
use std::sync::Arc;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Reference {
    Point([f64; 3]),
    Placement {
        placement: String,
    },
    Element {
        path: Vec<String>,
        id: String,
    },
    Row {
        row: String,
    },
    Axis {
        sweep: String,
        at: Station,
        #[serde(default)]
        point: Option<String>,
    },
    Slot {
        slot: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Station {
    End(String),
    Fraction(f64),
    Offset { from: String, mm: f64 },
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct SketchFields {
    #[serde(
        default,
        skip_serializing_if = "serde_json::Value::is_null",
        rename = "name"
    )]
    pub name: serde_json::Value,
    #[serde(
        default,
        skip_serializing_if = "serde_json::Value::is_null",
        rename = "use"
    )]
    pub usage: serde_json::Value,
    #[serde(
        default,
        skip_serializing_if = "serde_json::Value::is_null",
        rename = "plane"
    )]
    pub plane: serde_json::Value,
    #[serde(
        default,
        skip_serializing_if = "serde_json::Value::is_null",
        rename = "points"
    )]
    pub points: serde_json::Value,
    #[serde(
        default,
        skip_serializing_if = "serde_json::Value::is_null",
        rename = "origin"
    )]
    pub origin: serde_json::Value,
    #[serde(
        default,
        skip_serializing_if = "serde_json::Value::is_null",
        rename = "attach"
    )]
    pub attach: serde_json::Value,
    #[serde(
        default,
        skip_serializing_if = "serde_json::Value::is_null",
        rename = "dims"
    )]
    pub dims: serde_json::Value,
    #[serde(
        default,
        skip_serializing_if = "serde_json::Value::is_null",
        rename = "extrude"
    )]
    pub extrude: serde_json::Value,
    #[serde(
        default,
        skip_serializing_if = "serde_json::Value::is_null",
        rename = "next_id"
    )]
    pub next_id: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Row {
    pub uid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curves: Option<serde_json::Value>,
    #[serde(flatten)]
    pub sketch_fields: SketchFields,
    pub kind: String,
    #[serde(flatten)]
    pub repeat: super::repeat::Fields,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cuts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clearance: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cutter: Option<super::cut::Cutter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<super::cut::Anchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<bool>,
    #[serde(default = "one")]
    pub v: u32,
    #[serde(default = "unit", skip_serializing_if = "is_unit")]
    pub size: [f64; 3],
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub open_top: bool,
    #[serde(default)]
    pub inputs: Vec<Reference>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub slots: std::collections::BTreeMap<String, Reference>,
    #[serde(default)]
    pub state: String,
    #[serde(flatten, deserialize_with = "read_sweep")]
    pub sweep: Option<Box<super::sweep::Sweep>>,
}
// The JSON reader uses arbitrary-precision numbers. Buffer flattened fields as
// JSON so decimal dimensions also survive the live_set and file-load paths.
fn read_sweep<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<Box<super::sweep::Sweep>>, D::Error> {
    let remaining = serde_json::Value::deserialize(d)?;
    if remaining.get("profile").is_none() && remaining.get("path").is_none() {
        return Ok(None);
    }
    serde_json::from_value(remaining)
        .map(Some)
        .map_err(serde::de::Error::custom)
}
fn one() -> u32 {
    1
}
fn unit() -> [f64; 3] {
    [1.; 3]
}
fn is_unit(size: &[f64; 3]) -> bool {
    *size == unit()
}
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Key {
    Root,
    Placement(String),
    Row(String),
}
impl Row {
    pub fn references(&self) -> impl Iterator<Item = &Reference> {
        self.inputs
            .iter()
            .chain(self.slots.values())
            .chain(self.anchor.iter().map(|a| &a.reference))
            .chain(self.sweep.iter().flat_map(|s| s.path.references()))
    }
    fn keys(&self) -> HashSet<Key> {
        let mut keys: HashSet<_> = self.cuts.iter().cloned().map(Key::Row).collect();
        if let Some(uid) = self.sweep.as_ref().and_then(|s| s.profile.sketch.as_ref()) {
            keys.insert(Key::Row(uid.clone()));
        }
        for r in self.references() {
            match r {
                Reference::Point(_) | Reference::Slot { .. } => (),
                Reference::Axis { sweep, .. } => {
                    keys.insert(Key::Row(sweep.clone()));
                }
                Reference::Placement { placement } => {
                    keys.insert(Key::Placement(placement.clone()));
                }
                Reference::Row { row } => {
                    keys.insert(Key::Row(row.clone()));
                }
                Reference::Element { path, .. } => {
                    if path.is_empty() {
                        keys.insert(Key::Root);
                    }
                    keys.extend(path.iter().cloned().map(Key::Placement))
                }
            }
        }
        keys
    }
}
#[pyo3::pyclass(frozen, skip_from_py_object)]
#[derive(Default, Clone)]
pub struct LiveTable {
    pub presets: Arc<std::collections::BTreeMap<String, super::presets::Preset>>,
    pub cut_targets: OrdSet<String>,
    pub repeats: OrdSet<String>,
    pub groups: Arc<std::collections::HashMap<String, u64>>,
    pub rows: HashMap<String, Arc<Row>>,
    pub owners: HashMap<String, String>,
    pub pending: OrdSet<String>,
    pub(super) frames: HashMap<u64, OrdSet<String>>,
    pub(super) row_frames: HashMap<String, Vec<u64>>,
    pub(super) users: HashMap<Key, OrdSet<String>>,
    pub(super) order: HashMap<String, u64>,
    pub(super) serial: u64,
}
impl LiveTable {
    pub fn by_creation(&self) -> Vec<&Row> {
        let mut rows: Vec<_> = self.rows.values().map(Arc::as_ref).collect();
        rows.sort_by_key(|r| self.order[&r.uid]);
        rows
    }
    fn clear_frames(&mut self, uid: &str) {
        for frame in self.row_frames.remove(uid).into_iter().flatten() {
            if let Some(users) = self.frames.get_mut(&frame) {
                users.remove(uid);
                if users.is_empty() {
                    self.frames.remove(&frame);
                }
            }
        }
    }

    pub(super) fn bind_frames(&mut self, uid: &str, mut frames: Vec<u64>) -> bool {
        frames.sort_unstable();
        if self
            .row_frames
            .get(uid)
            .map(Vec::as_slice)
            .unwrap_or_default()
            == frames
        {
            return false;
        }
        self.clear_frames(uid);
        if !frames.is_empty() {
            for k in &frames {
                self.frames.entry(*k).or_default().insert(uid.into());
            }
            self.row_frames.insert(uid.into(), frames);
        }
        true
    }
    pub fn remove(&mut self, uid: &str) -> Option<Arc<Row>> {
        let row = self.rows.remove(uid)?;
        self.clear_frames(uid);
        self.cut_targets.remove(uid);
        self.repeats.remove(uid);
        if row.kind != "cut" {
            self.owners.remove(&row.owner);
        }
        for k in row.keys() {
            if let Some(users) = self.users.get_mut(&k) {
                users.remove(uid);
                if users.is_empty() {
                    self.users.remove(&k);
                }
            }
        }
        self.order.remove(uid);
        Some(row)
    }
    pub fn insert(&mut self, row: Row) -> Result<(), &'static str> {
        if row.kind != "cut"
            && !row.owner.is_empty()
            && self.owners.get(&row.owner).is_some_and(|id| id != &row.uid)
        {
            return Err("placement already has a recipe");
        }
        if self
            .rows
            .get(&row.uid)
            .is_some_and(|old| old.owner != row.owner || old.kind != row.kind)
        {
            return Err("recipe owner cannot change");
        }
        let ordinal = self.order.get(&row.uid).copied().unwrap_or_else(|| {
            self.serial += 1;
            self.serial
        });
        self.remove(&row.uid);
        for k in row.keys() {
            self.users.entry(k).or_default().insert(row.uid.clone());
        }
        if row.kind != "cut" && !row.owner.is_empty() {
            self.owners.insert(row.owner.clone(), row.uid.clone());
        }
        if row.kind == "repeat" {
            self.repeats.insert(row.uid.clone());
        }
        if !row.cuts.is_empty() {
            self.cut_targets.insert(row.uid.clone());
        }
        self.order.insert(row.uid.clone(), ordinal);
        self.rows.insert(row.uid.clone(), Arc::new(row));
        Ok(())
    }
    pub fn set_state(&mut self, uid: &str, state: &str) {
        let row = self.rows.get_mut(uid).unwrap();
        if row.state != state {
            Arc::make_mut(row).state = state.to_owned();
        }
    }
    pub fn users(&self, k: &Key) -> impl Iterator<Item = &String> {
        self.users.get(k).into_iter().flatten()
    }
    pub fn schedule(
        &self,
        seeds: impl IntoIterator<Item = String>,
    ) -> Result<Vec<String>, &'static str> {
        let mut dirty = HashSet::new();
        let mut queue: Vec<_> = seeds.into_iter().collect();
        while let Some(id) = queue.pop() {
            let Some(row) = self.rows.get(&id) else {
                continue;
            };
            if dirty.insert(id.clone()) {
                queue.extend(self.users(&Key::Row(id)).cloned());
                if row.kind != "cut" {
                    queue.extend(self.users(&Key::Placement(row.owner.clone())).cloned());
                }
            }
        }
        let mut ready = BTreeSet::new();
        let mut next: StdMap<&str, Vec<&str>> = StdMap::new();
        let mut counts = StdMap::new();
        for id in &dirty {
            let deps: HashSet<_> = self.rows[id]
                .keys()
                .into_iter()
                .filter_map(|k| match k {
                    Key::Root => None,
                    Key::Row(id) => Some(id),
                    Key::Placement(p) => self.owners.get(&p).cloned(),
                })
                .filter(|k| dirty.contains(k) && self.rows[id].state != "cycle")
                .collect();
            for dep in &deps {
                next.entry(self.rows[dep].uid.as_str())
                    .or_default()
                    .push(id);
            }
            if deps.is_empty() {
                ready.insert((self.order[id], id.as_str()));
            }
            counts.insert(id.as_str(), deps.len());
        }
        let mut result = Vec::new();
        while let Some((_, id)) = ready.pop_first() {
            result.push(id.to_owned());
            for &user in next.get(id).into_iter().flatten() {
                let n = counts.get_mut(user).unwrap();
                *n -= 1;
                if *n == 0 {
                    ready.insert((self.order[user], user));
                }
            }
        }
        if result.len() != dirty.len() {
            return Err("cycle");
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, owner: &str, inputs: Vec<Reference>) -> Row {
        Row {
            repeat: Default::default(),
            uid: id.into(),
            owner: owner.into(),
            kind: "test_box".into(),
            v: 1,
            size: unit(),
            sweep: None,
            curves: None,
            sketch_fields: Default::default(),
            cuts: vec![],
            source: None,
            clearance: None,
            cutter: None,
            anchor: None,
            operation: None,
            end: None,
            span: None,
            offset: None,
            on: None,
            open_top: false,
            inputs,
            slots: Default::default(),
            state: "ok".into(),
        }
    }

    #[test]
    fn a_diamond_runs_once_in_creation_order() {
        let mut t = LiveTable::default();
        let source = || Reference::Placement {
            placement: "anchor".into(),
        };
        t.insert(row("right", "b", vec![source()])).unwrap();
        t.insert(row("left", "a", vec![source()])).unwrap();
        t.insert(row(
            "end",
            "c",
            vec![
                Reference::Row { row: "left".into() },
                Reference::Placement {
                    placement: "b".into(),
                },
            ],
        ))
        .unwrap();
        let saved = t.clone();
        let affected = t.users(&Key::Placement("anchor".into())).cloned();
        assert_eq!(t.schedule(affected).unwrap(), ["right", "left", "end"]);
        t.insert(row(
            "left",
            "a",
            vec![Reference::Placement {
                placement: "c".into(),
            }],
        ))
        .unwrap();
        assert_eq!(t.schedule(["left".into()]), Err("cycle"));
        assert_eq!(saved.schedule(["left".into()]).unwrap(), ["left", "end"]);
    }
}
