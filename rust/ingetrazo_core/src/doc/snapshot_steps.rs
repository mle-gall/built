// SPDX-License-Identifier: GPL-3.0-or-later
//! Snapshot and composite operations. The Python command constructors own the
//! plugin-visible fields so subclasses can initialise or replace them.
mod import;
use pyo3::{
    prelude::*,
    types::{PyDict, PySet, PyTuple},
};
type Obj<'py> = Bound<'py, PyAny>;

fn target<'py>(step: &Obj<'py>, scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    Ok(match step.getattr_opt("_mesh")?.filter(|m| !m.is_none()) {
        Some(m) => m,
        None => scene.getattr("mesh")?,
    })
}
fn restore(step: &Obj<'_>, field: &str, scene: &Obj<'_>, mesh: &Obj<'_>) -> PyResult<()> {
    let state = step.getattr(field)?;
    if !state.is_none() {
        mesh.call_method1("restore_state", (state,))?;
        super::steps::version(scene)?;
    }
    Ok(())
}
fn capture(step: &Obj<'_>, field: &str, mesh: &Obj<'_>) -> PyResult<()> {
    step.setattr(field, step.call_method1("_direct_snapshot", (mesh,))?)
}
#[pyclass(subclass, module = "ingetrazo_core")]
struct MutationStep;
#[pymethods]
impl MutationStep {
    #[new]
    #[pyo3(signature=(*_args,**_kw))]
    fn new(_args: &Bound<'_, PyTuple>, _kw: Option<&Bound<'_, PyDict>>) -> Self {
        Self
    }
    fn r#do(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<()> {
        let s = slf.as_any();
        let m = target(s, scene)?;
        if !s.getattr("after")?.is_none() {
            return restore(s, "after", scene, &m);
        }
        capture(s, "before", &m)?;
        s.call_method1("mutate", (scene,))?;
        capture(s, "after", &m)?;
        super::steps::version(scene)
    }
    fn undo(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<()> {
        restore(slf.as_any(), "before", scene, &target(slf.as_any(), scene)?)
    }
    fn geometry_meshes<'py>(slf: &Bound<'py, Self>, scene: &Obj<'py>) -> PyResult<(Obj<'py>,)> {
        Ok((target(slf.as_any(), scene)?,))
    }
}

fn run_parts(parent: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let native = parent.py().import("ingetrazo_core")?;
    let face = native.getattr("AddFaceCommand")?;
    let journal = parent.getattr("_journal_geometry")?.is_truthy()?;
    let covered = if journal {
        [
            "AddEdgeCommand",
            "DeleteEdgesCommand",
            "DeleteFaceCommand",
            "TagCurveCommand",
            "ExplodeCurvesCommand",
            "PruneOrphanEdgesCommand",
            "MoveVerticesCommand",
        ]
        .iter()
        .map(|n| native.getattr(*n))
        .collect::<PyResult<Vec<_>>>()?
    } else {
        Vec::new()
    };
    for part in parent.getattr("inner")?.try_iter()? {
        let part = part?;
        let kind = part.get_type();
        if kind.is(&face) || covered.iter().any(|c| kind.is(c)) {
            let prior = part.getattr("_journal_geometry")?;
            part.setattr("_journal_geometry", true)?;
            let result = part.call_method1("do", (scene,));
            part.setattr("_journal_geometry", prior)?;
            result?;
        } else {
            part.call_method1("do", (scene,))?;
        }
    }
    Ok(())
}
#[pyclass(subclass, module = "ingetrazo_core")]
struct DrawingSnapshotStep;
#[pymethods]
impl DrawingSnapshotStep {
    #[new]
    #[pyo3(signature=(*_args,**_kw))]
    fn new(_args: &Bound<'_, PyTuple>, _kw: Option<&Bound<'_, PyDict>>) -> Self {
        Self
    }
    #[pyo3(signature=(scene,*,transaction_captured_before=false))]
    fn r#do(
        slf: &Bound<'_, Self>,
        scene: &Obj<'_>,
        transaction_captured_before: bool,
    ) -> PyResult<()> {
        let s = slf.as_any();
        let mesh = scene.getattr("mesh")?;
        if !s.getattr("after")?.is_none() {
            return restore(s, "after", scene, &mesh);
        }
        if !transaction_captured_before {
            capture(s, "before", &mesh)?;
        }
        let topology = s.py().import("core.topology")?;
        let heal = s.getattr("_heal")?.is_truthy()?;
        let footprint = if heal {
            Some(topology.call_method1("winding_footprint", (&mesh,))?)
        } else {
            None
        };
        run_parts(s, scene)?;
        if let Some(f) = footprint {
            let kw = PyDict::new(s.py());
            kw.set_item("footprint", f)?;
            let removed = topology.call_method("heal_overlapping_faces", (&mesh,), Some(&kw))?;
            for face in removed.try_iter()? {
                scene
                    .getattr("selection")?
                    .call_method1("discard", (face?,))?;
            }
        }
        mesh.call_method0("resplit_curves")?;
        capture(s, "after", &mesh)?;
        super::steps::version(scene)
    }
    fn undo(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<()> {
        restore(slf.as_any(), "before", scene, &scene.getattr("mesh")?)
    }
}

