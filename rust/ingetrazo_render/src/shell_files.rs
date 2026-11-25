// SPDX-License-Identifier: GPL-3.0-or-later
//! Documents in the shell: open, save, close, recover, import, link, copy.
//! The submodule `ingetrazo_render.shell_files`.
//!
//! The shell binds document commands to the active view, the native file
//! panels and the model clipboard. Import, link and recovery also call the
//! document backends and the chrome's dialogs.
//!
//! Commands (`command`) come from the menu bar (`rust/built_app/src/menu.rs`),
//! the start page and the chrome's document ▾. The functions call each
//! other through the module, so a test replacing `open_path` or `save`
//! replaces it for the commands too.
use crate::settings_rows::{func, none, obj, opt, s, tr, trf};
use pyo3::{
    create_exception,
    exceptions::PyException,
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
use std::sync::atomic::{AtomicI64, Ordering};

type Any<'py> = Bound<'py, PyAny>;

create_exception!(shell_files, Abort, PyException, "An open the user cancelled.");

/// What `open_path` reads as an import; the rest are documents.
const IMPORTS: [&str; 7] = [".skp", ".dae", ".dxf", ".dwg", ".rfa", ".glb", ".gltf"];

/// kind → (panel title, filter).
const IMPORT_FILTERS: [(&str, &str, &str); 7] = [
    ("skp", "Import .skp", ".skp (*.skp)"),
    ("dae", "Import COLLADA", "COLLADA (*.dae)"),
    ("dxf", "Import DXF", "AutoCAD DXF (*.dxf)"),
    ("dwg", "Import DWG", "AutoCAD DWG (*.dwg)"),
    ("rfa", "Import Revit family", "Revit family (*.rfa)"),
    ("glb", "Import glTF/GLB", "glTF (*.glb *.gltf)"),
    ("igz", "Import a model as a group", "Built documents (*.built *.igz)"),
];

/// This submodule, through its attributes.
fn fm(py: Python<'_>) -> PyResult<Any<'_>> {
    py.import("ingetrazo_render")?.getattr("shell_files")
}

fn render(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("ingetrazo_render")
}

fn cd(py: Python<'_>) -> PyResult<Any<'_>> {
    render(py)?.getattr("chrome_dialogs")
}

fn path_of<'py>(py: Python<'py>, p: &Any<'py>) -> PyResult<Any<'py>> {
    py.import("pathlib")?.getattr("Path")?.call1((p,))
}

fn kw<'py>(py: Python<'py>, items: &[(&str, Any<'py>)]) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    for (k, v) in items {
        d.set_item(*k, v)?;
    }
    Ok(d)
}

fn b(py: Python<'_>, v: bool) -> Any<'_> {
    pyo3::types::PyBool::new(py, v).to_owned().into_any()
}

fn nil(py: Python<'_>) -> Any<'_> {
    py.None().into_bound(py)
}

fn truthy(v: &Any<'_>) -> bool {
    v.is_truthy().unwrap_or(false)
}

fn status(win: &Any<'_>, text: String) -> PyResult<()> {
    win.call_method0("statusBar")?.call_method1("showMessage", (text, 4000))?;
    Ok(())
}

fn file_dialogs(py: Python<'_>) -> PyResult<Any<'_>> {
    py.import("views.filedialogs")?.getattr("file_dialogs")
}

/// `host._error(where)` for `err`. The host formats the exception being
/// handled, so it is raised and caught around the call.
fn host_error(py: Python<'_>, win: &Any<'_>, err: &PyErr, place: &str) -> PyResult<()> {
    let code = c"def report(host, where, exc):\n    try:\n        raise exc\n    except BaseException:\n        host._error(where)\n";
    let helper = PyModule::from_code(py, code, c"shell_files_report.py", c"shell_files_report")?;
    helper.call_method1("report", (win.getattr("_host")?, place, err.value(py)))?;
    Ok(())
}

// ---- the file dialogs: NSOpenPanel / NSSavePanel ------------------------------------

/// `QFileDialog`'s static pickers, as `views.filedialogs.file_dialogs`
/// calls them, answered by the native panels (rfd).
#[pyclass(module = "ingetrazo_render.shell_files")]
struct ShellFileDialog;

fn split_dir(py: Python<'_>, directory: &Any<'_>) -> PyResult<(String, String)> {
    let directory = if truthy(directory) { directory.str()?.to_string() } else { String::new() };
    let p = path_of(py, &s(py, &directory))?;
    if !directory.is_empty() && !p.call_method0("is_dir")?.is_truthy()? && p.getattr("suffix")?.is_truthy()? {
        return Ok((p.getattr("parent")?.str()?.to_string(), p.getattr("name")?.str()?.to_string()));
    }
    Ok((directory, String::new()))
}

fn native_panels(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("app_shell.native")
}

fn text_or<'py>(py: Python<'py>, v: &Any<'py>, fallback: &str) -> Any<'py> {
    if truthy(v) { v.clone() } else { s(py, fallback) }
}

fn folder_or_none<'py>(py: Python<'py>, folder: &str) -> Any<'py> {
    if folder.is_empty() { nil(py) } else { s(py, folder) }
}

#[pymethods]
impl ShellFileDialog {
    #[new]
    fn new() -> Self {
        ShellFileDialog
    }

    #[staticmethod]
    #[pyo3(name = "_split")]
    fn split(py: Python<'_>, directory: &Any<'_>) -> PyResult<(String, String)> {
        split_dir(py, directory)
    }

    #[staticmethod]
    #[pyo3(signature = (parent=None, caption=None, dir=None, filter=None, *_args, **_kwargs))]
    #[allow(non_snake_case, clippy::too_many_arguments)]
    fn getOpenFileName<'py>(py: Python<'py>, parent: Option<Any<'py>>, caption: Option<Any<'py>>, dir: Option<Any<'py>>, filter: Option<Any<'py>>, _args: &Bound<'py, PyTuple>, _kwargs: Option<&Bound<'py, PyDict>>) -> PyResult<(Any<'py>, Any<'py>)> {
        let _ = parent;
        let (caption, dir, filter) = (caption.unwrap_or_else(|| s(py, "")), dir.unwrap_or_else(|| s(py, "")), filter.unwrap_or_else(|| s(py, "")));
        let (folder, _) = split_dir(py, &dir)?;
        let got = native_panels(py)?.call_method1("open_files", (text_or(py, &caption, "Open"), &filter, folder_or_none(py, &folder)))?;
        Ok((if truthy(&got) { got.get_item(0)? } else { s(py, "") }, filter))
    }

    #[staticmethod]
    #[pyo3(signature = (parent=None, caption=None, dir=None, filter=None, *_args, **_kwargs))]
    #[allow(non_snake_case, clippy::too_many_arguments)]
    fn getOpenFileNames<'py>(py: Python<'py>, parent: Option<Any<'py>>, caption: Option<Any<'py>>, dir: Option<Any<'py>>, filter: Option<Any<'py>>, _args: &Bound<'py, PyTuple>, _kwargs: Option<&Bound<'py, PyDict>>) -> PyResult<(Any<'py>, Any<'py>)> {
        let _ = parent;
        let (caption, dir, filter) = (caption.unwrap_or_else(|| s(py, "")), dir.unwrap_or_else(|| s(py, "")), filter.unwrap_or_else(|| s(py, "")));
        let (folder, _) = split_dir(py, &dir)?;
        let got = native_panels(py)?.getattr("open_files")?.call((text_or(py, &caption, "Open"), &filter, folder_or_none(py, &folder)), Some(&kw(py, &[("multiple", b(py, true))])?))?;
        Ok((got, filter))
    }

    #[staticmethod]
    #[pyo3(signature = (parent=None, caption=None, dir=None, filter=None, *_args, **_kwargs))]
    #[allow(non_snake_case, clippy::too_many_arguments)]
    fn getSaveFileName<'py>(py: Python<'py>, parent: Option<Any<'py>>, caption: Option<Any<'py>>, dir: Option<Any<'py>>, filter: Option<Any<'py>>, _args: &Bound<'py, PyTuple>, _kwargs: Option<&Bound<'py, PyDict>>) -> PyResult<(Any<'py>, Any<'py>)> {
        let _ = parent;
        let (caption, dir, filter) = (caption.unwrap_or_else(|| s(py, "")), dir.unwrap_or_else(|| s(py, "")), filter.unwrap_or_else(|| s(py, "")));
        let (folder, name) = split_dir(py, &dir)?;
        let got = native_panels(py)?.call_method1("save_file", (text_or(py, &caption, "Save"), name, &filter, folder_or_none(py, &folder)))?;
        Ok((if truthy(&got) { got } else { s(py, "") }, filter))
    }

    #[staticmethod]
    #[pyo3(signature = (parent=None, caption=None, dir=None, *_args, **_kwargs))]
    #[allow(non_snake_case)]
    fn getExistingDirectory<'py>(py: Python<'py>, parent: Option<Any<'py>>, caption: Option<Any<'py>>, dir: Option<Any<'py>>, _args: &Bound<'py, PyTuple>, _kwargs: Option<&Bound<'py, PyDict>>) -> PyResult<Any<'py>> {
        let _ = parent;
        let caption = caption.unwrap_or_else(|| s(py, ""));
        let folder = dir.filter(truthy).map(|d| d.str().map(|t| t.to_string())).transpose()?.unwrap_or_default();
        let got = native_panels(py)?.call_method1("pick_folder", (text_or(py, &caption, "Choose a folder"), folder_or_none(py, &folder)))?;
        Ok(if truthy(&got) { got } else { s(py, "") })
    }
}

/// Install the shell's bindings for document commands.
#[pyfunction]
fn install(py: Python<'_>, win: &Any<'_>) -> PyResult<()> {
    let _ = win;
    // The shell draws with the GPU by construction: no probe process, no
    // fallback to report (the start page's footer, the chrome's checks).
    let rs = py.import("core.renderer_settings")?;
    if !rs.getattr("_shell_installed").map(|v| truthy(&v)).unwrap_or(false) {
        let capability = rs.getattr("Capability")?.call1((true,))?.unbind();
        rs.setattr("capability", func(py, move |py, _| Ok(capability.clone_ref(py)))?)?;
        rs.setattr("status_text", func(py, |py, _| obj(py, "wgpu (native app)"))?)?;
        rs.setattr("_shell_installed", true)?;
    }
    Ok(())
}

// ---- commands -------------------------------------------------------------------------

/// A command by name (menu bar, start page, document ▾). Returns what the
/// command returns (open/save: whether it happened).
#[pyfunction]
#[pyo3(signature = (win, name, *args))]
fn command<'py>(py: Python<'py>, win: &Any<'py>, name: &str, args: &Bound<'py, PyTuple>) -> PyResult<Any<'py>> {
    let m = fm(py)?;
    let r = render(py)?;
    let first = || -> PyResult<Any<'py>> { if args.is_empty() { Ok(nil(py)) } else { args.get_item(0) } };
    match name {
        "about" | "extensions" | "cutlist" => return r.call_method1("show_report_sheet", (win, name)),
        "delivery_export" => return r.getattr("delivery_export_form")?.call((win,), Some(&kw(py, &[("one_off", b(py, true))])?)),
        "cut" | "copy" | "paste" | "paste_in_place" => return m.getattr("_clipboard")?.call_method1("run", (win, name)),
        _ => {}
    }
    let (kind, arg) = name.split_once(':').unwrap_or((name, ""));
    let docs = || win.getattr("_documents");
    match kind {
        "extension" => return win.getattr("_extension_menus")?.call_method1("trigger", (name,)),
        "import" if IMPORT_FILTERS.iter().any(|f| f.0 == arg) => return m.call_method1("import_dialog", (win, arg)),
        "recent" => return m.call_method1("open_recent_index", (win, arg)),
        "open_path" => return m.call_method1("open_path", (win, path_of(py, &s(py, arg))?)),
        // Scripted runs and harnesses: the same paths without the panels.
        "save_to" => return m.call_method1("do_save", (win, path_of(py, &s(py, arg))?)),
        "save_copy_to" => return m.getattr("do_save")?.call((win, path_of(py, &s(py, arg))?), Some(&kw(py, &[("copy", b(py, true))])?)),
        "export_image_to" => return win.call_method1("_export_image_to", (path_of(py, &s(py, arg))?,)),
        "export_igz_to" => return m.call_method1("export_igz", (win, path_of(py, &s(py, arg))?)),
        "sheet_export_to" => {
            // sheet_export_to:pdf:/path/file.pdf
            let (what, path) = arg.split_once(':').unwrap_or((arg, ""));
            let ws = py.import("views.wgpu_sheets")?.call_method1("workspace_of", (win.getattr("view")?,))?;
            return if ws.is_none() { Ok(nil(py)) } else { ws.call_method1("_export_to", (what, path)) };
        }
        "link_source" => return py.import("views.wgpu_tabs")?.call_method1("open_link_source", (win, path_of(py, &s(py, arg))?)),
        "ctx_demo" => return r.call_method1("context_menu_demo", (win, arg)),
        "close_doc" => {
            let doc = if arg.is_empty() { nil(py) } else { docs()?.get_item(py.import("builtins")?.getattr("int")?.call1((arg,))?)? };
            return m.call_method1("close_document", (win, doc));
        }
        "activate" => {
            let docs = docs()?;
            let i: i64 = py.import("builtins")?.getattr("int")?.call1((arg,))?.extract()?;
            if i >= 0 && (i as usize) < docs.len()? {
                win.call_method1("activate_document", (docs.get_item(i)?,))?;
            }
            return Ok(b(py, true));
        }
        _ => {}
    }
    match name {
        "new" => m.call_method1("new_model", (win,)),
        "new_window" => m.call_method1("new_window", (win,)),
        "move_to_window" => m.call_method1("move_to_new_window", (win, first()?)),
        "open" => m.call_method1("open_dialog", (win,)),
        "home" => m.call_method1("show_start", (win,)),
        "close_tab" => m.call_method1("close_document", (win, first()?)),
        "save" => m.call_method1("save", (win,)),
        "save_as" => m.call_method1("save_as", (win,)),
        "save_copy" => m.call_method1("save_copy", (win,)),
        "units" => win.call_method0("_on_units"),
        "check_updates" => win.call_method0("_on_check_updates"),
        "check_updates_quiet" => win.call_method0("check_updates_quietly"),
        "export_view_svg" => win.call_method0("_on_export_view_svg"),
        "export_image" => win.call_method0("_on_export_image"),
        "soften" => win.call_method0("_on_soften_edges"),
        "export_igz" => m.call_method1("export_igz", (win,)),
        "link" => m.call_method1("link_document", (win,)),
        "reload_links" => win.call_method0("_reload_all_links"),
        "prefs" => m.call_method1("open_settings", (win,)),
        "library" => m.call_method1("open_library", (win,)),
        "undo" => win.call_method0("_on_undo"),
        "redo" => win.call_method0("_on_redo"),
        "select_all" => win.call_method0("_on_select_all"),
        "select_none" => win.call_method0("_on_select_none"),
        "next_doc" => m.call_method1("step_document", (win, 1)),
        "prev_doc" => m.call_method1("step_document", (win, -1)),
        // The workspaces (views/wgpu_sheets.py): the top bar's workspace ▾.
        "sheets" => win.call_method0("_to_sheets"),
        "model" => win.call_method0("_show_model"),
        _ => {
            let out = r.call_method1("run_model_menu", (win, name))?;
            Ok(if truthy(&out) { out } else { nil(py) })
        }
    }
}

