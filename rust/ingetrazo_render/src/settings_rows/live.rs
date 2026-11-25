//! Apply changes to the persistent settings and every affected document.
use super::*;
type Obj<'p> = Bound<'p, PyAny>;
pub(crate) const NEW_UNITS: &str = "units/use_for_new_documents";

fn viewports<'p>(window: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    if let Some(all) = window
        .getattr_opt("all_viewports")?
        .filter(|v| v.is_callable())
    {
        return all.call0()?.try_iter()?.collect();
    }
    Ok(opt(window, "viewport").into_iter().collect())
}
fn notify_if_present(owner: &Obj, method: &str) -> PyResult<()> {
    if let Some(f) = owner.getattr_opt(method)?.filter(|f| f.is_callable()) {
        f.call0()?;
    }
    Ok(())
}
fn general<'p>(py: Python<'p>) -> PyResult<Bound<'p, PyModule>> {
    py.import("ingetrazo_core.general_settings")
}
fn bit(on: bool) -> &'static str {
    if on {
        "1"
    } else {
        "0"
    }
}

#[pyfunction]
fn apply_language(code: &str, st: &Obj) -> PyResult<bool> {
    let lang = st.py().import("core.i18n")?;
    let saved = st.call_method1("value", ("language",))?;
    let saved = if saved.is_truthy()? {
        saved
    } else {
        lang.getattr("SYSTEM")?
    };
    if saved.eq(code)? {
        return Ok(false);
    }
    st.call_method1("setValue", ("language", code))?;
    let resolved = lang.call_method1("resolve_language", (code,))?;
    if resolved.eq(lang.call_method0("current_language")?)? {
        return Ok(false);
    }
    lang.call_method1("set_language", (resolved,))?;
    Ok(true)
}
#[pyfunction]
fn apply_rest_mode(window: &Obj, mode: &str, fade: f64) -> PyResult<()> {
    let changed = opt(window, "viewport")
        .map(|vp| vp.getattr("edit_rest_mode")?.ne(mode))
        .transpose()?
        .unwrap_or(false);
    for vp in viewports(window)? {
        if let Some(set) = vp
            .getattr_opt("set_edit_rest_fade")?
            .filter(|f| f.is_callable())
        {
            set.call1((fade,))?;
        }
        if !vp.getattr("edit_rest_mode")?.eq(mode)? {
            vp.call_method1("set_edit_rest_mode", (mode,))?;
        }
    }
    if changed {
        if let Some(actions) = opt(window, "_rest_actions") {
            let action = actions.call_method1("get", (mode,))?;
            if !action.is_none() {
                action.call_method1("setChecked", (true,))?;
            }
        }
    }
    Ok(())
}
#[pyfunction]
fn apply_autosave(
    window: &Obj,
    on: bool,
    minutes: u32,
    backup: Option<bool>,
    st: &Obj,
) -> PyResult<()> {
    let gs = general(window.py())?;
    gs.call_method1("set_value", ("autosave", bit(on), st))?;
    gs.call_method1("set_value", ("autosave_min", minutes, st))?;
    if let Some(b) = backup {
        gs.call_method1("set_value", ("backup", bit(b), st))?;
    }
    notify_if_present(window, "_setup_autosave")
}
#[pyfunction]
fn apply_undo_steps(window: &Obj, steps: u32, st: &Obj) -> PyResult<()> {
    general(window.py())?.call_method1("set_value", ("undo_steps", steps, st))?;
    for vp in viewports(window)? {
        if let Some(history) = opt(&vp, "history") {
            history.setattr("max_steps", steps)?;
        }
    }
    Ok(())
}
fn nav_bool(window: &Obj, on: bool, st: &Obj, key: &str, attr: &str) -> PyResult<()> {
    st.call_method1("setValue", (key, bit(on)))?;
    for vp in viewports(window)? {
        vp.setattr(attr, on)?;
    }
    Ok(())
}
#[pyfunction]
fn apply_invert_wheel(window: &Obj, on: bool, st: &Obj) -> PyResult<()> {
    nav_bool(window, on, st, "nav/invert_wheel", "_invert_wheel")
}
#[pyfunction]
fn apply_invert_orbit(window: &Obj, on: bool, st: &Obj) -> PyResult<()> {
    nav_bool(window, on, st, "nav/invert_orbit_y", "_invert_orbit_y")
}
#[pyfunction]
fn apply_slides(window: &Obj, trackpad: &str, st: &Obj) -> PyResult<()> {
    st.call_method1("setValue", ("nav/trackpad_slide", trackpad))?;
    for vp in viewports(window)? {
        vp.setattr("_trackpad_slide", trackpad)?;
    }
    Ok(())
}
#[pyfunction]
fn apply_cursor_style(window: &Obj, style: &str, st: &Obj) -> PyResult<()> {
    general(window.py())?.call_method1("set_value", ("cursor_style", style, st))?;
    if let Some(vp) = opt(window, "viewport") {
        notify_if_present(&vp, "_apply_tool_cursor")?;
    }
    Ok(())
}
#[pyfunction]
fn apply_soft_shadows(window: &Obj, on: bool, level: u32, st: &Obj) -> PyResult<()> {
    let value = if on { level } else { 0 };
    st.call_method1("setValue", ("display/soft_shadows", value))?;
    st.call_method1("setValue", ("display/soft_shadows_level", level))?;
    for vp in viewports(window)? {
        if !vp
            .getattr_opt("_soft_shadows")?
            .map(|s| s.eq(value))
            .transpose()?
            .unwrap_or(false)
        {
            vp.setattr("_soft_shadows", value)?;
            notify_if_present(&vp, "update")?;
        }
    }
    if let Some(docs) = opt(window, "_documents") {
        for doc in docs.try_iter()? {
            if let Some(view) = opt(&doc?, "stand_in") {
                notify_if_present(&view, "update")?;
            }
        }
    }
    Ok(())
}
#[pyfunction]
pub(crate) fn document_history<'p>(viewport: &Obj<'p>) -> PyResult<Obj<'p>> {
    if let Some(history) = opt(viewport, "history") {
        if history
            .getattr_opt("execute")?
            .is_some_and(|f| f.is_callable())
        {
            return Ok(history);
        }
    }
    let history = viewport
        .py()
        .import("ingetrazo_core.history")?
        .call_method1("History", (viewport.getattr("scene")?,))?;
    viewport.setattr("history", &history)?;
    Ok(history)
}
#[pyfunction]
pub(crate) fn apply_document_units(window: &Obj, chosen: &Obj) -> PyResult<bool> {
    let py = window.py();
    let Some(vp) = opt(window, "viewport") else {
        return Ok(false);
    };
    let Some(scene) = opt(&vp, "scene") else {
        return Ok(false);
    };
    if chosen.eq(py
        .import("core.units")?
        .call_method1("model_units_of", (scene,))?)?
    {
        return Ok(false);
    }
    if !py
        .import("ingetrazo_core.command_protocol")?
        .call_method1("set_model_units", (document_history(&vp)?, chosen))?
        .get_item("ok")?
        .is_truthy()?
    {
        return Ok(false);
    }
    if let Some(notify) = vp
        .getattr_opt("notify_scene_changed")?
        .filter(|n| n.is_callable())
    {
        notify.call0()?;
    } else {
        vp.call_method0("update")?;
    }
    Ok(true)
}
#[pyfunction]
fn read_named_ndof_profiles(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    let raw = py
        .import("ingetrazo_core.settings")?
        .call_method0("settings")?
        .call_method1("value", ("ndof/named_profiles", "{}"))?;
    let parse = || -> PyResult<Bound<'_, PyDict>> {
        let table = py.import("json")?.call_method1("loads", (raw.str()?,))?;
        let profiles = PyDict::new(py);
        if let Ok(table) = table.cast::<PyDict>() {
            let convert = crate::preferences::device_profile::profile_from_dict;
            for (name, data) in table.iter() {
                if (1..=80).contains(&name.len()?) {
                    profiles.set_item(name, convert(&data)?)?;
                }
            }
        }
        Ok(profiles)
    };
    match parse() {
        Err(e)
            if e.is_instance_of::<pyo3::exceptions::PyValueError>(py)
                || e.is_instance_of::<pyo3::exceptions::PyTypeError>(py) =>
        {
            Ok(PyDict::new(py))
        }
        other => other,
    }
}
#[pyfunction]
fn legacy_ndof_inversion<'p>(nd: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let out = PyDict::new(nd.py());
    for (key, parent, axis) in [
        ("right", "invert_pan", "invert_pan_x"),
        ("up", "invert_pan", "invert_pan_y"),
        ("tilt", "invert_rotate", "invert_tilt"),
        ("spin", "invert_rotate", "invert_spin"),
    ] {
        out.set_item(key, nd.getattr(parent)?.ne(nd.getattr(axis)?)?)?;
    }
    out.set_item("forward", nd.getattr("invert_zoom")?)?;
    out.set_item("roll", false)?;
    Ok(out)
}
#[pyfunction]
fn save_ndof(nd: &Obj, named: &Bound<PyDict>, selected: &str, st: &Obj) -> PyResult<()> {
    let py = nd.py();
    nd.setattr(
        "mapping_preset",
        if selected == "freecad" {
            "freecad"
        } else {
            "custom"
        },
    )?;
    let profiles = PyDict::new(py);
    let convert = crate::preferences::device_profile::profile_to_dict;
    for (name, profile) in named.iter() {
        profiles.set_item(name, convert(&profile)?)?;
    }
    st.call_method1(
        "setValue",
        (
            "ndof/named_profiles",
            py.import("json")?.call_method1("dumps", (profiles,))?,
        ),
    )?;
    st.call_method1(
        "setValue",
        (
            "ndof/selected_profile",
            selected.strip_prefix("named:").unwrap_or(""),
        ),
    )?;
    py.import("views.ndof_input")?
        .call_method1("save_settings", (nd,))?;
    Ok(())
}
pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    for f in [
        wrap_pyfunction!(apply_language, m)?,
        wrap_pyfunction!(apply_rest_mode, m)?,
        wrap_pyfunction!(apply_autosave, m)?,
        wrap_pyfunction!(apply_undo_steps, m)?,
        wrap_pyfunction!(apply_invert_wheel, m)?,
        wrap_pyfunction!(apply_invert_orbit, m)?,
        wrap_pyfunction!(apply_slides, m)?,
        wrap_pyfunction!(apply_cursor_style, m)?,
        wrap_pyfunction!(apply_soft_shadows, m)?,
        wrap_pyfunction!(document_history, m)?,
        wrap_pyfunction!(apply_document_units, m)?,
        wrap_pyfunction!(read_named_ndof_profiles, m)?,
        wrap_pyfunction!(legacy_ndof_inversion, m)?,
        wrap_pyfunction!(save_ndof, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