#[pyclass(subclass, module = "ingetrazo_core")]
struct StitchSnapshotStep;
#[pymethods]
impl StitchSnapshotStep {
    #[new]
    #[pyo3(signature=(*_args,**_kw))]
    fn new(_args: &Bound<'_, PyTuple>, _kw: Option<&Bound<'_, PyDict>>) -> Self {
        Self
    }
    fn r#do(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<()> {
        let py = slf.py();
        let s = slf.as_any();
        let mesh = scene.getattr("mesh")?;
        capture(s, "snapshot", &mesh)?;
        let set = py.get_type::<PySet>();
        let before = set.call1((mesh.getattr("faces")?,))?;
        run_parts(s, scene)?;
        let fresh = set
            .call1((mesh.getattr("faces")?,))?
            .call_method1("difference", (before,))?;
        let key = py.import("core.topology")?.getattr("_key")?;
        let seeds = PySet::empty(py)?;
        for p in s.getattr("seed")?.try_iter()? {
            seeds.add(key.call1((p?,))?)?;
        }
        crate::doc::history_support::run_stitch(&mesh,&seeds,Some(&fresh),true,true)?;
        super::steps::version(scene)
    }
    fn undo(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<()> {
        restore(slf.as_any(), "snapshot", scene, &scene.getattr("mesh")?)
    }
}

#[pyclass(subclass, module = "ingetrazo_core")]
struct CompoundStep;
#[pymethods]
impl CompoundStep {
    #[new]
    #[pyo3(signature=(*_args,**_kw))]
    fn new(_args: &Bound<'_, PyTuple>, _kw: Option<&Bound<'_, PyDict>>) -> Self {
        Self
    }
    #[getter]
    fn keeps_rows(slf: &Bound<'_, Self>) -> PyResult<bool> {
        keeps(slf.as_any(), "keeps_rows")
    }
    #[getter]
    fn keeps_groups(slf: &Bound<'_, Self>) -> PyResult<bool> {
        keeps(slf.as_any(), "keeps_groups")
    }
    #[getter]
    fn keeps_paint(slf: &Bound<'_, Self>) -> PyResult<bool> {
        keeps(slf.as_any(), "keeps_paint")
    }
    fn r#do(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<()> {
        let mut done: Vec<Obj<'_>> = Vec::new();
        for part in slf.getattr("commands")?.try_iter()? {
            let part = part?;
            if let Err(e) = part.call_method1("do", (scene,)) {
                let rejected = slf
                    .py()
                    .import("core.history")?
                    .getattr("CommandRejected")?;
                if e.value(slf.py()).is_instance(&rejected)?
                    && !slf.getattr("_journal_geometry")?.is_truthy()?
                {
                    for cmd in done.iter().rev() {
                        cmd.call_method1("undo", (scene,))?;
                    }
                }
                return Err(e);
            }
            done.push(part);
        }
        Ok(())
    }
    fn undo(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<()> {
        for cmd in slf
            .getattr("commands")?
            .call_method0("__reversed__")?
            .try_iter()?
        {
            cmd?.call_method1("undo", (scene,))?;
        }
        Ok(())
    }
    fn doc_undo(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<bool> {
        super::history::DocumentHistory::transaction_covers(&slf.getattr("commands")?, scene)
    }
    fn doc_after(slf: &Bound<'_, Self>, scene: &Obj<'_>, phase: &str) -> PyResult<()> {
        let mut parts = slf.getattr("commands")?;
        if phase == "undo" {
            parts = parts.call_method0("__reversed__")?;
        }
        for cmd in parts.try_iter()? {
            cmd?.call_method1("doc_after", (scene, phase))?;
        }
        Ok(())
    }
}
fn keeps(step: &Obj<'_>, flag: &str) -> PyResult<bool> {
    for cmd in step.getattr("commands")?.try_iter()? {
        let cmd = cmd?;
        if cmd
            .getattr_opt("keeps_paint")?
            .map(|v| v.is_truthy())
            .transpose()?
            .unwrap_or(false)
        {
            continue;
        }
        if flag == "keeps_paint"
            || !cmd
                .getattr_opt(flag)?
                .map(|v| v.is_truthy())
                .transpose()?
                .unwrap_or(false)
        {
            return Ok(false);
        }
    }
    Ok(true)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<import::ImportStep>()?;
    m.add_class::<MutationStep>()?;
    m.add_class::<DrawingSnapshotStep>()?;
    m.add_class::<StitchSnapshotStep>()?;
    m.add_class::<CompoundStep>()
}