/// Window ▸ Library: the Library sheet.
#[pyfunction]
fn open_library<'py>(py: Python<'py>, win: &Any<'py>) -> PyResult<Any<'py>> {
    render(py)?.getattr("wgpu_library")?.call_method1("open_library", (win,))
}

#[pyfunction]
fn open_settings<'py>(py: Python<'py>, win: &Any<'py>) -> PyResult<Any<'py>> {
    render(py)?.getattr("settings_sheet")?.call_method1("show", (win,))
}

#[pyfunction]
fn step_document(win: &Any<'_>, step: i64) -> PyResult<()> {
    let nxt = win.getattr("_project_tabs")?.call_method1("neighbour", (win.getattr("_doc")?, step))?;
    if !nxt.is_none() {
        win.call_method1("activate_document", (nxt,))?;
    }
    Ok(())
}

/// File ▸ New: an untouched untitled project starts over, else a new
/// project; the start page goes.
#[pyfunction]
fn new_model<'py>(py: Python<'py>, win: &Any<'py>) -> PyResult<Any<'py>> {
    let start = render(py)?.getattr("wgpu_start")?;
    let workspace = win.getattr("_workspace")?;
    if !workspace.is_none() {
        if let Some(new) = opt(&workspace, "new") {
            new.call0()?;
        }
        return Ok(nil(py));
    }
    if start.call_method1("shown", (win,))?.is_truthy()? {
        return start.call_method1("new_model", (win,));
    }
    win.call_method0("_on_new")?;
    win.call_method0("_dismiss_home")?;
    Ok(nil(py))
}

