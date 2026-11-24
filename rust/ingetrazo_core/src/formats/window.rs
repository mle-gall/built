// SPDX-License-Identifier: GPL-3.0-or-later
//! Presentation handles belong to the window host. A captured frame owns its pixels.
use super::Obj;
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyBytes, PyDict, PyInt, PyType},
};
use std::sync::Arc;

#[pyfunction]
fn surface_window_handles<'p>(window: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = window.py();
    for owner in std::iter::once(window.clone()).chain(window.getattr_opt("_host")?) {
        if let (Some(id), Some(handles)) = (
            owner.getattr_opt("ns_view")?,
            owner.getattr_opt("surface_handles")?,
        ) {
            return Ok((id, handles).into_pyobject(py)?.into_any());
        }
    }
    // Some legacy widgets synthesize a numeric ID even in an offscreen process.
    // Only an ID from a real platform window can be passed to wgpu.
    let backend = std::env::var("QT_QPA_PLATFORM").unwrap_or_default();
    if !matches!(
        backend.as_str(),
        "" | "windows" | "cocoa" | "xcb" | "wayland"
    ) {
        return Err(PyValueError::new_err(
            "widget has no native presentation surface",
        ));
    }
    let id = py
        .get_type::<PyInt>()
        .call1((window.call_method0("winId")?,))?;
    let args = PyDict::new(py);
    #[cfg(target_os = "linux")]
    {
        // X11 window IDs can be addressed on another connection to the same server.
        // A Wayland token cannot: the host must supply its borrowed wl_surface.
        if backend == "wayland"
            || (backend.is_empty() && std::env::var_os("WAYLAND_DISPLAY").is_some())
        {
            return Err(PyValueError::new_err(
                "Wayland surfaces require the host's display and surface handles",
            ));
        }
        let display = x11_connection().map_err(PyValueError::new_err)?;
        args.set_item("display", display)?;
        args.set_item("platform", "xcb")?;
    }
    Ok((id, args).into_pyobject(py)?.into_any())
}

#[cfg(target_os = "linux")]
fn x11_connection() -> Result<usize, String> {
    use std::{
        ffi::{c_char, c_void},
        ptr,
        sync::OnceLock,
    };
    // The connection and library must outlive every presentation surface. XCB
    // serializes access from the event and render threads.
    static CONNECTION: OnceLock<(libloading::Library, usize)> = OnceLock::new();
    static OPEN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _lock = OPEN.lock().map_err(|e| e.to_string())?;
    if let Some((_, p)) = CONNECTION.get() {
        return Ok(*p);
    }
    unsafe {
        let lib = libloading::Library::new("libxcb.so.1").map_err(|e| e.to_string())?;
        let connect = lib
            .get::<unsafe extern "C" fn(*const c_char, *mut i32) -> *mut c_void>(b"xcb_connect\0")
            .map_err(|e| e.to_string())?;
        let status = lib
            .get::<unsafe extern "C" fn(*mut c_void) -> i32>(b"xcb_connection_has_error\0")
            .map_err(|e| e.to_string())?;
        let close = lib
            .get::<unsafe extern "C" fn(*mut c_void)>(b"xcb_disconnect\0")
            .map_err(|e| e.to_string())?;
        let p = connect(ptr::null(), ptr::null_mut());
        if p.is_null() {
            return Err("X11 display unavailable".into());
        }
        if status(p) != 0 {
            close(p);
            return Err("Could not connect to X11 display".into());
        }
        let _ = CONNECTION.set((lib, p as usize));
        Ok(p as usize)
    }
}

#[pyclass(frozen)]
struct CaptureColor([u8; 4]);
#[pymethods]
#[allow(non_snake_case)]
impl CaptureColor {
    fn red(&self) -> u8 {
        self.0[0]
    }
    fn green(&self) -> u8 {
        self.0[1]
    }
    fn blue(&self) -> u8 {
        self.0[2]
    }
    fn alpha(&self) -> u8 {
        self.0[3]
    }
    fn redF(&self) -> f64 {
        self.red() as f64 / 255.
    }
    fn greenF(&self) -> f64 {
        self.green() as f64 / 255.
    }
    fn blueF(&self) -> f64 {
        self.blue() as f64 / 255.
    }
    fn lightnessF(&self) -> f64 {
        let c = &self.0[..3];
        (f64::from(*c.iter().max().unwrap()) + f64::from(*c.iter().min().unwrap())) / 510.
    }
    fn name(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.red(), self.green(), self.blue())
    }
    fn getRgb(&self) -> (u8, u8, u8, u8) {
        self.0.into()
    }
}

