// SPDX-License-Identifier: GPL-3.0-or-later
use crate::settings_rows::*;
use pyo3::{prelude::*, types::PyList};
type Obj<'p> = Bound<'p, PyAny>;
const MONTHS: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
fn daylight(scene: &Obj<'_>, sh: &Obj<'_>) -> PyResult<Option<(i32, i32)>> {
    let sun = scene.py().import("core.sun")?;
    let site = scene.getattr_opt("georef")?.filter(|v| !v.is_none());
    let coord = |k: &str, default: &str| match &site {
        Some(s) => s.getattr(k),
        None => sun.getattr(default),
    };
    sun.call_method1(
        "daylight_minutes",
        (
            coord("lat", "DEFAULT_LAT")?,
            coord("lon", "DEFAULT_LON")?,
            sh.getattr("month")?,
            sh.getattr("day")?,
            sh.getattr("utc_offset")?,
        ),
    )?
    .extract()
}
fn number(o: &Obj<'_>, key: &str) -> PyResult<i32> {
    o.getattr(key)?.extract()
}
fn time(n: i32) -> String {
    format!("{:02}:{:02}", n / 60, n % 60)
}
fn get_value(py: Python<'_>, env: &Obj<'_>, k: &str) -> Res {
    let sh = env.getattr("scene")?.getattr("shadows")?;
    let (m, d) = (number(&sh, "month")?, number(&sh, "day")?);
    match k {
        "sun_day" => obj(
            py,
            MONTHS[..(m.clamp(1, 12) - 1) as usize].iter().sum::<i32>() + d,
        ),
        "sun_date" => obj(py, format!("{m:02}-{d:02}")),
        "sun_time" => obj(py, number(&sh, "hour")? * 60 + number(&sh, "minute")?),
        "sun_dark" => obj(
            py,
            ((1. - sh.getattr("darkness")?.extract::<f64>()?) * 100.).round() as i32,
        ),
        "sun_zone" => obj(
            py,
            sh.getattr("utc_offset")?
                .extract::<Option<i32>>()?
                .map(|v| v.to_string())
                .unwrap_or("auto".into()),
        ),
        _ => Ok(sh.getattr("enabled")?.unbind()),
    }
}
fn put(py: Python<'_>, k: &'static str) -> Res {
    func(py, move |py, a| {
        let env = a.get_item(0)?;
        let scene = env.getattr("scene")?;
        let sh = scene.getattr("shadows")?;
        let raw = a.get_item(1)?;
        let n = || raw.extract::<f64>().map(|v| v.round() as i32);
        let mut t = number(&sh, "hour")? * 60 + number(&sh, "minute")?;
        match k {
            "sun_day" => {
                let mut day = n()?.clamp(1, 365);
                for (i, &days) in MONTHS.iter().enumerate() {
                    if day <= days {
                        sh.setattr("month", i + 1)?;
                        sh.setattr("day", day)?;
                        break;
                    }
                    day -= days;
                }
            }
            "sun_date" => {
                let raw = raw.extract::<String>()?;
                let date = raw
                    .split_once('-')
                    .and_then(|(m, d)| Some((m.parse::<usize>().ok()?, d.parse::<i32>().ok()?)));
                let Some((m, d)) = date
                    .filter(|&(m, d)| (1..=12).contains(&m) && (1..=MONTHS[m - 1]).contains(&d))
                else {
                    return Err(pyo3::exceptions::PyValueError::new_err(
                        "Enter month and day as MM-DD",
                    ));
                };
                sh.setattr("month", m)?;
                sh.setattr("day", d)?;
            }
            "sun_zone" => {
                let v = raw.extract::<String>()?;
                sh.setattr(
                    "utc_offset",
                    if v == "auto" {
                        None
                    } else {
                        Some(v.parse::<i32>().map_err(|_| {
                            pyo3::exceptions::PyValueError::new_err("Invalid UTC offset")
                        })?)
                    },
                )?;
            }
            "sun_time" => t = n()?,
            "sun_dark" => sh.setattr("darkness", 1. - n()?.clamp(0, 100) as f64 / 100.)?,
            _ => sh.setattr("enabled", raw.is_truthy()?)?,
        }
        if let Some((lo, hi)) = daylight(&scene, &sh)? {
            t = t.clamp(lo, hi);
        }
        sh.setattr("hour", t / 60)?;
        sh.setattr("minute", t % 60)?;
        scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
        _scene_changed(&env)?;
        Ok(none(py))
    })
}
pub(crate) fn append<'p>(
    py: Python<'p>,
    env: &Obj<'p>,
    groups: &Bound<'p, PyList>,
) -> PyResult<()> {
    let scene = env.getattr("scene")?;
    let Some(sh) = scene.getattr_opt("shadows")?.filter(|sh| !sh.is_none()) else {
        return Ok(());
    };
    let daylight = daylight(&scene, &sh)?;
    let (lo, hi) = daylight.unwrap_or((0, 1439));
    let rows = PyList::empty(py);
    for (k, kind, title) in [
        ("sun_enabled", "check", "Shadows"),
        ("sun_date", "edit", "Date (MM-DD)"),
        ("sun_day", "slider", "Day of year"),
        ("sun_zone", "choice", "Time zone"),
        ("sun_time", "slider", "Time"),
        ("sun_dark", "slider", "Darkness"),
    ] {
        let mut kw = vec![];
        match k {
            "sun_time" => {
                kw.extend([
                    ("lo", obj(py, lo)?),
                    ("hi", obj(py, hi)?),
                    (
                        "fmt",
                        func(py, |py, a| {
                            obj(py, time(a.get_item(0)?.extract::<f64>()?.round() as i32))
                        })?,
                    ),
                ]);
                if daylight.is_some() {
                    kw.push(("tip", obj(py, format!("{} – {}", time(lo), time(hi)))?));
                }
            }
            "sun_day" => kw.extend([("lo", obj(py, 1)?), ("hi", obj(py, 365)?)]),
            "sun_dark" => kw.extend([("hi", obj(py, 100)?), ("suffix", obj(py, "%")?)]),
            "sun_zone" => {
                let opts = PyList::empty(py);
                opts.append(("auto", tr(py, "Automatic")?))?;
                for off in -12..=14 {
                    opts.append((off.to_string(), format!("UTC{off:+}")))?;
                }
                kw.push(("options", opts.into_any().unbind()));
            }
            _ => {}
        }
        rows.append(row(
            py,
            k,
            kind,
            &tr(py, title)?,
            Some(func(py, move |py, a| get_value(py, &a.get_item(0)?, k))?),
            Some(put(py, k)?),
            &kw,
        )?)?;
    }
    groups.append(("sun", tr(py, "Shadows")?, rows))?;
    Ok(())
}
