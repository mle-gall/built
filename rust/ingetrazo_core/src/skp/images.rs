// SPDX-License-Identifier: GPL-3.0-or-later
//! Image entities cover their entire local quad. A cutout which lies on
//! an actual support polygon remains attached when the camera moves.
use super::{
    annotations::placed_matrix,
    surface::{skp_positioned_uvs, skp_ring},
};
use crate::math::{xyz, Vec3};
use glam::{DMat4, DVec3};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::{HashMap, HashSet};

const INCH: f64 = 0.0254;
const GLUE: f64 = 0.01;

fn faces<'py>(definition: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    Ok(match definition.getattr_opt("faces")? {
        Some(faces) => faces.cast::<PyDict>()?.values(),
        None => PyList::empty(definition.py()),
    })
}
fn ring<'py>(d: &Bound<'py, PyAny>, f: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyList>>> {
    if let Some(loops) = f.getattr_opt("loops")?.filter(|v| !v.is_none()) {
        if loops.len()? != 0 {
            return skp_ring(d, &loops.get_item(0)?);
        }
    }
    Ok(None)
}

#[pyfunction]
pub fn skp_image_faces<'py>(
    child: &Bound<'py, PyAny>,
    placed: &Bound<'py, PyAny>,
    paints: &Bound<'py, PyDict>,
    inherited: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyList>> {
    let py = child.py();
    let world = DMat4::from_cols_array(&placed.call_method0("data")?.extract()?);
    let mut rings = Vec::new();
    let mut bounds = Bounds::empty();
    for face in faces(child)? {
        if let Some(raw) = ring(child, &face)? {
            let points = raw.extract::<Vec<[f64; 3]>>()?;
            for &p in &points {
                bounds.include(DVec3::from_array(p));
            }
            rings.push((points, face));
        }
    }
    let span = bounds.high - bounds.low;
    let width = if span.x == 0. { 1. } else { span.x };
    let height = if span.y == 0. { 1. } else { span.y };
    let out = PyList::empty(py);
    for (points, f) in rings {
        if points.len() < 3 {
            continue;
        }
        let outer = PyList::empty(py);
        let mut uv = Vec::with_capacity(points.len());
        for p in points {
            let v = DVec3::from_array(p);
            outer.append(Vec3::from(world.transform_point3(v * INCH)))?;
            uv.push(((v.x - bounds.low.x) / width, (v.y - bounds.low.y) / height));
        }
        let key = f.getattr_opt("material_id")?.filter(|v| !v.is_none());
        let mut attrs = paints.get_item(key.as_ref().unwrap_or(inherited))?;
        if let Some(ref a) = attrs {
            let row = a.cast::<PyDict>()?;
            if let Some(tex) = row.get_item("texture")? {
                let fit = crate::model::module(py, "ingetrazo_core")?
                    .call_method1("texture_fit_uv_affine", (&outer, uv))?;
                if !fit.is_none() {
                    let tex = tex.cast::<PyDict>()?.copy()?;
                    tex.set_item("uvw", fit)?;
                    let copy = row.copy()?;
                    copy.set_item("texture", tex)?;
                    attrs = Some(copy.into_any());
                }
            }
        }
        out.append((outer, PyList::empty(py), attrs))?;
    }
    Ok(out)
}

