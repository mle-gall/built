// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
enum Framing {
    Aspect(f64),
    PanZoom(f64, f64, f64),
    Fit(Py<PyAny>),
}
#[pyclass]
pub(super) struct CameraScope {
    camera: Py<PyAny>,
    change: Framing,
    saved: Option<(Py<PyAny>, f64, f64, f64)>,
}
impl CameraScope {
    fn new(camera: Py<PyAny>, change: Framing) -> Self {
        Self {
            camera,
            change,
            saved: None,
        }
    }
    fn restore(&mut self, py: Python) -> PyResult<()> {
        if let Some((target, distance, fov, aspect)) = self.saved.take() {
            let c = self.camera.bind(py);
            c.setattr("target", target)?;
            c.setattr("distance", distance)?;
            c.setattr("fov_deg", fov)?;
            c.setattr("aspect", aspect)?;
        }
        Ok(())
    }
    fn apply(&self, c: &Obj, distance: f64, fov: f64, old_aspect: f64) -> PyResult<()> {
        let py = c.py();
        let grow = match &self.change {
            Framing::Aspect(aspect) => {
                c.setattr("aspect", *aspect)?;
                if *aspect >= old_aspect {
                    return Ok(());
                }
                old_aspect / aspect
            }
            Framing::Fit(bounds) => {
                let (lo, hi): (Obj, Obj) = bounds.bind(py).extract()?;
                if !lo.is_none() && !hi.is_none() {
                    c.call_method1("fit_box", (lo, hi, 1.02))?;
                }
                return Ok(());
            }
            Framing::PanZoom(x, y, z) => {
                let (x0, y0, x1, y1) = rect(c)?;
                let basis = py
                    .import("ingetrazo_core")?
                    .call_method1("hlr_camera_basis", (c,))?;
                let right: [f64; 3] = basis.get_item(1)?.extract()?;
                let up: [f64; 3] = basis.get_item(2)?.extract()?;
                let shift = |i: usize| right[i] * x * (x1 - x0) + up[i] * y * (y1 - y0);
                let v = py.import("ingetrazo_core")?.getattr("Vec3")?.call1((
                    shift(0),
                    shift(1),
                    shift(2),
                ))?;
                c.setattr("target", c.getattr("target")?.add(v)?)?;
                if (z - 1.).abs() < 1e-9 {
                    return Ok(());
                }
                1. / z.max(1e-3)
            }
        };
        if c.getattr("perspective")?.is_truthy()? {
            c.setattr(
                "fov_deg",
                (2. * ((fov.to_radians() / 2.).tan() * grow).atan()).to_degrees(),
            )
        } else {
            c.setattr("distance", distance * grow)
        }
    }
}
#[pymethods]
impl CameraScope {
    fn __enter__(&mut self, py: Python) -> PyResult<()> {
        let c = self.camera.bind(py);
        let distance = c.getattr("distance")?.extract()?;
        let fov = c.getattr("fov_deg")?.extract()?;
        let aspect = c.getattr("aspect")?.extract()?;
        self.saved = Some((c.getattr("target")?.unbind(), distance, fov, aspect));
        if let Err(e) = self.apply(c, distance, fov, if aspect == 0. { 1. } else { aspect }) {
            self.restore(py)?;
            return Err(e);
        }
        Ok(())
    }
    fn __exit__(&mut self, py: Python, _kind: &Obj, _err: &Obj, _tb: &Obj) -> PyResult<()> {
        self.restore(py)
    }
}
#[pyfunction]
pub(super) fn widened(camera: Py<PyAny>, aspect: f64) -> CameraScope {
    CameraScope::new(camera, Framing::Aspect(aspect))
}
#[pyfunction]
pub(super) fn framed(camera: Py<PyAny>, frame: [f64; 3]) -> CameraScope {
    CameraScope::new(camera, Framing::PanZoom(frame[0], frame[1], frame[2]))
}
pub(super) fn fitted(camera: Py<PyAny>, bounds: Py<PyAny>) -> CameraScope {
    CameraScope::new(camera, Framing::Fit(bounds))
}

