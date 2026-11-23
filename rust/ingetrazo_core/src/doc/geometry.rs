//! Canonical geometry in a document savepoint. Capture the facade along with
//! the store's COW columns: face attribute dictionaries still live in Python.
use crate::store::py::{PyMeshStore, PyStoreSnapshot};
use pyo3::gc::{PyTraverseError, PyVisit};
use pyo3::prelude::*;
use std::cell::RefCell;

struct Entry {
    lineage: u64,
    mesh: Py<PyAny>,
    before: Py<PyAny>,
}

pub(super) struct Edit {
    entry: Entry,
    after: Py<PyAny>,
}

thread_local! {
    static LEVELS: RefCell<Vec<Vec<Entry>>> = const { RefCell::new(Vec::new()) };
}

pub(super) fn begin() {
    LEVELS.with_borrow_mut(|levels| levels.push(Vec::new()));
}

/// Keep native column snapshots in the original definition frame. Attribute
/// tables still belong to the facade. A legacy kernel can materialise an
/// object graph mid-command, in which case that graph is the after state.
fn capture(mesh: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let Some(store) = mesh.getattr_opt("_store")? else {
        return Ok(mesh.call_method0("capture_state")?.unbind());
    };
    let snapshot = store.cast::<PyMeshStore>()?.try_borrow()?.snapshot();
    Ok(mesh.call_method1("_journal_state", (snapshot,))?.unbind())
}

pub(super) fn touch(mesh: &Bound<'_, PyAny>, initial: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
    let owner = mesh
        .getattr_opt("_instance_definition")?
        .unwrap_or_else(|| mesh.clone());
    let lineage = owner
        .getattr("_store")?
        .cast::<PyMeshStore>()?
        .try_borrow()?
        .inner
        .lineage;
    let needs = LEVELS.with_borrow(|levels| {
        levels
            .last()
            .is_some_and(|level| !level.iter().any(|e| e.lineage == lineage))
    });
    if needs {
        let before = match initial {
            Some(state) if !owner.is(mesh) => {
                let snap = state.getattr("snap")?.cast_into::<PyStoreSnapshot>()?;
                let exact = snap.get().canonical();
                state
                    .get_type()
                    .call1((
                        exact,
                        state.getattr("own")?,
                        state.getattr("table")?,
                        &owner,
                    ))?
                    .unbind()
            }
            Some(state) => state.clone().unbind(),
            None => capture(&owner)?,
        };
        LEVELS.with_borrow_mut(|levels| {
            if let Some(level) = levels.last_mut() {
                if !level.iter().any(|e| e.lineage == lineage) {
                    level.push(Entry {
                        lineage,
                        mesh: owner.unbind(),
                        before,
                    });
                }
            }
        });
    }
    Ok(())
}

pub(super) fn commit(py: Python<'_>) -> PyResult<Vec<Edit>> {
    let targets = LEVELS.with_borrow(|levels| match levels.as_slice() {
        [level] => level.iter().map(|e| e.mesh.clone_ref(py)).collect(),
        _ => Vec::new(),
    });
    // Keep the savepoint until every capture succeeds so History can roll back.
    let after = targets
        .iter()
        .map(|m| capture(m.bind(py)))
        .collect::<PyResult<Vec<_>>>()?;
    let level = LEVELS.with_borrow_mut(|levels| levels.pop().unwrap_or_default());
    if LEVELS.with_borrow(|levels| !levels.is_empty()) {
        // A released savepoint contributes its first write only when the
        // enclosing level has not already captured that definition.
        let mut discarded = Vec::new();
        LEVELS.with_borrow_mut(|levels| {
            let parent = levels.last_mut().unwrap();
            for entry in level {
                if parent.iter().any(|e| e.lineage == entry.lineage) {
                    discarded.push(entry);
                } else {
                    parent.push(entry);
                }
            }
        });
        return Ok(Vec::new());
    }
    Ok(level
        .into_iter()
        .zip(after)
        .map(|(entry, after)| Edit { entry, after })
        .collect())
}

pub(super) fn rollback(py: Python<'_>) -> PyResult<()> {
    let level = LEVELS.with_borrow_mut(|levels| levels.pop().unwrap_or_default());
    for e in level.into_iter().rev() {
        e.mesh.call_method1(py, "restore_state", (e.before,))?;
    }
    Ok(())
}

impl Edit {
    pub(super) fn append_to(&self, py: Python<'_>, edits: &mut Vec<Edit>) {
        if let Some(existing) = edits
            .iter_mut()
            .find(|e| e.entry.lineage == self.entry.lineage)
        {
            existing.after = self.after.clone_ref(py);
        } else {
            edits.push(Edit {
                entry: Entry {
                    lineage: self.entry.lineage,
                    mesh: self.entry.mesh.clone_ref(py),
                    before: self.entry.before.clone_ref(py),
                },
                after: self.after.clone_ref(py),
            });
        }
    }

    pub(super) fn lineage(&self) -> u64 {
        self.entry.lineage
    }

    pub(super) fn states(&self, py: Python<'_>, lineage: u64) -> Option<(Py<PyAny>, Py<PyAny>)> {
        (self.entry.lineage == lineage)
            .then(|| (self.entry.before.clone_ref(py), self.after.clone_ref(py)))
    }

    pub(super) fn apply(&self, py: Python<'_>, backwards: bool) -> PyResult<()> {
        let state = if backwards {
            &self.entry.before
        } else {
            &self.after
        };
        self.entry
            .mesh
            .call_method1(py, "restore_state", (state,))?;
        Ok(())
    }

    pub(super) fn traverse(&self, visit: &PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.entry.mesh)?;
        visit.call(&self.entry.before)?;
        visit.call(&self.after)
    }
}
