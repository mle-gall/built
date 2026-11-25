//! Document settings mutations. The regular root attribute journal owns replay.
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

#[pyclass(module = "built_core")]
pub struct DocumentSettings {
    units: Option<Py<PyDict>>,
    style: Py<PyDict>,
}

#[pymethods]
impl DocumentSettings {
    #[staticmethod]
    fn model_units(values: &Bound<'_, PyDict>) -> PyResult<Self> {
        let py = values.py();
        let payload = PyDict::new(py);
        payload.set_item("units", values)?;
        let units = py
            .import("core.units")?
            .call_method1("model_units_of", (payload,))?;
        let style = PyDict::new(py);
        style.set_item("units", units.get_item("length")?)?;
        style.set_item("decimals", units.get_item("precision")?)?;
        Ok(Self {
            units: Some(units.cast_into::<PyDict>()?.unbind()),
            style: style.unbind(),
        })
    }

    #[staticmethod]
    fn annotation_font(
        py: Python<'_>,
        family: String,
        bold: bool,
        italic: bool,
        size: i32,
    ) -> PyResult<Self> {
        let style = PyDict::new(py);
        style.set_item("font_family", family)?;
        style.set_item("font_bold", bold)?;
        style.set_item("font_italic", italic)?;
        style.set_item("font_size", size.clamp(6, 28))?;
        Ok(Self {
            units: None,
            style: style.unbind(),
        })
    }

    #[staticmethod]
    fn dimension_style(key: &str, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let valid = match key {
            "font_size" => value.extract::<u8>().is_ok_and(|n| (6..=28).contains(&n)),
            "decimals" => value.extract::<u8>().is_ok_and(|n| n <= 6),
            "ends" => matches!(
                value.extract::<String>().as_deref(),
                Ok("arrow" | "tick" | "none")
            ),
            "units" => value
                .py()
                .import("core.units")?
                .getattr("UNIT_CHOICES")?
                .contains(value)?,
            "show_unit" => value.extract::<bool>().is_ok(),
            "color" => value.extract::<[u8; 3]>().is_ok(),
            "base_step_mm" => value
                .extract::<f64>()
                .is_ok_and(|n| n.is_finite() && n > 0.),
            _ => false,
        };
        if !valid {
            return Err(PyValueError::new_err(format!(
                "Invalid dimension style field or value: {key}"
            )));
        }
        let style = PyDict::new(value.py());
        style.set_item(key, value)?;
        Ok(Self {
            units: None,
            style: style.unbind(),
        })
    }

    fn apply(&self, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = scene.py();
        if let Some(units) = &self.units {
            scene.setattr("units", units.bind(py))?;
        }
        let style = scene
            .getattr("dimension_style")?
            .cast_into::<PyDict>()?
            .copy()?;
        style.update(self.style.bind(py).as_mapping())?;
        scene.setattr("dimension_style", style)?;
        Ok(())
    }
}
