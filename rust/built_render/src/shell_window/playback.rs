//! Per-window ownership of the native camera timeline and frame timer.
use super::*;
use crate::settings_rows;

#[pyclass(dict, module = "built_render")]
pub(super) struct WindowPlayback;

fn pose<'p>(camera: &Obj<'p>) -> PyResult<Obj<'p>> {
    camera
        .py()
        .import("built_core")?
        .getattr("Pose")?
        .call_method1("of_camera", (camera,))
}
fn view_pose<'p>(view: &Obj<'p>) -> PyResult<Obj<'p>> {
    view.py()
        .import("built_core")?
        .getattr("Pose")?
        .call_method1("of_view", (view,))
}

impl WindowPlayback {
    pub(super) fn create<'p>(window: &Obj<'p>, timers: &Obj<'p>) -> PyResult<Bound<'p, Self>> {
        let py = window.py();
        let player = Bound::new(py, Self)?;
        player.setattr("_window", window)?;
        player.setattr("_clock", py.import("time")?.getattr("monotonic")?)?;
        player.setattr(
            "timeline",
            py.import("built_core")?.call_method0("ScenePlayback")?,
        )?;
        player.setattr("_display", py.None())?;
        let kw = PyDict::new(py);
        kw.set_item("interval", 16)?;
        kw.set_item("repeat", true)?;
        player.setattr(
            "_timer",
            timers.call_method("timer", (player.getattr("tick")?,), Some(&kw))?,
        )?;
        Ok(player)
    }
    fn begin(slf: &Bound<Self>, vp: &Obj, views: Vec<Obj>, arrive: &Obj) -> PyResult<()> {
        slf.setattr("_viewport", vp)?;
        slf.setattr("_views", views)?;
        slf.setattr("_arrive", arrive)?;
        let doc = optional(&slf.getattr("_window")?, "_doc")?;
        let display = optional(&doc, "stand_in")?;
        slf.setattr(
            "_display",
            if optional(&display, "source_viewport")?.is(vp) {
                display
            } else {
                none(slf.py())
            },
        )?;
        slf.getattr("_timer")?.call_method0("start")?;
        Ok(())
    }
    fn end(slf: &Bound<Self>) -> PyResult<()> {
        slf.getattr("_timer")?.call_method0("stop")?;
        for key in ["_display", "_arrive", "_viewport", "_views"] {
            slf.setattr(key, slf.py().None())?;
        }
        Ok(())
    }
}
#[pymethods]
impl WindowPlayback {
    fn is_active(slf: &Bound<Self>) -> PyResult<bool> {
        slf.getattr("timeline")?
            .call_method0("is_active")?
            .is_truthy()
    }
    fn is_playing(slf: &Bound<Self>) -> PyResult<bool> {
        slf.getattr("timeline")?
            .call_method0("is_playing")?
            .is_truthy()
    }
    fn cancel(slf: &Bound<Self>) -> PyResult<()> {
        if Self::is_active(slf)? {
            slf.getattr("timeline")?.call_method0("cancel")?;
            Self::end(slf)?;
        }
        Ok(())
    }
    fn stop(slf: &Bound<Self>) -> PyResult<()> {
        if !Self::is_active(slf)? {
            return Ok(());
        }
        let pending = slf.getattr("timeline")?.call_method0("stop")?;
        let views = slf.getattr("_views")?;
        let arrive = slf.getattr("_arrive")?;
        Self::end(slf)?;
        if !pending.is_none() && !arrive.is_none() {
            arrive.call1((views.get_item(pending)?,))?;
        }
        Ok(())
    }
    fn glide_to(slf: &Bound<Self>, view: &Obj, seconds: f64, arrive: &Obj) -> PyResult<bool> {
        Self::cancel(slf)?;
        let vp = slf.getattr("_window")?.call_method0("_active_view")?;
        if !slf
            .getattr("timeline")?
            .call_method1(
                "start_glide",
                (
                    pose(&vp.getattr("camera")?)?,
                    view_pose(view)?,
                    seconds,
                    slf.call_method0("_clock")?,
                ),
            )?
            .is_truthy()?
        {
            return Ok(false);
        }
        Self::begin(slf, &vp, vec![view.clone()], arrive)?;
        Ok(true)
    }
    /// A view command's camera move (a standard view, Look at Face, zoom
    /// extents): the camera is already where it ends; it goes back to
    /// `from` and eases in and out to there over `seconds`. Any input that
    /// moves the camera, a click in the view or Esc stops it where it is.
    /// False (the camera stays put) when there is nothing to move.
    fn glide_camera(slf: &Bound<Self>, from: &Obj, seconds: f64) -> PyResult<bool> {
        Self::cancel(slf)?;
        let py = slf.py();
        let vp = slf.getattr("_window")?.call_method0("_active_view")?;
        let camera = vp.getattr("camera")?;
        let started = slf.getattr("timeline")?.call_method1(
            "start_glide",
            (from, pose(&camera)?, seconds, slf.call_method0("_clock")?),
        )?;
        if !started.is_truthy()? {
            return Ok(false);
        }
        from.call_method1("put_on", (&camera,))?;
        Self::begin(slf, &vp, vec![py.None().into_bound(py)], &py.None().into_bound(py))?;
        Ok(true)
    }
    fn play(
        slf: &Bound<Self>,
        views: &Obj,
        start: isize,
        transition: f64,
        pause: f64,
        arrive: &Obj,
    ) -> PyResult<bool> {
        Self::cancel(slf)?;
        let mut ordered: Vec<_> = views.try_iter()?.collect::<PyResult<_>>()?;
        if ordered.is_empty() {
            return Ok(false);
        }
        let offset = start.rem_euclid(ordered.len() as isize) as usize;
        ordered.rotate_left(offset);
        let vp = slf.getattr("_window")?.call_method0("_active_view")?;
        let poses = ordered
            .iter()
            .map(view_pose)
            .collect::<PyResult<Vec<_>>>()?;
        slf.getattr("timeline")?.call_method1(
            "start_run",
            (
                pose(&vp.getattr("camera")?)?,
                poses,
                transition,
                pause,
                slf.call_method0("_clock")?,
            ),
        )?;
        Self::begin(slf, &vp, ordered, arrive)?;
        Self::tick(slf)?;
        Ok(true)
    }
    fn tick(slf: &Bound<Self>) -> PyResult<()> {
        if !Self::is_active(slf)? {
            return Ok(());
        }
        let vp = slf.getattr("_viewport")?;
        if !vp.is(&slf.getattr("_window")?.call_method0("_active_view")?) {
            return Self::cancel(slf);
        }
        let cam = vp.getattr("camera")?;
        let timeline = slf.getattr("timeline")?;
        let result = timeline.call_method1("tick", (slf.call_method0("_clock")?, pose(&cam)?))?;
        let next = result.get_item(0)?;
        if !next.is_none() {
            next.call_method1("put_on", (&cam,))?;
            vp.call_method0("update")?;
            let display = slf.getattr("_display")?;
            if !display.is_none() {
                display.call_method0("update")?;
            }
        }
        let index = result.get_item(1)?;
        let view = if index.is_none() {
            None
        } else {
            Some(slf.getattr("_views")?.get_item(index)?)
        };
        if let Some(v) = &view {
            if Self::is_playing(slf)? {
                let mut present = false;
                for saved in vp.getattr("scene")?.getattr("saved_views")?.try_iter()? {
                    if saved?.is(v) {
                        present = true;
                        break;
                    }
                }
                if !present {
                    return Self::cancel(slf);
                }
            }
        }
        let arrive = slf.getattr("_arrive")?;
        if !Self::is_active(slf)? {
            Self::end(slf)?;
        }
        if let Some(v) = view.filter(|_| !arrive.is_none()) {
            arrive.call1((v,))?;
            timeline.call_method1("accept_camera", (pose(&cam)?,))?;
        }
        Ok(())
    }
}

