// SPDX-License-Identifier: GPL-3.0-or-later
//! Cursor inference for document and renderer picks. Reference points are
//! binary64 values. Object references are scoped to the owning viewport.
use crate::math::{xyz, Vec3};
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
    IntoPyObjectExt,
};
type Obj<'p> = Bound<'p, PyAny>;
mod query;
fn none(py: Python) -> Obj {
    py.None().into_bound(py)
}
fn attr<'p>(obj: &Obj<'p>, key: &str) -> PyResult<Obj<'p>> {
    Ok(obj.getattr_opt(key)?.unwrap_or_else(|| none(obj.py())))
}
fn flag(obj: &Obj, key: &str) -> PyResult<bool> {
    attr(obj, key)?.is_truthy()
}
#[pyclass(module = "built_core.inference", dict, weakref, subclass)]
#[derive(Default)]
pub(crate) struct InferenceEngine {
    #[pyo3(get, set)]
    viewport: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    axis_lock: Option<String>,
    #[pyo3(get, set)]
    last_snap: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    reference_edge: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    reference_mode: Option<String>,
    #[pyo3(get, set)]
    _shift_lock: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    _sticky_axis: Option<String>,
    #[pyo3(get, set)]
    _shift_lock_ref: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    _hover_edge: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    _hover_center: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    _center_ref: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    _acquired_edge: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    _acquired_face_normal: Option<Py<PyAny>>,
    encouraged: Vec<DVec3>,
    dwell: Option<DVec3>,
    acquired: Option<DVec3>,
    timer: Option<Py<PyAny>>,
}
#[pyfunction]
fn on_dwell(reference: &Obj) -> PyResult<()> {
    let engine = reference.call0()?;
    if !engine.is_none() {
        engine.call_method0("_encourage_dwelt")?;
    }
    Ok(())
}
#[pymethods]
impl InferenceEngine {
    #[new]
    #[pyo3(signature=(viewport=None))]
    fn new(viewport: Option<Py<PyAny>>) -> Self {
        Self {
            viewport,
            ..Self::default()
        }
    }
    fn __traverse__(&self, visit: pyo3::PyVisit) -> Result<(), pyo3::PyTraverseError> {
        for value in [
            &self.viewport,
            &self.last_snap,
            &self.reference_edge,
            &self._shift_lock,
            &self._shift_lock_ref,
            &self._hover_edge,
            &self._hover_center,
            &self._center_ref,
            &self._acquired_edge,
            &self._acquired_face_normal,
            &self.timer,
        ] {
            if let Some(v) = value {
                visit.call(v)?;
            }
        }
        Ok(())
    }
    fn __clear__(&mut self) {
        *self = Self::default();
    }
    #[getter(_encouraged)]
    fn encouraged(&self) -> Vec<Vec3> {
        self.encouraged.iter().copied().map(Vec3::from).collect()
    }
    #[setter(_encouraged)]
    fn set_encouraged(&mut self, points: &Obj) -> PyResult<()> {
        self.encouraged = points
            .try_iter()?
            .map(|p| xyz(&p?))
            .collect::<PyResult<_>>()?;
        Ok(())
    }
    #[getter(_acquired_point)]
    fn acquired(&self) -> Option<Vec3> {
        self.acquired.map(Vec3::from)
    }
    #[setter(_acquired_point)]
    fn set_acquired(&mut self, p: Option<&Obj>) -> PyResult<()> {
        self.acquired = p.map(xyz).transpose()?;
        Ok(())
    }
    #[getter(_dwell_point)]
    fn dwell(&self) -> Option<Vec3> {
        self.dwell.map(Vec3::from)
    }
    #[setter(_dwell_point)]
    fn set_dwell(&mut self, p: Option<&Obj>) -> PyResult<()> {
        self.dwell = p.map(xyz).transpose()?;
        Ok(())
    }
    #[getter(_dwell_timer)]
    fn dwell_timer<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        let py = slf.py();
        if let Some(timer) = &slf.borrow().timer {
            return Ok(timer.bind(py).clone());
        }
        let ref_ = py.import("weakref")?.call_method1("ref", (slf,))?;
        let cb = py
            .import("functools")?
            .call_method1("partial", (wrap_pyfunction!(on_dwell, py)?, ref_))?;
        let vp = slf.getattr("viewport")?;
        let timer = py
            .import("built_core.timers")?
            .call_method1("timer", (&vp, cb, vp.getattr("ENCOURAGE_MS")?))?;
        slf.borrow_mut().timer = Some(timer.clone().unbind());
        Ok(timer)
    }
    fn _dwell_on(slf: &Bound<Self>, point: Option<&Obj>) -> PyResult<()> {
        let p = point.map(xyz).transpose()?;
        if let Some(p) = p {
            if slf
                .borrow()
                .dwell
                .is_some_and(|prev| prev.distance(p) < 1e-6)
            {
                return Ok(());
            }
        }
        slf.borrow_mut().dwell = p;
        Self::dwell_timer(slf)?.call_method0(if p.is_some() { "start" } else { "stop" })?;
        Ok(())
    }
    fn _encourage_dwelt(slf: &Bound<Self>) -> PyResult<()> {
        let p = slf.borrow().dwell;
        if let Some(p) = p {
            slf.call_method1("encourage_point", (Vec3::from(p),))?;
        }
        Ok(())
    }
    fn encourage_point(slf: &Bound<Self>, point: &Obj) -> PyResult<()> {
        let p = xyz(point)?;
        {
            let mut s = slf.borrow_mut();
            s.encouraged.retain(|q| q.distance(p) > 1e-6);
            s.encouraged.push(p);
            let n = s.encouraged.len().saturating_sub(2);
            s.encouraged.drain(..n);
            s.acquired = Some(p);
        }
        slf.getattr("viewport")?.call_method0("update")?;
        Ok(())
    }
    fn clear_references(&mut self, py: Python) -> PyResult<()> {
        if let Some(timer) = &self.timer {
            timer.bind(py).call_method0("stop")?;
        }
        self._sticky_axis = None;
        self._shift_lock_ref = None;
        self._hover_edge = None;
        self._hover_center = None;
        self._center_ref = None;
        self._acquired_edge = None;
        self._acquired_face_normal = None;
        self.acquired = None;
        self.encouraged.clear();
        self.dwell = None;
        self.last_snap = None;
        Ok(())
    }
    fn reset(&mut self, py: Python) -> PyResult<()> {
        self.clear_references(py)?;
        self.axis_lock = None;
        self.reference_edge = None;
        self.reference_mode = None;
        self._shift_lock = None;
        Ok(())
    }
    fn direction_locked(&self, modifiers: i64) -> bool {
        self.axis_lock.as_ref().is_some_and(|s| !s.is_empty())
            || self._shift_lock.is_some()
            || self.reference_edge.is_some()
                && self.reference_mode.as_ref().is_some_and(|s| !s.is_empty())
            || modifiers & SHIFT != 0
    }
    #[pyo3(signature=(tool,edge=None,face_normal=None,corner=None))]
    fn note_drawing_refs(
        &mut self,
        tool: &Obj,
        edge: Option<&Obj>,
        face_normal: Option<&Obj>,
        corner: Option<&Obj>,
    ) -> PyResult<()> {
        if !flag(tool, "uses_snap")? || attr(tool, "start_point")?.is_none() {
            self._acquired_edge = None;
            self._acquired_face_normal = None;
            return Ok(());
        }
        if let Some(e) = edge {
            self._acquired_edge = Some(e.clone().unbind());
        }
        if let Some(n) = face_normal {
            self._acquired_face_normal = Some(n.clone().unbind());
        }
        if let Some(c) = corner {
            self.acquired = Some(xyz(c)?);
        }
        Ok(())
    }
    #[pyo3(signature=(px,py))]
    fn acquire(slf: &Bound<Self>, px: f64, py: f64) -> PyResult<()> {
        let vp = slf.getattr("viewport")?;
        let tool = vp.getattr("active_tool")?;
        let uses = flag(&tool, "uses_snap")?;
        let hover = vp.call_method1(
            if uses || flag(&tool, "hover_group_edges")? {
                "pick_edge_any"
            } else {
                "pick_edge"
            },
            (px, py),
        )?;
        slf.setattr("_hover_edge", &hover)?;
        if uses {
            slf.setattr(
                "_hover_center",
                vp.call_method1("_update_center_ref", (px, py))?,
            )?;
        }
        if attr(&tool, "start_point")?.is_none() {
            let mut s = slf.borrow_mut();
            s._acquired_edge = None;
            s._acquired_face_normal = None;
        } else {
            if uses {
                let p = vp.call_method1("pick_vertex", (px, py))?;
                if !p.is_none() {
                    slf.borrow_mut().acquired = Some(xyz(&p)?);
                }
            }
            if !hover.is_none() {
                slf.setattr("_acquired_edge", hover)?;
            }
            if let Some((_, n)) = query::pick_plane(&vp, px, py)? {
                slf.setattr("_acquired_face_normal", n)?;
            }
        }
        Ok(())
    }
    fn snap_options<'p>(
        slf: &Bound<'p, Self>,
        tool: &Obj<'p>,
        modifiers: i64,
    ) -> PyResult<Bound<'p, PyDict>> {
        query::options(slf, tool, modifiers)
    }
    fn snap<'p>(slf: &Bound<'p, Self>, px_x: f64, px_y: f64, modifiers: i64) -> PyResult<Obj<'p>> {
        query::snap(slf, px_x, px_y, modifiers)
    }
    fn _hover_lock_plane<'p>(slf: &Bound<'p, Self>, px: f64, py: f64) -> PyResult<Obj<'p>> {
        query::hover_lock(&slf.getattr("viewport")?, px, py)
    }
    fn _face_centers<'p>(slf: &Bound<'p, Self>, px_x: f64, px_y: f64) -> PyResult<Obj<'p>> {
        query::centers(&slf.getattr("viewport")?, px_x, px_y)
    }
    #[pyo3(signature=(snap,px_x,px_y,picks=None))]
    fn _axis_source_cue<'p>(
        slf: &Bound<'p, Self>,
        snap: &Obj<'p>,
        px_x: f64,
        px_y: f64,
        picks: Option<&Obj<'p>>,
    ) -> PyResult<Obj<'p>> {
        query::axis_cue(picks.unwrap_or(&slf.getattr("viewport")?), snap, px_x, px_y)
    }
    fn _extension_snap<'p>(
        slf: &Bound<'p, Self>,
        snap: &Obj<'p>,
        px_x: f64,
        px_y: f64,
    ) -> PyResult<Obj<'p>> {
        query::extension(&slf.getattr("viewport")?, snap, px_x, px_y)
    }
    fn _limit_snap<'p>(&self, snap: &Obj<'p>, tool: &Obj) -> PyResult<Obj<'p>> {
        query::limit(snap, tool)
    }
    #[pyo3(signature=(snap,px_x,px_y,tool,picks=None))]
    fn finish_snap<'p>(
        slf: &Bound<'p, Self>,
        snap: &Obj<'p>,
        px_x: f64,
        px_y: f64,
        tool: &Obj,
        picks: Option<&Obj<'p>>,
    ) -> PyResult<Obj<'p>> {
        slf.borrow_mut()._sticky_axis = if !snap.is_none() && snap.getattr("kind")?.eq("axis")? {
            snap.getattr("axis")?.extract()?
        } else {
            None
        };
        let vp = slf.getattr("viewport")?;
        let cue = query::axis_cue(picks.unwrap_or(&vp), snap, px_x, px_y)?;
        query::limit(&query::extension(&vp, &cue, px_x, px_y)?, tool)
    }
}
const SHIFT: i64 = 0x02000000;
pub(crate) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_class::<InferenceEngine>()?;
    for (attr, env) in [
        ("_STICKY_AXIS", "BUILT_STICKY_AXIS"),
        ("_DWELL_ARROW", "BUILT_DWELL_ARROW"),
    ] {
        m.add(attr, std::env::var_os(env).is_some_and(|v| v == "1"))?;
    }
    Ok(())
}
