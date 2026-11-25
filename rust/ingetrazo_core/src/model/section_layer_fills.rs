// SPDX-License-Identifier: GPL-3.0-or-later
//! Group cut rings by their owning layer, then project its paper hatch back
//! into the plane. Unpatterned layers share the first draw batch.
use super::*;
use crate::model::{hlr_view, tag_styles};
use pyo3::types::PyDict;
use std::collections::BTreeMap;
type Obj<'p> = Bound<'p, PyAny>;
const MODULE: &str = "ingetrazo_core.section";
const SCALE: f64 = 50.;
const MOST: usize = 250000;
type Rgb = (f64, f64, f64);
fn rgb(v: &Obj) -> PyResult<Rgb> {
    Ok((
        v.get_item(0)?.extract()?,
        v.get_item(1)?.extract()?,
        v.get_item(2)?.extract()?,
    ))
}

#[pyfunction(signature=(chords,tags,normal,scene,color,scale_n=SCALE))]
fn layer_fills<'p>(
    chords: &Obj<'p>,
    tags: &Obj<'p>,
    normal: &Obj<'p>,
    scene: &Obj<'p>,
    color: &Obj<'p>,
    scale_n: f64,
) -> PyResult<Bound<'p, PyList>> {
    let py = chords.py();
    let result = PyList::empty(py);
    let data = chords_of(chords)?;
    if data.is_empty() {
        return Ok(result);
    }
    let tagged = hlr_view::hlr_closing_tags(py, chords, tags)?;
    if data.len() != tagged.len() * 6 {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "expected one tag per cut chord",
        ));
    }
    let mut layers = BTreeMap::<String, Vec<f64>>::new();
    for (segment, tag) in data.chunks_exact(6).zip(tagged.iter()) {
        if let Ok(tag) = tag.extract::<String>() {
            layers.entry(tag).or_default().extend_from_slice(segment);
        }
    }
    let color = rgb(color)?;
    let normal = xyz(normal)?;
    let patterns = tag_styles::cut_hatches(py, scene)?;
    let forms = tag_styles::cut_hatch_forms(py, scene, Some(scale_n.into_pyobject(py)?.as_any()))?;
    let k = 1000. / if scale_n == 0. { SCALE } else { scale_n };
    let (u, v) = sheet_basis(normal);
    let n = cross(&u, &v);
    let cut = if normal[2].abs() >= 0.7 {
        "plan"
    } else {
        "vertical"
    };
    let hatch = py
        .import("ingetrazo_core")?
        .getattr("hatches")?
        .getattr("hatch_lines")?;
    let mut plain = Vec::new();
    let mut budget = MOST;
    for (tag, chords) in layers {
        let Some(key) = patterns.get_item(&tag)?.filter(|k| !k.is_none()) else {
            plain.extend(chords);
            continue;
        };
        if !key.is_truthy()? {
            plain.extend(chords);
            continue;
        }
        let (groups, _) = fill_groups(&chords, normal);
        let tris = group_triangles(&groups);
        if tris.is_empty() {
            continue;
        }
        let tris = tris_array(py, tris)?;
        let layer = if let Some(get) = scene.getattr_opt("layer")? {
            Some(get.call1((&tag,))?).filter(|x| !x.is_none())
        } else {
            None
        };
        let layer_color = |ink| -> PyResult<Option<Rgb>> {
            let Some(l) = layer.as_ref() else {
                return Ok(None);
            };
            let c = if ink {
                tag_styles::hatch_ink_rgb(py, l)?
            } else {
                tag_styles::hatch_ground_rgb(py, l)?
            };
            if c.is_none() {
                Ok(None)
            } else {
                rgb(&c).map(Some)
            }
        };
        let ink = layer_color(true)?.unwrap_or(color);
        if key.eq("solid")? {
            result.append((tris, ink, py.None(), ink))?;
            continue;
        }
        let pale = |c: f64| c + (1. - c) * 0.85;
        let ground = layer_color(false)?.unwrap_or_else(|| (pale(ink.0), pale(ink.1), pale(ink.2)));
        let form = match forms.get_item(&tag)? {
            Some(f) => f.cast::<PyDict>()?.copy()?,
            None => PyDict::new(py),
        };
        form.set_item("cut", cut)?;
        let height = dot3(&groups[0].world[0][0], &n);
        let mut lines = vec![];
        for group in groups {
            if budget == 0 {
                break;
            }
            let paper: Vec<Vec<(f64, f64)>> = group
                .world
                .iter()
                .map(|r| {
                    r.iter()
                        .map(|p| (dot3(p, &u) * k, -dot3(p, &v) * k))
                        .collect()
                })
                .collect();
            let segments = hatch
                .call1((&key, paper, &form))?
                .extract::<Vec<[f64; 4]>>()?;
            budget = budget.saturating_sub(segments.len());
            for seg in segments {
                for xy in seg.chunks_exact(2) {
                    for axis in 0..3 {
                        lines.push(
                            (height * n[axis] + xy[0] / k * u[axis] - xy[1] / k * v[axis]) as f32,
                        );
                    }
                }
            }
        }
        let count = lines.len() / 6;
        result.append((
            tris,
            ground,
            if count == 0 {
                None
            } else {
                Some(arr(py, lines, &[count, 2, 3])?)
            },
            ink,
        ))?;
    }
    if !plain.is_empty() {
        let tris = group_triangles(&fill_groups(&plain, normal).0);
        if !tris.is_empty() {
            result.insert(0, (tris_array(py, tris)?, color, py.None(), color))?;
        }
    }
    Ok(result)
}

