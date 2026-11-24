// SPDX-License-Identifier: GPL-3.0-or-later
//! Input events as the Qt-era handlers read them, built from winit's
//! (`rust/built_app/src/keys.rs` already sends Qt's codes).
//!
//! The handlers in `views/` and `tools/` call `event.position()`,
//! `button()`, `buttons()`, `modifiers()`, `key()`, `text()`,
//! `angleDelta()`, `pixelDelta()`, `phase()`… These value objects answer
//! those calls with the types of `core.input_types` (Qt's own under PySide,
//! the native point classes and input codes in the `built` binary). No
//! QEvent is constructed: a QMouseEvent needs a QPointingDevice, which needs
//! a real platform.
use pyo3::{exceptions::PyKeyError, prelude::*};

fn types(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("core.input_types")
}

fn qt<'py>(py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyAny>> {
    types(py)?.getattr("Qt")?.getattr(name)
}

fn point_f(py: Python<'_>, (x, y): (f64, f64)) -> PyResult<Bound<'_, PyAny>> {
    types(py)?.getattr("QPointF")?.call1((x, y))
}

fn point(py: Python<'_>, (x, y): (i64, i64)) -> PyResult<Bound<'_, PyAny>> {
    types(py)?.getattr("QPoint")?.call1((x, y))
}

fn modifiers(py: Python<'_>, mask: u32) -> PyResult<Bound<'_, PyAny>> {
    qt(py, "KeyboardModifier")?.call1((mask,))
}

fn mouse_buttons(py: Python<'_>, mask: u32) -> PyResult<Bound<'_, PyAny>> {
    qt(py, "MouseButton")?.call1((mask,))
}

/// What every event answers: its type, accepted or not, where it is.
#[pyclass(module = "ingetrazo_render", subclass)]
pub struct ShellEvent {
    /// The `QEvent.Type` member's name.
    kind: &'static str,
    accepted: bool,
    pos: (f64, f64),
    global: Option<(f64, f64)>,
    mods: u32,
    held: u32,
}

impl ShellEvent {
    fn new(kind: &'static str, pos: (f64, f64), mods: u32) -> Self {
        Self { kind, accepted: true, pos, global: None, mods, held: 0 }
    }

    fn global_or_pos(&self) -> (f64, f64) {
        self.global.unwrap_or(self.pos)
    }
}

#[pymethods]
impl ShellEvent {
    fn r#type<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        types(py)?.getattr("QEvent")?.getattr(self.kind)
    }

    fn accept(&mut self) {
        self.accepted = true;
    }

    fn ignore(&mut self) {
        self.accepted = false;
    }

    #[allow(non_snake_case)]
    fn isAccepted(&self) -> bool {
        self.accepted
    }

    #[allow(non_snake_case)]
    fn setAccepted(&mut self, on: bool) {
        self.accepted = on;
    }

    /// The same event seen from a pane: local coordinates, global kept.
    fn set_local_position(&mut self, x: f64, y: f64) {
        self.global = Some(self.global_or_pos());
        self.pos = (x, y);
    }

    fn spontaneous(&self) -> bool {
        true
    }
}

/// `event.device()`: a mouse, a trackpad (TouchPad) or a touch screen.
#[pyclass(module = "ingetrazo_render")]
pub struct PointerDevice {
    kind: &'static str,
}

#[pymethods]
impl PointerDevice {
    fn r#type<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        types(py)?.getattr("DeviceType")?.getattr(self.kind)
    }

    #[allow(non_snake_case)]
    fn systemId(&self) -> i64 {
        0
    }

    fn name(&self) -> &'static str {
        if self.kind == "Mouse" { "mouse" } else { "trackpad" }
    }
}

fn device(py: Python<'_>, touch_screen: bool, mouse: bool) -> PyResult<Py<PointerDevice>> {
    let kind = if mouse { "Mouse" } else if touch_screen { "TouchScreen" } else { "TouchPad" };
    Py::new(py, PointerDevice { kind })
}

/// A press, release, move or double click.
#[pyclass(module = "ingetrazo_render", extends = ShellEvent, subclass)]
pub struct MouseEvent {
    button: u32,
    timestamp: i64,
}

fn mouse_kind(kind: &str) -> PyResult<&'static str> {
    Ok(match kind {
        "press" => "MouseButtonPress",
        "release" => "MouseButtonRelease",
        "move" => "MouseMove",
        "double" => "MouseButtonDblClick",
        other => return Err(PyKeyError::new_err(other.to_owned())),
    })
}

