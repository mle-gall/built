//! Device preferences shown in the Settings sheet. Python supplies the driver
//! and persists its existing portable profile format.
use pyo3::{exceptions::PyValueError, prelude::*};
use serde_json::{json, Value};

const AXES: [&str; 6] = ["right", "up", "forward", "tilt", "spin", "roll"];
const AXIS_LABELS: [&str; 6] = [
    "Move left / right",
    "Move up / down",
    "Push / pull",
    "Tilt forward / back",
    "Twist left / right",
    "Roll left / right",
];
const ACTIONS: [&str; 6] = ["pan_x", "pan_y", "zoom", "orbit_x", "orbit_y", "disabled"];
const ACTION_LABELS: [&str; 6] = ["Pan X", "Pan Y", "Zoom", "Orbit X", "Orbit Y", "Disabled"];
const ORIGINAL: [&str; 6] = ["pan_x", "pan_y", "zoom", "orbit_y", "orbit_x", "disabled"];
const FREECAD: [&str; 6] = ["pan_x", "zoom", "pan_y", "orbit_y", "disabled", "orbit_x"];

fn decode(data: &str) -> PyResult<Value> {
    serde_json::from_str(data).map_err(|e| PyValueError::new_err(e.to_string()))
}

#[pyclass]
pub struct DevicePreferences {
    profile: Value,
    named: Value,
    #[pyo3(get)]
    selected: String,
    #[pyo3(get)]
    enabled: bool,
    #[pyo3(get)]
    locked: bool,
}

#[pymethods]
impl DevicePreferences {
    #[new]
    fn new(
        profile: &str,
        named: &str,
        preset: &str,
        name: &str,
        enabled: bool,
        locked: bool,
    ) -> PyResult<Self> {
        let profile = decode(profile)?;
        let named = decode(named)?;
        let original = AXES.iter().enumerate().all(|(i, axis)| {
            profile["mapping"][axis] == ORIGINAL[i]
                && profile["inversion"][axis] == false
                && profile["sensitivity"][axis].as_f64() == Some(1.)
        }) && profile["global_speed"].as_f64() == Some(1.);
        let selected = if named.get(name).is_some() {
            format!("named:{name}")
        } else if preset == "freecad" {
            "freecad".into()
        } else if preset == "original" || original {
            "original".into()
        } else {
            "custom".into()
        };
        Ok(Self {
            profile,
            named,
            selected,
            enabled,
            locked,
        })
    }

    fn profile(&self) -> String {
        self.profile.to_string()
    }

    /// Edits and reset share the same path as applying an imported preset.
    fn edit(&mut self, id: &str, value: &str) -> PyResult<()> {
        let v = decode(value)?;
        let bad = || PyValueError::new_err("Invalid 3D mouse setting");
        let percent = || -> PyResult<Value> {
            let n = v
                .as_f64()
                .filter(|n| (25.0..=400.0).contains(n))
                .ok_or_else(bad)?;
            Ok(json!(n / 100.0))
        };
        match id {
            "ndof_on" => self.enabled = v.as_bool().ok_or_else(bad)?,
            "ndof_lock" => self.locked = v.as_bool().ok_or_else(bad)?,
            "ndof_preset" => {
                let choice = v.as_str().ok_or_else(bad)?;
                if let Some(name) = choice.strip_prefix("named:") {
                    self.profile = self.named.get(name).ok_or_else(bad)?.clone();
                } else if matches!(choice, "freecad" | "original") {
                    let free = choice == "freecad";
                    for (i, axis) in AXES.iter().enumerate() {
                        self.profile["mapping"][axis] =
                            json!(if free { FREECAD[i] } else { ORIGINAL[i] });
                        self.profile["inversion"][axis] =
                            json!(free && matches!(*axis, "up" | "tilt"));
                        self.profile["sensitivity"][axis] = json!(1.);
                    }
                    self.profile["global_speed"] = json!(1.);
                } else if choice != "custom" {
                    return Err(bad());
                }
                self.selected = choice.into();
            }
            _ => {
                if id == "ndof_speed" {
                    self.profile["global_speed"] = percent()?;
                } else {
                    let (kind, axis) = id.rsplit_once('_').ok_or_else(bad)?;
                    if !AXES.contains(&axis) {
                        return Err(bad());
                    }
                    match kind {
                        "ndof_map" if v.as_str().is_some_and(|s| ACTIONS.contains(&s)) => {
                            self.profile["mapping"][axis] = v
                        }
                        "ndof_inv" if v.is_boolean() => self.profile["inversion"][axis] = v,
                        "ndof_speed" => self.profile["sensitivity"][axis] = percent()?,
                        _ => return Err(bad()),
                    }
                }
                self.selected = "custom".into();
            }
        }
        Ok(())
    }

