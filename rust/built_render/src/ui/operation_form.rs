//! Options for one-shot modeling and export commands shown by the chrome.
use super::dialog::{Field, DISABLED};
use pyo3::{prelude::*, types::PyDict};

const UNITS: &[(&str, &str)] = &[
    ("m", "Metres"),
    ("cm", "Centimetres"),
    ("mm", "Millimetres"),
    ("in", "Inches"),
    ("ft", "Feet"),
];
const SIMPLIFY: &[(&str, &str)] = &[
    ("none", "No mesh simplification"),
    (
        "principal",
        "Merge coplanar triangles on XY, XZ and YZ planes",
    ),
    ("all", "Advanced: merge all coplanar surfaces"),
];

fn choice(id: &str, label: &str, index: usize, items: &[(&str, &str)]) -> Field {
    (
        id.into(),
        "choice".into(),
        label.into(),
        String::new(),
        index as f64,
        0.,
        items.len() as f64,
        items.iter().map(|(_, text)| (*text).into()).collect(),
        0,
        String::new(),
    )
}

const VIDEO_SIZES: [(u32, u32); 4] = [(1280, 720), (1920, 1080), (2560, 1440), (3840, 2160)];
const VIDEO_RATES: [u32; 4] = [24, 25, 30, 60];

enum Options {
    Survey {
        northern: bool,
    },
    Animation {
        enabled: bool,
        transition: String,
        valid_transition: f64,
        pause: String,
    },
    Video {
        view: (u32, u32),
        scenes: usize,
        seconds: f64,
        size: usize,
        view_label: String,
        duration_label: String,
        rate: usize,
    },
    MeshImport {
        unit: usize,
        simplify: Option<usize>,
    },
    Section {
        name: String,
        symbol: String,
        skip: Option<bool>,
    },
    Soften {
        smooth: bool,
        coplanar: bool,
        materials: bool,
        harden: bool,
    },
    Image {
        alpha_supported: bool,
        transparent: bool,
    },
}

#[pyclass]
pub struct OperationForm {
    options: Options,
    number: String,
}

fn number_row(
    id: &str,
    label: &str,
    raw: &str,
    range: (f64, f64),
    suffix: &str,
    step: impl ToString,
    flags: u32,
) -> Field {
    (
        id.into(),
        "number".into(),
        label.into(),
        raw.into(),
        0.,
        range.0,
        range.1,
        vec![suffix.into(), step.to_string()],
        flags,
        String::new(),
    )
}

fn check(id: &str, label: &str, on: bool, flags: u32, tip: &str) -> Field {
    (
        id.into(),
        "check".into(),
        label.into(),
        String::new(),
        f64::from(on),
        0.,
        1.,
        vec![],
        flags,
        tip.into(),
    )
}

fn seconds(value: &str) -> Option<f64> {
    value
        .trim()
        .replace(',', ".")
        .parse()
        .ok()
        .filter(|x| (0. ..=60.).contains(x))
}

#[pymethods]
impl OperationForm {
    #[staticmethod]
    fn survey() -> Self {
        Self {
            options: Options::Survey { northern: false },
            number: "18".into(),
        }
    }
    #[staticmethod]
    fn animation(enabled: bool, transition: f64, pause: f64) -> Self {
        Self {
            number: String::new(),
            options: Options::Animation {
                enabled,
                transition: transition.to_string(),
                valid_transition: transition,
                pause: pause.to_string(),
            },
        }
    }
    #[staticmethod]
    #[pyo3(signature=(width, height, scenes, seconds, view_label="Like the view ({w} × {h})", duration_label="{scenes} scenes, {seconds} s ({frames} frames)"))]
    fn video(
        width: u32,
        height: u32,
        scenes: usize,
        seconds: f64,
        view_label: &str,
        duration_label: &str,
    ) -> Self {
        Self {
            number: String::new(),
            options: Options::Video {
                view: (width.max(2) & !1, height.max(2) & !1),
                scenes,
                seconds,
                size: 1,
                view_label: view_label.into(),
                duration_label: duration_label.into(),
                rate: 2,
            },
        }
    }

