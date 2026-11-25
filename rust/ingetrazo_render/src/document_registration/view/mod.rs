// SPDX-License-Identifier: GPL-3.0-or-later
//! Document host services installed on the Python-facing classes and modules.
use super::*;
use built_python_bindings::Adapter;
use pyo3::{
    types::{PyList, PySet},
    IntoPyObjectExt,
};
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
fn optional<'py>(o: &Obj<'py>, name: &str) -> Reply<'py> {
    Ok(o.getattr_opt(name)?.unwrap_or_else(|| none(o.py())))
}
fn attr_flag(obj: &Obj, field: &str, default: bool) -> PyResult<bool> {
    obj.getattr_opt(field)?
        .map_or(Ok(default), |value| value.is_truthy())
}
fn positional<'py>(o: &Obj<'py>) -> PyResult<Args<'py>> {
    Ok(o.py().get_type::<PyTuple>().call1((o,))?.cast_into()?)
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
macro_rules! fields {
    ($py:expr; $($key:expr => $value:expr),* $(,)?) => {{
        let fields=PyDict::new($py); $(fields.set_item($key,$value)?;)* fields
    }};
}
mod array_bake;
mod chrome_actions;
mod chrome_frame;
mod chrome_input;
mod chrome_inspector;
mod chrome_tree;
mod chrome_tree_action;
mod device;
mod frame_annotations;
mod frame_lines;
mod frame_marks;
mod frame_scene;
mod frame_tabs;
mod document_controller;
mod document_interaction;
mod document_tools;
mod metal_view;
mod metal_view_capture;
mod metal_view_frame;
mod metal_view_input;
mod metal_view_scene;
mod metal_view_selection;
mod modules;
mod interaction_host;
mod ndof_input;
mod texture_probe;
mod view_commands;
mod view_host;
mod view_navigation;
/// Call one of this lane's bindings from Rust: `args` is every parameter,
/// defaults included (nothing fills them in on this path). For the
/// functions no one outside their own module calls, which are therefore
/// not names on the Python module.
fn direct<'py, A>(py: Python<'py>, binding: fn(&Args<'py>) -> Reply<'py>, args: A) -> Reply<'py>
where
    A: IntoPyObject<'py, Target = PyTuple>,
{
    binding(&pyo3::BoundObject::into_bound(args.into_pyobject(py).map_err(Into::into)?))
}
/// This lane's functions that the rest of the crate calls: the Rust behind
/// what were names on `views.wgpu_ui` and `views.wgpu_tabs`.
pub(crate) mod calls {
    use super::*;

