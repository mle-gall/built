//! A presentation surface without a window or a toolkit, for GPU tests and
//! probes: a `CAMetalLayer` the renderer draws into as into a window's
//! (``Renderer(headless_layer(w, h), w, h, 1., platform="layer")``). It
//! replaces the hidden Qt widgets the tests took a native handle from.
use pyo3::{exceptions::PyRuntimeError, prelude::*};

#[cfg(target_os = "macos")]
mod objc {
    use std::ffi::{c_char, c_void};
    #[link(name = "QuartzCore", kind = "framework")]
    extern "C" {}
    #[link(name = "objc")]
    extern "C" {
        pub fn objc_getClass(name: *const c_char) -> *mut c_void;
        pub fn sel_registerName(name: *const c_char) -> *mut c_void;
        pub fn objc_msgSend();
    }
    /// `[receiver selector]` returning an object.
    pub unsafe fn send(receiver: *mut c_void, selector: &std::ffi::CStr) -> *mut c_void {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        f(receiver, sel_registerName(selector.as_ptr()))
    }
}

/// A retained `CAMetalLayer` of ``width`` × ``height`` points, as the address
/// the renderer takes for ``platform="layer"``. It lives for the process.
#[pyfunction]
#[pyo3(signature = (width=64, height=64))]
pub fn headless_layer(width: u32, height: u32) -> PyResult<usize> {
    let _ = (width, height);
    #[cfg(target_os = "macos")]
    unsafe {
        let class = objc::objc_getClass(c"CAMetalLayer".as_ptr());
        if class.is_null() {
            return Err(PyRuntimeError::new_err("Core Animation is unavailable"));
        }
        let layer = objc::send(class, c"layer");
        if layer.is_null() {
            return Err(PyRuntimeError::new_err("no CAMetalLayer"));
        }
        objc::send(layer, c"retain");
        return Ok(layer as usize);
    }
    #[cfg(not(target_os = "macos"))]
    Err(PyRuntimeError::new_err("a headless layer needs macOS"))
}
