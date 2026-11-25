// SPDX-License-Identifier: GPL-3.0-or-later
//! Hatch families, piece sizes and personal shortcuts used by the Drawing lens.
use super::{kit, tokens::*};
use pyo3::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

const RECENT: &str = "layers/recent_hatches";
const FAVOURITES: &str = "layers/favourite_hatches";
const OWN: &str = "layers/own_pieces";
type Sizes = BTreeMap<String, Vec<(String, Vec<f64>)>>;

#[derive(Default)]
pub(super) struct Hatches {
    pub labels: BTreeMap<String, String>,
    groups: Vec<(String, Vec<String>)>,
    pub recent: Vec<String>,
    pub favourites: Vec<String>,
    pub placed: BTreeSet<String>,
    pub wood: BTreeSet<String>,
    pub fitted: BTreeSet<String>,
    pub coursed: BTreeSet<String>,
    pub kinds: BTreeMap<String, String>,
    sizes: Sizes,
    doc: BTreeMap<String, Vec<f64>>,
    own: BTreeMap<String, Vec<Vec<f64>>>,
}

fn settings(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("built_core.settings")?.getattr("settings")?.call0()
}

fn keys(raw: Bound<'_, PyAny>, limit: usize) -> Vec<String> {
    let values = raw
        .extract::<Vec<String>>()
        .unwrap_or_else(|_| raw.extract::<String>().into_iter().collect());
    values
        .into_iter()
        .filter(|s| !s.is_empty())
        .take(limit)
        .collect()
}

impl Hatches {
    pub fn sections(
        &self,
        imported: &BTreeMap<String, serde_json::Value>,
    ) -> Vec<(String, Vec<String>)> {
        let valid = |key: &String| {
            self.labels.contains_key(key)
                || key
                    .strip_prefix("pat:")
                    .is_some_and(|k| imported.contains_key(k))
        };
        let mut groups = Vec::new();
        for (label, source) in [("Favourites", &self.favourites), ("Recent", &self.recent)] {
            let keys: Vec<_> = source.iter().filter(|k| valid(k)).cloned().collect();
            if !keys.is_empty() {
                groups.push((label.to_owned(), keys));
            }
        }
        if self.groups.is_empty() {
            groups.push((
                "General".into(),
                std::iter::once("")
                    .chain(kit::HATCH_KEYS.lines())
                    .map(str::to_owned)
                    .collect(),
            ));
        } else {
            groups.extend(self.groups.iter().cloned());
        }
        if !imported.is_empty() {
            let mut keys: Vec<_> = imported.keys().map(|k| format!("pat:{k}")).collect();
            keys.sort_by_cached_key(|k| k.to_lowercase());
            groups.push(("Imported".into(), keys));
        }
        groups
    }