fn selected(chosen: &Obj, e: &Obj) -> PyResult<bool> {
    if chosen.contains(e)? {
        return Ok(true);
    }
    if let Some(faces) = e.getattr_opt("faces")? {
        for f in faces.try_iter()? {
            if chosen.contains(f?)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
#[pyclass]
pub(super) struct SelectionScope {
    scene: Py<PyAny>,
    previous: Option<Py<PyAny>>,
}
#[pyfunction]
pub(super) fn selection_only(scene: Py<PyAny>) -> SelectionScope {
    SelectionScope {
        scene,
        previous: None,
    }
}
#[pymethods]
impl SelectionScope {
    fn __enter__(&mut self, py: Python) -> PyResult<()> {
        let scene = self.scene.bind(py);
        let chosen = scene.getattr("selection")?.call_method0("copy")?.unbind();
        self.previous = scene
            .getattr("__dict__")?
            .cast_into::<PyDict>()?
            .get_item("entity_visible")?
            .map(Bound::unbind);
        let shown = scene.getattr("entity_visible")?.unbind();
        let filter = crate::settings_rows::func(py, move |py, args| {
            let e = args.get_item(0)?;
            crate::settings_rows::obj(
                py,
                selected(chosen.bind(py), &e)? && shown.bind(py).call1((e,))?.is_truthy()?,
            )
        })?;
        scene.setattr("entity_visible", filter)
    }
    fn __exit__(&mut self, py: Python, _kind: &Obj, _err: &Obj, _tb: &Obj) -> PyResult<()> {
        let scene = self.scene.bind(py);
        match self.previous.take() {
            Some(v) => scene.setattr("entity_visible", v),
            None => scene.delattr("entity_visible"),
        }
    }
}

/// Find the roots to hide. A selected parent keeps its entire subtree. A
/// partially selected subtree keeps its ancestors and hides its siblings.
fn hide_group<'p>(
    g: &Obj<'p>,
    chosen: &Obj,
    hist: &Bound<PyModule>,
    out: &mut Vec<Obj<'p>>,
) -> PyResult<bool> {
    if chosen.contains(g)? {
        return Ok(true);
    }
    let start = out.len();
    let mut contains = false;
    if let Some(children) = g.getattr_opt("children")?.filter(|v| !v.is_none()) {
        for child in children.try_iter()? {
            contains |= hide_group(&child?, chosen, hist, out)?;
        }
    }
    if !contains {
        out.truncate(start);
        if !hist.call_method1("_is_hidden", (g,))?.is_truthy()? {
            out.push(g.clone())
        }
    }
    Ok(contains)
}
#[pyclass]
pub(super) struct PictureScope {
    vp: Py<PyAny>,
    scene: Py<PyAny>,
    hidden: Vec<Py<PyAny>>,
}
impl PictureScope {
    pub(super) fn new(vp: &Obj) -> PyResult<Self> {
        Ok(Self {
            vp: vp.clone().unbind(),
            scene: vp.getattr("scene")?.unbind(),
            hidden: vec![],
        })
    }
    fn bump(&self, py: Python) -> PyResult<()> {
        let scene = self.scene.bind(py);
        py.import("core.history")?
            .call_method1("_dirty_group_chunks", (scene,))?;
        scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)
    }
    fn unhide(&mut self, py: Python) -> PyResult<()> {
        let hist = py.import("core.history")?;
        for e in self.hidden.drain(..) {
            hist.call_method1("_set_hidden", (e, false))?;
        }
        Ok(())
    }
}
#[pymethods]
impl PictureScope {
    fn __enter__(&mut self, py: Python) -> PyResult<()> {
        let scene = self.scene.bind(py);
        let hist = py.import("core.history")?;
        let chosen = scene.getattr("selection")?;
        let mut hide = Vec::new();
        for g in scene.getattr("groups")?.try_iter()? {
            hide_group(&g?, &chosen, &hist, &mut hide)?;
        }
        for field in ["faces", "edges"] {
            for e in scene.getattr(field)?.try_iter()? {
                let e = e?;
                let keep = if field == "edges" {
                    selected(&chosen, &e)?
                } else {
                    chosen.contains(&e)?
                };
                if !keep && !hist.call_method1("_is_hidden", (&e,))?.is_truthy()? {
                    hide.push(e);
                }
            }
        }
        let result = (|| {
            for e in hide {
                hist.call_method1("_set_hidden", (&e, true))?;
                self.hidden.push(e.unbind());
            }
            self.bump(py)
        })();
        if result.is_err() {
            self.unhide(py)?;
        }
        result
    }
    fn __exit__(&mut self, py: Python, _kind: &Obj, _err: &Obj, _tb: &Obj) -> PyResult<()> {
        self.unhide(py)?;
        self.bump(py)?;
        let s = self.scene.bind(py);
        s.setattr(
            "view_version",
            s.getattr("view_version")?.extract::<u64>()? + 2,
        )?;
        self.vp.bind(py).call_method0("update")?;
        Ok(())
    }
}
