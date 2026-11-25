// SPDX-License-Identifier: GPL-3.0-or-later
//! Host effects surrounding a committed document transaction.
use super::{dispatch::in_frame, DocumentHistory};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
type Obj<'py> = Bound<'py, PyAny>;

pub(crate) struct TagTargets<'py> {
    pub before: Obj<'py>,
    pub include_groups: bool,
}
pub(crate) fn objects_before<'py>(
    scene: &Obj<'py>,
    include_groups: bool,
) -> PyResult<TagTargets<'py>> {
    let py = scene.py();
    let layers = py.import("core.layers")?;
    let before = if layers
        .call_method1("active_layer", (scene,))?
        .eq(layers.getattr("DEFAULT_LAYER")?)?
    {
        layers.call_method1("auto_ids", (scene,))?
    } else {
        let kw = PyDict::new(py);
        kw.set_item("include_groups", include_groups)?;
        layers.call_method("object_ids", (scene,), Some(&kw))?
    };
    Ok(TagTargets {
        before,
        include_groups,
    })
}
impl TagTargets<'_> {
    pub(crate) fn apply(&self, scene: &Obj<'_>) -> PyResult<()> {
        if self.before.is_none() {
            return Ok(());
        }
        let py = scene.py();
        let tagged = PyList::empty(py);
        let kw = PyDict::new(py);
        kw.set_item("include_groups", self.include_groups)?;
        kw.set_item("tagged_groups", &tagged)?;
        if py
            .import("core.layers")?
            .call_method("tag_new_objects", (scene, &self.before), Some(&kw))?
            .is_truthy()?
        {
            for g in py
                .import("core.purge")?
                .call_method1("iter_groups", (tagged,))?
                .try_iter()?
            {
                let mesh = g?.getattr("mesh")?;
                mesh.setattr("_chunk_dirty", true)?;
                mesh.setattr("_attrs_dirty", true)?;
            }
            super::super::steps::version(scene)?;
        }
        Ok(())
    }
}
pub(crate) fn follow_text(host: &Obj<'_>) -> PyResult<()> {
    let scene = host.getattr("scene")?;
    let Some(anchors) = scene.getattr_opt("text_placement_anchors")? else {
        return Ok(());
    };
    if anchors.is_truthy()? {
        let frame = host
            .py()
            .import("core.coordinate_frame")?
            .call_method0("model_frame")?;
        in_frame(&frame, || {
            for anchor in anchors.try_iter()? {
                let a = anchor?;
                if a.call_method1("follow", (&scene,))?.is_truthy()? {
                    host.getattr("_moved_signs")?
                        .call_method1("append", (a.getattr("path")?.get_item(-1)?,))?;
                }
            }
            Ok(())
        })?;
    }
    Ok(())
}
fn content_version<'py>(scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    Ok(scene
        .getattr_opt("content_version")?
        .unwrap_or_else(|| scene.py().None().into_bound(scene.py())))
}
fn dirty(host: &Obj<'_>) -> PyResult<()> {
    let txn = host.getattr("last_txn")?;
    if txn.is_none() {
        return Ok(());
    }
    let txn = txn.get_item(0)?;
    if !txn.is_none() {
        let fields = PyList::new(host.py(), ["layer", "hidden", "material"])?;
        let groups = txn.call_method1("placements", (fields,))?;
        if groups.is_truthy()? {
            crate::doc::history_support::dirty_placements(&groups)?;
        }
    }
    Ok(())
}

pub(super) fn execute<'py>(host: &Obj<'py>, cmd: &Obj<'py>, nested: bool) -> PyResult<Obj<'py>> {
    let start = std::time::Instant::now();
    let py = host.py();
    let scene = host.getattr("scene")?;
    host.setattr("last_rejected", py.None())?;
    let before = content_version(&scene)?;
    let tags = objects_before(&scene, !cmd.getattr("keeps_rows")?.is_truthy()?)?;
    host.setattr("last_txn", py.None())?;
    let api = py.import("core.history")?;
    if let Err(e) = super::super::execute::execute(py, host, cmd, Some(&tags)) {
        if !e.is_instance_of::<pyo3::exceptions::PyException>(py) {
            return Err(e);
        }
        super::super::steps::version(&scene)?;
        if e.value(py).is_instance(&api.getattr("CommandRejected")?)? {
            host.setattr("last_error", py.None())?;
            host.setattr("last_rejected", e.value(py).str()?)?;
        } else {
            scene.getattr("selection")?.call_method0("clear")?;
            host.setattr(
                "last_error",
                format!("{}: {}", cmd.get_type().getattr("__name__")?, e.value(py)),
            )?;
            log_failure(host, cmd, &e)?;
        }
        return Ok(py.None().into_bound(py));
    }
    dirty(host)?;
    host.setattr("last_error", py.None())?;
    let elapsed = start.elapsed().as_secs_f64() * 1000.;
    if elapsed >= 100. && perf_logging() {
        log_timing(cmd, elapsed)?;
    }
    cmd.setattr("_history_mesh", scene.getattr("mesh")?)?;
    if !scene.getattr("edit_group")?.is_none() {
        let levels = scene
            .getattr("_edit_stack")?
            .try_iter()?
            .map(|v| v?.get_item("session"))
            .collect::<PyResult<Vec<_>>>()?;
        cmd.setattr("_occurrences", PyTuple::new(py, levels)?)?;
    }
    keep_geometry_caches(host, cmd, &before)?;
    if !nested {
        let limit = host.getattr("max_steps")?;
        let cap = if limit.is_truthy()? {
            py.import("builtins")?.call_method1("int", (limit,))?
        } else {
            0usize.into_pyobject(py)?.into_any()
        };
        let cap = if cap.lt(0)? {
            0usize.into_pyobject(py)?.into_any()
        } else {
            cap
        };
        host.getattr("_native")?.call_method1("push", (cmd, cap))?;
        DocumentHistory::refresh_dimensions(&scene)?;
    }
    Ok(py.None().into_bound(py))
}

