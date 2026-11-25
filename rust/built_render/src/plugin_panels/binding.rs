// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::types::PyString;

pub(super) enum Source<'a, 'p> {
    Factory(&'a Obj<'p>),
    Utility(&'a Obj<'p>),
}

/// Store plugin objects in Python's instance dictionary, including callbacks
/// that refer back to this binding. The cyclic collector can see those edges.
#[pyclass(dict, weakref, module = "built_core.plugin_panels")]
pub(super) struct Binding {
    #[pyo3(get)]
    instance: String,
    #[pyo3(get, set)]
    dirty: bool,
    #[pyo3(get, set)]
    last_event: u64,
    #[pyo3(get, set)]
    sent_revision: Option<u64>,
    #[pyo3(get, set)]
    error: String,
    #[pyo3(get, set)]
    last_sections: String,
}
impl Binding {
    pub(super) fn create<'p>(
        win: &Obj<'p>,
        key: &Obj<'p>,
        source: Source<'_, 'p>,
        view: &Obj<'p>,
    ) -> PyResult<Bound<'p, Self>> {
        let py = win.py();
        let b = Bound::new(
            py,
            Self {
                instance: scope(py)?.extract()?,
                dirty: true,
                last_event: 0,
                sent_revision: None,
                error: String::new(),
                last_sections: "[]".to_owned(),
            },
        )?;
        b.setattr("key", key)?;
        b.setattr("document", view.getattr("_plugin_document")?)?;
        b.setattr("scene", scene(win)?)?;
        b.setattr("view", view)?;
        let model = match source {
            Source::Factory(factory) => {
                let app = py
                    .import("views.extension_api")?
                    .getattr("ExtensionApp")?
                    .call1((win, key))?;
                factory.call1((app, b.getattr("invalidate")?))?
            }
            Source::Utility(m) => m.clone(),
        };
        b.setattr("model", &model)?;
        for attr in ["id", "title", "slot"] {
            b.setattr(attr, model.getattr(attr)?)?;
        }
        for attr in ["id", "title"] {
            let v = b.getattr(attr)?;
            if !v.is_instance_of::<PyString>() || !v.call_method0("strip")?.is_truthy()? {
                return Err(PyValueError::new_err(
                    "A panel requires a nonempty string ID and title",
                ));
            }
        }
        Ok(b)
    }
}

fn error_section<'p>(py: Python<'p>, msg: &Obj<'p>, failed: bool) -> PyResult<Bound<'p, PyDict>> {
    let row = PyDict::new(py);
    row.set_item("id", "_error")?;
    row.set_item(
        "title",
        if failed {
            "Panel unavailable"
        } else {
            "Plugin error"
        },
    )?;
    let log = PyDict::new(py);
    log.set_item("kind", "log")?;
    log.set_item("id", "_exception")?;
    log.set_item("label", "")?;
    log.set_item("lines", PyList::new(py, [msg])?)?;
    let items = PyList::new(py, [log])?;
    if failed {
        let retry = PyDict::new(py);
        retry.set_item("kind", "button")?;
        retry.set_item("id", "_retry")?;
        retry.set_item("label", "Retry")?;
        items.append(retry)?;
    }
    row.set_item("items", items)?;
    Ok(row)
}

#[pymethods]
impl Binding {
    #[new]
    fn new<'p>(
        win: &Obj<'p>,
        key: &Obj<'p>,
        factory: &Obj<'p>,
        view: &Obj<'p>,
    ) -> PyResult<Bound<'p, Self>> {
        Self::create(win, key, Source::Factory(factory), view)
    }
    fn invalidate(s: &Bound<Self>) -> PyResult<()> {
        s.borrow_mut().dirty = true;
        match s.getattr("view")?.call_method0("update") {
            Err(e) if e.is_instance_of::<PyRuntimeError>(s.py()) => Ok(()),
            other => other.map(drop),
        }
    }
    fn push(s: &Bound<Self>) -> PyResult<()> {
        let py = s.py();
        let revision: u64 = s.getattr("scene")?.getattr("version")?.extract()?;
        let renderer = s.getattr("view")?.getattr("renderer")?;
        let model = s.getattr("model")?;
        let visible = !flagged(&model, "sections_on_show")?
            || renderer
                .call_method1("ui_plugin_visible", (s.getattr("instance")?,))?
                .is_truthy()?;
        {
            let mut state = s.borrow_mut();
            if (!visible && state.sent_revision.is_some())
                || (!state.dirty && state.sent_revision == Some(revision))
            {
                return Ok(());
            }
            state.dirty = !visible;
            state.sent_revision = Some(revision);
        }
        let envelope = PyDict::new(py);
        envelope.set_item(
            "api",
            model
                .getattr_opt("api")?
                .unwrap_or(1_i32.into_pyobject(py)?.into_any()),
        )?;
        for (k, attr) in [
            ("plugin", "key"),
            ("panel", "id"),
            ("title", "title"),
            ("slot", "slot"),
            ("instance", "instance"),
            ("document", "document"),
        ] {
            envelope.set_item(k, s.getattr(attr)?)?;
        }
        envelope.set_item("revision", revision)?;
        let json = py.import("json")?;
        let opts = PyDict::new(py);
        opts.set_item("allow_nan", false)?;
        let publish = || -> PyResult<()> {
            let sections = if visible {
                model.call_method0("sections")?
            } else {
                PyList::empty(py).into_any()
            };
            let clean = json.call_method("dumps", (&sections,), Some(&opts))?;
            let error = s.getattr("error")?;
            if error.is_truthy()? {
                let rows = py
                    .get_type::<PyList>()
                    .call1((sections,))?
                    .cast_into::<PyList>()?;
                rows.append(error_section(py, &error, false)?)?;
                envelope.set_item("sections", rows)?;
            } else {
                envelope.set_item("sections", sections)?;
            }
            renderer.call_method1(
                "ui_plugin_panel",
                (json.call_method("dumps", (&envelope,), Some(&opts))?,),
            )?;
            s.borrow_mut().last_sections = clean.extract()?;
            Ok(())
        };
        let Err(e) = publish() else { return Ok(()) };
        if !isolated(py, &e) {
            return Err(e);
        }
        envelope.set_item("api", 1)?;
        let slot = s.getattr("slot")?;
        if !slot.eq("dock")? && !slot.eq("utility")? {
            envelope.set_item("slot", "dock")?;
        }
        let unavailable = error_section(py, e.value(py).str()?.as_any(), true)?;
        let rows = json
            .call_method1("loads", (s.getattr("last_sections")?,))?
            .cast_into::<PyList>()?;
        rows.append(&unavailable)?;
        envelope.set_item("sections", rows)?;
        let report = || {
            renderer.call_method1(
                "ui_plugin_panel",
                (json.call_method1("dumps", (&envelope,))?,),
            )
        };
        match report() {
            Err(e) if e.is_instance_of::<PyValueError>(py) => {
                envelope.set_item("sections", PyList::new(py, [unavailable])?)?;
                report().map(drop)
            }
            other => other.map(drop),
        }
    }
}
