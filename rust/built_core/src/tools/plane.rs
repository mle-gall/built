//! Drawing planes: the in-plane axes, a loop's winding, and the plane lock
//! the planar tools share (`tools.base.PlaneLock` before R2.5 slice 3).
//!
//! `PlaneLock` holds the arrow-key lock, the Down-arrow / Shift reference
//! and the plane the cursor last hit, and decides the plane a shape is
//! laid out on. The Python mixin keeps only the viewport glue (status
//! messages, redraws, what is under the cursor) and reads this state.
//! Vectors are `glam::DVec3`, as the Python `Vec3`, so results match it.

use glam::DVec3;
use pyo3::prelude::*;

use crate::math::{xyz, Vec3};

/// The drawing axes: the world's, or the open group's (`built_core.axes`).
#[derive(Clone, Copy, Debug)]
pub struct Axes { pub o: DVec3, pub x: DVec3, pub y: DVec3, pub z: DVec3 }

impl Axes {
    pub fn world() -> Self { Axes { o: DVec3::ZERO, x: DVec3::X, y: DVec3::Y, z: DVec3::Z } }
    pub fn axis(&self, k: usize) -> DVec3 { [self.x, self.y, self.z][k] }
}

/// Two orthonormal in-plane axes: `u` the drawing red projected on the
/// plane (the green when the plane faces red), `v = n × u`.
pub fn plane_axes(normal: DVec3, axes: &Axes) -> (DVec3, DVec3) {
    let n = normal.normalize_or_zero();
    let mut u = axes.x - n * axes.x.dot(n);
    if u.length() < 0.1 {
        u = axes.y - n * axes.y.dot(n);
    }
    let u = u.normalize_or_zero();
    (u, n.cross(u).normalize_or_zero())
}

/// Newell normal of a closed loop (unnormalised).
pub fn loop_normal(corners: &[DVec3]) -> DVec3 {
    let mut n = DVec3::ZERO;
    for i in 0..corners.len() {
        n += corners[i].cross(corners[(i + 1) % corners.len()]);
    }
    n
}

/// The loop wound to look the way its plane does; a loop square to the
/// plane is left as it came.
pub fn face_the_plane(corners: &[DVec3], normal: Option<DVec3>) -> Vec<DVec3> {
    let Some(normal) = normal else { return corners.to_vec() };
    if corners.len() < 3 {
        return corners.to_vec();
    }
    if loop_normal(corners).dot(normal) < -1e-12 {
        let mut out = vec![corners[0]];
        out.extend(corners[1..].iter().rev());
        return out;
    }
    corners.to_vec()
}

pub type Plane = (DVec3, DVec3);

/// How far off the drawing plane a point must be to count as elsewhere.
pub const OFF_PLANE_TOL: f64 = 1e-3;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaneLock {
    /// The arrow-key axis the plane is normal to (0 = x, 1 = y, 2 = z).
    pub lock: Option<usize>,
    /// Down arrow / held Shift: `(normal, "edge" | "face" | "plane")`.
    pub reference: Option<(DVec3, String)>,
    /// The plane the viewport used for the last cursor hit.
    pub hover: Option<Plane>,
}

impl PlaneLock {
    /// The locked plane through `point`, or None without a lock.
    pub fn locked(&self, point: DVec3, axes: &Axes) -> Option<Plane> {
        if let Some(k) = self.lock {
            return Some((point, axes.axis(k)));
        }
        self.reference.as_ref().map(|(n, _)| (point, *n))
    }

    /// The captured plane, else the last hit's, else the context's ground.
    pub fn own_plane(&self, work: Option<Plane>, axes: &Axes) -> Plane {
        work.or(self.hover).unwrap_or((axes.o, axes.z))
    }

    /// The plane a shape's points ask for when a snapped one lies off
    /// `plane` (a window opening filled jamb to jamb, #37): through three
    /// points not in a line, else the one axis plane holding both of two;
    /// None to keep `plane`. A lock never yields, and a cursor standing on
    /// a face (`names_point` false) keeps that face's plane (#545).
    pub fn snapped(&self, plane: Plane, points: &[DVec3], names_point: Option<bool>, axes: &Axes) -> Option<Plane> {
        if self.lock.is_some() || self.reference.is_some() || names_point == Some(false) || points.len() < 2 {
            return None;
        }
        let (origin, normal) = plane;
        let n = normal.normalize_or_zero();
        if points.iter().all(|p| (*p - origin).dot(n).abs() < OFF_PLANE_TOL) {
            return None;
        }
        let a = points[0];
        if points.len() >= 3 {
            let through = (points[1] - a).cross(points[2] - a);
            if through.length() > OFF_PLANE_TOL * OFF_PLANE_TOL {
                return Some((a, through.normalize_or_zero()));
            }
        }
        let span = *points.last().unwrap() - a;
        let flat: Vec<DVec3> = (0..3).map(|k| axes.axis(k)).filter(|ax| span.dot(*ax).abs() < OFF_PLANE_TOL).collect();
        (flat.len() == 1).then(|| (a, flat[0]))
    }

