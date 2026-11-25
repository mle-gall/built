// SPDX-License-Identifier: GPL-3.0-or-later
//! Publish an update check only while it is still the window's current request.
use super::ReleaseReview;
use crate::sheet_runtime::Provider;
use pyo3::{
    prelude::*,
    types::{IntoPyDict, PyDict},
};
type Obj<'p> = Bound<'p, PyAny>;
fn settings(py: Python) -> PyResult<Obj> {
    py.import("built_core.settings")?.call_method0("settings")
}
fn sheet(py: Python) -> PyResult<Bound<PyModule>> {
    py.import("built_render.chrome_sheet")
}
fn dialogs(py: Python) -> PyResult<Bound<PyModule>> {
    py.import("built_render.chrome_dialogs")
}
fn translation(py: Python) -> PyResult<Obj> {
    py.import("built_core.i18n")?.getattr("tr")
}
#[pyclass(extends=Provider)]
struct ReleaseSheet {
    model: Py<ReleaseReview>,
}
#[pymethods]
impl ReleaseSheet {
    #[classattr]
    #[pyo3(name = "id")]
    const ID: &'static str = "update";
    fn build<'p>(&self, py: Python<'p>, _ctx: &Obj) -> PyResult<Obj<'p>> {
        let wire = self.model.borrow(py).sheet(&translation(py)?)?;
        sheet(py)?.call_method1(
            "Spec",
            (py.import("json")?.call_method1("loads", (wire,))?,),
        )
    }
    fn act(&self, ctx: &Obj, verb: &str, args: Vec<String>) -> PyResult<()> {
        let py = ctx.py();
        if verb != "verb" {
            return Ok(());
        }
        let model = self.model.borrow(py);
        if let Some((effect, value, close)) = args.first().and_then(|k| model.action(k)) {
            match effect {
                "skip" => {
                    py.import("built_render.release_check")?
                        .call_method1("skip_version", (settings(py)?, &value))?;
                }
                "open" => {
                    py.import("built_core.desktop_open")?
                        .call_method1("open_url", (value,))?;
                }
                _ => {}
            }
            if close {
                sheet(py)?.call_method1("close", (ctx.getattr("win")?,))?;
            }
        }
        Ok(())
    }
}
#[pyclass]
struct Check {
    win: Py<PyAny>,
    settings: Py<PyAny>,
    result: Py<PyAny>,
    current: String,
    quiet: bool,
}
impl Check {
    fn current(&self, py: Python) -> PyResult<bool> {
        Ok(self
            .win
            .bind(py)
            .getattr("_release_check")?
            .is(self.result.bind(py)))
    }
    fn open(&self, py: Python, model: Py<ReleaseReview>) -> PyResult<()> {
        let provider = Bound::new(
            py,
            PyClassInitializer::from(Provider).add_subclass(ReleaseSheet { model }),
        )?;
        sheet(py)?.call_method1("open", (self.win.bind(py), provider))?;
        Ok(())
    }
}
#[pymethods]
impl Check {
    fn receive(slf: &Bound<Self>) -> PyResult<()> {
        let py = slf.py();
        let s = slf.borrow();
        if !s.current(py)? {
            return Ok(());
        }
        let future = s.result.bind(py);
        if !future.call_method0("done")?.is_truthy()? {
            py.import("built_core.timers")?
                .call_method1("later", (s.win.bind(py), 100, slf.getattr("receive")?))?;
            return Ok(());
        }
        let update = py.import("built_render.release_check")?;
        let settings = s.settings.bind(py);
        let model = match future.call_method0("result") {
            Ok(release) => {
                update.call_method1("mark_checked", (settings,))?;
                let version: String = release.getattr("version")?.extract()?;
                let newer = update
                    .call_method1("is_newer", (&version, &s.current))?
                    .is_truthy()?;
                let skipped = update
                    .call_method1("is_skipped", (settings, &version))?
                    .is_truthy()?;
                ReleaseReview::new(
                    s.current.clone(),
                    version,
                    release.getattr("date")?.extract()?,
                    &release.getattr("notes")?.extract::<String>()?,
                    release.getattr("url")?.extract()?,
                    s.quiet,
                    newer,
                    skipped,
                )
            }
            Err(e) if e.matches(py, update.getattr("UpdateCheckError")?)? => ReleaseReview::new(
                s.current.clone(),
                String::new(),
                String::new(),
                "",
                String::new(),
                s.quiet,
                false,
                false,
            ),
            Err(e) => return Err(e),
        };
        drop(s);
        Self::present(slf, Bound::new(py, model)?)
    }
    fn present(slf: &Bound<Self>, model: Bound<ReleaseReview>) -> PyResult<()> {
        let py = slf.py();
        let s = slf.borrow();
        if !s.current(py)? {
            return Ok(());
        }
        let win = s.win.bind(py);
        let dialogs = dialogs(py)?;
        let busy = dialogs.call_method1("modal", (win,))?.is_truthy()?
            || dialogs.call_method1("async_host", (win,))?.is_none();
        let partial = |method: &str| {
            py.import("functools")?
                .call_method1("partial", (slf.getattr(method)?, &model))
        };
        let presentation = model.borrow().presentation(busy);
        match presentation {
            "silent" => {}
            "wait" => {
                py.import("built_core.timers")?
                    .call_method1("later", (win, 5000, partial("present")?))?;
            }
            "review" => s.open(py, model.unbind())?,
            _ => {
                let (title, text, tone, label) = model.borrow().notice(&translation(py)?)?;
                let kw = PyDict::new(py);
                kw.set_item("tone", tone)?;
                if !label.is_empty() {
                    kw.set_item("action_label", label)?;
                    kw.set_item("action", partial("review")?)?;
                }
                dialogs
                    .getattr("notify")?
                    .call((win, title, text), Some(&kw))?;
            }
        }
        Ok(())
    }
    fn review(&self, py: Python, model: Py<ReleaseReview>) -> PyResult<()> {
        if self.current(py)?
            && !py
                .import("built_render.release_check")?
                .call_method1(
                    "is_skipped",
                    (self.settings.bind(py), &model.borrow(py).version),
                )?
                .is_truthy()?
        {
            self.open(py, model)?;
        }
        Ok(())
    }
}
#[pyfunction]
#[pyo3(signature=(window,quiet=false,fetch=None,current=None))]
pub(crate) fn start_check(
    window: &Obj,
    quiet: bool,
    fetch: Option<Py<PyAny>>,
    current: Option<String>,
) -> PyResult<()> {
    let py = window.py();
    let update = py.import("built_render.release_check")?;
    let settings = settings(py)?;
    let sys = py.import("sys")?;
    let frozen = sys
        .getattr_opt("frozen")?
        .is_some_and(|f| f.is_truthy().unwrap_or(false))
        || (sheet(py)?.call_method0("in_shell")?.is_truthy()? && sys.hasattr("_MEIPASS")?);
    if quiet
        && !update
            .getattr("should_check_at_start")?
            .call((&settings,), Some(&[("frozen", frozen)].into_py_dict(py)?))?
            .is_truthy()?
    {
        return Ok(());
    }
    let result = py.import("concurrent.futures")?.call_method0("Future")?;
    let worker = result.clone().unbind();
    let fetch = match fetch {
        Some(f) => f,
        None => update.getattr("fetch_latest")?.unbind(),
    };
    std::thread::Builder::new()
        .name("Built update check".into())
        .spawn(move || {
            Python::try_attach(|py| {
                let out = match fetch.call0(py) {
                    Ok(v) => worker.bind(py).call_method1("set_result", (v,)),
                    Err(e) => worker
                        .bind(py)
                        .call_method1("set_exception", (e.value(py),)),
                };
                if let Err(e) = out {
                    e.write_unraisable(py, Some(worker.bind(py)));
                }
            });
        })?;
    window.setattr("_release_check", &result)?;
    let current = match current {
        Some(v) => v,
        None => py
            .import("built_core.version")?
            .getattr("__version__")?
            .extract()?,
    };
    let job = Bound::new(
        py,
        Check {
            win: window.clone().unbind(),
            settings: settings.unbind(),
            result: result.unbind(),
            current,
            quiet,
        },
    )?;
    py.import("built_core.timers")?
        .call_method1("later", (window, 0, job.getattr("receive")?))?;
    Ok(())
}