#[pyfunction]
fn apply_animation(win: &Obj, scene: &Obj, values: &Bound<PyDict>) -> PyResult<()> {
    let fresh = win
        .py()
        .import("built_core")?
        .getattr("SceneAnimation")?
        .call((), Some(values))?;
    if win.getattr("viewport")?.getattr("scene")?.is(scene)
        && !scene.getattr("animation")?.eq(&fresh)?
    {
        scene.setattr("animation", &fresh)?;
        if !fresh.getattr("enabled")?.is_truthy()? {
            win.getattr("scene_player")?.call_method0("stop")?;
        }
        scene.setattr("version", scene.getattr("version")?.add(1)?)?;
        win.call_method0("_sync_animation_menu")?;
    }
    Ok(())
}
#[pyfunction]
pub(crate) fn edit_animation<'p>(win: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = win.py();
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let current = scene.getattr("animation")?;
    let model = py.import(RENDER)?.getattr("OperationForm")?.call_method1(
        "animation",
        (
            current.getattr("enabled")?,
            current.getattr("transition")?,
            current.getattr("pause")?,
        ),
    )?;
    let apply = py.import("functools")?.call_method1(
        "partial",
        (wrap_pyfunction!(apply_animation, py)?, win, scene),
    )?;
    let forms = py.import("built_render.chrome_forms")?;
    let form = forms.call_method1(
        "OperationForm",
        (settings_rows::tr(py, "Animation settings")?, model, apply),
    )?;
    form.setattr("placement", "drawer")?;
    forms.call_method1("show", (win, form))
}
