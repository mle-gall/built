// SPDX-License-Identifier: GPL-3.0-or-later
//! Plugin-facing definitions for select_host.
use super::*;

#[pyfunction]
fn edit_done(
    view: &Obj,
    entity: &Obj,
    measured: &Obj,
    dimension: bool,
    text: &Obj,
) -> PyResult<()> {
    let py = view.py();
    if !text.is_none() {
        let stripped = text.call_method0("strip")?;
        let value = if dimension
            && (!stripped.is_truthy()? || stripped.eq("<>")? || stripped.eq(measured)?)
        {
            none(py)
        } else {
            stripped
        };
        if (dimension || value.is_truthy()?) && !value.eq(entity.getattr("text")?)? {
            let command = if dimension {
                imported(py, "built_core.history")?.getattr("EditDimensionTextCommand")?
            } else {
                imported(py, "tools.select_host")?.getattr("EditTextLabelCommand")?
            };
            view.getattr("history")?
                .call_method1("execute", (command.call1((entity, value))?,))?;
        }
    }
    view.call_method0("update")?;
    Ok(())
}
fn prompt<'py>(a: &Args<'py>, dimension: bool) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let entity = a.get_item(1)?;
    let measured = if dimension {
        if let Some(fmt) = view
            .getattr_opt("_format_dim_value")?
            .filter(|x| !x.is_none())
        {
            let style = view
                .getattr("scene")?
                .getattr_opt("dimension_style")?
                .filter(|v| v.is_truthy().unwrap_or(false))
                .unwrap_or_else(|| PyDict::new(py).into_any());
            fmt.call1((entity.call_method0("value")?, style))?
        } else {
            entity.call_method0("label")?
        }
    } else {
        none(py)
    };
    let mut current = entity.getattr("text")?;
    if dimension && !current.is_truthy()? {
        current = measured.clone();
    }
    let (title, label) = if dimension {
        ("Dimension", "Dimension text (<> = measured value):")
    } else {
        ("Text", "Label text:")
    };
    let done = py.import("functools")?.call_method1(
        "partial",
        (
            wrap_pyfunction!(edit_done, py)?,
            &view,
            entity,
            measured,
            dimension,
        ),
    )?;
    let options = fields!(py;"done"=>done);
    if !dimension {
        options.set_item("multiline", true)?;
    }
    let tr = imported(py, "built_core.i18n")?.getattr("tr")?;
    imported(py, "built_render")?
        .getattr("chrome_forms")?
        .call_method(
            "ask_text",
            (
                view.call_method0("window")?,
                tr.call1((title,))?,
                tr.call1((label,))?,
                current,
            ),
            Some(&options),
        )?;
    Ok(none(py))
}
fn edit_text<'py>(a: &Args<'py>) -> Reply<'py> {
    prompt(a, false)
}
fn edit_dimension<'py>(a: &Args<'py>) -> Reply<'py> {
    prompt(a, true)
}

pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    register_tool_functions(&module.dict())?;
    module.add("CYCLE_PX", 4.0)?;
    install_methods(
        module,
        module.dict().as_any(),
        &[
            (
                "_edit_text",
                Mode::Function,
                edit_text,
                &["viewport", "entity"],
                "None",
                "",
            ),
            (
                "_edit_dimension",
                Mode::Function,
                edit_dimension,
                &["viewport", "entity"],
                "None",
                "",
            ),
        ],
    )?;
    module.add("HOST_VERBS",fields!(py;"toggle_section"=>module.getattr("_toggle_section")?,
    "edit_text"=>module.getattr("_edit_text")?,"edit_dimension"=>module.getattr("_edit_dimension")?))?;
    Ok(())
}
