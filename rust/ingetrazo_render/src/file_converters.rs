//! File commands whose reader or converter runs outside the document transaction.
use super::model_edit::translated;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

const DXF_HELP: &str = "To work on the drawing anyway: save it as DXF from your CAD program (or convert it with a free DWG → DXF converter such as ODA File Converter), then File ▸ Import ▸ AutoCAD DXF.";

fn label(win: &Bound<'_, PyAny>, text: &str) -> PyResult<String> {
    translated(win.py(), text, None)?.extract()
}

#[pyfunction]
fn import_dwg(win: &Bound<'_, PyAny>, path: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = win.py();
    let backend = py.import("ingetrazo_core.formats.dwg_bridge")?;
    let dialogs = py.import("ingetrazo_render.chrome_dialogs")?;
    let title = label(win, "Import DWG failed")?;
    let help = label(win, DXF_HELP)?;
    if !backend.call_method0("have_dwg_support")?.is_truthy()? {
        let detail = label(
            win,
            "The LibreDWG converter (dwg2dxf) is not available in this installation.",
        )?;
        dialogs.call_method1("critical", (win, title, format!("{detail}\n\n{help}")))?;
        return Ok(false);
    }
    let args = PyDict::new(py);
    args.set_item("name", path.getattr("name")?)?;
    let progress = win
        .call_method1(
            "_import_progress",
            (translated(py, "Importing {name}…", Some(&args))?,),
        )?
        .get_item(0)?;
    progress.call_method1("setRange", (0, 0))?;
    progress.call_method1("setLabelText", (label(win, "Converting DWG…")?,))?;
    progress.call_method1("setCancelButtonText", (label(win, "Cancel")?,))?;
    let converter = backend.call_method1("DwgConverter", (win.getattr("_pump_file_command")?,))?;
    progress
        .getattr("canceled")?
        .call_method1("connect", (converter.getattr("cancel")?,))?;
    let dxf = converter.call_method1("convert", (path,));
    progress
        .getattr("canceled")?
        .call_method1("disconnect", (converter.getattr("cancel")?,))?;
    let dxf = match dxf {
        Ok(path) => path,
        Err(err) => {
            let e = err.value(py);
            dialogs.call_method1("close_failed", (&progress, e))?;
            if !e.is_instance(&backend.getattr("DwgBridgeError")?)? {
                return Err(err);
            }
            let kind: String = e.getattr("kind")?.extract()?;
            if kind == "cancelled" {
                win.call_method0("statusBar")?
                    .call_method1("showMessage", (label(win, "DWG import cancelled.")?, 5000))?;
            } else {
                let detail = if kind == "timeout" {
                    label(win, "The DWG conversion took too long and was stopped.")?
                } else {
                    args.set_item("details", e.str()?)?;
                    format!(
                        "{}\n{}",
                        label(win, "The DWG converter could not convert this drawing.")?,
                        translated(py, "Details: {details}", Some(&args))?
                    )
                };
                dialogs.call_method1("critical", (win, title, format!("{detail}\n\n{help}")))?;
            }
            return Ok(false);
        }
    };
    let imported = (|| {
        progress.call_method0("close")?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("label", path)?;
        win.call_method("_import_dxf_path", (&dxf,), Some(&kwargs))?
            .is_truthy()
    })();
    backend.call_method1("discard_temp_dxf", (&dxf,))?;
    imported
}