/// File ▸ New Window (⇧⌘N): a window on the start page, in this process.
#[pyfunction]
fn new_window(py: Python<'_>, win: &Any<'_>) -> PyResult<bool> {
    py.import("app_shell.windows")?.call_method1("request", (win.getattr("_host")?, "new"))?;
    Ok(true)
}

/// Window ▸ Move Project to New Window, and the tab menu's row.
#[pyfunction]
#[pyo3(signature = (win, doc=None))]
fn move_to_new_window(py: Python<'_>, win: &Any<'_>, doc: Option<Any<'_>>) -> PyResult<bool> {
    let doc = match doc.filter(truthy) {
        Some(d) => d,
        None => win.getattr("_doc")?,
    };
    Ok(!py.import("app_shell.windows")?.call_method1("move_to_new_window", (win, doc))?.is_none())
}

#[pyfunction]
fn show_start(py: Python<'_>, win: &Any<'_>) -> PyResult<()> {
    render(py)?.getattr("wgpu_start")?.call_method1("show", (win,))?;
    Ok(())
}

// ---- open -----------------------------------------------------------------------------

#[pyfunction]
fn open_dialog<'py>(py: Python<'py>, win: &Any<'py>) -> PyResult<Any<'py>> {
    if let Some(opener) = opt(&win.getattr("_workspace")?, "open") {
        opener.call0()?;
        return Ok(b(py, true));
    }
    let filter: String = py.import("formats.built")?.getattr("DOCUMENT_FILTER")?.extract()?;
    win.getattr("viewport")?.call_method0("end_group_edit")?;
    let filter = format!("{filter};;Built projects (*.builtproject *.igzproj);;Imports (*.skp *.dae *.dxf *.dwg *.rfa *.glb *.gltf *.obj *.stl)");
    let path = file_dialogs(py)?.call_method1("getOpenFileName", (win, tr(py, "Open Built document")?, "", filter))?.get_item(0)?;
    if !truthy(&path) {
        return Ok(b(py, false));
    }
    fm(py)?.call_method1("open_path", (win, path_of(py, &path)?))
}

#[pyfunction]
fn open_recent_index<'py>(py: Python<'py>, win: &Any<'py>, arg: &str) -> PyResult<Any<'py>> {
    if arg == "clear" {
        py.import("core.recent")?.call_method0("clear")?;
        win.getattr("_host")?.call_method0("recent_changed")?;
        return Ok(b(py, true));
    }
    let paths = win.call_method0("_recent_paths")?;
    let Ok(i) = arg.trim().parse::<isize>() else { return Ok(b(py, false)) };
    let Ok(path) = paths.get_item(i) else { return Ok(b(py, false)) };
    render(py)?.getattr("wgpu_start")?.call_method1("open_recent", (win, path_of(py, &path)?))
}

/// Open `path`: a file already open is shown; an untouched untitled
/// project in front takes it, else a project of its own; imports go
/// through their pipelines into it.
#[pyfunction]
fn open_path<'py>(py: Python<'py>, win: &Any<'py>, path: &Any<'py>) -> PyResult<Any<'py>> {
    let m = fm(py)?;
    let path = path_of(py, path)?;
    let host = win.getattr("_host")?;
    let suffix: String = path.getattr("suffix")?.call_method0("lower")?.extract()?;
    if win.getattr("_closed")?.is_truthy()? {
        return Ok(b(py, false));
    }
    let opener = win.getattr("file_openers")?.call_method1("get", (&suffix,))?;
    if !opener.is_none() {
        return Ok(b(py, opener.call1((&path,))?.is_truthy()?));
    }
    if !win.getattr("_workspace")?.is_none() && !win.call_method0("leave_workspace")?.is_truthy()? {
        return Ok(b(py, false));
    }
    if suffix == ".builtproject" || suffix == ".igzproj" {
        // Built projects open here (app_shell/project.py); an .igzproj is
        // converted in place first.
        return py.import("app_shell.project")?.call_method1("open_project", (win, &path));
    }
    for doc in win.getattr("_documents")?.try_iter()? {
        let doc = doc?;
        if doc.call_method1("same_file", (&path,))?.is_truthy()? {
            win.call_method1("activate_document", (doc,))?;
            win.call_method0("_dismiss_home")?;
            return Ok(b(py, true));
        }
    }
    // A file opens only once: open in another window, that window and its
    // tab come to the front.
    let found = py.import("app_shell.windows")?.getattr("find_document")?.call((&path,), Some(&kw(py, &[("exclude", win.clone())])?))?;
    let (other, doc) = (found.get_item(0)?, found.get_item(1)?);
    if !other.is_none() {
        let window = other.getattr("window")?;
        window.call_method1("activate_document", (doc,))?;
        window.call_method0("_dismiss_home")?;
        other.setattr("focus_requested", true)?;
        other.call_method0("request_frame")?;
        return Ok(b(py, true));
    }
    if !path.call_method0("is_file")?.is_truthy()? {
        cd(py)?.call_method1("warning", (win, tr(py, "Open")?, trf(py, "{name} is no longer there.", &[("name", path.getattr("name")?)])?))?;
        return Ok(b(py, false));
    }
    let in_tab = |load: Py<PyAny>| -> PyResult<Any<'py>> { m.call_method1("_open_in_tab", (win, load)) };
    if suffix == ".obj" || suffix == ".stl" {
        // Unitless meshes need an answer before an Open creates a tab.
        let w = win.clone().unbind();
        let open_tab = func(py, move |py, a| Ok(fm(py)?.call_method1("_open_in_tab", (w.bind(py), a.get_item(0)?))?.unbind()))?;
        let got = win.getattr("_mesh_import_options")?.call((&path,), Some(&kw(py, &[("open_tab", open_tab.into_bound(py))])?))?;
        return Ok(b(py, got.eq("chrome")?));
    }
    let windowed = host.getattr("ns_view")?.is_truthy()?;
    let is_import = IMPORTS.contains(&suffix.as_str());
    let open = py.import("views.document_open");
    if suffix == ".skp" && windowed {
        let open = open?;
        return open.call_method1("start", (win, &path, open.getattr("install_skp")?));
    }
    if !is_import && windowed {
        // A Finder/CLI open can arrive before the first frame. Its camera
        // fit uses the initial aperture, before the dock and tool rail appear.
        let mut initial_frame = nil(py);
        if py.import("views.wgpu_ui")?.call_method1("free_area", (win,))?.is_none() && !host.getattr("_picture_seen")?.is_truthy()? {
            let view = win.getattr("view")?;
            initial_frame = PyTuple::new(py, [win.getattr("viewport")?.getattr("camera")?.getattr("aspect")?, view.call_method0("width")?, view.call_method0("height")?])?.into_any();
        }
        let install = py.import("functools")?.getattr("partial")?.call((m.getattr("_install_open")?,), Some(&kw(py, &[("initial_frame", initial_frame)])?))?;
        return open?.call_method1("start", (win, &path, install));
    }
    let (w, p) = (win.clone().unbind(), path.clone().unbind());
    let loader = if is_import { "import_path" } else { "open_document" };
    in_tab(func(py, move |py, _| Ok(fm(py)?.call_method1(loader, (w.bind(py), p.bind(py)))?.unbind()))?)
}

