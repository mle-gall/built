// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::math::{mat, xyz, Vec3};
use glam::DVec3;

fn saved<'p>(scene: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    elements(&value(scene, "saved_views")?)
}
fn frame_scene(frame: &Obj) -> PyResult<Option<String>> {
    Ok(text(&value(frame, "view_key")?)?
        .strip_prefix("scene:")
        .map(str::to_owned))
}
#[pyfunction]
fn frames_showing<'p>(scene: &Obj<'p>, name: &str) -> PyResult<Vec<(Obj<'p>, Obj<'p>)>> {
    let mut found = vec![];
    for c in elements(&value(scene, "compositions")?)? {
        for f in c.getattr("frames")?.try_iter()? {
            let f = f?;
            if frame_scene(&f)?.as_deref() == Some(name) {
                found.push((c.clone(), f));
            }
        }
    }
    Ok(found)
}
#[pyfunction]
fn is_orphan(scene: &Obj, frame: &Obj) -> PyResult<bool> {
    let Some(name) = frame_scene(frame)? else {
        return Ok(false);
    };
    for v in saved(scene)? {
        if v.getattr("name")?.eq(&name)? {
            return Ok(false);
        }
    }
    Ok(true)
}
#[pyfunction]
fn rename_scene(scene: &Obj, view: &Obj, name: &str) -> PyResult<bool> {
    let name = name.trim();
    let old = view.getattr("name")?.extract::<String>()?;
    if name.is_empty() || old == name {
        return Ok(false);
    }
    for v in saved(scene)? {
        if !v.is(view) && v.getattr("name")?.eq(name)? {
            return Ok(false);
        }
    }
    let key = format!("scene:{name}");
    for (_, f) in frames_showing(scene, &old)? {
        f.setattr("view_key", &key)?
    }
    view.setattr("name", name)?;
    Ok(true)
}
#[pyfunction]
fn section_mode(plane: &Obj) -> PyResult<&'static str> {
    Ok(match (flag(plane, "active")?, flag(plane, "hidden")?) {
        (true, false) => "both",
        (true, true) => "cut",
        (false, false) => "frame",
        (false, true) => "off",
    })
}
#[pyfunction]
fn section_label(plane: &Obj) -> PyResult<String> {
    let a = text(&value(plane, "symbol")?)?;
    let b = text(&value(plane, "name")?)?;
    Ok(match (a.is_empty(), b.is_empty()) {
        (false, false) => format!("{a} — {b}"),
        (false, true) => a,
        (true, false) => b,
        (true, true) => format!("#{}", plane.getattr("uid")?),
    })
}
#[pyfunction]
fn unique_name(base: &str, taken: &Obj) -> PyResult<String> {
    let taken = taken
        .try_iter()?
        .map(|x| x?.extract::<String>())
        .collect::<PyResult<HashSet<_>>>()?;
    if !taken.contains(base) {
        return Ok(base.to_string());
    }
    for i in 2.. {
        let name = format!("{base} {i}");
        if !taken.contains(&name) {
            return Ok(name);
        }
    }
    unreachable!()
}
#[pyfunction]
fn world_bounds(groups: &Obj, group: &Obj) -> PyResult<Option<((f64, f64, f64), (f64, f64, f64))>> {
    let mut base = none(groups.py());
    if let Some(path) = path_to(groups, group)? {
        for g in path {
            base = compose(&base, &value(&g, "xform")?)?;
        }
    }
    let placements = groups
        .py()
        .import("built_core.group")?
        .call_method1("iter_placements", (group, base))?;
    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for row in placements.try_iter()? {
        let (g, m): (Obj, Obj) = row?.extract()?;
        let xf = if m.is_none() { None } else { Some(mat(&m)?) };
        for v in elements(&value(&g.getattr("mesh")?, "vertices")?)? {
            let p = xyz(&v.getattr("position")?)?;
            let p = xf.map_or(p, |m| m.transform_point3(p));
            lo = lo.min(p);
            hi = hi.max(p);
        }
    }
    Ok(lo
        .is_finite()
        .then_some(((lo.x, lo.y, lo.z), (hi.x, hi.y, hi.z))))
}
fn number(o: &Obj, k: &str) -> PyResult<f64> {
    o.getattr(k)?.extract()
}
#[pyfunction(signature=(sheet,w_mm,h_mm,gap=10.,fallback=true))]
fn free_slot(
    sheet: &Obj,
    w_mm: f64,
    h_mm: f64,
    gap: f64,
    fallback: bool,
) -> PyResult<Option<(f64, f64)>> {
    if !gap.is_finite() || gap <= 0. {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "gap must be finite and positive",
        ));
    }
    let (w, h): (f64, f64) = sheet.call_method0("page_size_mm")?.extract()?;
    let m = sheet
        .getattr_opt("margin_mm")?
        .map(|x| x.extract::<Option<f64>>())
        .transpose()?
        .unwrap_or(Some(10.))
        .unwrap_or(0.);
    let mut boxes = vec![];
    for f in sheet.getattr("frames")?.try_iter()? {
        let f = f?;
        let (x, y) = (number(&f, "x_mm")?, number(&f, "y_mm")?);
        boxes.push((x, y, x + number(&f, "w_mm")?, y + number(&f, "h_mm")?));
    }
    let mut y = m + gap;
    while y + h_mm <= h - m {
        let mut x = m + gap;
        while x + w_mm <= w - m {
            if boxes.iter().all(|&(a, b, c, d)| {
                x + w_mm + gap / 2. <= a
                    || x - gap / 2. >= c
                    || y + h_mm + gap / 2. <= b
                    || y - gap / 2. >= d
            }) {
                return Ok(Some((x, y)));
            }
            x += gap;
        }
        y += gap;
    }
    Ok(fallback.then(|| {
        boxes.last().map_or((m + gap, m + gap), |&(x, y, _, _)| {
            (x + 2. * gap, y + 2. * gap)
        })
    }))
}
const SCALES: [u32; 22] = [
    1, 2, 5, 10, 20, 25, 50, 75, 100, 125, 150, 200, 250, 300, 500, 750, 1000, 1250, 2000, 2500,
    5000, 10000,
];
#[pyfunction(signature=(scene,view,w_mm,h_mm,fill=0.85))]
fn fit_scale(scene: &Obj, view: &Obj, w_mm: f64, h_mm: f64, fill: f64) -> PyResult<f64> {
    let (lo, hi): (Obj, Obj) = scene.call_method0("bounds")?.extract()?;
    if lo.is_none() {
        return Ok(100.);
    }
    let camera = scene
        .py()
        .import("built_core.camera")?
        .call_method0("OrbitCamera")?;
    camera.setattr("target", Vec3::from(xyz(&view.getattr("target")?)?))?;
    for key in ["distance", "yaw", "pitch"] {
        camera.setattr(key, view.getattr(key)?)?;
    }
    camera.setattr("perspective", false)?;
    let mx = mat(&camera.call_method0("view_matrix")?)?;
    let (lo, hi) = (xyz(&lo)?, xyz(&hi)?);
    let (mut a, mut b) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for x in [lo.x, hi.x] {
        for y in [lo.y, hi.y] {
            for z in [lo.z, hi.z] {
                let p = mx.transform_point3(DVec3::new(x, y, z));
                a = a.min(p);
                b = b.max(p);
            }
        }
    }
    let size = b - a;
    let need =
        (size.x * 1000. / (w_mm * fill).max(1e-6)).max(size.y * 1000. / (h_mm * fill).max(1e-6));
    Ok(f64::from(
        SCALES
            .into_iter()
            .find(|&s| f64::from(s) >= need)
            .unwrap_or(10000),
    ))
}
pub(super) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    m.add("STANDARD_SCALES", PyTuple::new(m.py(), SCALES)?)?;
    for f in [
        wrap_pyfunction!(frames_showing, m)?,
        wrap_pyfunction!(is_orphan, m)?,
        wrap_pyfunction!(rename_scene, m)?,
        wrap_pyfunction!(section_mode, m)?,
        wrap_pyfunction!(section_label, m)?,
        wrap_pyfunction!(unique_name, m)?,
        wrap_pyfunction!(world_bounds, m)?,
        wrap_pyfunction!(free_slot, m)?,
        wrap_pyfunction!(fit_scale, m)?,
    ] {
        m.add_function(f)?
    }
    Ok(())
}
