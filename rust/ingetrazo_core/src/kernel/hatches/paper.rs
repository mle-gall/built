// SPDX-License-Identifier: GPL-3.0-or-later
//! Compose paper-space marks from native pattern, masonry and clipping operations.
use super::*;
use crate::hatch_pattern::{fit_band, fitted_marks, generate_hatch};
type Point = (f64, f64);
type Line = (f64, f64, f64, f64);
type Ring = Vec<Point>;
type Band = (Point, Point, Point, f64, f64);

fn ring_points(ring: &Obj<'_>) -> PyResult<Ring> {
    ring.try_iter()?
        .map(|p| {
            let p = p?;
            Ok((number(&p.get_item(0)?)?, number(&p.get_item(1)?)?))
        })
        .collect()
}
#[pyfunction]
pub(super) fn vertical_band(ring: &Obj<'_>) -> PyResult<Option<Band>> {
    Ok(upright_band(&ring_points(ring)?))
}
fn upright_band(ring: &[Point]) -> Option<Band> {
    let [x, y, x1, y1] = bounds(ring);
    (ring.len() >= 3 && x1 - x > 1e-9 && y1 - y > 1e-9).then_some((
        (x, y1),
        (0., -1.),
        (1., 0.),
        y1 - y,
        x1 - x,
    ))
}
fn bounds(points: &[Point]) -> [f64; 4] {
    points.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |[a, b, c, d], &(x, y)| [a.min(x), b.min(y), c.max(x), d.max(y)],
    )
}
#[pyfunction(signature=(joints,scale_n=100.))]
pub(super) fn joints_mode(joints: Option<&str>, scale_n: f64) -> &str {
    match joints {
        Some(s @ ("single" | "double" | "none")) => s,
        _ if scale(scale_n) >= 75. => "hatch",
        _ if scale(scale_n) <= 15. => "double",
        _ => "single",
    }
}
#[pyfunction(pass_module,signature=(key,k,thick,brick=None,lo=0.,hi=0.))]
pub(super) fn bond_course(
    m: &Module<'_>,
    key: &str,
    k: i64,
    thick: f64,
    brick: Option<&Obj<'_>>,
    lo: f64,
    hi: f64,
) -> PyResult<(f64, f64, Vec<(f64, f64, f64, f64, &'static str)>)> {
    Ok(hatch_bond::masonry_course(
        key,
        k,
        thick,
        dims(m, brick, piece_kind(m, Some(key))?)?,
        lo,
        hi,
    ))
}
#[pyfunction(pass_module,signature=(key,band,cut,scale=1.,scale_n=100.,brick=None,joints="single",offset=None))]
pub(super) fn bond_marks<'p>(
    m: &Module<'p>,
    key: Option<&str>,
    band: Option<Band>,
    cut: &str,
    scale: f64,
    scale_n: f64,
    brick: Option<&Obj<'p>>,
    joints: &str,
    offset: Option<[f64; 2]>,
) -> PyResult<Option<Bound<'p, PyDict>>> {
    let (Some(k), Some((p, a, b, l, t))) = (key, band) else {
        return Ok(None);
    };
    if !m.getattr("COURSED")?.contains(k)? || !matches!(cut, "plan" | "vertical") {
        return Ok(None);
    }
    let unit = real_unit(m, Some(k), brick)?.unwrap();
    hatch_bond::masonry_marks(
        m.py(),
        k,
        (p.into(), a.into(), b.into(), l, t),
        cut,
        scale,
        super::scale(scale_n),
        dims(m, brick, piece_kind(m, Some(k))?)?,
        unit.into(),
        joints,
        offset,
    )
}
#[pyfunction(signature=(band,scale_n=100.,depth=None))]
pub(super) fn cut_across(band: Option<Band>, scale_n: f64, depth: Option<f64>) -> bool {
    let Some((_, _, _, l, t)) = band else {
        return false;
    };
    t > 1e-9
        && depth.map_or_else(
            || l * scale(scale_n).max(1e-6) <= 600. && l <= 6. * t,
            |d| d >= l,
        )
}
#[pyfunction]
pub(super) fn cross_marks(
    origin: [f64; 2],
    along: [f64; 2],
    across: [f64; 2],
    length: f64,
    thick: f64,
) -> Vec<Ring> {
    fitted_marks("wood_cross", origin, along, across, length, thick, 1.)
}
#[pyfunction(signature=(origin,along,across,length,thick,density=1.))]
pub(super) fn zigzag_marks(
    origin: [f64; 2],
    along: [f64; 2],
    across: [f64; 2],
    length: f64,
    thick: f64,
    density: f64,
) -> Vec<Ring> {
    fitted_marks(
        "insulation_zigzag",
        origin,
        along,
        across,
        length,
        thick,
        density,
    )
}
#[pyfunction(signature=(origin,along,across,length,thick,density=1.))]
pub(super) fn batt_marks(
    origin: [f64; 2],
    along: [f64; 2],
    across: [f64; 2],
    length: f64,
    thick: f64,
    density: f64,
) -> Vec<Ring> {
    fitted_marks("insulation", origin, along, across, length, thick, density)
}
fn get_number(d: &Bound<'_, PyDict>, k: &str, default: f64) -> PyResult<f64> {
    match d.get_item(k)? {
        Some(v) if v.is_truthy()? => number(&v),
        _ => Ok(default),
    }
}
#[pyfunction]
pub(super) fn paper_offset(form: Option<&Bound<'_, PyDict>>) -> PyResult<Option<Point>> {
    let Some(form) = form else { return Ok(None) };
    let Some(off) = form.get_item("offset")? else {
        return Ok(None);
    };
    if !off.is_truthy()? {
        return Ok(None);
    }
    let (x, y) = (number(&off.get_item(0)?)?, number(&off.get_item(1)?)?);
    let n = get_number(form, "scale_n", 100.)?.max(1e-6);
    Ok((x != 0. || y != 0.).then_some((x / n, -y / n)))
}

