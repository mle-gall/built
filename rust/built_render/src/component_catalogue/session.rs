// SPDX-License-Identifier: GPL-3.0-or-later
//! Keep online catalogue I/O serial across windows and install downloads only
//! in the document that requested them.
use super::ComponentCatalogue;
use pyo3::{
    prelude::*,
    sync::PyOnceLock,
    types::{IntoPyDict, PyBytes, PyList},
};
type Obj<'p> = Bound<'p, PyAny>;
fn files(py: Python<'_>) -> PyResult<&Py<PyAny>> {
    static FILES: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
    FILES.get_or_try_init(py, || {
        Ok(py
            .import("concurrent.futures")?
            .getattr("ThreadPoolExecutor")?
            .call(
                (1,),
                Some(&[("thread_name_prefix", "component-files")].into_py_dict(py)?),
            )?
            .unbind())
    })
}
#[pyclass]
pub(crate) struct CatalogueSession {
    model: ComponentCatalogue,
    ctx: Py<PyAny>,
    document: Py<PyAny>,
    timer: Option<Py<PyAny>>,
    #[pyo3(get)]
    job: Option<Py<PyAny>>,
    entry: Option<Py<PyAny>>,
    artwork: Vec<(String, u32, u32, Py<PyBytes>)>,
}
impl CatalogueSession {
    pub(crate) fn create<'p>(ctx: &Obj<'p>) -> PyResult<Bound<'p, Self>> {
        let py = ctx.py();
        let win = ctx.getattr("win")?;
        let session = Bound::new(
            py,
            Self {
                model: ComponentCatalogue::new(),
                ctx: ctx.clone().unbind(),
                document: win.getattr("_doc")?.unbind(),
                timer: None,
                job: None,
                entry: None,
                artwork: Vec::new(),
            },
        )?;
        let timer = py.import("built_core.timers")?.getattr("timer")?.call(
            (win, session.getattr("poll")?, 80),
            Some(&[("repeat", true)].into_py_dict(py)?),
        )?;
        session.borrow_mut().timer = Some(timer.unbind());
        session.borrow_mut().index(py, false)?;
        Ok(session)
    }
    fn index(&mut self, py: Python, refresh: bool) -> PyResult<()> {
        self.job = Some(
            files(py)?
                .bind(py)
                .call_method1(
                    "submit",
                    (py.import("built_core")?.getattr("component_library")?.getattr("index")?, refresh),
                )?
                .unbind(),
        );
        self.start(py)
    }
    fn start(&self, py: Python) -> PyResult<()> {
        if let Some(t) = &self.timer {
            t.bind(py).call_method0("start")?;
        }
        Ok(())
    }
}
#[pymethods]
impl CatalogueSession {
    fn close(&mut self, py: Python) -> PyResult<()> {
        if let Some(t) = self.timer.take() {
            t.bind(py).call_method0("stop")?;
        }
        if let Some(job) = self.job.take() {
            job.bind(py).call_method0("cancel")?;
        }
        Ok(())
    }
    fn build<'p>(&mut self, base: &Obj<'p>, query: &str) -> PyResult<Obj<'p>> {
        let py = base.py();
        let i18n = py.import("built_core.i18n")?;
        let json = py.import("json")?;
        let wire = json
            .call_method1("dumps", (base.call_method0("wire")?,))?
            .extract::<String>()?;
        let lang = i18n.call_method0("current_language")?.extract::<String>()?;
        let data = self
            .model
            .sheet(&wire, &lang, query, &i18n.getattr("tr")?)?;
        let spec = py
            .import("built_render.chrome_sheet")?
            .call_method1("Spec", (json.call_method1("loads", (data,))?,))?;
        for icon in self.artwork.drain(..) {
            spec.call_method1("icon", icon)?;
        }
        Ok(spec)
    }
    fn act(&mut self, py: Python, verb: &str, args: Vec<String>) -> PyResult<bool> {
        let key = args.first().map(String::as_str).unwrap_or("");
        let library = py.import("built_core")?.getattr("component_library")?;
        match verb {
            "visible" => {
                library.call_method1("prefetch_thumbnails", (self.model.visible(args),))?;
                self.start(py)?;
            }
            "focus" => {
                self.model.select(key);
                return Ok(key.starts_with("online:"));
            }
            "side" => self.model.select_category(key),
            "verb" if key == "refresh_catalogue" && self.job.is_none() => self.index(py, true)?,
            _ if (verb == "open" && key.starts_with("online:"))
                || (verb == "verb" && key == "insert_online") =>
            {
                if self.job.is_none() {
                    if let Some(record) = self.model.insert((verb == "open").then_some(key)) {
                        let entry = py.import("json")?.call_method1("loads", (record,))?;
                        self.job = Some(
                            files(py)?
                                .bind(py)
                                .call_method1("submit", (library.getattr("model_file")?, &entry))?
                                .unbind(),
                        );
                        self.entry = Some(entry.unbind());
                        self.start(py)?;
                    }
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn poll(slf: &Bound<Self>) -> PyResult<()> {
        let py = slf.py();
        let library = py.import("built_core")?.getattr("component_library")?;
        let mut s = slf.borrow_mut();
        if s.timer.is_none() {
            return Ok(());
        }
        let completed = s
            .job
            .as_ref()
            .map(|j| j.bind(py).call_method0("done")?.is_truthy())
            .transpose()?
            .unwrap_or(false);
        if completed {
            let future = s.job.take().unwrap().into_bound(py);
            let value = if future.call_method0("exception")?.is_none() {
                future.call_method0("result")?
            } else {
                py.None().into_bound(py)
            };
            if let Some(entry) = s.entry.take() {
                s.model.download_finished(!value.is_none());
                let win = s.ctx.bind(py).getattr("win")?;
                if !value.is_none() && win.getattr("_doc")?.is(s.document.bind(py)) {
                    drop(s);
                    py.import("built_render.chrome_sheet")?
                        .call_method1("close", (&win,))?;
                    win.call_method1("insert_library_component", (entry, value))?;
                    return Ok(());
                }
            } else {
                let rows = if value.is_truthy()? {
                    value
                } else {
                    PyList::empty(py).into_any()
                };
                s.model.load(
                    &py.import("json")?
                        .call_method1("dumps", (rows,))?
                        .extract::<String>()?,
                )?;
            }
        }
        let images = s.model.previews(
            py,
            &library.call_method0("library_cache")?.str()?.to_string(),
        );
        s.artwork.extend(images);
        let redraw = completed || !s.artwork.is_empty();
        if s.job.is_none() && !library.call_method0("thumbnails_pending")?.is_truthy()? {
            if let Some(t) = &s.timer {
                t.bind(py).call_method0("stop")?;
            }
        }
        let ctx = s.ctx.clone_ref(py);
        drop(s);
        if redraw {
            ctx.bind(py).call_method0("refresh")?;
        }
        Ok(())
    }
}