    #[staticmethod]
    #[pyo3(signature = (unit, simplify=None))]
    fn mesh_import(unit: &str, simplify: Option<&str>) -> Self {
        let fallback_unit = if simplify.is_some() { 2 } else { 0 };
        Self {
            options: Options::MeshImport {
                unit: UNITS
                    .iter()
                    .position(|x| x.0 == unit)
                    .unwrap_or(fallback_unit),
                simplify: simplify.map(|key| SIMPLIFY.iter().position(|x| x.0 == key).unwrap_or(1)),
            },
            number: String::new(),
        }
    }

    #[staticmethod]
    fn soften(saved: &str) -> Self {
        let angle = saved
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .unwrap_or(30.)
            .clamp(0., 180.);
        Self {
            options: Options::Soften {
                smooth: true,
                coplanar: true,
                materials: true,
                harden: false,
            },
            number: format!("{angle:.1}"),
        }
    }

    #[staticmethod]
    fn section(name: String, symbol: String, automatic: bool) -> Self {
        Self {
            options: Options::Section {
                name,
                symbol,
                skip: automatic.then_some(false),
            },
            number: String::new(),
        }
    }

    #[staticmethod]
    fn image(suffix: &str) -> Self {
        Self {
            options: Options::Image {
                alpha_supported: suffix.eq_ignore_ascii_case(".png"),
                transparent: false,
            },
            number: "3840".into(),
        }
    }

    fn rows(&self) -> Vec<Field> {
        match self.options {
            Options::Survey { northern } => vec![
                number_row("zone", "UTM zone", &self.number, (1., 60.), "", 1, 0),
                choice(
                    "hemisphere",
                    "Hemisphere:",
                    usize::from(northern),
                    &[("south", "South"), ("north", "North")],
                ),
            ],
            Options::Animation {
                enabled,
                ref transition,
                ref pause,
                ..
            } => vec![
                check(
                    "enabled",
                    "Glide from scene to scene",
                    enabled,
                    0,
                    "Off: showing a scene jumps straight to it, as it always did.",
                ),
                number_row(
                    "transition",
                    "Scene transition:",
                    transition,
                    (0., 60.),
                    " s",
                    0.5,
                    if enabled { 0 } else { DISABLED },
                ),
                number_row("pause", "Scene delay:", pause, (0., 60.), " s", 0.5, 0),
            ],
            Options::Video {
                view,
                scenes,
                seconds,
                size,
                ref view_label,
                ref duration_label,
                rate,
            } => {
                let mut sizes: Vec<String> = VIDEO_SIZES
                    .iter()
                    .map(|(w, h)| format!("{w} × {h}"))
                    .collect();
                sizes.push(
                    view_label
                        .replace("{w}", &view.0.to_string())
                        .replace("{h}", &view.1.to_string()),
                );
                let rates = VIDEO_RATES.iter().map(u32::to_string).collect();
                let row = |id: &str, label: &str, at: usize, options: Vec<String>| {
                    (
                        id.into(),
                        "choice".into(),
                        label.into(),
                        String::new(),
                        at as f64,
                        0.,
                        options.len() as f64,
                        options,
                        0,
                        String::new(),
                    )
                };
                let frames = (seconds * f64::from(VIDEO_RATES[rate])).round_ties_even() as u64 + 1;
                vec![
                    row("size", "Video size:", size, sizes),
                    row("fps", "Frames per second:", rate, rates),
                    (
                        "duration".into(),
                        "text".into(),
                        "Duration:".into(),
                        duration_label
                            .replace("{scenes}", &scenes.to_string())
                            .replace("{seconds}", &format!("{seconds:.1}"))
                            .replace("{frames}", &frames.to_string()),
                        0.,
                        0.,
                        0.,
                        vec![],
                        0,
                        String::new(),
                    ),
                ]
            }
            Options::MeshImport { unit, simplify } => {
                let label = if simplify.is_some() {
                    "An STL file does not record its unit. What is this model in?"
                } else {
                    "An OBJ file does not record its unit. What is this model in?"
                };
                let mut rows = vec![choice("unit", label, unit, UNITS)];
                if let Some(mode) = simplify {
                    rows.push(choice("simplify", "Mesh simplification", mode, SIMPLIFY));
                }
                rows
            }
            Options::Section {
                ref name,
                ref symbol,
                skip,
            } => {
                let edit = |id: &str, label: &str, value: &String| {
                    (
                        id.into(),
                        "edit".into(),
                        label.into(),
                        value.clone(),
                        0.,
                        0.,
                        0.,
                        vec![],
                        0,
                        String::new(),
                    )
                };
                let mut rows = vec![edit("name", "Name", name), edit("symbol", "Symbol", symbol)];
                if let Some(checked) = skip {
                    rows.push(check(
                        "skip",
                        "Don't ask again — keep the default name and symbol",
                        checked,
                        0,
                        "",
                    ));
                }
                rows
            }
            Options::Soften {
                smooth,
                coplanar,
                materials,
                harden,
            } => {
                let flags = if harden { DISABLED } else { 0 };
                let angle = self.angle().unwrap_or(0.);
                vec![
                    (
                        "slider".into(),
                        "slider".into(),
                        "Angle between faces".into(),
                        format!("{angle:.1}°"),
                        angle * 10.,
                        0.,
                        1800.,
                        vec![],
                        flags,
                        String::new(),
                    ),
                    check("smooth", "Smooth normals", smooth, flags, ""),
                    check("coplanar", "Soften coplanar edges", coplanar, flags, ""),
                    check(
                        "materials",
                        "Soften between different materials",
                        materials,
                        flags,
                        "",
                    ),
                    check("harden", "Unsoften all selected edges", harden, 0, ""),
                ]
            }
            Options::Image {
                alpha_supported,
                transparent,
            } => vec![
                number_row(
                    "width",
                    "Width in pixels:",
                    &self.number,
                    (640., 16384.),
                    "",
                    320,
                    0,
                ),
                check(
                    "transparent",
                    "Transparent background",
                    transparent,
                    if alpha_supported { 0 } else { DISABLED },
                    if alpha_supported {
                        ""
                    } else {
                        "Only PNG images keep transparency."
                    },
                ),
            ],
        }
    }

