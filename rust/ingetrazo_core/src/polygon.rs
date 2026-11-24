// SPDX-License-Identifier: GPL-3.0-or-later
//! Binary64 polygons used by the scripting API and object-mesh previews.
use glam::DVec3;
use pyo3::{exceptions::PyValueError, prelude::*};

use crate::math::{xyz, Vec3};
use crate::tools::plane::{plane_axes, Axes};

fn newell(points: &[DVec3]) -> DVec3 {
    let mut normal = DVec3::ZERO;
    for (a, b) in points.iter().zip(points.iter().cycle().skip(1)) {
        normal.x += (a.y - b.y) * (a.z + b.z);
        normal.y += (a.z - b.z) * (a.x + b.x);
        normal.z += (a.x - b.x) * (a.y + b.y);
    }
    if normal.length() < 1e-9 {
        DVec3::Z
    } else {
        normal.normalize_or_zero()
    }
}

#[pyfunction]
pub fn polygon_normal(vertices: Vec<Bound<'_, PyAny>>) -> PyResult<Vec3> {
    let points = vertices.iter().map(xyz).collect::<PyResult<Vec<_>>>()?;
    Ok(newell(&points).into())
}

#[pyfunction]
pub fn polygon_is_convex(poly: Vec<[f64; 2]>) -> bool {
    crate::facegeom::is_convex(&poly.iter().map(|&[x, y]| (x, y)).collect::<Vec<_>>())
}

#[pyfunction]
#[pyo3(signature=(points, hole_indices=None))]
pub fn polygon_earcut(
    points: Vec<[f64; 2]>,
    hole_indices: Option<Vec<usize>>,
) -> PyResult<Vec<(usize, usize, usize)>> {
    let holes = hole_indices.unwrap_or_default();
    if holes.last().is_some_and(|&h| h >= points.len()) || holes.windows(2).any(|w| w[0] >= w[1]) {
        return Err(PyValueError::new_err(
            "hole indices must start distinct rings inside points",
        ));
    }
    Ok(crate::earcut::earcut(&points, &holes)
        .into_iter()
        .map(|[a, b, c]| (a, b, c))
        .collect())
}

type Corner<'py> = Bound<'py, PyAny>;
type Triangle<'py> = (Corner<'py>, Corner<'py>, Corner<'py>);

#[pyfunction]
#[pyo3(signature=(outer, holes=None, normal=None))]
pub fn polygon_triangles<'py>(
    mut outer: Vec<Corner<'py>>,
    holes: Option<Vec<Vec<Corner<'py>>>>,
    normal: Option<Corner<'py>>,
) -> PyResult<Vec<Triangle<'py>>> {
    if outer.len() < 3 {
        return Ok(Vec::new());
    }
    let mut points = outer.iter().map(xyz).collect::<PyResult<Vec<_>>>()?;
    let normal = match normal {
        Some(n) => xyz(&n)?,
        None => newell(&points),
    };
    let (u, v) = plane_axes(normal, &Axes::world());
    let mut starts = Vec::new();
    for ring in holes
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.len() >= 3)
    {
        starts.push(outer.len());
        for p in ring {
            points.push(xyz(&p)?);
            outer.push(p);
        }
    }
    let origin = points[0];
    let projected: Vec<_> = points
        .into_iter()
        .map(|p| {
            let d = p - origin;
            (d.dot(u), d.dot(v))
        })
        .collect();
    let indices = if starts.is_empty() && crate::facegeom::is_convex(&projected) {
        (1..outer.len() - 1).map(|i| [0, i, i + 1]).collect()
    } else {
        crate::earcut::earcut(
            &projected
                .into_iter()
                .map(|(x, y)| [x, y])
                .collect::<Vec<_>>(),
            &starts,
        )
    };
    Ok(indices
        .into_iter()
        .map(|[a, b, c]| (outer[a].clone(), outer[b].clone(), outer[c].clone()))
        .collect())
}