    pub fn label(&self, key: &str) -> String {
        if key.is_empty() {
            return "No hatch (frame fill)".into();
        }
        self.labels
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.trim_start_matches("pat:").replace('_', " "))
    }

    pub fn matching(
        &self,
        groups: Vec<(String, Vec<String>)>,
        imported: &BTreeMap<String, serde_json::Value>,
        category: &str,
        query: &str,
    ) -> Vec<(String, Vec<String>)> {
        let query = query.trim().to_lowercase();
        groups
            .into_iter()
            .filter_map(|(title, mut keys)| {
                if !category.is_empty() && category != title {
                    return None;
                }
                keys.retain(|key| {
                    self.label(key).to_lowercase().contains(&query)
                        || key.replace('_', " ").to_lowercase().contains(&query)
                        || key
                            .strip_prefix("pat:")
                            .and_then(|k| imported.get(k))
                            .and_then(|p| p["label"].as_str())
                            .is_some_and(|text| text.to_lowercase().contains(&query))
                });
                (!keys.is_empty()).then_some((title, keys))
            })
            .collect()
    }

    pub fn sync(&mut self, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        if self.sizes.is_empty() {
            let py = scene.py();
            let domain = py.import("built_core.hatches")?;
            let names: BTreeMap<String, String> = domain.getattr("HATCH_LABELS")?.extract()?;
            let translate = py.import("built_core.i18n")?.getattr("tr")?;
            let groups = domain
                .getattr("HATCH_GROUPS")?
                .extract::<Vec<(String, Vec<String>)>>()?;
            for (title, mut keys) in groups {
                if title == "General" {
                    keys.insert(0, String::new());
                }
                self.groups
                    .push((translate.call1((title,))?.extract()?, keys));
            }
            for (key, name) in names {
                if let Some(kind) = domain
                    .call_method1("piece_kind", (&key,))?
                    .extract::<Option<String>>()?
                {
                    self.kinds.insert(key.clone(), kind);
                }
                self.labels
                    .insert(key, translate.call1((name,))?.extract()?);
            }
            self.wood = domain
                .getattr("WOOD")?
                .extract::<Vec<_>>()?
                .into_iter()
                .collect();
            self.placed = domain.getattr("PLACED")?.extract()?;
            self.fitted = domain
                .getattr("FITTED")?
                .try_iter()?
                .map(|k| k?.extract())
                .collect::<PyResult<_>>()?;
            self.coursed = domain.getattr("COURSED")?.extract()?;
            let store = settings(py)?;
            self.recent = keys(store.call_method1("value", (RECENT,))?, 7);
            self.favourites = keys(store.call_method1("value", (FAVOURITES,))?, 21);
            let saved = store
                .call_method1("value", (OWN, "{}"))?
                .extract::<String>()
                .unwrap_or_default();
            self.own = serde_json::from_str(&saved).unwrap_or_default();
            self.sizes = domain.getattr("PIECE_SIZES")?.extract()?;
        }
        self.doc = scene.getattr("bricks")?.extract()?;
        Ok(())
    }

    pub fn preference(&mut self, py: Python<'_>, verb: &str, payload: &str) -> PyResult<()> {
        let store = settings(py)?;
        match verb {
            "recent" | "pin" => {
                if payload.is_empty() {
                    return Ok(());
                }
                let (name, list, limit) = if verb == "recent" {
                    (RECENT, &mut self.recent, 7)
                } else {
                    (FAVOURITES, &mut self.favourites, 21)
                };
                let mut next = keys(store.call_method1("value", (name,))?, limit);
                let existed = next.iter().any(|s| s == payload);
                next.retain(|s| s != payload);
                if verb == "recent" {
                    next.insert(0, payload.into());
                } else if !existed {
                    next.push(payload.into());
                }
                next.truncate(limit);
                store.call_method1("setValue", (name, &next))?;
                *list = next;
            }
            "piece" => {
                let (kind, raw): (String, Vec<f64>) = serde_json::from_str(payload)
                    .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
                let value = py
                    .import("built_core.hatches")?
                    .call_method1("clean_brick", (raw,))?;
                let size: Vec<f64> = value.extract().map_err(|_| {
                    pyo3::exceptions::PyValueError::new_err("Invalid piece dimensions")
                })?;
                let Some(builtins) = self.sizes.get(&kind) else {
                    return Err(pyo3::exceptions::PyValueError::new_err(
                        "Unknown piece kind",
                    ));
                };
                if builtins.iter().any(|(_, v)| v == &size) {
                    return Ok(());
                }
                let saved = store
                    .call_method1("value", (OWN, "{}"))?
                    .extract::<String>()
                    .unwrap_or_default();
                let mut all: BTreeMap<String, Vec<Vec<f64>>> =
                    serde_json::from_str(&saved).unwrap_or_default();
                let mine = all.entry(kind).or_default();
                if mine.contains(&size) {
                    mine.retain(|v| v != &size);
                } else {
                    mine.push(size);
                }
                all.retain(|_, v| !v.is_empty());
                store.call_method1("setValue", (OWN, serde_json::json!(all).to_string()))?;
                self.own = all;
            }
            _ => {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "Unknown hatch preference",
                ))
            }
        }
        Ok(())
    }

    pub fn dimensions(&self, key: &str, raw: Option<&str>) -> Vec<f64> {
        let Some(kind) = self.kinds.get(key) else {
            return vec![];
        };
        if let Some(v) = raw
            .filter(|s| !s.is_empty())
            .and_then(|s| {
                s.split(',')
                    .map(|t| t.trim().parse::<f64>().ok())
                    .collect::<Option<Vec<_>>>()
            })
            .filter(|v| {
                matches!(v.len(), 4 | 5)
                    && v.iter().all(|n| n.is_finite() && (0.0..=600.).contains(n))
            })
        {
            return v;
        }
        self.doc
            .get(kind)
            .or_else(|| self.sizes.get(kind)?.first().map(|(_, v)| v))
            .cloned()
            .unwrap_or_default()
    }
    pub fn piece(
        &self,
        ui: &mut egui::Ui,
        kind: &str,
        current: Option<&str>,
        can_promote: bool,
        actions: &mut Vec<String>,
    ) -> Option<String> {
        let choices = self.sizes.get(kind)?;
        let (_, default) = choices.first()?;
        let doc = self.doc.get(kind).unwrap_or(default);
        let mut selected = None;
        let mixed = current.is_none();
        let current = current.unwrap_or_default();
        let size = if current.is_empty() {
            Some(doc.clone())
        } else {
            current
                .split(',')
                .map(|t| t.trim().parse::<f64>().ok())
                .collect()
        };
        let saved = self.own.get(kind).map(Vec::as_slice).unwrap_or_default();
        let describe = |s: &Vec<f64>| {
            s.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        };
        let dimensions = |value: &[f64]| {
            value
                .iter()
                .take(3)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" × ")
        };
        let name = match size.as_ref() {
            _ if mixed => "Mixed".into(),
            _ if current.is_empty() => format!("Document · {}", dimensions(doc)),
            Some(v) if saved.contains(v) => format!("Saved · {}", dimensions(v)),
            Some(v) if choices.iter().any(|(_, s)| s == v) => dimensions(v),
            _ => "Custom".into(),
        };
        kit::property_control(ui, "Preset", |ui| {
            egui::ComboBox::from_id_salt("piece size preset")
                .selected_text(name)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(
                            current.is_empty(),
                            format!("Document · {}", dimensions(doc)),
                        )
                        .clicked()
                    {
                        selected = Some(String::new());
                    }
                    for (_, v) in choices {
                        if ui
                            .selectable_label(Some(v) == size.as_ref(), dimensions(v))
                            .clicked()
                        {
                            selected = Some(if v == doc { String::new() } else { describe(v) });
                        }
                    }
                    for v in saved {
                        if ui
                            .selectable_label(
                                Some(v) == size.as_ref(),
                                format!("Saved · {}", dimensions(v)),
                            )
                            .clicked()
                        {
                            selected = Some(if v == doc { String::new() } else { describe(v) });
                        }
                    }
                });
        });
        if let Some(mut size) =
            size.filter(|v| matches!(v.len(), 4 | 5) && v.iter().all(|n| n.is_finite()))
        {
            let mut changed = false;
            for (i, label, min, max) in [
                (0, "Length", 60., 600.),
                (1, "Width", 30., 400.),
                (2, "Height", 20., 400.),
                (3, "Bed joint", 0., 40.),
            ] {
                kit::property_control(ui, label, |ui| {
                    changed |= ui
                        .add(kit::number(&mut size[i], " mm").range(min..=max))
                        .changed();
                });
            }
            let mut linked = size.len() == 4;
            let was_linked = linked;
            kit::property_control(ui, "Match joints", |ui| {
                kit::toggle(ui, &mut linked, "");
            });
            if was_linked != linked {
                if linked {
                    size.truncate(4);
                } else {
                    size.push(size[3]);
                }
                changed = true;
            }
            if !linked {
                kit::property_control(ui, "Head joint", |ui| {
                    changed |= ui
                        .add(kit::number(&mut size[4], " mm").range(0.0..=40.0))
                        .changed();
                });
            }
            if changed {
                selected = Some(describe(&size));
            }
            ui.horizontal_wrapped(|ui| {
                if !choices.iter().any(|(_, v)| v == &size) {
                    let label = if saved.contains(&size) {
                        "Remove saved size"
                    } else {
                        "Save this size"
                    };
                    if kit::quiet(ui, label).clicked() {
                        actions.push(format!(
                            "hatchpref\u{1f}piece\u{1f}{}",
                            serde_json::json!([kind, size])
                        ));
                    }
                }
                if can_promote
                    && kit::quiet(ui, "Use in whole model")
                        .on_hover_text("Clear individual sizes of this piece kind")
                        .clicked()
                {
                    let value = if size == *default {
                        vec![]
                    } else {
                        size.clone()
                    };
                    actions.push(format!(
                        "documentpiece\u{1f}{kind}\u{1f}{}",
                        serde_json::json!(value)
                    ));
                }
            });
        }
        selected
    }
}