    fn receive(&mut self, id: &str, raw: &str) {
        let on = raw == "1";
        match &mut self.options {
            Options::Survey { northern } => match id {
                "zone" => self.number = raw.into(),
                "hemisphere" if raw == "0" || raw == "1" => *northern = raw == "1",
                _ => (),
            },
            Options::Animation {
                enabled,
                transition,
                valid_transition,
                pause,
            } => match id {
                "enabled" => {
                    *enabled = on;
                    if !on && seconds(transition).is_none() {
                        *transition = valid_transition.to_string();
                    }
                }
                "transition" if *enabled => {
                    *transition = raw.into();
                    if let Some(value) = seconds(raw) {
                        *valid_transition = value;
                    }
                }
                "pause" => *pause = raw.into(),
                _ => (),
            },
            Options::Video { size, rate, .. } => {
                if let Ok(n) = raw.parse::<usize>() {
                    match id {
                        "size" if n <= VIDEO_SIZES.len() => *size = n,
                        "fps" if n < VIDEO_RATES.len() => *rate = n,
                        _ => (),
                    }
                }
            }
            Options::MeshImport { unit, simplify } => {
                if let Ok(index) = raw.parse::<usize>() {
                    match id {
                        "unit" if index < UNITS.len() => *unit = index,
                        "simplify" if simplify.is_some() && index < SIMPLIFY.len() => {
                            *simplify = Some(index)
                        }
                        _ => (),
                    }
                }
            }
            Options::Section { name, symbol, skip } => match id {
                "name" => *name = raw.into(),
                "symbol" => *symbol = raw.chars().take(3).collect(),
                "skip" if skip.is_some() => *skip = Some(on),
                _ => (),
            },
            Options::Soften {
                smooth,
                coplanar,
                materials,
                harden,
            } => match id {
                "harden" => *harden = on,
                _ if *harden => (),
                "angle" => self.number = raw.into(),
                "slider" => {
                    if let Ok(n) = raw.parse::<u16>() {
                        self.number = format!("{:.1}", n.min(1800) as f64 / 10.);
                    }
                }
                "smooth" => *smooth = on,
                "coplanar" => *coplanar = on,
                "materials" => *materials = on,
                _ => (),
            },
            Options::Image {
                alpha_supported,
                transparent,
            } => match id {
                "width" => self.number = raw.into(),
                "transparent" if *alpha_supported => *transparent = on,
                _ => (),
            },
        }
    }

