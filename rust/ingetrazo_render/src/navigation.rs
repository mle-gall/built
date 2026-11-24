//! Shared gesture recognizer for the winit shell and the compatibility viewport.
use pyo3::prelude::*;
use std::time::{Duration, Instant};

type Obj<'py> = Bound<'py, PyAny>;

pub fn setting(py: Python<'_>, key: &str, default: &str) -> PyResult<String> {
    py.import("core.settings")?
        .getattr("settings")?
        .call0()?
        .call_method1("value", (key, default))?
        .str()?
        .extract()
}

#[pyfunction]
pub(super) fn load_slide_mode(py: Python<'_>) -> PyResult<String> {
    Ok(match setting(py, "nav/trackpad_slide", "zoom")?
        .to_ascii_lowercase()
        .as_str()
    {
        "pan" => "pan",
        "orbit" => "orbit",
        _ => "zoom",
    }
    .into())
}

fn attr<'py>(view: &Obj<'py>, name: &str) -> Option<Obj<'py>> {
    view.getattr(name)
        .ok()
        .filter(|v| !v.is_none())
        .or_else(|| view.getattr("source_viewport").ok()?.getattr(name).ok())
}

fn enum_name(ev: &Obj, method: &str) -> PyResult<String> {
    ev.call_method0(method)?.getattr("name")?.extract()
}

#[pyfunction]
fn is_finger_device(ev: &Obj) -> PyResult<bool> {
    Ok(matches!(
        enum_name(&ev.call_method0("device")?, "type")?.as_str(),
        "TouchPad" | "TouchScreen"
    ))
}

/// Zoom per pixel of trackpad or touchscreen slide, against a wheel notch's
/// 120 angle units: 20 px of travel per 10% step.
pub(super) const TRACKPAD_ZOOM_GAIN: f64 = 3.0;

/// Cocoa's natural-scroll flag reverses a finger device's zoom direction.
#[pyfunction]
#[pyo3(signature = (event, invert=false))]
pub(super) fn zoom_steps(event: &Obj, invert: bool) -> PyResult<f64> {
    let mut steps = event.call_method0("angleDelta")?.call_method0("y")?.extract::<f64>()? / 120.;
    let finger = is_finger_device(event)?;
    if finger {
        // A slide hands over 2 angle units per pixel: 60 px per 10% step
        // made a whole trackpad slide barely zoom (owner, 2026-11-24, ride
        // file). Mouse wheel notches keep their 120 units per step.
        steps *= TRACKPAD_ZOOM_GAIN;
    }
    if event.py().import("sys")?.getattr("platform")?.eq("darwin")?
        && finger && event.call_method0("inverted")?.is_truthy()? {
        steps = -steps;
    }
    Ok(if invert { -steps } else { steps })
}

#[pyfunction]
fn is_slide(ev: &Obj) -> PyResult<bool> {
    Ok(enum_name(ev, "phase")? != "NoScrollPhase" || is_finger_device(ev)?)
}

#[pyfunction]
fn is_touch_screen(ev: &Obj) -> PyResult<bool> {
    // The shell tags verified Cocoa emulation and raw contacts as TouchScreen.
    // Qt's primary pointer ID alone cannot establish where a scroll originated.
    Ok(enum_name(&ev.call_method0("device")?, "type")? == "TouchScreen")
}

fn xy(p: &Obj) -> PyResult<(f64, f64)> {
    Ok((
        p.call_method0("x")?.extract()?,
        p.call_method0("y")?.extract()?,
    ))
}
#[pyfunction]
fn slide_pixels(ev: &Obj) -> PyResult<(f64, f64)> {
    let (mut x, mut y) = xy(&ev.call_method0("pixelDelta")?)?;
    if x == 0. && y == 0. {
        (x, y) = xy(&ev.call_method0("angleDelta")?)?;
        x /= 8.;
        y /= 8.;
    }
    Ok(if ev.call_method0("inverted")?.is_truthy()? {
        (x, y)
    } else {
        (-x, -y)
    })
}

