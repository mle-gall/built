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
    pub recent: Vec<String>,
    pub favourites: Vec<String>,
    pub placed: BTreeSet<String>,
    pub wood: BTreeSet<String>,
    pub coursed: BTreeSet<String>,
    pub kinds: BTreeMap<String, String>,
    sizes: Sizes,
    doc: BTreeMap<String, Vec<f64>>,
    own: BTreeMap<String, Vec<Vec<f64>>>,
}

fn settings(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("PySide6.QtCore")?.getattr("QSettings")?.call0()
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
    pub fn sync(&mut self, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        if self.sizes.is_empty() {
            let py = scene.py();
            let domain = py.import("core.hatches")?;
            let names: BTreeMap<String, String> = domain.getattr("HATCH_LABELS")?.extract()?;
            let translate = py.import("core.i18n")?.getattr("tr")?;
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
                    .import("core.hatches")?
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
        let current = current?;
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
        let name = if current.is_empty() {
            "Document size".to_owned()
        } else {
            choices
                .iter()
                .find(|(_, v)| Some(v) == size.as_ref())
                .map_or_else(|| "Custom".into(), |(n, _)| n.clone())
        };
        kit::property_control(ui, "Preset", |ui| {
            egui::ComboBox::from_id_salt("piece size preset")
                .selected_text(name)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(
                            current.is_empty(),
                            format!("Document · {}", describe(doc)),
                        )
                        .clicked()
                    {
                        selected = Some(String::new());
                    }
                    for (label, v) in choices {
                        if ui
                            .selectable_label(
                                Some(v) == size.as_ref(),
                                format!("{label} · {}", describe(v)),
                            )
                            .clicked()
                        {
                            selected = Some(describe(v));
                        }
                    }
                    for v in saved {
                        if ui
                            .selectable_label(
                                Some(v) == size.as_ref(),
                                format!("Saved · {}", describe(v)),
                            )
                            .clicked()
                        {
                            selected = Some(describe(v));
                        }
                    }
                });
        });
        if let Some(size) =
            size.filter(|v| matches!(v.len(), 4 | 5) && v.iter().all(|n| n.is_finite()))
        {
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
