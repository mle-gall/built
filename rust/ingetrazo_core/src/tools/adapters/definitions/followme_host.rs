// SPDX-License-Identifier: GPL-3.0-or-later
//! Public classes and initial state for followme_host.
use super::*;
fn follow_me_host_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.followme_host")?;
    let tool = a.get_item(0)?;
    py.import("builtins")?
        .getattr("super")?
        .call1((module.getattr("FollowMeHost")?, &tool))?
        .call_method0("__init__")?;
    tool.setattr("_entities", PyDict::new(py).into_any())?;
    Ok(none(py))
}
pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add("_MESSAGES", fields!(py; "invalid" => "Follow Me: the selected edges must form one simple path", "intro" => "Follow Me: click the profile and drag along the path (it highlights red), click to finish — or select the path first, then click the profile", "drag" => "Follow Me: drag along the path (it highlights red) — click to finish, Esc to cancel; hold Alt over a face to follow its perimeter", "touch" => "Follow Me: touch the path edges to extrude along them").into_any())?;
    module.add(
        "_FAILED",
        "Follow Me: could not sweep along that path (reversal or degenerate segment)",
    )?;
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("wireframe_color", (0.85, 0.16, 0.16, 1.0))?;
        let cls = define(
            module,
            "FollowMeHost",
            &[py.import("builtins")?.getattr("object")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[(
                "__init__",
                Mode::Method,
                follow_me_host_init,
                &["self"],
                "",
                "",
            )],
        )?;
    }
    Ok(())
}