#[pyclass(frozen)]
struct CapturedFrame {
    pixels: Arc<[u8]>,
    width: usize,
    height: usize,
    scale: f64,
    layout: u8,
}
#[pymethods]
#[allow(non_snake_case, non_upper_case_globals)]
impl CapturedFrame {
    #[classattr]
    const Format_RGB32: u8 = 4;
    #[classattr]
    const Format_ARGB32: u8 = 5;
    #[classattr]
    const Format_RGBA8888: u8 = 17;
    #[getter]
    fn Format<'p>(slf: &Bound<'p, Self>) -> Bound<'p, PyType> {
        slf.get_type()
    }
    fn convertToFormat(&self, format: &Obj<'_>) -> PyResult<Self> {
        let code = format
            .getattr_opt("value")?
            .unwrap_or_else(|| format.clone())
            .extract::<u8>()?;
        if !matches!(code, 4 | 5 | 17) {
            return Err(PyValueError::new_err("unsupported capture buffer layout"));
        }
        Ok(Self {
            pixels: self.pixels.clone(),
            width: self.width,
            height: self.height,
            scale: self.scale,
            layout: code,
        })
    }
    fn width(&self) -> usize {
        self.width
    }
    fn height(&self) -> usize {
        self.height
    }
    fn isNull(&self) -> bool {
        self.pixels.is_empty()
    }
    fn bytesPerLine(&self) -> usize {
        self.width * 4
    }
    fn sizeInBytes(&self) -> usize {
        self.pixels.len()
    }
    fn devicePixelRatio(&self) -> f64 {
        self.scale
    }
    fn pixelColor(&self, x: usize, y: usize) -> PyResult<CaptureColor> {
        if x >= self.width || y >= self.height {
            return Err(PyValueError::new_err("pixel outside frame"));
        }
        let i = (y * self.width + x) * 4;
        Ok(CaptureColor(self.pixels[i..i + 4].try_into().unwrap()))
    }
    fn rgba<'p>(&self, py: Python<'p>) -> Bound<'p, PyBytes> {
        PyBytes::new(py, &self.pixels)
    }
    fn constBits<'p>(&self, py: Python<'p>) -> Bound<'p, PyBytes> {
        if self.layout == Self::Format_RGBA8888 {
            return self.rgba(py);
        }
        let mut bytes = self.pixels.to_vec();
        for p in bytes.chunks_exact_mut(4) {
            let alpha = if self.layout == Self::Format_RGB32 {
                255
            } else {
                p[3]
            };
            let argb = u32::from_be_bytes([alpha, p[0], p[1], p[2]]);
            p.copy_from_slice(&argb.to_ne_bytes());
        }
        PyBytes::new(py, &bytes)
    }
}
#[pyfunction]
fn capture_document_frame(controller: &Obj<'_>) -> PyResult<CapturedFrame> {
    let display = controller.getattr("keys_stand_in")?;
    display.call_method1("set_scene", (controller.getattr("scene")?,))?;
    display.call_method0("_prepare_pick_frame")?;
    let renderer = display.getattr("renderer")?;
    renderer.call_method0("request_capture")?;
    display.call_method0("draw_frame")?;
    let pixels = renderer
        .call_method0("take_capture")?
        .extract::<Vec<u8>>()?;
    let scale = display
        .call_method0("devicePixelRatioF")?
        .extract::<f64>()?;
    let width =
        (display.call_method0("width")?.extract::<f64>()? * scale).round_ties_even() as usize;
    let stride = width
        .checked_mul(4)
        .filter(|n| *n > 0)
        .ok_or_else(|| PyValueError::new_err("empty capture"))?;
    if pixels.is_empty() || pixels.len() % stride != 0 {
        return Err(PyValueError::new_err("incomplete frame capture"));
    }
    Ok(CapturedFrame {
        height: pixels.len() / stride,
        width,
        pixels: pixels.into(),
        scale,
        layout: CapturedFrame::Format_RGBA8888,
    })
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    functions!(m;surface_window_handles,capture_document_frame);
    Ok(())
}
