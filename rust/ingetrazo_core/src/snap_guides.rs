// SPDX-License-Identifier: GPL-3.0-or-later
//! Screen selection of axis feet, plane guides and crossings for arbitrary projectors.
use crate::{
    math::{xyz, Vec3},
    snap::{self, P},
    snap_planes,
};
use glam::DVec3;
use pyo3::prelude::*;
type Obj<'py> = Bound<'py, PyAny>;
fn vec3(p: P) -> Vec3 {
    DVec3::from_array(p).into()
}
struct Cursor<'a, 'py> {
    position: [f64; 2],
    radius: f64,
    project: &'a Obj<'py>,
    hidden: Option<&'a Obj<'py>>,
}
impl Cursor<'_, '_> {
    fn distance(&self, p: &Obj<'_>) -> PyResult<Option<f64>> {
        let xy = self.project.call1((p,))?;
        if xy.is_none() {
            return Ok(None);
        }
        let dx = xy.get_item(0)?.extract::<f64>()? - self.position[0];
        let dy = xy.get_item(1)?.extract::<f64>()? - self.position[1];
        let d = dx.hypot(dy);
        Ok((d <= self.radius).then_some(d))
    }
    fn clear(&self, p: &Obj<'_>) -> PyResult<bool> {
        Ok(match self.hidden {
            Some(h) => !h.call1((p,))?.is_truthy()?,
            None => true,
        })
    }
}

#[pyfunction]
#[pyo3(signature=(reference,candidate,axes,position,radius,project,hidden,normal,line))]
pub(crate) fn axis_guide<'py>(
    reference: &Obj<'py>,
    candidate: &Obj<'py>,
    axes: Vec<Obj<'py>>,
    position: [f64; 2],
    radius: f64,
    project: &Obj<'py>,
    hidden: Option<&Obj<'py>>,
    normal: Option<&Obj<'py>>,
    line: Option<&Obj<'py>>,
) -> PyResult<Option<(usize, Obj<'py>)>> {
    let cursor = Cursor {
        position,
        radius,
        project,
        hidden,
    };
    let mut best = None;
    let mut offer = |k, p: Obj<'py>| -> PyResult<()> {
        if let Some(d) = cursor.distance(&p)? {
            if cursor.clear(&p)? && best.as_ref().is_none_or(|(s, _, _)| d < *s) {
                best = Some((d, k, p));
            }
        }
        Ok(())
    };
    if let Some(line) = line {
        for (k, a) in axes.iter().enumerate() {
            offer(k, line.call1((reference, a))?)?;
        }
    } else {
        let directions = axes
            .iter()
            .map(|a| Ok(xyz(a)?.to_array()))
            .collect::<PyResult<Vec<_>>>()?;
        let directions = directions
            .try_into()
            .map_err(|_| pyo3::exceptions::PyValueError::new_err("expected three axes"))?;
        let n = normal.map(|n| xyz(n).map(|v| v.to_array())).transpose()?;
        for (k, p) in snap::reference_feet(
            xyz(reference)?.to_array(),
            xyz(candidate)?.to_array(),
            directions,
            n,
        ) {
            offer(k, vec3(p).into_pyobject(project.py())?.into_any())?;
        }
    }
    Ok(best.map(|(_, k, p)| (k, p)))
}

fn colour_axis(v: DVec3, axes: [P; 3]) -> Option<usize> {
    if v.length() < 1e-6 {
        return None;
    }
    let u = v.normalize_or_zero();
    let mut k = 0;
    for i in 1..3 {
        if u.dot(axes[i].into()).abs() > u.dot(axes[k].into()).abs() {
            k = i;
        }
    }
    Some(k)
}
type Guide<'py> = (usize, Obj<'py>, Vec3, Option<usize>, Option<usize>, bool);
#[pyfunction]
#[pyo3(signature=(reference,candidate,normal,axes,position,radius,project,hidden))]
pub(crate) fn plane_guide<'py>(
    reference: &Obj<'py>,
    candidate: &Obj<'py>,
    normal: Option<&Obj<'py>>,
    axes: [P; 3],
    position: [f64; 2],
    radius: f64,
    project: &Obj<'py>,
    hidden: Option<&Obj<'py>>,
) -> PyResult<Option<Guide<'py>>> {
    let Some(n) = normal else { return Ok(None) };
    let r = xyz(reference)?;
    let cursor = Cursor {
        position,
        radius,
        project,
        hidden,
    };
    let mut best = None;
    for (axis, foot, dir) in snap_planes::plane_alignments(
        r.to_array(),
        xyz(candidate)?.to_array(),
        xyz(n)?.to_array(),
        axes,
    ) {
        let p = vec3(foot).into_pyobject(project.py())?.into_any();
        if let Some(d) = cursor.distance(&p)? {
            if cursor.clear(&p)?
                && best
                    .as_ref()
                    .is_none_or(|(s, ..): &(f64, usize, Obj<'py>, P)| d < *s)
            {
                best = Some((d, axis, p, dir));
            }
        }
    }
    let Some((_, k, p, dir)) = best else {
        return Ok(None);
    };
    let f = xyz(&p)?;
    let u = DVec3::from_array(dir);
    let elbow = f + u * (r - f).dot(u);
    Ok(Some((
        k,
        p,
        elbow.into(),
        colour_axis(elbow - r, axes),
        colour_axis(f - elbow, axes),
        (elbow - r).length() > 1e-4,
    )))
}

#[pyfunction]
#[pyo3(signature=(refs,axes,position,radius,project,hidden))]
pub(crate) fn cross_guide<'py>(
    refs: &Obj<'py>,
    axes: [P; 3],
    position: [f64; 2],
    radius: f64,
    project: &Obj<'py>,
    hidden: Option<&Obj<'py>>,
) -> PyResult<Option<(Obj<'py>, Obj<'py>, usize, Obj<'py>, usize)>> {
    let mut ps = Vec::new();
    for p in refs.try_iter()? {
        let p = p?;
        if !p.is_none() {
            ps.push(p);
        }
    }
    let coords = ps
        .iter()
        .map(|p| Ok(xyz(p)?.to_array()))
        .collect::<PyResult<_>>()?;
    let cursor = Cursor {
        position,
        radius,
        project,
        hidden,
    };
    let mut best = None;
    for (p, i, k, j, l) in snap::reference_crossings(coords, axes) {
        let p = vec3(p).into_pyobject(project.py())?.into_any();
        if let Some(d) = cursor.distance(&p)? {
            if best.as_ref().is_none_or(|(s, _)| d < *s) && cursor.clear(&p)? {
                best = Some((d, (p, ps[i].clone(), k, ps[j].clone(), l)));
            }
        }
    }
    Ok(best.map(|(_, v)| v))
}

