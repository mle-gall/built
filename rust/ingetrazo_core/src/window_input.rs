//! Last input delivered to one winit window, including its nested modal loop.
use pyo3::prelude::*;

#[derive(Default)]
#[pyclass(module = "ingetrazo_core")]
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