#[derive(Clone, Copy)]
struct Bounds {
    low: DVec3,
    high: DVec3,
}
impl Bounds {
    fn empty() -> Self {
        Self {
            low: DVec3::splat(f64::INFINITY),
            high: DVec3::splat(f64::NEG_INFINITY),
        }
    }
    fn include(&mut self, p: DVec3) {
        self.low = self.low.min(p);
        self.high = self.high.max(p);
    }
    fn contains(&self, c: DVec3, tol: f64) -> bool {
        (self.low - tol).cmple(c).all() && c.cmple(self.high + tol).all()
    }
}
fn geometry_bounds(def: &Bound<'_, PyAny>) -> Option<Bounds> {
    if !def.getattr("faces").ok()?.is_truthy().ok()? {
        return None;
    }
    let vertices = def.getattr("vertices").ok()?;
    let vertices = vertices.cast::<PyDict>().ok()?;
    if vertices.is_empty() {
        return None;
    }
    let mut b = Bounds::empty();
    for (_, v) in vertices {
        let point = DVec3::new(
            v.getattr("x").ok()?.extract().ok()?,
            v.getattr("y").ok()?.extract().ok()?,
            v.getattr("z").ok()?.extract().ok()?,
        ) * INCH;
        if !point.is_finite() {
            return None;
        }
        b.include(point);
    }
    Some(b)
}
#[pyfunction]
pub fn skp_image_bounds(d: &Bound<'_, PyAny>) -> Option<([f64; 3], [f64; 3])> {
    geometry_bounds(d).map(|b| (b.low.to_array(), b.high.to_array()))
}
fn normal(points: &[DVec3]) -> Option<DVec3> {
    let sum = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| a.cross(*b))
        .sum::<DVec3>();
    (sum.length() >= 1e-12).then(|| sum.normalize())
}
struct Contact {
    centre: DVec3,
    normal: DVec3,
}
impl Contact {
    fn supported(&self, points: &[DVec3]) -> bool {
        let Some(n) = normal(points) else {
            return false;
        };
        if (n.dot(self.normal).abs() - 1.).abs() > 1e-3
            || (self.centre - points[0]).dot(n).abs() > GLUE
        {
            return false;
        }
        let a = n
            .cross(if n.x.abs() < 0.9 { DVec3::X } else { DVec3::Y })
            .normalize();
        let b = n.cross(a);
        let uv = |p: DVec3| ((p - self.centre).dot(a), (p - self.centre).dot(b));
        let mut previous = uv(*points.last().unwrap());
        let mut inside = false;
        for &p in points {
            let (x, y) = uv(p);
            if (y > 0.) != (previous.1 > 0.) && x + (previous.0 - x) * (-y) / (previous.1 - y) > 0.
            {
                inside = !inside;
            }
            previous = (x, y);
        }
        inside
    }
}
struct Support {
    bounds: Option<Bounds>,
    faces: Option<Vec<(Bounds, Vec<DVec3>)>>,
}
struct Search<'a, 'py> {
    query: Contact,
    by_id: &'a Bound<'py, PyDict>,
    defs: HashMap<usize, Support>,
    path: Vec<usize>,
    broad: bool,
}
impl Search<'_, '_> {
    fn visit(&mut self, d: &Bound<'_, PyAny>, matrix: DMat4) -> PyResult<bool> {
        let key = d.as_ptr() as usize;
        if self.path.len() > 32 || self.path.contains(&key) {
            return Ok(false);
        }
        let inv = matrix.inverse();
        let centre = inv.transform_point3(self.query.centre);
        let scale = [matrix.x_axis, matrix.y_axis, matrix.z_axis]
            .iter()
            .map(|v| v.truncate().length())
            .fold(1., f64::min);
        let tol = GLUE / if scale == 0. { 1. } else { scale };
        let shape = self.defs.entry(key).or_insert_with(|| Support {
            bounds: if self.broad { geometry_bounds(d) } else { None },
            faces: None,
        });
        if shape
            .bounds
            .is_none_or(|b| !inv.is_finite() || b.contains(centre, tol))
        {
            if shape.faces.is_none() {
                let mut polys = Vec::new();
                for face in faces(d)? {
                    if let Some(raw) = ring(d, &face)? {
                        let mut bounds = Bounds::empty();
                        let p: Vec<_> = raw
                            .extract::<Vec<[f64; 3]>>()?
                            .into_iter()
                            .map(|p| {
                                let point = DVec3::from_array(p) * INCH;
                                bounds.include(point);
                                point
                            })
                            .collect();
                        if p.len() >= 3 {
                            polys.push((bounds, p));
                        }
                    }
                }
                shape.faces = Some(polys);
            }
            let polys = shape.faces.as_ref().unwrap();
            if !polys.is_empty() && !inv.is_finite() {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "singular image support transform",
                ));
            }
            if d.py().detach(|| {
                polys.iter().any(|(b, points)| {
                    b.contains(centre, tol)
                        && self.query.supported(
                            &points
                                .iter()
                                .map(|&p| matrix.transform_point3(p))
                                .collect::<Vec<_>>(),
                        )
                })
            }) {
                return Ok(true);
            }
        }
        self.path.push(key);
        if let Some(occurrences) = d.getattr_opt("instances")? {
            for ins in occurrences.try_iter()? {
                let ins = ins?;
                if let Some(child) = self.by_id.get_item(ins.getattr("ref_idx")?)? {
                    if ["is_image", "always_faces_camera"].iter().any(|&k| {
                        child
                            .getattr(k)
                            .and_then(|v| v.is_truthy())
                            .unwrap_or(false)
                    }) {
                        continue;
                    }
                    if self.visit(
                        &child,
                        matrix * placed_matrix(&ins.getattr("matrix")?.extract::<Vec<_>>()?)?,
                    )? {
                        return Ok(true);
                    }
                }
            }
        }
        self.path.pop();
        Ok(false)
    }
}

