// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::model::pyfloat::{cos, sin};
use pyo3::exceptions::PyValueError;
use pyo3::types::PyDict;
const LAT: f64 = -16.409;
const LON: f64 = -71.537;
#[pyfunction]
fn solar_position(lat: f64, lon: f64, when_utc: &Obj<'_>) -> PyResult<(f64, f64)> {
    let field = |n: &str| when_utc.getattr(n)?.extract::<i32>();
    let (mut y, mut m) = (field("year")?, field("month")?);
    let (h, min, s) = (
        field("hour")? as f64,
        field("minute")? as f64,
        field("second")? as f64,
    );
    let day = field("day")? as f64 + h / 24. + min / 1440. + s / 86400.;
    if m <= 2 {
        y -= 1;
        m += 12;
    }
    let a = y.div_euclid(100);
    let b = 2 - a + a.div_euclid(4);
    let jd =
        (365.25 * (y + 4716) as f64).floor() + (30.6001 * (m + 1) as f64).floor() + day + b as f64
            - 1524.5;
    let t = (jd - 2451545.) / 36525.;
    let l0 = (280.46646 + t * (36000.76983 + 0.0003032 * t)).rem_euclid(360.);
    let anomaly = 357.52911 + t * (35999.05029 - 0.0001537 * t);
    let ecc = 0.016708634 - t * (0.000042037 + 0.0000001267 * t);
    let mr = anomaly.to_radians();
    let center = sin(mr) * (1.914602 - t * (0.004817 + 0.000014 * t))
        + sin(2. * mr) * (0.019993 - 0.000101 * t)
        + sin(3. * mr) * 0.000289;
    let omega = (125.04 - 1934.136 * t).to_radians();
    let app = l0 + center - 0.00569 - 0.00478 * sin(omega);
    let obliq = 23.
        + (26. + (21.448 - t * (46.815 + t * (0.00059 - t * 0.001813))) / 60.) / 60.
        + 0.00256 * cos(omega);
    let decl = (sin(obliq.to_radians()) * sin(app.to_radians()))
        .asin()
        .to_degrees();
    let v = (obliq / 2.).to_radians().tan().powi(2);
    let l = l0.to_radians();
    let eot = 4.
        * (v * sin(2. * l) - 2. * ecc * sin(mr) + 4. * ecc * v * sin(mr) * cos(2. * l)
            - 0.5 * v * v * sin(4. * l)
            - 1.25 * ecc * ecc * sin(2. * mr))
        .to_degrees();
    let ha =
        ((h * 60. + min + s / 60. + eot + 4. * lon).rem_euclid(1440.) / 4. - 180.).to_radians();
    let (lat, dec) = (lat.to_radians(), decl.to_radians());
    let cz = (sin(lat) * sin(dec) + cos(lat) * cos(dec) * cos(ha)).clamp(-1., 1.);
    let zen = cz.acos().to_degrees();
    let denom = cos(lat) * sin(zen.to_radians());
    let az = if denom.abs() < 1e-12 {
        180.
    } else {
        let a = ((sin(lat) * cz - sin(dec)) / denom)
            .clamp(-1., 1.)
            .acos()
            .to_degrees();
        (if ha > 0. { a + 180. } else { 540. - a }).rem_euclid(360.)
    };
    Ok((90. - zen, az))
}
#[pyfunction]
fn sun_direction(lat: f64, lon: f64, when_utc: &Obj<'_>) -> PyResult<Option<(f64, f64, f64)>> {
    let (e, a) = solar_position(lat, lon, when_utc)?;
    let (e, a) = (e.to_radians(), a.to_radians());
    Ok(if e <= 0. {
        None
    } else {
        Some((cos(e) * sin(a), cos(e) * cos(a), sin(e)))
    })
}
#[pyfunction]
fn wrap_north_deg(deg: f64) -> f64 {
    let d = deg % 360.;
    if d <= -180. {
        d + 360.
    } else if d > 180. {
        d - 360.
    } else {
        d
    }
}
#[pyfunction]
fn scene_north_deg(scene: &Obj<'_>) -> PyResult<f64> {
    let datum = scene.getattr_opt("georef")?.filter(|v| !v.is_none());
    let v = match datum {
        Some(d) => d.getattr_opt("north")?,
        None => scene.getattr_opt("north_deg")?,
    };
    if let Some(v) = v {
        if v.is_truthy()? {
            return v
                .py()
                .import("builtins")?
                .call_method1("float", (v,))?
                .extract();
        }
    }
    Ok(0.)
}
#[pyfunction]
fn north_vector_xy(north_deg: f64) -> (f64, f64) {
    let b = north_deg.to_radians();
    (sin(b), cos(b))
}
#[pyfunction]
fn grid_to_model_xy(north_deg: f64, east: f64, north: f64) -> (f64, f64) {
    let (s, c) = north_vector_xy(north_deg);
    (east * c + north * s, north * c - east * s)
}
#[pyfunction]
fn turn_sun_to_model<'p>(scene: &Obj<'p>, d: &Obj<'p>) -> PyResult<Obj<'p>> {
    if d.is_none() {
        return Ok(d.clone());
    }
    let deg = scene_north_deg(scene)?;
    if deg == 0. {
        return Ok(d.clone());
    }
    let (x, y) = grid_to_model_xy(deg, d.get_item(0)?.extract()?, d.get_item(1)?.extract()?);
    Ok((x, y, d.get_item(2)?).into_pyobject(d.py())?.into_any())
}
#[pyfunction]
fn default_utc_offset(lon: f64) -> i32 {
    (lon / 15.).round_ties_even() as i32
}
fn utc<'p>(
    py: Python<'p>,
    date: (i32, i32, i32),
    clock: (i32, i32),
    off: i32,
) -> PyResult<Obj<'p>> {
    let dt = py.import("datetime")?;
    let span = dt.call_method1("timedelta", (0, off as i64 * 3600))?;
    let kw = PyDict::new(py);
    kw.set_item("tzinfo", dt.call_method1("timezone", (span,))?)?;
    dt.getattr("datetime")?
        .call((date.0, date.1, date.2, clock.0, clock.1), Some(&kw))?
        .call_method1("astimezone", (dt.getattr("timezone")?.getattr("utc")?,))
}
#[pyfunction(signature=(lat,lon,month,day,utc_offset=None,year=2026,step=5))]
fn daylight_minutes(
    py: Python<'_>,
    lat: f64,
    lon: f64,
    month: i32,
    day: i32,
    utc_offset: Option<i32>,
    year: i32,
    step: i32,
) -> PyResult<Option<(i32, i32)>> {
    if step == 0 {
        return Err(PyValueError::new_err("range() arg 3 must not be zero"));
    }
    if step < 0 {
        return Ok(None);
    }
    let off = utc_offset.unwrap_or_else(|| default_utc_offset(lon));
    let mut span = None;
    for minute in (0i32..1440).step_by(step as usize) {
        let t = utc(py, (year, month, day), (minute / 60, minute % 60), off)?;
        if solar_position(lat, lon, &t)?.0 > 0. {
            let r = span.get_or_insert((minute, minute));
            r.1 = minute.saturating_add(step - 1).min(1439);
        }
    }
    Ok(span)
}
const FIELDS: [&str; 7] = [
    "enabled",
    "month",
    "day",
    "hour",
    "minute",
    "darkness",
    "utc_offset",
];
#[pyfunction(signature=(this,lon,year=2026))]
fn when_utc<'p>(this: &Obj<'p>, lon: f64, year: i32) -> PyResult<Obj<'p>> {
    let val = |n: &str| this.getattr(n)?.extract::<i32>();
    let o = this
        .getattr("utc_offset")?
        .extract::<Option<i32>>()?
        .unwrap_or_else(|| default_utc_offset(lon));
    utc(
        this.py(),
        (year, val("month")?, val("day")?),
        (val("hour")?, val("minute")?),
        o,
    )
}
#[pyfunction]
fn to_dict<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    let out = PyDict::new(this.py());
    for n in FIELDS {
        let v = this.getattr(n)?;
        if n != "utc_offset" || !v.is_none() {
            out.set_item(n, v)?;
        }
    }
    Ok(out.into_any())
}
#[pyfunction]
fn from_dict<'p>(cls: &Obj<'p>, raw: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = cls.py();
    let b = py.import("builtins")?;
    let d = cls.call0()?;
    let kw = PyDict::new(py);
    for (n, lo, hi) in [
        ("month", 1, 12),
        ("day", 1, 31),
        ("hour", 0, 23),
        ("minute", 0, 59),
    ] {
        let v = raw.call_method1("get", (n, d.getattr(n)?))?;
        if let Ok(i) = b.call_method1("int", (v,)).and_then(|i| i.extract::<i32>()) {
            if (lo..=hi).contains(&i) {
                kw.set_item(n, i)?;
            }
        }
    }
    let off = raw.call_method1("get", ("utc_offset",))?;
    if !off.is_none() {
        kw.set_item("utc_offset", b.call_method1("int", (off,))?)?;
    }
    kw.set_item(
        "enabled",
        raw.call_method1("get", ("enabled", false))?.is_truthy()?,
    )?;
    let dark = raw.call_method1("get", ("darkness", d.getattr("darkness")?))?;
    if let Ok(v) = b
        .call_method1("float", (dark,))
        .and_then(|v| v.extract::<f64>())
    {
        kw.set_item("darkness", 1f64.min(v).max(0.))?;
    }
    cls.call((), Some(&kw))
}
#[pyfunction]
fn scene_sun_direction<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = scene.py();
    let Some(sh) = scene.getattr_opt("shadows")?.filter(|v| !v.is_none()) else {
        return Ok(py.None().into_bound(py));
    };
    if !sh
        .getattr_opt("enabled")?
        .is_some_and(|v| v.is_truthy().unwrap_or(false))
    {
        return Ok(py.None().into_bound(py));
    }
    let geo = scene
        .getattr_opt("georef")?
        .unwrap_or_else(|| py.None().into_bound(py));
    let val = |n: &str| -> PyResult<Option<f64>> {
        geo.getattr_opt(n)?
            .map(|v| v.extract::<Option<f64>>())
            .transpose()
            .map(Option::flatten)
    };
    let (lat, lon) = val("lat")?.zip(val("lon")?).unwrap_or((LAT, LON));
    let year = py
        .import("datetime")?
        .getattr("date")?
        .call_method0("today")?
        .getattr("year")?;
    let kw = PyDict::new(py);
    kw.set_item("year", year)?;
    match sh.call_method("when_utc", (lon,), Some(&kw)) {
        Ok(t) => turn_sun_to_model(
            scene,
            &sun_direction(lat, lon, &t)?.into_pyobject(py)?.into_any(),
        ),
        Err(e) if e.is_instance_of::<PyValueError>(py) => Ok(py.None().into_bound(py)),
        Err(e) => Err(e),
    }
}
pub(super) fn install(m: &Module<'_>) -> PyResult<()> {
    let py = m.py();
    m.add("DEFAULT_LAT", LAT)?;
    m.add("DEFAULT_LON", LON)?;
    for f in [
        wrap_pyfunction!(solar_position, m)?,
        wrap_pyfunction!(sun_direction, m)?,
        wrap_pyfunction!(wrap_north_deg, m)?,
        wrap_pyfunction!(scene_north_deg, m)?,
        wrap_pyfunction!(north_vector_xy, m)?,
        wrap_pyfunction!(grid_to_model_xy, m)?,
        wrap_pyfunction!(turn_sun_to_model, m)?,
        wrap_pyfunction!(default_utc_offset, m)?,
        wrap_pyfunction!(scene_sun_direction, m)?,
    ] {
        m.add_function(f)?;
    }
    let cache = py.import("functools")?.call_method1("lru_cache", (64,))?;
    m.add(
        "daylight_minutes",
        cache.call1((wrap_pyfunction!(daylight_minutes, m)?,))?,
    )?;
    let ns = PyDict::new(py);
    ns.set_item("enabled", false)?;
    for (k, v) in [("month", 3), ("day", 21), ("hour", 12), ("minute", 0)] {
        ns.set_item(k, v)?;
    }
    ns.set_item("darkness", 0.55)?;
    ns.set_item("utc_offset", py.None())?;
    method(&ns, "when_utc", wrap_pyfunction!(when_utc, m)?.as_any())?;
    method(&ns, "to_dict", wrap_pyfunction!(to_dict, m)?.as_any())?;
    class_method(&ns, "from_dict", wrap_pyfunction!(from_dict, m)?.as_any())?;
    data_class(m, "ShadowSettings", &FIELDS, &ns, None)?;
    Ok(())
}
