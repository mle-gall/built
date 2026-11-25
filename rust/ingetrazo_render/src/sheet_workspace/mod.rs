//! Document-bound host of the sheet workspace.
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

type Obj<'py> = Bound<'py, PyAny>;

#[pyclass(dict, weakref, subclass, module = "ingetrazo_render.sheet_workspace")]
pub struct SheetHost;

#[pymethods]
#[allow(non_snake_case)]
impl SheetHost {
    #[classattr]
    const EMBEDDED: bool = true;

    #[new]
    fn new(main_window: &Obj<'_>) -> Self {
        let _ = main_window;
        Self
    }

    fn __init__(slf: &Bound<'_, Self>, main_window: &Obj<'_>) -> PyResult<()> {
        let py = slf.py();
        let settings = py.import("ingetrazo_core.settings")?.call_method0("settings")?;
        let auto = settings
            .call_method1("value", ("composer/auto_render", "1"))?
            .str()?
            .to_str()?
            != "0";
        slf.setattr("_window", main_window)?;
        slf.setattr("comp", py.None())?;
        slf.setattr("tool_mode", "select")?;
        slf.setattr("_auto_render", auto)?;
        let workspace = py.get_type::<workspace::Workspace>().call1((slf,))?;
        slf.setattr("_ws", workspace)
    }

    fn setVisible(slf: &Bound<'_, Self>, on: bool) -> PyResult<()> {
        slf.getattr("_ws")?
            .call_method0(if on { "enter" } else { "leave" })?;
        Ok(())
    }
    fn show(slf: &Bound<'_, Self>) -> PyResult<()> {
        Self::setVisible(slf, true)
    }
    fn hide(slf: &Bound<'_, Self>) -> PyResult<()> {
        Self::setVisible(slf, false)
    }
    fn isVisible(slf: &Bound<'_, Self>) -> PyResult<bool> {
        slf.getattr("_ws")?.getattr("shown")?.is_truthy()
    }
    fn isMinimized(&self) -> bool {
        false
    }
    fn showMinimized(&self) {}
    fn raise_(&self) {}
    fn activateWindow(&self) {}
    fn set_document_title(&self, name: &Obj<'_>) {
        let _ = name;
    }
    fn _covers(&self, other: &Obj<'_>) -> bool {
        let _ = other;
        true
    }
    fn close(slf: &Bound<'_, Self>) -> PyResult<bool> {
        let ws = slf.getattr("_ws")?;
        ws.call_method0("leave")?;
        ws.call_method0("detach")?;
        Ok(true)
    }
    fn _show_model(slf: &Bound<'_, Self>) -> PyResult<()> {
        Self::hide(slf)?;
        Self::_refresh_sheet_tabs(slf)
    }
    fn _scene<'py>(slf: &Bound<'py, Self>) -> PyResult<Obj<'py>> {
        slf.getattr("_window")?
            .getattr("viewport")?
            .getattr("scene")
    }
    fn _sheets<'py>(slf: &Bound<'py, Self>) -> PyResult<Obj<'py>> {
        Self::_scene(slf)?.getattr("compositions")
    }
    fn _adopt_document(slf: &Bound<'_, Self>) -> PyResult<()> {
        let py = slf.py();
        let sheets = Self::_sheets(slf)?;
        if !sheets.is_truthy()? {
            let args = PyDict::new(py);
            args.set_item("n", 1)?;
            let name = py
                .import("ingetrazo_core.i18n")?
                .getattr("tr")?
                .call(("Sheet {n}",), Some(&args))?;
            let args = PyDict::new(py);
            args.set_item("name", name)?;
            let comp = py
                .import("core.composition")?
                .getattr("Composicion")?
                .call((), Some(&args))?;
            comp.getattr("frames")?
                .call_method1("append", (comp.call_method0("default_frame")?,))?;
            sheets.call_method1("append", (&comp,))?;
        }
        if !sheets.contains(slf.getattr("comp")?)? {
            slf.setattr("comp", sheets.get_item(0)?)?;
        }
        Ok(())
    }
    fn _on_comp_switched(slf: &Bound<'_, Self>, index: isize) -> PyResult<()> {
        let sheets = Self::_sheets(slf)?;
        if index >= 0 && (index as usize) < sheets.len()? {
            let comp = sheets.get_item(index)?;
            if !comp.is(slf.getattr("comp")?) {
                slf.setattr("comp", comp)?;
                slf.getattr("_ws")?.call_method0("sheet_changed")?;
            }
        }
        Self::_refresh_sheet_tabs(slf)
    }
    fn show_sheet(slf: &Bound<'_, Self>, index: isize) -> PyResult<()> {
        Self::_on_comp_switched(slf, index)?;
        Self::show(slf)
    }
    fn step_sheet(slf: &Bound<'_, Self>, step: isize) -> PyResult<()> {
        let sheets = Self::_sheets(slf)?;
        let comp = slf.getattr("comp")?;
        if sheets.contains(&comp)? {
            let index = sheets
                .call_method1("index", (comp,))?
                .extract::<isize>()?
                .saturating_add(step);
            if index >= 0 && (index as usize) < sheets.len()? {
                Self::show_sheet(slf, index)?;
            }
        }
        Ok(())
    }
    fn _reload_comp_combo(slf: &Bound<'_, Self>) -> PyResult<()> {
        slf.getattr("_ws")?.call_method0("sheet_changed")?;
        Self::_refresh_sheet_tabs(slf)
    }
    fn _refresh_sheet_tabs(slf: &Bound<'_, Self>) -> PyResult<()> {
        let sheets = Self::_sheets(slf)?;
        let comp = slf.getattr("comp")?;
        if sheets.contains(&comp)? {
            let win = slf.getattr("_window")?;
            win.getattr("_host_window")
                .unwrap_or(win)
                .setattr("_last_sheet", sheets.call_method1("index", (comp,))?)?;
        }
        Ok(())
    }
    fn _on_comp_add(slf: &Bound<'_, Self>) -> PyResult<()> {
        slf.getattr("_ws")?.call_method0("_add_sheet")?;
        Ok(())
    }
    fn _new_sheet_tab(slf: &Bound<'_, Self>) -> PyResult<()> {
        Self::_on_comp_add(slf)?;
        Self::show(slf)
    }
    fn _on_model_version(slf: &Bound<'_, Self>, version: &Obj<'_>) -> PyResult<()> {
        slf.getattr("_ws")?
            .call_method1("model_changed", (version,))?;
        Ok(())
    }
    fn frame_is_perspective(&self, frame: &Obj<'_>) -> PyResult<bool> {
        frame
            .getattr("perspective")
            .map_or(Ok(false), |v| v.is_truthy())
    }
    #[pyo3(signature = (path, sheets=None))]
    fn export_all_pdf<'py>(
        slf: &Bound<'py, Self>,
        path: &Obj<'py>,
        sheets: Option<&Obj<'py>>,
    ) -> PyResult<Obj<'py>> {
        let py = slf.py();
        let ws = slf.getattr("_ws")?;
        let view = ws.getattr("view")?;
        let view = if view.is_truthy()? {
            view
        } else {
            ws.call_method0("_front_view")?
        };
        let sheets = match sheets {
            Some(items) => items.clone(),
            None => Self::_sheets(slf)?,
        };
        let comps = PyList::empty(py);
        for comp in sheets.try_iter()? {
            comps.append(comp?)?;
        }
        let frames = py.import("ingetrazo_render.sheet_frames")?;
        let ground = frames.call_method1(
            "Ground",
            (wrap_pyfunction!(ignore_ground_update, py)?, &view),
        )?;
        let win = slf.getattr("_window")?;
        frames.call_method1(
            "export_pdf",
            (
                view,
                comps,
                path,
                ground,
                win.getattr("_current_path").ok(),
                win.getattr("_ext_sheet_annots").ok(),
            ),
        )
    }
}

