// SPDX-License-Identifier: GPL-3.0-or-later
//! The shared Rotate/Protractor cursor frame. Machine arms stay authoritative;
//! the frame owns inference, arrow/Shift locks and the click-drag tilt.
use super::{
    host_geometry::{protractor_metrics, Projection},
    plane::{plane_axes, Axes},
    protractor, P3,
};
use crate::math::{xyz, Vec3};
use glam::DVec3;
use pyo3::prelude::*;

#[pyclass(module = "built_core", subclass)]
#[derive(Default)]
pub struct ToolDisc {
    #[pyo3(get, set)]
    work_plane: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    _centre_plane: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    _axis_pick: Option<String>,
    #[pyo3(get, set)]
    _custom_axis: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    _shift_normal: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    _axis_drag_live: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    _axis_drag_armed: bool,
    #[pyo3(get, set)]
    _disc_r: f64,
    #[pyo3(get, set)]
    _snap_ticks: bool,
    #[pyo3(get, set)]
    _exact_snap: bool,
}
fn drawing_axes(py: Python<'_>) -> PyResult<Axes> {
    let a = py.import("built_core.axes")?.getattr("AXES")?;
    Ok(Axes {
        o: DVec3::ZERO,
        x: xyz(&a.get_item("x")?)?,
        y: xyz(&a.get_item("y")?)?,
        z: xyz(&a.get_item("z")?)?,
    })
}
fn vector(s: &Bound<'_, ToolDisc>, key: &str) -> PyResult<Option<DVec3>> {
    let obj = s.getattr(key)?;
    if obj.is_none() {
        Ok(None)
    } else {
        xyz(&obj).map(Some)
    }
}
impl ToolDisc {
    fn axis(&self, py: Python<'_>, axes: &Axes) -> PyResult<DVec3> {
        if let Some(axis) = self._custom_axis.as_ref().or(self._shift_normal.as_ref()) {
            return xyz(axis.bind(py));
        }
        if let Some(axis) = &self._axis_pick {
            return Ok(match axis.as_str() {
                "x" => axes.x,
                "y" => axes.y,
                _ => axes.z,
            });
        }
        if let Some(plane) = &self.work_plane {
            return Ok(xyz(&plane.bind(py).get_item(1)?)?.normalize_or_zero());
        }
        Ok(DVec3::Z)
    }
    fn basis(&self, py: Python<'_>) -> PyResult<[P3; 2]> {
        let axes = drawing_axes(py)?;
        let (u, v) = plane_axes(self.axis(py, &axes)?, &axes);
        Ok([u.to_array(), v.to_array()])
    }
    fn arms(s: &Bound<'_, Self>, p: &Bound<'_, PyAny>) -> PyResult<Option<(P3, P3)>> {
        Ok(vector(s, "start_point")?
            .zip(vector(s, "ref_point")?)
            .map(|(c, r)| xyz(p).map(|p| ((r - c).to_array(), (p - c).to_array())))
            .transpose()?)
    }
}
#[pymethods]
impl ToolDisc {
    #[new]
    fn new() -> Self {
        Self {
            _disc_r: 1.,
            ..Default::default()
        }
    }
    fn _reset_protractor(slf: &Bound<'_, Self>) -> PyResult<()> {
        {
            let mut d = slf.borrow_mut();
            let pick = d._axis_pick.take();
            *d = Self {
                _axis_pick: pick,
                _disc_r: d._disc_r,
                ..Default::default()
            };
        }
        slf.setattr("start_point", slf.py().None())?;
        slf.setattr("ref_point", slf.py().None())
    }
    fn _track_axis_drag(slf: &Bound<'_, Self>, view: &Bound<'_, PyAny>) -> PyResult<()> {
        let mut axis = None;
        if slf.borrow()._axis_drag_armed {
            if let (Some(c), Some(h)) = (vector(slf, "start_point")?, vector(slf, "hover_point")?) {
                let visible = if view.hasattr("_world_to_pixel")? {
                    let proj = Projection::from_view(view)?;
                    matches!((proj.pixel(c)?,proj.pixel(h)?),(Some(a),Some(b)) if (a[0]-b[0]).hypot(a[1]-b[1])>8.)
                } else {
                    true
                };
                let d = h - c;
                if visible && d.length() > 1e-9 {
                    axis = Some(Py::new(slf.py(), Vec3::from(d.normalize()))?.into_any())
                }
            }
        }
        slf.borrow_mut()._axis_drag_live = axis;
        Ok(())
    }
    fn _release_axis_drag(slf: &Bound<'_, Self>, view: &Bound<'_, PyAny>) -> PyResult<bool> {
        if !slf.borrow()._axis_drag_armed {
            return Ok(false);
        }
        Self::_track_axis_drag(slf, view)?;
        {
            let mut d = slf.borrow_mut();
            d._axis_drag_armed = false;
            let Some(n) = d._axis_drag_live.take() else {
                return Ok(false);
            };
            d._custom_axis = Some(n);
        }
        view.call_method0("update")?;
        Ok(true)
    }
    fn lock_disc(slf: &Bound<'_, Self>, view: &Bound<'_, PyAny>, axis: &str) -> PyResult<bool> {
        let py = slf.py();
        let c = vector(slf, "start_point")?;
        let h = vector(slf, "hover_point")?;
        {
            let mut d = slf.borrow_mut();
            if d._axis_pick.as_deref() == Some(axis) {
                d._axis_pick = None;
                if c.is_some() && d._centre_plane.is_some() {
                    d.work_plane = d._centre_plane.as_ref().map(|p| p.clone_ref(py))
                }
            } else {
                d._axis_pick = Some(axis.to_owned());
                let n = xyz(&py.import("built_core.axes")?.getattr("AXES")?.get_item(axis)?)?;
                d.work_plane = Some(
                    (Vec3::from(c.or(h).unwrap_or_default()), Vec3::from(n))
                        .into_pyobject(py)?
                        .into_any()
                        .unbind(),
                );
            }
        }
        view.call_method0("update")?;
        Ok(true)
    }
    fn _axis(&self, py: Python<'_>) -> PyResult<Vec3> {
        Ok(self.axis(py, &drawing_axes(py)?)?.into())
    }
    fn _basis(&self, py: Python<'_>) -> PyResult<[P3; 2]> {
        self.basis(py)
    }
    fn infer_disc(slf: &Bound<'_, Self>, ctx: &Bound<'_, PyAny>, shift: bool) -> PyResult<()> {
        let py = slf.py();
        if !shift {
            slf.borrow_mut()._shift_normal = None
        }
        if vector(slf, "start_point")?.is_some() {
            return Ok(());
        }
        if shift {
            if slf.borrow()._shift_normal.is_none() {
                let n = slf.borrow()._axis(py)?;
                slf.borrow_mut()._shift_normal = Some(Py::new(py, n)?.into_any())
            }
            return Ok(());
        }
        let lock = slf.borrow()._axis_pick.clone();
        let n = if let Some(lock) = lock {
            Some(xyz(&py
                .import("built_core.axes")?
                .getattr("AXES")?
                .get_item(lock)?)?)
        } else {
            let view = ctx.getattr("viewport")?;
            if let Some(pick) = view.getattr_opt("pick_face_any")? {
                let screen = ctx.getattr("screen")?;
                let face = pick
                    .call1((screen.call_method0("x")?, screen.call_method0("y")?))?
                    .get_item(0)?;
                if face.is_none() {
                    None
                } else {
                    Some(xyz(&face.call_method0("normal")?)?.normalize_or_zero())
                }
            } else {
                None
            }
        };
        slf.borrow_mut().work_plane = n
            .map(|n| {
                Ok::<_, PyErr>(
                    (ctx.getattr("world")?, Vec3::from(n))
                        .into_pyobject(py)?
                        .into_any()
                        .unbind(),
                )
            })
            .transpose()?;
        Ok(())
    }
    fn _update_screen_metrics(slf: &Bound<'_, Self>, ctx: &Bound<'_, PyAny>) -> PyResult<()> {
        let c = vector(slf, "start_point")?;
        let Some(pos) = c.or(vector(slf, "hover_point")?) else {
            return Ok(());
        };
        let view = ctx.getattr("viewport")?;
        if !view.hasattr("_world_to_pixel")? {
            return Ok(());
        }
        let screen = ctx.getattr("screen")?;
        let cursor = [
            screen.call_method0("x")?.extract()?,
            screen.call_method0("y")?.extract()?,
        ];
        let result = {
            let d = slf.borrow();
            protractor_metrics(
                &view,
                &pos.into(),
                d.basis(slf.py())?,
                cursor,
                c.is_some(),
                d._disc_r,
            )?
        };
        if let Some((radius, near)) = result {
            let mut d = slf.borrow_mut();
            d._disc_r = radius;
            d._snap_ticks = near
        }
        Ok(())
    }
    fn _note_snap(&mut self, ctx: &Bound<'_, PyAny>) -> PyResult<()> {
        let kind = ctx
            .getattr_opt("snap")?
            .map(|s| s.getattr_opt("kind"))
            .transpose()?
            .flatten()
            .filter(|s| !s.is_none())
            .map(|s| s.extract::<String>())
            .transpose()?;
        self._exact_snap = !matches!(kind.as_deref(), None | Some("" | "none" | "on_face"));
        Ok(())
    }
    fn _angle_to(slf: &Bound<'_, Self>, p: &Bound<'_, PyAny>) -> PyResult<Option<f64>> {
        let Some((a, b)) = Self::arms(slf, p)? else {
            return Ok(None);
        };
        Ok(protractor::angle(slf.borrow().basis(slf.py())?, a, b))
    }
    fn _display_deg(slf: &Bound<'_, Self>, p: &Bound<'_, PyAny>) -> PyResult<Option<f64>> {
        Ok(Self::_angle_to(slf, p)?.map(|a| protractor::display(a, slf.borrow()._snap_ticks)))
    }
    fn _commit_deg(slf: &Bound<'_, Self>, p: &Bound<'_, PyAny>) -> PyResult<Option<f64>> {
        let d = slf.borrow();
        if d._exact_snap && !d._snap_ticks {
            Self::_angle_to(slf, p)
        } else {
            Self::_display_deg(slf, p)
        }
    }
    fn _direction_at(slf: &Bound<'_, Self>, deg: f64) -> PyResult<Vec3> {
        let arm = vector(slf, "ref_point")?.unwrap_or_default()
            - vector(slf, "start_point")?.unwrap_or_default();
        Ok(DVec3::from(protractor::direction(
            slf.borrow().basis(slf.py())?,
            arm.to_array(),
            deg,
        ))
        .into())
    }
}
