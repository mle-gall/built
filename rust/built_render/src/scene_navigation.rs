//! Navigation uses the marked saved view of the active document.
use pyo3::{prelude::*, types::PyDict};

/// The scene selected for recall and editing belongs to the document.
#[pyclass]
#[derive(Default)]
pub struct SceneMark {
    view: Option<Py<PyAny>>,
    revision: Option<(usize, u64)>,
}

#[pymethods]
impl SceneMark {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    #[getter]
    fn tabs(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn marked_view(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.view.as_ref().map(|view| view.clone_ref(py))
    }

    fn mark_view(&mut self, view: Option<Py<PyAny>>) {
        self.view = view;
        self.revision = None;
    }

    fn for_scene<'py>(
        mut slf: PyRefMut<'py, Self>,
        scene: &Bound<'py, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        if let Some(view) = &slf.view {
            let revision = (
                scene.as_ptr() as usize,
                scene.getattr("content_version")?.extract()?,
            );
            if slf.revision != Some(revision) {
                let views = scene.getattr("saved_views")?;
                if !views.try_iter()?.try_fold(false, |found, item| {
                    Ok::<_, PyErr>(found || item?.is(view.bind(scene.py())))
                })? {
                    slf.view = None;
                }
                slf.revision = Some(revision);
            }
        }
        Ok(slf)
    }
}

/// The egui row is from the last painted menu. A scene deleted, moved to
/// another document, or undone since that frame must not change this camera.
#[pyfunction]
fn show_listed_scene(
    win: &Bound<'_, PyAny>,
    listed: &Bound<'_, PyAny>,
    index: usize,
) -> PyResult<()> {
    if index < listed.len()? {
        let selected = listed.get_item(index)?;
        let current = win
            .getattr("viewport")?
            .getattr("scene")?
            .getattr("saved_views")?;
        for view in current.try_iter()? {
            if view?.is(&selected) {
                win.getattr("tray")?
                    .getattr("scenes")?
                    .call_method1("show_view", (selected,))?;
                break;
            }
        }
    }
    Ok(())
}

#[pyfunction]
pub fn marked_scene_index(win: &Bound<'_, PyAny>) -> PyResult<isize> {
    let marked = win
        .getattr("_scene_tabs")?
        .getattr("tabs")?
        .call_method0("marked_view")?;
    let views = win
        .getattr("viewport")?
        .getattr("scene")?
        .getattr("saved_views")?;
    for (i, view) in views.try_iter()?.enumerate() {
        if view?.is(&marked) {
            return Ok(i as isize);
        }
    }
    Ok(-1)
}
#[pyfunction]
fn step_scene(win: &Bound<'_, PyAny>, step: isize) -> PyResult<()> {
    let views = win
        .getattr("viewport")?
        .getattr("scene")?
        .getattr("saved_views")?;
    let count = views.len()? as isize;
    if count > 0 {
        let mut i = marked_scene_index(win)?;
        if i < 0 && step <= 0 {
            i = 0;
        }
        let next = (i + step).rem_euclid(count);
        win.getattr("tray")?
            .getattr("scenes")?
            .call_method1("show_view", (views.get_item(next)?,))?;
    }
    Ok(())
}
#[pyfunction]
fn play_scenes(win: &Bound<'_, PyAny>, on: bool) -> PyResult<()> {
    let player = win.getattr("scene_player")?;
    if !on {
        player.call_method0("stop")?;
        return Ok(());
    }
    let py = win.py();
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let views = py
        .import("builtins")?
        .call_method1("list", (scene.getattr("saved_views")?,))?;
    let active = views.is_truthy()?;
    let (tip, ms) = if active {
        let animation = scene.getattr("animation")?;
        let kw = PyDict::new(py);
        kw.set_item("camera", false)?;
        let finish = win
            .getattr("tray")?
            .getattr("scenes")?
            .getattr("finish_view")?;
        let arrive = py
            .import("functools")?
            .getattr("partial")?
            .call((finish,), Some(&kw))?;
        let start = marked_scene_index(win)? + 1;
        player.call_method1(
            "play",
            (
                views,
                start,
                animation.getattr("transition")?,
                animation.getattr("pause")?,
                arrive,
            ),
        )?;
        (
            "Playing the scenes: Esc or a click in the view stops.",
            3000,
        )
    } else {
        ("This project has no scenes to play: keep a view as a scene first (Scenes panel, «+ Scene»).",4000)
    };
    win.call_method1("_on_player_active", (active,))?;
    win.call_method0("statusBar")?.call_method1(
        "showMessage",
        (py.import("built_core.i18n")?.call_method1("tr", (tip,))?, ms),
    )?;
    Ok(())
}
#[pyfunction]
fn scene_transitions(win: &Bound<'_, PyAny>, on: bool) -> PyResult<()> {
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let animation = scene.getattr("animation")?;
    if animation.getattr("enabled")?.is_truthy()? != on {
        let kw = PyDict::new(win.py());
        kw.set_item("enabled", on)?;
        kw.set_item("transition", animation.getattr("transition")?)?;
        kw.set_item("pause", animation.getattr("pause")?)?;
        scene.setattr(
            "animation",
            win.py()
                .import("built_core")?
                .getattr("SceneAnimation")?
                .call((), Some(&kw))?,
        )?;
        if !on {
            win.getattr("scene_player")?.call_method0("stop")?;
        }
        scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
        win.call_method0("_sync_animation_menu")?;
    }
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<SceneMark>()?;
    m.add_function(wrap_pyfunction!(show_listed_scene, m)?)?;
    m.add_function(wrap_pyfunction!(marked_scene_index, m)?)?;
    m.add_function(wrap_pyfunction!(step_scene, m)?)?;
    m.add_function(wrap_pyfunction!(play_scenes, m)?)?;
    m.add_function(wrap_pyfunction!(scene_transitions, m)?)?;
    Ok(())
}