#[pyfunction]
fn slide_mode<'a>(ev: &Obj, default: &'a str) -> PyResult<&'a str> {
    let mods: u32 = ev.call_method0("modifiers")?.getattr("value")?.extract()?;
    Ok(if mods & 0x02000000 != 0 {
        "pan"
    } else if mods & 0x04000000 != 0 {
        "zoom"
    } else if matches!(default, "orbit" | "pan" | "select") {
        default
    } else {
        "zoom"
    })
}

#[pyfunction]
fn pinch_steps(value: f64) -> Option<f64> {
    (value.is_finite() && value > -1.).then(|| value.ln_1p() / -0.9f64.ln())
}

#[derive(Clone, Copy, PartialEq)]
enum Motion {
    Slide,
    Pinch,
}

#[derive(Default)]
#[pyclass(unsendable)]
struct Gesture {
    last: Option<Instant>,
    lock: Option<Motion>,
    pinch: f64,
    pinch_travel: f64,
    drag: (f64, f64),
    travel: f64,
    depth: Option<f64>,
    #[pyo3(get)]
    pivot: Option<Py<PyAny>>,
}
impl Gesture {
    fn begin(&mut self, restart: bool) {
        if restart
            || self
                .last
                .is_some_and(|t| t.elapsed() > Duration::from_millis(350))
        {
            *self = Self::default();
        }
        self.last = Some(Instant::now());
    }
}

#[pyfunction]
fn gesture(view: &Obj) -> PyResult<Py<Gesture>> {
    if let Ok(g) = view.getattr("_trackpad_gesture") {
        return Ok(g.extract()?);
    }
    let g = Py::new(view.py(), Gesture::default())?;
    view.setattr("_trackpad_gesture", &g)?;
    Ok(g)
}

fn drag(view: &Obj, g: &mut Gesture, pos: &Obj, mode: &str, dx: f64, mut dy: f64) -> PyResult<()> {
    let cam = view.getattr("camera")?;
    let height = view.call_method0("height")?;
    if g.pivot.is_none() {
        let pivot = view.call_method1("_orbit_pivot_at", xy(pos)?)?;
        if !pivot.is_none() {
            let delta = pivot.call_method1("__sub__", (cam.call_method0("eye")?,))?;
            g.depth = Some(
                delta
                    .call_method1("dotProduct", (&delta, cam.call_method0("forward")?))?
                    .extract()?,
            );
            g.pivot = Some(pivot.unbind());
        }
    }
    if mode == "pan" {
        cam.call_method1("pan", (dx, dy, height, g.depth))?;
    } else {
        if attr(view, "_invert_orbit_y").is_some_and(|v| v.is_truthy().unwrap_or(false)) {
            dy = -dy;
        }
        if let Some(p) = &g.pivot {
            cam.call_method1("orbit_about", (p, dx, dy, height))?;
        } else {
            cam.call_method1("orbit", (dx, dy, height))?;
        }
    }
    Ok(())
}

