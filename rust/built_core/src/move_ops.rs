// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared move and copy commands for the tool and document protocol.
use pyo3::{
    prelude::*,
    sync::PyOnceLock,
    types::{PyDict, PyList, PyTuple},
};
use std::collections::HashSet;
type Obj<'p> = Bound<'p, PyAny>;
#[pyfunction]
fn copied_guide<'p>(guide: &Obj<'p>, m: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = guide.py();
    let api = py.import("built_core.guide")?;
    let mapped = |attr: &str, method: &str| -> PyResult<Obj<'p>> {
        let p = guide.getattr(attr)?;
        if p.is_none() {
            Ok(p)
        } else {
            m.call_method1(method, (p,))
        }
    };
    if guide
        .getattr_opt("is_circle")?
        .is_some_and(|v| v.is_truthy().unwrap_or(false))
    {
        let kw = PyDict::new(py);
        kw.set_item("layer", guide.getattr_opt("layer")?)?;
        api.call_method(
            "GuideCircle",
            (
                mapped("center", "map")?,
                mapped("normal", "mapVector")?,
                guide.getattr("radius")?,
            ),
            Some(&kw),
        )
    } else {
        api.call_method1(
            "Guide",
            (
                mapped("point", "map")?,
                mapped("direction", "mapVector")?,
                mapped("origin", "map")?,
            ),
        )
    }
}
#[pyfunction]
fn dimension_modes<'p>(
    dims: &Obj<'p>,
    groups: &Obj<'p>,
    verts: &Obj<'p>,
) -> PyResult<Bound<'p, PyDict>> {
    let keys = verts
        .try_iter()?
        .map(|v| v?.getattr("key")?.extract::<u128>())
        .collect::<PyResult<HashSet<_>>>()?;
    let groups = groups
        .try_iter()?
        .map(|v| Ok(v?.as_ptr()))
        .collect::<PyResult<HashSet<_>>>()?;
    let out = PyDict::new(dims.py());
    for d in dims.try_iter()? {
        let d = d?;
        let mut anchored = 0;
        let mut riding = 0;
        for field in ["anchor_a", "anchor_b"] {
            let a = d.getattr(field)?;
            if a.is_none() {
                continue;
            }
            anchored += 1;
            let mut moves = keys.contains(&a.getattr("vertex")?.getattr("key")?.extract::<u128>()?);
            if !moves {
                for g in a.getattr("chain")?.try_iter()? {
                    if groups.contains(&g?.as_ptr()) {
                        moves = true;
                        break;
                    }
                }
            }
            riding += usize::from(moves);
        }
        if riding != 2 {
            out.set_item(d, if anchored == 2 { "line" } else { "rigid" })?;
        }
    }
    Ok(out)
}
fn list<'p>(v: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    PyList::new(v.py(), v.try_iter()?.collect::<PyResult<Vec<_>>>()?)
}
#[pyfunction]
#[pyo3(signature=(delta,*,groups=None,positions=None,labels=None,guides=None,dims=None,splanes=None,images=None))]
fn move_commands<'p>(
    delta: &Obj<'p>,
    groups: Option<&Obj<'p>>,
    positions: Option<&Obj<'p>>,
    labels: Option<&Obj<'p>>,
    guides: Option<&Obj<'p>>,
    dims: Option<&Obj<'p>>,
    splanes: Option<&Obj<'p>>,
    images: Option<&Obj<'p>>,
) -> PyResult<Bound<'p, PyList>> {
    let py = delta.py();
    let h = py.import("built_core.history")?;
    let out = PyList::empty(py);
    if let Some(gs) = groups {
        for g in gs.try_iter()? {
            out.append(h.call_method1("MoveGroupCommand", (g?, delta))?)?;
        }
    }
    for (cls, input) in [
        ("MoveVerticesCommand", positions),
        ("MoveTextLabelsCommand", labels),
        ("MoveGuidesCommand", guides),
        ("MoveDimensionsCommand", dims),
        ("MoveSectionPlanesCommand", splanes),
        ("MoveImagePlanesCommand", images),
    ] {
        let Some(v) = input else {
            continue;
        };
        if !v.is_truthy()? {
            continue;
        }
        let args = if cls == "MoveDimensionsCommand" {
            v.clone()
        } else {
            list(v)?.into_any()
        };
        let kw = PyDict::new(py);
        if cls == "MoveVerticesCommand" {
            kw.set_item("carry_textures", true)?;
        }
        out.append(h.call_method(cls, (args, delta), Some(&kw))?)?;
    }
    Ok(out)
}
#[pyfunction]
fn plan_init(cmd: &Obj, segments: &Obj) -> PyResult<()> {
    cmd.setattr("_segments", list(segments)?)?;
    cmd.setattr("_cmd", cmd.py().None())
}
#[pyfunction]
fn plan_do(cmd: &Obj, scene: &Obj) -> PyResult<()> {
    let mut step = cmd.getattr("_cmd")?;
    if step.is_none() {
        let kw = PyDict::new(cmd.py());
        kw.set_item("preserve_faces", true)?;
        kw.set_item("plane_rebuild", true)?;
        step = cmd.py().import("core.edits")?.call_method(
            "build_add_edges",
            (scene, cmd.getattr("_segments")?),
            Some(&kw),
        )?;
        cmd.setattr("_cmd", &step)?;
    }
    step.call_method1("do", (scene,))?;
    Ok(())
}
#[pyfunction]
fn plan_undo(cmd: &Obj, scene: &Obj) -> PyResult<()> {
    let step = cmd.getattr("_cmd")?;
    if !step.is_none() {
        step.call_method1("undo", (scene,))?;
    }
    Ok(())
}
#[pyfunction]
fn copy_init(
    cmd: &Obj,
    commands: &Obj,
    groups: &Obj,
    guides: &Obj,
    faces: &Obj,
    segments: &Obj,
) -> PyResult<()> {
    let py = cmd.py();
    py.import("built_core.history")?
        .getattr("CompoundCommand")?
        .call_method1("__init__", (cmd, commands))?;
    cmd.setattr(
        "_selection",
        py.import("built_core")?
            .call_method1("CopySelection", (groups, guides, faces, segments))?,
    )
}
fn apply_copy(cmd: &Obj, scene: &Obj, method: &str, base: &str) -> PyResult<()> {
    let callback = cmd
        .py()
        .import("built_core.history")?
        .getattr("CompoundCommand")?
        .getattr(base)?
        .call_method1("__get__", (cmd,))?;
    cmd.getattr("_selection")?
        .call_method1(method, (scene, callback))?;
    Ok(())
}
#[pyfunction]
fn copy_do(cmd: &Obj, scene: &Obj) -> PyResult<()> {
    apply_copy(cmd, scene, "execute", "do")
}
#[pyfunction]
fn copy_undo(cmd: &Obj, scene: &Obj) -> PyResult<()> {
    apply_copy(cmd, scene, "undo", "undo")
}
fn class(py: Python<'_>, copy: bool) -> PyResult<&Bound<'_, PyAny>> {
    static PLAN: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
    static COPY: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
    let cache = if copy { &COPY } else { &PLAN };
    Ok(cache
        .get_or_try_init(py, || -> PyResult<_> {
            let kw = PyDict::new(py);
            kw.set_item("__module__", "move_ops")?;
            let (name, base, init, execute, undo) = if copy {
                kw.set_item("doc_undo", false)?;
                (
                    "MoveCopiesCommand",
                    "CompoundCommand",
                    wrap_pyfunction!(copy_init, py)?,
                    wrap_pyfunction!(copy_do, py)?,
                    wrap_pyfunction!(copy_undo, py)?,
                )
            } else {
                (
                    "PlannedEdges",
                    "Command",
                    wrap_pyfunction!(plan_init, py)?,
                    wrap_pyfunction!(plan_do, py)?,
                    wrap_pyfunction!(plan_undo, py)?,
                )
            };
            let bind = py.import("functools")?.getattr("partialmethod")?;
            for (field, f) in [("__init__", init), ("do", execute), ("undo", undo)] {
                kw.set_item(field, bind.call1((f,))?)?;
            }
            let base = py.import("built_core.history")?.getattr(base)?;
            Ok(py
                .import("builtins")?
                .call_method1("type", (name, (base,), kw))?
                .unbind())
        })?
        .bind(py))
}
pub(crate) fn make<'p>(py: Python<'p>, copy: bool, args: &Bound<'p, PyTuple>) -> PyResult<Obj<'p>> {
    class(py, copy)?.call1(args)
}
pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    let name = "built_core.move_ops";
    let sub = PyModule::new(m.py(), name)?;
    for f in [
        wrap_pyfunction!(copied_guide, &sub)?,
        wrap_pyfunction!(dimension_modes, &sub)?,
        wrap_pyfunction!(move_commands, &sub)?,
    ] {
        sub.add_function(f)?;
    }
    m.py()
        .import("sys")?
        .getattr("modules")?
        .set_item(name, &sub)?;
    m.add("move_ops", sub)
}