#[pyfunction(pass_module,signature=(key,r#box,scale=1.,angle=0.,families=None,offset=None))]
pub(super) fn hatch_marks<'p>(
    m: &Module<'p>,
    key: Option<&Obj<'p>>,
    r#box: [f64; 4],
    scale: f64,
    angle: f64,
    families: Option<&Obj<'p>>,
    offset: Option<[f64; 2]>,
) -> PyResult<Bound<'p, PyDict>> {
    let k = key.and_then(|v| v.extract::<&str>().ok()).unwrap_or("");
    if let Some(f) = families {
        return generate_hatch(
            m.py(),
            k,
            r#box,
            scale,
            angle,
            pat::clean_families(m.py(), Some(f))?.as_any(),
            offset,
            None,
            None,
            None,
        );
    }
    let rows = value(m, "_FAMILIES", k)?.unwrap_or_else(|| PyTuple::empty(m.py()).into_any());
    let turn = value(m, "_TURN", k)?
        .map(|v| v.extract::<f64>())
        .transpose()?
        .unwrap_or(0.);
    let scatter = value(m, "_SCATTERED", k)?;
    generate_hatch(
        m.py(),
        k,
        r#box,
        scale,
        angle + turn,
        &rows,
        offset,
        scatter.as_ref().map(|v| v.extract()).transpose()?,
        value(m, "_WAVES", k)?.map(|v| v.extract()).transpose()?,
        value(m, "FITTED", k)?.map(|v| v.extract()).transpose()?,
    )
}

