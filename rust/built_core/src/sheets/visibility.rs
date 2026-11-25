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
//! when it lists folders, else the document's), or a parent hidden. A tag
//! set not to print (`printable` False) stays out of every sheet, as
//! `built_core.layers.hide_unprinted` left it.

use crate::doc::{self, value::Value, Data, Key};
use pyo3::prelude::*;
use std::collections::{HashMap, HashSet};

/// `built_core.layers.DEFAULT_LAYER`.
pub const DEFAULT_LAYER: &str = "Layer 0";

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

/// The tags of the document as a frame's view shows them: each tag's
/// effective visibility (its switch or the view's, printable, its folder
/// chain) and lock, owned so it outlives the document lock.
pub struct Tags {
    /// Key → (shown, locked).
    tags: HashMap<String, (bool, bool)>,
}

impl Tags {
    pub fn read(d: &doc::Doc, root: Key, view: &ViewState) -> PyResult<Tags> {
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
        let printable = doc::attr_id("printable").ok();
        let mut tags = HashMap::new();
        let mut folders = HashMap::new();
        for &t in &rn.tags {
            let Some(node) = d.node(t) else { continue };
            let Data::Tag(tag) = &node.data else { continue };
            let Some(k) = tag.key.as_deref() else { continue };
            let prints = !matches!(printable.and_then(|a| node.attr(a)), Some(Value::Bool(false)));
            let on = match &view.hidden_tags {
                Some(off) => !off.contains(k),
                None => tag.visible,
            };
            let folder = tag.folder.clone().unwrap_or_default();
            let open = *folders.entry(folder.clone()).or_insert_with(|| folder_visible(&folder));
            tags.insert(k.to_string(), (on && prints && open, tag.locked));
        }
        Ok(Tags { tags })
    }
    /// Whether a tag shows (an unknown tag does, as the default).
    pub fn shown(&self, key: &str) -> bool {
        self.tags.get(key).is_none_or(|t| t.0)
    }
    /// A known tag that is hidden or locked (`visible_placements`'s blocking).
    pub fn blocking(&self, key: &str) -> bool {
        self.tags.get(key).is_some_and(|t| !t.0 || t.1)
    }
}

/// Hide on a placement as the frame's view has it.
pub fn own_hidden(p: &doc::PlacementData, view: &ViewState) -> bool {
    match &view.hidden_objects {
        Some(set) => p.uid.as_deref().is_some_and(|u| set.contains(u)),
        None => p.hidden,
    }
}

/// For each placement under `roots` (the scene's top level), in `doc_tree`
/// order: its key and whether the frame hides it. A placement listed twice
/// (a shared child) keeps the last answer, as the composer's lookup did.
pub fn hidden(root: Key, roots: &[Key], view: &ViewState) -> PyResult<HashMap<Key, bool>> {
    let d = doc::lock();
    let tags = Tags::read(&d, root, view)?;
    let mut out = HashMap::new();
    let mut stack: Vec<(Key, bool)> = roots.iter().rev().map(|k| (*k, false)).collect();
    while let Some((k, parent_hidden)) = stack.pop() {
        let Some(n) = d.node(k) else { continue };
        let Data::Placement(p) = &n.data else { continue };
        let tag = p.tag.as_deref().filter(|t| !t.is_empty()).unwrap_or(DEFAULT_LAYER);
        let h = parent_hidden || own_hidden(p, view) || !tags.shown(tag);
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
    py.import("built_core.doc_model")?.call_method0("flush")?;
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


/// The same tag and folder visibility for non-mesh reference layers.
#[pyfunction]
pub fn sheet_frame_hidden_tags(py: Python<'_>, root: &Bound<'_, PyAny>,
                              view: Option<&Bound<'_, PyAny>>) -> PyResult<HashSet<String>> {
    py.import("built_core.doc_model")?.call_method0("flush")?;
    let state = match view {
        Some(view) => ViewState::of_view(&super::model::read_fields(doc::py::handle_key(view)?)?),
        None => ViewState::default(),
    };
    let root = doc::py::handle_key(root)?;
    let tags = Tags::read(&doc::lock(), root, &state)?;
    Ok(tags.tags.into_iter().filter_map(|(tag, (shown, _))| (!shown).then_some(tag)).collect())
}
