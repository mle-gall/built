// SPDX-License-Identifier: GPL-3.0-or-later
//! Host bindings for view_commands.
use super::*;
fn snapshot<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let window = a.get_item(0)?;
    let view = window.call_method0("_active_view")?;
    let scene = view.getattr("scene")?;
    let style = window.call_method0("active_style")?;
    let mut active = view.getattr("nav_mode")?;
    if !active.is_truthy()? {
        active = window.call_method1("_tool_key", (view.getattr("active_tool")?,))?;
    }
    if !active.is_truthy()? {
        active = "".into_bound_py_any(py)?;
    }
    let flags = PySet::empty(py)?;
    for (owner, names) in [
        (&style, &["back_edges", "section_fill"][..]),
        (
            &scene,
            &[
                "show_section_planes",
                "show_all_cutters",
                "show_section_cuts",
                "show_hidden_geometry",
                "show_axes",
            ][..],
        ),
    ] {
        for name in names {
            if attr_flag(owner, name, false)? {
                flags.add(name)?;
            }
        }
    }
    if optional(&style, "face_mode")?.eq("xray")? {
        flags.add("xray")?;
    }
    if scene.getattr("shadows")?.getattr("enabled")?.is_truthy()? {
        flags.add("shadows")?;
    }
    (
        active,
        view.getattr("camera")?.call_method0("projection_mode")?,
        style
            .getattr_opt("name")?
            .unwrap_or("".into_bound_py_any(py)?),
        flags,
        scene.getattr("selection")?.is_truthy()?,
    )
        .into_bound_py_any(py)
}

fn sync<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.view_commands")?;
    let cluster: Obj<'py>;
    let menu: Obj<'py>;
    let view = a.get_item(0)?;
    let win = a.get_item(1)?;
    let current_language = py.import("built_core.i18n")?.getattr("current_language")?;
    let snapshot = module.getattr("_snapshot")?.call1((&win,))?;
    let language = current_language.call0()?;
    let sig = (
        &snapshot,
        &language,
        py.import("builtins")?
            .getattr("vars")?
            .call1((&win,))?
            .call_method1("get", ("_view_binding_version", 0))?,
    )
        .into_bound_py_any(py)?;
    if sig.ne(optional(&view, "_ui_cluster_sig")?)? {
        {
            let [part0, part1] = unpack::<2>(
                &module.getattr("command_chrome")?.call1((
                    &snapshot,
                    module.getattr("_labels")?.call1((&language,))?,
                    py.import("builtins")?
                        .getattr("vars")?
                        .call1((&win,))?
                        .call_method1("get", ("_view_bindings", PyDict::new(py).into_any()))?,
                ))?,
            )?;
            cluster = part0;
            menu = part1;
        }
        view.getattr("renderer")?
            .call_method1("ui_set_cluster", (&cluster, &menu))?;
        view.setattr("_ui_cluster_sig", &sig)?;
    }
    view_state(&view, &win)?;
    Ok(none(py))
}

/// The axis gizmo follows the active camera every frame (UX pass 2, M03).
fn view_state<'py>(view: &Obj<'py>, win: &Obj<'py>) -> PyResult<()> {
    let py = view.py();
    let camera = win.call_method0("_active_view")?.getattr("camera")?;
    let (yaw, pitch): (f64, f64) = (camera.getattr("yaw")?.extract()?, camera.getattr("pitch")?.extract()?);
    let projection: String = camera.call_method0("projection_mode")?.extract()?;
    let style = win.call_method0("active_style")?;
    let style: String = if style.is_none() { String::new() } else {
        style.getattr_opt("name")?.map(|n| n.extract()).transpose()?.unwrap_or_default()
    };
    let aligned = crate::ui::kit::view_gizmo::aligned_view(yaw, pitch);
    let tr = |s: &str| crate::settings_rows::tr(py, s).unwrap_or_else(|_| s.to_owned());
    let label = crate::ui::kit::view_gizmo::view_label(aligned, &projection, &style, tr);
    view.getattr("renderer")?.call_method1("ui_set_view_state", (yaw, pitch, label))?;
    Ok(())
}

fn run<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.view_commands")?;
    let key = a.get_item(1)?;
    let win = a.get_item(0)?;
    let bindings = py
        .import("builtins")?
        .getattr("vars")?
        .call1((&win,))?
        .call_method1("get", ("_view_bindings", PyDict::new(py).into_any()))?;
    if !(bindings
        .call_method1("get", (&key, ("", true).into_bound_py_any(py)?))?
        .get_item(1)?
        .is_truthy()?)
    {
        return (false).into_bound_py_any(py);
    }
    let ran = module.getattr("activate_view_command")?.call1((
        &win,
        &key,
        module.getattr("_snapshot")?.call1((&win,))?,
    ))?;
    if ran.is_truthy()?
        && bindings.contains(&key)?
        && !py
            .import("builtins")?
            .getattr("vars")?
            .call1((&win,))?
            .contains("_host")?
    {
        win.call_method0("_sync_section_menu")?;
        win.call_method0("_sync_style_menu")?;
    }
    Ok(ran.clone())
}

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Method] = &[
    ("_snapshot", Mode::Function, snapshot, &["win"], "", ""),
    ("sync", Mode::Function, sync, &["view", "win"], "", ""),
    ("run", Mode::Function, run, &["win", "key"], "", ""),
];