pub(super) fn preview_color(raw: Option<&str>, tag: Option<&str>, ground: bool) -> egui::Color32 {
    if raw == Some("layer") {
        if let Some(c) = tag.and_then(super::parse_hex) {
            let rgb = [c.r(), c.g(), c.b()].map(|v| {
                if ground {
                    (0.75 * 255. + f32::from(v) * 0.25).round() as u8
                } else {
                    (0.7 * f32::from(v)).round() as u8
                }
            });
            return document_rgb(rgb);
        }
    }
    raw.and_then(super::parse_hex)
        .unwrap_or(if ground { WELL } else { INK_2 })
}

#[cfg(test)]
mod picker_tests {
    use super::*;

    #[test]
    fn browse_groups_search_within_a_group_and_hide_missing_recent_imports() {
        let mut hatches = Hatches::default();
        hatches.groups = vec![
            ("General".into(), vec!["".into(), "grid".into()]),
            ("Masonry".into(), vec!["brick".into(), "stone".into()]),
        ];
        hatches.labels = [
            ("brick".into(), "Ladrillo".into()),
            ("stone".into(), "Stone".into()),
        ]
        .into();
        hatches.recent = vec!["pat:Missing".into(), "brick".into()];
        hatches.favourites = vec!["stone".into()];
        let imported = [("Hex".into(), serde_json::json!({"label":"Masonry sample"}))].into();
        let groups = hatches.sections(&imported);
        assert_eq!(
            groups.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            ["Favourites", "Recent", "General", "Masonry", "Imported"]
        );
        assert_eq!(groups[1].1, ["brick"]);
        assert_eq!(groups[2].1[0], "");
        assert_eq!(
            hatches.matching(groups.clone(), &imported, "Masonry", " LADR "),
            [("Masonry".into(), vec!["brick".into()])]
        );
        assert_eq!(
            hatches.matching(groups.clone(), &imported, "", "sample"),
            [("Imported".into(), vec!["pat:Hex".into()])]
        );
        assert!(hatches
            .matching(groups, &imported, "General", "stone")
            .is_empty());
    }
}