pub(super) fn replay<'py>(host: &Obj<'py>, cmd: &Obj<'py>, undo: bool) -> PyResult<Obj<'py>> {
    let py = host.py();
    let scene = host.getattr("scene")?;
    let before = content_version(&scene)?;
    DocumentHistory::replay(host.getattr("_native")?.extract()?, py, host, undo, false)?;
    dirty(host)?;
    keep_geometry_caches(host, cmd, &before)?;
    DocumentHistory::refresh_dimensions(&scene)?;
    Ok(true.into_pyobject(py)?.to_owned().into_any())
}

fn keep_geometry_caches(host: &Obj<'_>, cmd: &Obj<'_>, before: &Obj<'_>) -> PyResult<()> {
    let py = host.py();
    let api = py.import("core.history")?;
    let kinds = PyTuple::new(
        py,
        [
            "SnapshotMutation",
            "SnapshotCompound",
            "AddEdgeCommand",
            "AddFaceCommand",
            "AddDimensionCommand",
            "AddTextLabelCommand",
            "AddGuideCommand",
        ]
        .iter()
        .map(|k| api.getattr(*k))
        .collect::<PyResult<Vec<_>>>()?,
    )?;
    if !cmd.is_instance(&kinds)? {
        return Ok(());
    }
    let scene = host.getattr("scene")?;
    let mesh = scene.getattr("mesh")?;
    let cv = content_version(&scene)?;
    let scene_id = scene.as_ptr() as usize;
    let mesh_id = mesh.as_ptr() as usize;
    if let Some(rest) = host.getattr_opt("layer_count_rest")? {
        if rest.is_truthy()?
            && rest.get_item(0)?.eq((scene_id, before, mesh_id))?
            && cmd
                .getattr_opt("_history_mesh")?
                .is_some_and(|m| m.is(&mesh))
            && (!cmd.is_instance(&api.getattr("SnapshotMutation")?)?
                || cmd.call_method1("_target", (&scene,))?.is(&mesh))
        {
            host.setattr(
                "layer_count_rest",
                ((scene_id, &cv, mesh_id), rest.get_item(1)?),
            )?;
        }
    }
    let anchors = scene
        .getattr("__dict__")?
        .call_method1("get", ("_anchor_placements",))?;
    if anchors.is_truthy()? {
        let n = scene.getattr("groups")?.len()?;
        let edit = scene.getattr("edit_group")?.as_ptr() as usize;
        if anchors.get_item(0)?.eq((before, n, edit, mesh_id))? {
            scene.setattr(
                "_anchor_placements",
                ((cv, n, edit, mesh_id), anchors.get_item(1)?),
            )?;
        }
    }
    Ok(())
}

fn perf_logging() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("INGETRAZO_PERF").is_some_and(|s| !s.is_empty()))
}

fn log_timing(cmd: &Obj<'_>, elapsed: f64) -> PyResult<()> {
    use std::{fs::OpenOptions, io::Write, path::PathBuf};
    let py = cmd.py();
    let path = if let Some(path) = std::env::var_os("INGETRAZO_PERF_LOG").filter(|v| !v.is_empty())
    {
        PathBuf::from(path)
    } else {
        py.import("pathlib")?
            .getattr("Path")?
            .call_method0("home")?
            .extract::<PathBuf>()?
            .join("ingetrazo-perf.log")
    };
    let time = py.import("time")?.call_method1("strftime", ("%H:%M:%S",))?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(
        file,
        "{} [{}] command {:.0}ms {}",
        time,
        std::process::id(),
        elapsed,
        cmd.get_type().getattr("__name__")?
    )?;
    Ok(())
}

/// The traceback must include the original Python frames even though the
/// command error has already crossed the extension boundary.
fn log_failure(host: &Obj<'_>, cmd: &Obj<'_>, err: &PyErr) -> PyResult<()> {
    use std::{fs::OpenOptions, io::Write, path::PathBuf};
    let py = host.py();
    let date = py
        .import("datetime")?
        .getattr("datetime")?
        .call_method0("now")?;
    let kw = PyDict::new(py);
    kw.set_item("timespec", "seconds")?;
    let stamp = date.call_method("isoformat", (), Some(&kw))?;
    let lines = py
        .import("traceback")?
        .call_method1("format_exception", (err.value(py),))?;
    let traceback = lines.extract::<Vec<String>>()?.concat();
    let text = format!(
        "[{stamp}] command {} failed and was rolled back:\n{traceback}\n",
        cmd.get_type().getattr("__name__")?
    );
    let stderr = py.import("sys")?.getattr("stderr")?;
    if !stderr.is_none() {
        stderr.call_method1("write", (format!("{text}\n"),))?;
    }
    let mut path = host.getattr("error_log")?;
    if path.eq(py
        .import("core.history")?
        .getattr("History")?
        .getattr("error_log")?)?
    {
        match host.call_method0("default_error_log") {
            Ok(resolved) => path = resolved,
            Err(e) if e.is_instance_of::<pyo3::exceptions::PyException>(py) => (),
            Err(e) => return Err(e),
        }
    }
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path.extract::<PathBuf>()?)
    {
        let _ = file.write_all(text.as_bytes());
    }
    Ok(())
}
