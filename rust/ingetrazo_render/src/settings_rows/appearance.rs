//! Document style controls in Settings → This model. Only the edited field is written.
use super::*;

const STEPS: f64 = 200.0;

fn distance_at(step: f64, lo: f64, hi: f64) -> f64 {
    let value = lo * (hi / lo).powf(step.clamp(0.0, STEPS) / STEPS);
    let precision = if value < 1.0 {
        100.0
    } else if value < 10.0 {
        10.0
    } else {
        1.0
    };
    ((value * precision).round() / precision).clamp(lo, hi)
}

fn active<'py>(env: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let win = env.getattr("win")?;
    if win.hasattr("active_style")? {
        win.call_method0("active_style")
    } else {
        env.getattr("scene")?.getattr("display_style")
    }
}

pub(super) fn groups<'py>(
    py: Python<'py>,
    env: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyList>> {
    let (lo, hi): (f64, f64) = py
        .import("ingetrazo_core.style")?
        .getattr("AO_DISTANCE_RANGE")?
        .extract()?;
    let units = py.import("core.units")?;
    let unit: String = units.call_method0("model_unit")?.extract()?;
    let symbol = if unit.starts_with("ft") {
        "ft".to_owned()
    } else if unit.starts_with("in") {
        "in".to_owned()
    } else {
        units
            .getattr("UNITS")?
            .get_item(&unit)?
            .getattr("symbol")?
            .extract()?
    };
    let label = trf(py, "Distance ({unit}):", &[("unit", s(py, &symbol))])?;
    let enabled = || {
        func(py, |_, a| {
            Ok(active(&a.get_item(0)?)?
                .getattr("ambient_occlusion")?
                .unbind())
        })
    };
    let write = |key: &'static str| {
        func(py, move |py, args| {
            let env = args.get_item(0)?;
            let style = active(&env)?;
            let raw = args.get_item(1)?;
            let value = match key {
                "ambient_occlusion" => obj(py, raw.extract::<bool>()?)?,
                _ if !style.getattr("ambient_occlusion")?.is_truthy()? => return Ok(none(py)),
                _ => {
                    let v: f64 = raw.extract()?;
                    if !v.is_finite() {
                        return Err(pyo3::exceptions::PyValueError::new_err(
                            "Invalid slider value",
                        ));
                    }
                    obj(
                        py,
                        if key == "ao_distance" {
                            distance_at(v, lo, hi)
                        } else {
                            (v.clamp(0., 2.) * 10.).round() / 10.
                        },
                    )?
                }
            };
            if !style.getattr(key)?.eq(value.bind(py))? {
                let win = env.getattr("win")?;
                let edited = win.call_method0("own_style")?;
                edited.setattr(key, value)?;
                let scene = env.getattr("scene")?;
                if edited.is(&scene.getattr("display_style")?) {
                    scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
                }
                win.call_method0("repaint_styled")?;
            }
            Ok(none(py))
        })
    };
    let current = active(env)?.getattr("ao_distance")?.extract::<f64>()?;
    let shown = units
        .call_method1("length_number", (current, &unit))?
        .extract::<f64>()?;
    let distance = row(
        py,
        "ao_distance",
        "slider",
        label.trim_end_matches(':'),
        Some(func(py, move |py, a| {
            let metres = active(&a.get_item(0)?)?
                .getattr("ao_distance")?
                .extract::<f64>()?;
            obj(
                py,
                ((metres.clamp(lo, hi) / lo).ln() / (hi / lo).ln() * STEPS).round(),
            )
        })?),
        Some(write("ao_distance")?),
        &[
            ("hi", obj(py, STEPS)?),
            ("enabled", enabled()?),
            (
                "fmt",
                func(py, move |py, _| {
                    Ok(py
                        .import("core.units")?
                        .call_method1("localized", (format!("{shown:.2}"),))?
                        .unbind())
                })?,
            ),
        ],
    )?;
    let intensity = row(
        py,
        "ao_intensity",
        "slider",
        &tr(py, "Intensity")?,
        Some(func(py, |_, a| {
            Ok(active(&a.get_item(0)?)?.getattr("ao_intensity")?.unbind())
        })?),
        Some(write("ao_intensity")?),
        &[
            ("hi", obj(py, 2.0)?),
            ("decimals", obj(py, 2)?),
            ("enabled", enabled()?),
            (
                "fmt",
                func(py, |py, a| {
                    let value: f64 = a.get_item(0)?.extract()?;
                    Ok(py
                        .import("core.units")?
                        .call_method1("localized", (format!("{value:.2}"),))?
                        .unbind())
                })?,
            ),
        ],
    )?;
    let toggle = row(
        py,
        "ambient_occlusion",
        "check",
        &tr(py, "Ambient occlusion")?,
        Some(enabled()?),
        Some(write("ambient_occlusion")?),
        &[],
    )?;
    let rows = PyList::new(py, [toggle, distance, intensity])?;
    PyList::new(
        py,
        [PyTuple::new(
            py,
            [
                s(py, "occlusion"),
                s(py, &tr(py, "Ambient occlusion")?),
                rows.into_any(),
            ],
        )?],
    )
}
