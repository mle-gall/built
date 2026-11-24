//! View-space queries shared by transform tools. Native viewports supply one
//! camera matrix per query. Scripted viewports can supply a projection callback.
use super::{flip, scale_grip::BoxGeometry};
use crate::{
    math::{xyz, Vec3},
    pick::Projector,
};
use glam::{DMat3, DMat4, DVec3};
use numpy::PyReadonlyArray2;
use pyo3::{exceptions::PyValueError, prelude::*, types::PyDict};

type P3 = [f64; 3];
type Pixel = [f64; 2];

pub(super) enum Projection<'py> {
    Camera(Projector),
    Custom(Bound<'py, PyAny>),
}
impl<'py> Projection<'py> {
    pub(super) fn from_view(view: &Bound<'py, PyAny>) -> PyResult<Self> {
        if let Some(camera) = view.getattr_opt("_camera_px")? {
            match camera.call0() {
                Ok(frame) => {
                    let (matrix, w, h): ([f64; 16], f64, f64) = frame.extract()?;
                    return Ok(Self::Camera(Projector::new(&matrix, w, h)));
                }
                Err(error)
                    if error.is_instance_of::<pyo3::exceptions::PyAttributeError>(view.py()) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(Self::Custom(view.getattr("_world_to_pixel")?))
    }
    pub(super) fn pixel(&self, point: DVec3) -> PyResult<Option<Pixel>> {
        match self {
            Self::Camera(projector) => {
                let (x, y, visible) = projector.px(point.to_array());
                Ok(visible.then_some([x, y]))
            }
            Self::Custom(f) => f.call1((Vec3::from(point),))?.extract(),
        }
    }
}

/// Accumulate exact extrema in the grip frame across loose geometry and
/// nested occurrences. A mesh's positions can stay in their native buffer.
#[pyclass(module = "ingetrazo_core")]
pub struct ToolBounds {
    origin: DVec3,
    axes: DMat3,
    lo: DVec3,
    hi: DVec3,
    geometry: std::sync::OnceLock<BoxGeometry>,
}
impl ToolBounds {
    fn geometry(&self) -> &BoxGeometry {
        self.geometry
            .get_or_init(|| BoxGeometry::new((self.hi - self.lo).to_array()))
    }
    fn add(&mut self, p: DVec3) {
        let local = self.axes.transpose() * (p - self.origin);
        self.lo = self.lo.min(local);
        self.hi = self.hi.max(local);
    }
    fn point(&self, t: P3) -> DVec3 {
        self.origin + self.axes * (self.lo + (self.hi - self.lo) * DVec3::from(t))
    }
}
#[pymethods]
impl ToolBounds {
    #[new]
    #[pyo3(signature=(frame=None))]
    fn new(frame: Option<[P3; 4]>) -> Self {
        let (origin, axes) = frame
            .map(|[o, x, y, z]| (o.into(), DMat3::from_cols(x.into(), y.into(), z.into())))
            .unwrap_or((DVec3::ZERO, DMat3::IDENTITY));
        Self {
            origin,
            axes,
            lo: DVec3::INFINITY,
            hi: DVec3::NEG_INFINITY,
            geometry: Default::default(),
        }
    }
    fn add_points(&mut self, points: &Bound<'_, PyAny>) -> PyResult<()> {
        self.geometry.take();
        for p in points.try_iter()? {
            self.add(xyz(&p?)?);
        }
        Ok(())
    }
    #[pyo3(signature=(store, matrix=None))]
    fn add_store(&mut self, store: &crate::store::py::PyMeshStore, matrix: Option<[f64; 16]>) {
        self.geometry.take();
        let m = matrix.as_ref().map(DMat4::from_cols_array);
        let st = &store.inner;
        for slot in st.live_slots(crate::store::keys::Kind::Vertex) {
            let p = DVec3::from(*st.state.vpos.get(slot as usize));
            self.add(m.map_or(p, |m| m.transform_point3(p)));
        }
    }
    #[pyo3(signature=(positions, matrix=None))]
    fn add_mesh(
        &mut self,
        positions: PyReadonlyArray2<'_, f64>,
        matrix: Option<[f64; 16]>,
    ) -> PyResult<()> {
        self.geometry.take();
        let rows = positions.as_array();
        if rows.ncols() != 3 {
            return Err(PyValueError::new_err("expected an N by 3 vertex array"));
        }
        let transform = matrix.as_ref().map(DMat4::from_cols_array);
        for row in rows.rows() {
            let p = DVec3::new(row[0], row[1], row[2]);
            self.add(transform.map_or(p, |m| m.transform_point3(p)));
        }
        Ok(())
    }
    fn limits(&self) -> (Option<Vec3>, Option<Vec3>) {
        if self.lo.x > self.hi.x {
            (None, None)
        } else {
            (Some(self.lo.into()), Some(self.hi.into()))
        }
    }
    fn grip(&self, params: P3) -> Vec3 {
        self.point(params).into()
    }
    fn local(&self, point: &Bound<'_, PyAny>) -> PyResult<Vec3> {
        Ok((self.axes.transpose() * (xyz(point)? - self.origin)).into())
    }
    fn scale_grips(&self) -> Vec<(P3, Vec<usize>)> {
        self.geometry().grips.clone()
    }
    fn pick_scale(
        &self,
        view: &Bound<'_, PyAny>,
        cursor: Pixel,
        threshold: f64,
        eye: &Vec3,
        forward: &Vec3,
    ) -> PyResult<Option<usize>> {
        let projection = Projection::from_view(view)?;
        let mut best = None;
        for (index, (t, _)) in self.geometry().grips.iter().enumerate() {
            let p = self.point(*t);
            let Some([x, y]) = projection.pixel(p)? else {
                continue;
            };
            let dist = (x - cursor[0]).hypot(y - cursor[1]);
            if dist >= threshold {
                continue;
            }
            // Inside the painted grip choose the frontmost. In its margin
            // choose the nearest; stable order resolves coincident grips.
            let key = if dist <= 5. {
                (
                    false,
                    (p - DVec3::from(eye.coords())).dot(forward.coords().into()),
                )
            } else {
                (true, dist)
            };
            if best.is_none_or(|(old, _)| key < old) {
                best = Some((key, index));
            }
        }
        Ok(best.map(|(_, i)| i))
    }
    #[pyo3(signature=(factors, anchor=None, active=None, hover=None))]
    fn scale_preview<'py>(
        &self,
        py: Python<'py>,
        factors: P3,
        anchor: Option<&Vec3>,
        active: Option<P3>,
        hover: Option<P3>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let geometry = self.geometry();
        let point = |t| {
            let p = self.point(t);
            let p = anchor.map_or(p, |a| {
                let a = DVec3::from(a.coords());
                a + self.axes * (DVec3::from(factors) * (self.axes.transpose() * (p - a)))
            });
            Vec3::from(p)
        };
        let segments: Vec<_> = geometry
            .edges
            .iter()
            .copied()
            .map(|(a, b)| (point(a), point(b)))
            .collect();
        let mut grips: Vec<_> = geometry
            .grips
            .iter()
            .map(|&(t, _)| {
                let role = if active == Some(t) {
                    "active"
                } else if active.is_none() && hover == Some(t) {
                    "hover"
                } else {
                    "idle"
                };
                (point(t), role)
            })
            .collect();
        if let Some(a) = anchor {
            grips.push((Vec3::from(DVec3::from(a.coords())), "anchor"));
        }
        let out = PyDict::new(py);
        out.set_item("segments", segments)?;
        out.set_item("grips", grips)?;
        Ok(out)
    }
    fn flip_pick(&self, view: &Bound<'_, PyAny>, cursor: Pixel) -> PyResult<Option<&'static str>> {
        let proj = Projection::from_view(view)?;
        let center = (self.lo + self.hi) * 0.5;
        if proj.pixel(center)?.is_none() {
            return Ok(None);
        }
        let mut best = None;
        for axis in ["x", "y", "z"] {
            let quad = flip::quad(
                axis,
                center.to_array(),
                ((self.hi - self.lo) * 0.5).to_array(),
                1.15,
            );
            let mut pixels = [[0.; 2]; 4];
            let mut visible = true;
            for (pixel, corner) in pixels.iter_mut().zip(quad) {
                if let Some(p) = proj.pixel(corner.into())? {
                    *pixel = p
                } else {
                    visible = false;
                    break;
                }
            }
            if !visible {
                continue;
            }
            let mut inside = false;
            for (a, b) in pixels.iter().zip(pixels.iter().cycle().skip(1)) {
                if (a[1] > cursor[1]) != (b[1] > cursor[1])
                    && cursor[0] < (b[0] - a[0]) * (cursor[1] - a[1]) / (b[1] - a[1]) + a[0]
                {
                    inside = !inside
                }
            }
            if inside {
                let distance = (0..2)
                    .map(|i| (pixels.iter().map(|p| p[i]).sum::<f64>() * 0.25 - cursor[i]).powi(2))
                    .sum::<f64>();
                if best.is_none_or(|(d, _)| distance < d) {
                    best = Some((distance, axis));
                }
            }
        }
        Ok(best.map(|(_, axis)| axis))
    }
}

#[pyfunction]
pub fn protractor_metrics(
    view: &Bound<'_, PyAny>,
    centre: &Vec3,
    basis: [P3; 2],
    cursor: Pixel,
    placed: bool,
    radius: f64,
) -> PyResult<Option<(f64, bool)>> {
    let projection = Projection::from_view(view)?;
    let c = DVec3::from(centre.coords());
    let Some([x, y]) = projection.pixel(c)? else {
        return Ok(None);
    };
    let mut scale: f64 = 0.;
    for axis in basis {
        let Some([u, v]) = projection.pixel(c + DVec3::from(axis))? else {
            return Ok(None);
        };
        scale = scale.max((u - x).hypot(v - y));
    }
    Ok(Some((
        if scale > 1e-6 { 60. / scale } else { radius },
        placed && (cursor[0] - x).hypot(cursor[1] - y) <= 75.,
    )))
}

#[pyfunction]
pub fn move_grip_hit(
    view: &Bound<'_, PyAny>,
    grips: &Bound<'_, PyAny>,
    cursor: Pixel,
    margin: f64,
) -> PyResult<Option<usize>> {
    let proj = Projection::from_view(view)?;
    let (mut best, mut distance) = (None, margin);
    for (i, g) in grips.try_iter()?.enumerate() {
        let p = xyz(&g?.get_item(0)?)?;
        if let Some([x, y]) = proj.pixel(p)? {
            let d = (x - cursor[0]).hypot(y - cursor[1]);
            if d <= distance {
                best = Some(i);
                distance = d;
            }
        }
    }
    Ok(best)
}

#[pyfunction]
pub fn move_box_contains(
    view: &Bound<'_, PyAny>,
    frame: [P3; 3],
    lo: P3,
    hi: P3,
    cursor: Pixel,
    margin: f64,
) -> PyResult<bool> {
    let proj = Projection::from_view(view)?;
    let [u, v, w] = frame.map(DVec3::from);
    let mut points = Vec::with_capacity(8);
    for x in [lo[0], hi[0]] {
        for y in [lo[1], hi[1]] {
            for z in [lo[2], hi[2]] {
                let Some(p) = proj.pixel(u * x + v * y + w * z)? else {
                    return Ok(false);
                };
                points.push(p);
            }
        }
    }
    Ok(super::move_grips::hover_hull(points, cursor, margin))
}