fn rows<'p, T: FromPyObjectOwned<'p>>(marks: &Bound<'p, PyDict>, key: &str) -> PyResult<Vec<T>> {
    marks
        .get_item(key)?
        .map(|v| v.extract())
        .transpose()
        .map(Option::unwrap_or_default)
}
#[pyfunction]
pub(super) fn shift_marks<'p>(
    marks: &Bound<'p, PyDict>,
    dx: f64,
    dy: f64,
) -> PyResult<Bound<'p, PyDict>> {
    let out = marks.copy()?;
    out.set_item(
        "lines",
        rows::<[f64; 4]>(marks, "lines")?
            .into_iter()
            .map(|[x, y, u, v]| (x + dx, y + dy, u + dx, v + dy))
            .collect::<Vec<_>>(),
    )?;
    out.set_item(
        "dots",
        rows::<[f64; 2]>(marks, "dots")?
            .into_iter()
            .map(|[x, y]| (x + dx, y + dy))
            .collect::<Vec<_>>(),
    )?;
    out.set_item(
        "circles",
        rows::<[f64; 3]>(marks, "circles")?
            .into_iter()
            .map(|[x, y, r]| (x + dx, y + dy, r))
            .collect::<Vec<_>>(),
    )?;
    for key in ["polys", "closed", "bricks"] {
        if marks.contains(key)? {
            let mut paths = rows::<Vec<[f64; 2]>>(marks, key)?
                .into_iter()
                .map(|r| r.into_iter().map(Point::from).collect::<Ring>())
                .collect::<Vec<_>>();
            for (x, y) in paths.iter_mut().flatten() {
                *x += dx;
                *y += dy;
            }
            out.set_item(key, paths)?;
        }
    }
    Ok(out)
}

fn lines(points: &[Point]) -> impl Iterator<Item = Line> + '_ {
    points.windows(2).map(|p| (p[0].0, p[0].1, p[1].0, p[1].1))
}
#[pyfunction]
pub(super) fn _segments(marks: &Bound<'_, PyDict>) -> PyResult<Vec<Line>> {
    let mut out = rows::<[f64; 4]>(marks, "lines")?
        .into_iter()
        .map(Line::from)
        .collect::<Vec<_>>();
    for key in ["polys", "closed", "bricks"] {
        for mut path in rows::<Vec<[f64; 2]>>(marks, key)?
            .into_iter()
            .map(|r| r.into_iter().map(Point::from).collect::<Ring>())
            .collect::<Vec<_>>()
        {
            if key == "closed" && path.len() > 2 {
                path.push(path[0]);
            }
            out.extend(lines(&path));
        }
    }
    for [x, y, r] in rows::<[f64; 3]>(marks, "circles")? {
        let circle = (0..=16)
            .map(|i| {
                let a = std::f64::consts::TAU * f64::from(i) / 16.;
                (x + r * a.cos(), y + r * a.sin())
            })
            .collect::<Ring>();
        out.extend(lines(&circle));
    }
    Ok(out)
}

