// SPDX-License-Identifier: GPL-3.0-or-later
//! Native material-registry steps (docs/commands-port.md): restamp a
//! material everywhere it is worn, edit a textured one in place, delete one.
//!
//! The faces, groups and components wearing the material are rewritten by
//! `MaterialRestamp` (Rust). When History journals every mesh the
//! transaction replays the edit; otherwise the restamp keeps what it
//! replaced and restores it, its real inverse.
use super::steps::{running, version, After, Meshes, Obj, RecordStep};
use pyo3::exceptions::PyKeyError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};
use pyo3::PyClassInitializer;

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn optional<'py>(o: &Obj<'py>, name: &str) -> PyResult<Option<Obj<'py>>> {
    Ok(o.getattr_opt(name)?.filter(|v| !v.is_none()))
}

/// `o` as a tuple, or None.
fn tuple_of<'py>(py: Python<'py>, o: Option<Obj<'py>>) -> PyResult<Obj<'py>> {
    match o.filter(|v| !v.is_none()) {
        Some(v) => Ok(PyTuple::new(py, v.try_iter()?.collect::<PyResult<Vec<_>>>()?)?.into_any()),
        None => Ok(none(py)),
    }
}

fn dirty_chunks(py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    py.import("core.history_commands")?.getattr("_dirty_group_chunks")?.call1((scene,))?;
    Ok(())
}

fn every_mesh<'py>(_s: &RecordStep, py: Python<'py>, scene: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    py.import("core.purge")?.getattr("iter_meshes")?.call1((scene,))?.try_iter()?.collect()
}

/// The meshes the edit touched changed under their chunks, and so did
/// the groups drawing them.
fn touch(py: Python<'_>, me: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    for mesh in me.getattr("_meshes")?.try_iter()? {
        let mesh = mesh?;
        let serial: i64 = mesh.getattr("_mut_serial")?.extract()?;
        mesh.setattr("_mut_serial", serial + 1)?;
    }
    dirty_chunks(py, scene)?;
    version(scene)
}

fn restamp(s: &RecordStep, py: Python<'_>, me: &Obj<'_>, scene: &Obj<'_>, kind: &str) -> PyResult<()> {
    let materials = scene.getattr("materials")?;
    let new = me.getattr("_new")?;
    if optional(me, "_edit")?.is_none() {
        let groups = py.import("core.purge")?.getattr("iter_groups")?.call1((scene.getattr("groups")?,))?;
        let edit = py.import("ingetrazo_core")?.getattr("MaterialRestamp")?.call1((
            scene.getattr("loose_mesh")?,
            groups,
            &materials,
            s.arg(py, 0),
            new.getattr("name")?,
            kind,
            !s.journalled(),
        ))?;
        me.setattr("_meshes", edit.getattr("meshes")?)?;
        me.setattr("_edit", edit)?;
    }
    let edited = if kind == "texture" { me.getattr("edited")? } else { none(py) };
    me.getattr("_edit")?.call_method1("apply", (&materials, new, edited))?;
    touch(py, me, scene)?;
    if s.journalled() {
        me.setattr("_edit", none(py))?;
    }
    Ok(())
}

fn run_restamp(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    restamp(s, py, &me, scene, "stamp")
}

fn run_delete(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    let name = s.arg(py, 0);
    if optional(&me, "_edit")?.is_none() && !scene.getattr("materials")?.contains(&name)? {
        return Err(PyKeyError::new_err(format!("no material called {}", name.repr()?)));
    }
    restamp(s, py, &me, scene, "delete")
}

fn run_edit_textured(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    let name = s.arg(py, 0);
    let old = scene.getattr("materials")?.call_method1("get", (&name,))?;
    if old.is_none() || !old.getattr("texture")?.is_truthy()? {
        return Ok(());
    }
    if optional(&me, "_edit")?.is_none() {
        let kw = PyDict::new(py);
        kw.set_item("color", old.getattr("color")?)?;
        kw.set_item("texture", me.call_method1("edited", (old.getattr("texture")?,))?)?;
        kw.set_item("opacity", old.getattr("opacity")?)?;
        kw.set_item("finish", old.getattr("finish")?)?;
        kw.set_item("pbr", old.getattr("pbr")?)?;
        let new = py.import("core.materials")?.getattr("Material")?.call((&name,), Some(&kw))?;
        me.setattr("_new", new)?;
    }
    restamp(s, py, &me, scene, "texture")
}

/// A direct undo puts the registry entry, each face's and each
/// container's paint back; a transaction replay only marks what changed.
fn after_restamp(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    if phase == Some("undo") && s.is_direct() {
        let Some(edit) = optional(&me, "_edit")? else { return Ok(()) };
        edit.call_method1("restore", (scene.getattr("materials")?,))?;
    }
    touch(py, &me, scene)
}

