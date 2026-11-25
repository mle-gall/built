// SPDX-License-Identifier: GPL-3.0-or-later
//! Rebind nested sources and dimension anchors when a linked definition is released.
pub(crate) mod tags;
use crate::model::links;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
use std::collections::HashSet;
type Obj<'py> = Bound<'py, PyAny>;

#[pyfunction(name = "link_fill")]
#[pyo3(signature=(scene,group,mirror=None,host_path=None))]
pub(crate) fn fill<'py>(
    scene: &Obj<'py>,
    group: &Obj<'py>,
    mirror: Option<&Obj<'py>>,
    host_path: Option<&Obj<'py>>,
) -> PyResult<Option<String>> {
    let Some(record) = links::link_of(group)? else {
        return Ok(Some("not a link".into()));
    };
    let py = scene.py();
    let io = py.import("core.links")?;
    let found = io.call_method1("locate", (&record, host_path))?;
    if found.is_none() {
        return Ok(Some(format!("{} was not found", record.get_item("path")?)));
    }
    let path = found.call_method0("resolve")?;
    let norm = links::norm(&path)?;
    if norm != links::norm(&record.get_item("path")?)? {
        record.set_item("path", path.str()?)?;
    }
    let prior = links::registry(scene)?.call_method1("get", (norm,))?;
    let fresh = !prior.is_truthy()? || !prior.contains("tags")?;
    let materials = py
        .get_type::<PyDict>()
        .call1((scene.getattr("materials")?,))?;
    let source = match io.call_method1(
        "_read_definition",
        (
            &materials,
            &path,
            record.call_method1("get", ("select",))?,
            record.call_method1("get", ("include_nested", true))?,
        ),
    ) {
        Ok(s) => s,
        Err(e) if e.value(py).is_instance(&io.getattr("LinkError")?)? => {
            return Ok(Some(e.value(py).str()?.to_string()))
        }
        Err(e) => return Err(e),
    };
    scene
        .getattr("materials")?
        .call_method1("update", (materials,))?;
    links::populate(group, &source.get_item(0)?, false, None)?;
    io.call_method1(
        "_hold",
        (
            scene,
            &path,
            record.call_method1("get", ("select",))?,
            record.call_method1("get", ("include_nested", true))?,
            source.get_item(0)?,
            source.get_item(1)?,
        ),
    )?;
    links::sync_tags(scene, &path, &source.get_item(1)?)?;
    let entry = links::entry(scene, &path)?;
    entry.set_item(
        "loaded_mtime_ns",
        path.call_method0("stat")?.getattr("st_mtime_ns")?,
    )?;
    if let Some(mirror) = mirror {
        if fresh && mirror.is_truthy()? {
            let prefix = links::stem(mirror)?;
            for name in entry
                .call_method1("get", ("tags", PyTuple::empty(py)))?
                .try_iter()?
            {
                let name = name?;
                let layer = scene.call_method1("layer_by_key", (&name,))?;
                let old = scene.call_method1("layer_by_key", (format!("{prefix} / {name}"),))?;
                if !layer.is_none() && !old.is_none() {
                    let visible = old.getattr("visible")?.is_truthy()?;
                    if visible != layer.getattr("visible")?.is_truthy()? {
                        layer.setattr("visible", visible)?;
                        entry.get_item("overrides")?.set_item(name, visible)?;
                    }
                }
            }
        }
    }
    super::steps::version(scene)?;
    Ok(None)
}
pub(super) fn move_holds<'py>(
    scene: &Obj<'py>,
    placements: &Obj<'py>,
    moved: &Bound<'py, PyDict>,
) -> PyResult<()> {
    let Some(dims) = scene.getattr_opt("dimensions")? else {
        return Ok(());
    };
    if !dims.is_truthy()? || moved.is_empty() {
        return Ok(());
    }
    let py = scene.py();
    let roots = placements
        .try_iter()?
        .map(|p| Ok(p?.as_ptr() as usize))
        .collect::<PyResult<HashSet<_>>>()?;
    let ctor = py.import("core.dimension")?.getattr("VertexAnchor")?;
    let remap = |hold: Obj<'py>| -> PyResult<Obj<'py>> {
        if hold.is_none() {
            return Ok(hold);
        }
        let parts = hold
            .getattr("chain")?
            .try_iter()?
            .collect::<PyResult<Vec<_>>>()?;
        let mut relevant = false;
        let mut chain = Vec::with_capacity(parts.len());
        for p in parts {
            let id = p.as_ptr() as usize;
            let replacement = moved.get_item(id)?;
            relevant |= roots.contains(&id) || replacement.is_some();
            chain.push(replacement.unwrap_or(p));
        }
        if !relevant {
            return Ok(hold);
        }
        let old = hold.getattr("mesh")?;
        let mesh = moved
            .get_item(old.as_ptr() as usize)?
            .unwrap_or_else(|| old.clone());
        let vertex = hold.getattr("vertex")?;
        let vertex = if mesh.is(&old) {
            vertex
        } else {
            let lookup = || {
                mesh.call_method1(
                    "vertex_at",
                    (py.import("ingetrazo_core")?
                        .call_method1("Vec3", (vertex.getattr("position")?,))?,),
                )
            };
            match lookup() {
                Ok(v) if !v.is_none() => v,
                Ok(_) => return Ok(py.None().into_bound(py)),
                Err(e) if e.is_instance_of::<pyo3::exceptions::PyException>(py) => {
                    return Ok(py.None().into_bound(py))
                }
                Err(e) => return Err(e),
            }
        };
        ctor.call1((vertex, mesh, PyTuple::new(py, chain)?))
    };
    for dim in dims.try_iter()? {
        let dim = dim?;
        for (side, field) in [("a", "anchor_a"), ("b", "anchor_b")] {
            if let Some(h) = dim.getattr_opt(field)?.filter(|h| !h.is_none()) {
                dim.setattr(field, remap(h)?)?;
            }
            if let Some(holds) = dim.getattr_opt("_holds")? {
                if holds.is_truthy()? {
                    let values = holds.call_method1("get", (side,))?;
                    if values.is_truthy()? {
                        let out = PyList::empty(py);
                        for h in values.try_iter()? {
                            let v = remap(h?)?;
                            if !v.is_none() {
                                out.append(v)?;
                            }
                        }
                        holds.set_item(side, out)?;
                    }
                }
            }
        }
    }
    Ok(())
}
