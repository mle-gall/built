// SPDX-License-Identifier: GPL-3.0-or-later
//! Assemble the public Python record at the transaction boundary. Mesh
//! facades still supply attribute dictionaries and compatibility command hints.
pub(super) mod capture;
mod hints;
use pyo3::prelude::*;
use pyo3::types::PyList;
use std::collections::{BTreeSet, HashSet};

fn keys(dict: &Bound<'_, PyAny>, field: &str) -> PyResult<Vec<String>> {
    dict.get_item(field)?.extract()
}

pub(super) fn build<'py>(
    py: Python<'py>,
    history: &Bound<'py, PyAny>,
    cmd: &Bound<'py, PyAny>,
    phase: &str,
    tokens: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let api = py.import("core.protocol.feed")?;
    let scene = history.getattr("scene")?;
    let name = cmd
        .getattr("history_name")
        .or_else(|_| cmd.get_type().getattr("__name__"))?;
    let step = api
        .getattr("Step")?
        .call1((history.getattr("revision")?, phase, name))?;
    let paints = hints::painted(cmd, &tokens.get_item(0)?.get_item(0)?)?;
    let meshes = step.getattr("meshes")?;
    let change_type = api.getattr("MeshChange")?;
    for pair in tokens.try_iter()? {
        let pair = pair?;
        let mesh = pair.get_item(0)?;
        let painted = paints.call_method1("get", (&mesh,))?;
        let change = capture::mesh_change(&change_type, &mesh, &pair.get_item(1)?, &painted)?;
        if painted.is_truthy()? {
            let modified = change.getattr("modified")?.get_item("face")?;
            let seen = py.import("builtins")?.getattr("set")?.call1((&modified,))?;
            for key in painted.try_iter()? {
                let key = key?;
                if !seen.contains(&key)? {
                    modified.call_method1("append", (&key,))?;
                    seen.call_method1("add", (key,))?;
                }
            }
        }
        if !change.call_method0("empty")?.is_truthy()? {
            meshes.call_method1("append", (change,))?;
        }
    }
    if phase != "undo" {
        step.setattr("copies", hints::copies(cmd)?)?;
    }
    let record = history.getattr("last_txn")?;
    let placements = step.getattr("placements")?;
    if !record.is_none() {
        let root = scene.getattr("_root")?.getattr("_doc_key")?;
        let changes = record
            .get_item(0)?
            .call_method1("changes", (record.get_item(1)?, root))?;
        let p = changes.get_item("placements")?;
        let t = changes.get_item("tags")?;
        let m = changes.get_item("materials")?;
        for field in ["created", "removed", "modified"] {
            placements.set_item(field, p.get_item(field)?)?;
            step.getattr("tags")?.set_item(
                field,
                keys(&t, field)?
                    .into_iter()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>(),
            )?;
        }
        let materials = step.getattr("materials")?;
        let registry = scene.getattr("materials")?;
        let mut membership = HashSet::new();
        for field in ["created", "removed"] {
            let names: BTreeSet<_> = keys(&m, field)?.into_iter().collect();
            membership.extend(names.iter().cloned());
            materials.set_item(field, Vec::from_iter(names))?;
        }
        let mut modified: BTreeSet<_> = keys(&m, "replaced")?.into_iter().collect();
        for name in keys(&m, "modified")? {
            if !membership.contains(&name) && registry.contains(&name)? {
                modified.insert(name);
            }
        }
        materials.set_item("modified", Vec::from_iter(modified))?;
        step.setattr("records", changes.get_item("scene")?.get_item("records")?)?;
        step.setattr(
            "scene_fields",
            changes.get_item("scene")?.get_item("modified")?,
        )?;
    }
    let known =
        api.getattr("_known_placements")?
            .call1((cmd, phase, &step, &scene, !record.is_none()))?;
    step.setattr(
        "complete",
        known.is_truthy()? && step.getattr("complete")?.is_truthy()?,
    )?;
    placements
        .get_item("modified")?
        .call_method1("extend", (history.getattr("_moved_signs")?,))?;
    // Command hints can overlap the journal. Preserve first occurrence order.
    let created = keys(&placements, "created")?;
    let removed = keys(&placements, "removed")?;
    let removed_set: HashSet<_> = removed.iter().collect();
    let both: BTreeSet<_> = created
        .iter()
        .filter(|k| removed_set.contains(k))
        .cloned()
        .collect();
    for (field, values) in [
        ("created", created),
        ("removed", removed),
        ("modified", keys(&placements, "modified")?),
    ] {
        let mut seen = HashSet::new();
        let list = PyList::empty(py);
        for key in values
            .iter()
            .chain(both.iter().filter(|_| field == "modified"))
        {
            if (field == "modified" || !both.contains(key)) && seen.insert(key) {
                list.append(key)?;
            }
        }
        placements.set_item(field, list)?;
    }
    Ok(step)
}

