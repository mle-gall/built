//! Current-view and Composer-frame editions of the one-off SVG export.
//! Dimensions, guide and face keys use the existing export_svg/v5 preferences.
use super::{check, number, Field, SvgOptions, FACES};
use pyo3::prelude::*;
use serde_json::{json, Value};

const FILL_KEYS: [&str; 5] = ["none", "view", "full", "colours", "default"];
const GUIDE_KEYS: [&str; 6] = ["none", "thirds", "golden", "grid", "diagonals", "centre"];
const AREAS: [&str; 3] = ["view", "model", "selection"];

impl View {
    pub(super) fn can_frame(&self) -> bool {
        self.advanced && self.frame_scale.is_none() && self.area == 0
    }
}

pub(super) struct View {
    pub advanced: bool,
    area: usize,
    scene: usize,
    scenes: Vec<String>,
    selection: bool,
    axes: bool,
    shadows: bool,
    frame_scale: Option<f64>,
    pub(super) preview_page: Option<(f64, f64, String)>,
    lock: Option<f64>,
    switch: Option<Py<PyAny>>,
}

fn choice(id: &str, title: &str, names: &[&str], index: usize) -> Field {
    (
        id.into(),
        "choice".into(),
        title.into(),
        String::new(),
        index as f64,
        0.,
        names.len() as f64,
        names.iter().map(|s| s.to_string()).collect(),
        0,
        String::new(),
    )
}

fn index(keys: &[&str], value: &Value, default: usize) -> usize {
    keys.iter().position(|key| value == key).unwrap_or(default)
}

fn flag(value: &Value, default: bool) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::String(s) => !matches!(s.to_lowercase().as_str(), "0" | "false"),
        Value::Number(n) => n.as_u64() != Some(0),
        _ => default,
    }
}

fn dimension(v: &Value, default: f64) -> f64 {
    v.as_f64()
        .or_else(|| v.as_str()?.parse().ok())
        .filter(|n| (1.0..=10000.).contains(n))
        .unwrap_or(default)
}

impl SvgOptions {
    pub(super) fn view_rows(&self, v: &View) -> Vec<Field> {
        let mut rows = vec![check("advanced", "Advanced options", v.advanced)];
        if !v.scenes.is_empty() {
            let names: Vec<_> = std::iter::once("Current view")
                .chain(v.scenes.iter().map(String::as_str))
                .collect();
            rows.push(choice("svg_scene", "Scene", &names, v.scene));
        }
        if v.advanced {
            let areas = ["Current view", "Whole model", "Selection"];
            if v.frame_scale.is_none() {
                rows.push(choice(
                    "area",
                    "Drawing area",
                    &areas[..if v.selection { 3 } else { 2 }],
                    v.area,
                ));
            }
            let mut common = self.delivery_fields();
            common.retain(|r| match r.0.as_str() {
                "scale" | "size" => false,
                "axes" => v.axes,
                "shadows" => v.shadows,
                "framed" | "pan_x" | "pan_y" | "zoom" => v.area == 0 && v.frame_scale.is_none(),
                _ => true,
            });
            rows.extend(common);
        } else {
            rows.push(choice(
                "basic_fill",
                "Faces",
                &FACES[1..],
                self.fill.saturating_sub(1),
            ));
        }
        let mut modes = vec!["Screen size", "Custom paper"];
        if v.advanced && v.frame_scale.is_some() {
            modes.push("Frame scale");
        }
        let mode = match self.paper.mode {
            2 => 1,
            0 => 2,
            _ => 0,
        };
        let mut size = choice("view_size", "Paper size", &modes, mode);
        if modes.len() == 3 {
            size.7[2] = format!("{} (1 ∶ {})", modes[2], v.frame_scale.unwrap());
        }
        let at = rows
            .iter()
            .position(|r| matches!(r.0.as_str(), "paper_preset" | "framed"))
            .unwrap_or(rows.len());
        rows.insert(at, size);
        if !v.advanced && self.paper.mode == 2 {
            rows.push(check("basic_lock", "Lock proportions", v.lock.is_some()));
        }
        if self.paper.mode == 1 || (!v.advanced && self.paper.mode == 2) {
            for (i, (id, label)) in [("width", "Paper width"), ("height", "Paper height")]
                .into_iter()
                .enumerate()
            {
                let screen = self.paper.mode == 1;
                let n = if screen {
                    self.paper.screen[i].round()
                } else {
                    self.paper.size[i]
                };
                let mut row = number(id, label, n, [1., 10000.], self.paper.editing[i].as_ref());
                row.7 = vec![" mm".into()];
                if screen {
                    row.8 = crate::ui::dialog::DISABLED;
                }
                rows.push(row);
            }
        }
        rows
    }