fn mouse_init(kind: &'static str, x: f64, y: f64, button: u32, held: u32, mods: u32, gx: Option<f64>, gy: Option<f64>, timestamp: i64) -> PyClassInitializer<MouseEvent> {
    let mut base = ShellEvent::new(kind, (x, y), mods);
    base.global = Some((gx.unwrap_or(x), gy.unwrap_or(y)));
    base.held = held;
    PyClassInitializer::from(base).add_subclass(MouseEvent { button, timestamp })
}

#[pymethods]
impl MouseEvent {
    #[new]
    #[pyo3(signature = (kind, x, y, button=0, held=0, modifiers=0, gx=None, gy=None, timestamp=0))]
    #[allow(clippy::too_many_arguments)]
    fn new(kind: &str, x: f64, y: f64, button: u32, held: u32, modifiers: u32, gx: Option<f64>, gy: Option<f64>, timestamp: i64) -> PyResult<PyClassInitializer<Self>> {
        Ok(mouse_init(mouse_kind(kind)?, x, y, button, held, modifiers, gx, gy, timestamp))
    }

    fn timestamp(&self) -> i64 {
        self.timestamp
    }

    fn position<'py>(slf: PyRef<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        point_f(slf.py(), slf.as_super().pos)
    }

    fn pos<'py>(slf: PyRef<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        point_f(slf.py(), slf.as_super().pos)?.call_method0("toPoint")
    }

    fn x(slf: PyRef<'_, Self>) -> i64 {
        slf.as_super().pos.0.trunc() as i64
    }

    fn y(slf: PyRef<'_, Self>) -> i64 {
        slf.as_super().pos.1.trunc() as i64
    }

    #[allow(non_snake_case)]
    fn scenePosition<'py>(slf: PyRef<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        point_f(slf.py(), slf.as_super().pos)
    }

    #[allow(non_snake_case)]
    fn globalPosition<'py>(slf: PyRef<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        point_f(slf.py(), slf.as_super().global_or_pos())
    }

    #[allow(non_snake_case)]
    fn globalPos<'py>(slf: PyRef<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        point_f(slf.py(), slf.as_super().global_or_pos())?.call_method0("toPoint")
    }

    fn button<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        mouse_buttons(py, self.button)
    }

    fn buttons<'py>(slf: PyRef<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        mouse_buttons(slf.py(), slf.as_super().held)
    }

    fn modifiers<'py>(slf: PyRef<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        modifiers(slf.py(), slf.as_super().mods)
    }

    fn source<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        qt(py, "MouseEventNotSynthesized")
    }

    fn device(&self) -> Option<Py<PyAny>> {
        None
    }
}

/// A right click, as Qt's QContextMenuEvent (reason: Mouse).
#[pyclass(module = "ingetrazo_render", extends = MouseEvent)]
pub struct ContextMenuEvent;

#[pymethods]
impl ContextMenuEvent {
    #[new]
    #[pyo3(signature = (x, y, modifiers=0))]
    fn new(x: f64, y: f64, modifiers: u32) -> PyClassInitializer<Self> {
        const RIGHT: u32 = 2;
        mouse_init("ContextMenu", x, y, RIGHT, RIGHT, modifiers, None, None, 0).add_subclass(ContextMenuEvent)
    }

    fn reason(&self) -> i64 {
        1
    }
}

/// A key press or release (Qt's key codes).
#[pyclass(module = "ingetrazo_render", extends = ShellEvent, subclass)]
pub struct KeyEvent {
    key: i64,
    text: String,
    repeat: bool,
}

fn key_init(kind: &'static str, key: i64, mods: u32, text: Option<String>, repeat: bool, accepted: bool) -> PyClassInitializer<KeyEvent> {
    let mut base = ShellEvent::new(kind, (0.0, 0.0), mods);
    base.accepted = accepted;
    PyClassInitializer::from(base).add_subclass(KeyEvent { key, text: text.unwrap_or_default(), repeat })
}

