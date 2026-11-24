// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The Python face of `sheets`: what `core/hlr.py`, `formats/svg_out.py`
//! and the Sheets workspace adapter (`views/wgpu_sheets.py`) call.

use super::{drawing, model, page, svg};
use numpy::{PyArray1, PyArray2, PyArray3, PyArrayMethods, PyReadonlyArray1, PyReadonlyArray2, PyReadonlyArray3};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

fn rows3(a: &numpy::ndarray::ArrayView2<'_, f64>) -> PyResult<Vec<[f64; 3]>> {
    if a.ncols() != 3 {
        return Err(PyValueError::new_err("expected (N,3) points"));
    }
    Ok((0..a.nrows()).map(|i| [a[[i, 0]], a[[i, 1]], a[[i, 2]]]).collect())
}

fn rows2(a: &numpy::ndarray::ArrayView2<'_, f64>) -> PyResult<Vec<[f64; 2]>> {
    if a.ncols() != 2 {
        return Err(PyValueError::new_err("expected (N,2) points"));
    }
    Ok((0..a.nrows()).map(|i| [a[[i, 0]], a[[i, 1]]]).collect())
}

/// The drawing pass of `core.hlr.hlr_drawing` from its camera-space edges:
/// ``a``/``b`` (E,3) camera-space endpoints, ``world`` (E,2,3), rows
/// ``[0, n_hard)`` hard edges then ``n_sil`` silhouettes then cut chords,
/// ``tris`` (T,3,3) camera-space occluders, the section cap as stacked
/// ring edges ``cap_p``/``cap_q`` (K,2) with its depth plane ``cap_abg``,
/// and a tag id per row (``tags``, rows of other tags never merge).
/// Returns ``(segs (N,4), world (N,2,3), kinds (N,) int8, rows (N,))``.
#[pyfunction]
#[pyo3(signature = (a, b, world, n_hard, n_sil, tris, profiles, hidden, cap_p=None, cap_q=None, cap_abg=None, tags=None))]
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn hlr_lines<'py>(
    py: Python<'py>,
    a: PyReadonlyArray2<'_, f64>,
    b: PyReadonlyArray2<'_, f64>,
    world: PyReadonlyArray3<'_, f64>,
    n_hard: usize,
    n_sil: usize,
    tris: PyReadonlyArray3<'_, f64>,
    profiles: bool,
    hidden: bool,
    cap_p: Option<PyReadonlyArray2<'_, f64>>,
    cap_q: Option<PyReadonlyArray2<'_, f64>>,
    cap_abg: Option<PyReadonlyArray1<'_, f64>>,
    tags: Option<PyReadonlyArray1<'_, i64>>,
) -> PyResult<(Bound<'py, PyArray2<f64>>, Bound<'py, PyArray3<f64>>, Bound<'py, PyArray1<i8>>, Bound<'py, PyArray1<i64>>)> {
    let a = rows3(&a.as_array())?;
    let b = rows3(&b.as_array())?;
    let w = world.as_array();
    let t = tris.as_array();
    if a.len() != b.len() || w.shape() != [a.len(), 2, 3] || n_hard + n_sil > a.len() {
        return Err(PyValueError::new_err("edges: a, b (E,3) and world (E,2,3) must agree"));
    }
    if t.shape()[0] > 0 && (t.shape()[1] != 3 || t.shape()[2] != 3) {
        return Err(PyValueError::new_err("tris must be (T,3,3)"));
    }
    let world: Vec<[[f64; 3]; 2]> = (0..a.len())
        .map(|i| [[w[[i, 0, 0]], w[[i, 0, 1]], w[[i, 0, 2]]], [w[[i, 1, 0]], w[[i, 1, 1]], w[[i, 1, 2]]]])
        .collect();
    let tris: Vec<[[f64; 3]; 3]> = (0..t.shape()[0])
        .map(|i| std::array::from_fn(|k| [t[[i, k, 0]], t[[i, k, 1]], t[[i, k, 2]]]))
        .collect();
    let cap = match (cap_p, cap_q, cap_abg) {
        (Some(p), Some(q), Some(abg)) => {
            let abg = abg.as_slice()?;
            if abg.len() != 3 {
                return Err(PyValueError::new_err("cap_abg: three values"));
            }
            Some(drawing::Cap { p: rows2(&p.as_array())?, q: rows2(&q.as_array())?, abg: [abg[0], abg[1], abg[2]] })
        }
        _ => None,
    };
    let tags: Option<Vec<i64>> = tags.map(|t| t.as_slice().map(<[i64]>::to_vec)).transpose()?;
    if tags.as_ref().is_some_and(|t| t.len() != a.len()) {
        return Err(PyValueError::new_err("one tag per edge"));
    }
    let d = py.detach(|| {
        drawing::drawing(&drawing::Input {
            a: &a, b: &b, world: &world, n_hard, n_sil, tris, profiles, hidden, cap,
            tags: tags.as_deref(),
        })
    });
    let n = d.segs.len();
    let segs = PyArray1::from_vec(py, d.segs.into_iter().flatten().collect()).reshape([n, 4])?;
    let world = PyArray1::from_vec(py, d.world.into_iter().flatten().flatten().collect()).reshape([n, 2, 3])?;
    Ok((segs, world, PyArray1::from_vec(py, d.kinds), PyArray1::from_vec(py, d.rows)))
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(hlr_lines, m)?)?;
    svg::register(m)?;
    model::register(m)?;
    page::register(m)?;
    super::display::register(m)?;
    m.add_function(wrap_pyfunction!(super::camera::sheet_frame_camera, m)?)?;
    m.add_function(wrap_pyfunction!(super::visibility::sheet_frame_hidden_tags, m)?)?;
    m.add_function(wrap_pyfunction!(super::visibility::sheet_frame_hidden, m)?)?;
    m.add_function(wrap_pyfunction!(super::geometry::sheet_frame_geometry, m)?)?;
    m.add_function(wrap_pyfunction!(super::profile::sheet_terrain_profiles, m)?)?;
    m.add_function(wrap_pyfunction!(super::pdf::sheet_pdf, m)?)?;
    m.add_function(wrap_pyfunction!(super::dxf::sheet_dxf, m)?)?;
    m.add_function(wrap_pyfunction!(super::dxf::sheet_dxf_ltypes, m)?)?;
    m.add_function(wrap_pyfunction!(super::raster::sheet_image, m)?)?;
    m.add_function(wrap_pyfunction!(super::create::sheet_new_item, m)?)?;
    m.add_function(wrap_pyfunction!(super::create::sheet_new_cota, m)?)?;
    m.add_function(wrap_pyfunction!(super::create::sheet_new_chain_total, m)?)?;
    m.add_function(wrap_pyfunction!(super::create::sheet_new_radial, m)?)?;
    m.add_function(wrap_pyfunction!(super::create::sheet_new_angular, m)?)?;
    m.add_function(wrap_pyfunction!(super::create::sheet_new_image, m)?)?;
    m.add_function(wrap_pyfunction!(super::create::sheet_from_template, m)?)?;
    m.add_function(wrap_pyfunction!(super::anchors::sheet_follow_anchors, m)?)?;
    m.add_function(wrap_pyfunction!(super::annots::sheet_frame_annotations, m)?)?;
    m.add_function(wrap_pyfunction!(super::annots::sheet_frame_annots, m)?)?;
    m.add_class::<super::annots::SheetContext>()?;
    m.add_function(wrap_pyfunction!(super::looks::sheet_remember_look, m)?)?;
    m.add_function(wrap_pyfunction!(super::looks::sheet_restore_looks, m)?)?;
    m.add_function(wrap_pyfunction!(super::looks::sheet_look, m)?)?;
    m.add_function(wrap_pyfunction!(super::looks::sheet_forget_looks, m)?)?;
    Ok(())
}
