// SPDX-License-Identifier: GPL-3.0-or-later
//! Paper hatch catalogue and scripting entry points shared by Drawing and Sheets.
mod paper;
mod pat;
use crate::{hatch_bond, hatch_pattern, hatch_region};
use pyo3::{
    prelude::*,
    types::{PyDict, PyFloat, PyFrozenSet, PyList, PyTuple},
};
type Obj<'p> = Bound<'p, PyAny>;
type Module<'p> = Bound<'p, PyModule>;

fn tuples<'p>(value: Obj<'p>) -> PyResult<Obj<'p>> {
    if let Ok(array) = value.cast::<PyList>() {
        let items = array.iter().map(tuples).collect::<PyResult<Vec<_>>>()?;
        Ok(PyTuple::new(value.py(), items)?.into_any())
    } else {
        Ok(value)
    }
}

fn catalog(m: &Module<'_>) -> PyResult<()> {
    let py = m.py();
    let raw = py
        .import("json")?
        .call_method1("loads", (include_str!("catalog.json"),))?
        .cast_into::<PyDict>()?;
    for (name, value) in &raw {
        let name = name.extract::<String>()?;
        if let Ok(rows) = value.cast::<PyDict>() {
            let table = PyDict::new(py);
            for (key, value) in rows {
                if name == "_FAMILIES" {
                    let families = PyList::empty(py);
                    for row in value.try_iter()? {
                        families.append(tuples(row?)?)?;
                    }
                    table.set_item(key, families)?;
                } else {
                    table.set_item(key, tuples(value)?)?;
                }
            }
            m.add(name.as_str(), table)?;
        } else {
            m.add(name.as_str(), tuples(value)?)?;
        }
    }
    let groups: Vec<(String, Vec<String>)> = m.getattr("HATCH_GROUPS")?.extract()?;
    m.add(
        "HATCH_ORDER",
        PyTuple::new(
            py,
            groups.iter().flat_map(|(_, rows)| rows).collect::<Vec<_>>(),
        )?,
    )?;
    let wood = groups
        .iter()
        .find(|(title, _)| title == "Wood")
        .unwrap()
        .1
        .iter()
        .filter(|k| k.as_str() != "wood_cross")
        .collect::<Vec<_>>();
    m.add("WOOD", PyTuple::new(py, wood)?)?;
    let sizes = m.getattr("PIECE_SIZES")?.cast_into::<PyDict>()?;
    let defaults = PyDict::new(py);
    for (key, row) in &sizes {
        defaults.set_item(key, row.get_item(0)?.get_item(1)?)?;
    }
    m.add("BRICK_SIZES", sizes.get_item("brick")?.unwrap())?;
    m.add("BRICK_DEFAULT", defaults.get_item("brick")?.unwrap())?;
    m.add("PIECE_DEFAULT", defaults)?;
    let mut coursed: Vec<String> = m.getattr("BRICKS")?.extract()?;
    for key in &coursed {
        m.getattr("_FAMILIES")?
            .set_item(key, brick_families(m, key, None)?)?;
        m.getattr("REAL_SIZE")?.set_item(key, (1.5, 0.075))?;
    }
    coursed.extend(["block", "aac_block", "ashlar"].map(str::to_owned));
    m.add("COURSED", PyFrozenSet::new(py, &coursed)?)?;
    coursed.extend(m.getattr("PLACED")?.extract::<Vec<String>>()?);
    m.setattr("PLACED", PyFrozenSet::new(py, &coursed)?)?;
    m.add("IMPORTED", "pat:")?;
    m.add("JOINTS", ("single", "double", "none"))?;
    m.add("JOINTS_AUTO", (15., 75.))?;
    m.add("_BATT_RADIUS", 0.22)?;
    Ok(())
}

fn value<'p>(m: &Module<'p>, dict: &str, key: &str) -> PyResult<Option<Obj<'p>>> {
    m.getattr(dict)?.cast_into::<PyDict>()?.get_item(key)
}
fn number(p: &Obj<'_>) -> PyResult<f64> {
    p.py().get_type::<PyFloat>().call1((p,))?.extract()
}