#[pyfunction]
pub(super) fn slide(view: &Obj, ev: &Obj) -> PyResult<bool> {
    if !is_slide(ev)? {
        return Ok(false);
    }
    let state = gesture(view)?;
    let mut g = state.borrow_mut(view.py());
    let phase = enum_name(ev, "phase")?;
    g.begin(phase == "ScrollBegin");
    if phase == "ScrollMomentum"
        && matches!(
            setting(view.py(), "nav/trackpad_inertia", "1")?.as_str(),
            "0" | "false"
        )
    {
        return Ok(true);
    }
    if phase == "ScrollBegin" || g.lock == Some(Motion::Pinch) {
        return Ok(true);
    }
    let mode: String = if is_touch_screen(ev)? {
        attr(view, "_touch_slide")
            .and_then(|v| v.extract().ok())
            .map_or_else(|| setting(view.py(), "nav/touch_slide", "orbit"), Ok)?
    } else {
        attr(view, "_trackpad_slide")
            .and_then(|v| v.extract().ok())
            .map_or_else(|| load_slide_mode(view.py()), Ok)?
    };
    let mode = slide_mode(ev, &mode)?;
    if mode == "zoom" {
        return Ok(false);
    }
    let (mut dx, mut dy) = slide_pixels(ev)?;
    if dx == 0. && dy == 0. {
        return Ok(true);
    }
    if g.lock.is_none() {
        g.drag.0 += dx;
        g.drag.1 += dy;
        g.travel += dx.hypot(dy);
        if g.travel < 6. {
            return Ok(true);
        }
        (dx, dy) = g.drag;
        g.lock = Some(Motion::Slide);
    }
    if mode != "select" {
        drag(view, &mut g, &ev.call_method0("position")?, mode, dx, dy)?;
    }
    ev.call_method0("accept")?;
    Ok(true)
}

#[pyfunction]
fn native_gesture(view: &Obj, ev: &Obj) -> PyResult<bool> {
    let state = gesture(view)?;
    let mut g = state.borrow_mut(view.py());
    let kind = enum_name(ev, "gestureType")?;
    g.begin(kind == "BeginNativeGesture");
    match kind.as_str() {
        "BeginNativeGesture" => (),
        "EndNativeGesture" => *g = Gesture::default(),
        "ZoomNativeGesture" => {
            let Some(mut steps) = pinch_steps(ev.call_method0("value")?.extract()?) else {
                return Ok(true);
            };
            if g.lock == Some(Motion::Slide) {
                return Ok(true);
            }
            if g.lock.is_none() {
                g.pinch += steps;
                g.pinch_travel += steps.abs();
                if g.pinch_travel < pinch_steps(0.03).unwrap() {
                    return Ok(true);
                }
                g.lock = Some(Motion::Pinch);
                steps = g.pinch;
            }
            let sensitivity = setting(view.py(), "nav/pinch_sensitivity", "100")?
                .parse::<f64>()
                .unwrap_or(100.)
                .clamp(10., 300.)
                / 100.;
            view.call_method1(
                "_zoom_at",
                (steps * sensitivity, ev.call_method0("position")?),
            )?;
            let serial = view
                .getattr("_input_serial")
                .and_then(|v| v.extract::<u64>())
                .unwrap_or(0);
            view.setattr("_input_serial", serial + 1)?;
            view.setattr(
                "_input_t",
                view.py().import("time")?.call_method0("monotonic")?,
            )?;
        }
        "RotateNativeGesture" => {
            let angle = ev.call_method0("value")?.extract::<f64>()?;
            if angle.is_finite() {
                drag(
                    view,
                    &mut g,
                    &ev.call_method0("position")?,
                    "orbit",
                    angle,
                    0.,
                )?;
            }
        }
        "SmartZoomNativeGesture" => {
            if let Ok(host) = view.getattr("_host") {
                host.getattr("window")?.call_method0("_on_smart_zoom")?;
            } else if let Some(signal) = attr(view, "smartZoomRequested") {
                signal.call_method0("emit")?;
            }
        }
        "PanNativeGesture" | "SwipeNativeGesture" => (),
        _ => return Ok(false),
    }
    ev.call_method0("accept")?;
    view.call_method0("update")?;
    Ok(true)
}

pub fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new(parent.py(), "trackpad_nav")?;
    macro_rules! export { ($($name:ident),*) => { $(m.add_function(wrap_pyfunction!($name,&m)?)?;)* }; }
    export!(
        gesture,
        slide,
        native_gesture,
        is_touch_screen,
        is_finger_device,
        is_slide,
        slide_mode,
        slide_pixels,
        load_slide_mode,
        pinch_steps,
        zoom_steps
    );
    m.add("TRACKPAD_ZOOM_GAIN", TRACKPAD_ZOOM_GAIN)?;
    parent.add_submodule(&m)
}
