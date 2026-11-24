// SPDX-License-Identifier: GPL-3.0-or-later
//! Definition replacement, kept templates and bulk group-to-group merging.
use crate::math::{xyz, Matrix4};
use numpy::{ndarray::Array2, IntoPyArray, PyArray1};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySlice},
};
type Obj<'py> = Bound<'py, PyAny>;

#[pyfunction]
pub(crate) fn replacement_targets<'py>(groups: &Obj<'py>, source: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let mesh = source.getattr("mesh")?;
    let mut out = Vec::new();
    for g in groups.try_iter()? {
        let g = g?;
        if !g.is(source)
            && g.call_method0("is_component")?.is_truthy()?
            && !g.getattr("mesh")?.is(&mesh)
        {
            out.push(g);
        }
    }
    Ok(out)
}
#[pyfunction]
pub(crate) fn replace_definitions<'py>(
    targets: Vec<Obj<'py>>,
    source: &Obj<'py>,
    fields: Vec<String>,
) -> PyResult<Vec<Bound<'py, PyDict>>> {
    let py = source.py();
    let copy = py.import("core.group")?.getattr("copy_group")?;
    let keywords = PyDict::new(py);
    keywords.set_item("_in_definition", true)?;
    let deepcopy = py.import("copy")?.getattr("deepcopy")?;
    let mut before = Vec::new();
    for g in &targets {
        let values = PyDict::new(py);
        for k in &fields {
            values.set_item(k, g.getattr(k.as_str())?)?;
        }
        before.push(values);
    }
    for g in targets {
        g.setattr("definition", source.getattr("definition")?)?;
        let ext = source.getattr("ext")?;
        if ext.is_truthy()? && ext.call_method1("get", ("link",))?.is_truthy()? {
            g.setattr(
                "children",
                py.import("core.links")?
                    .call_method1("copy_parts", (source, &g))?,
            )?;
        } else {
            let children = PyList::empty(py);
            let original = source.getattr("children")?;
            if original.is_truthy()? {
                for c in original.try_iter()? {
                    children.append(copy.call((c?,), Some(&keywords))?)?;
                }
            }
            g.setattr("children", children)?;
        }
        g.setattr("name", source.getattr("name")?)?;
        let axes = source.getattr("axes")?;
        g.setattr(
            "axes",
            if axes.is_none() {
                axes
            } else {
                py.get_type::<Matrix4>().call1((axes,))?
            },
        )?;
        g.setattr("component", true)?;
        let dict_copy = |v: Obj<'py>| -> PyResult<Obj<'py>> {
            if v.is_truthy()? {
                py.get_type::<PyDict>().call1((v,))
            } else {
                Ok(py.None().into_bound(py))
            }
        };
        g.setattr("text3d", dict_copy(source.getattr("text3d")?)?)?;
        let record = source.getattr("ifc")?;
        let value = if record.is_truthy()? {
            let fields = PyDict::new(py);
            for kv in record.call_method0("items")?.try_iter()? {
                let kv = kv?;
                let k = kv.get_item(0)?;
                if !k.eq("guid")? {
                    fields.set_item(k, kv.get_item(1)?)?;
                }
            }
            deepcopy.call1((fields,))?
        } else {
            py.None().into_bound(py)
        };
        g.setattr("ifc", value)?;
        g.setattr(
            "ext",
            if ext.is_truthy()? {
                deepcopy.call1((ext,))?
            } else {
                py.None().into_bound(py)
            },
        )?;
        g.setattr("billboard", source.getattr("billboard")?)?;
        g.setattr("exploded", dict_copy(source.getattr("exploded")?)?)?;
    }
    Ok(before)
}
#[pyfunction]
pub(crate) fn restore_definitions(groups: Vec<Obj<'_>>, before: Vec<Bound<'_, PyDict>>) -> PyResult<()> {
    for (g, values) in groups.into_iter().zip(before) {
        for (k, v) in values {
            g.setattr(k.extract::<String>()?.as_str(), v)?;
        }
    }
    Ok(())
}
#[pyfunction]
pub(crate) fn keep_template<'py>(
    scene: &Obj<'py>,
    instance: &Obj<'py>,
    keep: bool,
    previous: &Obj<'py>,
) -> PyResult<(Obj<'py>, Option<usize>)> {
    let py = scene.py();
    let api = py.import("core.kept_components")?;
    let current = api.call_method1("kept_template", (scene, instance.getattr("mesh")?))?;
    let kept = scene.getattr("kept_components")?;
    if keep && current.is_none() {
        let template = if previous.is_truthy()? {
            previous.clone()
        } else {
            api.call_method1("make_template", (instance,))?
        };
        kept.call_method1("append", (&template,))?;
        Ok((template, None))
    } else if !keep && !current.is_none() {
        let i = kept.call_method1("index", (&current,))?.extract()?;
        kept.call_method1("remove", (&current,))?;
        Ok((current, Some(i)))
    } else {
        Ok((py.None().into_bound(py), None))
    }
}
#[pyfunction]
pub(crate) fn restore_template(
    kept: &Obj<'_>,
    template: &Obj<'_>,
    keep: bool,
    index: &Obj<'_>,
) -> PyResult<()> {
    if !template.is_none() {
        if keep {
            kept.call_method1("remove", (template,))?;
        } else {
            kept.call_method1("insert", (index, template))?;
        }
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn merge_group_geometry<'py>(groups: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = groups.py();
    let api = py.import("core.group")?;
    let mut edge_coords = Vec::new();
    let mut face_coords = Vec::new();
    let mut ring_sizes = Vec::new();
    let mut ring_counts = Vec::new();
    let flags = PyList::empty(py);
    let attrs = PyList::empty(py);
    for group in groups.try_iter()? {
        let src = api.call_method1("world_mesh", (group?,))?;
        for face in src.getattr("faces")?.try_iter()? {
            let face = face?;
            let mut rings = vec![face.getattr("vertices")?];
            for hole in face.getattr("holes")?.try_iter()? {
                rings.push(hole?);
            }
            ring_counts.push(rings.len() as i64);
            for ring in rings {
                ring_sizes.push(ring.len()? as i64);
                for p in ring.try_iter()? {
                    face_coords.extend(xyz(&p?)?.to_array());
                }
            }
            let paint = face.getattr("attrs")?;
            attrs.append(if paint.is_truthy()? {
                py.get_type::<PyDict>().call1((paint,))?
            } else {
                py.None().into_bound(py)
            })?;
        }
        for e in src.getattr("edges")?.try_iter()? {
            let e = e?;
            for end in ["a", "b"] {
                edge_coords.extend(xyz(&e.getattr(end)?)?.to_array());
            }
            flags.append((
                e.getattr("soft")?.is_truthy()?,
                e.getattr("curve")?,
                e.getattr("layer")?,
                e.getattr("hidden")?.is_truthy()?,
            ))?;
        }
    }
    let mesh = py.import("core.mesh")?.getattr("Mesh")?.call0()?;
    let edge_end = (edge_coords.len() / 3) as isize;
    edge_coords.append(&mut face_coords);
    if !edge_coords.is_empty() {
        let all = Array2::from_shape_vec((edge_coords.len() / 3, 3), edge_coords)
            .expect("collected triples")
            .into_pyarray(py);
        let welded = mesh.call_method1("bulk_weld", (all,))?;
        let vs = welded.get_item(0)?;
        let ids = welded.get_item(1)?;
        let mut edge_map = py.None().into_bound(py);
        if edge_end > 0 {
            let a = ids.get_item(PySlice::new(py, 0, edge_end, 2))?;
            let b = ids.get_item(PySlice::new(py, 1, edge_end, 2))?;
            edge_map = mesh.call_method1("add_edges_welded", (&vs, a, b, flags))?;
        }
        if !ring_counts.is_empty() {
            let kwargs = PyDict::new(py);
            kwargs.set_item("edge_map", edge_map)?;
            let face_ids = ids.get_item(PySlice::new(py, edge_end, ids.len()? as isize, 1))?;
            mesh.call_method(
                "add_faces_welded",
                (
                    vs,
                    face_ids,
                    PyArray1::from_vec(py, ring_sizes),
                    PyArray1::from_vec(py, ring_counts),
                    attrs,
                ),
                Some(&kwargs),
            )?;
        }
    }
    let first = groups.get_item(0)?;
    let merged = api
        .getattr("Group")?
        .call1((mesh, first.getattr("name")?))?;
    merged.setattr("layer", first.getattr("layer")?)?;
    Ok(merged)
}

#[pyfunction]
pub(crate) fn replace_roots<'py>(
    roots: &Obj<'py>,
    groups: Vec<Obj<'py>>,
    merged: &Obj<'py>,
    discard: &Obj<'py>,
) -> PyResult<Vec<usize>> {
    let mut indices = Vec::new();
    for g in &groups {
        indices.push(roots.call_method1("index", (g,))?.extract()?);
    }
    for g in groups {
        roots.call_method1("remove", (&g,))?;
        discard.call1((g,))?;
    }
    roots.call_method1("append", (merged,))?;
    Ok(indices)
}
#[pyfunction]
pub(crate) fn group_owner<'py>(scene: &Obj<'py>, g: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let roots = scene.getattr("groups")?;
    if roots.contains(g)? {
        return Ok(Some(roots));
    }
    let walk = scene
        .py()
        .import("core.group")?
        .getattr("iter_placements")?;
    for root in roots.try_iter()? {
        for row in walk.call1((root?,))?.try_iter()? {
            let parent = row?.get_item(0)?;
            let kids = parent.getattr("children")?;
            if kids.is_truthy()? && kids.contains(g)? {
                return Ok(Some(kids));
            }
        }
    }
    Ok(None)
}
#[pyfunction]
pub(crate) fn delete_placement<'py>(scene: &Obj<'py>, g: &Obj<'py>) -> PyResult<(Obj<'py>, usize)> {
    let Some(owner) = group_owner(scene, g)? else {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "group is not in the scene",
        ));
    };
    let index = owner.call_method1("index", (g,))?.extract()?;
    owner.call_method1("remove", (g,))?;
    Ok((owner, index))
}
#[pyfunction]
pub(crate) fn restore_placement(
    roots: &Obj<'_>,
    owner: &Obj<'_>,
    index: Option<usize>,
    g: &Obj<'_>,
) -> PyResult<()> {
    let dest = if owner.is_none() { roots } else { owner };
    dest.call_method1("insert", (index.unwrap_or(0).min(dest.len()?), g))?;
    Ok(())
}
#[pyfunction]
pub(crate) fn set_placement_field<'py>(g: &Obj<'py>, field: &str, value: &Obj<'py>) -> PyResult<Obj<'py>> {
    let old = if field == "material" && !g.hasattr(field)? {
        g.py().None().into_bound(g.py())
    } else {
        g.getattr(field)?
    };
    g.setattr(field, value)?;
    Ok(old)
}
#[pyfunction]
pub(crate) fn append_members(list: &Obj<'_>, items: &Obj<'_>) -> PyResult<()> {
    list.call_method1("extend", (items,))?;
    Ok(())
}
#[pyfunction]
pub(crate) fn remove_members(list: &Obj<'_>, items: &Obj<'_>) -> PyResult<()> {
    for item in items.try_iter()? {
        let item = item?;
        if list.contains(&item)? {
            list.call_method1("remove", (item,))?;
        }
    }
    Ok(())
}
#[pyfunction]
pub(crate) fn rebuild_letter_placement<'py>(
    g: &Obj<'py>,
    params: &Obj<'py>,
    before: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = g.py();
    let api = py.import("core.text3d")?;
    let before = if before.is_none() {
        (
            g.getattr("children")?,
            g.getattr("text3d")?,
            g.getattr("name")?,
        )
            .into_pyobject(py)?
            .into_any()
    } else {
        before.clone()
    };
    g.setattr(
        "children",
        api.call_method1("rebuild_text_group", (g, params))?,
    )?;
    g.setattr(
        "text3d",
        api.call_method1("text_state", (params, g.getattr("children")?))?,
    )?;
    let text: String = params.get_item("text")?.call_method0("strip")?.extract()?;
    let name: String = text.chars().take(24).collect();
    if name.is_empty() {
        g.setattr("name", g.getattr("name")?)?;
    } else {
        g.setattr("name", name)?;
    }
    Ok(before)
}
#[pyfunction]
pub(crate) fn restore_letter_placement(g: &Obj<'_>, before: (Obj<'_>, Obj<'_>, Obj<'_>)) -> PyResult<()> {
    g.setattr("children", before.0)?;
    g.setattr("text3d", before.1)?;
    g.setattr("name", before.2)
}
type UnlinkPlan<'py> = (
    Vec<Bound<'py, PyDict>>,
    Vec<Bound<'py, PyDict>>,
    Vec<(Obj<'py>, Bound<'py, PyList>)>,
);
#[pyfunction]
pub(crate) fn unlink_placements<'py>(
    scene: &Obj<'py>,
    groups: Vec<Obj<'py>>,
    fields: Vec<String>,
    pairs: &Obj<'py>,
) -> PyResult<UnlinkPlan<'py>> {
    let py = scene.py();
    let links = py.import("core.links")?;
    let names = py.import("core.kept_components")?;
    let mut before = Vec::new();
    let mut after: Vec<Bound<'py, PyDict>> = Vec::new();
    let mut nested = Vec::new();
    for g in &groups {
        let row = PyDict::new(py);
        for field in &fields {
            row.set_item(field, g.getattr(field.as_str())?)?;
        }
        before.push(row);
    }
    for g in &groups {
        let inner = PyList::empty(py);
        let copied = links.call_method1("unlinked_copy", (g, pairs, &inner))?;
        nested.push((
            links.call_method1("link_of", (g,))?.get_item("path")?,
            inner,
        ));
        let mut name = g.getattr("name")?;
        if name.is_truthy()? {
            let skip = PyDict::new(py);
            skip.set_item("skip", g)?;
            let used = names.getattr("names_in_use")?.call((scene,), Some(&skip))?;
            for prior in &after {
                used.call_method1("add", (prior.as_any().get_item("name")?,))?;
            }
            if used.contains(&name)? {
                name = names.call_method1("numbered_name", (name, used))?;
            }
        }
        let row = PyDict::new(py);
        for (i, field) in ["mesh", "children", "ext"].into_iter().enumerate() {
            row.set_item(field, copied.get_item(i)?)?;
        }
        row.set_item("name", name)?;
        row.set_item("component", true)?;
        after.push(row);
    }
    restore_definitions(groups, after.clone())?;
    Ok((before, after, nested))
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "definition_edits")?;
    sub.add_function(wrap_pyfunction!(group_owner, &sub)?)?;
    sub.add_function(wrap_pyfunction!(delete_placement, &sub)?)?;
    sub.add_function(wrap_pyfunction!(restore_placement, &sub)?)?;
    sub.add_function(wrap_pyfunction!(set_placement_field, &sub)?)?;
    sub.add_function(wrap_pyfunction!(append_members, &sub)?)?;
    sub.add_function(wrap_pyfunction!(remove_members, &sub)?)?;
    sub.add_function(wrap_pyfunction!(rebuild_letter_placement, &sub)?)?;
    sub.add_function(wrap_pyfunction!(restore_letter_placement, &sub)?)?;
    sub.add_function(wrap_pyfunction!(unlink_placements, &sub)?)?;
    sub.add_function(wrap_pyfunction!(replacement_targets, &sub)?)?;
    sub.add_function(wrap_pyfunction!(replace_definitions, &sub)?)?;
    sub.add_function(wrap_pyfunction!(restore_definitions, &sub)?)?;
    sub.add_function(wrap_pyfunction!(keep_template, &sub)?)?;
    sub.add_function(wrap_pyfunction!(restore_template, &sub)?)?;
    sub.add_function(wrap_pyfunction!(merge_group_geometry, &sub)?)?;
    sub.add_function(wrap_pyfunction!(replace_roots, &sub)?)?;
    m.add_submodule(&sub)?;
    Ok(())
}
