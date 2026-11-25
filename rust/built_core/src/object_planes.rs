// SPDX-License-Identifier: GPL-3.0-or-later
//! Plane selection for materialised vertices, edges and public face handles.
use crate::math::{xyz, Vec3};
use crate::store::{
    drawing,
    geometry::{Hypot, V3},
};
use glam::DVec3;
use pyo3::prelude::*;
type Obj<'py> = Bound<'py, PyAny>;

#[pyfunction]
#[pyo3(signature=(points,tol=1e-4,vertices=None))]
pub(crate) fn coplanar_plane<'py>(
    points: &Obj<'py>,
    tol: f64,
    vertices: Option<&Obj<'py>>,
) -> PyResult<Option<(Obj<'py>, Vec3)>> {
    let mut coords = Vec::new();
    let mut origin = None;
    let mut add = |p: Obj<'py>| -> PyResult<()> {
        let v = xyz(&p)?;
        coords.push(V3(v.x, v.y, v.z));
        if origin.is_none() {
            origin = Some(p);
        }
        Ok(())
    };
    if let Some(verts) = vertices {
        for v in verts.try_iter()? {
            add(v?.getattr("position")?)?;
        }
    }
    for p in points.try_iter()? {
        add(p?)?;
    }
    let n = if coords.len() < 3 {
        [0., 0., 1.]
    } else {
        // The materialised API rejects distances strictly above tolerance.
        let plane = drawing::drawing_plane_where(coords.iter().copied(), Hypot::LibStdCxx, |d| {
            d.partial_cmp(&tol) != Some(std::cmp::Ordering::Greater)
        });
        let Some([_, n]) = plane else { return Ok(None) };
        n
    };
    let o = match origin {
        Some(p) => p,
        None => Vec3::from(DVec3::ZERO)
            .into_pyobject(points.py())?
            .into_any(),
    };
    Ok(Some((o, Vec3::from(DVec3::from_array(n)))))
}

#[pyfunction]
#[pyo3(signature=(edges,plane=None,curve_only=true))]
pub(crate) fn has_plane_edge(
    edges: &Obj<'_>,
    plane: Option<(Obj<'_>, Obj<'_>)>,
    curve_only: bool,
) -> PyResult<bool> {
    let plane = plane
        .map(|(o, n)| -> PyResult<_> { Ok((xyz(&o)?, xyz(&n)?)) })
        .transpose()?;
    for e in edges.try_iter()? {
        let e = e?;
        if curve_only && e.getattr_opt("curve")?.is_none_or(|c| c.is_none()) {
            continue;
        }
        if let Some((o, n)) = plane {
            if !((xyz(&e.getattr("a")?)? - o).dot(n).abs() < 1e-4
                && (xyz(&e.getattr("b")?)? - o).dot(n).abs() < 1e-4)
            {
                continue;
            }
        }
        return Ok(true);
    }
    Ok(false)
}

#[pyfunction]
pub(crate) fn face_rebuild_plane<'py>(
    faces: &Obj<'py>,
    points: &Obj<'py>,
    cap: usize,
) -> PyResult<Option<(Obj<'py>, Obj<'py>)>> {
    let pts = points
        .try_iter()?
        .map(|p| xyz(&p?))
        .collect::<PyResult<Vec<_>>>()?;
    // Keep the face's own origin and normal. Preview faces may override normal().
    for f in faces.try_iter()? {
        let f = f?;
        let o = f.getattr("vertices")?.get_item(0)?;
        let n = f.call_method0("normal")?;
        let (p, v) = (xyz(&o)?, xyz(&n)?);
        if v.length_squared() < 0.5 || !pts.iter().all(|q| (*q - p).dot(v).abs() < 1e-4) {
            continue;
        }
        let mut count = 0;
        for other in faces.try_iter()? {
            let other = other?;
            if (xyz(&other.getattr("vertices")?.get_item(0)?)? - p)
                .dot(v)
                .abs()
                < 1e-4
                && xyz(&other.call_method0("normal")?)?.dot(v).abs() > 0.999
            {
                count += 1;
                if count > cap {
                    return Ok(None);
                }
            }
        }
        return Ok(Some((o, n)));
    }
    Ok(None)
}