#[pymethods]
impl KeyEvent {
    #[new]
    #[pyo3(signature = (pressed, key, modifiers=0, text=None, repeat=false))]
    fn new(pressed: bool, key: i64, modifiers: u32, text: Option<String>, repeat: bool) -> PyClassInitializer<Self> {
        key_init(if pressed { "KeyPress" } else { "KeyRelease" }, key, modifiers, text, repeat, true)
    }

    fn key(&self) -> i64 {
        self.key
    }

    fn modifiers<'py>(slf: PyRef<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        modifiers(slf.py(), slf.as_super().mods)
    }

    fn text(&self) -> String {
        self.text.clone()
    }

    #[allow(non_snake_case)]
    fn isAutoRepeat(&self) -> bool {
        self.repeat
    }

    fn count(&self) -> i64 {
        1
    }

    #[allow(non_snake_case)]
    fn nativeModifiers(&self) -> i64 {
        0
    }

    #[allow(non_snake_case)]
    fn nativeScanCode(&self) -> i64 {
        0
    }

    /// The modifiers with the key, when the key is one Qt names.
    #[allow(non_snake_case)]
    fn keyCombination<'py>(slf: PyRef<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        let mods = modifiers(py, slf.as_super().mods)?;
        let keys = qt(py, "Key")?;
        if keys.getattr("_value2member_map_")?.contains(slf.key)? {
            mods.bitor(keys.call1((slf.key,))?)
        } else {
            Ok(mods)
        }
    }

    fn matches(&self, _seq: &Bound<'_, PyAny>) -> bool {
        false
    }
}

/// The key a shortcut may claim before it is pressed: not accepted until
/// someone claims it.
#[pyclass(module = "ingetrazo_render", extends = KeyEvent)]
pub struct ShortcutOverride;

#[pymethods]
impl ShortcutOverride {
    #[new]
    #[pyo3(signature = (key, modifiers=0, text=None, repeat=false))]
    fn new(key: i64, modifiers: u32, text: Option<String>, repeat: bool) -> PyClassInitializer<Self> {
        key_init("ShortcutOverride", key, modifiers, text, repeat, false).add_subclass(ShortcutOverride)
    }
}

/// A wheel notch (angle delta) or a trackpad slide (pixel delta, phase).
#[pyclass(module = "ingetrazo_render", extends = ShellEvent)]
pub struct WheelEvent {
    pixel: (i64, i64),
    angle: (i64, i64),
    phase: &'static str,
    inverted: bool,
    device: Py<PointerDevice>,
}

#[pymethods]
impl WheelEvent {
    #[new]
    #[pyo3(signature = (x, y, pixel_dx, pixel_dy, angle_dx, angle_dy, phase="none", modifiers=0, held=0, inverted=false, touch_screen=false))]
    #[allow(clippy::too_many_arguments)]
    fn new(py: Python<'_>, x: f64, y: f64, pixel_dx: f64, pixel_dy: f64, angle_dx: f64, angle_dy: f64, phase: &str, modifiers: u32, held: u32, inverted: bool, touch_screen: bool) -> PyResult<PyClassInitializer<Self>> {
        // Python's round(): halves to even.
        let r = |v: f64| v.round_ties_even() as i64;
        let pixel = (r(pixel_dx), r(pixel_dy));
        // winit's TouchPhase → Qt's ScrollPhase.
        let phase = match phase {
            "start" => "ScrollBegin",
            "move" => "ScrollUpdate",
            "end" => "ScrollEnd",
            "momentum" => "ScrollMomentum",
            _ => "NoScrollPhase",
        };
        let mut base = ShellEvent::new("Wheel", (x, y), modifiers);
        base.held = held;
        // A notch (no pixel delta) is a mouse wheel; a pixel delta a trackpad.
        let device = device(py, touch_screen, pixel == (0, 0))?;
        Ok(PyClassInitializer::from(base).add_subclass(WheelEvent { pixel, angle: (r(angle_dx), r(angle_dy)), phase, inverted, device }))
    }

