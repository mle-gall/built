// SPDX-License-Identifier: GPL-3.0-or-later
//! Host attachment, daemon reads and device-event dispatch for 3D mice.
use super::modules::{class, classic, method, py_super};
use super::*;
use pyo3::exceptions::{PyBlockingIOError, PyOSError};
use pyo3::types::PyBytes;

fn native(py: Python) -> PyResult<bool> {
    py.import("views.ndof_input")?
        .getattr("NATIVE")?
        .is_truthy()
}
fn init_input<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let hub = a.get_item(0)?;
    py_super(&py.import("views.ndof_input")?, "NdofInput", &hub)?
        .call_method1("__init__", (a.get_item(1)?,))?;
    for key in ["_backend", "_last_t", "_moved_t"] {
        hub.setattr(key, py.None())?;
    }
    hub.setattr("_driver_checked", false)?;
    Ok(none(py))
}
fn shared<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let shell = native(py)?;
    let module = py.import("views.ndof_input")?;
    let mut hub = module.getattr("_shared")?;
    let mut window = a.get_item(0)?;
    if hub.is_none() {
        if !shell {
            window = classic(py)?.call_method0("application")?;
        }
        hub = module.call_method1("NdofInput", (&window,))?;
        module.setattr("_shared", &hub)?;
    }
    if shell && !window.is_none() {
        let host = window.getattr("_host")?;
        let old = hub.getattr("_host")?;
        if !old.is_none() && !old.is(&host) {
            host.getattr("timers")?
                .call_method1("transfer_context", (old.getattr("timers")?, &hub))?;
        }
        hub.setattr("_host", host)?;
    }
    if !hub.getattr("_driver_checked")?.is_truthy()? && (!shell || !hub.getattr("_host")?.is_none())
    {
        hub.setattr("_driver_checked", true)?;
        hub.call_method0("start")?;
    }
    Ok(hub)
}
fn spnav_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let device = a.get_item(0)?;
    device.setattr("owner", a.get_item(1)?)?;
    device.setattr("sock", py.None())?;
    device.setattr("notifier", py.None())?;
    device.setattr("_buf", PyBytes::new(py, &[]))?;
    Ok(none(py))
}
fn spnav_open<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let device = a.get_item(0)?;
    let module = py.import("views.ndof_input")?;
    let exists = module.getattr("os")?.getattr("path")?.getattr("exists")?;
    let mut path = None;
    for candidate in module.getattr("SPNAV_SOCKETS")?.try_iter()? {
        let candidate = candidate?;
        if exists.call1((&candidate,))?.is_truthy()? {
            path = Some(candidate);
            break;
        }
    }
    let Some(path) = path else {
        return false.into_bound_py_any(py);
    };
    let socket = py.import("socket")?;
    let sock = socket.call_method1(
        "socket",
        (socket.getattr("AF_UNIX")?, socket.getattr("SOCK_STREAM")?),
    )?;
    if let Err(error) = sock.call_method1("connect", (path,)) {
        if !error.is_instance_of::<PyOSError>(py) {
            return Err(error);
        }
        sock.call_method0("close")?;
        return false.into_bound_py_any(py);
    }
    sock.call_method1("setblocking", (false,))?;
    device.setattr("sock", &sock)?;
    let owner = device.getattr("owner")?;
    let read = device.getattr("_read")?;
    let notifier = if native(py)? {
        let timer = py.import("core.timers")?.call_method(
            "timer",
            (owner, read, 12),
            Some(&fields!(py;"repeat"=>true)),
        )?;
        timer.call_method0("start")?;
        timer
    } else {
        classic(py)?.call_method1("socket_notifier", (sock, owner, read))?
    };
    device.setattr("notifier", notifier)?;
    true.into_bound_py_any(py)
}
fn spnav_close<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let device = a.get_item(0)?;
    let notifier = device.getattr("notifier")?;
    if !notifier.is_none() {
        if native(py)? {
            notifier.call_method0("stop")?;
        } else {
            notifier.call_method1("setEnabled", (false,))?;
            notifier.call_method0("deleteLater")?;
        }
        device.setattr("notifier", py.None())?;
    }
    let sock = device.getattr("sock")?;
    if !sock.is_none() {
        sock.call_method0("close")?;
        device.setattr("sock", py.None())?;
    }
    Ok(none(py))
}
#[pyfunction(signature = (this,*_args), text_signature = "(self, *_args)")]
fn spnav_read(this: &Obj, _args: &Bound<PyTuple>) -> PyResult<()> {
    let py = this.py();
    let data = match this.getattr("sock")?.call_method1("recv", (4096,)) {
        Ok(data) => data,
        Err(error) if error.is_instance_of::<PyBlockingIOError>(py) => return Ok(()),
        Err(error) if error.is_instance_of::<PyOSError>(py) => PyBytes::new(py, &[]).into_any(),
        Err(error) => return Err(error),
    };
    let chunk = data.extract::<&[u8]>()?;
    if chunk.is_empty() {
        this.call_method0("close")?;
    } else {
        ndof_input::feed_spnav(this, chunk)?;
    }
    Ok(())
}
fn raw_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let device = a.get_item(0)?;
    device.setattr("owner", a.get_item(1)?)?;
    device.setattr("filter", py.None())?;
    device.setattr("state", PyDict::new(py))?;
    Ok(none(py))
}
fn raw_open<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    if !register_hid() {
        return false.into_bound_py_any(py);
    }
    let device = a.get_item(0)?;
    if native(py)? {
        py.import("app_shell.native")?
            .getattr("_shell")?
            .call_method1("set_raw_input_handler", (device.getattr("_on_msg")?,))?;
    } else {
        device.setattr(
            "filter",
            classic(py)?.call_method1("raw_filter", (&device,))?,
        )?;
    }
    true.into_bound_py_any(py)
}
fn raw_close<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let device = a.get_item(0)?;
    if native(py)? {
        py.import("app_shell.native")?
            .getattr("_shell")?
            .call_method1("set_raw_input_handler", (py.None(),))?;
    } else {
        let filter = device.getattr("filter")?;
        if !filter.is_none() {
            let app = classic(py)?.call_method0("application")?;
            if !app.is_none() {
                app.call_method1("removeNativeEventFilter", (filter,))?;
            }
            device.setattr("filter", py.None())?;
        }
    }
    Ok(none(py))
}
#[cfg(not(windows))]
fn register_hid() -> bool {
    false
}
#[cfg(not(windows))]
fn raw_message<'py>(a: &Args<'py>) -> Reply<'py> {
    Ok(none(a.py()))
}
#[cfg(windows)]
fn register_hid() -> bool {
    use windows::Win32::UI::Input::{RegisterRawInputDevices, RAWINPUTDEVICE};
    let device = RAWINPUTDEVICE {
        usUsagePage: 1,
        usUsage: 8,
        ..Default::default()
    };
    // The slice and its element size are the Win32 registration contract.
    unsafe {
        RegisterRawInputDevices(&[device], std::mem::size_of::<RAWINPUTDEVICE>() as u32).is_ok()
    }
}
#[cfg(windows)]
fn raw_message<'py>(a: &Args<'py>) -> Reply<'py> {
    use windows::Win32::UI::{
        Input::{GetRawInputData, HRAWINPUT, RAWINPUTHEADER, RID_INPUT, RIM_TYPEHID},
        WindowsAndMessaging::{MSG, WM_INPUT},
    };
    let py = a.py();
    let device = a.get_item(0)?;
    let ptr = a.get_item(1)?.extract::<usize>()? as *const MSG;
    // The shell and the Classic native-event filter lend their MSG for this callback.
    if ptr.is_null() {
        return Ok(none(py));
    }
    // Copy through Windows so a stale pointer received from a host drops the event.
    let mut msg = MSG::default();
    let bytes = std::mem::size_of::<MSG>();
    let mut read = 0;
    let copied = unsafe {
        windows::Win32::System::Diagnostics::Debug::ReadProcessMemory(
            windows::Win32::System::Threading::GetCurrentProcess(),
            ptr.cast(),
            (&mut msg as *mut MSG).cast(),
            bytes,
            Some(&mut read),
        )
    };
    if copied.is_err() || read != bytes {
        return Ok(none(py));
    }
    if msg.message != WM_INPUT {
        return Ok(none(py));
    }
    let handle = HRAWINPUT(msg.lParam.0 as _);
    let header = std::mem::size_of::<RAWINPUTHEADER>();
    let mut size = 0u32;
    unsafe { GetRawInputData(handle, RID_INPUT, None, &mut size, header as u32) };
    if (size as usize) < header + 8 {
        return Ok(none(py));
    }
    // Win32 requires a DWORD-aligned destination.
    let capacity = size;
    let mut words = vec![0u32; (size as usize).div_ceil(4)];
    let got = unsafe {
        GetRawInputData(
            handle,
            RID_INPUT,
            Some(words.as_mut_ptr().cast()),
            &mut size,
            header as u32,
        )
    };
    if got != capacity {
        return Ok(none(py));
    }
    let data = unsafe { std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), got as usize) };
    let word = |bytes: &[u8]| u32::from_le_bytes(bytes.try_into().unwrap());
    if word(&data[..4]) != RIM_TYPEHID.0 {
        return Ok(none(py));
    }
    let width = word(&data[header..header + 4]) as usize;
    let count = word(&data[header + 4..header + 8]) as usize;
    if width != 0 {
        for report in data[header + 8..].chunks_exact(width).take(count) {
            ndof_input::feed_hid(&device, report)?;
        }
    }
    Ok(none(py))
}
pub(super) fn install_devices(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    let builtins = py.import("builtins")?;
    module.add("SPNAV_SOCKETS", ("/var/run/spnav.sock", "/run/spnav.sock"))?;
    module.add(
        "_SPNAV_PACKET",
        module.getattr("struct")?.call_method1("Struct", ("=8i",))?,
    )?;
    for (key, n) in [
        ("WM_INPUT", 0xff),
        ("RID_INPUT", 0x10000003),
        ("RIM_TYPEHID", 2),
        ("HID_USAGE_PAGE_GENERIC", 1),
        ("HID_USAGE_MULTI_AXIS", 8),
    ] {
        module.add(key, n)?;
    }
    let keys = PyList::empty(py);
    for key in [
        "enabled",
        "sensitivity",
        "invert_pan",
        "invert_zoom",
        "invert_rotate",
        "invert_pan_x",
        "invert_pan_y",
        "invert_tilt",
        "invert_spin",
        "lock_rotation",
    ] {
        let value = if key == "sensitivity" {
            1.0.into_bound_py_any(py)?
        } else {
            (key == "enabled").into_bound_py_any(py)?
        };
        keys.append((key, value))?;
    }
    module.add("_KEYS", py.get_type::<PyTuple>().call1((keys,))?)?;
    module.add("_settings_cache", py.None())?;
    module.add("_shared", py.None())?;
    let signal = module.getattr("Signal")?;
    let object = builtins.getattr("object")?;
    let hub = class(
        module,
        "NdofInput",
        &[module.getattr("HostService")?],
        &fields!(py;
        "motion"=>signal.call1((&object,builtins.getattr("float")?))?,"button"=>signal.call1((builtins.getattr("int")?,builtins.getattr("bool")?))?,
        "raw"=>signal.call1((object,))?,"WHEEL_QUIET_S"=>0.4),
    )?;
    register_view_class(&hub)?;
    install(
        &hub,
        &module.dict(),
        &[(
            "__init__",
            Mode::Method,
            init_input,
            &["self", "parent=None"],
            "None",
            "",
        )],
    )?;
    register_view_functions(&module.dict())?;
    install(
        module.dict().as_any(),
        &module.dict(),
        &[(
            "shared_input",
            Mode::Function,
            shared,
            &["window=None"],
            "NdofInput",
            "",
        )],
    )?;
    let daemon = class(
        module,
        "SpnavBackend",
        &[],
        &fields!(py;"name"=>"spacenavd"),
    )?;
    install(
        &daemon,
        &module.dict(),
        &[
            (
                "__init__",
                Mode::Method,
                spnav_init,
                &["self", "owner:NdofInput"],
                "None",
                "",
            ),
            ("open", Mode::Method, spnav_open, &["self"], "bool", ""),
            ("close", Mode::Method, spnav_close, &["self"], "None", ""),
        ],
    )?;
    method(
        &daemon,
        "_read",
        wrap_pyfunction!(spnav_read, module)?.as_any(),
    )?;
    let raw = class(
        module,
        "RawInputBackend",
        &[],
        &fields!(py;"name"=>"rawinput"),
    )?;
    install(
        &raw,
        &module.dict(),
        &[
            (
                "__init__",
                Mode::Method,
                raw_init,
                &["self", "owner:NdofInput"],
                "None",
                "",
            ),
            ("open", Mode::Method, raw_open, &["self"], "bool", ""),
            ("close", Mode::Method, raw_close, &["self"], "None", ""),
            (
                "_on_msg",
                Mode::Method,
                raw_message,
                &["self", "msg_ptr:int"],
                "None",
                "",
            ),
        ],
    )
}
