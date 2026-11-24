//! Watch linked documents across atomic replacement, disappearance and recreation.
//! The native event loop consumes the next deadline returned by poll().
use super::model_edit::translated;
use pyo3::{prelude::*, types::PyDict};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

const SCAN: Duration = Duration::from_secs(1);
const SETTLE: Duration = Duration::from_millis(400);

#[derive(PartialEq, Eq)]
struct Stamp {
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    len: u64,
    #[cfg(unix)]
    inode: u64,
}
fn stamp(path: &Path) -> Option<Stamp> {
    let m = fs::metadata(path).ok().filter(|m| m.is_file())?;
    Some(Stamp {
        modified: m.modified().ok(),
        created: m.created().ok(),
        len: m.len(),
        #[cfg(unix)]
        inode: std::os::unix::fs::MetadataExt::ino(&m),
    })
}

#[derive(Default)]
struct Files {
    watched: BTreeMap<PathBuf, (Option<Stamp>, Option<Instant>)>,
    next: Option<Instant>,
}
impl Files {
    fn refresh(&mut self, paths: BTreeSet<PathBuf>, now: Instant) -> bool {
        let before = self.watched.len();
        self.watched.retain(|path, _| paths.contains(path));
        let mut changed = before != self.watched.len();
        for path in paths {
            self.watched.entry(path).or_insert_with_key(|p| {
                changed = true;
                (stamp(p), None)
            });
        }
        if self.watched.is_empty() {
            self.next = None;
        } else if self.next.is_none() {
            self.next = Some(now + SCAN);
        }
        changed
    }

    fn tick(&mut self, now: Instant) -> Vec<PathBuf> {
        let mut ready = Vec::new();
        if !self.next.is_some_and(|t| t <= now) {
            return ready;
        }
        let mut next = now + SCAN;
        for (path, (old, settled)) in &mut self.watched {
            let current = stamp(path);
            if *old != current {
                *old = current;
                *settled = Some(now + SETTLE);
            }
            if let Some(deadline) = *settled {
                if deadline <= now {
                    ready.push(path.clone());
                    *settled = None;
                } else {
                    next = next.min(deadline);
                }
            }
        }
        self.next = (!self.watched.is_empty()).then_some(next);
        ready
    }
}

#[pyclass(unsendable)]
pub struct LinkWatch {
    window: Py<PyAny>, // weakref: neither a timer nor a source owns the host
    files: RefCell<Files>,
}

impl LinkWatch {
    pub fn new(win: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            window: win
                .py()
                .import("weakref")?
                .call_method1("ref", (win,))?
                .unbind(),
            files: Default::default(),
        })
    }

    fn reload(&self, win: &Bound<'_, PyAny>, paths: &[PathBuf]) -> PyResult<()> {
        let py = win.py();
        let links = py.import("core.links")?;
        let mut names = BTreeSet::new();
        let mut failures = Vec::new();
        for doc in win.call_method0("documents")?.try_iter()? {
            let doc = doc?;
            let vp = doc.getattr("viewport")?;
            if vp.is_none() {
                continue;
            }
            let scene = vp.getattr("scene")?;
            let sources: BTreeSet<_> = links
                .call_method1("sources", (&scene,))?
                .extract::<Vec<PathBuf>>()?
                .into_iter()
                .collect();
            let mut dirty = false;
            for path in paths.iter().filter(|p| sources.contains(*p)) {
                dirty = true;
                if !path.is_file()
                    || links
                        .call_method1("is_packed", (&scene, path))?
                        .is_truthy()?
                {
                    continue;
                }
                match links.call_method1("reload", (&scene, path, doc.getattr("path")?)) {
                    Ok(_) => {
                        vp.call_method0("notify_scene_changed")?;
                        vp.call_method0("update")?;
                        names.insert(
                            path.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned(),
                        );
                    }
                    Err(e) if e.is_instance(py, &links.getattr("LinkError")?) => {
                        failures.push(e.value(py).str()?.to_string())
                    }
                    Err(e) => return Err(e),
                }
            }
            let view = doc.getattr("stand_in")?;
            if dirty && !view.is_none() {
                for k in ["_ui_rules_cache", "_ui_tree_sig", "_ui_insp_sig"] {
                    view.setattr(k, py.None())?;
                }
                view.call_method0("update")?;
            }
        }
        let mut messages = Vec::new();
        if !names.is_empty() {
            let vars = PyDict::new(py);
            vars.set_item("names", names.into_iter().collect::<Vec<_>>().join(", "))?;
            messages.push(translated(py, "Reloaded {names}", Some(&vars))?.extract::<String>()?);
        }
        if let Some(error) = failures.first() {
            messages.push(error.clone());
        }
        if !messages.is_empty() {
            win.call_method0("statusBar")?
                .call_method1("showMessage", (messages.join("  ·  "), 5000))?;
        }
        Ok(())
    }
}
#[pymethods]
impl LinkWatch {
    #[getter]
    fn paths(&self) -> Vec<PathBuf> {
        self.files.borrow().watched.keys().cloned().collect()
    }

