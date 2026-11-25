// SPDX-License-Identifier: GPL-3.0-or-later
//! Per-document history. Stack entries are host commands while some tools
//! still implement Python inverses. Existing scripting clients edit these
//! lists directly when folding a group-edit session or cancelling a preview.
mod dispatch;
pub(crate) use dispatch::in_frame;
pub(crate) mod lifecycle;
use super::py::PyTransaction;
use std::collections::VecDeque;

use pyo3::class::gc::{PyTraverseError, PyVisit};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

/// Re-enter the geometry on which a step was recorded, even after the
/// editor has left that placement. Restore the active mesh on exceptions.
fn in_command_mesh<'py>(
    scene: &Bound<'py,PyAny>, cmd: &Bound<'py,PyAny>, action: &Bound<'py,PyAny>,
    args: impl pyo3::call::PyCallArgs<'py>,
) -> PyResult<()> {
    let current = scene.getattr("mesh")?;
    let Some(target) = cmd.getattr_opt("_history_mesh")?.filter(|m| !m.is_none() && !m.is(&current)) else {
        action.call1(args)?;
        return Ok(());
    };
    target.setattr("_mut_serial",target.getattr("_mut_serial")?.add(1)?)?;
    scene.setattr("mesh",target)?;
    let result = action.call1(args);
    scene.setattr("mesh",current)?;
    result.map(|_| ())
}

/// Journal a direct inverse for the change feed. Commands with host-owned
/// effects run in their original editor mesh just like transaction replays.
fn replay_direct(
    py: Python<'_>,
    host: &Bound<'_, PyAny>,
    cmd: &Bound<'_, PyAny>,
    undo: bool,
) -> PyResult<()> {
    let scene = host.getattr("scene")?;
    let objects = if undo {None} else {Some(lifecycle::objects_before(&scene,!cmd.getattr("keeps_rows")?.is_truthy()?)?)};
    let action = cmd.getattr(if undo { "undo" } else { "do" })?;
    let doc = py.import("core.doc_model")?;
    doc.call_method1("begin", (scene.getattr("_root")?.getattr("_doc_key")?,))?;
    host.setattr("last_txn", py.None())?;
    let result = (|| {
        in_command_mesh(&scene, cmd, &action, (&scene,))?;
        lifecycle::follow_text(host)?;
        doc.call_method0("commit")
    })();
    match result {
        Ok(txn) if !txn.is_none() => host.setattr("last_txn", (txn, false))?,
        Ok(_) => (),
        Err(err) => {
            doc.call_method0("rollback")?;
            return Err(err);
        }
    }
    if let Some(objects) = objects {
        objects.apply(&scene)?;
    }
    Ok(())
}

#[pyclass(module = "ingetrazo_core")]
pub struct DocumentHistory {
    #[pyo3(get)]
    undo: Py<PyList>,
    #[pyo3(get)]
    redo: Py<PyList>,
    #[pyo3(get)]
    subscribers: Py<PyList>,
    #[pyo3(get)]
    revision: u64,
    #[pyo3(get)]
    units_revision: u64,
    steps: VecDeque<(u64, Py<PyAny>)>,
    journal: VecDeque<(u64, String, String)>,
}

#[pymethods]
impl DocumentHistory {
    #[new]
    fn new(py: Python<'_>) -> Self {
        Self {
            undo: PyList::empty(py).unbind(),
            redo: PyList::empty(py).unbind(),
            subscribers: PyList::empty(py).unbind(),
            revision: 0,
            units_revision: 0,
            steps: VecDeque::new(),
            journal: VecDeque::new(),
        }
    }

    #[staticmethod]
    fn run<'py>(host:&Bound<'py,PyAny>, command:&Bound<'py,PyAny>, phase:&str) -> PyResult<Bound<'py,PyAny>> {
        dispatch::run(host, command, phase)
    }

    #[staticmethod]
    fn transaction_covers(commands:&Bound<'_,PyAny>, scene:&Bound<'_,PyAny>) -> PyResult<bool> {
        if !commands.is_truthy()? {return Ok(false);}
        for c in commands.try_iter()? {
            if !super::journal_plan::uses_transaction(&c?,scene)? {return Ok(false);}
        }
        Ok(true)
    }

