// SPDX-License-Identifier: GPL-3.0-or-later
//! Saved scenes, sheets and view frames as branches of the Project tree.
use crate::doc::{self, py::handle_key, Key};
use crate::project_tree::{row, text_matches, Row};
use pyo3::prelude::*;
use std::collections::{HashMap, HashSet};

#[derive(PartialEq)]
struct Frame {
    key: Key,
    view: String,
    scale: f64,
}

#[derive(PartialEq)]
struct Sheet {
    key: Key,
    name: String,
    paper: String,
    frames: Vec<Frame>,
}

#[derive(Default, PartialEq)]
struct Catalog {
    views: Vec<(Key, String)>,
    sheets: Vec<Sheet>,
}

impl Catalog {
    fn read(scene: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut catalog = Self::default();
        for view in scene.getattr("saved_views")?.try_iter()? {
            let view = view?;
            catalog
                .views
                .push((handle_key(&view)?, view.getattr("name")?.extract()?));
        }
        for sheet in scene.getattr("compositions")?.try_iter()? {
            let sheet = sheet?;
            let mut frames = Vec::new();
            for frame in sheet.getattr("frames")?.try_iter()? {
                let frame = frame?;
                frames.push(Frame {
                    key: handle_key(&frame)?,
                    view: frame.getattr("view_key")?.extract()?,
                    scale: frame.getattr("scale_n")?.extract()?,
                });
            }
            catalog.sheets.push(Sheet {
                key: handle_key(&sheet)?,
                name: sheet.getattr("name")?.extract()?,
                paper: sheet.getattr("paper")?.extract()?,
                frames,
            });
        }
        Ok(catalog)
    }
}

#[pyclass]
#[derive(Default)]
pub(crate) struct ProjectSheets {
    read: Option<(Key, u64)>,
    version: u64,
    catalog: Catalog,
}

fn entry(key: String, name: &str, detail: &str, depth: usize, children: usize, open: bool) -> Row {
    let glyph = if key.starts_with("saved-scene") { "scene" } else { "sheet" };
    let mut r = row(
        key,
        name.into(),
        detail,
        glyph,
        depth,
        8,
        children,
        0,
    );
    r.1[7] = i64::from(children > 0);
    r.1[8] = i64::from(children > 0 && open);
    r
}

#[pymethods]
impl ProjectSheets {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    fn refresh(&mut self, scene: &Bound<'_, PyAny>) -> PyResult<u64> {
        let root = scene.getattr("_root")?.getattr("_doc_key")?.extract()?;
        // Publish in-place saved-view and frame edits before sampling the revision.
        scene.py().import("built_core.doc_model")?.call_method0("flush")?;
        let serial = doc::lock().serial;
        if self.read != Some((root, serial)) {
            let catalog = Catalog::read(scene)?;
            if self.catalog != catalog {
                self.catalog = catalog;
                self.version += 1;
            }
            self.read = Some((root, serial));
        }
        Ok(self.version)
    }

    fn rows(&self, query: &str, expanded: HashSet<String>) -> Vec<Row> {
        let query = query.trim().to_lowercase();
        let searching = !query.is_empty();
        let matches = |s: &str| {
            query.split_whitespace().all(|word| {
                let (negative, pat) = word.strip_prefix('-').map_or((false, word), |w| (true, w));
                negative != text_matches(&s.to_lowercase(), pat)
            })
        };
        let cat = &self.catalog;
        let mut usage: HashMap<&str, Vec<&str>> = HashMap::new();
        for s in &cat.sheets {
            for f in &s.frames {
                if let Some(name) = f.view.strip_prefix("scene:") {
                    let places = usage.entry(name).or_default();
                    if !places.contains(&s.name.as_str()) {
                        places.push(&s.name);
                    }
                }
            }
        }
        let known: HashSet<_> = cat.views.iter().map(|(_, n)| n.as_str()).collect();
        let mut rows = Vec::new();
        let mut branches = Vec::new();
        for (key, name) in &cat.views {
            if matches(name) {
                let detail = usage.get(name.as_str()).map_or_else(
                    || "Not on a sheet".into(),
                    |s| format!("On {}", s.join(", ")),
                );
                branches.push(entry(
                    format!("saved-scene:{key}"),
                    name,
                    &detail,
                    1,
                    0,
                    false,
                ));
            }
        }
        if !branches.is_empty() {
            let open = searching || expanded.contains("saved-scenes:all");
            rows.push(entry(
                "saved-scenes:all".into(),
                "Saved scenes",
                "",
                0,
                branches.len(),
                open,
            ));
            if open {
                rows.append(&mut branches);
            }
        }
        branches.clear();
        for s in &cat.sheets {
            let key = format!("sheet-page:{}", s.key);
            let page_hit = matches(&s.name);
            let open = searching || expanded.contains(&key);
            let mut children = Vec::new();
            if open {
                for f in &s.frames {
                    let (name, detail) = if let Some(name) = f.view.strip_prefix("scene:") {
                        (
                            name,
                            if known.contains(name) {
                                "Scene"
                            } else {
                                "Scene missing"
                            },
                        )
                    } else if let Some(view) = f.view.strip_prefix("std:") {
                        (view, "Standard view")
                    } else {
                        (f.view.as_str(), "Current view")
                    };
                    if page_hit || matches(name) {
                        children.push(entry(
                            format!("sheet-frame:{}:{}", s.key, f.key),
                            &format!("{name} · 1:{}", f.scale),
                            detail,
                            2,
                            0,
                            false,
                        ));
                    }
                }
            }
            if page_hit || !children.is_empty() {
                branches.push(entry(key, &s.name, &s.paper, 1, s.frames.len(), open));
                branches.append(&mut children);
            }
        }
        if !branches.is_empty() {
            let open = searching || expanded.contains("sheet-pages:all");
            rows.push(entry(
                "sheet-pages:all".into(),
                "Sheets",
                "",
                0,
                cat.sheets.len(),
                open,
            ));
            if open {
                rows.append(&mut branches);
            }
        }
        rows
    }