    #[staticmethod]
    fn profile_name(name: &str, imported: bool) -> PyResult<String> {
        let name = name.trim();
        if matches!(name, "Original" | "FreeCAD-style" | "Custom") && imported {
            return Ok(format!("{name} (imported)"));
        }
        if !imported
            && (name.is_empty()
                || name.chars().count() > 80
                || matches!(name, "Original" | "FreeCAD-style" | "Custom"))
        {
            return Err(PyValueError::new_err(
                "Choose a different name (1–80 characters).",
            ));
        }
        Ok(name.into())
    }

    fn groups(&self, tr: &Bound<'_, PyAny>, driver: &str, live: (&str, &str)) -> PyResult<String> {
        let t = |s: &str| tr.call1((s,))?.extract::<String>();
        let mut presets = vec![
            json!(["original", t("Original")?]),
            json!(["freecad", t("FreeCAD-style")?]),
            json!(["custom", t("Custom")?]),
        ];
        if let Some(named) = self.named.as_object() {
            let mut names: Vec<_> = named.keys().collect();
            names.sort_by_key(|n| n.to_lowercase());
            presets.extend(
                names
                    .into_iter()
                    .map(|name| json!([format!("named:{name}"), name])),
            );
        }
        let percent = |n: &Value| (100. * n.as_f64().unwrap_or(1.)).round() as i64;
        let speed = |id: String, label: String, value: &Value| json!({"id":id,"kind":"number","label":label,"value":percent(value),"lo":25,"hi":400,"suffix":"%"});
        let status = if driver.is_empty() {
            t("No 3D mouse driver found. On Linux install and start «spacenavd»; on Windows the 3Dconnexion driver is enough. macOS is not supported yet.")?
        } else {
            t("Device driver found: {name}")?.replace("{name}", driver)
        };
        let mut device = vec![
            json!({"id":"ndof_on","kind":"check","label":t("Navigate with a 3D mouse (SpaceMouse)")?,"value":self.enabled,"default":true}),
            speed(
                "ndof_speed".into(),
                t("Speed")?,
                &self.profile["global_speed"],
            ),
            json!({"id":"ndof_lock","kind":"check","label":t("Pan and zoom only (no rotation — for drawing in plan)")?,"value":self.locked,"default":false}),
            json!({"id":"ndof_driver","kind":"text","label":t("Driver")?,"value":status}),
        ];
        if !driver.is_empty() {
            device.extend([
                json!({"id":"ndof_raw","kind":"text","label":t("Live reading — move the cap one way at a time:")?,"value":live.0}),
                json!({"id":"ndof_read","kind":"text","label":"","value":live.1}),
            ]);
        }
        device.push(json!({"id":"ndof_preset","kind":"choice","label":t("Axis mapping preset")?,"value":self.selected,"options":presets,"default":"original"}));
        let actions = ACTIONS
            .iter()
            .zip(ACTION_LABELS)
            .map(|(code, label)| Ok(json!([code, t(label)?])))
            .collect::<PyResult<Vec<_>>>()?;
        let mut axes = Vec::new();
        for (axis, label) in AXES.iter().zip(AXIS_LABELS) {
            axes.push(json!({"id":format!("ndof_map_{axis}"),"kind":"choice","label":t(label)?,"value":self.profile["mapping"][axis],"options":actions,"words":"axis"}));
            axes.push(json!({"id":format!("ndof_inv_{axis}"),"kind":"check","label":t("Invert")?,"value":self.profile["inversion"][axis],"words":t(label)?}));
            let mut row = speed(
                format!("ndof_speed_{axis}"),
                t("Speed")?,
                &self.profile["sensitivity"][axis],
            );
            row["words"] = json!(t(label)?);
            axes.push(row);
        }
        Ok(json!([["device", t("Device")?, device], ["axes", t("Axes")?, axes]]).to_string())
    }
}

#[pyclass]
#[derive(Default)]
pub struct DeviceReadings {
    raw: Option<[i32; 6]>,
    motion: Option<[f64; 5]>,
}

