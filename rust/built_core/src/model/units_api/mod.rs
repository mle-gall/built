//! Document binding and plugin interface for the shared units kernels.
use super::units_format as format;
use crate::units as parser;
use pyo3::{
    exceptions::{PyKeyError, PyTypeError, PyValueError},
    prelude::*,
    types::{PyDict, PyString, PyTuple},
};
mod readouts;
mod tables;
use readouts::*;

type Obj<'p> = Bound<'p, PyAny>;
fn module(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("built_core.units")
}
fn setting_store(py: Python<'_>) -> PyResult<Obj<'_>> {
    py.import("built_core.settings")?.call_method0("settings")
}
#[pyfunction]
fn bind_scene(py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    module(py)?.setattr("_SCENE", scene)
}
#[pyfunction]
fn model_units_of<'p>(scene: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let py = scene.py();
    let raw = if scene.is_instance_of::<PyDict>() {
        scene.call_method1("get", ("units",))?
    } else {
        py.import("builtins")?
            .call_method1("getattr", (scene, "units", py.None()))?
    };
    format::py_clean_units(&raw)
}
#[pyfunction]
fn model_units(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    model_units_of(&module(py)?.getattr("_SCENE")?)
}
#[pyfunction]
fn model_unit(py: Python<'_>) -> PyResult<String> {
    model_units(py)?.as_any().get_item("length")?.extract()
}
#[pyfunction]
fn model_precision(py: Python<'_>) -> PyResult<i64> {
    model_units(py)?.as_any().get_item("precision")?.extract()
}
#[pyfunction]
fn apply_units(scene: &Obj<'_>, units: &Obj<'_>) -> PyResult<()> {
    let chosen = format::py_clean_units(units)?;
    scene.setattr("units", &chosen)?;
    if let Some(style) = scene
        .getattr_opt("dimension_style")?
        .and_then(|s| s.cast_into::<PyDict>().ok())
    {
        style.set_item("units", chosen.as_any().get_item("length")?)?;
        style.set_item("decimals", chosen.as_any().get_item("precision")?)?;
    }
    Ok(())
}
#[pyfunction]
fn ask_at_start(py: Python<'_>) -> PyResult<bool> {
    Ok(setting_store(py)?
        .call_method1("value", ("units/ask_at_start", "1"))?
        .str()?
        .to_str()?
        != "0")
}
#[pyfunction]
fn set_ask_at_start(ask: &Obj<'_>) -> PyResult<()> {
    let store = setting_store(ask.py())?;
    store.call_method1(
        "setValue",
        (
            "units/ask_at_start",
            if ask.is_truthy()? { "1" } else { "0" },
        ),
    )?;
    store.call_method0("sync")?;
    Ok(())
}
#[pyfunction]
fn new_document_units(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    let raw =
        setting_store(py)?.call_method1("value", (module(py)?.getattr("NEW_DOCUMENT_KEY")?,))?;
    if raw.is_truthy()? {
        match py.import("json")?.call_method1("loads", (raw,)) {
            Ok(v) => return format::py_clean_units(&v),
            Err(e)
                if e.is_instance_of::<PyTypeError>(py) || e.is_instance_of::<PyValueError>(py) =>
            {
                ()
            }
            Err(e) => return Err(e),
        }
    }
    format::py_clean_units(py.None().bind(py))
}
#[pyfunction]
fn remember_new_document_units(units: &Obj<'_>) -> PyResult<()> {
    let py = units.py();
    let store = setting_store(py)?;
    let text = py
        .import("json")?
        .call_method1("dumps", (format::py_clean_units(units)?,))?;
    store.call_method1("setValue", (module(py)?.getattr("NEW_DOCUMENT_KEY")?, text))?;
    store.call_method0("sync")?;
    Ok(())
}
#[pyfunction]
fn unit_label(py: Python<'_>, code: &str) -> PyResult<String> {
    let m = module(py)?;
    let tr = py.import("built_core.i18n")?.getattr("tr")?;
    for name in ["UNIT_LABELS", "KIND_UNIT_NAMES"] {
        let labels = m.getattr(name)?;
        if labels.contains(code)? {
            let label: String = tr.call1((labels.get_item(code)?,))?.extract()?;
            return if name == "UNIT_LABELS" {
                Ok(label)
            } else {
                Ok(format!(
                    "{} ({})",
                    label,
                    m.getattr("UNITS")?.get_item(code)?.getattr("symbol")?
                ))
            };
        }
    }
    Ok(code.into())
}
#[pyfunction]
fn convert(value: &Obj<'_>, from_unit: &str, to_unit: &str) -> PyResult<f64> {
    let rows = module(value.py())?.getattr("UNITS")?;
    let a = rows.get_item(from_unit)?;
    let b = rows.get_item(to_unit)?;
    let ak = a.getattr("kind")?;
    let bk = b.getattr("kind")?;
    if !ak.eq(&bk)? {
        return Err(PyValueError::new_err(format!(
            "cannot convert {ak} to {bk}"
        )));
    }
    let v = to_float(value)?;
    if a.getattr("code")?.eq(b.getattr("code")?)? {
        Ok(v)
    } else {
        Ok(v * a.getattr("factor")?.extract::<f64>()? / b.getattr("factor")?.extract::<f64>()?)
    }
}
#[pyfunction]
fn bare_number_scale(py: Python<'_>) -> PyResult<f64> {
    let m = module(py)?;
    if m.getattr("_UNITLESS")?.is_truthy()? {
        Ok(1.)
    } else {
        m.getattr("_BARE_SCALE")?
            .call_method1("get", (model_unit(py)?, 1.))?
            .extract()
    }
}
#[pyclass(module = "built_core.units")]
struct Unitless {
    previous: Option<Py<PyAny>>,
}
#[pymethods]
impl Unitless {
    fn __enter__(&mut self, py: Python<'_>) -> PyResult<()> {
        let m = module(py)?;
        self.previous = Some(m.getattr("_UNITLESS")?.unbind());
        m.setattr("_UNITLESS", true)
    }
    fn __exit__(
        &mut self,
        py: Python<'_>,
        _ty: &Obj<'_>,
        _value: &Obj<'_>,
        _trace: &Obj<'_>,
    ) -> PyResult<()> {
        if let Some(prior) = self.previous.take() {
            module(py)?.setattr("_UNITLESS", prior)?
        }
        Ok(())
    }
    fn _recreate_cm(&self) -> Self {
        unitless_numbers()
    }
    fn __call__<'p>(slf: &Bound<'p, Self>, func: &Obj<'p>) -> PyResult<Obj<'p>> {
        slf.py()
            .import("contextlib")?
            .getattr("ContextDecorator")?
            .call_method1("__call__", (slf, func))
    }
}
#[pyfunction]
fn unitless_numbers() -> Unitless {
    Unitless { previous: None }
}
#[pyfunction(signature=(kind="length",unit=None))]
fn typed_context(py: Python<'_>, kind: &str, unit: Option<&str>) -> PyResult<(f64, bool)> {
    let m = module(py)?;
    let doc = if unit.unwrap_or("").is_empty() {
        model_unit(py)?
    } else {
        String::new()
    };
    let display = unit.filter(|u| !u.is_empty()).unwrap_or(&doc);
    let scale = match (kind, unit) {
        ("length", None) => bare_number_scale(py)?,
        ("length", Some(u)) if m.getattr("_BARE_SCALE")?.contains(u)? => {
            m.getattr("_BARE_SCALE")?.get_item(u)?.extract()?
        }
        _ => {
            let u = match unit {
                Some(u) => u,
                None => match kind {
                    "area" => format::py_area_unit_for(&doc),
                    "volume" => format::py_volume_unit_for(&doc),
                    "angle" => "deg",
                    _ => return Err(PyKeyError::new_err(kind.to_owned())),
                },
            };
            m.getattr("UNITS")?
                .get_item(u)?
                .getattr("factor")?
                .extract()?
        }
    };
    Ok((scale, m.getattr("_IMPERIAL_FORMS")?.contains(display)?))
}
#[pyfunction(signature=(text,kind="length",unit=None))]
fn parse<'p>(text: &Obj<'p>, kind: &str, unit: Option<&str>) -> PyResult<Obj<'p>> {
    parse_string(
        text.py(),
        text.cast::<PyString>()
            .ok()
            .map(|s| s.to_str())
            .transpose()?
            .unwrap_or(""),
        kind,
        unit,
    )
}
fn parse_string<'p>(
    py: Python<'p>,
    text: &str,
    kind: &str,
    unit: Option<&str>,
) -> PyResult<Obj<'p>> {
    let (bare, feet) = typed_context(py, kind, unit)?;
    module(py)?
        .getattr("Parsed")?
        .call1(parser::py_parse(text, kind, bare, feet))
}
#[pyfunction(signature=(text,kind="length",unit=None))]
fn parse_expression<'p>(text: &Obj<'p>, kind: &str, unit: Option<&str>) -> PyResult<Obj<'p>> {
    let raw = text
        .cast::<PyString>()
        .ok()
        .map(|s| s.to_str())
        .transpose()?
        .unwrap_or("");
    if raw.trim_start().starts_with('=') {
        parse_string(text.py(), raw, kind, unit)
    } else {
        parse_string(text.py(), &format!("={raw}"), kind, unit)
    }
}
#[pyfunction]
fn is_expression(text: &Obj<'_>) -> PyResult<bool> {
    Ok(text
        .cast::<PyString>()
        .ok()
        .map(|s| s.to_str())
        .transpose()?
        .is_some_and(|s| s.trim_start().starts_with('=')))
}
#[pyfunction]
fn parse_length_field<'p>(field: &Obj<'p>) -> PyResult<Obj<'p>> {
    parse(field, "length", None)?.getattr("value")
}
#[pyfunction(signature=(buffer,comma_lists=false))]
fn parse_value_box<'p>(py: Python<'p>, buffer: &str, comma_lists: bool) -> PyResult<Obj<'p>> {
    let (bare, feet) = typed_context(py, "length", None)?;
    parser::py_parse_value_buffer(py, buffer, comma_lists, bare, feet)
}
#[pyfunction]
fn _number(text: &Obj<'_>) -> PyResult<Option<f64>> {
    Ok(text
        .cast::<PyString>()
        .ok()
        .map(|s| s.to_str())
        .transpose()?
        .and_then(parser::py_number))
}
#[pyfunction]
fn parsed_ok(record: &Obj<'_>) -> PyResult<bool> {
    Ok(!record.getattr("value")?.is_none())
}
#[pyfunction]
fn message(record: &Obj<'_>) -> PyResult<String> {
    let py = record.py();
    let err: String = record.getattr("error")?.extract()?;
    if err.is_empty() {
        return Ok(err);
    }
    let kw = PyDict::new(py);
    if err.contains("{unit}") {
        kw.set_item("unit", record.getattr("word")?)?
    }
    py.import("built_core.i18n")?
        .call_method("tr", (err,), Some(&kw))?
        .extract()
}
pub(crate) fn install(m: &Bound<'_, PyModule>) -> PyResult<()> {
    tables::install(m)?;
    m.add_function(wrap_pyfunction!(bind_scene, m)?)?;
    m.add_function(wrap_pyfunction!(model_units_of, m)?)?;
    m.add_function(wrap_pyfunction!(model_units, m)?)?;
    m.add_function(wrap_pyfunction!(model_unit, m)?)?;
    m.add_function(wrap_pyfunction!(model_precision, m)?)?;
    m.add_function(wrap_pyfunction!(apply_units, m)?)?;
    m.add_function(wrap_pyfunction!(ask_at_start, m)?)?;
    m.add_function(wrap_pyfunction!(set_ask_at_start, m)?)?;
    m.add_function(wrap_pyfunction!(new_document_units, m)?)?;
    m.add_function(wrap_pyfunction!(remember_new_document_units, m)?)?;
    m.add_function(wrap_pyfunction!(unit_label, m)?)?;
    m.add_function(wrap_pyfunction!(convert, m)?)?;
    m.add_function(wrap_pyfunction!(bare_number_scale, m)?)?;
    m.add_function(wrap_pyfunction!(unitless_numbers, m)?)?;
    m.add_function(wrap_pyfunction!(typed_context, m)?)?;
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    m.add_function(wrap_pyfunction!(parse_expression, m)?)?;
    m.add_function(wrap_pyfunction!(is_expression, m)?)?;
    m.add_function(wrap_pyfunction!(parse_length_field, m)?)?;
    m.add_function(wrap_pyfunction!(parse_value_box, m)?)?;
    m.add_function(wrap_pyfunction!(_number, m)?)?;
    m.add_function(wrap_pyfunction!(length_number, m)?)?;
    m.add_function(wrap_pyfunction!(metres_from_number, m)?)?;
    m.add_function(wrap_pyfunction!(length_suffix, m)?)?;
    m.add_function(wrap_pyfunction!(format_length, m)?)?;
    m.add_function(wrap_pyfunction!(format_signed_level, m)?)?;
    m.add_function(wrap_pyfunction!(format_value, m)?)?;
    m.add_function(wrap_pyfunction!(format_area, m)?)?;
    m.add_function(wrap_pyfunction!(format_volume, m)?)?;
    m.add_function(wrap_pyfunction!(format_angle, m)?)?;
    m.add_function(wrap_pyfunction!(area_unit_for, m)?)?;
    m.add_function(wrap_pyfunction!(volume_unit_for, m)?)?;
    m.add_function(wrap_pyfunction!(localized, m)?)?;
    m.add_function(wrap_pyfunction!(display_step, m)?)?;
    m.add_function(wrap_pyfunction!(fmt_angle_deg, m)?)?;
    m.add_function(wrap_pyfunction!(typed_value_text, m)?)?;
    m.add_function(wrap_pyfunction!(fine_precision, m)?)?;
    m.add_function(wrap_pyfunction!(preview_text, m)?)?;
    m.add_function(wrap_pyfunction!(fmt_len, m)?)?;
    m.add_function(wrap_pyfunction!(fmt_area, m)?)?;
    m.add_function(wrap_pyfunction!(fmt_volume, m)?)?;
    m.add_function(wrap_pyfunction!(fmt_angle, m)?)?;
    m.add_function(wrap_pyfunction!(fmt_num, m)?)?;
    m.add_function(wrap_pyfunction!(fmt_pair, m)?)?;
    m.add_function(wrap_pyfunction!(fmt_len_fine, m)?)?;
    m.add_function(wrap_pyfunction!(fmt_triple, m)?)?;
    m.add_function(wrap_pyfunction!(fmt_len_with, m)?)?;
    m.add_function(wrap_pyfunction!(fmt_area_with, m)?)?;
    m.add_function(wrap_pyfunction!(fmt_volume_with, m)?)?;
    m.add_function(wrap_pyfunction!(fmt_angle_with, m)?)?;
    m.add("profile_of", wrap_pyfunction!(format::py_profile_of, m)?)?;
    m.add(
        "with_profile",
        wrap_pyfunction!(format::py_with_profile, m)?,
    )?;
    m.add("plain_marks", wrap_pyfunction!(parser::py_plain_marks, m)?)?;
    Ok(())
}