/// Last successful scan for each reader. A scan can be advanced only across
/// contiguous published steps and matching content versions. This also
/// catches preview and plugin writes between commands.
#[pyclass(module = "ingetrazo_core")]
#[derive(Default)]
pub struct FeedReaders {
    scans: std::collections::HashMap<String, (i64, u64)>,
    root: Option<super::Key>,
    #[pyo3(get)]
    revision: u64,
    browser_version: i64,
    filing_epoch: u64,
    dirty_paint: HashSet<String>,
    dirty_library_paint: HashSet<String>,
}

#[pymethods]
impl FeedReaders {
    #[new]
    fn new(root: Option<super::Key>) -> Self {
        if let Some(key) = root {
            super::lock().reader_epochs.entry(key).or_default();
        }
        Self {
            root,
            ..Self::default()
        }
    }

    /// A plugin can mutate a bound field without executing a command or
    /// bumping the scene version. Flush tracked Python containers before
    /// sampling the store's write serial at a reader boundary.
    fn stamp(&self, py: Python<'_>) -> PyResult<u64> {
        let Some(root) = self.root else {
            return Ok(0);
        };
        py.import("core.doc_model")?.call_method0("flush")?;
        Ok(super::lock().reader_epochs.get(&root).copied().unwrap_or(0))
    }

    fn mark(&mut self, py: Python<'_>, reader: String, version: i64) -> PyResult<()> {
        let serial = self.stamp(py)?;
        if reader == "mesh_paint" {
            self.dirty_paint.clear();
        } else if reader == "library_paint" {
            self.dirty_library_paint.clear();
        }
        self.scans.insert(reader, (version, serial));
        Ok(())
    }

    fn dirty_paint_meshes(&self) -> Vec<String> {
        self.dirty_paint.iter().cloned().collect()
    }

    fn dirty_library_paint_meshes(&self) -> Vec<String> {
        self.dirty_library_paint.iter().cloned().collect()
    }

    fn current(&self, py: Python<'_>, reader: &str, version: i64) -> PyResult<bool> {
        Ok(self.scans.get(reader) == Some(&(version, self.stamp(py)?)))
    }

    /// Each pane can cache its own projection against this shared membership revision.
    fn browser_version(&mut self, py: Python<'_>, version: i64) -> PyResult<i64> {
        // Avoid flushing bound containers on a frame with no document edits.
        if self.scans.get("browser").map(|stamp| stamp.0) != Some(version) {
            self.mark(py, "browser".into(), version)?;
            self.browser_version = version;
        }
        Ok(self.browser_version)
    }

    /// Multiple panes can consume an insert, removal or context transition
    /// independently. Any intervening unaccounted write starts a new epoch.
    fn filing_epoch(&mut self, py: Python<'_>, version: i64) -> PyResult<u64> {
        if self.scans.get("filing").map(|s| s.0) != Some(version) {
            self.mark(py, "filing".into(), version)?;
            self.filing_epoch += 1;
        }
        Ok(self.filing_epoch)
    }