    #[staticmethod]
    fn refresh_dimensions(scene: &Bound<'_,PyAny>) -> PyResult<()> {
        if let Some(dims) = scene.getattr_opt("dimensions")?.filter(|v| !v.is_none()) {
            for dim in dims.try_iter()? {
                if let Some(refresh) = dim?.getattr_opt("refresh_anchors")?.filter(|v| !v.is_none()) {
                    refresh.call1((scene,))?;
                }
            }
        }
        Ok(())
    }

    #[staticmethod]
    #[pyo3(signature=(scene,include_groups=true))]
    fn objects_before<'py>(scene:&Bound<'py,PyAny>,include_groups:bool)->PyResult<Bound<'py,PyAny>> {
        Ok(lifecycle::objects_before(scene,include_groups)?.before)
    }

    fn push(&self, py: Python<'_>, command: &Bound<'_, PyAny>, limit: usize) -> PyResult<()> {
        let undo = self.undo.bind(py);
        undo.append(command)?;
        if limit > 0 && undo.len() > limit {
            // While editing, the whole open suffix will count as one entry.
            // Retention must never discard its earliest transaction.
            let mut logical_len = undo.len();
            if let Ok(path) = command.getattr("_occurrences") {
                if path.len()? > 0 {
                    logical_len = self.group_start(py, &path.get_item(0)?)? + 1;
                }
            }
            undo.del_slice(0, logical_len.saturating_sub(limit))?;
        }
        let redo = self.redo.bind(py);
        redo.del_slice(0, redo.len())
    }

    /// Compose the contiguous suffix belonging to this edit context. Construct
    /// the replacement before touching the list, including for nested groups.
    fn fold_occurrence(
        &self,
        py: Python<'_>,
        session: &Bound<'_, PyAny>,
        factory: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let stack = self.undo.bind(py);
        let end = stack.len();
        let start = self.group_start(py, session)?;
        if end - start < 2 {
            return Ok(());
        }
        let parts = stack.get_slice(start, end);
        let group = factory.call1((&parts,))?;
        group.setattr("_occurrences", parts.get_item(0)?.getattr("_occurrences")?)?;
        let mut records = Vec::new();
        for cmd in parts.iter() {
            if let Ok(txn) = cmd
                .getattr("_doc_txn")
                .and_then(|t| Ok(t.extract::<Py<PyTransaction>>()?))
            {
                records.push(txn);
            } else {
                break;
            }
        }
        if records.len() == parts.len() {
            group.setattr("_doc_txn", PyTransaction::compose(py, records)?)?;
        }
        stack.set_slice(start, end, PyList::new(py, [group])?.as_any())
    }

    fn group_start(&self, py: Python<'_>, session: &Bound<'_, PyAny>) -> PyResult<usize> {
        let stack = self.undo.bind(py);
        let mut start = stack.len();
        while start > 0 {
            let cmd = stack.get_item(start - 1)?;
            if !cmd.hasattr("_occurrences")? || !cmd.getattr("_occurrences")?.contains(session)? {
                break;
            }
            start -= 1;
        }
        Ok(start)
    }

    fn close_group(
        slf: PyRef<'_, Self>,
        py: Python<'_>,
        host: &Bound<'_, PyAny>,
        start: usize,
        command: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let stack = slf.undo.bind(py).clone();
        drop(slf);
        let end = stack.len();
        let parts = stack.get_slice(start, end);
        stack.del_slice(start, end)?;
        let result = host.call_method1("execute", (command,));
        if result.is_err()
            || host.getattr("last_error")?.is_truthy()?
            || host.getattr("last_rejected")?.is_truthy()?
        {
            stack.call_method1("extend", (parts,))?;
        }
        result.map(|_| ())
    }

    /// Release the History borrow before host code (including extension
    /// inverses) runs. A failed dispatch restores the source stack.
    fn replay(
        slf: PyRef<'_, Self>,
        py: Python<'_>,
        host: &Bound<'_, PyAny>,
        undo: bool,
        sheet: bool,
    ) -> PyResult<()> {
        let document = if sheet { None } else {
            Some(host.getattr("scene")?.getattr("_root")?.getattr("_doc_key")?.extract()?)
        };
        let _writer = super::Writer::acquire(py, document)?;
        let (source, dest) = if undo {
            (&slf.undo, &slf.redo)
        } else {
            (&slf.redo, &slf.undo)
        };
        let source = source.bind(py).clone();
        let dest = dest.bind(py).clone();
        drop(slf);
        let cmd = source.call_method0("pop")?;

        let result: PyResult<()> = (|| {
            let record = cmd.getattr(if sheet { "_txn" } else { "_doc_txn" });
            if let Some(txn) = record.ok().filter(|v| !v.is_none()) {
                py.import("core.doc_model")?.call_method0("flush")?;
                if sheet {
                    txn.cast::<PyTransaction>()?.try_borrow()?.run(py, undo)?;
                } else {
                    let scene = host.getattr("scene")?;
                    in_command_mesh(&scene, &cmd, &txn.getattr("replay_editor")?, (&cmd, &scene, undo))?;
                    host.setattr("last_txn", (txn, undo))?;
                }
            } else if let Some(group) = cmd.getattr_opt("_mixed_group")? {
                let doc = py.import("core.doc_model")?;
                doc.call_method0("flush")?;
                let scene = host.getattr("scene")?;
                in_command_mesh(&scene, &cmd, &group.getattr(if undo { "undo" } else { "do" })?, (&scene,))?;
                doc.call_method0("flush")?;
                host.setattr("last_txn", (cmd.getattr("_group_diff")?, undo))?;
            } else if sheet {
                cmd.call_method0(if undo { "undo" } else { "do" })?;
            } else {
                replay_direct(py, host, &cmd, undo)?;
            }
            Ok(())
        })();
        if let Err(err) = result {
            source.append(cmd)?;
            return Err(err);
        }
        dest.append(cmd)
    }

    /// Mixed occurrence spans enter the tool's original mesh frame for
    /// each native record or extension inverse. Undo completed parts on error.
    #[staticmethod]
    fn replay_group(
        group: &Bound<'_, PyAny>,
        scene: &Bound<'_, PyAny>,
        undo: bool,
        hooks_only: bool,
    ) -> PyResult<()> {
        let parts = group.getattr("commands")?.cast_into::<PyList>()?;
        let dispatch = |cmd: &Bound<'_, PyAny>, backwards: bool| -> PyResult<()> {
            let phase = if backwards { "undo" } else { "redo" };
            let frame = group.getattr("_in_frame")?;
            if hooks_only {
                frame.call1((scene, cmd, cmd.getattr("doc_after")?, scene, phase))?;
            } else if let Some(txn) = cmd.getattr_opt("_doc_txn")?.filter(|v| !v.is_none()) {
                scene.py().import("core.doc_model")?.call_method0("flush")?;
                frame.call1((
                    scene,
                    cmd,
                    txn.getattr("replay_editor")?,
                    cmd,
                    scene,
                    backwards,
                ))?;
            } else if let Some(mixed) = cmd.getattr_opt("_mixed_group")? {
                frame.call1((
                    scene,
                    cmd,
                    mixed.getattr(if backwards { "undo" } else { "do" })?,
                    scene,
                ))?;
            } else {
                frame.call1((
                    scene,
                    cmd,
                    cmd.getattr(if backwards { "undo" } else { "do" })?,
                    scene,
                ))?;
            }
            Ok(())
        };
        for i in 0..parts.len() {
            let index = |j| if undo { parts.len() - 1 - j } else { j };
            if let Err(error) = dispatch(&parts.get_item(index(i))?, undo) {
                if !hooks_only {
                    for j in (0..i).rev() {
                        dispatch(&parts.get_item(index(j))?, !undo)?;
                    }
                }
                return Err(error);
            }
        }
        Ok(())
    }

    fn push_sheet(&self, py: Python<'_>, cmd: &Bound<'_, PyAny>, coalesce: bool) -> PyResult<()> {
        let stack = self.undo.bind(py);
        if coalesce && !stack.is_empty() {
            use crate::sheets::composition::{edit_signature, edits_of};
            let top = stack.get_item(stack.len() - 1)?;
            let fields = edit_signature(&top)?;
            if !fields.is_empty() && fields == edit_signature(cmd)? {
                let old = top.getattr("_txn")?;
                let new = cmd.getattr("_txn")?;
                let combined = if old.is_none() || new.is_none() {
                    py.None()
                } else {
                    Py::new(
                        py,
                        PyTransaction::compose(py, vec![old.extract()?, new.extract()?])?,
                    )?
                    .into_any()
                };
                for (first,last) in edits_of(&top)?.into_iter().zip(edits_of(cmd)?) {
                    first.setattr("after", last.getattr("after")?.call_method0("copy")?)?;
                }
                top.setattr("_txn", combined)?;
                return self.redo.bind(py).call_method0("clear").map(|_| ());
            }
        }
        self.push(py, cmd, 0)
    }

    fn clear_stacks(&mut self, py: Python<'_>) -> PyResult<()> {
        self.undo.bind(py).call_method0("clear")?;
        self.redo.bind(py).call_method0("clear")?;
        // Opening into an existing scene retains its History and renderer.
        self.units_revision += 1;
        Ok(())
    }

    /// Drop the mutable History borrow before invoking arbitrary subscribers.
    fn publish(
        mut slf: PyRefMut<'_, Self>,
        py: Python<'_>,
        step: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let revision = slf.revision;
        if step.getattr("scene_fields")?.contains("units")? {
            slf.units_revision += 1;
        }
        let expired = (slf.steps.len() == 256).then(|| slf.steps.pop_front());
        slf.steps.push_back((revision, step.clone().unbind()));
        let subscribers = slf.subscribers.bind(py).to_tuple();
        drop(slf);
        drop(expired);
        for entry in subscribers.iter() {
            let callback = entry.get_item(0)?;
            if let Err(err) = callback.call1((step,)) {
                let logger = py
                    .import("logging")?
                    .call_method1("getLogger", ("ingetrazo.protocol",))?;
                logger.call_method1(
                    "error",
                    ("Change feed subscriber failed: %s", err.to_string()),
                )?;
            }
        }
        Ok(())
    }

    fn feed_since<'py>(&self, py: Python<'py>, since: u64) -> PyResult<Bound<'py, PyDict>> {
        let result = PyDict::new(py);
        let floor = self.steps.front().map_or(self.revision, |(r, _)| r - 1);
        let steps = PyList::empty(py);
        for (_, step) in self.steps.iter().filter(|(r, _)| *r > since) {
            steps.append(step)?;
        }
        result.set_item("complete", since >= floor)?;
        result.set_item("revision", self.revision)?;
        result.set_item("steps", steps)?;
        Ok(result)
    }

    fn feed_step(&self, py: Python<'_>, revision: Option<u64>) -> Option<Py<PyAny>> {
        self.steps
            .iter()
            .rev()
            .find(|(r, _)| revision.is_none_or(|rev| rev == *r))
            .map(|(_, step)| step.clone_ref(py))
    }

    #[staticmethod]
    fn build_step<'py>(
        py: Python<'py>,
        history: &Bound<'py, PyAny>,
        cmd: &Bound<'py, PyAny>,
        phase: &str,
        tokens: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        super::feed::build(py, history, cmd, phase, tokens)
    }

    fn advance(&mut self, phase: String, command: String) {
        self.revision += 1;
        if self.journal.len() == 256 {
            self.journal.pop_front();
        }
        self.journal.push_back((self.revision, phase, command));
    }

    fn changes_since<'py>(&self, py: Python<'py>, since: u64) -> PyResult<Bound<'py, PyDict>> {
        let entries = PyList::empty(py);
        for (rev, phase, command) in self.journal.iter().filter(|(r, _, _)| *r > since) {
            let entry = PyDict::new(py);
            entry.set_item("revision", rev)?;
            entry.set_item("phase", phase)?;
            entry.set_item("command", command)?;
            entries.append(entry)?;
        }
        let result = PyDict::new(py);
        let floor = self
            .journal
            .front()
            .map_or(self.revision, |entry| entry.0 - 1);
        result.set_item("revision", self.revision)?;
        result.set_item("complete", since >= floor)?;
        result.set_item("changes", entries)?;
        Ok(result)
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        for (_, step) in &self.steps {
            visit.call(step)?;
        }
        visit.call(&self.undo)?;
        visit.call(&self.redo)?;
        visit.call(&self.subscribers)
    }
}