#[pyfunction]
#[pyo3(name = "_open_in_tab")]
fn open_in_tab<'py>(py: Python<'py>, win: &Any<'py>, load: &Any<'py>) -> PyResult<Any<'py>> {
    let previous = win.getattr("_doc")?;
    let mut created = nil(py);
    if !win.call_method0("_doc_is_blank")?.is_truthy()? {
        created = win.call_method0("new_document")?;
    }
    let ok = load.call0()?;
    let good = ok.is_truthy()?;
    if !good && !created.is_none() && win.getattr("_documents")?.contains(&created)? && win.call_method1("_doc_is_blank", (&created,))?.is_truthy()? {
        win.getattr("activate_document")?.call((previous,), Some(&kw(py, &[("closing", b(py, true))])?))?;
        win.call_method1("_retire_document", (&created,))?;
    }
    if good {
        win.call_method0("_dismiss_home")?;
    }
    win.getattr("_host")?.call_method0("request_frame")?;
    Ok(ok)
}

/// Read a .built/.igz into the project in front: recovery offer, the
/// progress capsule with Cancel, camera and views home, the resident pack
/// under the same capsule, links and the heap after the first picture.
#[pyfunction]
fn open_document<'py>(py: Python<'py>, win: &Any<'py>, path: &Any<'py>) -> PyResult<bool> {
    let m = fm(py)?;
    let dialogs = cd(py)?;
    let host = win.getattr("_host")?;
    let doc = win.getattr("_doc")?;
    let vp = doc.getattr("viewport")?;
    let view = doc.getattr("stand_in")?;
    let mut recovered = false;
    let slot = py.import("core.autosave")?.call_method1("pending", (path,))?;
    let defer_recovery = !slot.is_none() && !dialogs.call_method1("live", (win,))?.is_none();
    if !slot.is_none() && !defer_recovery {
        let got = render(py)?.call_method1("named_recovery", (win, path))?;
        if got.is_none() {
            return Ok(false);
        }
        recovered = got.is_truthy()?;
    }
    let made = win.getattr("_import_progress")?.call((trf(py, "Opening {name}…", &[("name", path.getattr("name")?)])?,), Some(&kw(py, &[("monotonic", b(py, true))])?))?;
    let (bar, cb) = (made.get_item(0)?, made.get_item(1)?);
    win.setattr("_open_cancelled", false)?;
    win.setattr("_open_aborted", false)?;
    if bar.is_instance(&dialogs.getattr("ChromeBar")?)? {
        bar.getattr("canceled")?.call_method1("connect", (win.getattr("_on_open_cancelled")?,))?;
        bar.call_method1("set_cancellable", (true,))?;
    }
    let (w, c) = (win.clone().unbind(), cb.clone().unbind());
    let guarded = func(py, move |py, a| {
        let win = w.bind(py);
        if win.getattr("_open_cancelled")?.is_truthy()? || win.getattr("_open_aborted")?.is_truthy()? {
            return Err(Abort::new_err(()));
        }
        let frac: f64 = a.get_item(0)?.extract()?;
        c.bind(py).call1((0.6 * frac, a.get_item(1)?))?;
        Ok(none(py))
    })?;
    win.setattr("_open_in_progress", true)?;
    win.setattr("_opening_document", true)?;
    if view.hasattr("blank")? && !view.getattr("renderer")?.is_none() {
        view.call_method0("blank")?;
    }
    let work = || -> PyResult<bool> {
        let source = if recovered { slot.clone() } else { path.clone() };
        let loaded = py.import("formats.igz")?.getattr("load_into")?.call((vp.getattr("scene")?, source), Some(&kw(py, &[("progress", guarded.into_bound(py))])?));
        match loaded {
            Ok(_) => {}
            Err(e) if e.is_instance_of::<Abort>(py) => {
                bar.call_method0("close")?;
                win.call_method0("_finish_cancelled_open")?;
                return Ok(false);
            }
            Err(e) if e.is_instance_of::<PyException>(py) => {
                // Any IO/parse error goes to the user.
                dialogs.call_method1("close_failed", (&bar, e.value(py)))?;
                host_error(py, win, &e, "open")?;
                dialogs.call_method1("critical", (win, tr(py, "Open failed")?, e.value(py).str()?))?;
                return Ok(false);
            }
            Err(e) => return Err(e),
        }
        m.call_method1("_finish_open", (win, path, recovered, if defer_recovery { slot.clone() } else { nil(py) }, &bar, &cb))?.is_truthy()
    };
    let out = work();
    win.setattr("_opening_document", false)?;
    win.setattr("_open_in_progress", false)?;
    bar.call_method0("close")?;
    host.call_method0("request_frame")?;
    out
}

#[pyfunction]
#[pyo3(name = "_install_open", signature = (win, path, bar, result, *, initial_frame=None))]
fn install_open<'py>(py: Python<'py>, win: &Any<'py>, path: &Any<'py>, bar: &Any<'py>, result: &Any<'py>, initial_frame: Option<Any<'py>>) -> PyResult<Any<'py>> {
    let (scene, packed, tracker, tree) = (result.get_item(0)?, result.get_item(1)?, result.get_item(2)?, result.get_item(3)?);
    let doc = win.getattr("_doc")?;
    doc.getattr("model_views")?.call_method1("suspend", (&doc,))?;
    let history = doc.getattr("viewport")?.getattr("history")?;
    history.setattr("scene", &scene)?;
    doc.getattr("viewport")?.call_method1("set_document", (&scene, &history))?;
    let mut initial_aspect = nil(py);
    if let Some(frame) = initial_frame.filter(truthy) {
        let view = win.getattr("view")?;
        if frame.get_item(1)?.eq(view.call_method0("width")?)? && frame.get_item(2)?.eq(view.call_method0("height")?)? {
            initial_aspect = frame.get_item(0)?;
        }
    }
    let slot = py.import("core.autosave")?.call_method1("pending", (path,))?;
    let prepared = PyTuple::new(py, [packed, tracker, tree])?;
    fm(py)?.call_method1("_finish_open", (win, path, false, slot, bar, bar.getattr("job")?.getattr("update")?, prepared, initial_aspect))
}