fn extract_iter<'p, T: FromPyObjectOwned<'p>>(v: &Obj<'p>) -> PyResult<Vec<T>> {
    v.try_iter()?
        .map(|row| row?.extract::<T>().map_err(Into::into))
        .collect()
}
#[pyfunction]
pub(super) fn clip_segments(lines: &Obj<'_>, rings: &Obj<'_>) -> PyResult<Vec<Line>> {
    Ok(hatch_region::hatch_clip_segments(
        extract_iter(lines)?,
        extract_iter(rings)?,
    ))
}
#[pyfunction]
pub(super) fn fill_quads(rings: &Obj<'_>) -> PyResult<Vec<(Point, Point, Point, Point)>> {
    Ok(hatch_region::hatch_fill_quads(extract_iter(rings)?))
}
fn clip(lines: Vec<Line>, rings: &[Ring]) -> Vec<Line> {
    hatch_region::hatch_clip_segments(
        lines.into_iter().map(Into::into).collect(),
        rings
            .iter()
            .map(|r| r.iter().copied().map(Into::into).collect())
            .collect(),
    )
}
fn fitted_lines(
    m: &Module<'_>,
    key: &str,
    (p, a, b, l, t): Band,
    ring: &Ring,
) -> PyResult<Vec<Line>> {
    let paths: Vec<Ring> = m
        .call_method1("fitted_marks", (key, p, a, b, l, t))?
        .extract()?;
    Ok(clip(
        paths.iter().flat_map(|p| lines(p)).collect(),
        std::slice::from_ref(ring),
    ))
}
fn fit(py: Python<'_>, r: &Ring) -> PyResult<Option<Band>> {
    fit_band(PyList::new(py, r)?.as_any())
}
#[pyfunction(pass_module,signature=(key,rings,form=None,frame_scale=1.,depths=None))]
pub(super) fn hatch_lines(
    m: &Module<'_>,
    key: Option<&str>,
    rings: &Obj<'_>,
    form: Option<&Bound<'_, PyDict>>,
    frame_scale: f64,
    depths: Option<&Obj<'_>>,
) -> PyResult<Vec<Line>> {
    let py = m.py();
    let Some(mut k) = key else { return Ok(vec![]) };
    let mut rings = rings
        .try_iter()?
        .map(|r| ring_points(&r?))
        .collect::<PyResult<Vec<_>>>()?;
    rings.retain(|r| r.len() >= 3);
    if k == "solid" || rings.is_empty() {
        return Ok(vec![]);
    }
    let empty = PyDict::new(py);
    let form = form.unwrap_or(&empty);
    let cut = form.get_item("cut")?;
    let cut = cut
        .as_ref()
        .and_then(|v| v.extract::<&str>().ok())
        .unwrap_or("");
    let n = get_number(form, "scale_n", 100.)?;
    let mut scale = frame_scale * get_number(form, "scale", 1.)?;
    let mut out = Vec::new();
    if m.getattr("WOOD")?.contains(k)?
        && !cut.is_empty()
        && form
            .get_item("cross")?
            .is_some_and(|v| v.is_truthy().unwrap_or(false))
    {
        let depths = match depths {
            Some(d) => extract_iter(d)?,
            None => vec![None; rings.len()],
        };
        let mut remaining = Vec::new();
        for (r, d) in rings.into_iter().zip(depths) {
            let band = fit(py, &r)?;
            if let Some(b) = band.filter(|_| cut_across(band, n, d)) {
                out.extend(fitted_lines(m, "wood_cross", b, &r)?);
            } else {
                remaining.push(r);
            }
        }
        rings = remaining;
        if rings.is_empty() {
            return Ok(out);
        }
    }
    if m.getattr("COURSED")?.contains(k)? && matches!(cut, "plan" | "vertical") {
        let joints = form.get_item("joints")?;
        let mode = joints_mode(joints.as_ref().and_then(|v| v.extract().ok()), n);
        match mode {
            "none" => return Ok(out),
            "hatch" => {
                k = "diagonal";
                scale = frame_scale * get_number(form, "hatch_scale", 1.)?;
            }
            _ => {
                let brick = form.get_item("brick")?;
                let offset = form
                    .get_item("offset")?
                    .map(|v| v.extract())
                    .transpose()?
                    .flatten();
                for r in &rings {
                    let b = if cut == "plan" {
                        fit(py, r)?
                    } else {
                        upright_band(r)
                    };
                    if let Some(marks) =
                        bond_marks(m, Some(k), b, cut, scale, n, brick.as_ref(), mode, offset)?
                    {
                        out.extend(clip(_segments(&marks)?, std::slice::from_ref(r)));
                    }
                }
                return Ok(out);
            }
        }
    }
    if m.getattr("FITTED")?.contains(k)? {
        for r in &rings {
            if let Some(b) = fit(py, r)? {
                out.extend(fitted_lines(m, k, b, r)?);
            }
        }
        return Ok(out);
    }
    let area = bounds(&rings.iter().flatten().copied().collect::<Ring>());
    let families = if !m.getattr("COURSED")?.contains(k)? || m.getattr("BRICKS")?.contains(k)? {
        form.get_item("families")?
    } else {
        None
    };
    let offset = if m.getattr("PLACED")?.contains(k)? {
        paper_offset(Some(form))?.map(Into::into)
    } else {
        None
    };
    let key = k.into_pyobject(py)?.into_any();
    let marks = hatch_marks(
        m,
        Some(&key),
        area,
        scale,
        get_number(form, "angle", 0.)?,
        families.filter(|v| !v.is_none()).as_ref(),
        offset,
    )?;
    out.extend(clip(_segments(&marks)?, &rings));
    Ok(out)
}