#[pyfunction(name="__init__",signature=(plane,point,normal,name="",symbol="",active=false,uid=None,hidden=false))]
#[allow(clippy::too_many_arguments)]
fn init(
    plane: &Obj,
    point: &Obj,
    normal: &Obj,
    name: &str,
    symbol: &str,
    #[pyo3(from_py_with = Bound::is_truthy)] active: bool,
    uid: Option<&Obj>,
    #[pyo3(from_py_with = Bound::is_truthy)] hidden: bool,
) -> PyResult<()> {
    let py = plane.py();
    let vec = py.import("ingetrazo_core")?.getattr("Vec3")?;
    plane.setattr("point", vec.call1((point,))?)?;
    let mut n = vec.call1((normal,))?;
    if n.call_method0("length")?.lt(1e-12)? {
        n = vec.call1((0., 0., 1.))?;
    }
    plane.setattr("normal", n.call_method0("normalized")?)?;
    plane.setattr("name", name)?;
    plane.setattr("symbol", symbol)?;
    plane.setattr("active", active)?;
    plane.setattr("hidden", hidden)?;
    plane.setattr("fill", py.None())?;
    plane.setattr("layer", py.None())?;
    let module = py.import(MODULE)?;
    let last = module.getattr("_last_uid")?;
    let id = if let Some(value) = uid {
        py.get_type::<pyo3::types::PyInt>().call1((value,))?
    } else {
        last.add(1)?
    };
    if id.gt(last)? {
        module.setattr("_last_uid", &id)?;
    }
    plane.setattr("uid", id)
}
#[pyfunction]
fn flip(plane: &Obj) -> PyResult<()> {
    plane.setattr("normal", plane.getattr("normal")?.neg()?)
}
#[pyfunction]
fn side(plane: &Obj, p: &Obj) -> PyResult<f64> {
    Ok(crate::math::xyz(&plane.getattr("normal")?)?
        .dot(crate::math::xyz(p)? - crate::math::xyz(&plane.getattr("point")?)?))
}
#[pyfunction(name = "__repr__")]
fn repr(plane: &Obj) -> PyResult<String> {
    let mut label = plane.getattr("symbol")?;
    if !label.is_truthy()? {
        label = plane.getattr("uid")?;
    }
    Ok(format!(
        "SectionPlane({}{})",
        label.str()?,
        if plane.getattr("active")?.is_truthy()? {
            " ACTIVE"
        } else {
            ""
        }
    ))
}
#[pyfunction(name = "__getattr__")]
fn lazy<'p>(py: Python<'p>, name: &str) -> PyResult<Obj<'p>> {
    if name != "SectionPlane" {
        return Err(pyo3::exceptions::PyAttributeError::new_err(name.to_owned()));
    }
    let m = py.import(MODULE)?;
    let cls = crate::record_class::build(
        &m,
        name,
        &[
            "point", "normal", "name", "symbol", "active", "uid", "hidden", "fill", "layer",
        ],
        true,
        vec![
            wrap_pyfunction!(init, &m)?,
            wrap_pyfunction!(flip, &m)?,
            wrap_pyfunction!(side, &m)?,
            wrap_pyfunction!(repr, &m)?,
        ],
        &[],
    )?;
    crate::record_class::bind_methods(
        &cls,
        &[
            ("to_dict", "section_plane_archive", false),
            ("from_dict", "section_plane_load", true),
        ],
    )?;
    Ok(cls)
}
pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    let sub = PyModule::new(py, MODULE)?;
    sub.add("_last_uid", 0)?;
    sub.add("FILL_PATTERNS", PyTuple::new(py, FILL_PATTERNS)?)?;
    sub.add("DEFAULT_HATCH_SPACING", 0.05)?;
    sub.add("LAYER_HATCH_SCALE", SCALE)?;
    sub.add("LAYER_HATCH_MOST", MOST)?;
    sub.add_function(wrap_pyfunction!(lazy, &sub)?)?;
    sub.add_function(wrap_pyfunction!(layer_fills, &sub)?)?;
    sub.setattr(
        "__all__",
        [
            "SectionPlane",
            "FILL_PATTERNS",
            "DEFAULT_HATCH_SPACING",
            "LAYER_HATCH_SCALE",
            "LAYER_HATCH_MOST",
            "layer_fills",
        ],
    )?;
    py.import("sys")?
        .getattr("modules")?
        .set_item(MODULE, &sub)?;
    m.add("section", sub)
}
