// SPDX-License-Identifier: GPL-3.0-or-later
//! Host-side tool actions. The class and model handles belong to the plugin API;
//! events and commands still enter the same ToolMachine and history.
use built_python_bindings::{Adapter, Endpoint, Mode, Obj, Registrar};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet, PyTuple},
    IntoPyObjectExt,
};

// Most machine events consist of a few named fields.
macro_rules! fields {
    ($py:expr; $($key:expr => $value:expr),* $(,)?) => {{
        let fields = PyDict::new($py);
        $(fields.set_item($key, $value)?;)*
        fields
    }};
}

fn positional<'py>(value: &Obj<'py>) -> PyResult<Args<'py>> {
    if let Ok(tuple) = value.cast::<PyTuple>() {
        return Ok(tuple.clone());
    }
    Ok(value
        .py()
        .get_type::<PyTuple>()
        .call1((value,))?
        .cast_into()?)
}

fn unpack<'py, const N: usize>(value: &Obj<'py>) -> PyResult<[Obj<'py>; N]> {
    if value.cast_exact::<PyTuple>().is_ok() {
        return value.extract();
    }
    let mut items = value.try_iter()?;
    let row: Vec<_> = items.by_ref().take(N).collect::<PyResult<_>>()?;
    if row.len() != N || items.next().transpose()?.is_some() {
        return Err(pyo3::exceptions::PyValueError::new_err(format!(
            "expected {N} fields"
        )));
    }
    row.try_into()
        .map_err(|_| pyo3::exceptions::PyValueError::new_err("invalid row"))
}
mod eye_host_api;
mod fillet_host_api;
mod first_person_host_api;
mod move_host_api;
mod offset_host_api;
mod position_camera_host_api;
mod protractor_base_api;
mod protractor_host_api;
mod protractor_tool_api;
mod pushpull;
mod rotate_host_api;
mod rotate_tool_api;
mod scale;
mod select_tool_api;
mod texture;
mod texture_preview;
mod walk_host_api;
type Args<'py> = Bound<'py, PyTuple>;
type Reply<'py> = PyResult<Obj<'py>>;
type Method = (
    &'static str,
    Mode,
    Adapter,
    &'static [&'static str],
    &'static str,
    &'static str,
);

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn optional<'py>(obj: &Obj<'py>, name: &str) -> Reply<'py> {
    Ok(obj.getattr_opt(name)?.unwrap_or_else(|| none(obj.py())))
}

fn screen<'py>(ctx: &Obj<'py>) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let point = ctx.getattr("screen")?;
    Ok((point.call_method0("x")?, point.call_method0("y")?))
}

fn registration(module: &str, name: &str) -> Option<(&'static [Method], &'static str)> {
    Some(match (module, name) {
        ("tools.scale_host", "ScaleHost") => (scale::METHODS, stringify!(scale)),
        ("tools.texture_position_metal", "TexturePreview") => {
            (texture_preview::METHODS, stringify!(texture_preview))
        }
        ("tools.texture_position", "TexturePositionTool") => {
            (texture::METHODS, stringify!(texture))
        }
        ("tools.fillet_host", "FilletHost") => {
            (fillet_host_api::METHODS, stringify!(fillet_host_api))
        }
        ("tools.rust_tools", "SelectTool") => {
            (select_tool_api::METHODS, stringify!(select_tool_api))
        }
        ("tools.move_host", "MoveHost") => (move_host_api::METHODS, stringify!(move_host_api)),
        ("tools.rotate_host", "RotateHost") => {
            (rotate_host_api::METHODS, stringify!(rotate_host_api))
        }
        ("tools.rust_tools", "RotateTool") => {
            (rotate_tool_api::METHODS, stringify!(rotate_tool_api))
        }
        ("tools.walkthrough_host", "EyeHost") => (eye_host_api::METHODS, stringify!(eye_host_api)),
        ("tools.walkthrough_host", "PositionCameraHost") => (
            position_camera_host_api::METHODS,
            stringify!(position_camera_host_api),
        ),
        ("tools.walkthrough_host", "WalkHost") => {
            (walk_host_api::METHODS, stringify!(walk_host_api))
        }
        ("tools.walkthrough_host", "FirstPersonHost") => (
            first_person_host_api::METHODS,
            stringify!(first_person_host_api),
        ),
        ("tools.offset_host", "OffsetHost") => {
            (offset_host_api::METHODS, stringify!(offset_host_api))
        }
        ("tools.protractor_host", "ProtractorBase") => (
            protractor_base_api::METHODS,
            stringify!(protractor_base_api),
        ),
        ("tools.protractor_host", "ProtractorHost") => (
            protractor_host_api::METHODS,
            stringify!(protractor_host_api),
        ),
        ("tools.rust_tools", "ProtractorTool") => (
            protractor_tool_api::METHODS,
            stringify!(protractor_tool_api),
        ),
        ("tools.pushpull", "PushPullTool") => (pushpull::METHODS, stringify!(pushpull)),
        _ => return None,
    })
}