#[pyfunction]
#[pyo3(name = "_finish_open", signature = (win, path, recovered, slot, bar, cb, prepared=None, initial_aspect=None))]
#[allow(clippy::too_many_arguments)]
fn finish_open<'py>(py: Python<'py>, win: &Any<'py>, path: &Any<'py>, recovered: bool, slot: &Any<'py>, bar: &Any<'py>, cb: &Any<'py>, prepared: Option<Any<'py>>, initial_aspect: Option<Any<'py>>) -> PyResult<bool> {
    let dialogs = cd(py)?;
    let prepared = prepared.filter(|p| !p.is_none());
    let initial_aspect = initial_aspect.filter(|a| !a.is_none());
    let doc = win.getattr("_doc")?;
    doc.getattr("model_views")?.call_method1("restore", (&doc,))?;
    let (host, vp, view) = (win.getattr("_host")?, doc.getattr("viewport")?, win.getattr("view")?);
    py.import("gc")?.call_method1("set_threshold", (win.getattr("GC_THRESHOLD0")?, 10, 10))?;
    vp.getattr("history")?.call_method0("clear")?;
    if let Err(e) = vp.call_method0("reset_texture_cache") {
        if !e.is_instance_of::<PyException>(py) {
            return Err(e);
        }
        host_error(py, win, &e, "reset_texture_cache")?;
    }
    win.call_method0("_forget_coordinate")?;
    vp.getattr("inference")?.call_method0("reset")?;
    let scene = vp.getattr("scene")?;
    let home = scene.getattr("camera_home").unwrap_or_else(|_| nil(py));
    win.call_method1("_apply_camera_dict", (&home,))?;
    if !home.is_instance_of::<PyDict>() {
        match &initial_aspect {
            None => {
                win.call_method0("_on_zoom_extents")?;
            }
            Some(aspect) => {
                let bounds = win.call_method0("_extents_bounds")?;
                let area = py.import("views.wgpu_ui")?.call_method1("free_area", (win,))?;
                if !bounds.is_none() {
                    let camera = vp.getattr("camera")?;
                    let kept = camera.getattr("aspect")?;
                    camera.setattr("aspect", aspect)?;
                    let free = if truthy(&area) { PyTuple::new(py, [obj(py, -1.0)?.into_bound(py), area.get_item(1)?, obj(py, 1.0)?.into_bound(py), area.get_item(3)?])?.into_any() } else { nil(py) };
                    let fitted = camera.getattr("fit_box")?.call(bounds.cast::<PyTuple>()?.clone(), Some(&kw(py, &[("free", free)])?));
                    camera.setattr("aspect", kept)?;
                    fitted?;
                }
            }
        }
    }
    win.setattr("_current_path", path)?;
    if prepared.is_none() {
        win.call_method1("_remember_recent", (path,))?;
        host.call_method0("recent_changed")?;
    } else {
        let (w, d, p) = (win.clone().unbind(), doc.clone().unbind(), path.clone().unbind());
        let remember = func(py, move |py, _| {
            let (win, doc, path) = (w.bind(py), d.bind(py), p.bind(py));
            if win.call_method0("documents")?.contains(doc)? && doc.getattr("path")?.eq(path)? {
                win.getattr("_remember_recent")?.call((path,), Some(&kw(py, &[("doc", doc.clone())])?))?;
                win.getattr("_host")?.call_method0("recent_changed")?;
            }
            Ok(none(py))
        })?;
        host.call_method1("after_picture", (remember,))?;
    }
    win.setattr("_import_name", py.None())?;
    win.setattr("_autosaved_version", py.None())?;
    // A recovered drawing is not the file on disk yet: dirty until saved.
    if recovered {
        win.setattr("_saved_version", -1)?;
    } else {
        win.setattr("_saved_version", scene.getattr("version")?)?;
    }
    let repaired = scene.getattr("load_repairs").unwrap_or_else(|_| obj(py, 0).expect("int").into_bound(py));
    if truthy(&repaired) {
        win.setattr("_saved_version", -1)?;
        let text = trf(py, "{n} damaged pieces (a coordinate that is not a number) were left out so the rest of “{name}” could open. Save it to keep the repaired document.", &[("n", repaired), ("name", path.getattr("name")?)])?;
        dialogs.getattr("notify")?.call((win, tr(py, "Document repaired")?, text), Some(&kw(py, &[("tone", s(py, "warning"))])?))?;
    }
    vp.call_method0("notify_scene_changed")?;
    let clean = !win.call_method0("_is_dirty")?.is_truthy()?;
    // The pack and upload, under the same capsule.
    let version = scene.getattr("version")?;
    match &prepared {
        None => {
            let c = cb.clone().unbind();
            let progress = func(py, move |py, a| {
                let frac: f64 = a.get_item(0)?.extract()?;
                c.bind(py).call1((0.6 + 0.38 * frac, a.get_item(1)?))?;
                Ok(none(py))
            })?;
            view.getattr("set_scene")?.call((&scene,), Some(&kw(py, &[("progress", progress.into_bound(py)), ("opening", b(py, true))])?))?;
        }
        Some(p) => {
            view.getattr("set_scene")?.call((&scene,), Some(&kw(py, &[("opening", b(py, true)), ("prepared", p.clone())])?))?;
        }
    }
    view.setattr("_shell_resident_version", version)?;
    if clean {
        win.setattr("_saved_version", scene.getattr("version")?)?;
    }
    render(py)?.call_method1("defer_document_terrain", (&host, &vp))?;
    host.call_method1("after_picture", (win.getattr("_after_links_loaded")?,))?;
    if let Some(jobs) = opt(&view, "after_open").filter(truthy) {
        for job in jobs.try_iter()?.collect::<PyResult<Vec<_>>>()? {
            host.call_method1("after_picture", (job,))?;
        }
    }
    view.setattr("after_open", PyList::empty(py))?;
    if prepared.is_none() {
        host.call_method1("after_picture", (win.getattr("settle_heap")?,))?;
    }
    win.call_method0("_refresh_scene_tabs")?;
    win.call_method0("_update_title")?;
    vp.setattr("_pick_warm_due", true)?;
    cb.call1((1.0, "Document loaded"))?;
    dialogs.call_method1("opened", (bar, win, path))?;
    if !slot.is_none() {
        dialogs.call_method1("offer_recovery", (win, path, slot))?;
    }
    Ok(true)
}

// ---- the progress capsule -----------------------------------------------------------

/// `canceled` of a bar nothing shows: connects to nothing.
#[pyclass(module = "ingetrazo_render.shell_files")]
struct NullSignal;