#[pymethods]
impl DeviceReadings {
    #[new]
    fn new() -> Self {
        Self::default()
    }
    fn raw(&mut self, values: [i32; 6]) {
        self.raw = Some(values);
    }
    fn motion(&mut self, values: [f64; 5]) {
        self.motion = Some(values);
    }
    fn text(&self, tr: &Bound<'_, PyAny>) -> PyResult<(String, String)> {
        let mut raw = "—".to_string();
        if let Some(values) = self.raw {
            raw = tr.call1(("Device:",))?.extract::<String>()?;
            for (name, value) in ["X", "Y", "Z", "RX", "RY", "RZ"].iter().zip(values) {
                raw.push_str(&format!("  {name} {value:+5}"));
            }
        }
        let mut read = "—".to_string();
        if let Some(values) = self.motion {
            read = tr.call1(("Built: right {r:+.2f}  up {u:+.2f}  forward {f:+.2f}  tilt {t:+.2f}  twist {s:+.2f}",))?.extract::<String>()?;
            for (name, value) in ["r", "u", "f", "t", "s"].iter().zip(values) {
                read = read.replace(&format!("{{{name}:+.2f}}"), &format!("{value:+.2}"));
            }
        }
        Ok((raw, read))
    }
}

/// One snapshot of a machine-settings page. Only the backend supplies the
/// installed languages, drivers, figures, and values from this host.
#[pyclass(frozen)]
pub struct PreferencePage {
    groups: Value,
}

#[pymethods]
impl PreferencePage {
    #[new]
    fn new(section: &str, snapshot: &str, tr: &Bound<'_, PyAny>) -> PyResult<Self> {
        use std::sync::OnceLock;
        static CATALOG: OnceLock<Value> = OnceLock::new();
        let catalog = CATALOG.get_or_init(|| {
            serde_json::from_str(include_str!("preference_rows.json"))
                .expect("valid Settings catalogue")
        });
        let groups = catalog[section]
            .as_array()
            .ok_or_else(|| PyValueError::new_err("Unknown preferences section"))?;
        let snapshot = decode(snapshot)?;
        let translate = |v: &Value| -> PyResult<Value> {
            let s = v.as_str().unwrap_or_default();
            if s.is_empty() {
                return Ok(json!(""));
            }
            Ok(json!(tr.call1((s,))?.extract::<String>()?))
        };
        let mut result = Vec::new();
        for group in groups {
            let mut rows = Vec::new();
            for schema in group[2].as_array().unwrap() {
                let id = schema["id"].as_str().unwrap();
                let Some(host) = snapshot[id].as_object() else {
                    continue;
                };
                let mut row = schema.as_object().unwrap().clone();
                for key in ["label", "tip", "suffix", "restart"] {
                    if let Some(v) = row.get_mut(key) {
                        *v = translate(v)?;
                    }
                }
                if let Some(options) = row.get_mut("options").and_then(Value::as_array_mut) {
                    for opt in options {
                        opt[1] = translate(&opt[1])?;
                    }
                }
                for (rule, output) in [("enabled_if", "enabled"), ("visible_if", "visible")] {
                    if let Some(cond) = row.remove(rule) {
                        let val = &snapshot[cond[0].as_str().unwrap()]["value"];
                        let matches = if cond[1] == "nonempty" {
                            val.as_str().is_some_and(|s| !s.is_empty())
                        } else {
                            *val == cond[1]
                        };
                        row.insert(output.into(), json!(matches));
                    }
                }
                row.extend(host.clone());
                rows.push(Value::Object(row));
            }
            result.push(json!([group[0], translate(&group[1])?, rows]));
        }
        Ok(Self {
            groups: Value::Array(result),
        })
    }

    fn groups(&self) -> String {
        self.groups.to_string()
    }

    fn edit(&self, id: &str, raw: &str) -> PyResult<String> {
        let invalid = || PyValueError::new_err(format!("Invalid setting: {id}"));
        let row = self
            .groups
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|g| g[2].as_array().unwrap())
            .find(|r| r["id"] == id)
            .ok_or_else(invalid)?;
        let v = decode(raw)?;
        match row["kind"].as_str() {
            Some("choice")
                if row["options"]
                    .as_array()
                    .is_some_and(|opts| opts.iter().any(|o| o[0] == v)) => {}
            Some("check") if v.is_boolean() => {}
            Some("path" | "edit") if v.is_string() => {}
            Some("number" | "slider") => {
                let n = v
                    .as_f64()
                    .filter(|x| {
                        x.is_finite()
                            && *x >= row["lo"].as_f64().unwrap_or(f64::MIN)
                            && *x <= row["hi"].as_f64().unwrap_or(f64::MAX)
                    })
                    .ok_or_else(invalid)?;
                if row["decimals"].as_u64().unwrap_or(0) == 0 {
                    return Ok(json!(n.round() as i64).to_string());
                }
            }
            _ => return Err(invalid()),
        }
        Ok(v.to_string())
    }
}

