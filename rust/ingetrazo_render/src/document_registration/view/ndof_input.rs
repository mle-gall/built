// SPDX-License-Identifier: GPL-3.0-or-later
//! Host bindings for ndof_input.
use super::*;
fn ndof_input_backend_name<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    optional(&tool.getattr("_backend")?, "name")
}

fn ndof_input_start<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let this = a.get_item(0)?;
    if !this.getattr("_backend")?.is_none() {
        return true.into_bound_py_any(py);
    }
    let module = py.import("views.ndof_input")?;
    for class in module
        .call_method1(
            "_backends_for",
            (module.getattr("sys")?.getattr("platform")?,),
        )?
        .try_iter()?
    {
        let result = class?
            .call1((&this,))
            .and_then(|backend| Ok((backend.call_method0("open")?.is_truthy()?, backend)));
        match result {
            Ok((true, backend)) => {
                this.setattr("_backend", backend)?;
                return true.into_bound_py_any(py);
            }
            Ok((false, _)) => {}
            Err(err) if err.is_instance_of::<pyo3::exceptions::PyException>(py) => {}
            Err(err) => return Err(err),
        }
    }
    false.into_bound_py_any(py)
}

fn ndof_input_stop<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if !tool.getattr("_backend")?.is_none() {
        let action = (|| -> Reply<'py> {
            tool.getattr("_backend")?.call_method0("close")?;
            Ok(none(py))
        })();
        tool.setattr("_backend", none(py))?;
        action?;
    }
    tool.setattr("_last_t", none(py))?;
    Ok(none(py))
}

fn ndof_input_emit_motion<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let this = a.get_item(0)?;
    let sample = a.get_item(1)?;
    let requested = a.get_item(2)?;
    let module = py.import("views.ndof_input")?;
    let now: f64 = module
        .getattr("time")?
        .call_method0("monotonic")?
        .extract()?;
    let dt = if requested.is_none() {
        this.getattr("_last_t")?
            .extract::<Option<f64>>()?
            .map_or(1. / 60., |last| now - last)
    } else {
        py.get_type::<pyo3::types::PyFloat>()
            .call1((requested,))?
            .extract()?
    };
    this.setattr(
        "_last_t",
        (!sample.call_method0("is_idle")?.is_truthy()?).then_some(now),
    )?;
    let deadzone = module
        .call_method0("current_settings")?
        .getattr("deadzone")?
        .extract::<f64>()?;
    for axis in ["right", "up", "forward", "tilt", "spin"] {
        if sample.getattr(axis)?.extract::<f64>()?.abs() > deadzone {
            this.setattr("_moved_t", now)?;
            break;
        }
    }
    this.getattr("motion")?.call_method1("emit", (sample, dt))?;
    Ok(none(py))
}

fn ndof_input_emit_raw<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let mut v: Obj<'py>;
    let values = a.get_item(1)?;
    tool.getattr("raw")?.call_method1(
        "emit",
        (py.get_type::<pyo3::types::PyTuple>().into_any().call1(({
            let collected = PyList::empty(py);
            for item in values.try_iter()? {
                v = item?;
                collected.append(
                    py.get_type::<pyo3::types::PyInt>()
                        .into_any()
                        .call1((&v,))?,
                )?;
            }
            collected.into_any()
        },))?,),
    )?;
    Ok(none(py))
}

fn ndof_input_in_use<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.ndof_input")?;
    let tool = a.get_item(0)?;
    Ok({
        let value = (!tool.getattr("_moved_t")?.is_none()).into_bound_py_any(py)?;
        if !value.is_truthy()? {
            value
        } else {
            (module
                .getattr("time")?
                .call_method0("monotonic")?
                .sub(tool.getattr("_moved_t")?)?
                .lt(tool.getattr("WHEEL_QUIET_S")?)?)
            .into_bound_py_any(py)?
        }
    })
}

fn ndof_input_emit_button<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let down = a.get_item(2)?;
    let number = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.getattr("button")?.call_method1(
        "emit",
        (
            py.get_type::<pyo3::types::PyInt>()
                .into_any()
                .call1((&number,))?,
            (down.is_truthy()?).into_bound_py_any(py)?,
        ),
    )?;
    Ok(none(py))
}