#[pyfunction]
fn import_skp(win: &Bound<'_, PyAny>, path: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = win.py();
    let ui = py.import("ingetrazo_render.document_open")?;
    let host = win
        .getattr("__dict__")?
        .cast_into::<PyDict>()?
        .get_item("_host")?;
    let visible = match host {
        Some(host) => host.getattr("ns_view")?.is_truthy()?,
        None => win.call_method0("isVisible")?.is_truthy()?,
    };
    if visible {
        let kw = PyDict::new(py);
        kw.set_item("target", win.getattr("_doc")?)?;
        return ui
            .getattr("start")?
            .call((win, path, ui.getattr("install_insertion")?), Some(&kw))?
            .is_truthy();
    }
    let skp = py.import("ingetrazo_core.formats.skp")?;
    let dialogs = py.import("ingetrazo_render.chrome_dialogs")?;
    let name = path.getattr("name")?;
    let args = PyDict::new(py);
    args.set_item("name", &name)?;
    if skp.call_method1("can_handle", (path,))?.is_truthy()? {
        let (bar, cb): (Bound<'_, PyAny>, Bound<'_, PyAny>) = win
            .call_method1(
                "_import_progress",
                (translated(py, "Importing {name}…", Some(&args))?,),
            )?
            .extract()?;
        let (payload, exc): (Bound<'_, PyAny>, Bound<'_, PyAny>) = win
            .call_method1("_parse_skp_threaded", (path, &cb))?
            .extract()?;
        if payload.is_instance(&py.import("ingetrazo_core")?.getattr("SkpInsertion")?)? {
            let inserted = ui.call_method1("install_insertion", (win, path, &bar, &payload));
            bar.call_method0("close")?;
            return inserted?.is_truthy();
        }
        if !exc.is_none() {
            dialogs.call_method1("close_failed", (&bar, &exc))?;
            if !exc.is_instance(&skp.getattr("NeedsConverter")?)? {
                dialogs.call_method1(
                    "critical",
                    (win, label(win, "Import SKP failed")?, exc.str()?),
                )?;
                return Ok(false);
            }
        } else if !payload.is_none() {
            let kwargs = PyDict::new(py);
            kwargs.set_item("progress", &cb)?;
            kwargs.set_item("payload", &payload)?;
            let load = py
                .import("functools")?
                .getattr("partial")?
                .call((skp.getattr("apply_payload")?,), Some(&kwargs))?;
            let cmd = win.call_method1(
                "_run_import",
                (load, &bar, label(win, "Import SKP failed")?),
            )?;
            if cmd.is_none() {
                return Ok(false);
            }
            win.call_method1("_prepare_import_display", (cmd, &cb))?;
            bar.call_method0("close")?;
            win.getattr("viewport")?.call_method0("update")?;
            win.setattr("_import_name", name)?;
            win.call_method0("_update_title")?;
            let empty = payload.call_method1("get", ("empty",))?.is_truthy()?;
            let (caption, ms) = if empty {
                ("{name} has no geometry — nothing to import.", 8000)
            } else {
                win.call_method0("_on_zoom_extents")?;
                ("Imported {name}", 3000)
            };
            win.call_method0("statusBar")?
                .call_method1("showMessage", (translated(py, caption, Some(&args))?, ms))?;
            return Ok(true);
        } else {
            dialogs.call_method1(
                "close_failed",
                (&bar, label(win, "Unreadable .skp file")?),
            )?;
        }
    }
    dialogs.call_method1("warning",(win,label(win,"Import SKP")?,translated(py,"Built could not read {name} with its built-in .skp reader.\n\nOpen it in the program it came from and export it as COLLADA (.dae) or OBJ, then import that file here.",Some(&args))?))?;
    Ok(false)
}

#[pyfunction]
fn clear_texture_cache(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let backend = py.import("core.texture")?;
    let dialogs = py.import("ingetrazo_render.chrome_dialogs")?;
    let (count, size): (u64, u64) = backend.call_method0("texture_cache_stats")?.extract()?;
    let args = PyDict::new(py);
    args.set_item("path", backend.call_method0("texture_cache_root")?.str()?)?;
    if count == 0 {
        dialogs.call_method1(
            "information",
            (
                win,
                label(win, "Texture cache")?,
                translated(
                    py,
                    "The texture cache is already empty.\n\n{path}",
                    Some(&args),
                )?,
            ),
        )?;
        return Ok(());
    }
    args.set_item("count", count)?;
    args.set_item("mb", size as f64 / 1048576.)?;
    let buttons = PyList::new(py, [("no", label(win, "No")?), ("yes", label(win, "Yes")?)])?;
    let kw = PyDict::new(py);
    kw.set_item("default", "no")?;
    kw.set_item("cancel", "no")?;
    let msg = "Delete {count} image(s) ({mb:.1f} MB) from:\n{path}\n\nSaved .igz documents carry their own copy and rebuild it when opened. Faces textured by a .skp import that was never saved lose their texture until you import the .skp again.";
    if dialogs
        .getattr("ask")?
        .call(
            (
                win,
                label(win, "Clear the texture cache?")?,
                translated(py, msg, Some(&args))?,
                buttons,
            ),
            Some(&kw),
        )?
        .eq("yes")?
    {
        args.set_item("count", backend.call_method0("clear_texture_cache")?)?;
        win.call_method0("statusBar")?.call_method1(
            "showMessage",
            (
                translated(py, "Texture cache cleared ({count} files).", Some(&args))?,
                4000,
            ),
        )?;
    }
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(import_dwg, m)?)?;
    m.add_function(wrap_pyfunction!(import_skp, m)?)?;
    m.add_function(wrap_pyfunction!(clear_texture_cache, m)?)?;
    Ok(())
}