mod hooks;
mod tools;
pub(crate) use tools::RAIL;
mod workspace;

pub fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = parent.py();
    let sub = PyModule::new(py, "sheet_workspace")?;
    sub.add_class::<SheetHost>()?;
    sub.add_class::<workspace::Workspace>()?;
    hooks::register(&sub)?;
    sub.add_function(wrap_pyfunction!(_cajetin_template, &sub)?)?;
    sub.add_function(wrap_pyfunction!(is_shell, &sub)?)?;
    sub.add_function(wrap_pyfunction!(embeds, &sub)?)?;
    sub.add("FORCE", py.None())?;
    let groups = PyList::empty(py);
    let names = PyDict::new(py);
    for group in tools::RAIL {
        let row = PyList::empty(py);
        for (key, label, _) in *group {
            row.append(*key)?;
            names.set_item(*key, *label)?;
        }
        groups.append(row)?;
    }
    sub.add("RAIL", groups)?;
    sub.add(
        "NATIVE_TOOLS",
        py.get_type::<pyo3::types::PySet>().call1((&names,))?,
    )?;
    sub.add("TOOL_NAMES", names)?;
    parent.add_submodule(&sub)?;
    py.import("sys")?
        .getattr("modules")?
        .set_item("ingetrazo_render.sheet_workspace", sub)
}

fn template_name(py: Python<'_>, setting: &str) -> PyResult<Option<String>> {
    let value = py
        .import("ingetrazo_core.settings")?
        .call_method0("settings")?
        .call_method1("value", (setting, ""))?;
    Ok(if value.is_truthy()? {
        Some(value.str()?.to_string())
    } else {
        None
    })
}
fn app_data(py: Python<'_>) -> PyResult<std::path::PathBuf> {
    py.import("ingetrazo_core.autosave")?
        .call_method0("_app_data_base")?
        .extract()
}
#[pyfunction]
fn _cajetin_template(py: Python<'_>) -> PyResult<(String, Option<String>)> {
    Ok((
        app_data(py)?
            .join("cajetines")
            .to_string_lossy()
            .into_owned(),
        template_name(py, "composer/default_cajetin")?,
    ))
}
fn sheet_template(py: Python<'_>) -> PyResult<Option<std::path::PathBuf>> {
    template_name(py, "composer/default_template")?
        .map(|n| Ok(app_data(py)?.join("plantillas").join(format!("{n}.json"))))
        .transpose()
}

#[pyfunction]
fn is_shell(win: &Obj<'_>) -> bool {
    !win.is_none()
}
#[pyfunction]
fn embeds(win: &Obj<'_>) -> PyResult<bool> {
    let force = win
        .py()
        .import("ingetrazo_render.sheet_workspace")?
        .getattr("FORCE")?;
    if force.is_none() {
        Ok(is_shell(win))
    } else {
        force.is_truthy()
    }
}

/// A standalone PDF export has no live page to invalidate.
#[pyfunction]
fn ignore_ground_update() {}
