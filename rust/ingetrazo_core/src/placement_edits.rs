// SPDX-License-Identifier: GPL-3.0-or-later
//! Placement membership edits preserve owner list order and world transforms.
use crate::math::Matrix4;
use pyo3::{prelude::*, types::PyList};
type Obj<'py> = Bound<'py, PyAny>;
type Moved<'py> = (Obj<'py>, Obj<'py>, usize, Obj<'py>, Obj<'py>);
type TargetState<'py> = (Obj<'py>, Obj<'py>);
type Positioned<'py> = (usize, Obj<'py>);

fn siblings<'py>(roots: &Obj<'py>, parent: &Obj<'py>) -> PyResult<Obj<'py>> {
    if parent.is_none() {
        Ok(roots.clone())
    } else {
        parent.getattr("children")
    }
}
fn ensure_container(group: &Obj<'_>) -> PyResult<()> {
    if group.getattr("xform")?.is_none() {
        group.setattr("xform", group.py().get_type::<Matrix4>().call0()?)?;
        group.setattr("component", false)?;
    }
    Ok(())
}

#[pyfunction]
fn insert_placement<'py>(
    roots: &Obj<'py>,
    context: &Obj<'py>,
    active_mesh: &Obj<'py>,
    group: &Obj<'py>,
) -> PyResult<(Obj<'py>, bool)> {
    let py = roots.py();
    let owner = siblings(roots, context)?;
    let mut baked = false;
    if !context.is_none() {
        let inst = active_mesh.hasattr("_instance_definition")?
            && !active_mesh.getattr("_instance_definition")?.is_none();
        let frame = if inst {
            active_mesh
                .getattr("_store")?
                .call_method0("tool_placement")?
        } else {
            context.getattr("xform")?
        };
        if !frame.is_none() {
            let inverted = frame.call_method0("inverted")?;
            if inverted.get_item(1)?.is_truthy()? {
                let inverse = inverted.get_item(0)?;
                let xform = group.getattr("xform")?;
                if !xform.is_none() {
                    group.setattr("xform", inverse.mul(xform)?)?;
                } else if inst {
                    group.setattr("component", false)?;
                    group.setattr("xform", inverse)?;
                } else {
                    let facade = py.import("core.group")?;
                    group.setattr(
                        "mesh",
                        facade
                            .call_method1("transformed_mesh", (group.getattr("mesh")?, &inverse))?,
                    )?;
                    facade.call_method1("carry_axes", (group, inverse))?;
                    baked = true;
                }
            }
        }
    }
    owner.call_method1("append", (group,))?;
    Ok((owner, baked))
}

#[pyfunction]
fn convert_component<'py>(
    group: &Obj<'py>,
    name: &Obj<'py>,
) -> PyResult<(Obj<'py>, Obj<'py>, Obj<'py>)> {
    let old = (
        group.getattr("xform")?,
        group.getattr("component")?,
        group.getattr("name")?,
    );
    if old.0.is_none() {
        group.setattr("xform", group.py().get_type::<Matrix4>().call0()?)?;
    }
    group.setattr("component", true)?;
    if name.is_truthy()? {
        group.setattr("name", name)?;
    }
    Ok(old)
}
#[pyfunction]
fn restore_component(group: &Obj<'_>, old: (Obj<'_>, Obj<'_>, Obj<'_>)) -> PyResult<()> {
    group.setattr("xform", old.0)?;
    group.setattr("component", old.1)?;
    group.setattr("name", old.2)
}

#[pyfunction]
fn nest<'py>(
    roots: &Obj<'py>,
    group: &Obj<'py>,
    children: Vec<Obj<'py>>,
    discard: &Obj<'py>,
) -> PyResult<(Obj<'py>, Vec<Positioned<'py>>)> {
    if !roots.contains(group)? {
        roots.call_method1("append", (group,))?;
    }
    let xform = group.getattr("xform")?;
    let mut positions = Vec::new();
    for child in children {
        positions.push((roots.call_method1("index", (&child,))?.extract()?, child));
    }
    positions.sort_by_key(|r| r.0);
    let children = PyList::empty(group.py());
    for (_, child) in &positions {
        roots.call_method1("remove", (child,))?;
        discard.call1((child,))?;
        children.append(child)?;
    }
    group.call_method1("adopt", (children,))?;
    group.setattr("component", false)?;
    Ok((xform, positions))
}
#[pyfunction]
fn unnest(
    roots: &Obj<'_>,
    group: &Obj<'_>,
    xform: &Obj<'_>,
    positions: Vec<Positioned<'_>>,
) -> PyResult<()> {
    group.setattr("children", PyList::empty(roots.py()))?;
    group.setattr("xform", xform)?;
    for (i, child) in positions {
        roots.call_method1("insert", (i.min(roots.len()?), child))?;
    }
    Ok(())
}