    /// Where the model is seen, in device coordinates; `None` without the
    /// chrome (`ui.rs` `free_ndc`).
    pub(crate) fn free_area<'py>(win: &Obj<'py>) -> Reply<'py> {
        direct(win.py(), chrome_actions::free_area, (win,))
    }
    /// Whether exclusive work holds the window, so a frame draws the chrome
    /// alone.
    pub(crate) fn chrome_only(view: &Obj<'_>) -> PyResult<bool> {
        direct(view.py(), chrome_frame::chrome_only, (view,))?.is_truthy()
    }
    /// Whether the chrome owns this key (a shortcut override).
    pub(crate) fn claims_key<'py>(view: &Obj<'py>, event: &Obj<'py>) -> Reply<'py> {
        direct(view.py(), chrome_input::claims_key, (view, event))
    }
    /// `(host document, placements)` of a tab whose file another open
    /// project links, else `(None, 0)`.
    pub(crate) fn link_host<'py>(win: &Obj<'py>, doc: &Obj<'py>) -> Reply<'py> {
        direct(win.py(), frame_tabs::link_host, (win, doc, win.py().None()))
    }
    /// Run a core protocol command on the viewport's history; a key outside
    /// the open context is told in the status line and answers `None`.
    pub(crate) fn core_command<'py>(viewport: &Obj<'py>, command: &str, args: Option<&Bound<'py, PyDict>>) -> Reply<'py> {
        chrome_tree_action::core_command(viewport, command.into_bound_py_any(viewport.py())?.as_any(), args)
    }
    /// The tree takes the next pick as where to file `count` items.
    pub(crate) fn file_to(win: &Obj<'_>, count: usize) -> PyResult<()> {
        direct(win.py(), chrome_inspector::file_to, (win, count)).map(drop)
    }
    /// `#rrggbb` of a colour, "" without one.
    pub(crate) fn hex<'py>(rgb: &Obj<'py>) -> Reply<'py> {
        direct(rgb.py(), chrome_tree::hex, (rgb,))
    }
    /// The bundled components' folder.
    pub(crate) fn components_dir(py: Python<'_>) -> Reply<'_> {
        direct(py, chrome_tree::components_dir, ())
    }
    /// The bundled set: `sc:<key>` 3D starters, `pp:<key>` 2D people.
    pub(crate) fn starter_components(py: Python<'_>) -> Reply<'_> {
        direct(py, chrome_tree::starter_components, ())
    }
    /// The name of the material Paint holds, or `None`.
    pub(crate) fn current_material_name(py: Python<'_>) -> Reply<'_> {
        direct(py, chrome_tree::current_material_name, ())
    }
}

fn install(owner: &Obj, namespace: &Bound<PyDict>, entries: &[Method]) -> PyResult<()> {
    let py = owner.py();
    let registrar = Bindings::new(py)?
        .with_plugin_method(py.import("tools")?.getattr("_plugin_method")?)
        .with_module_annotations(namespace)?;
    for &(name, mode, action, args, result, doc) in entries {
        registrar.define(
            owner,
            mode,
            name,
            Endpoint::Adapter(action),
            args,
            result,
            doc,
        )?;
    }
    Ok(())
}
#[pyfunction]
fn register_view_class(cls: &Obj) -> PyResult<()> {
    let module = cls.getattr("__module__")?.extract::<String>()?;
    let name = cls.getattr("__name__")?.extract::<String>()?;
    let methods = match (module.as_str(), name.as_str()) {
        ("views.document_tools", "DocumentTools") => document_tools::DOCUMENT_TOOLS,
        ("views.document_controller", "DocumentController") => {
            document_controller::DOCUMENT_CONTROLLER
        }
        ("views.view_navigation", "ViewNavigation") => {
            return view_navigation::register(cls, &cls.py().import(module)?)
        }
        ("views.view_host", "WidgetShim") => view_host::WIDGET_SHIM,
        ("views.wgpu_view", "MetalView") => {
            return metal_view::register(cls, &cls.py().import(module)?)
        }
        ("views.document_interaction", "DocumentInteraction") => {
            document_interaction::DOCUMENT_INTERACTION
        }
        ("views.ndof_input", "NdofInput") => ndof_input::NDOF_INPUT,
        _ => return Err(pyo3::exceptions::PyValueError::new_err((module, name))),
    };
    install(cls, &cls.py().import(module)?.dict(), methods)
}
#[pyfunction]
fn register_view_functions(namespace: &Bound<PyDict>) -> PyResult<()> {
    let name = namespace
        .get_item("__name__")?
        .unwrap()
        .extract::<String>()?;
    let methods = match name.as_str() {
        "views.document_tools" => document_tools::FUNCTIONS,
        "views.document_interaction" => document_interaction::FUNCTIONS,
        "views.view_commands" => view_commands::FUNCTIONS,
        "views.ndof_input" => ndof_input::FUNCTIONS,
        "views.wgpu_view" => metal_view::FUNCTIONS,
        _ => return Err(pyo3::exceptions::PyValueError::new_err(name)),
    };
    install(namespace.as_any(), namespace, methods)
}
pub(super) fn register(module: &Bound<PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(
        modules::install_interaction_module,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        texture_probe::measure_texture_position,
        module
    )?)?;

    Ok(())
}
