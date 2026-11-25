// SPDX-License-Identifier: GPL-3.0-or-later
//! History's change-feed listener. Nested steps contribute to the outer record.
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyInt, PyTuple},
};
type Obj<'py> = Bound<'py, PyAny>;
#[pyclass(subclass, module = "ingetrazo_core")]
pub(crate) struct HistoryFeed;
fn revision(feed: &Obj<'_>) -> PyResult<u64> {
    let h = feed.getattr("_history")?.call0()?;
    Ok(if h.is_none() {
        0
    } else {
        h.getattr("revision")?.extract()?
    })
}
#[pymethods]
impl HistoryFeed {
    #[new]
    #[pyo3(signature=(*_args,**_kw))]
    fn new(_args: &Bound<'_, PyTuple>, _kw: Option<&Bound<'_, PyDict>>) -> Self {
        Self
    }
    fn _connect(slf: &Bound<'_, Self>, history: &Obj<'_>) -> PyResult<()> {
        let py = slf.py();
        let native = history.getattr("_native")?;
        let reference = py.import("weakref")?.call_method1("ref", (history,))?;
        slf.setattr("_history", reference)?;
        slf.setattr("_subscribers", native.getattr("subscribers")?)?;
        slf.setattr("_native", native)?;
        let root = history.getattr("scene")?.getattr_opt("_root")?;
        let key = root
            .map(|r| r.getattr_opt("_doc_key"))
            .transpose()?
            .flatten();
        let readers = py
            .import("ingetrazo_core")?
            .call_method1("FeedReaders", (key,))?;
        slf.setattr("readers", &readers)?;
        slf.call_method1("subscribe", (readers,))?;
        slf.setattr("_pending", py.None())?;
        slf.setattr("_depth", 0)?;
        let kw = PyDict::new(py);
        kw.set_item("before", slf.getattr("_before")?)?;
        kw.set_item("after", slf.getattr("_after")?)?;
        slf.setattr(
            "unsubscribe",
            history.call_method("add_listener", (), Some(&kw))?,
        )
    }
    fn subscribe<'py>(slf: &Bound<'py, Self>, r#fn: &Obj<'py>) -> PyResult<Obj<'py>> {
        let py = slf.py();
        let subscribers = slf.getattr("_subscribers")?;
        let entry = pyo3::types::PyList::new(py, [r#fn])?;
        subscribers.call_method1("append", (&entry,))?;
        let owner = slf.clone().unbind();
        let entry = entry.unbind();
        let unsubscribe = pyo3::types::PyCFunction::new_closure(
            py,
            None,
            None,
            move |args, kwargs| -> PyResult<()> {
                if !args.is_empty() || kwargs.is_some_and(|kw| !kw.is_empty()) {
                    return Err(pyo3::exceptions::PyTypeError::new_err(
                        "unsubscribe takes no arguments",
                    ));
                }
                let list = owner.bind(args.py()).getattr("_subscribers")?;
                if list.contains(entry.bind(args.py()))? {
                    list.call_method1("remove", (entry.bind(args.py()),))?;
                }
                Ok(())
            },
        )?;
        Ok(unsubscribe.into_any())
    }
    #[getter]
    fn revision(slf: &Bound<'_, Self>) -> PyResult<u64> {
        revision(slf.as_any())
    }
    fn current(slf: &Bound<'_, Self>, reader: &Obj<'_>, version: &Obj<'_>) -> PyResult<bool> {
        let r = slf.getattr("readers")?;
        Ok(r.getattr("revision")?.eq(revision(slf.as_any())?)?
            && r.call_method1("current", (reader, version))?.is_truthy()?)
    }
    #[getter]
    fn last<'py>(slf: &Bound<'py, Self>) -> PyResult<Obj<'py>> {
        slf.getattr("_native")?
            .call_method1("feed_step", (slf.py().None(),))
    }
    fn step<'py>(slf: &Bound<'py, Self>, revision: &Obj<'py>) -> PyResult<Obj<'py>> {
        if revision.ge(0)? && revision.le(slf.getattr("revision")?)? {
            slf.getattr("_native")?
                .call_method1("feed_step", (revision,))
        } else {
            Ok(slf.py().None().into_bound(slf.py()))
        }
    }
    #[pyo3(signature=(revision=Python::attach(|py|0_i64.into_pyobject(py).unwrap().into_any().unbind())))]
    fn since<'py>(slf: &Bound<'py, Self>, revision: Py<PyAny>) -> PyResult<Obj<'py>> {
        let rev = revision.bind(slf.py());
        if !rev.is_exact_instance_of::<PyInt>() || rev.lt(0)? || rev.gt(slf.getattr("revision")?)? {
            return Err(PyValueError::new_err(
                "revision must be between zero and the current revision",
            ));
        }
        slf.getattr("_native")?.call_method1("feed_since", (rev,))
    }
    fn _before(slf: &Bound<'_, Self>, cmd: &Obj<'_>, phase: &str, ran: &Obj<'_>) -> PyResult<()> {
        let _ = ran;
        let depth = slf.getattr("_depth")?.extract::<usize>()? + 1;
        slf.setattr("_depth", depth)?;
        if depth > 1 {
            return Ok(());
        }
        let py = slf.py();
        let h = slf.getattr("_history")?.call0()?;
        let scene = if h.is_none() {
            py.None().into_bound(py)
        } else {
            h.getattr("scene")?
        };
        if scene.is_none() {
            return slf.setattr("_pending", py.None());
        }
        let serial = slf.getattr("readers")?.call_method0("stamp")?;
        let limit = crate::model::module(py, "core.protocol.feed")?
            .getattr("ATTRS_COPY_MAX")?
            .extract()?;
        let tokens = super::capture::tokens(cmd, &scene, phase, limit)?;
        slf.setattr(
            "_pending",
            (
                cmd,
                phase,
                tokens,
                scene.getattr_opt("content_version")?,
                serial,
            ),
        )
    }
    fn _after<'py>(
        slf: &Bound<'py, Self>,
        cmd: &Obj<'py>,
        phase: &str,
        ran: &Obj<'py>,
    ) -> PyResult<Option<Obj<'py>>> {
        let py = slf.py();
        let d = slf.getattr("_depth")?.extract::<usize>()?.saturating_sub(1);
        slf.setattr("_depth", d)?;
        if d != 0 {
            return Ok(None);
        }
        let pending = slf.getattr("_pending")?;
        slf.setattr("_pending", py.None())?;
        if !ran.is_truthy()? || pending.is_none() || !pending.get_item(0)?.is(cmd) {
            return Ok(None);
        }
        let h = slf.getattr("_history")?.call0()?;
        if h.is_none() {
            return Ok(None);
        }
        let step = super::build(py, &h, cmd, phase, &pending.get_item(2)?)?;
        let hint = |name: &str| -> PyResult<Obj<'py>> {
            Ok(cmd
                .getattr_opt(name)?
                .unwrap_or(false.into_pyobject(py)?.to_owned().into_any()))
        };
        step.setattr(
            "_reader_context",
            (
                pending.get_item(3)?,
                h.getattr("scene")?.getattr_opt("content_version")?,
                pending.get_item(4)?,
                hint("keeps_paint")?,
                hint("keeps_rows")?,
                hint("keeps_groups")?,
            ),
        )?;
        Ok(Some(
            slf.getattr("_native")?.call_method1("publish", (step,))?,
        ))
    }
}