#[rustfmt::skip]
pub(super) const NDOF_INPUT: &[Method] = &[
    ("backend_name", Mode::Property, ndof_input_backend_name, &["self"], "str | None", ""),
    ("start", Mode::Method, ndof_input_start, &["self"], "bool", "Connect to the device's driver. False when there is none — the\nnormal case on a machine without a 3D mouse, and silent."),
    ("stop", Mode::Method, ndof_input_stop, &["self"], "None", ""),
    ("_emit_motion", Mode::Method, ndof_input_emit_motion, &["self", "sample:NdofSample", "dt:float | None=None"], "None", ""),
    ("_emit_raw", Mode::Method, ndof_input_emit_raw, &["self", "values"], "None", ""),
    ("in_use", Mode::Method, ndof_input_in_use, &["self"], "bool", "True while the cap is off centre, and a moment after."),
    ("_emit_button", Mode::Method, ndof_input_emit_button, &["self", "number:int", "down:bool"], "None", ""),
];

fn backends_for<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.ndof_input")?;
    let platform = a.get_item(0)?;
    if platform
        .call_method1("startswith", ("linux",))?
        .is_truthy()?
        || platform
            .call_method1("startswith", ("freebsd",))?
            .is_truthy()?
    {
        return Ok(PyList::new(py, [module.getattr("SpnavBackend")?])?.into_any());
    }
    if platform.eq("win32")? {
        return Ok(PyList::new(py, [module.getattr("RawInputBackend")?])?.into_any());
    }
    Ok(PyList::empty(py).into_any())
}

fn parse_spnav_packets<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let data = a.get_item(0)?.extract::<Vec<u8>>()?;
    let packets = PyList::empty(py);
    for packet in data.chunks_exact(32) {
        let axes = spnav_words(packet);
        packets.append((axes[0], &axes[1..]))?;
    }
    Ok(packets.into_any())
}

fn parse_hid_report<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let bytes = a.get_item(0)?.extract::<Vec<u8>>()?;
    let state = a.get_item(1)?;
    let Some((&kind, body)) = bytes.split_first() else {
        return Ok(none(py));
    };
    let triple = |offset: usize| {
        let read = |i| i16::from_le_bytes([body[offset + i], body[offset + i + 1]]);
        (read(0), read(2), read(4))
    };
    match kind {
        1 if body.len() >= 6 => {
            state.set_item("t", triple(0))?;
            if body.len() >= 12 {
                state.set_item("r", triple(6))?;
            }
        }
        2 if body.len() >= 6 => state.set_item("r", triple(0))?,
        3 => {
            let mut mask = [0u8; 4];
            let count = body.len().min(4);
            mask[..count].copy_from_slice(&body[..count]);
            state.set_item("buttons", u32::from_le_bytes(mask))?;
            return Ok(none(py));
        }
        _ => return Ok(none(py)),
    }
    let t = state.call_method1("get", ("t", (0, 0, 0)))?;
    let r = state.call_method1("get", ("r", (0, 0, 0)))?;
    let axes = PyTuple::new(
        py,
        t.try_iter()?
            .chain(r.try_iter()?)
            .collect::<PyResult<Vec<_>>>()?,
    )?;
    py.import("views.ndof_input")?
        .call_method1("from_hid", axes)
}

fn load_settings<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.ndof_input")?;
    let ndof = py.import("ingetrazo_render.ndof")?;
    let st = py.import("core.settings")?.call_method0("settings")?;
    let settings = ndof.call_method0("NdofSettings")?;
    for row in module.getattr("_KEYS")?.try_iter()? {
        let pair = row?;
        let key = pair.get_item(0)?.extract::<String>()?;
        let raw = st.call_method1("value", (format!("ndof/{key}"), py.None()))?;
        if raw.is_none() {
            continue;
        }
        if pair.get_item(1)?.is_instance_of::<pyo3::types::PyBool>() {
            settings.setattr(&key, truth_setting(&raw)?)?;
        } else if let Some(value) = read_number(&raw)? {
            settings.setattr(&key, bounded_speed(value))?;
        }
    }
    let preset = st
        .call_method1("value", ("ndof/mapping_preset", "original"))?
        .str()?
        .to_string();
    settings.setattr("mapping_preset", preset_name(&preset))?;
    for axis in ndof.getattr("AXES")?.try_iter()? {
        let axis = axis?.extract::<String>()?;
        let default = ndof.getattr("DEFAULT_MAPPING")?.get_item(&axis)?;
        let action = st
            .call_method1("value", (format!("ndof/map/{axis}"), &default))?
            .str()?;
        settings.getattr("axis_mapping")?.set_item(
            &axis,
            if ndof.getattr("ACTIONS")?.contains(&action)? {
                action.into_any()
            } else {
                default
            },
        )?;
        let inverted = st.call_method1("value", (format!("ndof/map_invert/{axis}"), "0"))?;
        settings
            .getattr("axis_inversion")?
            .set_item(&axis, truth_setting(&inverted)?)?;
        let speed = st.call_method1("value", (format!("ndof/map_speed/{axis}"), 1.))?;
        if let Some(x) = read_number(&speed)?.filter(|x| x.is_finite()) {
            settings
                .getattr("axis_sensitivity")?
                .set_item(axis, bounded_speed(x))?;
        }
    }
    Ok(settings)
}

