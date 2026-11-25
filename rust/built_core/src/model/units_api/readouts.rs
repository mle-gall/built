use super::*;

pub(super) fn to_float(x: &Obj<'_>) -> PyResult<f64> {
    x.py()
        .import("builtins")?
        .call_method1("float", (x,))?
        .extract()
}
fn to_int(x: &Obj<'_>) -> PyResult<i64> {
    x.py()
        .import("builtins")?
        .call_method1("int", (x,))?
        .extract()
}
fn to_bool(x: &Obj<'_>) -> PyResult<bool> {
    x.is_truthy()
}
fn length_scale(unit: Option<&str>) -> f64 {
    match unit {
        Some("in" | "in-frac") => 1. / 0.0254,
        Some("ft" | "ft-in" | "ft-in-frac") => 1. / 0.3048,
        Some("cm") => 100.,
        Some("mm") => 1000.,
        _ => 1.,
    }
}

#[pyfunction(signature=(metres,units=Some("m")))]
pub(super) fn length_number(
    #[pyo3(from_py_with=to_float)] metres: f64,
    units: Option<&str>,
) -> PyResult<f64> {
    Ok(metres * length_scale(units))
}
#[pyfunction(signature=(number,units=Some("m")))]
pub(super) fn metres_from_number(
    #[pyo3(from_py_with=to_float)] number: f64,
    units: Option<&str>,
) -> PyResult<f64> {
    Ok(number / length_scale(units))
}
#[pyfunction(signature=(units=Some("m")))]
pub(super) fn length_suffix(units: Option<&str>) -> PyResult<String> {
    Ok(match units {
        Some("in" | "in-frac") => "\"",
        Some("ft" | "ft-in" | "ft-in-frac") => "'",
        Some("cm") => " cm",
        Some("mm") => " mm",
        _ => " m",
    }
    .to_owned())
}
#[pyfunction(signature=(metres,units=Some("m"),decimals=2,show_unit=true))]
pub(super) fn format_length(
    #[pyo3(from_py_with=to_float)] metres: f64,
    units: Option<&str>,
    #[pyo3(from_py_with=to_int)] decimals: i64,
    #[pyo3(from_py_with=to_bool)] show_unit: bool,
) -> PyResult<String> {
    Ok(format::py_format_length(
        metres,
        units.filter(|s| !s.is_empty()).unwrap_or("m"),
        decimals,
        show_unit,
    ))
}
#[pyfunction(signature=(metres,units=Some("m"),decimals=2))]
pub(super) fn format_signed_level(
    #[pyo3(from_py_with=to_float)] metres: f64,
    units: Option<&str>,
    #[pyo3(from_py_with=to_int)] decimals: i64,
) -> PyResult<String> {
    Ok(format::py_format_signed_level(
        metres,
        units.filter(|s| !s.is_empty()).unwrap_or("m"),
        decimals,
    ))
}
#[pyfunction(signature=(value,unit,decimals=2,symbol=true))]
pub(super) fn format_value(
    #[pyo3(from_py_with=to_float)] value: f64,
    unit: &Obj<'_>,
    #[pyo3(from_py_with=to_int)] decimals: i64,
    #[pyo3(from_py_with=to_bool)] symbol: bool,
) -> PyResult<String> {
    let Some(unit) = unit.cast::<PyString>().ok() else {
        return Err(PyKeyError::new_err(unit.clone().unbind()));
    };
    format::py_format_value(value, unit.to_str()?, decimals, symbol)
}
#[pyfunction(signature=(square_metres,unit,decimals=2))]
pub(super) fn format_area(
    #[pyo3(from_py_with=to_float)] square_metres: f64,
    unit: &Obj<'_>,
    #[pyo3(from_py_with=to_int)] decimals: i64,
) -> PyResult<String> {
    format_value(square_metres, unit, decimals, true)
}
#[pyfunction(signature=(cubic_metres,unit,decimals=2))]
pub(super) fn format_volume(
    #[pyo3(from_py_with=to_float)] cubic_metres: f64,
    unit: &Obj<'_>,
    #[pyo3(from_py_with=to_int)] decimals: i64,
) -> PyResult<String> {
    format_value(cubic_metres, unit, decimals, true)
}
#[pyfunction(signature=(radians,unit="deg",decimals=1))]
pub(super) fn format_angle(
    #[pyo3(from_py_with=to_float)] radians: f64,
    unit: &str,
    #[pyo3(from_py_with=to_int)] decimals: i64,
) -> PyResult<String> {
    format::py_format_value(radians, unit, decimals, true)
}
#[pyfunction(signature=(length_unit))]
pub(super) fn area_unit_for(length_unit: Option<&str>) -> PyResult<String> {
    Ok(format::py_area_unit_for(length_unit.unwrap_or("")).to_owned())
}
#[pyfunction(signature=(length_unit))]
pub(super) fn volume_unit_for(length_unit: Option<&str>) -> PyResult<String> {
    Ok(format::py_volume_unit_for(length_unit.unwrap_or("")).to_owned())
}
#[pyfunction(signature=(text,settings=None))]
pub(super) fn localized(
    py: Python<'_>,
    text: &str,
    settings: Option<&Obj<'_>>,
) -> PyResult<String> {
    let settings = match settings {
        Some(s) => s.clone(),
        None => model_units(py)?.into_any(),
    };
    Ok(
        if settings.call_method1("get", ("decimal", "."))?.eq(",")? {
            text.replace(".", ",")
        } else {
            text.to_owned()
        },
    )
}
#[pyfunction(signature=(unit=None,decimals=None))]
pub(super) fn display_step(
    py: Python<'_>,
    unit: Option<&str>,
    decimals: Option<&Obj<'_>>,
) -> PyResult<f64> {
    let unit = match unit.filter(|s| !s.is_empty()) {
        Some(u) => u.to_owned(),
        None => model_unit(py)?,
    };
    let precision = match decimals {
        Some(v) => to_int(v)?,
        None => model_precision(py)?,
    };
    format::py_display_step(&unit, precision)
}
#[pyfunction(signature=(degrees,signed=false))]
pub(super) fn fmt_angle_deg(
    py: Python<'_>,
    #[pyo3(from_py_with=to_float)] degrees: f64,
    #[pyo3(from_py_with=to_bool)] signed: bool,
) -> PyResult<String> {
    format::py_fmt_angle_with(degrees.to_radians(), model_units(py)?.as_any(), signed)
}
#[pyfunction(signature=(buffer))]
pub(super) fn typed_value_text(py: Python<'_>, buffer: Option<&str>) -> PyResult<String> {
    Ok(format::py_typed_value_text(
        buffer.unwrap_or(""),
        &model_unit(py)?,
    ))
}
#[pyfunction(signature=())]
pub(super) fn fine_precision(py: Python<'_>) -> PyResult<i64> {
    format::py_fine_precision(model_units(py)?.as_any())
}
#[pyfunction(signature=(settings))]
pub(super) fn preview_text(settings: &Obj<'_>) -> PyResult<String> {
    format::py_preview_text(settings)
}
#[pyfunction(signature=(metres))]
pub(super) fn fmt_len(
    py: Python<'_>,
    #[pyo3(from_py_with=to_float)] metres: f64,
) -> PyResult<String> {
    format::py_fmt_len_with(metres, model_units(py)?.as_any())
}
#[pyfunction(signature=(square_metres))]
pub(super) fn fmt_area(
    py: Python<'_>,
    #[pyo3(from_py_with=to_float)] square_metres: f64,
) -> PyResult<String> {
    format::py_fmt_area_with(square_metres, model_units(py)?.as_any())
}
#[pyfunction(signature=(cubic_metres))]
pub(super) fn fmt_volume(
    py: Python<'_>,
    #[pyo3(from_py_with=to_float)] cubic_metres: f64,
) -> PyResult<String> {
    format::py_fmt_volume_with(cubic_metres, model_units(py)?.as_any())
}
#[pyfunction(signature=(radians,signed=false))]
pub(super) fn fmt_angle(
    py: Python<'_>,
    #[pyo3(from_py_with=to_float)] radians: f64,
    #[pyo3(from_py_with=to_bool)] signed: bool,
) -> PyResult<String> {
    format::py_fmt_angle_with(radians, model_units(py)?.as_any(), signed)
}
#[pyfunction(signature=(metres))]
pub(super) fn fmt_num(
    py: Python<'_>,
    #[pyo3(from_py_with=to_float)] metres: f64,
) -> PyResult<String> {
    format::py_fmt_num_with(metres, model_units(py)?.as_any())
}
#[pyfunction(signature=(a,b))]
pub(super) fn fmt_pair(
    py: Python<'_>,
    #[pyo3(from_py_with=to_float)] a: f64,
    #[pyo3(from_py_with=to_float)] b: f64,
) -> PyResult<String> {
    format::py_fmt_pair_with(a, b, model_units(py)?.as_any())
}
#[pyfunction(signature=(metres))]
pub(super) fn fmt_len_fine(
    py: Python<'_>,
    #[pyo3(from_py_with=to_float)] metres: f64,
) -> PyResult<String> {
    format::py_fmt_len_fine_with(metres, model_units(py)?.as_any())
}
#[pyfunction(signature=(a,b,c,fine=false))]
pub(super) fn fmt_triple(
    py: Python<'_>,
    #[pyo3(from_py_with=to_float)] a: f64,
    #[pyo3(from_py_with=to_float)] b: f64,
    #[pyo3(from_py_with=to_float)] c: f64,
    #[pyo3(from_py_with=to_bool)] fine: bool,
) -> PyResult<String> {
    format::py_fmt_triple_with(a, b, c, model_units(py)?.as_any(), fine)
}
#[pyfunction(signature=(metres,settings))]
pub(super) fn fmt_len_with(
    #[pyo3(from_py_with=to_float)] metres: f64,
    settings: &Obj<'_>,
) -> PyResult<String> {
    format::py_fmt_len_with(metres, settings)
}
#[pyfunction(signature=(square_metres,settings))]
pub(super) fn fmt_area_with(
    #[pyo3(from_py_with=to_float)] square_metres: f64,
    settings: &Obj<'_>,
) -> PyResult<String> {
    format::py_fmt_area_with(square_metres, settings)
}
#[pyfunction(signature=(cubic_metres,settings))]
pub(super) fn fmt_volume_with(
    #[pyo3(from_py_with=to_float)] cubic_metres: f64,
    settings: &Obj<'_>,
) -> PyResult<String> {
    format::py_fmt_volume_with(cubic_metres, settings)
}
#[pyfunction(signature=(radians,settings,signed=false))]
pub(super) fn fmt_angle_with(
    #[pyo3(from_py_with=to_float)] radians: f64,
    settings: &Obj<'_>,
    #[pyo3(from_py_with=to_bool)] signed: bool,
) -> PyResult<String> {
    format::py_fmt_angle_with(radians, settings, signed)
}
