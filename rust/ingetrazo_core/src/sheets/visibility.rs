// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! What a sheet frame shows of the model (docs/rust-sheets.md step 4): the
//! placements its saved view hides, worked out from the document alone.
//! The live scene is never toggled: until now the composer applied the
//! view to the scene (`SavedView.apply_state`), read the rows
//! (views/wgpu_sections.py `capture_rows`) and put the scene back.
//!
//! The rule, per placement, walking the tree as `doc_tree` does: hidden by
//! Hide (its own flag, or the view's hidden objects when it lists them),
//! or its tag hidden (the view's hidden tags when it shows a scene, else
//! the tag's own switch), or a folder of its tag switched off (the view's
//! when it lists folders, else the document's), or a parent hidden.

use crate::doc::{self, value::Value, Data, Key};
use pyo3::prelude::*;
use std::collections::{HashMap, HashSet};

/// What a view says about visibility (None: the document's own state).
#[derive(Default, Clone, Debug)]
pub struct ViewState {
    pub hidden_tags: Option<HashSet<String>>,
    pub hidden_folders: Option<HashSet<String>>,
    pub hidden_objects: Option<HashSet<String>>,
}

fn strings(v: Option<&Value>) -> Option<HashSet<String>> {
    match v {
        Some(Value::List(l)) | Some(Value::Tuple(l)) => Some(l.iter().filter_map(|x| match x {
            Value::Str(s) => Some(s.clone()),
            _ => None,
        }).collect()),
        _ => None,
    }
}

impl ViewState {
    pub fn of_view(f: &super::model::Fields) -> ViewState {
        ViewState {
            // A view always sets every tag: what it does not list shows.
            hidden_tags: Some(strings(f.0.get("hidden_layers")).unwrap_or_default()),
            hidden_folders: strings(f.0.get("hidden_folders")),
            hidden_objects: strings(f.0.get("hidden_objects")),
        }
    }
}

fn dict_str<'a>(v: Option<&'a Value>) -> HashMap<&'a str, &'a Value> {
    match v {
        Some(Value::Dict(d)) => d.iter().filter_map(|(k, v)| match k {
            Value::Str(s) => Some((s.as_str(), v)),
            _ => None,
        }).collect(),
        _ => HashMap::new(),
    }
}

/// For each placement under `roots` (the scene's top level), in `doc_tree`
/// order: its key and whether the frame hides it. A placement listed twice
/// (a shared child) keeps the last answer, as the composer's lookup did.
pub fn hidden(root: Key, roots: &[Key], view: &ViewState) -> PyResult<HashMap<Key, bool>> {
    let d = doc::lock();
    let rn = d.need(root).map_err(pyo3::exceptions::PyRuntimeError::new_err)?;
    let attr = |name: &str| doc::attr_id(name).ok().and_then(|a| rn.attr(a));
    let flags = dict_str(attr("folder_visibility"));
    let parents = dict_str(attr("folder_parents"));
    let folder_visible = |start: &str| -> bool {
        let mut key = Some(start.to_string());
        let mut seen = HashSet::new();
        while let Some(k) = key.filter(|k| !k.is_empty() && !seen.contains(k)) {
            seen.insert(k.clone());
            let on = match &view.hidden_folders {
                Some(off) => !off.contains(&k),
                None => !matches!(flags.get(k.as_str()), Some(Value::Bool(false) | Value::Int(0) | Value::None)),
            };
            if !on {
                return false;
            }
            key = match parents.get(k.as_str()) {
                Some(Value::Str(p)) => Some(p.clone()),
                _ => None,
            };
        }
        true
    };
    let mut tags: HashMap<&str, (bool, Option<&str>)> = HashMap::new();
    for &t in &rn.tags {
        if let Some(Data::Tag(tag)) = d.node(t).map(|n| &n.data) {
            if let Some(k) = tag.key.as_deref() {
                tags.insert(k, (tag.visible, tag.folder.as_deref()));
            }
        }
    }
    let tag_visible = |key: &str| -> bool {
        match tags.get(key) {
            None => true,
            Some((on, folder)) => {
                let on = match &view.hidden_tags {
                    Some(off) => !off.contains(key),
                    None => *on,
                };
                on && folder_visible(folder.unwrap_or(""))
            }
        }
    };
    let mut out = HashMap::new();
    let mut stack: Vec<(Key, bool)> = roots.iter().rev().map(|k| (*k, false)).collect();
    while let Some((k, parent_hidden)) = stack.pop() {
        let Some(n) = d.node(k) else { continue };
        let Data::Placement(p) = &n.data else { continue };
        let own = match &view.hidden_objects {
            Some(set) => p.uid.as_deref().is_some_and(|u| set.contains(u)),
            None => p.hidden,
        };
        let tag = p.tag.as_deref().filter(|t| !t.is_empty()).unwrap_or("Layer 0");
        let h = parent_hidden || own || !tag_visible(tag);
        out.insert(k, h);
        for &c in n.children.iter().rev() {
            stack.push((c, h));
        }
    }
    Ok(out)
}

/// The hidden flag of each row a frame draws: ``rows`` the placements of
/// the resident rows in order (None for the loose model), ``roots`` the
/// scene's top level, ``root`` the scene root, ``view`` the frame's saved
/// view or None (the document as it is). Returns one 0/1 byte per row.
#[pyfunction]
#[pyo3(signature = (root, roots, rows, view=None))]
pub fn sheet_frame_hidden(py: Python<'_>, root: &Bound<'_, PyAny>, roots: Vec<Bound<'_, PyAny>>,
                          rows: Vec<Option<Bound<'_, PyAny>>>, view: Option<Bound<'_, PyAny>>) -> PyResult<Vec<u8>> {
    py.import("core.doc_model")?.call_method0("flush")?;
    let state = match view.filter(|v| !v.is_none()) {
        Some(v) => ViewState::of_view(&super::model::read_fields(doc::py::handle_key(&v)?)?),
        None => ViewState::default(),
    };
    let root = doc::py::handle_key(root)?;
    let roots: Vec<Key> = roots.iter().map(doc::py::handle_key).collect::<PyResult<_>>()?;
    let map = hidden(root, &roots, &state)?;
    rows.iter().map(|r| Ok(match r {
        None => 0,
        Some(g) => match map.get(&doc::py::handle_key(g)?) {
            Some(h) => u8::from(*h),
            None => 1,
        },
    })).collect()
}
