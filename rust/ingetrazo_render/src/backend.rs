//! Instance policy and host-owned presentation surfaces.
use pyo3::{exceptions::PyValueError, PyResult};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

pub fn instance() -> wgpu::Instance {
    wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY.with_env(),
        ..Default::default()
    })
}

/// The caller keeps the native window and its display alive until the renderer drops.
pub fn surface(
    instance: &wgpu::Instance,
    window: usize,
    display: usize,
    platform: &str,
) -> PyResult<wgpu::Surface<'static>> {
    if window == 0 {
        return Err(PyValueError::new_err("null native window"));
    }
    // A bare Core Animation layer (`headless_layer`): GPU tests and probes
    // without a window or a toolkit.
    #[cfg(target_os = "macos")]
    if platform == "layer" {
        return unsafe { instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(window as *mut _)) }
            .map_err(|e| PyValueError::new_err(format!("GPU surface unavailable: {e}")));
    }
    let (display_handle, window_handle) = handles(window, display, platform)?;
    unsafe {
        instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: display_handle,
            raw_window_handle: window_handle,
        })
    }
    .map_err(|e| PyValueError::new_err(format!("GPU surface unavailable: {e}")))
}

#[cfg(target_os = "macos")]
fn handles(window: usize, _: usize, _: &str) -> PyResult<(RawDisplayHandle, RawWindowHandle)> {
    use raw_window_handle::{AppKitDisplayHandle, AppKitWindowHandle};
    Ok((
        AppKitDisplayHandle::new().into(),
        AppKitWindowHandle::new(std::ptr::NonNull::new(window as *mut _).unwrap()).into(),
    ))
}

#[cfg(target_os = "windows")]
fn handles(window: usize, _: usize, _: &str) -> PyResult<(RawDisplayHandle, RawWindowHandle)> {
    use raw_window_handle::{Win32WindowHandle, WindowsDisplayHandle};
    let hwnd = std::num::NonZeroIsize::new(window as isize).unwrap();
    #[link(name = "user32")]
    extern "system" {
        #[cfg_attr(target_pointer_width = "64", link_name = "GetWindowLongPtrW")]
        #[cfg_attr(target_pointer_width = "32", link_name = "GetWindowLongW")]
        fn get_window_long(hwnd: isize, index: i32) -> isize;
    }
    // Vulkan surface creation needs the HINSTANCE that owns this HWND.
    let mut handle = Win32WindowHandle::new(hwnd);
    handle.hinstance = std::num::NonZeroIsize::new(unsafe { get_window_long(hwnd.get(), -6) });
    Ok((WindowsDisplayHandle::new().into(), handle.into()))
}

#[cfg(target_os = "linux")]
fn handles(
    window: usize,
    display: usize,
    platform: &str,
) -> PyResult<(RawDisplayHandle, RawWindowHandle)> {
    use raw_window_handle::{
        WaylandDisplayHandle, WaylandWindowHandle, XcbDisplayHandle, XcbWindowHandle,
        XlibDisplayHandle, XlibWindowHandle,
    };
    use std::{num::NonZeroU32, ptr::NonNull};
    let display = NonNull::new(display as *mut _)
        .ok_or_else(|| PyValueError::new_err("missing native display connection"))?;
    match platform {
        "wayland" => Ok((
            WaylandDisplayHandle::new(display).into(),
            WaylandWindowHandle::new(NonNull::new(window as *mut _).unwrap()).into(),
        )),
        "xlib" => Ok((
            XlibDisplayHandle::new(Some(display), 0).into(),
            XlibWindowHandle::new(window as _).into(),
        )),
        "xcb" => {
            let xid = u32::try_from(window)
                .ok()
                .and_then(NonZeroU32::new)
                .ok_or_else(|| PyValueError::new_err("invalid X11 window id"))?;
            Ok((
                XcbDisplayHandle::new(Some(display), 0).into(),
                XcbWindowHandle::new(xid).into(),
            ))
        }
        _ => Err(PyValueError::new_err("unknown native display protocol")),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn handles(_: usize, _: usize, _: &str) -> PyResult<(RawDisplayHandle, RawWindowHandle)> {
    Err(PyValueError::new_err("unsupported native window platform"))
}