#[pyfunction(pass_module)]
fn piece_kind<'a>(m: &Module<'_>, key: Option<&'a str>) -> PyResult<Option<&'static str>> {
    Ok(match key {
        Some("block") => Some("block"),
        Some("aac_block") => Some("aac"),
        Some("hollow_brick") => Some("hollow"),
        Some("perforated_block") => Some("perforated"),
        Some("concrete_brick") => Some("cored"),
        Some(k) if m.getattr("BRICKS")?.contains(k)? => Some("brick"),
        _ => None,
    })
}

fn size(raw: Option<&Obj<'_>>) -> Option<Vec<f64>> {
    let raw = raw?;
    let vals = raw
        .try_iter()
        .ok()?
        .map(|v| number(&v?))
        .collect::<PyResult<Vec<_>>>()
        .ok()?;
    let limits = [(60., 600.), (30., 400.), (20., 400.), (0., 40.), (0., 40.)];
    if !(4..=5).contains(&vals.len())
        || vals[1] > vals[0]
        || !vals
            .iter()
            .zip(limits)
            .all(|(v, (a, b))| (a..=b).contains(v))
    {
        return None;
    }
    let equal = vals.len() == 5 && (vals[3] - vals[4]).abs() < 1e-9;
    Some(if equal { vals[..4].to_vec() } else { vals })
}
#[pyfunction]
fn clean_brick<'p>(py: Python<'p>, raw: Option<&Obj<'p>>) -> PyResult<Option<Bound<'p, PyTuple>>> {
    size(raw).map(|v| PyTuple::new(py, v)).transpose()
}
fn dims(m: &Module<'_>, brick: Option<&Obj<'_>>, kind: Option<&str>) -> PyResult<[f64; 5]> {
    let vals = match size(brick) {
        Some(v) => v,
        None => value(m, "PIECE_DEFAULT", kind.unwrap_or("brick"))?
            .unwrap_or(m.getattr("BRICK_DEFAULT")?)
            .extract()?,
    };
    Ok([vals[0], vals[1], vals[2], vals[3], *vals.last().unwrap()])
}
#[pyfunction(pass_module, signature=(brick=None,kind="brick"))]
fn brick_dims<'p>(
    m: &Module<'p>,
    brick: Option<&Obj<'_>>,
    kind: &str,
) -> PyResult<Bound<'p, PyTuple>> {
    PyTuple::new(m.py(), dims(m, brick, Some(kind))?)
}
#[pyfunction(pass_module,signature=(key,brick=None))]
fn real_unit(
    m: &Module<'_>,
    key: Option<&str>,
    brick: Option<&Obj<'_>>,
) -> PyResult<Option<(f64, f64)>> {
    let Some(key) = key else { return Ok(None) };
    let Some(v) = value(m, "REAL_SIZE", key)? else {
        return Ok(None);
    };
    let (unit, length) = v.extract()?;
    let kind = piece_kind(m, Some(key))?;
    Ok(Some((
        unit,
        if kind.is_some() {
            let b = dims(m, brick, kind)?;
            (b[2] + b[3]) / 1000.
        } else {
            length
        },
    )))
}
#[pyfunction(pass_module,signature=(key,scale_n,brick=None))]
fn real_scale(
    m: &Module<'_>,
    key: Option<&str>,
    scale_n: f64,
    brick: Option<&Obj<'_>>,
) -> PyResult<Option<f64>> {
    Ok(real_unit(m, key, brick)?
        .map(|(unit, length)| (length * 1000. / scale(scale_n).max(1e-6) / unit).clamp(0.25, 12.)))
}
fn scale(n: f64) -> f64 {
    if n == 0. {
        100.
    } else {
        n
    }
}