    fn inspector(&self, key: &str) -> Option<(String, Vec<(String, Vec<(String, String)>)>)> {
        let (kind, id) = key.split_once(':')?;
        let catalog = &self.catalog;
        let (name, details, action) = match kind {
            "saved-scene" => {
                let (_, name) = catalog.views.iter().find(|(k, _)| k.to_string() == id)?;
                let needle = format!("scene:{name}");
                let uses: Vec<_> = catalog
                    .sheets
                    .iter()
                    .filter(|s| s.frames.iter().any(|f| f.view == needle))
                    .map(|s| s.name.as_str())
                    .collect();
                (
                    name.clone(),
                    vec![(
                        "Sheets".into(),
                        if uses.is_empty() {
                            "Not on a sheet".into()
                        } else {
                            uses.join(", ")
                        },
                    )],
                    "Show scene",
                )
            }
            "sheet-page" => {
                let page = catalog.sheets.iter().find(|s| s.key.to_string() == id)?;
                (
                    page.name.clone(),
                    vec![
                        ("Paper".into(), page.paper.clone()),
                        ("View frames".into(), page.frames.len().to_string()),
                    ],
                    "Open sheet",
                )
            }
            "sheet-frame" => {
                let (page, frame) = id.split_once(':')?;
                let page = catalog.sheets.iter().find(|s| s.key.to_string() == page)?;
                let frame = page.frames.iter().find(|f| f.key.to_string() == frame)?;
                let name = frame
                    .view
                    .strip_prefix("scene:")
                    .or_else(|| frame.view.strip_prefix("std:"))
                    .unwrap_or(&frame.view);
                (
                    name.into(),
                    vec![
                        ("Sheet".into(), page.name.clone()),
                        ("Scale".into(), format!("1:{}", frame.scale)),
                    ],
                    "Open sheet",
                )
            }
            _ => return None,
        };
        Some((
            name,
            vec![
                ("Details".into(), details),
                (
                    "Actions".into(),
                    vec![(format!("opensheetrow:{key}"), action.into())],
                ),
            ],
        ))
    }

    /// Resolve against the live scene when activated. Deleting or reordering a
    /// sheet between pointer events must never open another sheet in its place.
    fn target(&self, scene: &Bound<'_, PyAny>, key: &str) -> PyResult<Option<(String, usize)>> {
        let parts: Vec<_> = key.split(':').collect();
        let (family, field) = match parts.as_slice() {
            ["saved-scene", _] => ("scene", "saved_views"),
            ["sheet-page", _] | ["sheet-frame", _, _] => ("sheet", "compositions"),
            _ => return Ok(None),
        };
        let Ok(id) = parts[1].parse::<Key>() else {
            return Ok(None);
        };
        for (index, obj) in scene.getattr(field)?.try_iter()?.enumerate() {
            let obj = obj?;
            if handle_key(&obj)? == id {
                if let Some(frame) = parts.get(2) {
                    let frame = frame.parse::<Key>().ok();
                    for f in obj.getattr("frames")?.try_iter()? {
                        if Some(handle_key(&f?)?) == frame {
                            return Ok(Some((family.into(), index)));
                        }
                    }
                    return Ok(None);
                }
                return Ok(Some((family.into(), index)));
            }
        }
        Ok(None)
    }
}
