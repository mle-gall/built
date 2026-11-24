// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! Remembered looks: the next dimension, level mark or ground line is born
//! with the look of the last one edited (the composer's
//! `_last_cota_style`, `_last_nivel_style`, `_last_ground_style`). The
//! dimension's look is also the app's across sessions, in the host's
//! preference store under the composer's key; the others last the session.
//! Units and decimals are never remembered: a new annotation is born in
//! the document's (issue #403).

use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::collections::HashMap;
use std::sync::Mutex;

/// What each kind of item remembers.
const COTA: &[&str] = &["text_mm", "ends", "stroke_mm", "color", "offset_mm", "text_color", "text_bg", "text_bg_opacity"];
const NIVEL: &[&str] = &["text", "symbol", "datum_m", "size_mm", "line_mm", "mirror", "stroke_mm"];
const GROUND: &[&str] = &["ground", "tick_mm", "tick_step_mm", "band_mm", "stroke_mm"];

/// The preference key of the remembered dimension look, and the older
/// keys read once when it is empty, minus the fields whose meaning or
/// factory default changed since (the composer's `_COTA_STYLE_KEYS_OLD`).
const COTA_KEY: &str = "composer/default_cota_style_4";
const COTA_KEYS_OLD: &[(&str, &[&str])] = &[
    ("composer/default_cota_style_3", &["units", "decimals"]),
    ("composer/default_cota_style_2", &["offset_mm", "units", "decimals"]),
    ("composer/default_cota_style", &["ends", "text_along", "text_pos", "offset_mm", "units", "decimals"]),
];

static LOOKS: Mutex<Option<HashMap<&'static str, Py<PyDict>>>> = Mutex::new(None);

fn fields_of(kind: &str) -> &'static [&'static str] {
    match kind {
        "cota" => COTA,
        "nivel" => NIVEL,
        "ground" => GROUND,
        _ => &[],
    }
}

fn kind_key(kind: &str) -> Option<&'static str> {
    match kind {
        "cota" => Some("cota"),
        "nivel" => Some("nivel"),
        "ground" => Some("ground"),
        _ => None,
    }
}

/// Which look an item carries: a dimension's, a level mark's, a ground
/// line's; None for the others.
fn kind_of(item: &Bound<'_, PyAny>) -> PyResult<Option<&'static str>> {
    let class = item.get_type().name()?.to_string();
    Ok(match class.as_str() {
        "CotaItem" => Some("cota"),
        "NivelItem" => Some("nivel"),
        "FormaItem" if item.getattr("kind")?.extract::<String>()? == "terreno" => Some("ground"),
        _ => None,
    })
}

/// The remembered look of ``kind`` ("cota", "nivel", "ground"): its
/// fields as a new item takes them; empty when there is none.
pub fn look<'py>(py: Python<'py>, kind: &str) -> Bound<'py, PyDict> {
    let got = LOOKS.lock().ok().and_then(|l| l.as_ref().and_then(|m| m.get(kind).map(|d| d.clone_ref(py))));
    match got {
        Some(d) => d.into_bound(py).copy().unwrap_or_else(|_| PyDict::new(py)),
        None => PyDict::new(py),
    }
}

fn store(py: Python<'_>, kind: &'static str, d: &Bound<'_, PyDict>) {
    if let Ok(mut l) = LOOKS.lock() {
        l.get_or_insert_with(HashMap::new).insert(kind, d.copy().map_or_else(|_| PyDict::new(py).unbind(), Bound::unbind));
    }
}

/// The host's preference store (core.settings).
fn prefs(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("core.settings")?.getattr("settings")?.call0()
}

/// ``item``'s look becomes the next one of its kind when ``changed`` —
/// the fields an edit set — touches it (every field when None); a
/// dimension's is saved for the next session too. Returns the kind it
/// remembered, else None.
#[pyfunction]
#[pyo3(signature = (item, changed=None))]
pub fn sheet_remember_look(py: Python<'_>, item: &Bound<'_, PyAny>, changed: Option<Vec<String>>) -> PyResult<Option<String>> {
    let Some(kind) = kind_of(item)? else { return Ok(None) };
    let fields = fields_of(kind);
    if let Some(changed) = &changed {
        // A move or a new text teaches nothing; an edit of the look does.
        if !changed.iter().any(|c| fields.contains(&c.as_str())) {
            return Ok(None);
        }
    }
    let d = PyDict::new(py);
    for f in fields {
        if let Some(v) = item.getattr_opt(*f)? {
            d.set_item(*f, v)?;
        }
    }
    store(py, kind, &d);
    if kind == "cota" {
        let raw = py.import("json")?.call_method1("dumps", (&d,))?;
        // A look that cannot be saved still holds for the session.
        if let Ok(s) = prefs(py) {
            let _ = s.call_method1("setValue", (COTA_KEY, raw));
        }
    }
    Ok(Some(kind.to_string()))
}

/// The dimension look saved by an earlier session (the composer's
/// `_load_default_cota_style`), read from the host's preference store.
#[pyfunction]
pub fn sheet_restore_looks(py: Python<'_>) -> PyResult<()> {
    let Ok(s) = prefs(py) else { return Ok(()) };
    let json = py.import("json")?;
    let read = |key: &str| -> Bound<'_, PyDict> {
        let raw = s.call_method1("value", (key, "")).ok().filter(|v| v.is_truthy().unwrap_or(false));
        raw.and_then(|r| r.str().ok().and_then(|t| json.call_method1("loads", (t,)).ok()))
            .and_then(|v| v.cast_into::<PyDict>().ok())
            .unwrap_or_else(|| PyDict::new(py))
    };
    let mut style = read(COTA_KEY);
    for (key, dropped) in COTA_KEYS_OLD {
        if !style.is_empty() {
            break;
        }
        style = read(key);
        for k in *dropped {
            let _ = style.del_item(*k);
        }
    }
    let keep = PyDict::new(py);
    for (k, v) in style.iter() {
        if COTA.contains(&k.str()?.to_string().as_str()) {
            keep.set_item(k, v)?;
        }
    }
    store(py, "cota", &keep);
    Ok(())
}

/// The remembered look of ``kind`` (a copy), for the inspector and tests.
#[pyfunction]
pub fn sheet_look<'py>(py: Python<'py>, kind: &str) -> PyResult<Bound<'py, PyDict>> {
    Ok(match kind_key(kind) {
        Some(k) => look(py, k),
        None => PyDict::new(py),
    })
}

/// Forget every remembered look (a new document session, tests).
#[pyfunction]
pub fn sheet_forget_looks() {
    if let Ok(mut l) = LOOKS.lock() {
        *l = None;
    }
}

/// Put the remembered look of ``kind`` into the keyword arguments ``kw``
/// of a new item, only the fields in ``only`` when given.
pub fn apply<'py>(py: Python<'py>, kind: &str, kw: &Bound<'py, PyDict>, only: Option<&[&str]>) -> PyResult<()> {
    for (k, v) in look(py, kind).iter() {
        let name = k.str()?.to_string();
        if only.is_none_or(|o| o.contains(&name.as_str())) {
            kw.set_item(k, v)?;
        }
    }
    Ok(())
}
