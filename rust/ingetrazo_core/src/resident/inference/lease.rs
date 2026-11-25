// SPDX-License-Identifier: GPL-3.0-or-later
//! Hold a scene during background reads; release it before building the index.
use pyo3::prelude::*;
#[pyclass(module = "ingetrazo_core", skip_from_py_object)]
pub(super) struct SceneLease {
    #[pyo3(get, set)]
    scene: Option<Py<PyAny>>,
    #[pyo3(get)]
    released: bool,
    #[pyo3(get, name = "_reading")]
    reading: Py<PyAny>,
}
#[pymethods]
impl SceneLease {
    #[new]
    fn new(scene: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            scene: Some(scene.clone().unbind()),
            released: false,
            reading: scene
                .py()
                .import("threading")?
                .call_method0("Lock")?
                .unbind(),
        })
    }
    fn release(this: &Bound<'_, Self>) -> PyResult<()> {
        let py = this.py();
        let lock = {
            let mut lease = this.borrow_mut();
            lease.released = true;
            lease.reading.clone_ref(py)
        };
        let lock = lock.bind(py);
        lock.call_method0("acquire")?;
        this.borrow_mut().scene = None;
        lock.call_method0("release")?;
        Ok(())
    }
}
