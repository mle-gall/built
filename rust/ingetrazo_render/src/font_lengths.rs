//! Length drafts for extruded text. Use the document's existing expression
//! and fractional-unit parser for exactly the input accepted by modelling tools.
use pyo3::{exceptions::PyValueError, prelude::*};

#[pyfunction]
pub fn font_length(py: Python<'_>, text: &str, unit: &str, low: f64, high: f64) -> PyResult<f64> {
    let units = py.import("core.units")?;
    let parsed = units.call_method1("parse", (text, "length", unit))?;
    if !parsed.getattr("ok")?.is_truthy()? {
        return Err(PyValueError::new_err(
            parsed.call_method0("message")?.extract::<String>()?,
        ));
    }
    let value = units
        .call_method1("length_number", (parsed.getattr("value")?, unit))?
        .extract::<f64>()?;
    if !value.is_finite() || value < low || value > high {
        return Err(PyValueError::new_err(format!(
            "Enter a length between {low} and {high} {unit}."
        )));
    }
    Ok(value)
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct FontLengths {
    pub unit: String,
    pub text: [String; 2],
    #[serde(skip)]
    errors: [String; 2],
}

impl FontLengths {
    pub fn edit(&mut self, index: usize, ui: &mut egui::Ui, value: &mut f64, min: f64, max: f64) {
        if ui
            .add(crate::ui::kit::field(&mut self.text[index]).desired_width(115.))
            .changed()
        {
            Python::attach(
                |py| match font_length(py, &self.text[index], &self.unit, min, max) {
                    Ok(v) => {
                        *value = v;
                        self.errors[index].clear();
                    }
                    Err(e) => self.errors[index] = e.value(py).to_string(),
                },
            );
        }
    }

    pub fn error(&self) -> Option<&str> {
        self.errors
            .iter()
            .find(|s| !s.is_empty())
            .map(String::as_str)
    }
}
