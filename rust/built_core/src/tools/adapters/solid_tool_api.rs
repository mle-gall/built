// SPDX-License-Identifier: GPL-3.0-or-later
//! SolidTool callbacks on the live document.
use super::*;

fn siblings<'py>(a: &Args<'py>) -> Reply<'py> {
    let scene = a.get_item(0)?;
    Ok(if scene.getattr("edit_group")?.is_truthy()? {
        scene.getattr("edit_group")?.getattr("children")?
    } else {
        scene.getattr("groups")?
    })
}
fn qt_cursor<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    // «×» over anything that is not a solid, «1» or «2» over a solid for
    // the pick it will be.
    let badge: String = tool.getattr("_badge")?.extract()?;
    pyo3::IntoPyObjectExt::into_bound_py_any(format!("tool:solid-{badge}"), py)
}
fn host_solid_selection<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let group = imported(py, "built_core.group")?.getattr("Group")?;
    tool.setattr("_viewport", &vp)?;
    return Ok({
        let items = PyList::empty(py);
        for item in tool
            .call_method1("_siblings", (vp.getattr("scene")?,))?
            .try_iter()?
        {
            let g = item?;
            if !(g.is_instance(&group)?
                && vp.getattr("scene")?.getattr("selection")?.contains(&g)?)
            {
                continue;
            }
            items.append(
                (
                    g.getattr("uid")?,
                    tool.call_method1("_is_solid", (vp.getattr("scene")?, &g))?,
                )
                    .into_bound_py_any(py)?,
            )?;
        }
        items.into_any()
    });
}
fn is_solid<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let group = a.get_item(2)?;
    let scene = a.get_item(1)?;
    let tool = a.get_item(0)?;
    let is_solid = imported(py, "built_core.solids")?.getattr("is_solid")?;
    if tool
        .getattr("_cache_version")?
        .ne(scene.getattr("version")?)?
    {
        tool.getattr("_cache")?.call_method0("clear")?;
        tool.setattr("_cache_version", scene.getattr("version")?)?;
    }
    if !tool.getattr("_cache")?.contains(group.getattr("uid")?)? {
        tool.getattr("_cache")?
            .set_item(group.getattr("uid")?, is_solid.call1((&group,))?)?;
    }
    tool.getattr("_cache")?.get_item(group.getattr("uid")?)
}
fn host_solid_pick<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let px = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    tool.setattr("_viewport", &vp)?;
    let g = vp.call_method1("pick_group", positional(&px)?)?;
    Ok(if !g.is_none() {
        (
            g.getattr("uid")?,
            tool.call_method1("_is_solid", (vp.getattr("scene")?, &g))?,
        )
            .into_bound_py_any(py)?
    } else {
        none(py)
    })
}
fn host_solid_error<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.solid_host")?;
    let message = a.get_item(2)?;
    let vp = a.get_item(1)?;
    let tr = imported(py, "built_core.i18n")?.getattr("tr")?;
    vp.call_method1(
        "flash_status",
        (
            tr.call1((module.getattr("_SOLID_ERRORS")?.call_method1(
                "get",
                (
                    &message,
                    ({
                        let value = message.clone();
                        if value.is_truthy()? {
                            value
                        } else {
                            ("empty").into_bound_py_any(py)?
                        }
                    }),
                ),
            )?,))?,
            5000,
        ),
    )?;
    Ok(none(py))
}
fn host_solid_cutter_hint<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let vp = a.get_item(1)?;
    let tr = imported(py, "built_core.i18n")?.getattr("tr")?;
    vp.call_method1(
        "flash_status",
        (
            tr.call1(("Click the solid that cuts, then the one it cuts",))?,
            4000,
        ),
    )?;
    Ok(none(py))
}
fn on_activate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let window: Obj<'py>;
    if tool
        .call_method1("_send", (&vp, fields!(py; "type" => "activate").into_any()))?
        .get_item("committed")?
        .is_truthy()?
    {
        vp.call_method0("update")?;
        window = if vp.hasattr("window")? {
            vp.call_method0("window")?
        } else {
            none(py)
        };
        if window.hasattr("_activate_tool")? {
            window.call_method1("_activate_tool", ("select",))?;
        }
    }
    Ok(none(py))
}
fn on_hover<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let apply: Obj<'py>;
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    if tool
        .call_method1(
            "_send",
            (
                ctx.getattr("viewport")?,
                tool.call_method1("_pointer", ("hover", &ctx))?,
            ),
        )?
        .get_item("redraw")?
        .is_truthy()?
    {
        apply = optional(&ctx.getattr("viewport")?, "_apply_tool_cursor")?;
        if apply.is_truthy()? {
            apply.call0()?;
        }
    }
    Ok(none(py))
}

fn first<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    let uid = tool.call_method1("_state", ("first",))?;
    let vp = tool.getattr("_viewport")?;
    if vp.is_truthy()? && uid.is_truthy()? {
        for group in tool
            .call_method1("_siblings", (vp.getattr("scene")?,))?
            .try_iter()?
        {
            let group = group?;
            if group.getattr("uid")?.eq(&uid)? {
                return Ok(group);
            }
        }
    }
    Ok(none(a.py()))
}
fn host_solid_select<'py>(a: &Args<'py>) -> Reply<'py> {
    let [tool, vp, uid] = unpack::<3>(a.as_any())?;
    let scene = vp.getattr("scene")?;
    if uid.is_none() {
        scene.getattr("selection")?.call_method0("clear")?;
        scene.call_method0("bump_view")?;
    } else {
        let selected = PySet::empty(a.py())?;
        for group in tool.call_method1("_siblings", (&scene,))?.try_iter()? {
            let group = group?;
            if group.getattr("uid")?.eq(&uid)? {
                selected.add(group)?;
            }
        }
        scene.setattr("selection", selected)?;
        scene.setattr("version", scene.getattr("version")?.add(1)?)?;
    }
    Ok(none(a.py()))
}
fn host_solid_result<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    let scene = a.get_item(1)?.getattr("scene")?;
    let selection = scene.getattr("selection")?;
    for group in tool.call_method1("_siblings", (&scene,))?.try_iter()? {
        let group = group?;
        if selection.contains(&group)? {
            return group.getattr("uid");
        }
    }
    Ok(none(a.py()))
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("_siblings", Mode::Static, siblings, &["scene"], "", ""),
    ("first", Mode::Property, first, &["self"], "", ""),
    ("qt_cursor", Mode::Property, qt_cursor, &["self"], "", ""),
    ("host_solid_selection", Mode::Method, host_solid_selection, &["self", "vp", "_"], "", ""),
    ("_is_solid", Mode::Method, is_solid, &["self", "scene", "group"], "", ""),
    ("host_solid_pick", Mode::Method, host_solid_pick, &["self", "vp", "px"], "", ""),
    ("host_solid_select", Mode::Method, host_solid_select, &["self", "vp", "uid"], "", ""),
    ("host_solid_result", Mode::Method, host_solid_result, &["self", "vp", "_"], "", ""),
    ("host_solid_error", Mode::Method, host_solid_error, &["self", "vp", "message"], "", ""),
    ("host_solid_cutter_hint", Mode::Method, host_solid_cutter_hint, &["self", "vp", "_"], "", ""),
    ("on_activate", Mode::Method, on_activate, &["self", "vp"], "", ""),
    ("on_hover", Mode::Method, on_hover, &["self", "ctx"], "", ""),
];