#[pymethods]
impl NullSignal {
    #[pyo3(signature = (*_args))]
    fn connect(&self, _args: &Bound<'_, PyTuple>) {}
    #[pyo3(signature = (*_args))]
    fn disconnect(&self, _args: &Bound<'_, PyTuple>) {}
    #[pyo3(signature = (*_args))]
    fn emit(&self, _args: &Bound<'_, PyTuple>) {}
}

/// The ChromeBar surface when no chrome can show it (headless).
#[pyclass(module = "ingetrazo_render.shell_files", dict)]
struct NullBar {
    #[pyo3(get)]
    canceled: Py<NullSignal>,
}

#[pymethods]
#[allow(non_snake_case)]
impl NullBar {
    #[new]
    fn new(py: Python<'_>) -> PyResult<Self> {
        Ok(NullBar { canceled: Py::new(py, NullSignal)? })
    }
    #[pyo3(signature = (*_args))]
    fn setValue(&self, _args: &Bound<'_, PyTuple>) {}
    fn value(&self) -> i64 {
        0
    }
    #[pyo3(signature = (*_args))]
    fn setLabelText(&self, _args: &Bound<'_, PyTuple>) {}
    #[pyo3(signature = (*_args))]
    fn setRange(&self, _args: &Bound<'_, PyTuple>) {}
    #[pyo3(signature = (*_args))]
    fn setCancelButtonText(&self, _args: &Bound<'_, PyTuple>) {}
    #[pyo3(signature = (*_args))]
    fn set_cancellable(&self, _args: &Bound<'_, PyTuple>) {}
    fn wasCanceled(&self) -> bool {
        false
    }
    fn isVisible(&self) -> bool {
        false
    }
    fn reset(&self) {}
    fn hide(&self) {}
    fn close(&self) -> bool {
        true
    }
    fn deleteLater(&self) {}
}

/// The capsule in the top bar, whose callback turns the modal pump so it
/// moves and Cancel works: `(bar, callback(fraction, text))`.
#[pyfunction]
#[pyo3(signature = (win, title, *, monotonic=false))]
fn import_progress<'py>(py: Python<'py>, win: &Any<'py>, title: &Any<'py>, monotonic: bool) -> PyResult<(Any<'py>, Py<PyAny>)> {
    let dialogs = cd(py)?;
    if dialogs.call_method1("live", (win,))?.is_none() {
        return Ok((Bound::new(py, NullBar::new(py)?)?.into_any(), func(py, |py, _| Ok(none(py)))?));
    }
    let bar = dialogs.call_method1("ChromeBar", (win, title))?;
    let (held, last) = (bar.clone().unbind(), AtomicI64::new(0));
    let cb = func(py, move |py, a| {
        let frac: f64 = a.get_item(0)?.extract()?;
        let value = ((frac * 100.0) as i64).clamp(0, 100);
        let shown = if monotonic { last.fetch_max(value, Ordering::Relaxed).max(value) } else { value };
        last.store(shown, Ordering::Relaxed);
        let bar = held.bind(py);
        bar.call_method1("setValue", (shown,))?;
        bar.call_method1("setLabelText", (tr(py, &a.get_item(1)?.str()?.to_string())?,))?;
        py.import("app_shell.modal")?.call_method0("tick")?;
        Ok(none(py))
    })?;
    Ok((bar, cb))
}

/// Parse a mesh on a worker and deliver progress on the window thread:
/// `(payload, exception)`.
#[pyfunction]
#[pyo3(signature = (parser, path, cb, **options))]
fn parse_threaded<'py>(py: Python<'py>, parser: &Any<'py>, path: &Any<'py>, cb: &Any<'py>, options: Option<&Bound<'py, PyDict>>) -> PyResult<(Any<'py>, Any<'py>)> {
    let result = PyDict::new(py);
    let latest = py.import("collections")?.getattr("deque")?.call((), Some(&kw(py, &[("maxlen", obj(py, 1)?.into_bound(py))])?))?;
    let options = match options {
        Some(o) => o.copy()?,
        None => PyDict::new(py),
    };
    let l = latest.clone().unbind();
    let progress = func(py, move |py, a| {
        l.bind(py).call_method1("append", ((a.get_item(0)?, a.get_item(1)?),))?;
        Ok(none(py))
    })?;
    options.set_item("progress", progress)?;
    let (pa, pt, op, res) = (parser.clone().unbind(), path.clone().unbind(), options.unbind(), result.clone().unbind());
    let run = func(py, move |py, _| {
        let result = res.bind(py);
        match pa.bind(py).call((pt.bind(py),), Some(op.bind(py))) {
            Ok(payload) => {
                result.set_item("payload", payload)?;
                result.set_item("exc", py.None())?;
            }
            Err(e) if e.is_instance_of::<PyException>(py) => {
                // Reported to the caller.
                result.set_item("payload", py.None())?;
                result.set_item("exc", e.value(py))?;
            }
            Err(e) => return Err(e),
        }
        Ok(none(py))
    })?;
    let name = parser.getattr("__name__")?;
    let worker = py.import("threading")?.getattr("Thread")?.call((), Some(&kw(py, &[("target", run.into_bound(py)), ("name", name), ("daemon", b(py, true))])?))?;
    worker.call_method0("start")?;
    let (l, c, wk) = (latest.clone().unbind(), cb.clone().unbind(), worker.clone().unbind());
    let done = func(py, move |py, _| {
        let latest = l.bind(py);
        if latest.is_truthy()? {
            c.bind(py).call1(latest.call_method0("popleft")?.cast_into::<PyTuple>()?)?;
        }
        obj(py, !wk.bind(py).call_method0("is_alive")?.is_truthy()?)
    })?;
    if !py.import("app_shell.modal")?.call_method1("run_until", (done,))?.is_truthy()? {
        worker.call_method0("join")?;
    }
    while latest.is_truthy()? {
        cb.call1(latest.call_method0("popleft")?.cast_into::<PyTuple>()?)?;
    }
    Ok((result.call_method1("get", ("payload",))?, result.call_method1("get", ("exc",))?))
}

// ---- imports --------------------------------------------------------------------------

#[pyfunction]
fn import_dialog<'py>(py: Python<'py>, win: &Any<'py>, kind: &str) -> PyResult<Any<'py>> {
    let (title, filter) = IMPORT_FILTERS.iter().find(|f| f.0 == kind).map_or(("Import", "All files (*)"), |f| (f.1, f.2));
    win.getattr("viewport")?.call_method0("end_group_edit")?;
    let path = file_dialogs(py)?.call_method1("getOpenFileName", (win, tr(py, title)?, "", filter))?.get_item(0)?;
    if !truthy(&path) {
        return Ok(b(py, false));
    }
    let path = path_of(py, &path)?;
    if kind == "igz" {
        return Ok(b(py, win.call_method1("import_igz_path", (path,))?.is_truthy()?));
    }
    fm(py)?.call_method1("import_path", (win, path))
}

/// Run the selected importer against the active document.
#[pyfunction]
fn import_path(win: &Any<'_>, path: &Any<'_>) -> PyResult<bool> {
    let suffix: String = path.getattr("suffix")?.call_method0("lower")?.extract()?;
    let method = match suffix.as_str() {
        ".rfa" => "_import_rfa_path",
        ".dae" => "_import_dae_path",
        ".skp" => "import_skp_path",
        ".dxf" => "_import_dxf_path",
        ".dwg" => "_import_dwg_path",
        ".glb" | ".gltf" => "_import_glb_path",
        _ => {
            win.getattr("_host")?.call_method1("not_here", (format!("{} import", path.getattr("suffix")?.str()?),))?;
            return Ok(false);
        }
    };
    let ok = win.call_method1(method, (path,))?.is_truthy()?;
    win.getattr("_host")?.call_method0("request_frame")?;
    Ok(ok)
}

/// Choose a source file, then ask which of its contents to place.
#[pyfunction]
fn link_document(py: Python<'_>, win: &Any<'_>) -> PyResult<bool> {
    let filter = py.import("formats.built")?.getattr("DOCUMENT_FILTER")?;
    let path = file_dialogs(py)?.call_method1("getOpenFileName", (win, tr(py, "Link a document")?, "", filter))?.get_item(0)?;
    if !truthy(&path) {
        return Ok(false);
    }
    win.call_method1("_choose_link_contents", (path,))?;
    Ok(true)
}

// ---- save -----------------------------------------------------------------------------

#[pyfunction]
fn save<'py>(py: Python<'py>, win: &Any<'py>) -> PyResult<Any<'py>> {
    let workspace = win.getattr("_workspace")?;
    if !workspace.is_none() {
        let result = workspace.call_method0("save")?;
        win.call_method0("_update_title")?;
        return Ok(b(py, !result.is(&b(py, false))));
    }
    win.call_method0("_end_group_edits")?;
    let path = win.getattr("_current_path")?;
    if path.is_none() || !py.import("formats.built")?.call_method1("is_built", (&path,))?.is_truthy()? {
        return fm(py)?.call_method1("save_as", (win,));
    }
    fm(py)?.call_method1("do_save", (win, path_of(py, &path)?))
}

fn ask_save_path<'py>(py: Python<'py>, win: &Any<'py>, caption: String, default: &Any<'py>) -> PyResult<Option<Any<'py>>> {
    let built = py.import("formats.built")?;
    let chosen = file_dialogs(py)?.call_method1("getSaveFileName", (win, caption, default.str()?, built.getattr("BUILT_FILTER")?))?.get_item(0)?;
    if !truthy(&chosen) {
        return Ok(None);
    }
    let mut path = path_of(py, &chosen)?;
    if !built.call_method1("is_built", (&path,))?.is_truthy()? {
        path = path.call_method1("with_suffix", (built.getattr("BUILT_SUFFIX")?,))?;
    }
    Ok(Some(path))
}

#[pyfunction]
#[pyo3(name = "_ask_save_path")]
fn ask_save_path_py<'py>(py: Python<'py>, win: &Any<'py>, caption: String, default: &Any<'py>) -> PyResult<Any<'py>> {
    Ok(ask_save_path(py, win, caption, default)?.unwrap_or_else(|| nil(py)))
}

/// The document's file, else its display name, as a path.
fn base_path<'py>(py: Python<'py>, win: &Any<'py>) -> PyResult<Any<'py>> {
    let current = win.getattr("_current_path")?;
    if truthy(&current) { Ok(current) } else { path_of(py, &win.getattr("_doc")?.call_method0("display_name")?) }
}

#[pyfunction]
fn save_as<'py>(py: Python<'py>, win: &Any<'py>) -> PyResult<Any<'py>> {
    let workspace = win.getattr("_workspace")?;
    if !workspace.is_none() {
        let result = workspace.call_method0("save_as")?;
        win.call_method0("_update_title")?;
        return Ok(b(py, !result.is(&b(py, false))));
    }
    win.call_method0("_end_group_edits")?;
    let default = base_path(py, win)?.call_method1("with_suffix", (py.import("formats.built")?.getattr("BUILT_SUFFIX")?,))?;
    match ask_save_path(py, win, tr(py, "Save Built document")?, &default)? {
        None => Ok(b(py, false)),
        Some(path) => fm(py)?.call_method1("do_save", (win, path)),
    }
}

/// File ▸ Save a Copy…: the document as it is, to another .built; the
/// window keeps its own file, name and unsaved state.
#[pyfunction]
fn save_copy<'py>(py: Python<'py>, win: &Any<'py>) -> PyResult<Any<'py>> {
    win.call_method0("_end_group_edits")?;
    let base = base_path(py, win)?;
    let suffix: String = py.import("formats.built")?.getattr("BUILT_SUFFIX")?.extract()?;
    let default = base.call_method1("with_name", (format!("{} copy{suffix}", base.getattr("stem")?.str()?),))?;
    match ask_save_path(py, win, tr(py, "Save a Copy")?, &default)? {
        None => Ok(b(py, false)),
        Some(path) => fm(py)?.getattr("do_save")?.call((win, path), Some(&kw(py, &[("copy", b(py, true))])?)),
    }
}

/// The picture a save keeps in the file: one small offscreen frame of the
/// resident model.
#[pyfunction]
#[pyo3(name = "_picture_png")]
fn picture_png<'py>(py: Python<'py>, win: &Any<'py>, doc: &Any<'py>) -> PyResult<Any<'py>> {
    let Some(view) = opt(doc, "stand_in") else { return Ok(nil(py)) };
    if opt(&view, "renderer").is_none() || opt(&view, "packed").is_none() {
        return Ok(nil(py));
    }
    let made = view.call_method1("render_pixels", (win.getattr("SAVE_PICTURE_PX")?,)).and_then(|px| render(py)?.getattr("archive_picture_png")?.call1(px.cast_into::<PyTuple>()?));
    match made {
        Ok(png) => Ok(png),
        Err(e) if e.is_instance_of::<PyException>(py) => Ok(nil(py)), // a missing picture only
        Err(e) => Err(e),
    }
}

