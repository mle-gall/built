//! Last input delivered to one winit window, including its nested modal loop.
use pyo3::prelude::*;

#[derive(Default)]
#[pyclass(module = "built_core")]
pub struct WindowInput {
    #[pyo3(get, set)]
    modifiers: u32,
    #[pyo3(get)]
    buttons: u32,
    #[pyo3(get)]
    position: (f64, f64),
}

#[pymethods]
impl WindowInput {
    #[new]
    fn new() -> Self {
        Self::default()
    }
    fn pointer(&mut self, x: f64, y: f64, buttons: u32, modifiers: u32) {
        self.position = (x, y);
        self.buttons = buttons;
        self.modifiers = modifiers;
    }
    fn modal_pointer(&mut self, kind: &str, x: f64, y: f64, button: u32, modifiers: u32) -> u32 {
        let buttons = match kind {
            "press" => self.buttons | button,
            "release" => self.buttons & !button,
            _ => self.buttons,
        };
        self.pointer(x, y, buttons, modifiers);
        buttons
    }
    fn focus_lost(&mut self) {
        self.buttons = 0;
        self.modifiers = 0;
    }
}

/// Look up the owning host in the instance dictionary: a document can
/// delegate missing attributes to its presentation, including during setup.
fn on_host<'p>(owner: &Bound<'p, PyAny>) -> PyResult<Option<Bound<'p, PyAny>>> {
    let Some(dict) = owner.getattr_opt("__dict__")? else {
        return Ok(None);
    };
    let Some(host) = dict.cast_into::<pyo3::types::PyDict>()?.get_item("_host")? else {
        return Ok(None);
    };
    host.getattr_opt("input_state")
}
fn state<'p>(owner: &Bound<'p, PyAny>) -> PyResult<Option<Bound<'p, PyAny>>> {
    if let Some(state) = on_host(owner)? {
        return Ok(Some(state));
    }
    match owner.getattr_opt("view")? {
        Some(view) => on_host(&view),
        None => Ok(None),
    }
}

#[pyfunction]
fn modifiers(owner: &Bound<'_, PyAny>) -> PyResult<u32> {
    state(owner)?
        .filter(|v| !v.is_none())
        .map_or(Ok(0), |s| s.getattr("modifiers")?.extract())
}
#[pyfunction]
fn mouse_buttons(owner: &Bound<'_, PyAny>) -> PyResult<u32> {
    state(owner)?
        .filter(|v| !v.is_none())
        .map_or(Ok(0), |s| s.getattr("buttons")?.extract())
}
#[pyfunction]
fn cursor_position(owner: &Bound<'_, PyAny>) -> PyResult<crate::screen::PixelPoint> {
    let (x, y): (f64, f64) = state(owner)?
        .filter(|v| !v.is_none())
        .map_or(Ok((0., 0.)), |s| s.getattr("position")?.extract())?;
    Ok(glam::DVec2::new(x.trunc(), y.trunc()).into())
}
pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add_class::<WindowInput>()?;
    let input = PyModule::new(py, "built_core.window_input")?;
    input.add_function(wrap_pyfunction!(modifiers, &input)?)?;
    input.add_function(wrap_pyfunction!(mouse_buttons, &input)?)?;
    input.add_function(wrap_pyfunction!(cursor_position, &input)?)?;
    py.import("sys")?
        .getattr("modules")?
        .set_item(input.name()?, &input)?;
    m.add("window_input", input)
}