#[pyfunction(pass_module,signature=(key,brick=None))]
fn brick_families<'p>(
    m: &Module<'p>,
    key: &str,
    brick: Option<&Obj<'_>>,
) -> PyResult<Bound<'p, PyList>> {
    let [length, width, height, bed, head] = dims(m, brick, piece_kind(m, Some(key))?)?;
    let h = 1.5;
    let u = h / (height + bed);
    let (l, w) = ((length + head) * u, (width + head) * u);
    let list = PyList::empty(m.py());
    let row = |angle: f64, x: f64, y: f64, along: f64, spacing: f64, d: &[f64]| {
        list.append((angle, x, y, along, spacing, PyTuple::new(m.py(), d)?))
    };
    let course = if matches!(key, "brick_on_edge" | "brick_rowlock") {
        (width + bed) * u
    } else {
        h
    };
    row(0., 0., 0., 0., course, &[])?;
    let dash = [course, -course];
    let vertical = |x, y, a, s, d: &[f64]| row(90., x, y, a, s, d);
    match key {
        "brick_rowlock" => vertical(0., 0., 0., (height + head) * u, &[])?,
        "brick_header" => vertical(0., 0., h, w / 2., &dash)?,
        "brick_bond" | "brick_one_half" => {
            vertical(0., 0., 0., l, &dash)?;
            vertical(w / 2., h, 0., w, &dash)?;
        }
        "brick_cross" => {
            vertical(0., 0., 0., l, &[h, -3. * h])?;
            vertical(l / 2., 2. * h, 0., l, &[h, -3. * h])?;
            vertical(w / 2., h, 0., w, &dash)?;
        }
        "brick_flemish" | "brick_dutch" => {
            let period = l + w;
            vertical(0., 0., 0., period, &dash)?;
            vertical(l, 0., 0., period, &dash)?;
            if key == "brick_dutch" {
                vertical(w / 2., h, 0., w, &dash)?;
            } else {
                vertical(l / 2. - w / 2., h, 0., period, &dash)?;
                vertical(l / 2. + w / 2., h, 0., period, &dash)?;
            }
        }
        "brick_honeycomb" => {
            let gap = l / 2.;
            let half = (l + gap) / 2.;
            vertical(0., 0., h, half, &dash)?;
            vertical(gap, 0., h, half, &dash)?;
        }
        _ => {
            vertical(0., 0., course, l / 2., &dash)?;
            if key == "hollow_brick" {
                let k = l / 6.;
                row(
                    0.,
                    0.6 * k,
                    h / 2.,
                    l / 2.,
                    h,
                    &[0.9, -0.4, 0.9, -0.4, 0.9, -2.5].map(|v| v * k),
                )?;
            }
        }
    }
    Ok(list)
}

#[pyfunction(pass_module)]
fn known(m: &Module<'_>, key: Option<&Obj<'_>>) -> PyResult<bool> {
    let Some(k) = key else { return Ok(true) };
    Ok(m.getattr("HATCH_LABELS")?.contains(k)?
        || k.extract::<&str>()
            .is_ok_and(|s| s.starts_with("pat:") && s.len() > 4))
}

pub(super) fn register(root: &Module<'_>) -> PyResult<()> {
    let m = PyModule::new(root.py(), "built_core.hatches")?;
    catalog(&m)?;
    macro_rules! add {($($f:path),*)=>{$(m.add_function(wrap_pyfunction!($f,&m)?)?;)*};}
    add!(
        piece_kind,
        clean_brick,
        brick_dims,
        brick_families,
        real_unit,
        real_scale,
        known,
        hatch_bond::masonry_course,
        hatch_bond::masonry_marks,
        hatch_pattern::fit_band,
        hatch_pattern::fitted_marks,
        hatch_pattern::generate_hatch,
        hatch_region::hatch_clip_segments,
        hatch_region::hatch_fill_quads,
        pat::parse_pat,
        pat::clean_families,
        pat::line_families,
        pat::to_pat,
        paper::vertical_band,
        paper::joints_mode,
        paper::bond_marks,
        paper::bond_course,
        paper::cut_across,
        paper::hatch_marks,
        paper::cross_marks,
        paper::zigzag_marks,
        paper::batt_marks,
        paper::paper_offset,
        paper::shift_marks,
        paper::_segments,
        paper::clip_segments,
        paper::fill_quads,
        paper::hatch_lines
    );
    root.add_submodule(&m)
}
