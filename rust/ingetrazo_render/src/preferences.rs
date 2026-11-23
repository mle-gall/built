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