/// `progress(fraction, phase)` for a save under `job`.
fn save_progress<'py>(py: Python<'py>, job: &Any<'py>) -> PyResult<Any<'py>> {
    let j = job.clone().unbind();
    Ok(func(py, move |py, a| {
        j.bind(py).call_method1("update", (a.get_item(0)?, tr(py, &a.get_item(1)?.str()?.to_string())?))?;
        py.import("app_shell.modal")?.call_method0("tick")?;
        Ok(none(py))
    })?
    .into_bound(py))
}

/// Run `body(job)` inside `with activity:`.
fn with_busy<'py, T>(py: Python<'py>, activity: &Any<'py>, body: impl FnOnce(&Any<'py>) -> PyResult<T>) -> PyResult<T> {
    let job = activity.call_method0("__enter__")?;
    match body(&job) {
        Ok(v) => {
            activity.call_method1("__exit__", (py.None(), py.None(), py.None()))?;
            Ok(v)
        }
        Err(e) => {
            activity.call_method1("__exit__", (e.get_type(py), e.value(py), e.traceback(py)))?;
            Err(e)
        }
    }
}

/// Save the project's own scene and camera, links relative to where it
/// goes, its picture; the previous file kept as .bak by the save itself.
#[pyfunction]
#[pyo3(signature = (win, path, copy=false))]
fn do_save<'py>(py: Python<'py>, win: &Any<'py>, path: &Any<'py>, copy: bool) -> PyResult<bool> {
    let m = fm(py)?;
    let dialogs = cd(py)?;
    let autosave = py.import("core.autosave")?;
    let links = py.import("core.links")?;
    let project = py.import("app_shell.project")?;
    if !win.getattr("_workspace")?.is_none() {
        return Ok(false);
    }
    let path = path_of(py, path)?;
    let doc = win.getattr("_doc")?;
    let vp = doc.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let write = || -> PyResult<Any<'py>> {
        if !copy {
            project.call_method1("before_save", (win, &doc, &path))?;
        }
        scene.setattr("camera_home", win.call_method1("_camera_dict", (&vp,))?)?;
        scene.setattr("views_home", win.call_method1("_views_dict", (&doc,))?)?;
        links.call_method1("refresh_paths", (&scene, &path))?;
        let png = m.call_method1("_picture_png", (win, &doc))?;
        if !png.is_none() {
            scene.setattr("_home_thumbnail_png", png)?;
        }
        let activity = dialogs.getattr("busy")?.call((win, trf(py, "Saving {name}…", &[("name", path.getattr("name")?)])?), Some(&kw(py, &[("output_path", path.clone()), ("exclusive", b(py, true))])?))?;
        with_busy(py, &activity, |job| {
            let save = py.import("formats.igz")?.getattr("save_scene")?;
            let stats = if job.is_none() { save.call1((&scene, &path))? } else { save.call((&scene, &path), Some(&kw(py, &[("progress", save_progress(py, job)?)])?))? };
            Ok(if truthy(&stats) { stats } else { PyDict::new(py).into_any() })
        })
    };
    let written = write();
    if copy && !doc.getattr("path")?.is_none() {
        // The copy's links were made relative to it: back to the document's own.
        if let Err(e) = links.call_method1("refresh_paths", (&scene, path_of(py, &doc.getattr("path")?)?)) {
            if !e.is_instance_of::<PyException>(py) {
                return Err(e);
            }
        }
    }
    let stats = match written {
        Ok(s) => s,
        Err(e) if e.is_instance_of::<PyException>(py) => {
            host_error(py, win, &e, "save")?;
            let w = win.clone().unbind();
            let retry = func(py, move |py, _| Ok(fm(py)?.call_method1(if copy { "save_copy" } else { "save_as" }, (w.bind(py),))?.unbind()))?;
            dialogs.call_method1("save_failed", (win, e.value(py), retry))?;
            return Ok(false);
        }
        Err(e) => return Err(e),
    };
    let int = |key: &str| -> PyResult<i64> { py.import("builtins")?.getattr("int")?.call1((stats.call_method1("get", (key, 0))?,))?.extract() };
    let (missing, embedded) = (int("missing")?, int("embedded")?);
    let backup_failed = truthy(&stats.call_method1("get", ("backup_failed",))?);
    if missing != 0 {
        let text = trf(py, "{n} texture image(s) could not be read, so they were not packed into the document — those faces will lose their texture on another computer.", &[("n", obj(py, missing)?.into_bound(py))])?;
        dialogs.call_method1("warning", (win, tr(py, "Saved with missing textures")?, text))?;
    } else if backup_failed {
        status(win, tr(py, "Could not write the backup copy.")?)?;
    }
    let name = path.getattr("name")?;
    if copy {
        status(win, trf(py, "Saved a copy as {name}", &[("name", name)])?)?;
        return Ok(true);
    }
    // A clean save ends the recovery window for this document and for what
    // it was called until now (untitled, or the old name of a Save As).
    autosave.call_method1("clear", (&path,))?;
    autosave.getattr("clear")?.call((doc.getattr("path")?,), Some(&kw(py, &[("untitled", doc.getattr("untitled_no")?)])?))?;
    if missing == 0 && !backup_failed {
        let text = if embedded != 0 { trf(py, "Saved — {n} texture(s) packed into the document.", &[("n", obj(py, embedded)?.into_bound(py))])? } else { trf(py, "Saved {name}", &[("name", name)])? };
        status(win, text)?;
    }
    doc.setattr("path", &path)?;
    win.call_method1("_remember_recent", (&path,))?;
    win.getattr("_host")?.call_method0("recent_changed")?;
    win.setattr("_saved_version", scene.getattr("version")?)?;
    project.call_method1("saved", (win, &doc))?;
    win.call_method0("_update_title")?;
    Ok(true)
}

