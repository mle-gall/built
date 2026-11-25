// SPDX-License-Identifier: GPL-3.0-or-later
//! Fully editable letter solids from shaped outlines. The back face stays at
//! local y=0 when a sign's thickness is edited.
use crate::math::{xyz, Vec3};
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
use std::collections::BTreeMap;
mod groups;
type Obj<'p> = Bound<'p, PyAny>;
type Point = [f64; 2];
type Ring = Vec<Point>;
type Glyph = Vec<Ring>;

fn area(ring: &Ring) -> f64 {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum::<f64>()
        / 2.
}
fn contains([x, y]: Point, ring: &Ring) -> bool {
    let mut inside = false;
    for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
        if (a[1] > y) != (b[1] > y) && x < a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
            inside = !inside;
        }
    }
    inside
}
fn outlines(
    py: Python<'_>,
    text: &str,
    family: &str,
    bold: bool,
    italic: bool,
) -> PyResult<BTreeMap<usize, Glyph>> {
    py.import("ingetrazo_render")?
        .call_method1("text_outlines", (text, family, bold, italic))?
        .extract()
}
fn scale(paths: &BTreeMap<usize, Glyph>, height: f64) -> Option<(f64, f64)> {
    let (min, max) = paths
        .values()
        .flatten()
        .flatten()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), pt| {
            (min.min(pt[1]), max.max(pt[1]))
        });
    min.is_finite()
        .then(|| (height / (max - min).max(1e-9), min))
}
fn solid<'p>(
    py: Python<'p>,
    glyph: &[&Ring],
    scale: f64,
    base: f64,
    thickness: f64,
) -> PyResult<Obj<'p>> {
    let mesh = py.import("ingetrazo_core.mesh")?.call_method0("Mesh")?;
    let rings: Vec<_> = glyph.iter().copied().filter(|r| r.len() >= 3).collect();
    let depths: Vec<_> = rings
        .iter()
        .enumerate()
        .map(|(i, r)| {
            rings
                .iter()
                .enumerate()
                .filter(|(j, c)| *j != i && contains(r[0], c))
                .count()
        })
        .collect();
    let parents: Vec<_> = rings
        .iter()
        .enumerate()
        .map(|(hole, ring)| {
            if depths[hole] % 2 == 0 {
                return None;
            }
            rings.iter().enumerate().position(|(outer, candidate)| {
                depths[outer] + 1 == depths[hole] && contains(ring[0], candidate)
            })
        })
        .collect();
    let vertex =
        |[x, y]: Point, depth: f64| Vec3::from(DVec3::new(x * scale, depth, (y - base) * scale));
    for (i, outer) in rings
        .iter()
        .enumerate()
        .filter(|(i, _)| depths[*i] % 2 == 0)
    {
        let mut contours = vec![(*outer).clone()];
        if area(&contours[0]) < 0. {
            contours[0].reverse();
        }
        for (j, h) in rings.iter().enumerate() {
            if parents[j] == Some(i) {
                let mut hole = (*h).clone();
                if area(&hole) >= 0. {
                    hole.reverse();
                }
                contours.push(hole);
            }
        }
        let cap = |depth: f64, reverse: bool| -> PyResult<()> {
            let mut loops = contours
                .iter()
                .map(|r| {
                    let mut pts = r.iter().map(|&p| vertex(p, depth)).collect::<Vec<_>>();
                    if reverse {
                        pts.reverse();
                    }
                    pts
                })
                .collect::<Vec<_>>();
            let outer = loops.remove(0);
            mesh.call_method1("add_face", (outer, (!loops.is_empty()).then_some(loops)))?;
            Ok(())
        };
        cap(-thickness, false)?;
        if thickness > 1e-9 {
            cap(0., true)?;
            for ring in &contours {
                for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                    mesh.call_method1(
                        "add_face",
                        (vec![
                            vertex(a, -thickness),
                            vertex(a, 0.),
                            vertex(b, 0.),
                            vertex(b, -thickness),
                        ],),
                    )?;
                }
            }
        }
    }
    for edge in mesh.getattr("edges")?.try_iter()? {
        let e = edge?;
        let fs = e.getattr("faces")?;
        if fs.len()? != 2 {
            continue;
        }
        let a = xyz(&fs.get_item(0)?.call_method0("normal")?)?;
        let b = xyz(&fs.get_item(1)?.call_method0("normal")?)?;
        if a.y.abs() < 0.5 && b.y.abs() < 0.5 && a.dot(b) > 0.85 {
            e.setattr("soft", true)?;
        }
    }
    Ok(mesh)
}
#[pyfunction]
#[pyo3(signature=(text,font_family="",bold=false,italic=false,height=0.25,thickness=0.05))]
fn build_text_mesh<'p>(
    py: Python<'p>,
    text: &str,
    font_family: &str,
    bold: bool,
    italic: bool,
    height: f64,
    thickness: f64,
) -> PyResult<Obj<'p>> {
    let shapes = outlines(py, text, font_family, bold, italic)?;
    let Some((factor, base)) = scale(&shapes, height) else {
        return py.import("ingetrazo_core.mesh")?.call_method0("Mesh");
    };
    solid(
        py,
        &shapes.values().flatten().collect::<Vec<_>>(),
        factor,
        base,
        thickness,
    )
}
#[pyfunction]
#[pyo3(signature=(text,font_family="",bold=false,italic=false,height=0.25,thickness=0.05))]
fn build_text_letters<'p>(
    py: Python<'p>,
    text: &str,
    font_family: &str,
    bold: bool,
    italic: bool,
    height: f64,
    thickness: f64,
) -> PyResult<Vec<(char, Obj<'p>)>> {
    let shapes = outlines(py, text, font_family, bold, italic)?;
    let Some((factor, base)) = scale(&shapes, height) else {
        return Ok(vec![]);
    };
    let letters = text.chars().collect::<Vec<_>>();
    let mut out = vec![];
    for (i, glyph) in shapes {
        if glyph.is_empty() {
            continue;
        }
        let mesh = solid(
            py,
            &glyph.iter().collect::<Vec<_>>(),
            factor,
            base,
            thickness,
        )?;
        if mesh.getattr("faces")?.is_truthy()? {
            out.push((letters[i], mesh));
        }
    }
    Ok(out)
}
pub(crate) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(build_text_mesh, m)?)?;
    m.add_function(wrap_pyfunction!(build_text_letters, m)?)?;
    groups::install(m)
}