    /// The plane the shape is laid out on.
    pub fn drawing(&self, work: Option<Plane>, points: &[DVec3], names_point: Option<bool>, axes: &Axes) -> Plane {
        let own = self.own_plane(work, axes);
        self.snapped(own, points, names_point, axes).unwrap_or(own)
    }

    /// An arrow key: lock that axis's plane, or free it when it is the one.
    pub fn toggle(&mut self, k: usize) {
        self.reference = None;
        self.lock = if self.lock == Some(k) { None } else { Some(k) };
    }

    pub fn clear(&mut self) {
        self.lock = None;
        self.reference = None;
    }
}

// ---- Python -------------------------------------------------------------

pub(crate) fn axes_of(v: Option<&Bound<'_, PyAny>>) -> PyResult<Axes> {
    let Some(v) = v.filter(|v| !v.is_none()) else { return Ok(Axes::world()) };
    let (o, x, y, z): (Bound<PyAny>, Bound<PyAny>, Bound<PyAny>, Bound<PyAny>) = v.extract()?;
    Ok(Axes { o: xyz(&o)?, x: xyz(&x)?, y: xyz(&y)?, z: xyz(&z)? })
}

pub(crate) fn plane_of(v: &Bound<'_, PyAny>) -> PyResult<Option<Plane>> {
    if v.is_none() {
        return Ok(None);
    }
    let (p, n): (Bound<PyAny>, Bound<PyAny>) = v.extract()?;
    Ok(Some((xyz(&p)?, xyz(&n)?)))
}

pub(crate) fn vec(v: DVec3) -> Vec3 { v.into() }

pub(crate) const AXIS: [&str; 3] = ["x", "y", "z"];

/// `built_core.axes.plane_axes` / `built_core.triangulate.plane_axes`: `(u, v)` of a
/// plane normal against the drawing axes `(o, x, y, z)` (the world's when
/// omitted).
#[pyfunction]
#[pyo3(signature = (normal, axes=None))]
pub fn drawing_plane_axes(normal: &Bound<'_, PyAny>, axes: Option<&Bound<'_, PyAny>>) -> PyResult<(Vec3, Vec3)> {
    let (u, v) = plane_axes(xyz(normal)?, &axes_of(axes)?);
    Ok((vec(u), vec(v)))
}

/// Newell normal of a closed loop of points.
#[pyfunction]
#[pyo3(name = "loop_normal")]
pub fn py_loop_normal(corners: Vec<Bound<'_, PyAny>>) -> PyResult<Vec3> {
    let pts = corners.iter().map(xyz).collect::<PyResult<Vec<_>>>()?;
    Ok(vec(loop_normal(&pts)))
}

/// The loop wound to look the way `normal` does.
#[pyfunction]
#[pyo3(name = "face_the_plane")]
pub fn py_face_the_plane(corners: Vec<Bound<'_, PyAny>>, normal: Option<Bound<'_, PyAny>>) -> PyResult<Vec<Vec3>> {
    let pts = corners.iter().map(xyz).collect::<PyResult<Vec<_>>>()?;
    let n = normal.as_ref().filter(|n| !n.is_none()).map(xyz).transpose()?;
    Ok(face_the_plane(&pts, n).into_iter().map(vec).collect())
}

/// The plane lock's state, held by each planar tool (`tools.base.PlaneLock`).
#[pyclass(module = "built_core", name = "PlaneLock")]
#[derive(Default)]
pub struct PyPlaneLock {
    pub inner: PlaneLock,
}

#[pymethods]
impl PyPlaneLock {
    #[new]
    fn new() -> Self { Self::default() }