/// Dependency audit for the published Metal tool coverage. Only the developer
/// diagnostic reads source files. Interactive dispatch uses the method table.
#[pyfunction]
pub fn tool_preview_hooks(
    root: std::path::PathBuf,
    module: &str,
    name: &str,
) -> PyResult<Vec<String>> {
    use std::collections::BTreeSet;
    use std::sync::LazyLock;
    let Some((methods, source)) = registration(module, name) else {
        return Ok(Vec::new());
    };
    let path = root
        .join("rust/ingetrazo_core/src/tools/adapters")
        .join(format!("{source}.rs"));
    let content = std::fs::read_to_string(path)?;
    static HOOKS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
        r#"(?:viewport|vp)\s*\.\s*(?:getattr|call_method[01]?)\(\s*"(pick_\w+|\w*preview\w*|_world_to_pixel|_pixel_to_ray)""#
    ).expect("tool viewport hook pattern")
    });
    let mut hooks: BTreeSet<String> = HOOKS
        .captures_iter(&content)
        .map(|m| m[1].to_owned())
        .collect();
    for &(name, _, _, _, _, _) in methods {
        match name {
            "rubber_band_lines" => {
                hooks.insert(name.to_owned());
            }
            "scale_box_state" => {
                hooks.insert("scale_box_state (screen handles)".to_owned());
            }
            _ => {}
        }
    }
    Ok(hooks.into_iter().collect())
}

#[pyfunction]
pub fn register_tool_adapter<'py>(cls: &Obj<'py>) -> Reply<'py> {
    let py = cls.py();
    let factory =
        Registrar::new(py)?.with_plugin_method(py.import("tools")?.getattr("_plugin_method")?);
    let module: String = cls.getattr("__module__")?.extract()?;
    let name: String = cls.getattr("__name__")?.extract()?;
    if module == "tools.walkthrough_host" && matches!(name.as_str(), "WalkHost" | "FirstPersonHost")
    {
        factory.bind_default(
            "Qt.NoModifier",
            &py.import("tools.walkthrough_host")?
                .getattr("Qt")?
                .getattr("NoModifier")?,
        )?;
    }
    let (methods, _) = registration(&module, &name).ok_or_else(|| {
        pyo3::exceptions::PyValueError::new_err(format!("Unknown tool adapter {module}.{name}"))
    })?;
    for &(name, mode, action, params, result, doc) in methods {
        factory.define(
            cls,
            mode,
            name,
            Endpoint::Adapter(action),
            params,
            result,
            doc,
        )?;
    }
    py.import("abc")?
        .call_method1("update_abstractmethods", (cls,))?;
    Ok(cls.clone())
}

mod move_functions;
mod offset_functions;
mod selection;
mod walk_functions;
#[pyfunction]
pub fn register_tool_functions(namespace: &Bound<'_, PyDict>) -> PyResult<()> {
    let factory = Registrar::new(namespace.py())?;
    let module: String = namespace
        .get_item("__name__")?
        .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("__name__"))?
        .extract()?;
    let methods = match module.as_str() {
        "tools.select_host" => selection::METHODS,
        "tools.offset_host" => offset_functions::METHODS,
        "tools.move_host" => move_functions::METHODS,
        "tools.walkthrough_host" => walk_functions::METHODS,
        _ => {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Unknown tool module {module}"
            )))
        }
    };
    if module == "tools.paint_host" {
        factory.bind_default(
            "RECENT_MAX",
            &namespace
                .get_item("RECENT_MAX")?
                .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("RECENT_MAX"))?,
        )?;
    }
    for &(name, mode, action, params, result, doc) in methods {
        factory.define(
            namespace.as_any(),
            mode,
            name,
            Endpoint::Adapter(action),
            params,
            result,
            doc,
        )?;
    }
    Ok(())
}

pub fn register_interaction(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(tool_preview_hooks, m)?)?;
    m.add_function(wrap_pyfunction!(register_tool_adapter, m)?)?;
    m.add_function(wrap_pyfunction!(register_tool_functions, m)?)?;
    Ok(())
}
