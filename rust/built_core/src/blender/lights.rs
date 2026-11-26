// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::types::PyString;
const NAMED: [(&str, i32); 3] = [("warm", 2700), ("neutral", 4000), ("cool", 6500)];
#[pyfunction]
fn kelvin_to_rgb(kelvin: f64) -> (f64, f64, f64) {
    let t = kelvin.clamp(1000., 40000.) / 100.;
    let (r, g, b) = if t <= 66. {
        (
            255.,
            99.4708025861 * t.ln() - 161.1195681661,
            if t <= 19. {
                0.
            } else {
                138.5177312231 * (t - 10.).ln() - 305.0447927307
            },
        )
    } else {
        (
            329.698727446 * (t - 60.).powf(-0.1332047592),
            288.1221695283 * (t - 60.).powf(-0.0755148492),
            255.,
        )
    };
    let norm = |c: f64| (c.clamp(0., 255.) / 255. * 1e4).round() / 1e4;
    (norm(r), norm(g), norm(b))
}
#[pyfunction]
fn aim_from_angles(heading_deg: f64, tilt_deg: f64) -> Vec<f64> {
    let (h, t) = (heading_deg.to_radians(), tilt_deg.to_radians());
    vec![t.cos() * h.sin(), t.cos() * h.cos(), -t.sin()]
}
#[pyfunction]
fn angles_from_aim(d: [f64; 3]) -> (f64, f64) {
    let [x, y, z] = d;
    let len = (x * x + y * y + z * z).sqrt();
    let t = (-z / if len == 0. { 1. } else { len })
        .clamp(-1., 1.)
        .asin()
        .to_degrees();
    let h = if x.hypot(y) > 1e-9 {
        x.atan2(y).to_degrees().rem_euclid(360.)
    } else {
        0.
    };
    (
        (h * 10.).round_ties_even() / 10.,
        (t * 10.).round_ties_even() / 10.,
    )
}
fn num(v: &Obj) -> PyResult<f64> {
    v.py()
        .import("builtins")?
        .call_method1("float", (v,))?
        .extract()
}
fn triple(v: &Obj) -> PyResult<[f64; 3]> {
    let a = v
        .try_iter()?
        .take(3)
        .map(|x| num(&x?))
        .collect::<PyResult<Vec<_>>>()?;
    a.try_into()
        .map_err(|_| PyValueError::new_err("three coordinates required"))
}
fn light<'p>(lt: &Obj<'p>) -> PyResult<Option<Bound<'p, PyDict>>> {
    let py = lt.py();
    let kind = lt
        .call_method1("get", ("kind", "point"))?
        .extract::<String>()?;
    if !["point", "spot"].contains(&kind.as_str()) {
        return Ok(None);
    }
    let pos = triple(&lt.get_item("pos")?)?;
    let mut dir = triple(&lt.call_method1("get", ("dir", (0., 0., -1.)))?)?;
    let power =
        num(&lt.call_method1("get", ("power", if kind == "spot" { 1500. } else { 400. }))?)?;
    let angle = num(&lt.call_method1("get", ("angle", 60.))?)?;
    if !pos.iter().chain(&dir).all(|v| v.is_finite()) {
        return Ok(None);
    }
    if dir[0].hypot(dir[1]).hypot(dir[2]) < 1e-9 {
        dir = [0., 0., -1.];
    }
    let k = lt.call_method1("get", ("kelvin",))?;
    let color = lt.call_method1("get", ("color",))?;
    let k = if !k.is_none() {
        Some(
            num(&k)
                .map(|v| v.clamp(1800., 10000.) as i32)
                .unwrap_or(2700),
        )
    } else if color.is_none() || color.is_instance_of::<PyString>() {
        Some(
            NAMED
                .iter()
                .find(|(name, _)| color.eq(*name).unwrap_or(false))
                .map_or(2700, |(_, k)| *k),
        )
    } else {
        None
    };
    let raw_rgb = if k.is_none() {
        triple(&color).ok()
    } else {
        None
    };
    let (rgb, k) = match raw_rgb {
        Some(rgb) => (rgb.map(|x| x.clamp(0., 1.)), None),
        None => {
            let k = k.unwrap_or(2700);
            let (r, g, b) = kelvin_to_rgb(k as f64);
            ([r, g, b], Some(k))
        }
    };
    let out = PyDict::new(py);
    out.set_item("kind", kind)?;
    out.set_item("pos", pos.to_vec())?;
    out.set_item("dir", dir.to_vec())?;
    out.set_item("kelvin", k)?;
    out.set_item("color", rgb.to_vec())?;
    out.set_item("power", power.min(1e6).max(0.))?;
    out.set_item("angle", angle.min(179.).max(1.))?;
    out.set_item("on", lt.call_method1("get", ("on", true))?.is_truthy()?)?;
    out.set_item("name", lt.call_method1("get", ("name", ""))?.str()?)?;
    // The Blender link names lights by id; older lights have none.
    let id = lt.call_method1("get", ("id",))?;
    if id.is_truthy()? {
        out.set_item("id", id.str()?)?;
    }
    Ok(Some(out))
}
#[pyfunction]
pub(super) fn clean_lights<'p>(raw: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let py = raw.py();
    let out = PyList::empty(py);
    if let Ok(list) = raw.cast::<PyList>() {
        for v in list.iter() {
            match light(&v) {
                Ok(Some(lt)) => out.append(lt)?,
                Ok(None) => (),
                Err(e)
                    if e.is_instance_of::<pyo3::exceptions::PyAttributeError>(py)
                        || e.is_instance_of::<pyo3::exceptions::PyTypeError>(py)
                        || e.is_instance_of::<pyo3::exceptions::PyKeyError>(py)
                        || e.is_instance_of::<PyValueError>(py) =>
                {
                    ()
                }
                Err(e) => return Err(e),
            }
        }
    }
    Ok(out)
}
pub(super) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("MIN_KELVIN", 1800)?;
    m.add("MAX_KELVIN", 10000)?;
    m.add("DEFAULT_KELVIN", 2700)?;
    m.add("LIGHT_KINDS", ("point", "spot"))?;
    m.add(
        "DEFAULT_POWER",
        [("point", 400.), ("spot", 1500.)].into_py_dict(py)?,
    )?;
    m.add("NAMED_KELVIN", NAMED.into_py_dict(py)?)?;
    m.add(
        "LIGHT_COLORS",
        NAMED
            .map(|(n, k)| (n, kelvin_to_rgb(k as f64)))
            .into_py_dict(py)?,
    )?;
    for f in [
        wrap_pyfunction!(clean_lights, m)?,
        wrap_pyfunction!(kelvin_to_rgb, m)?,
        wrap_pyfunction!(angles_from_aim, m)?,
        wrap_pyfunction!(aim_from_angles, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