#[cfg(test)]
mod piece_tests {
    use super::*;

    #[test]
    fn piece_presets_show_dimensions_and_independent_head_joints_when_needed() {
        let mut h = Hatches::default();
        h.kinds.insert("brick".into(), "brick".into());
        h.sizes.insert(
            "brick".into(),
            vec![("Example".into(), vec![215., 102.5, 65., 10.])],
        );
        let own = vec![300., 150., 80., 12., 8.];
        h.own.insert("brick".into(), vec![own.clone()]);
        h.doc.insert("brick".into(), vec![240., 110., 65., 8.]);
        let ctx = crate::ui::test_context();
        let labels = |value: &str| {
            let output = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    assert!(h
                        .piece(ui, "brick", Some(value), true, &mut vec![])
                        .is_none());
                });
            });
            output
                .shapes
                .into_iter()
                .filter_map(|s| match s.shape {
                    egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let doc = labels("");
        assert!(doc.iter().any(|s| s.contains("240 × 110 × 65")));
        assert!(doc.iter().any(|s| s == "Bed joint") && !doc.iter().any(|s| s == "Head joint"));
        let saved = labels("300,150,80,12,8");
        assert!(saved.iter().any(|s| s.contains("Saved · 300 × 150 × 80")));
        assert!(saved.iter().any(|s| s == "Head joint"));
        assert!(saved.iter().any(|s| s == "Remove saved size"));
        assert_eq!(h.dimensions("brick", Some("300,150,80,12,8")), own);
        assert_eq!(h.dimensions("brick", Some("")), h.doc["brick"]);
        assert!(h.dimensions("concrete", None).is_empty());
    }

    #[test]
    fn by_tag_preview_uses_distinct_ground_and_ink_and_custom_colours_override_them() {
        let ground = preview_color(Some("layer"), Some("#800000"), true);
        let ink = preview_color(Some("layer"), Some("#800000"), false);
        assert!(ground.r() > ink.r() && ground.g() > ink.g());
        assert_eq!(
            preview_color(Some("#ffffff"), None, true).to_array(),
            [255; 4]
        );
        assert_eq!(
            preview_color(Some("#000000"), None, false).to_array(),
            [0, 0, 0, 255]
        );
        assert_ne!(preview_color(None, None, true), ground);
    }
}
