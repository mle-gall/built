// SPDX-License-Identifier: GPL-3.0-or-later
//! Survey points, editable path records, horizontal chainage and elevation profiles.
use super::Obj;
use crate::math::{xyz, Vec3};
use glam::DVec3;
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList},
};
pub(super) fn points(obj: &Obj<'_>) -> PyResult<Vec<DVec3>> {
    obj.try_iter()?.map(|p| xyz(&p?)).collect()
}
fn length(v: &[DVec3]) -> f64 {
    v.windows(2)
        .map(|p| (p[1] - p[0]).truncate().length())
        .sum()
}
fn walk(v: &[DVec3], s: f64) -> (f64, f64) {
    let Some(first) = v.first() else {
        return (0., 0.);
    };
    if s <= 0. {
        return (first.x, first.y);
    }
    let mut sum = 0.;
    for pair in v.windows(2) {
        let d = pair[1] - pair[0];
        let l = d.truncate().length();
        if sum + l >= s {
            let t = if l > 1e-12 { (s - sum) / l } else { 0. };
            return (pair[0].x + t * d.x, pair[0].y + t * d.y);
        }
        sum += l
    }
    let p = v.last().unwrap();
    (p.x, p.y)
}
#[pyfunction(signature=(obj,position,name="",desc=""))]
fn geo_point_init(obj: &Obj<'_>, position: &Obj<'_>, name: &str, desc: &str) -> PyResult<()> {
    obj.setattr("position", Vec3::from(xyz(position)?))?;
    obj.setattr("name", name)?;
    obj.setattr("desc", desc)
}
#[pyfunction(signature=(obj,points,closed=false,name="",surface=None))]
fn geo_path_init(
    obj: &Obj<'_>,
    points: &Obj<'_>,
    closed: bool,
    name: &str,
    surface: Option<&str>,
) -> PyResult<()> {
    let list = PyList::empty(obj.py());
    for p in points.try_iter()? {
        list.append(Vec3::from(xyz(&p?)?))?
    }
    obj.setattr("points", list)?;
    obj.setattr("closed", closed)?;
    obj.setattr("name", name)?;
    obj.setattr("surface", surface)?;
    obj.setattr("_surface_tris", obj.py().None())
}
#[pyfunction]
fn geo_record_dict<'p>(py: Python<'p>, obj: &Obj<'p>) -> PyResult<Obj<'p>> {
    let d = PyDict::new(py);
    let fields = if let Some(pos) = obj.getattr_opt("position")? {
        d.set_item("position", xyz(&pos)?.to_array().to_vec())?;
        &["name", "desc"][..]
    } else {
        d.set_item(
            "points",
            points(&obj.getattr("points")?)?
                .iter()
                .map(|v| v.to_array().to_vec())
                .collect::<Vec<_>>(),
        )?;
        &["closed", "name", "surface"][..]
    };
    for k in fields {
        let v = obj.getattr(*k)?;
        if v.is_truthy()? {
            d.set_item(*k, v)?
        }
    }
    Ok(d.into_any())
}
#[pyfunction]
fn geo_point_from<'p>(cls: &Obj<'p>, data: &Obj<'p>) -> PyResult<Obj<'p>> {
    let v = data.call_method1("get", ("position", vec![0.; 3]))?;
    cls.call1((
        Vec3::from(xyz(&v)?),
        data.call_method1("get", ("name", ""))?,
        data.call_method1("get", ("desc", ""))?,
    ))
}
#[pyfunction]
fn geo_path_from<'p>(cls: &Obj<'p>, data: &Obj<'p>) -> PyResult<Obj<'p>> {
    cls.call1((
        data.call_method1("get", ("points", PyList::empty(cls.py())))?,
        data.call_method1("get", ("closed", false))?,
        data.call_method1("get", ("name", ""))?,
        data.call_method1("get", ("surface",))?,
    ))
}
#[pyfunction]
fn profile_points<'p>(py: Python<'p>, obj: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let p = py
        .get_type::<PyList>()
        .call1((obj.getattr("points")?,))?
        .cast_into::<PyList>()?;
    if obj.getattr("closed")?.is_truthy()? && p.len() > 2 {
        p.append(Vec3::from(xyz(&p.get_item(0)?)?))?
    }
    Ok(p)
}
#[pyfunction]
fn path_segments<'p>(py: Python<'p>, obj: &Obj<'p>) -> PyResult<Obj<'p>> {
    let p = obj.getattr("points")?;
    let l = PyList::empty(py);
    for i in 1..p.len()? {
        l.append((p.get_item(i - 1)?, p.get_item(i)?))?
    }
    if p.len()? > 2 && obj.getattr("closed")?.is_truthy()? {
        l.append((p.get_item(p.len()? - 1)?, p.get_item(0)?))?
    }
    Ok(l.try_iter()?.into_any())
}
#[pyfunction]
fn path_length(obj: &Obj<'_>) -> PyResult<f64> {
    let p = points(&obj.getattr("points")?)?;
    let mut l = length(&p);
    if p.len() > 2 && obj.getattr("closed")?.is_truthy()? {
        l += (p[0] - p[p.len() - 1]).truncate().length()
    }
    Ok(l)
}
#[pyfunction]
fn path_area(obj: &Obj<'_>) -> PyResult<f64> {
    let p = points(&obj.getattr("points")?)?;
    if p.len() < 3 || !obj.getattr("closed")?.is_truthy()? {
        return Ok(0.);
    }
    Ok(p.iter()
        .zip(p.iter().cycle().skip(1))
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum::<f64>()
        .abs()
        / 2.)
}
#[pyfunction]
fn path_surface_area(obj: &Obj<'_>) -> PyResult<Option<f64>> {
    let tri = obj.getattr("_surface_tris")?;
    if !tri.is_truthy()? {
        return Ok(None);
    }
    let mut area = 0.;
    for t in tri.try_iter()? {
        let t = t?;
        let a = xyz(&t.get_item(0)?)?;
        area += (xyz(&t.get_item(1)?)? - a)
            .cross(xyz(&t.get_item(2)?)? - a)
            .length()
            / 2.;
    }
    Ok(Some(area))
}
#[pyfunction]
fn parse_points_csv(py: Python<'_>, text: String) -> PyResult<Py<PyAny>> {
    crate::file_open::DocumentOpen::stage(
        py,
        None,
        move |py, _| Ok(parse_rows(py, &text)?.unbind()),
    )
}
fn parse_rows<'p>(py: Python<'p>, text: &str) -> PyResult<Obj<'p>> {
    let list = PyList::empty(py);
    let read = py.import("csv")?.getattr("reader")?;
    let options = |sep: &str| -> PyResult<_> {
        let args = PyDict::new(py);
        args.set_item("delimiter", sep)?;
        Ok(args)
    };
    let comma = options(",")?;
    let semi = options(";")?;
    for (i, line) in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .enumerate()
    {
        if i % 256 == 0 {
            crate::file_open::checkpoint(py)?;
        }
        let cells: Vec<String> = if let Some(sep) = if line.contains(';') {
            Some(';')
        } else if line.contains(',') {
            Some(',')
        } else {
            None
        } {
            read.call((vec![line],), Some(if sep == ',' { &comma } else { &semi }))?
                .call_method0("__next__")?
                .extract()?
        } else {
            line.split_whitespace().map(str::to_owned).collect()
        };
        if cells.len() < 4 {
            continue;
        }
        let num = |i: usize| cells[i].trim().replace(',', ".").parse::<f64>().ok();
        let (Some(n), Some(e), Some(z)) = (num(1), num(2), num(3)) else {
            continue;
        };
        let d = PyDict::new(py);
        d.set_item("name", cells[0].trim())?;
        d.set_item("north", n)?;
        d.set_item("east", e)?;
        d.set_item("z", z)?;
        d.set_item(
            "desc",
            cells[4..]
                .iter()
                .map(|s| s.trim())
                .collect::<Vec<_>>()
                .join(",")
                .trim(),
        )?;
        list.append(d)?;
    }
    if list.is_empty() {
        return Err(PyValueError::new_err(
            "No survey points found (expected P,N,E,Z[,desc])",
        ));
    }
    Ok(list.into_any())
}
#[pyfunction]
fn points_from_rows<'p>(py: Python<'p>, rows: &Obj<'p>, datum: &Obj<'p>) -> PyResult<Obj<'p>> {
    let out = PyList::empty(py);
    let cls = py.import("built_core.georef.points")?.getattr("GeoPoint")?;
    for r in rows.try_iter()? {
        let r = r?;
        let p = datum.call_method1(
            "utm_to_local",
            (r.get_item("east")?, r.get_item("north")?, r.get_item("z")?),
        )?;
        out.append(cls.call1((p, r.get_item("name")?, r.get_item("desc")?))?)?
    }
    Ok(out.into_any())
}
#[pyfunction]
fn datum_for_rows<'p>(
    py: Python<'p>,
    rows: &Obj<'p>,
    zone: i32,
    northern: bool,
) -> PyResult<Obj<'p>> {
    let r = rows.get_item(0)?;
    let (lat, lon) = crate::location::utm_inverse(
        r.get_item("east")?.extract()?,
        r.get_item("north")?.extract()?,
        zone,
        northern,
    );
    let d = py
        .import("built_core.georef.datum")?
        .call_method1("SceneDatum", (lat, lon, r.get_item("z")?))?;
    if d.getattr("zone")?.extract::<i32>()? != zone {
        d.setattr("zone", zone)?;
        d.setattr("northern", northern)?;
        let (e, n) = crate::location::utm_forward(lat, lon, zone);
        d.setattr("_east0", e)?;
        d.setattr("_north0", n)?
    }
    Ok(d)
}
#[pyfunction]
fn polyline_length(pts: &Obj<'_>) -> PyResult<f64> {
    Ok(length(&points(pts)?))
}
#[pyfunction]
fn point_at_station(pts: &Obj<'_>, s: f64) -> PyResult<(f64, f64)> {
    Ok(walk(&points(pts)?, s))
}
#[pyfunction]
fn selected_geopath<'p>(py: Python<'p>, scene: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    let cls = py.import("built_core.georef.geopath")?.getattr("GeoPath")?;
    let mut selected = None;
    for p in scene.getattr("selection")?.try_iter()? {
        let p = p?;
        if p.is_instance(&cls)? {
            if selected.is_some() {
                return Ok(None);
            }
            selected = Some(p)
        }
    }
    if selected.is_none() {
        if let Some(paths) = scene.getattr_opt("geo_paths")? {
            if paths.len()? == 1 {
                return Ok(Some(paths.get_item(0)?));
            }
        }
    }
    Ok(selected)
}
#[pyfunction(signature=(polyline,sampler,spacing=None))]
fn sample_profile<'p>(
    py: Python<'p>,
    polyline: &Obj<'p>,
    sampler: &Obj<'p>,
    spacing: Option<f64>,
) -> PyResult<Obj<'p>> {
    let pts = points(polyline)?;
    let len = length(&pts);
    let module = py.import("built_core.georef.profile")?;
    let samples = PyList::empty(py);
    if len <= 1e-9 {
        return module.call_method1("Profile", (samples, 0., true));
    }
    let step = spacing.unwrap_or((len / 256.).max(1.));
    if step == 0. || !step.is_finite() {
        return Err(PyValueError::new_err("invalid profile spacing"));
    }
    let d = sampler.getattr("datum")?;
    let (mut s, mut w, mut n, mut e) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for p in &pts {
        let (la, lo, _) = d
            .call_method1("local_to_geodetic", (Vec3::from(*p),))?
            .extract::<(f64, f64, f64)>()?;
        s = s.min(la);
        n = n.max(la);
        w = w.min(lo);
        e = e.max(lo)
    }
    sampler.call_method1("ensure_area", (s, w, n, e))?;
    let count = ((len / step).ceil() as usize).max(1);
    let mut complete = true;
    let height = sampler.getattr("elevation_at_local")?;
    let sample = module.getattr("ProfileSample")?;
    let mut seg = 0;
    let mut start = 0.;
    for i in 0..=count {
        let station = (i as f64 * step).min(len);
        while seg + 2 < pts.len() {
            let l = (pts[seg + 1] - pts[seg]).truncate().length();
            if start + l >= station {
                break;
            }
            start += l;
            seg += 1
        }
        let (x, y) = walk(&pts[seg..seg + 2], station - start);
        let z = height.call1((Vec3::from(DVec3::new(x, y, 0.)),))?;
        complete &= !z.is_none();
        samples.append(sample.call1((station, x, y, z))?)?
    }
    module.call_method1("Profile", (samples, len, complete))
}
#[pyfunction]
fn elevations(profile: &Obj<'_>) -> PyResult<Vec<f64>> {
    let mut out = vec![];
    for s in profile.getattr("samples")?.try_iter()? {
        if let Some(z) = s?.getattr("elevation")?.extract::<Option<f64>>()? {
            out.push(z)
        }
    }
    Ok(out)
}
#[pyfunction]
fn min_elevation(profile: &Obj<'_>) -> PyResult<Option<f64>> {
    Ok(elevations(profile)?.into_iter().reduce(f64::min))
}
#[pyfunction]
fn max_elevation(profile: &Obj<'_>) -> PyResult<Option<f64>> {
    Ok(elevations(profile)?.into_iter().reduce(f64::max))
}
#[pyfunction]
fn total_gain(profile: &Obj<'_>) -> PyResult<f64> {
    Ok(elevations(profile)?
        .windows(2)
        .map(|p| (p[1] - p[0]).max(0.))
        .sum())
}
#[pyfunction]
fn slopes(profile: &Obj<'_>) -> PyResult<Vec<(f64, f64)>> {
    let mut last = None;
    let mut v = vec![];
    for sample in profile.getattr("samples")?.try_iter()? {
        let s = sample?;
        let station = s.getattr("station")?.extract::<f64>()?;
        let z = s.getattr("elevation")?.extract::<Option<f64>>()?;
        if let (Some((p, a)), Some(b)) = (last, z) {
            let ds = station - p;
            if ds > 1e-9 {
                v.push((p, (b - a) / ds * 100.));
            }
        }
        last = z.map(|e| (station, e));
    }
    Ok(v)
}
#[pyfunction(signature=(profile,vertical="local",datum_alt=None))]
fn profile_to_csv(profile: &Obj<'_>, vertical: &str, datum_alt: Option<f64>) -> PyResult<String> {
    use std::fmt::Write;
    let notes = profile
        .py()
        .import("built_core.georef.profile")?
        .getattr("VERTICAL_NOTES")?;
    let note = notes.call_method1("get", (vertical, notes.get_item("local")?))?;
    let mut out = format!("# elevation reference: {note}\n");
    if let Some(alt) = datum_alt {
        writeln!(out, "# scene Z=0 is at {alt:.3} m in that reference").unwrap()
    }
    out.push_str("station_m,x_m,y_m,elevation_m\n");
    for s in profile.getattr("samples")?.try_iter()? {
        let s = s?;
        for k in ["station", "x", "y"] {
            write!(out, "{:.3},", s.getattr(k)?.extract::<f64>()?).unwrap()
        }
        if let Some(z) = s.getattr("elevation")?.extract::<Option<f64>>()? {
            write!(out, "{z:.3}").unwrap()
        }
        out.push('\n')
    }
    Ok(out)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    functions!(m;geo_point_init,geo_path_init,geo_record_dict,geo_point_from,geo_path_from,profile_points,path_segments,path_length,path_area,path_surface_area,parse_points_csv,points_from_rows,datum_for_rows,polyline_length,point_at_station,selected_geopath,sample_profile,elevations,min_elevation,max_elevation,total_gain,slopes,profile_to_csv);
    Ok(())
}