    pub(super) fn edit_view(&mut self, id: &str, raw: &str) -> bool {
        let Some(v) = self.view.as_mut() else {
            return false;
        };
        let choice = raw.parse::<usize>().ok();
        let on = matches!(raw, "1" | "true" | "True");
        match id {
            "advanced" => {
                if on != v.advanced {
                    v.advanced = on;
                    v.area = 0;
                    if !on {
                        if self.fill == 0 {
                            self.fill = 2;
                        }
                        if self.paper.mode == 0 {
                            self.paper.mode = 2;
                        }
                        self.preview.update(None);
                    }
                    if self.paper.mode == 2 {
                        self.paper.preset = 0;
                        self.paper.ratio = 0;
                        if v.lock.is_some() {
                            v.lock = Some(self.paper.size[0] / self.paper.size[1]);
                        }
                    }
                }
            }
            "basic_fill" => {
                if let Some(i @ 0..=3) = choice {
                    self.fill = i + 1;
                }
            }
            "view_size" => {
                if let Some(i @ 0..=2) = choice {
                    if i < 2 || (v.advanced && v.frame_scale.is_some()) {
                        self.paper.change("size", ["1", "2", "0"][i]);
                        if !v.advanced && i == 1 {
                            self.paper.ratio = 0;
                            if v.lock.is_some() {
                                v.lock = Some(self.paper.size[0] / self.paper.size[1]);
                            }
                        }
                    }
                }
            }
            "area" => {
                if let Some(i @ 0..=2) = choice {
                    if i != 2 || v.selection {
                        v.area = i;
                    }
                }
            }
            "svg_scene" => {
                if let Some(i) = choice.filter(|i| *i <= v.scenes.len()) {
                    v.scene = i;
                    self.frame = [0., 0., 1.];
                    self.framed = false;
                }
            }
            "basic_lock" => v.lock = on.then(|| self.paper.size[0] / self.paper.size[1]),
            "width" | "height" if !v.advanced => {
                if self.paper.mode == 2 {
                    self.paper.change(id, raw);
                    if let Some(r) = v.lock {
                        if self.paper.validation_error().is_none() {
                            let [w, h] = self.paper.size;
                            self.paper.editing[usize::from(id == "width")] = None;
                            self.paper.size = if id == "width" {
                                [w, (w / r).round().clamp(1., 10000.)]
                            } else {
                                [(h * r).round().clamp(1., 10000.), h]
                            };
                        }
                    }
                }
            }
            _ => return false,
        }
        true
    }

    pub(super) fn view_result(&self, v: &View) -> Value {
        let style = self.record();
        let faces = match self.fill {
            0 => Value::Null,
            1 if v.axes && !self.axes => json!({"image":true,"axes":false}),
            1 => json!({"image":true}),
            _ => {
                let mut faces = style["faces"].clone();
                faces["shadows"] = (v.shadows && self.shadows).into();
                faces
            }
        };
        let mut paper = json!({"area":AREAS[v.area], "size":style["paper"]});
        if v.advanced && v.area == 0 && self.framed && v.frame_scale.is_none() {
            paper["frame"] = json!(self.frame);
        }
        json!({"faces":faces,"paper":paper,"scene":v.scene})
    }

    pub(super) fn switch_scene(&self, py: Python<'_>) -> PyResult<()> {
        if let Some(v) = &self.view {
            if let Some(switch) = &v.switch {
                switch.call1(py, (v.scene.checked_sub(1),))?;
            }
        }
        Ok(())
    }
}

impl SvgOptions {
    pub(super) fn from_view(
        preferences: &str,
        screen: [f64; 2],
        axes: bool,
        shadows: bool,
        selection: bool,
        scenes: Vec<String>,
        frame_scale: Option<f64>,
        switch: Option<Py<PyAny>>,
    ) -> PyResult<Self> {
        let p: Value = serde_json::from_str(preferences)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        let advanced = flag(&p["advanced"], false);
        let prefix = if advanced { "" } else { "basic_" };
        let key = |k: &str| format!("{prefix}{k}");
        let size = match p[key("size")].as_str() {
            Some("custom") => json!([
                dimension(&p[key("width")], screen[0]),
                dimension(&p[key("height")], screen[1])
            ]),
            Some("scale") if advanced && frame_scale.is_some() => Value::Null,
            _ => json!("screen"),
        };
        let ratio = match p["ratio"].as_str() {
            Some("free") => "Free",
            Some("screen") => "Screen",
            Some(r) if advanced => r,
            None if advanced => "Screen",
            _ => "Free",
        };
        let mut s = Self::restore(
            &json!({"paper":size,"paper_layout":{"preset":p["sheet"],"ratio":ratio}}),
            screen,
        );
        s.fill = index(&FILL_KEYS, &p[key("faces")], 1);
        if !advanced && s.fill == 0 {
            s.fill = 2;
        }
        s.axes = !flag(&p["hide_axes"], true);
        s.shadows = flag(&p["shadows"], true);
        s.lighting = flag(&p["as_view"], true);
        s.guide = index(&GUIDE_KEYS, &p["guides"], 1);
        s.preview.enabled = flag(&p["preview"], true);
        let area = if advanced && frame_scale.is_none() {
            index(&AREAS, &p["area"], 0)
        } else {
            0
        };
        s.view = Some(View {
            advanced,
            area: if area == 2 && !selection { 0 } else { area },
            scene: 0,
            scenes,
            selection,
            axes,
            shadows,
            frame_scale,
            preview_page: None,
            lock: flag(&p["basic_lock"], true).then(|| s.paper.size[0] / s.paper.size[1]),
            switch,
        });
        Ok(s)
    }

    pub(super) fn saved_view_preferences(&self, v: &View) -> String {
        let layout = self.paper.options();
        let [w, h] = self.paper.size;
        let size = ["scale", "screen", "custom"][self.paper.mode];
        let basic_fill = if self.fill == 0 { 2 } else { self.fill };
        json!({
            "advanced": v.advanced,
            "faces": FILL_KEYS[self.fill],
            "basic_faces": FILL_KEYS[basic_fill],
            "preview": self.preview.enabled,
            "guides": GUIDE_KEYS[self.guide],
            "area": AREAS[v.area],
            "as_view": self.lighting,
            "shadows": self.shadows,
            "hide_axes": !self.axes,
            "size": size,
            "basic_size": if size == "scale" {"custom"} else {size},
            "turn": if self.paper.portrait {"portrait"} else {"landscape"},
            "sheet": layout["paper_layout"]["preset"].as_str().unwrap_or("none"),
            "ratio": layout["paper_layout"]["ratio"].as_str().unwrap_or("free").to_lowercase(),
            "width": w, "height": h, "basic_width": w, "basic_height": h,
            "basic_lock": v.lock.is_some()
        })
        .to_string()
    }
}
