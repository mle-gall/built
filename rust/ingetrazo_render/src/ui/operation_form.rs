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

enum Options {
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
    step: u32,
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

#[pymethods]
impl OperationForm {
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
                    number_row(
                        "angle",
                        "Angle between faces",
                        &self.number,
                        (0., 180.),
                        "°",
                        1,
                        flags,
                    ),
                    (
                        "slider".into(),
                        "slider".into(),
                        String::new(),
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
            Options::Soften { harden: false, .. } if self.angle().is_none() => Some((0, 180)),
            Options::Image { .. } if self.width().is_none() => Some((640, 16384)),
            _ => None,
        }
    }

    fn values(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let values = PyDict::new(py);
        match self.options {
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
        assert_eq!(model.rows()[1].4, 274.);
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