    fn context_changed(
        &mut self,
        py: Python<'_>,
        before: i64,
        serial: u64,
        after: i64,
    ) -> PyResult<()> {
        let now = self.stamp(py)?;
        for (reader, stamp) in &mut self.scans {
            if *stamp == (before, serial) && matches!(reader.as_str(), "mesh_paint" | "library_paint" | "filing")
            {
                *stamp = (after, now);
            }
        }
        Ok(())
    }

    fn __call__(&mut self, step: &Bound<'_, PyAny>) -> PyResult<()> {
        let revision = step.getattr("revision")?.extract()?;
        let contiguous = self.revision + 1 == revision;
        self.revision = revision;
        if !contiguous || !step.getattr("complete")?.is_truthy()? {
            self.scans.clear();
            return Ok(());
        }
        let (before, after, serial, paint, rows, groups): (
            Option<i64>,
            Option<i64>,
            u64,
            bool,
            bool,
            bool,
        ) = step.getattr("_reader_context")?.extract()?;
        let (Some(before), Some(after)) = (before, after) else {
            self.scans.clear();
            return Ok(());
        };
        let after_serial = self
            .root
            .and_then(|k| super::lock().reader_epochs.get(&k).copied())
            .unwrap_or(0);
        let placements = step.getattr("placements")?;
        let modified = placements.get_item("modified")?.is_truthy()?;
        let membership = placements.get_item("created")?.is_truthy()?
            || placements.get_item("removed")?.is_truthy()?;
        let materials = step.getattr("materials")?;
        let mut registry = false;
        for kind in ["created", "removed", "modified"] {
            registry |= materials.get_item(kind)?.is_truthy()?;
        }
        let changes = step.getattr("meshes")?;
        let keyed_paint = changes.is_truthy()?
            && !registry
            && changes.try_iter()?.all(|change| {
                change
                    .and_then(|c| c.getattr("backing")?.eq("store"))
                    .unwrap_or(false)
            });
        if keyed_paint {
            for (reader, pending) in [("mesh_paint", &mut self.dirty_paint),
                                       ("library_paint", &mut self.dirty_library_paint)] {
                if self.scans.get(reader) == Some(&(before, serial)) {
                    for change in changes.try_iter()? {
                        pending.insert(change?.getattr("key")?.extract()?);
                    }
                }
            }
        }
        let command: String = step.getattr("command")?.extract()?;
        let tags = step.getattr("tags")?;
        let mut tag_changes = false;
        for field in ["created", "removed", "modified"] {
            tag_changes |= tags.get_item(field)?.is_truthy()?;
        }
        self.scans.retain(|reader, version| {
            let keep = match reader.as_str() {
                "paint" | "bim" | "library_paint_totals" => paint && !registry && !membership && !tag_changes,
                "rows" => (paint || rows) && !membership && !tag_changes && !registry,
                "mesh_paint" | "library_paint" => (paint || groups || keyed_paint) && !registry,
                "filing" => !registry && !tag_changes && (
                    matches!(command.as_str(), "DeleteGroupCommand" | "InsertGroupCommand")
                    || (command == "MoveGroupCommand" && paint)),
                "browser" => !membership && !tag_changes && !registry && (
                    (command == "MoveGroupCommand" && paint) ||
                    (!modified && matches!(command.as_str(),
                        "SnapshotMutation" | "SnapshotCompound" | "MoveVerticesCommand"
                        | "AddEdgeCommand" | "AddFaceCommand" | "EraseSelectionCommand"
                        | "DeleteEdgesCommand" | "DeleteFaceCommand"))
                ),
                "structure" => {
                    !modified
                        && !tag_changes
                        && !membership
                        && matches!(
                            command.as_str(),
                            "SnapshotMutation"
                                | "SnapshotCompound"
                                | "AddEdgeCommand"
                                | "AddFaceCommand"
                                | "AddDimensionCommand"
                                | "AddTextLabelCommand"
                                | "AddGuideCommand"
                        )
                }
                _ => false,
            };
            if keep && *version == (before, serial) {
                *version = (after, after_serial);
                true
            } else {
                false
            }
        });
        Ok(())
    }
}