const SECTIONS: &[(&str, &str, &str, &str)] = &[
    ("general", "General", "mac", "Applies at once."),
    (
        "navigation",
        "Navigation",
        "mac",
        "Applies at once. Shift pans and ⌘ zooms whatever is chosen here.",
    ),
    (
        "display",
        "Display",
        "mac",
        "Applies at once, to every open project.",
    ),
    (
        "ndof",
        "3D mouse",
        "mac",
        "Applies at once. Editing an axis makes the mapping Custom.",
    ),
    (
        "import",
        "Import",
        "mac",
        "The answers the import dialogs start with; each still asks.",
    ),
    (
        "keyboard",
        "Keyboard",
        "mac",
        "Pick a command and change its keys. Changes apply at once.",
    ),
    (
        "units",
        "Units",
        "model",
        "Saved with this model. A number typed without a unit is in this unit.",
    ),
    (
        "dimensions",
        "Dimension style",
        "model",
        "Saved with this model: how its dimensions read.",
    ),
    (
        "location",
        "Location & north",
        "model",
        "Saved with this model: where it stands and where north lies.",
    ),
];

#[pyclass]
pub struct PreferencesView {
    #[pyo3(get)]
    section: String,
    #[pyo3(get, set)]
    query: String,
    #[pyo3(get, set)]
    kb_focus: Option<String>,
    #[pyo3(get, set)]
    note: Option<(String, String)>,
    focus: bool,
    typed: std::collections::HashMap<String, String>,
}

#[pymethods]
impl PreferencesView {
    #[new]
    fn new(section: &str) -> Self {
        let mut view = Self {
            section: "general".into(),
            query: String::new(),
            kb_focus: None,
            note: None,
            focus: false,
            typed: Default::default(),
        };
        view.goto(section, None);
        view
    }

    #[staticmethod]
    fn sections() -> Vec<(&'static str, &'static str, &'static str)> {
        SECTIONS
            .iter()
            .map(|&(id, label, scope, _)| (id, label, scope))
            .collect()
    }
    fn heading(&self) -> (&'static str, &'static str, &'static str) {
        let &(_, label, scope, sub) = SECTIONS.iter().find(|row| row.0 == self.section).unwrap();
        (label, scope, sub)
    }
    fn goto(&mut self, page: &str, shortcut: Option<String>) {
        if let Some(shortcut) = shortcut {
            self.section = "keyboard".into();
            self.kb_focus = Some(format!("sc:{shortcut}"));
            self.focus = true;
        } else {
            let id = match page {
                "3dmouse" | "mouse3d" => "ndof",
                "shortcuts" | "keys" => "keyboard",
                "dimension_style" | "dimstyle" => "dimensions",
                "north" => "location",
                id => id,
            };
            if SECTIONS.iter().any(|row| row.0 == id) {
                self.section = id.into();
            }
        }
        self.query.clear();
        self.note = None;
    }

    fn take_focus(&mut self) -> bool {
        std::mem::take(&mut self.focus)
    }
    fn type_number(&mut self, id: String, text: String) {
        self.typed.insert(id, text);
    }
    fn number_text(&mut self, id: &str, value: f64) -> Option<String> {
        let text = self.typed.get(id)?;
        if text
            .replace(',', ".")
            .parse::<f64>()
            .is_ok_and(|n| (n - value).abs() < 1e-9)
        {
            return Some(text.clone());
        }
        self.typed.remove(id);
        None
    }
}

#[pyclass]
#[derive(Default)]
pub struct RestartPreferences {
    changes: std::collections::BTreeMap<String, (String, String)>,
}

#[pymethods]
impl RestartPreferences {
    #[new]
    fn new() -> Self {
        Self::default()
    }
    fn record(&mut self, id: &str, label: &str, before: &str, value: &str) {
        let start = self.changes.get(id).map(|v| v.1.as_str()).unwrap_or(before);
        if start == value {
            self.changes.remove(id);
        } else if !self.changes.contains_key(id) {
            self.changes
                .insert(id.into(), (label.into(), before.into()));
        }
    }
    fn labels(&self) -> Vec<String> {
        self.changes.values().map(|v| v.0.clone()).collect()
    }
    fn clear(&mut self) {
        self.changes.clear();
    }
}
