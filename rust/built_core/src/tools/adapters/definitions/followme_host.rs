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
    module.add("_MESSAGES", fields!(py; "invalid" => "Follow Me: the selected edges must form one simple path", "touch" => "Follow Me needs at least one path edge.").into_any())?;
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
