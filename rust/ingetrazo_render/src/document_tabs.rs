//! The window's open projects for the chrome's document ▾ and project tab
//! strip (formerly `wgpu_ui._projects` and `wgpu_tabs.before_frame`).
//!
//! Each frame reads the window's document list once (names, the one in
//! front, unsaved flags) and sends only what changed. What was last sent is
//! kept here, by the renderer that shows it, not on the document's view: a
//! view handed back after another document had the renderer (A, B, then
//! A) used to compare against its own stale copy and skip the send while
//! the renderer still showed B's tabs (port review finding 5).
use pyo3::{prelude::*, types::PyList};
use std::time::{Duration, Instant};

type Project = (String, bool, bool);
type Tab = (String, String, bool, bool, bool, String, String);
type Elsewhere = Vec<(String, String, String)>;

/// The tab strip rebuilds its tips (saved times, link hosts) at least this
/// often while frames continue, even when nothing in the list changed.
const TIPS: Duration = Duration::from_secs(1);

#[derive(PartialEq)]
struct Seen {
    ids: Vec<usize>,
    front: usize,
    dirty: Vec<bool>,
    names: Vec<String>,
    pictures: bool,
    elsewhere: Elsewhere,
}

#[derive(PartialEq)]
struct Payload {
    tabs: Vec<Tab>,
    elsewhere: Elsewhere,
    can_move: bool,
    pictures: bool,
}

#[pyclass(unsendable)]
#[derive(Default)]
pub struct DocumentTabs {
    projects: Option<Vec<Project>>,
    seen: Option<(Seen, Instant)>,
    payload: Option<Payload>,
}

fn attr<'py>(obj: &Bound<'py, PyAny>, name: &str) -> Option<Bound<'py, PyAny>> {
    obj.getattr(name).ok().filter(|v| !v.is_none())
}

/// The window's documents in tab order: the tab bar's list, else the
/// window's own (as `wgpu_tabs._docs`).
fn documents<'py>(win: &Bound<'py, PyAny>) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let Some(bar) = attr(win, "_project_tabs") else { return Ok(Vec::new()) };
    let list = match bar.call_method0("documents") {
        Ok(list) => list,
        Err(_) => match attr(win, "_documents") {
            Some(list) => list,
            None => return Ok(Vec::new()),
        },
    };
    list.try_iter()?.collect()
}

fn dirty(win: &Bound<'_, PyAny>, doc: &Bound<'_, PyAny>) -> bool {
    win.call_method1("_doc_is_dirty", (doc,)).and_then(|v| v.is_truthy()).unwrap_or(false)
}

#[pymethods]
impl DocumentTabs {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    /// Send the document ▾ rows and the tab strip to ``renderer`` when they
    /// changed. ``left`` is where the document ▾ starts.
    fn sync(&mut self, py: Python<'_>, renderer: &Bound<'_, PyAny>, win: &Bound<'_, PyAny>, left: f32) -> PyResult<()> {
        let (docs, names, flags) = self.projects_of(renderer, win, left)?;
        self.strip(py, renderer, win, docs, names, flags)
    }

    /// The document ▾ alone (the Sheets workspace's frame).
    fn sync_projects(&mut self, renderer: &Bound<'_, PyAny>, win: &Bound<'_, PyAny>, left: f32) -> PyResult<()> {
        self.projects_of(renderer, win, left).map(|_| ())
    }

    /// The strip is rebuilt on the next frame (after a tab action).
    fn stale(&mut self) {
        self.seen = None;
    }

    /// Everything is sent again on the next frame (a renderer whose chrome
    /// state was reset).
    fn reset(&mut self) {
        *self = Self::default();
    }
}

type Read<'py> = (Vec<Bound<'py, PyAny>>, Vec<String>, Vec<bool>);

impl DocumentTabs {
    /// Read the documents once; send the document ▾ rows when they changed.
    fn projects_of<'py>(&mut self, renderer: &Bound<'py, PyAny>, win: &Bound<'py, PyAny>, left: f32) -> PyResult<Read<'py>> {
        let docs = documents(win)?;
        let names = docs
            .iter()
            .map(|d| d.call_method0("display_name")?.extract::<String>())
            .collect::<PyResult<Vec<_>>>()?;
        let flags: Vec<bool> = docs.iter().map(|d| dirty(win, d)).collect();
        // The document ▾: the tab bar's current document is in front.
        if let Some(bar) = attr(win, "_project_tabs") {
            let current = bar.call_method0("current")?;
            let rows: Vec<Project> = docs
                .iter()
                .zip(&names)
                .zip(&flags)
                .map(|((d, n), &f)| (n.clone(), d.is(&current), f))
                .collect();
            if self.projects.as_ref() != Some(&rows) {
                renderer.call_method1("ui_set_projects", (left, rows.clone()))?;
                self.projects = Some(rows);
            }
        }
        Ok((docs, names, flags))
    }

    fn strip<'py>(&mut self, py: Python<'py>, renderer: &Bound<'py, PyAny>, win: &Bound<'py, PyAny>,
                  docs: Vec<Bound<'py, PyAny>>, names: Vec<String>, flags: Vec<bool>) -> PyResult<()> {
        // The strip.
        if !renderer.hasattr("ui_set_project_tabs")? {
            return Ok(());
        }
        let tabs = py.import("views.wgpu_tabs")?;
        let pictures: bool = tabs.call_method0("pictures_on")?.is_truthy()?;
        let elsewhere: Elsewhere = tabs.call_method1("_elsewhere", (win,))?.extract()?;
        let front = attr(win, "_doc").map_or(0, |d| d.as_ptr() as usize);
        let seen = Seen {
            ids: docs.iter().map(|d| d.as_ptr() as usize).collect(),
            front,
            dirty: flags,
            names,
            pictures,
            elsewhere: elsewhere.clone(),
        };
        let now = Instant::now();
        if self.seen.as_ref().is_some_and(|(last, at)| *last == seen && now.duration_since(*at) < TIPS) {
            return Ok(());
        }
        let list = PyList::new(py, &docs)?;
        let payload = Payload {
            tabs: tabs.call_method1("tabs", (win, list))?.extract()?,
            elsewhere,
            can_move: tabs.call_method1("_can_move", (win,))?.is_truthy()?,
            pictures,
        };
        if self.payload.as_ref() != Some(&payload) {
            renderer.call_method1(
                "ui_set_project_tabs",
                (payload.tabs.clone(), payload.elsewhere.clone(), payload.can_move, payload.pictures),
            )?;
            self.payload = Some(payload);
        }
        self.seen = Some((seen, now));
        Ok(())
    }
}