#[pyfunction]
fn reparent<'py>(
    roots: &Obj<'py>,
    groups: &Obj<'py>,
    target: &Obj<'py>,
    discard: &Obj<'py>,
) -> PyResult<(Option<TargetState<'py>>, Vec<Moved<'py>>)> {
    let py = roots.py();
    let outliner = py.import("core.outliner")?;
    let up = outliner.call_method1("parents", (roots,))?;
    let moving = outliner.call_method1("topmost", (roots, groups))?;
    let state = if target.is_none() {
        None
    } else {
        let old = (target.getattr("xform")?, target.getattr("component")?);
        ensure_container(target)?;
        Some(old)
    };
    let new = outliner.call_method1("world_matrix", (roots, target))?;
    let inverse = if new.is_none() {
        new
    } else {
        new.call_method0("inverted")?.get_item(0)?
    };
    let mut moves = Vec::new();
    let dest = siblings(roots, target)?;
    for g in moving.try_iter()? {
        let g = g?;
        let old = up.call_method1("get", (g.as_ptr() as usize,))?;
        let source = siblings(roots, &old)?;
        let index = source.call_method1("index", (&g,))?.extract()?;
        let from = outliner.call_method1("world_matrix", (roots, &old))?;
        let xform = g.getattr("xform")?;
        let mesh = g.getattr("mesh")?;
        let delta = outliner.call_method1("compose", (&inverse, from))?;
        if !delta.is_none() && !delta.call_method0("isIdentity")?.is_truthy()? {
            if xform.is_none() {
                g.setattr(
                    "mesh",
                    py.import("core.group")?
                        .call_method1("transformed_mesh", (&mesh, delta))?,
                )?;
            } else {
                g.setattr("xform", delta.mul(&xform)?)?;
            }
        }
        source.call_method1("remove", (&g,))?;
        dest.call_method1("append", (&g,))?;
        discard.call1((&g,))?;
        moves.push((g, old, index, xform, mesh));
    }
    Ok((state, moves))
}
#[pyfunction]
fn restore_parent(
    roots: &Obj<'_>,
    target: &Obj<'_>,
    state: Option<TargetState<'_>>,
    moves: Vec<Moved<'_>>,
) -> PyResult<()> {
    let dest = siblings(roots, target)?;
    for (g, old, index, xform, mesh) in moves.into_iter().rev() {
        dest.call_method1("remove", (&g,))?;
        g.setattr("xform", xform)?;
        g.setattr("mesh", mesh)?;
        siblings(roots, &old)?.call_method1("insert", (index, g))?;
    }
    if let Some((xform, component)) = state {
        target.setattr("xform", xform)?;
        target.setattr("component", component)?;
    }
    Ok(())
}
#[pyfunction]
fn split_placement<'py>(
    g: &Obj<'py>,
    pieces: &Obj<'py>,
    emptied: &Obj<'py>,
) -> PyResult<((Obj<'py>, Obj<'py>, Obj<'py>, Obj<'py>), Obj<'py>)> {
    let py = g.py();
    let before = (
        g.getattr("mesh")?,
        py.get_type::<PyList>().call1((g.getattr("children")?,))?,
        g.getattr("xform")?,
        g.getattr("exploded")?,
    );
    let mesh = if emptied.is_none() {
        py.import("core.mesh")?.getattr("Mesh")?.call0()?
    } else {
        emptied.clone()
    };
    g.setattr("mesh", &mesh)?;
    g.call_method1("adopt", (pieces,))?;
    g.setattr("exploded", py.None())?;
    Ok((before, mesh))
}
#[pyfunction]
fn restore_split(g: &Obj<'_>, before: (Obj<'_>, Obj<'_>, Obj<'_>, Obj<'_>)) -> PyResult<()> {
    g.setattr("mesh", before.0)?;
    g.setattr("children", before.1)?;
    g.setattr("xform", before.2)?;
    g.setattr("exploded", before.3)
}
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "placement_edits")?;
    sub.add_function(wrap_pyfunction!(insert_placement, &sub)?)?;
    sub.add_function(wrap_pyfunction!(convert_component, &sub)?)?;
    sub.add_function(wrap_pyfunction!(restore_component, &sub)?)?;
    sub.add_function(wrap_pyfunction!(nest, &sub)?)?;
    sub.add_function(wrap_pyfunction!(unnest, &sub)?)?;
    sub.add_function(wrap_pyfunction!(reparent, &sub)?)?;
    sub.add_function(wrap_pyfunction!(restore_parent, &sub)?)?;
    sub.add_function(wrap_pyfunction!(split_placement, &sub)?)?;
    sub.add_function(wrap_pyfunction!(restore_split, &sub)?)?;
    m.add_submodule(&sub)?;
    Ok(())
}