    /// `"x"`, `"y"`, `"z"` or None.
    #[getter]
    fn get_plane_lock(&self) -> Option<&'static str> { self.inner.lock.map(|k| AXIS[k]) }
    #[setter]
    fn set_plane_lock(&mut self, axis: Option<&str>) -> PyResult<()> {
        self.inner.lock = match axis {
            None => None,
            Some(a) => Some(AXIS.iter().position(|x| *x == a)
                .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("a plane lock is x, y or z"))?),
        };
        Ok(())
    }

    /// `(normal, kind)` or None.
    #[getter]
    fn get_plane_ref(&self) -> Option<(Vec3, String)> {
        self.inner.reference.as_ref().map(|(n, k)| (vec(*n), k.clone()))
    }
    #[setter]
    fn set_plane_ref(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner.reference = if value.is_none() {
            None
        } else {
            let (n, k): (Bound<PyAny>, String) = value.extract()?;
            Some((xyz(&n)?, k))
        };
        Ok(())
    }

    #[getter]
    fn get_hover_plane(&self) -> Option<(Vec3, Vec3)> { self.inner.hover.map(|(p, n)| (vec(p), vec(n))) }
    #[setter]
    fn set_hover_plane(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner.hover = plane_of(value)?;
        Ok(())
    }

    /// The arrow key for axis `"x"`/`"y"`/`"z"`: lock it, or free it.
    fn toggle(&mut self, axis: &str) -> PyResult<()> {
        let k = AXIS.iter().position(|x| *x == axis)
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("a plane lock is x, y or z"))?;
        self.inner.toggle(k);
        Ok(())
    }

    fn clear(&mut self) { self.inner.clear() }

    #[pyo3(signature = (point, axes=None))]
    fn locked(&self, point: &Bound<'_, PyAny>, axes: Option<&Bound<'_, PyAny>>) -> PyResult<Option<(Vec3, Vec3)>> {
        Ok(self.inner.locked(xyz(point)?, &axes_of(axes)?).map(|(p, n)| (vec(p), vec(n))))
    }

    #[pyo3(signature = (work_plane, axes=None))]
    fn own_plane(&self, work_plane: &Bound<'_, PyAny>, axes: Option<&Bound<'_, PyAny>>) -> PyResult<(Vec3, Vec3)> {
        let (p, n) = self.inner.own_plane(plane_of(work_plane)?, &axes_of(axes)?);
        Ok((vec(p), vec(n)))
    }

    /// `names_point`: whether the cursor's snap names a point of the model
    /// (None when no kind is known).
    #[pyo3(signature = (plane, points, names_point, axes=None))]
    fn snapped(&self, plane: &Bound<'_, PyAny>, points: Vec<Bound<'_, PyAny>>, names_point: Option<bool>,
               axes: Option<&Bound<'_, PyAny>>) -> PyResult<Option<(Vec3, Vec3)>> {
        let Some(plane) = plane_of(plane)? else { return Ok(None) };
        let pts = points.iter().filter(|p| !p.is_none()).map(xyz).collect::<PyResult<Vec<_>>>()?;
        Ok(self.inner.snapped(plane, &pts, names_point, &axes_of(axes)?).map(|(p, n)| (vec(p), vec(n))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axes_of_a_wall_run_horizontally() {
        let (u, v) = plane_axes(DVec3::Y, &Axes::world());
        assert_eq!(u, DVec3::X);
        assert!((v - DVec3::new(0.0, 0.0, -1.0)).length() < 1e-12);
        let (u, _) = plane_axes(DVec3::X, &Axes::world());
        assert!((u - DVec3::Y).length() < 1e-12);
    }

    #[test]
    fn winding_follows_the_plane() {
        let sq = [DVec3::ZERO, DVec3::X, DVec3::new(1.0, 1.0, 0.0), DVec3::Y];
        assert_eq!(face_the_plane(&sq, Some(DVec3::Z)), sq.to_vec());
        let back = face_the_plane(&sq, Some(-DVec3::Z));
        assert_eq!(back, vec![sq[0], sq[3], sq[2], sq[1]]);
    }

    #[test]
    fn points_off_the_plane_pick_the_axis_plane_holding_them() {
        let lock = PlaneLock::default();
        let ground = (DVec3::ZERO, DVec3::Z);
        let pts = [DVec3::new(0.0, 0.0, 1.0), DVec3::new(2.0, 0.0, 3.0)];
        assert_eq!(lock.snapped(ground, &pts, Some(true), &Axes::world()), Some((pts[0], DVec3::Y)));
        assert_eq!(lock.snapped(ground, &pts, Some(false), &Axes::world()), None);
        let mut locked = PlaneLock::default();
        locked.toggle(2);
        assert_eq!(locked.snapped(ground, &pts, Some(true), &Axes::world()), None);
        locked.toggle(2);
        assert_eq!(locked.lock, None);
    }
}
