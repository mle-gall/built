// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::exceptions::{PyInterruptedError, PyOSError, PyRuntimeError, PyValueError};

#[pyclass]
struct SceneChoice {
    win: Py<PyAny>,
    viewport: Py<PyAny>,
    history: Py<PyAny>,
    scene: Py<PyAny>,
    scenes: Vec<Py<PyAny>>,
    before: Option<Py<PyAny>>,
    selected: bool,
    builder: Py<PyAny>,
}
impl SceneChoice {
    fn same_document(&self, py: Python) -> PyResult<bool> {
        Ok(self
            .win
            .bind(py)
            .getattr("viewport")?
            .getattr("history")?
            .is(self.history.bind(py)))
    }
}
#[pymethods]
impl SceneChoice {
    #[pyo3(signature=(index=None))]
    fn switch(&mut self, py: Python, index: Option<usize>) -> PyResult<()> {
        let target = match index {
            Some(i) => self.scenes.get(i),
            None if self.selected => self.before.as_ref(),
            _ => return Ok(()),
        };
        let Some(target) = target else { return Ok(()) };
        let scene = self.scene.bind(py);
        let vp = self.viewport.bind(py);
        target
            .bind(py)
            .call_method1("apply", (scene, vp.getattr("camera")?))?;
        self.selected = index.is_some();
        scene.call_method0("bump_view")?;
        vp.call_method0("update")?;
        if self.same_document(py)? {
            self.builder = self.win.bind(py).call_method0("view_svg_builder")?.unbind();
        }
        Ok(())
    }
}
#[pyclass]
struct ExportJob {
    choice: Py<SceneChoice>,
    settings: Py<PyAny>,
    svg: Py<PyAny>,
    form_ref: Option<Py<PyAny>>,
}
impl ExportJob {
    fn draw<'p>(&self, py: Python<'p>, preview: bool) -> ResultObj<'p> {
        let opt = py
            .import("json")?
            .call_method1("loads", (self.svg.bind(py).call_method0("options_json")?,))?;
        let b = self.choice.borrow(py).builder.clone_ref(py);
        b.bind(py).call_method1(
            "build",
            (opt.get_item("faces")?, opt.get_item("paper")?, preview),
        )
    }
    fn write(&self, py: Python, answer: &str, values: &Obj) -> PyResult<()> {
        if answer != "export" || !self.choice.borrow(py).same_document(py)? {
            return Ok(());
        }
        let ch = self.choice.borrow(py);
        let win = ch.win.bind(py);
        let svg = self.svg.bind(py);
        let path = py.import("pathlib")?.call_method1(
            "Path",
            (svg.call_method1("destination", (values.get_item("destination")?,))?,),
        )?;
        let ctx =
            render(py)?.call_method1("svg_progress", (win, ch.history.bind(py), "Export SVG"))?;
        let result = scoped(&ctx, || self.draw(py, false))?;
        let data = result.get_item(0)?.call_method1("encode", ("utf-8",))?;
        let output = py
            .import("ingetrazo_core")?
            .call_method1("AtomicDocument", (&path,))?;
        output.call_method1("write", (data,))?;
        output.call_method0("publish")?;
        let prefs = py
            .import("json")?
            .call_method1("loads", (svg.call_method0("view_preferences")?,))?;
        for (k, v) in prefs.cast::<PyDict>()? {
            self.settings.bind(py).call_method1("setValue", (k, v))?;
        }
        let msg = trf(
            py,
            "Exported {n} lines to {name}",
            &[("n", result.get_item(1)?), ("name", path)],
        )?;
        win.call_method0("statusBar")?
            .call_method1("showMessage", (msg, 5000))?;
        Ok(())
    }
}
#[pymethods]
impl ExportJob {
    fn preview<'p>(slf: &Bound<'p, Self>, ticket: Obj<'p>) -> ResultObj<'p> {
        let py = slf.py();
        let job = slf.borrow();
        let choice = job.choice.borrow(py);
        let check = py
            .import("functools")?
            .call_method1("partial", (slf.getattr("current")?, &ticket))?;
        let ctx = render(py)?.call_method1(
            "svg_progress",
            (
                choice.win.bind(py),
                choice.history.bind(py),
                "SVG preview",
                check,
            ),
        )?;
        drop(choice);
        let result = scoped(&ctx, || job.draw(py, true))?;
        let text = result.get_item(0)?;
        let head = text.get_item(pyo3::types::PySlice::new(py, 0, 1024, 1))?;
        job.svg
            .bind(py)
            .call_method1("preview_header", (ticket, head))?;
        render(py)?.call_method1("svg_preview", (text,))
    }
    fn current(&self, py: Python, ticket: &Obj) -> PyResult<bool> {
        let Some(r) = &self.form_ref else {
            return Ok(false);
        };
        let form = r.bind(py).call0()?;
        Ok(!form.is_none()
            && !form.getattr("closed")?.is_truthy()?
            && self
                .svg
                .bind(py)
                .call_method1("preview_current", (ticket,))?
                .is_truthy()?)
    }
    fn done(&self, py: Python, answer: &str, values: &Obj) -> PyResult<()> {
        let result = (|| {
            let Err(e) = self.write(py, answer, values) else {
                return Ok(());
            };
            let ch = self.choice.borrow(py);
            let win = ch.win.bind(py);
            if e.is_instance_of::<PyInterruptedError>(py) {
                win.call_method0("statusBar")?
                    .call_method1("showMessage", (tr(py, "SVG export cancelled")?, 5000))?;
            } else if e.is_instance_of::<PyOSError>(py)
                || e.is_instance_of::<PyRuntimeError>(py)
                || e.is_instance_of::<PyValueError>(py)
            {
                py.import("ingetrazo_render.chrome_dialogs")?
                    .getattr("notify")?
                    .call(
                        (win, tr(py, "Export SVG failed")?, e.value(py).str()?),
                        Some(&[("tone", "error")].into_py_dict(py)?),
                    )?;
            } else {
                return Err(e);
            }
            Ok(())
        })();
        self.choice.borrow_mut(py).switch(py, None)?;
        result
    }
}
#[pyfunction]
#[pyo3(signature=(win,frame_builder=None))]
pub(super) fn open_view_export<'p>(
    win: &Obj<'p>,
    frame_builder: Option<&Obj<'p>>,
) -> ResultObj<'p> {
    let py = win.py();
    let vp = win.call_method0("_active_view")?;
    let scene = vp.getattr("scene")?;
    let hist = win.getattr("viewport")?.getattr("history")?;
    let scenes = if frame_builder.is_none() {
        scene
            .getattr("saved_views")?
            .try_iter()?
            .collect::<PyResult<Vec<_>>>()?
    } else {
        vec![]
    };
    let names = scenes
        .iter()
        .map(|s| s.getattr("name"))
        .collect::<PyResult<Vec<_>>>()?;
    let builder = match frame_builder {
        Some(b) => b.clone(),
        None => win.call_method0("view_svg_builder")?,
    };
    let before = if scenes.is_empty() {
        None
    } else {
        Some(
            py.import("ingetrazo_core.saved_views")?
                .getattr("SavedView")?
                .call_method1("capture", ("", &scene, vp.getattr("camera")?))?
                .unbind(),
        )
    };
    let choice = Bound::new(
        py,
        SceneChoice {
            win: win.clone().unbind(),
            viewport: vp.unbind(),
            history: hist.unbind(),
            scene: scene.clone().unbind(),
            scenes: scenes.into_iter().map(Bound::unbind).collect(),
            before,
            selected: false,
            builder: builder.clone().unbind(),
        },
    )?;
    let settings = py.import("ingetrazo_core.settings")?.call_method0("settings")?;
    settings.call_method1("beginGroup", ("export_svg/v5",))?;
    let prefs = PyDict::new(py);
    for key in settings.call_method0("childKeys")?.try_iter()? {
        let key = key?;
        prefs.set_item(&key, settings.call_method1("value", (&key,))?)?;
    }
    let json = py.import("json")?.getattr("dumps")?.call(
        (prefs,),
        Some(&[("default", py.import("builtins")?.getattr("str")?)].into_py_dict(py)?),
    )?;
    let sun = py
        .import("ingetrazo_core.sun")?
        .call_method1("scene_sun_direction", (&scene,))?;
    let switch = if names.is_empty() {
        None
    } else {
        Some(choice.getattr("switch")?)
    };
    let svg = render(py)?.getattr("SvgOptions")?.call_method1(
        "current_view",
        (
            json,
            builder.getattr("screen_mm")?,
            scene.getattr("show_axes")?,
            !sun.is_none(),
            scene.getattr("selection")?.is_truthy()?,
            names,
            frame_builder.map(|b| b.getattr("scale_n")).transpose()?,
            switch,
        ),
    )?;
    let job = Bound::new(
        py,
        ExportJob {
            choice: choice.unbind(),
            settings: settings.unbind(),
            svg: svg.clone().unbind(),
            form_ref: None,
        },
    )?;
    let form = render(py)?.call_method1(
        "show_view_export",
        (win, svg, job.getattr("done")?, job.getattr("preview")?),
    )?;
    job.borrow_mut().form_ref = Some(
        py.import("weakref")?
            .call_method1("ref", (&form,))?
            .unbind(),
    );
    Ok(form)
}
