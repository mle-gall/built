//! Render a saved-scene tour and publish the destination only on success.
use super::*;
use crate::settings_rows::{s, tr as translate, trf};
use pyo3::{create_exception, exceptions::PyRuntimeError, types::PyBool};
use std::path::{Path, PathBuf};

create_exception!(built_render, VideoError, PyRuntimeError);
const NO_SCENES: &str =
    "This project has no scenes to play: keep a view as a scene first (Scenes panel, «+ Scene»).";

fn message(py: Python, e: PyErr) -> PyErr {
    let text = e
        .value(py)
        .str()
        .map(|s| s.to_string())
        .unwrap_or_else(|_| e.to_string());
    let err = VideoError::new_err(text);
    err.set_cause(py, Some(e));
    err
}

#[pyfunction(signature=(window,path,width,height,fps,progress=None))]
pub(crate) fn export_video(
    window: &Obj,
    path: PathBuf,
    width: u32,
    height: u32,
    fps: u32,
    progress: Option<&Obj>,
) -> PyResult<usize> {
    let py = window.py();
    let core = py.import("built_core")?;
    let vp = window.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let views: Vec<Obj> = scene.getattr("saved_views")?.extract()?;
    if views.is_empty() {
        return Err(VideoError::new_err(translate(py, NO_SCENES)?));
    }
    window.getattr("scene_player")?.call_method0("cancel")?;
    let camera = vp.getattr("camera")?;
    let before = py
        .import("built_core.saved_views")?
        .getattr("SavedView")?
        .call_method1("capture", ("", &scene, &camera))?;
    let aspect = camera.getattr("aspect")?;
    let pose = core.getattr("Pose")?;
    let poses = views
        .iter()
        .map(|v| pose.call_method1("of_view", (v,)))
        .collect::<PyResult<Vec<_>>>()?;
    let anim = scene.getattr("animation")?;
    let transition = anim.getattr("transition")?;
    let pause = anim.getattr("pause")?;
    let total = core
        .call_method1("video_frame_count", (poses.len(), &transition, &pause, fps))?
        .extract::<usize>()?;
    let timeline = core.call_method1("VideoTimeline", (poses, transition, pause))?;
    let shown_state = core.getattr("saved_view_shown_state")?;
    let writer = core
        .call_method1("video_writer", (&path, width, height, fps))
        .map_err(|e| message(py, e))?;
    let render = || -> PyResult<usize> {
        let width: u32 = writer.getattr("width")?.extract()?;
        let height: u32 = writer.getattr("height")?.extract()?;
        let aspect = width as f64 / height as f64;
        let view_aspect = vp.call_method0("width")?.extract::<f64>()?.max(1.)
            / vp.call_method0("height")?.extract::<f64>()?.max(1.);
        let options = PyDict::new(py);
        options.set_item("overlays", (aspect - view_aspect).abs() < 0.01)?;
        options.set_item("annotations_only", true)?;
        let display = window.getattr("_doc")?.getattr("stand_in")?;
        let mut shown = None;
        for i in 0..total {
            let (pose, reached): (Obj, usize) = timeline
                .call_method1("at", (i as f64 / fps as f64,))?
                .extract()?;
            if shown != Some(reached) {
                let state = shown_state.call1((&scene,))?;
                views[reached].call_method1("apply_state", (&scene,))?;
                if shown_state.call1((&scene,))?.ne(state)? {
                    scene.call_method0("bump_view")?;
                }
                shown = Some(reached);
            }
            pose.call_method1("put_on", (&camera,))?;
            camera.setattr("aspect", aspect)?;
            let frame = display.call_method("render_pixels", (width, height), Some(&options))?;
            if frame.is_none()
                || !frame.get_item(0)?.eq(width)?
                || !frame.get_item(1)?.eq(height)?
            {
                return Err(VideoError::new_err(translate(
                    py,
                    "Could not render the view.",
                )?));
            }
            writer.call_method1("write", (frame.get_item(2)?,))?;
            if let Some(progress) = progress {
                if progress.call1((i + 1, total))?.is(&*PyBool::new(py, false)) {
                    return Ok(0);
                }
            }
        }
        writer.call_method0("close")?;
        writer.getattr("frames")?.extract()
    };
    let result = render();
    let discard = writer.call_method0("discard");
    let state = shown_state.call1((&scene,))?;
    before.call_method1("apply", (&scene, &camera))?;
    camera.setattr("aspect", aspect)?;
    if shown_state.call1((&scene,))?.ne(state)? {
        scene.call_method0("bump_view")?;
    }
    vp.call_method0("update")?;
    discard?;
    result.map_err(|e| {
        if e.is_instance_of::<VideoError>(py) {
            e
        } else {
            message(py, e)
        }
    })
}

#[pyfunction]
fn progress(
    window: &Obj,
    viewport: &Obj,
    bar: &Obj,
    update: &Obj,
    done: usize,
    total: usize,
) -> PyResult<bool> {
    update.call1((
        done as f64 / total as f64,
        translate(window.py(), "Rendering the video…")?,
    ))?;
    Ok(
        window.getattr("viewport")?.is(viewport)
            && !bar.call_method0("wasCanceled")?.is_truthy()?,
    )
}