fn current_settings<'py>(a: &Args<'py>) -> Reply<'py> {
    let module = a.py().import("views.ndof_input")?;
    let settings = module.getattr("_settings_cache")?;
    if !settings.is_none() {
        return Ok(settings);
    }
    let settings = module.call_method0("load_settings")?;
    module.setattr("_settings_cache", &settings)?;
    Ok(settings)
}

fn save_settings<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let settings = a.get_item(0)?;
    let module = py.import("views.ndof_input")?;
    module.setattr("_settings_cache", &settings)?;
    let st = py.import("core.settings")?.call_method0("settings")?;
    let ndof = py.import("ingetrazo_render.ndof")?;
    let flag = |b| if b { "1" } else { "0" };
    for row in module.getattr("_KEYS")?.try_iter()? {
        let pair = row?;
        let key = pair.get_item(0)?.extract::<String>()?;
        let value = settings.getattr(&key)?;
        let value = if pair.get_item(1)?.is_instance_of::<pyo3::types::PyBool>() {
            flag(value.is_truthy()?).into_bound_py_any(py)?
        } else {
            py.get_type::<pyo3::types::PyFloat>().call1((value,))?
        };
        st.call_method1("setValue", (format!("ndof/{key}"), value))?;
    }
    let preset = settings
        .getattr("mapping_preset")?
        .extract::<String>()
        .unwrap_or_default();
    st.call_method1("setValue", ("ndof/mapping_preset", preset_name(&preset)))?;
    for axis in ndof.getattr("AXES")?.try_iter()? {
        let axis = axis?.extract::<String>()?;
        let default = ndof.getattr("DEFAULT_MAPPING")?.get_item(&axis)?;
        let action = settings
            .getattr("axis_mapping")?
            .call_method1("get", (&axis, &default))?;
        st.call_method1(
            "setValue",
            (
                format!("ndof/map/{axis}"),
                if ndof.getattr("ACTIONS")?.contains(&action)? {
                    action
                } else {
                    default
                },
            ),
        )?;
        let inverted = settings
            .getattr("axis_inversion")?
            .call_method1("get", (&axis, false))?
            .is_truthy()?;
        st.call_method1(
            "setValue",
            (format!("ndof/map_invert/{axis}"), flag(inverted)),
        )?;
        let speed = settings
            .getattr("axis_sensitivity")?
            .call_method1("get", (&axis, 1.))?;
        let value = py
            .get_type::<pyo3::types::PyFloat>()
            .call1((speed,))?
            .extract::<f64>()?;
        st.call_method1(
            "setValue",
            (format!("ndof/map_speed/{axis}"), bounded_speed(value)),
        )?;
    }
    st.call_method0("sync")?;
    Ok(none(py))
}

