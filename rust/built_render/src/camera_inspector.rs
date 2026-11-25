//! Keep scene-specific camera edits attached to the original saved view.
//! Camera physics and serialization still belong to OrbitCamera and SavedView.
use pyo3::{prelude::*, types::PyTuple};

#[pyclass]
#[derive(Default)]
pub struct CameraInspector {
    #[pyo3(get, set)]
    advanced: bool,
    #[pyo3(get, set)]
    custom_sensor: bool,
    target: Option<Py<PyAny>>,
    camera: Option<Py<PyAny>>,
    last: Option<Py<PyTuple>>,
}

fn pose<'py>(cam: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyTuple>> {
    let py = cam.py();
    PyTuple::new(
        py,
        [
            cam.getattr("target")?.call_method0("toTuple")?,
            cam.getattr("distance")?,
            cam.getattr("yaw")?,
            cam.getattr("pitch")?,
            cam.getattr("fov_deg")?,
            cam.call_method0("projection_mode")?,
            cam.call_method0("lens_dict")?,
        ],
    )
}

#[pymethods]
impl CameraInspector {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    fn bind(&mut self, cam: &Bound<'_, PyAny>, target: &Bound<'_, PyAny>) -> PyResult<()> {
        self.clear();
        if !target.is_none() {
            self.last = Some(pose(cam)?.unbind());
            self.camera = Some(cam.clone().unbind());
            self.target = Some(target.clone().unbind());
        }
        Ok(())
    }

    fn clear(&mut self) {
        self.target = None;
        self.camera = None;
        self.last = None;
    }

    /// Called before rendering and before each edit, including edits from an
    /// already painted frame. A deleted or replaced scene cannot receive them.
    fn observe(
        &mut self,
        cam: &Bound<'_, PyAny>,
        scene: &Bound<'_, PyAny>,
        marked: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        let py = cam.py();
        let Some(target) = &self.target else {
            return Ok(false);
        };
        let valid = target.bind(py).is(marked)
            && self.camera.as_ref().is_some_and(|c| c.bind(py).is(cam))
            && self.last.as_ref().unwrap().bind(py).eq(pose(cam)?)?
            && scene
                .getattr("saved_views")?
                .try_iter()?
                .any(|v| v.is_ok_and(|v| v.is(marked)));
        if !valid {
            self.clear();
        }
        Ok(valid)
    }

    /// Commit only the fields stored by SavedView.take_camera. An edit which
    /// leaves the camera unchanged leaves the document's revision alone.
    fn applied(&mut self, cam: &Bound<'_, PyAny>, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = cam.py();
        if let (Some(target), Some(last)) = (&self.target, &self.last) {
            let now = pose(cam)?;
            if self.camera.as_ref().unwrap().bind(py).is(cam) && !now.eq(last.bind(py))? {
                target.call_method1(py, "take_camera", (cam,))?;
                let version: u64 = scene.getattr("version")?.extract()?;
                scene.setattr("version", version + 1)?;
                self.last = Some(now.unbind());
            }
        }
        Ok(())
    }
}
