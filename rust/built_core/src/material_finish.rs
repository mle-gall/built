// SPDX-License-Identifier: GPL-3.0-or-later
//! Surface appearance shared by the material inspector and render exporters.
use pyo3::prelude::*;
use regex::Regex;
use std::sync::LazyLock;

pub struct Finish {
    pub id: &'static str,
    pub label: &'static str,
    /// Roughness, metallic, transmission, index of refraction.
    pub pbr: [f64; 4],
}

pub const FINISHES: &[Finish] = &[
    Finish {
        id: "matte",
        label: "Matte",
        pbr: [0.9, 0.0, 0.0, 1.5],
    },
    Finish {
        id: "satin",
        label: "Satin",
        pbr: [0.45, 0.0, 0.0, 1.5],
    },
    Finish {
        id: "gloss",
        label: "Gloss",
        pbr: [0.12, 0.0, 0.0, 1.5],
    },
    Finish {
        id: "metal",
        label: "Metal",
        pbr: [0.3, 1.0, 0.0, 1.5],
    },
    Finish {
        id: "glass",
        label: "Glass",
        pbr: [0.02, 0.0, 1.0, 1.45],
    },
    Finish {
        id: "water",
        label: "Water",
        pbr: [0.03, 0.0, 0.0, 1.33],
    },
];

// Earlier rules take precedence within a name. Short words like "mar"
// require a right boundary so they do not classify "marble" as water.
static RULES: LazyLock<Vec<(&str, Regex)>> = LazyLock::new(|| {
    [
        ("water", r"agua|water|pool|piscina|pileta|espejo[_ ]de[_ ]agua|(?:lago|lake|mar|sea)(?:$|[^a-záéíóúñç])|ocean|oc[eé]ano|[aá]gua"),
        ("glass", r"vidri|glass|cristal|crystal|vitral|window[_ ]?pane|mampara|vidro"),
        ("metal", r"metal|acero|steel|a[cç]o(?:$|[^a-záéíóúñç])|alumin|hierro|iron|ferro|chrom|cromo|inox|bronce|bronze|cobre|copper|lat[oó]n|brass|galvaniz|zinc|zinco|titani"),
        ("gloss", r"m[aá]rmol|marble|m[aá]rmore|granit|porcel|cer[aá]mic|ceramic|azulej|tile|lacad|lacquer|laca|pulid|polish|brillant|glossy|pl[aá]stic|acr[ií]lic|esmalt|enamel|lacquered|gleam"),
        ("satin", r"madera|wood|madeira|parquet|laminad|laminate|cuero|leather|couro|satin|satinad|semi"),
    ].into_iter().map(|(id, words)| {
        (id, Regex::new(&format!(r"(?i)(?:^|[^a-záéíóúñç])(?:{words})")).unwrap())
    }).collect()
});

pub fn lookup(id: &str) -> Option<&'static Finish> {
    FINISHES.iter().find(|f| f.id == id)
}

#[pyfunction]
#[pyo3(signature=(name, picture=None, opacity=None))]
pub fn guess_finish(
    name: Option<&str>,
    picture: Option<&str>,
    opacity: Option<f64>,
) -> &'static str {
    for text in [name, picture].into_iter().flatten() {
        let normalized = text.replace('-', "_");
        if let Some((id, _)) = RULES.iter().find(|(_, rx)| rx.is_match(&normalized)) {
            return id;
        }
    }
    if opacity.is_some_and(|alpha| alpha < 0.999) {
        "glass"
    } else {
        "matte"
    }
}

#[pyfunction]
#[pyo3(signature=(chosen, name, picture=None, opacity=None))]
pub fn resolve_finish(
    chosen: Option<&str>,
    name: Option<&str>,
    picture: Option<&str>,
    opacity: Option<f64>,
) -> &'static str {
    chosen
        .and_then(lookup)
        .map(|f| f.id)
        .unwrap_or_else(|| guess_finish(name, picture, opacity))
}