#[pyfunction]
fn write(window: &Obj, viewport: &Obj, path: PathBuf, values: &Bound<PyDict>) -> PyResult<()> {
    if !window.getattr("viewport")?.is(viewport) {
        return Ok(());
    }
    let py = window.py();
    let (bar, update): (Obj, Obj) = window
        .call_method1(
            "_import_progress",
            (translate(py, "Rendering the video…")?,),
        )?
        .extract()?;
    let progress = py.import("functools")?.call_method1(
        "partial",
        (
            wrap_pyfunction!(progress, py)?,
            window,
            viewport,
            &bar,
            update,
        ),
    )?;
    let (width, height, fps) = (
        values.as_any().get_item("width")?.extract()?,
        values.as_any().get_item("height")?.extract()?,
        values.as_any().get_item("fps")?.extract()?,
    );
    let frames = match export_video(window, path.clone(), width, height, fps, Some(&progress)) {
        Ok(frames) => frames,
        Err(e) => {
            let dialogs = py.import("built_render.chrome_dialogs")?;
            dialogs.call_method1("close_failed", (&bar, e.value(py)))?;
            dialogs.call_method1(
                "critical",
                (
                    window,
                    translate(py, "Export Video failed")?,
                    e.value(py).str()?,
                ),
            )?;
            return Ok(());
        }
    };
    bar.call_method0("close")?;
    let note = if frames == 0 {
        translate(py, "Video export cancelled.")?
    } else {
        let text = translate(py, "Exported video {w}×{h}, {frames} frames → {name}")?;
        let kw = PyDict::new(py);
        kw.set_item("w", width)?;
        kw.set_item("h", height)?;
        kw.set_item("frames", frames)?;
        kw.set_item("name", path.file_name())?;
        s(py, &text)
            .call_method("format", (), Some(&kw))?
            .extract()?
    };
    window
        .call_method0("statusBar")?
        .call_method1("showMessage", (note, 6000))?;
    Ok(())
}
pub(crate) fn run_export(window: &Obj) -> PyResult<()> {
    let py = window.py();
    let dialogs = py.import("built_render.chrome_dialogs")?;
    let title = translate(py, "Export Video")?;
    let viewport = window.getattr("viewport")?;
    let scene = viewport.getattr("scene")?;
    let count = scene.getattr("saved_views")?.len()?;
    if count == 0 {
        dialogs.call_method1("information", (window, title, translate(py, NO_SCENES)?))?;
        return Ok(());
    }
    let current: Option<PathBuf> = window.getattr("_current_path")?.extract()?;
    let name = current
        .as_deref()
        .and_then(Path::file_stem)
        .and_then(|s| s.to_str())
        .unwrap_or("untitled");
    let core = py.import("built_core")?;
    let (suffix, label): (String, String) = core.call_method0("video_format")?.extract()?;
    let chosen: String = py
        .import(FILES)?
        .getattr("ShellFileDialog")?
        .call_method1(
            "getSaveFileName",
            (window, &title, format!("{name}{suffix}"), &label),
        )?
        .get_item(0)?
        .extract()?;
    if chosen.is_empty() {
        return Ok(());
    }
    let ext = suffix.trim_start_matches('.');
    let mut path = PathBuf::from(chosen);
    if !path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case(ext))
    {
        path.set_extension(ext);
        if path.exists() {
            let kw = PyDict::new(py);
            kw.set_item("default", "cancel")?;
            kw.set_item("cancel", "cancel")?;
            let prompt = trf(
                py,
                "Replace {name}?",
                &[(
                    "name",
                    s(py, &path.file_name().unwrap_or_default().to_string_lossy()),
                )],
            )?;
            let buttons = vec![
                ("cancel", translate(py, "Cancel")?),
                ("replace", translate(py, "Replace")?),
            ];
            if !dialogs
                .call_method("ask", (window, title, prompt, buttons), Some(&kw))?
                .eq("replace")?
            {
                return Ok(());
            }
        }
    }
    let animation = scene.getattr("animation")?;
    let seconds = core.call_method1(
        "video_seconds",
        (
            count,
            animation.getattr("transition")?,
            animation.getattr("pause")?,
        ),
    )?;
    let model = py.import(RENDER)?.getattr("OperationForm")?.call_method1(
        "video",
        (
            viewport.call_method0("width")?,
            viewport.call_method0("height")?,
            count,
            seconds,
            translate(py, "Like the view ({w} × {h})")?,
            translate(py, "{scenes} scenes, {seconds} s ({frames} frames)")?,
        ),
    )?;
    let cb = py.import("functools")?.call_method1(
        "partial",
        (wrap_pyfunction!(write, py)?, window, viewport, path),
    )?;
    let kw = PyDict::new(py);
    kw.set_item(
        "note",
        format!(
            "{label}\n{}",
            translate(
                py,
                "The times are those of View ▸ Animation ▸ Animation Settings…"
            )?
        ),
    )?;
    let forms = py.import("built_render.chrome_forms")?;
    let form = forms.call_method(
        "OperationForm",
        (translate(py, "Export Video")?, model, cb),
        Some(&kw),
    )?;
    forms.call_method1("show", (window, form))?;
    Ok(())
}
