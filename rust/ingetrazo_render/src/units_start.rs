//! Transactional first-run units. Only an accepted draft leaves this model.
use crate::ui::dialog::{Field, DISABLED};
use pyo3::{exceptions::PyValueError, prelude::*};
use serde_json::{json, Value};

const PRESETS: [&str; 5] = ["metric", "centimetric", "millimetric", "imperial", "custom"];
const TITLES: [&str; 5] = [
    "Metres",
    "Centimetres",
    "Millimetres",
    "Imperial (feet and inches)",
    "Custom…",
];
const DETAILS: [&str; 4] = [
    "m · m² · m³ · °",
    "cm · cm² · cm³ · °",
    "mm · mm² · cm³ · °",
    "ft-in, 1/16\" · ft² · ft³ · °",
];

#[pyclass]
pub struct UnitsDraft {
    defaults: Value,
    document: Value,
    profiles: Value,
    draft: Value,
    selected: usize,
    codes: [Vec<String>; 4],
    #[pyo3(get)]
    custom: bool,
    #[pyo3(get)]
    ask_again: bool,
    #[pyo3(get)]
    for_new: bool,
}

fn field(id: &str, kind: &str, label: String, value: usize, options: Vec<String>) -> Field {
    (
        id.into(),
        kind.into(),
        label,
        String::new(),
        value as f64,
        0.,
        options.len() as f64,
        options,
        0,
        String::new(),
    )
}

fn profile(value: &Value, profiles: &Value) -> usize {
    let mut dimensions = value.clone();
    if let Some(obj) = dimensions.as_object_mut() {
        obj.remove("decimal");
    }
    PRESETS
        .iter()
        .position(|name| profiles[*name] == dimensions)
        .unwrap_or(4)
}

