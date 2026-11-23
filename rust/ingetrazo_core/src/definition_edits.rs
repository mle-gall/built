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
fn replacement_targets<'py>(groups: &Obj<'py>, source: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
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
fn replace_definitions<'py>(
    targets: Vec<Obj<'py>>,
    source: &Obj<'py>,
    fields: Vec<String>,
    native_document: bool,
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
        let field = if native_document {
            "definition"
        } else {
            "mesh"
        };
        g.setattr(field, source.getattr(field)?)?;
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
fn restore_definitions(groups: Vec<Obj<'_>>, before: Vec<Bound<'_, PyDict>>) -> PyResult<()> {
    for (g, values) in groups.into_iter().zip(before) {
        for (k, v) in values {
            g.setattr(k.extract::<String>()?.as_str(), v)?;
        }
    }
    Ok(())
}
#[pyfunction]
fn keep_template<'py>(
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
fn restore_template(
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
fn merge_group_geometry<'py>(groups: &Obj<'py>) -> PyResult<Obj<'py>> {
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
fn replace_roots<'py>(
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
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "definition_edits")?;
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