    fn invalid_range(&self) -> Option<(u32, u32)> {
        match self.options {
            Options::Survey { .. }
                if self
                    .number
                    .trim()
                    .parse::<u8>()
                    .ok()
                    .filter(|n| (1..=60).contains(n))
                    .is_none() =>
            {
                Some((1, 60))
            }
            Options::Animation {
                ref transition,
                ref pause,
                ..
            } if seconds(transition).is_none() || seconds(pause).is_none() => Some((0, 60)),
            Options::Soften { harden: false, .. } if self.angle().is_none() => Some((0, 180)),
            Options::Image { .. } if self.width().is_none() => Some((640, 16384)),
            _ => None,
        }
    }

    fn values(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let values = PyDict::new(py);
        match self.options {
            Options::Survey { northern } => {
                values.set_item("zone", self.number.trim().parse::<u8>().ok())?;
                values.set_item("northern", northern)?;
            }
            Options::Animation {
                enabled,
                ref transition,
                ref pause,
                ..
            } => {
                values.set_item("enabled", enabled)?;
                values.set_item("transition", seconds(transition))?;
                values.set_item("pause", seconds(pause))?;
            }
            Options::Video {
                view, size, rate, ..
            } => {
                let (w, h) = VIDEO_SIZES.get(size).copied().unwrap_or(view);
                values.set_item("width", w)?;
                values.set_item("height", h)?;
                values.set_item("fps", VIDEO_RATES[rate])?;
            }
            Options::MeshImport { unit, simplify } => {
                values.set_item("unit", UNITS[unit].0)?;
                if let Some(index) = simplify {
                    values.set_item("simplify", SIMPLIFY[index].0)?;
                }
            }
            Options::Section {
                ref name,
                ref symbol,
                skip,
            } => {
                values.set_item("name", name.trim())?;
                values.set_item("symbol", symbol.trim())?;
                values.set_item("skip", skip.unwrap_or(false))?;
            }
            Options::Soften {
                smooth,
                coplanar,
                materials,
                harden,
            } => {
                values.set_item("angle", self.angle())?;
                for (id, on) in [
                    ("smooth", smooth),
                    ("coplanar", coplanar),
                    ("materials", materials),
                    ("harden", harden),
                ] {
                    values.set_item(id, on)?;
                }
            }
            Options::Image { transparent, .. } => {
                values.set_item("width", self.width())?;
                values.set_item("transparent", transparent)?;
            }
        }
        Ok(values.unbind())
    }
}

impl OperationForm {
    fn angle(&self) -> Option<f64> {
        self.number
            .trim()
            .replace(',', ".")
            .parse::<f64>()
            .ok()
            .filter(|v| (0.0..=180.).contains(v))
            .map(|v| (v * 10.).round() / 10.)
    }

    fn width(&self) -> Option<u32> {
        self.number
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|v| (640..=16384).contains(v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angle_tracks_the_slider_and_rejects_invalid_text_until_corrected() {
        let mut model = OperationForm::soften("invalid setting");
        assert_eq!(model.angle(), Some(30.));
        model.receive("slider", "113");
        assert_eq!(model.angle(), Some(11.3));
        model.receive("angle", "27,4");
        assert_eq!(model.rows()[0].4, 274.);
        assert_eq!(model.rows().iter().filter(|row| row.0 == "angle").count(), 0);
        model.receive("angle", "181");
        assert!(model.invalid_range().is_some());
        model.receive("harden", "1");
        assert_eq!(model.invalid_range(), None);
    }

    #[test]
    fn section_symbols_hold_three_characters_and_only_placement_offers_to_skip() {
        let mut placement = OperationForm::section("Section 1".into(), "A".into(), true);
        placement.receive("symbol", "É三四5");
        placement.receive("skip", "1");
        assert_eq!(placement.rows()[1].3, "É三四");
        assert_eq!(placement.rows()[2].4, 1.);
        let mut rename = OperationForm::section("Section 1".into(), "A".into(), false);
        rename.receive("skip", "1");
        assert_eq!(rename.rows().len(), 2);
    }

    #[test]
    fn image_accepts_only_integer_widths_in_the_export_range() {
        let mut model = OperationForm::image(".PNG");
        for raw in ["", "1.5", "-1", "639", "16385"] {
            model.receive("width", raw);
            assert_eq!(model.invalid_range(), Some((640, 16384)));
        }
        for raw in ["640", "16384"] {
            model.receive("width", raw);
            assert_eq!(model.invalid_range(), None);
        }
    }
}