    fn position<'py>(slf: PyRef<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        point_f(slf.py(), slf.as_super().pos)
    }

    #[allow(non_snake_case)]
    fn globalPosition<'py>(slf: PyRef<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        point_f(slf.py(), slf.as_super().global_or_pos())
    }

    #[allow(non_snake_case)]
    fn pixelDelta<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        point(py, self.pixel)
    }

    #[allow(non_snake_case)]
    fn angleDelta<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        point(py, self.angle)
    }

    fn phase<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        qt(py, self.phase)
    }

    fn modifiers<'py>(slf: PyRef<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        modifiers(slf.py(), slf.as_super().mods)
    }

    fn buttons<'py>(slf: PyRef<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        mouse_buttons(slf.py(), slf.as_super().held)
    }

    fn inverted(&self) -> bool {
        self.inverted
    }

    fn source<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        qt(py, "MouseEventNotSynthesized")
    }

    fn device(&self, py: Python<'_>) -> Py<PointerDevice> {
        self.device.clone_ref(py)
    }

    #[allow(non_snake_case)]
    fn pointingDevice(&self, py: Python<'_>) -> Py<PointerDevice> {
        self.device.clone_ref(py)
    }
}

/// A trackpad gesture: zoom, rotate, smart zoom, pan, swipe, begin, end.
#[pyclass(module = "ingetrazo_render", extends = ShellEvent)]
pub struct NativeGestureEvent {
    kind: &'static str,
    value: f64,
    delta: (f64, f64),
    device: Py<PointerDevice>,
}

#[pymethods]
impl NativeGestureEvent {
    #[new]
    #[pyo3(signature = (kind, value, x, y, delta_x=0.0, delta_y=0.0, touch_screen=false, modifiers=0))]
    #[allow(clippy::too_many_arguments)]
    fn new(py: Python<'_>, kind: &str, value: f64, x: f64, y: f64, delta_x: f64, delta_y: f64, touch_screen: bool, modifiers: u32) -> PyResult<PyClassInitializer<Self>> {
        let kind = match kind {
            "zoom" => "ZoomNativeGesture",
            "rotate" => "RotateNativeGesture",
            "smart_zoom" => "SmartZoomNativeGesture",
            "pan" => "PanNativeGesture",
            "begin" => "BeginNativeGesture",
            "end" => "EndNativeGesture",
            "swipe" => "SwipeNativeGesture",
            other => return Err(PyKeyError::new_err(other.to_owned())),
        };
        let device = device(py, touch_screen, false)?;
        Ok(PyClassInitializer::from(ShellEvent::new("NativeGesture", (x, y), modifiers))
            .add_subclass(NativeGestureEvent { kind, value, delta: (delta_x, delta_y), device }))
    }

    #[allow(non_snake_case)]
    fn gestureType<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        qt(py, self.kind)
    }

    fn value(&self) -> f64 {
        self.value
    }

    fn position<'py>(slf: PyRef<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        point_f(slf.py(), slf.as_super().pos)
    }

    #[allow(non_snake_case)]
    fn globalPosition<'py>(slf: PyRef<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        point_f(slf.py(), slf.as_super().global_or_pos())
    }

    fn delta<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        point_f(py, self.delta)
    }

    fn device(&self, py: Python<'_>) -> Py<PointerDevice> {
        self.device.clone_ref(py)
    }

    #[allow(non_snake_case)]
    fn pointingDevice(&self, py: Python<'_>) -> Py<PointerDevice> {
        self.device.clone_ref(py)
    }

    #[allow(non_snake_case)]
    fn fingerCount(&self) -> i64 {
        2
    }

    fn modifiers<'py>(slf: PyRef<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        modifiers(slf.py(), slf.as_super().mods)
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<ShellEvent>()?;
    m.add_class::<PointerDevice>()?;
    m.add_class::<MouseEvent>()?;
    m.add_class::<ContextMenuEvent>()?;
    m.add_class::<KeyEvent>()?;
    m.add_class::<ShortcutOverride>()?;
    m.add_class::<WheelEvent>()?;
    m.add_class::<NativeGestureEvent>()?;
    Ok(())
}