fn decode(raw: &str) -> PyResult<Value> {
    serde_json::from_str::<serde_json::Map<String, Value>>(raw)
        .map(Value::Object)
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

#[pymethods]
impl UnitsDraft {
    #[new]
    fn new(
        defaults: &str,
        document: &str,
        profiles: &str,
        mut codes: [Vec<String>; 4],
    ) -> PyResult<Self> {
        let defaults = decode(defaults)?;
        let profiles = decode(profiles)?;
        if !PRESETS[..4].iter().all(|name| profiles[*name].is_object()) {
            return Err(PyValueError::new_err("Missing unit profile"));
        }
        for index in [1, 2] {
            codes[index].insert(0, "auto".into());
        }
        Ok(Self {
            selected: profile(&defaults, &profiles),
            draft: defaults.clone(),
            defaults,
            document: decode(document)?,
            profiles,
            codes,
            custom: false,
            ask_again: true,
            for_new: true,
        })
    }

    fn edit_document(&mut self, for_new: bool, ask_again: bool) {
        self.custom = true;
        self.for_new = for_new;
        self.ask_again = ask_again;
        self.draft = self.document.clone();
        self.selected = profile(&self.draft, &self.profiles);
    }

    #[getter]
    fn note(&self) -> &'static str {
        if self.custom {
            ""
        } else {
            "Which units do you work in? Lengths, areas, volumes and angles will be shown and typed in them. You can change this at any time in Preferences ▸ Units (Ctrl+U)."
        }
    }

    fn settings(&self) -> String {
        self.draft.to_string()
    }

    /// The first question also remembers "don't ask" when dismissed.
    fn answer(&mut self, accepted: bool) -> &'static str {
        if !accepted {
            return "cancel";
        }
        if !self.custom && self.selected == 4 {
            self.edit_document(true, self.ask_again);
            "custom"
        } else {
            "apply"
        }
    }

    fn receive(&mut self, id: &str, raw: &str) {
        match id {
            "dont_ask" if !self.custom => {
                self.ask_again = raw != "1";
                return;
            }
            "ask_again" if self.custom => {
                self.ask_again = raw == "1";
                return;
            }
            "for_new" if self.custom => {
                self.for_new = raw == "1";
                return;
            }
            _ => {}
        }
        let Ok(n) = raw.parse::<usize>() else {
            return;
        };
        if id == "profile" && n < PRESETS.len() {
            self.selected = n;
            if n < 4 {
                let decimal = self.draft["decimal"].clone();
                self.draft = self.profiles[PRESETS[n]].clone();
                if decimal == "," {
                    self.draft["decimal"] = decimal;
                }
            } else if !self.custom {
                self.draft = self.defaults.clone();
            }
            return;
        }
        if !self.custom {
            return;
        }
        if let Some(i) = ["length", "area", "volume", "angle"]
            .iter()
            .position(|k| *k == id)
        {
            let Some(code) = self.codes[i].get(n) else {
                return;
            };
            if id == "length" && code.ends_with("-frac") {
                self.draft["precision"] =
                    json!(self.draft["precision"].as_u64().unwrap_or(2).min(4));
            }
            if id == "area" && code == "auto" {
                self.draft.as_object_mut().unwrap().remove("area_precision");
            } else if id == "area" && self.draft["area_precision"].is_null() {
                self.draft["area_precision"] = self.draft["precision"].clone();
            }
            self.draft[id] = json!(code);
        } else {
            match id {
                "decimal" if n < 2 => self.draft[id] = json!([".", ","][n]),
                "precision" if n <= if self.fractional() { 4 } else { 6 } => {
                    self.draft[id] = json!(n)
                }
                "volume_precision" | "angle_precision" if n <= 6 => self.draft[id] = json!(n),
                "area_precision"
                    if n <= 6 && self.draft["area"].as_str().is_some_and(|s| s != "auto") =>
                {
                    self.draft[id] = json!(n)
                }
                _ => return,
            }
        }
        let obj = self.draft.as_object_mut().unwrap();
        for (key, default) in [
            ("area", json!("auto")),
            ("volume", json!("auto")),
            ("volume_precision", json!(3)),
            ("angle", json!("deg")),
            ("angle_precision", json!(1)),
            ("decimal", json!(".")),
        ] {
            if obj.get(key) == Some(&default) {
                obj.remove(key);
            }
        }
        self.selected = profile(&self.draft, &self.profiles);
    }

    fn rows(
        &self,
        tr: &Bound<'_, PyAny>,
        unit_label: &Bound<'_, PyAny>,
        preview: &str,
        auto_labels: (String, String),
        fractions: Vec<String>,
    ) -> PyResult<Vec<Field>> {
        let t = |s: &str| tr.call1((s,))?.extract::<String>();
        let mut choices = Vec::new();
        for (i, title) in TITLES.iter().enumerate() {
            let label = t(title)?;
            choices.push(if i < 4 {
                format!("{label} — {}", DETAILS[i])
            } else {
                label
            });
        }
        let mut rows = vec![field(
            "profile",
            "choice",
            t("Profile")?,
            self.selected,
            choices,
        )];
        if self.custom {
            let decimals = (0..=6)
                .map(|n| {
                    Ok(format!(
                        "{n}{}",
                        t(if n == 1 { " decimal" } else { " decimals" })?
                    ))
                })
                .collect::<PyResult<Vec<_>>>()?;
            for (i, (id, label, default, precision, fallback)) in [
                ("length", "Length", "m", "precision", 2),
                (
                    "area",
                    "Area",
                    "auto",
                    "area_precision",
                    self.draft["precision"].as_u64().unwrap_or(2),
                ),
                ("volume", "Volume", "auto", "volume_precision", 3),
                ("angle", "Angle", "deg", "angle_precision", 1),
            ]
            .iter()
            .enumerate()
            {
                let code = self.draft[*id].as_str().unwrap_or(default);
                let labels = self.codes[i]
                    .iter()
                    .map(|c| match c.as_str() {
                        "auto" if i == 1 => Ok(auto_labels.0.clone()),
                        "auto" => Ok(auto_labels.1.clone()),
                        _ => unit_label.call1((c,))?.extract::<String>(),
                    })
                    .collect::<PyResult<Vec<_>>>()?;
                let selected = self.codes[i].iter().position(|c| c == code).unwrap_or(0);
                rows.push(field(id, "choice", t(label)?, selected, labels));
                let fraction = i == 0 && self.fractional();
                let options = if fraction {
                    fractions.clone()
                } else {
                    decimals.clone()
                };
                let value = self.draft[*precision].as_u64().unwrap_or(*fallback) as usize;
                let mut row = field(
                    precision,
                    "choice",
                    t(if fraction {
                        "Smallest fraction"
                    } else {
                        "Decimals"
                    })?,
                    value.min(options.len().saturating_sub(1)),
                    options,
                );
                if fraction {
                    row.9 = t("The smallest fraction of an inch shown")?;
                }
                if i == 1 && code == "auto" {
                    row.8 = DISABLED;
                }
                rows.push(row);
            }
            rows.push(field(
                "decimal",
                "choice",
                t("Decimal separator:")?,
                usize::from(self.draft["decimal"] == ","),
                vec![t("Point (1.25)")?, t("Comma (1,25)")?],
            ));
        }
        let mut sample = field("preview", "text", t("Preview:")?, 0, vec![]);
        sample.3 = preview.into();
        rows.push(sample);
        if self.custom {
            rows.push(field(
                "for_new",
                "check",
                t("Also use for new documents")?,
                self.for_new.into(),
                vec![],
            ));
            rows.push(field(
                "ask_again",
                "check",
                t("Ask which units to use at startup")?,
                self.ask_again.into(),
                vec![],
            ));
        } else {
            rows.push(field(
                "dont_ask",
                "check",
                t("Don't show this at startup")?,
                (!self.ask_again).into(),
                vec![],
            ));
        }
        Ok(rows)
    }
}

impl UnitsDraft {
    fn fractional(&self) -> bool {
        self.draft["length"]
            .as_str()
            .is_some_and(|s| s.ends_with("-frac"))
    }
}
