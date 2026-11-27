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
    Placement { placement: String },
    Element { path: Vec<String>, id: String },
    Row { row: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Row {
    pub uid: String,
    pub owner: String,
    pub kind: String,
    #[serde(default = "one")]
    pub v: u32,
    #[serde(default = "unit")]
    pub size: [f64; 3],
    #[serde(default)]
    pub open_top: bool,
    #[serde(default)]
    pub inputs: Vec<Reference>,
    #[serde(default)]
    pub state: String,
}
fn one() -> u32 {
    1
}
fn unit() -> [f64; 3] {
    [1.; 3]
}
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Key {
    Root,
    Placement(String),
    Row(String),
}
impl Row {
    fn keys(&self) -> HashSet<Key> {
        let mut keys = HashSet::new();
        for r in &self.inputs {
            match r {
                Reference::Point(_) => (),
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
    pub groups: Arc<std::collections::HashMap<String, u64>>,
    pub rows: HashMap<String, Arc<Row>>,
    pub owners: HashMap<String, String>,
    pub pending: OrdSet<String>,
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
    pub fn remove(&mut self, uid: &str) -> Option<Arc<Row>> {
        let row = self.rows.remove(uid)?;
        self.owners.remove(&row.owner);
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
        if self.owners.get(&row.owner).is_some_and(|id| id != &row.uid) {
            return Err("placement already has a recipe");
        }
        if self
            .rows
            .get(&row.uid)
            .is_some_and(|old| old.owner != row.owner)
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
        self.owners.insert(row.owner.clone(), row.uid.clone());
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
                queue.extend(self.users(&Key::Placement(row.owner.clone())).cloned());
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