fn cap_in_use<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.ndof_input")?;
    Ok({
        let value = (!module.getattr("_shared")?.is_none()).into_bound_py_any(py)?;
        if !value.is_truthy()? {
            value
        } else {
            module.getattr("_shared")?.call_method0("in_use")?
        }
    })
}

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Method] = &[
    ("_backends_for", Mode::Function, backends_for, &["platform:str"], "list", ""),
    ("parse_spnav_packets", Mode::Function, parse_spnav_packets, &["data:bytes"], "list", "The packets in *data* as ``(kind, values)`` — for tests and for\nanyone checking what the daemon sends."),
    ("parse_hid_report", Mode::Function, parse_hid_report, &["report:bytes", "state:dict"], "NdofSample | None", "Fold one HID report from a 3Dconnexion device into *state* (the last\ntranslation and rotation) and return the six axes, or None for a report\nthat carries no motion. Report 1 is the translation — or, on the newer\ndevices (SpaceMouse Compact/Pro Wireless, the Universal Receiver), all\nsix axes in one; report 2 the rotation; report 3 the buttons."),
    ("load_settings", Mode::Function, load_settings, &[], "", "The user's 3D mouse settings (Preferences ▸ Navigation)."),
    ("current_settings", Mode::Function, current_settings, &[], "", "The settings every window moves the camera with — read once, then\nkept current by :func:`save_settings`."),
    ("save_settings", Mode::Function, save_settings, &["settings"], "None", ""),
    ("cap_in_use", Mode::Function, cap_in_use, &[], "bool", "Whether a 3D mouse is moving the view right now — without opening a\nconnection if there is none yet."),
];

fn spnav_words(packet: &[u8]) -> [i32; 8] {
    std::array::from_fn(|i| i32::from_ne_bytes(packet[i * 4..i * 4 + 4].try_into().unwrap()))
}

pub(super) fn feed_spnav(backend: &Obj, chunk: &[u8]) -> PyResult<()> {
    let py = backend.py();
    let mut data = backend.getattr("_buf")?.extract::<Vec<u8>>()?;
    data.extend_from_slice(chunk);
    let mut latest = None;
    let mut seconds = 0.;
    let owner = backend.getattr("owner")?;
    let module = py.import("views.ndof_input")?;
    let mut packets = data.chunks_exact(32);
    for packet in &mut packets {
        let [kind, x, y, z, rx, ry, rz, period] = spnav_words(packet);
        match kind {
            0 => {
                let axes = (x, y, z, rx, ry, rz);
                owner.call_method1("_emit_raw", (axes,))?;
                latest = Some(module.call_method1("from_spacenavd", axes)?);
                seconds += f64::from(period) / 1000.;
            }
            1 | 2 => {
                owner.call_method1("_emit_button", (x, kind == 1))?;
            }
            _ => {}
        }
    }
    backend.setattr("_buf", pyo3::types::PyBytes::new(py, packets.remainder()))?;
    if let Some(sample) = latest {
        owner.call_method1("_emit_motion", (sample, (seconds > 0.).then_some(seconds)))?;
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn feed_hid(backend: &Obj, report: &[u8]) -> PyResult<()> {
    let py = backend.py();
    let state = backend.getattr("state")?;
    let owner = backend.getattr("owner")?;
    let before = state
        .call_method1("get", ("buttons", 0))?
        .extract::<u32>()?;
    let sample = py.import("views.ndof_input")?.call_method1(
        "parse_hid_report",
        (pyo3::types::PyBytes::new(py, report), &state),
    )?;
    if !sample.is_none() {
        let t = state.call_method1("get", ("t", (0, 0, 0)))?;
        let r = state.call_method1("get", ("r", (0, 0, 0)))?;
        owner.call_method1("_emit_raw", (t.add(r)?,))?;
        owner.call_method1("_emit_motion", (sample,))?;
    }
    let after = state
        .call_method1("get", ("buttons", 0))?
        .extract::<u32>()?;
    for bit in 0..32 {
        let mask = 1 << bit;
        if (before ^ after) & mask != 0 {
            owner.call_method1("_emit_button", (bit, after & mask != 0))?;
        }
    }
    Ok(())
}
fn bounded_speed(x: f64) -> f64 {
    if x.is_nan() {
        4.
    } else {
        x.clamp(0.25, 4.)
    }
}
fn preset_name(name: &str) -> &str {
    match name {
        "custom" | "freecad" => name,
        _ => "original",
    }
}
fn truth_setting(value: &Obj) -> PyResult<bool> {
    let s = value.str()?.to_string();
    Ok(s == "1" || s.eq_ignore_ascii_case("true"))
}
fn read_number(value: &Obj) -> PyResult<Option<f64>> {
    let py = value.py();
    match py
        .get_type::<pyo3::types::PyFloat>()
        .call1((value,))
        .and_then(|n| n.extract())
    {
        Ok(n) => Ok(Some(n)),
        Err(e)
            if e.is_instance_of::<pyo3::exceptions::PyTypeError>(py)
                || e.is_instance_of::<pyo3::exceptions::PyValueError>(py) =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}