fn circle(c: &Obj<'_>, r: f64, n: &Obj<'_>) -> PyResult<crate::snap_curve::Circle> {
    Ok((xyz(c)?.to_array(), r, xyz(n)?.to_array()))
}
#[pyfunction]
pub(crate) fn circle_crossings(
    c1: &Obj<'_>,
    r1: f64,
    n1: &Obj<'_>,
    c2: &Obj<'_>,
    r2: f64,
    n2: &Obj<'_>,
) -> PyResult<Vec<Vec3>> {
    Ok(
        crate::snap_curve::circle_circle(circle(c1, r1, n1)?, circle(c2, r2, n2)?)
            .into_iter()
            .map(vec3)
            .collect(),
    )
}
#[pyfunction]
pub(crate) fn segment_crossings(
    a: &Obj<'_>,
    b: &Obj<'_>,
    c: &Obj<'_>,
    r: f64,
    n: &Obj<'_>,
) -> PyResult<Vec<Vec3>> {
    Ok(crate::snap_curve::segment_circle(
        [xyz(a)?.to_array(), xyz(b)?.to_array()],
        circle(c, r, n)?,
    )
    .into_iter()
    .map(vec3)
    .collect())
}
fn basis<'py>(n: &Obj<'py>) -> PyResult<Obj<'py>> {
    n.py().import("core.axes")?.call_method1("plane_axes", (n,))
}
#[pyfunction]
pub(crate) fn quadrants(c: &Obj<'_>, r: f64, n: &Obj<'_>) -> PyResult<Vec<Vec3>> {
    if r <= 0. {
        return Ok(Vec::new());
    }
    let c = xyz(c)?;
    let uv = basis(n)?;
    let u = xyz(&uv.get_item(0)?)? * r;
    let v = xyz(&uv.get_item(1)?)? * r;
    Ok(vec![
        (c + u).into(),
        (c - u).into(),
        (c + v).into(),
        (c - v).into(),
    ])
}
#[pyfunction]
pub(crate) fn closest_rim<'py>(
    circ: &Obj<'py>,
    candidate_world: &Obj<'py>,
    cx: f64,
    cy: f64,
    world_to_pixel: &Obj<'py>,
    et: f64,
) -> PyResult<(Option<f64>, Option<Obj<'py>>)> {
    let n = circ.getattr("normal")?;
    let circle = circle(
        &circ.getattr("center")?,
        circ.getattr("radius")?.extract()?,
        &n,
    )?;
    if circle.1 <= 0. {
        return Ok((None, None));
    }
    let p = xyz(candidate_world)?.to_array();
    let (c, _, normal) = circle;
    let off = snap::sub(p, c);
    let radial = snap::sub(snap::along(p, normal, -snap::dot(off, normal)), c);
    let fallback = if snap::norm(radial) < 1e-9 {
        xyz(&basis(&n)?.get_item(0)?)?.to_array()
    } else {
        [0.; 3]
    };
    let hit = vec3(crate::snap_curve::rim(circle, p, fallback))
        .into_pyobject(n.py())?
        .into_any();
    let cursor = Cursor {
        position: [cx, cy],
        radius: et,
        project: world_to_pixel,
        hidden: None,
    };
    Ok(match cursor.distance(&hit)? {
        Some(d) => (Some(d), Some(hit)),
        None => (None, None),
    })
}