macro_rules! steps {
    ($($ty:ident = $name:literal;)*) => {
        $(
            #[pyclass(module = "ingetrazo_core", name = $name, extends = RecordStep, frozen)]
            pub struct $ty;
        )*
        pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
            $(m.add_class::<$ty>()?;)*
            Ok(())
        }
    };
}

steps! {
    RestampMaterial = "RestampMaterialCommand";
    EditTexturedMaterial = "EditTexturedMaterialCommand";
    DeleteMaterial = "DeleteMaterialCommand";
}

fn restamp_step(name: &'static str, run: fn(&RecordStep, Python<'_>, &Obj<'_>) -> PyResult<()>, args: Vec<(&'static str, Obj<'_>)>) -> RecordStep {
    RecordStep::new(name, run, args).own_inverse().hook(After(after_restamp)).on_meshes(Meshes(every_mesh))
}

#[pymethods]
impl RestampMaterial {
    // What a restamp answers before it ran.
    #[classattr]
    fn _edit(py: Python<'_>) -> Py<PyAny> {
        py.None()
    }
    #[classattr]
    fn _meshes(py: Python<'_>) -> Py<PyAny> {
        PyTuple::empty(py).into_any().unbind()
    }

    /// Replace registry material `name` by `new_material` and restamp
    /// every face, group and component wearing it (colour, texture,
    /// opacity; keys the new recipe lacks are dropped). Another name
    /// renames as it restamps.
    #[new]
    fn new(name: Obj<'_>, new_material: Obj<'_>) -> PyClassInitializer<Self> {
        let step = restamp_step("RestampMaterialCommand", run_restamp, vec![("_name", name), ("_new", new_material)]);
        PyClassInitializer::from(step).add_subclass(Self)
    }
}

#[pymethods]
impl DeleteMaterial {
    // What a restamp answers before it ran.
    #[classattr]
    fn _edit(py: Python<'_>) -> Py<PyAny> {
        py.None()
    }
    #[classattr]
    fn _meshes(py: Python<'_>) -> Py<PyAny> {
        PyTuple::empty(py).into_any().unbind()
    }

    /// Remove registry material `name`: every face side and container
    /// painted with it shows the default material again.
    #[new]
    fn new(py: Python<'_>, name: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let blank = py.import("core.materials")?.getattr("Material")?.call1((&name,))?;
        let step = restamp_step("DeleteMaterialCommand", run_delete, vec![("_name", name), ("_new", blank)]);
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}

#[pymethods]
impl EditTexturedMaterial {
    // What a restamp answers before it ran.
    #[classattr]
    fn _edit(py: Python<'_>) -> Py<PyAny> {
        py.None()
    }
    #[classattr]
    fn _meshes(py: Python<'_>) -> Py<PyAny> {
        PyTuple::empty(py).into_any().unbind()
    }

    #[classattr]
    fn _new(py: Python<'_>) -> Py<PyAny> {
        py.None()
    }

    /// Edit a textured registry material's colour (and tile size) and
    /// carry it to every face wearing it, each face's own texture placement
    /// kept. `tint` is an RGB (0-1) or None; `size` is `(sw, sh, rot)` or
    /// None to leave each face's own.
    #[new]
    #[pyo3(signature = (name, tint, tint_mode = 0, size = None))]
    fn new(py: Python<'_>, name: Obj<'_>, tint: Option<Obj<'_>>, tint_mode: Option<i64>, size: Option<Obj<'_>>) -> PyResult<PyClassInitializer<Self>> {
        let args = vec![
            ("_name", name),
            ("_tint", tuple_of(py, tint)?),
            ("_tint_mode", tint_mode.unwrap_or(0).into_pyobject(py)?.into_any()),
            ("_size", tuple_of(py, size)?),
        ];
        Ok(PyClassInitializer::from(restamp_step("EditTexturedMaterialCommand", run_edit_textured, args)).add_subclass(Self))
    }

    /// `tex` with this edit: the colour (from its untinted image) and,
    /// when given, the size and turn.
    fn edited<'py>(slf: PyRef<'py, Self>, tex: Obj<'py>) -> PyResult<Obj<'py>> {
        let py = slf.py();
        if !tex.is_truthy()? {
            return Ok(tex);
        }
        let s: &RecordStep = &*slf.as_super();
        let texture = py.import("core.texture")?;
        let out = py.get_type::<PyDict>().call1((tex,))?;
        let size = s.arg(py, 3);
        if !size.is_none() {
            for (i, k) in ["sw", "sh", "rot"].iter().enumerate() {
                out.set_item(k, size.get_item(i)?.extract::<f64>()?)?;
            }
        }
        let tint = s.arg(py, 1);
        if tint.is_none() {
            return texture.getattr("untinted_texture")?.call1((out,));
        }
        texture.getattr("tinted_texture")?.call1((out, tint, s.arg(py, 2)))
    }
}