/// File ▸ Save a Copy as IngeTrazo (.igz): float32 geometry, after saying
/// how far that rounding moves points.
#[pyfunction]
#[pyo3(signature = (win, path=None))]
fn export_igz<'py>(py: Python<'py>, win: &Any<'py>, path: Option<Any<'py>>) -> PyResult<bool> {
    let m = fm(py)?;
    let dialogs = cd(py)?;
    win.call_method0("_end_group_edits")?;
    let doc = win.getattr("_doc")?;
    let path = match path.filter(|p| !p.is_none()) {
        Some(p) => p,
        None => {
            let own = doc.getattr("path")?;
            let base = if truthy(&own) { own } else { path_of(py, &doc.call_method0("display_name")?)? };
            let default = base.call_method1("with_suffix", (".igz",))?;
            let chosen = file_dialogs(py)?.call_method1("getSaveFileName", (win, tr(py, "Save a Copy as IngeTrazo")?, default.str()?, "IngeTrazo (*.igz)"))?.get_item(0)?;
            if !truthy(&chosen) {
                return Ok(false);
            }
            chosen
        }
    };
    let path = path_of(py, &path)?.call_method1("with_suffix", (".igz",))?;
    let w = win.clone().unbind();
    let confirm = func(py, move |py, a| {
        let dialogs = cd(py)?;
        let mm: f64 = a.get_item(0)?.extract()?;
        let mm = py.import("builtins")?.getattr("format")?.call1((mm, ".6g"))?;
        let text = trf(py, "Float32 rounding moves points by up to {mm} mm. Save the copy?", &[("mm", mm)])?;
        let got = dialogs.call_method1("question", (w.bind(py), tr(py, "Save the rounded copy?")?, text))?;
        obj(py, got.eq(dialogs.getattr("QMessageBox")?.getattr("Yes")?)?)
    })?
    .into_bound(py);
    let write = || -> PyResult<Any<'py>> {
        let viewport = doc.getattr("viewport")?;
        let scene = py.import("copy")?.call_method1("copy", (viewport.getattr("scene")?,))?;
        scene.setattr("camera_home", win.call_method1("_camera_dict", (&viewport,))?)?;
        scene.setattr("views_home", win.call_method1("_views_dict", (&doc,))?)?;
        let png = m.call_method1("_picture_png", (win, &doc))?;
        if !png.is_none() {
            scene.setattr("_home_thumbnail_png", png)?;
        }
        let activity = dialogs.getattr("busy")?.call((win, trf(py, "Saving {name}…", &[("name", path.getattr("name")?)])?), Some(&kw(py, &[("exclusive", b(py, true))])?))?;
        let stats = with_busy(py, &activity, |job| {
            let export = py.import("formats.igz")?.getattr("export_igz")?;
            let stats = if job.is_none() { export.call1((&scene, &path, &confirm))? } else { export.call((&scene, &path, &confirm), Some(&kw(py, &[("progress", save_progress(py, job)?)])?))? };
            if stats.is_none() && !job.is_none() {
                job.setattr("cancelled", true)?;
            } else if truthy(&stats) {
                activity.setattr("output_path", &path)?;
            }
            Ok(stats)
        })?;
        if truthy(&stats) && truthy(&stats.call_method1("get", ("missing",))?) {
            dialogs.call_method1("warning", (win, tr(py, "Saved with missing textures")?, tr(py, "Some texture images could not be packed.")?))?;
        }
        Ok(stats)
    };
    let stats = match write() {
        Ok(s) => s,
        Err(e) if e.is_instance_of::<PyException>(py) => {
            host_error(py, win, &e, "export igz")?;
            let w = win.clone().unbind();
            let retry = func(py, move |py, _| Ok(fm(py)?.call_method1("export_igz", (w.bind(py),))?.unbind()))?;
            dialogs.call_method1("save_failed", (win, e.value(py), retry))?;
            return Ok(false);
        }
        Err(e) => return Err(e),
    };
    if stats.is_none() {
        return Ok(false); // the precision question said no
    }
    status(win, trf(py, "Saved a copy as {name}", &[("name", path.getattr("name")?)])?)?;
    Ok(true)
}

// ---- close and quit -------------------------------------------------------------------

/// Close a project (⌘W, document ▾), asking about ITS unsaved changes. The
/// last one closed leaves an empty project behind the start page.
#[pyfunction]
#[pyo3(signature = (win, doc=None, ask=true))]
fn close_document<'py>(py: Python<'py>, win: &Any<'py>, doc: Option<Any<'py>>, ask: bool) -> PyResult<bool> {
    let doc = match doc.filter(truthy) {
        Some(d) => d,
        None => win.getattr("_doc")?,
    };
    let docs = win.getattr("_documents")?;
    if !docs.contains(&doc)? {
        return Ok(true);
    }
    let dialogs = cd(py)?;
    if !dialogs.call_method1("current_sheet", (win,))?.is_none() || dialogs.call_method1("modal", (win,))?.is_truthy()? {
        return Ok(false);
    }
    if !render(py)?.call_method1("confirm_close_tab", (win, &doc, ask))?.is_truthy()? {
        return Ok(false);
    }
    let last = docs.len()? == 1;
    if last {
        win.getattr("new_document")?.call((), Some(&kw(py, &[("activate", b(py, false))])?))?;
    }
    if doc.is(&win.getattr("_doc")?) {
        let docs = win.getattr("_documents")?;
        let i: isize = docs.call_method1("index", (&doc,))?.extract()?;
        let nxt = if (i + 1) < docs.len()? as isize { docs.get_item(i + 1)? } else { docs.get_item(i - 1)? };
        win.getattr("activate_document")?.call((nxt,), Some(&kw(py, &[("closing", b(py, true))])?))?;
    }
    win.call_method1("_retire_document", (&doc,))?;
    if let Err(e) = win.call_method0("_link_watcher").and_then(|w| w.call_method0("refresh")) {
        if !e.is_instance_of::<PyException>(py) {
            return Err(e);
        }
    }
    if last {
        win.call_method0("_start_blank_document")?;
        fm(py)?.call_method1("show_start", (win,))?;
    }
    win.call_method0("_update_title")?;
    Ok(true)
}

#[pyfunction]
fn sync_foreign_clipboard(py: Python<'_>, win: &Any<'_>) -> PyResult<()> {
    fm(py)?.getattr("_clipboard")?.call_method1("adopt", (win,))?;
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    let sub = PyModule::new(py, "shell_files")?;
    let clipboard = m.getattr("ModelClipboard")?.call0()?;
    sub.add("publish_clipboard", clipboard.getattr("publish")?)?;
    sub.add("_clipboard", clipboard)?;
    sub.add("cold_start", m.getattr("cold_start_document")?)?;
    sub.add("request_close", m.getattr("request_close")?)?;
    sub.add("IMPORTS", PyTuple::new(py, IMPORTS)?)?;
    let filters = PyDict::new(py);
    for (kind, title, filter) in IMPORT_FILTERS {
        filters.set_item(kind, (title, filter))?;
    }
    sub.add("_IMPORT_FILTERS", filters)?;
    sub.add("_Abort", py.get_type::<Abort>())?;
    sub.add_class::<ShellFileDialog>()?;
    sub.add_class::<NullBar>()?;
    for f in [
        wrap_pyfunction!(install, &sub)?,
        wrap_pyfunction!(command, &sub)?,
        wrap_pyfunction!(open_library, &sub)?,
        wrap_pyfunction!(open_settings, &sub)?,
        wrap_pyfunction!(step_document, &sub)?,
        wrap_pyfunction!(new_model, &sub)?,
        wrap_pyfunction!(new_window, &sub)?,
        wrap_pyfunction!(move_to_new_window, &sub)?,
        wrap_pyfunction!(show_start, &sub)?,
        wrap_pyfunction!(open_dialog, &sub)?,
        wrap_pyfunction!(open_recent_index, &sub)?,
        wrap_pyfunction!(open_path, &sub)?,
        wrap_pyfunction!(open_in_tab, &sub)?,
        wrap_pyfunction!(open_document, &sub)?,
        wrap_pyfunction!(install_open, &sub)?,
        wrap_pyfunction!(finish_open, &sub)?,
        wrap_pyfunction!(import_progress, &sub)?,
        wrap_pyfunction!(parse_threaded, &sub)?,
        wrap_pyfunction!(import_dialog, &sub)?,
        wrap_pyfunction!(import_path, &sub)?,
        wrap_pyfunction!(link_document, &sub)?,
        wrap_pyfunction!(save, &sub)?,
        wrap_pyfunction!(ask_save_path_py, &sub)?,
        wrap_pyfunction!(save_as, &sub)?,
        wrap_pyfunction!(save_copy, &sub)?,
        wrap_pyfunction!(picture_png, &sub)?,
        wrap_pyfunction!(do_save, &sub)?,
        wrap_pyfunction!(export_igz, &sub)?,
        wrap_pyfunction!(close_document, &sub)?,
        wrap_pyfunction!(sync_foreign_clipboard, &sub)?,
    ] {
        sub.add_function(f)?;
    }
    m.add_submodule(&sub)?;
    py.import("sys")?.getattr("modules")?.set_item("ingetrazo_render.shell_files", &sub)?;
    Ok(())
}