    fn refresh(&self, py: Python<'_>) -> PyResult<()> {
        let win = self.window.bind(py).call0()?;
        if win.is_none() {
            return Ok(());
        }
        let links = py.import("core.links")?;
        let mut paths = BTreeSet::new();
        for doc in win.call_method0("documents")?.try_iter()? {
            let vp = doc?.getattr("viewport")?;
            if vp.is_none() {
                continue;
            }
            let scene = vp.getattr("scene")?;
            for path in links.call_method1("sources", (&scene,))?.try_iter()? {
                let path = path?;
                if !links
                    .call_method1("is_packed", (&scene, &path))?
                    .is_truthy()?
                {
                    paths.insert(path.extract::<PathBuf>()?);
                }
            }
        }
        let changed = self.files.borrow_mut().refresh(paths, Instant::now());
        if changed {
            win.call_method0("_wake_link_watch")?;
        }
        Ok(())
    }

    fn poll(&self, py: Python<'_>) -> PyResult<Option<u64>> {
        let ready = self.files.borrow_mut().tick(Instant::now());
        if !ready.is_empty() {
            let win = self.window.bind(py).call0()?;
            if win.is_none() {
                return Ok(None);
            }
            self.reload(&win, &ready)?;
            self.refresh(py)?;
        }
        Ok(self
            .files
            .borrow()
            .next
            .map(|t| t.saturating_duration_since(Instant::now()).as_millis() as u64 + 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recheck_after_each_write_and_keep_missing_source_subscribed() {
        let path = std::env::temp_dir().join(format!("built-link-watch-{}", std::process::id()));
        let now = Instant::now();
        let mut f = Files::default();
        fs::write(&path, b"first").unwrap();
        f.refresh(BTreeSet::from([path.clone()]), now);
        fs::write(&path, b"second").unwrap();
        assert!(f.tick(now + SCAN).is_empty());
        fs::write(&path, b"third pass").unwrap();
        assert!(f.tick(now + SCAN + SETTLE).is_empty());
        assert_eq!(f.tick(now + SCAN + SETTLE * 2), vec![path.clone()]);
        fs::remove_file(&path).unwrap();
        let removed = now + SCAN * 3;
        assert!(f.tick(removed).is_empty());
        assert_eq!(f.tick(removed + SETTLE), vec![path.clone()]);
        fs::write(&path, b"recreated").unwrap();
        assert!(f.tick(removed + SCAN * 2).is_empty());
        assert_eq!(f.tick(removed + SCAN * 2 + SETTLE), vec![path.clone()]);
        f.refresh(BTreeSet::new(), removed + SCAN * 3);
        assert!(f.next.is_none());
        fs::remove_file(path).unwrap();
    }
}