#[pyfunction]
#[pyo3(signature=(roots,by_id,quad,broad=true))]
pub fn skp_image_attached(
    roots: &Bound<'_, PyAny>,
    by_id: &Bound<'_, PyDict>,
    quad: &Bound<'_, PyAny>,
    broad: bool,
) -> PyResult<bool> {
    let points = quad
        .try_iter()?
        .map(|p| xyz(&p?))
        .collect::<PyResult<Vec<_>>>()?;
    let Some(n) = normal(&points) else {
        return Ok(false);
    };
    let mut search = Search {
        query: Contact {
            normal: n,
            centre: points.iter().sum::<DVec3>() / points.len() as f64,
        },
        by_id,
        defs: HashMap::new(),
        path: vec![],
        broad,
    };
    for root in roots.try_iter()? {
        if search.visit(&root?, DMat4::IDENTITY)? {
            return Ok(true);
        }
    }
    Ok(false)
}

// Measure the proportion of shared corners with conflicting UVs, using the
// same corner and UV precision as the applied positioned-texture mapping.
#[pyfunction]
pub fn skp_mark_drapes(def: &Bound<'_, PyAny>, paints: &Bound<'_, PyDict>) -> PyResult<()> {
    let py = def.py();
    let groups = PyDict::new(py);
    let round = py.import("builtins")?.getattr("round")?;
    for f in faces(def)? {
        let Some(id) = f.getattr_opt("material_id")?.filter(|v| !v.is_none()) else {
            continue;
        };
        let Some(tex) = paints.get_item(&id)? else {
            continue;
        };
        let Some(matrix) = f.getattr_opt("uv_transform")? else {
            continue;
        };
        if !matrix.is_truthy()? || !tex.contains("texture")? {
            continue;
        }
        let key = pyo3::types::PyTuple::new(
            py,
            matrix
                .try_iter()?
                .map(|n| round.call1((n?, 1)))
                .collect::<PyResult<Vec<_>>>()?,
        )?;
        let tag = (id, key).into_pyobject(py)?;
        if let Some(list) = groups.get_item(&tag)? {
            list.cast::<PyList>()?.append(f)?
        } else {
            groups.set_item(tag, PyList::new(py, [f])?)?;
        }
    }
    for (key, group) in groups {
        if group.len()? < 8 {
            continue;
        }
        let texture = paints
            .as_any()
            .get_item(key.get_item(0)?)?
            .get_item("texture")?
            .cast_into::<PyDict>()?;
        let mut seams = [HashMap::new(), HashMap::new()];
        let mut complete = true;
        for f in group.cast::<PyList>()?.iter().take(80) {
            let Some(raw) = ring(def, &f)? else {
                complete = false;
                break;
            };
            let points = raw.extract::<Vec<[f64; 3]>>()?;
            for (i, projected) in [false, true].into_iter().enumerate() {
                let Some(uv) = skp_positioned_uvs(&f, &raw, &texture, None, projected)? else {
                    complete = false;
                    break;
                };
                for (point, (u, v)) in points.iter().zip(uv) {
                    // The keys encode the rounded coordinates, with equal signed zeros.
                    let q = |n: f64, digits: i32| -> PyResult<u64> {
                        let x = round.call1((n, digits))?.extract::<f64>()?;
                        Ok(if x == 0. { 0 } else { x.to_bits() })
                    };
                    let key = [q(point[0], 1)?, q(point[1], 1)?, q(point[2], 1)?];
                    seams[i]
                        .entry(key)
                        .or_insert_with(HashSet::new)
                        .insert((q(u, 2)?, q(v, 2)?));
                }
            }
            if !complete {
                break;
            }
        }
        let discontinuity = |i: usize| {
            seams[i].values().filter(|uv| uv.len() > 1).count() as f64 / seams[i].len() as f64
        };
        if complete && discontinuity(0) > 0.3 && discontinuity(1) < 0.1 {
            for f in group.try_iter()? {
                f?.setattr("_projected", true)?;
            }
        }
    }
    Ok(())
}
