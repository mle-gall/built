// SPDX-License-Identifier: GPL-3.0-or-later
//! Selection growth stays within the active edit context.
use pyo3::{
    prelude::*,
    types::{PyList, PySet, PyTuple},
};
type Obj<'p> = Bound<'p, PyAny>;
fn groups<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    match scene.getattr("edit_group").ok().filter(|g| !g.is_none()) {
        Some(g) => g.getattr("children"),
        None => scene.getattr("groups"),
    }
}
fn loops<'p>(face: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    let mut rings = vec![face.getattr("loop")?];
    for hole in face.getattr("hole_loops")?.try_iter()? {
        rings.push(hole?);
    }
    Ok(rings)
}
#[pyfunction]
fn all_connected<'p>(entities: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let py = entities.py();
    let mesh = py.import("built_core.mesh")?;
    let face = mesh.getattr("Face")?;
    let edge = mesh.getattr("Edge")?;
    let mut stack = vec![];
    for obj in entities.try_iter()? {
        let obj = obj?;
        if obj.is_instance(&face)? {
            for ring in loops(&obj)? {
                stack.extend(ring.try_iter()?.collect::<PyResult<Vec<_>>>()?);
            }
        } else if obj.is_instance(&edge)? {
            stack.extend([obj.getattr("v0")?, obj.getattr("v1")?]);
        }
    }
    let seen = PySet::empty(py)?;
    let edges = PySet::empty(py)?;
    let faces = PySet::empty(py)?;
    while let Some(v) = stack.pop() {
        if seen.contains(&v)? {
            continue;
        }
        seen.add(&v)?;
        for edge in v.getattr("edges")?.try_iter()? {
            let edge = edge?;
            if edges.contains(&edge)? {
                continue;
            }
            edges.add(&edge)?;
            stack.push(edge.call_method1("other", (&v,))?);
            for f in edge.getattr("faces")?.try_iter()? {
                let f = f?;
                if faces.contains(&f)? {
                    continue;
                }
                faces.add(&f)?;
                for ring in loops(&f)? {
                    stack.extend(ring.try_iter()?.collect::<PyResult<Vec<_>>>()?);
                }
            }
        }
    }
    let result = PyList::empty(py);
    result.call_method1("extend", (edges,))?;
    result.call_method1("extend", (faces,))?;
    Ok(result)
}
#[pyfunction]
fn bounding_edges<'p>(entities: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let py = entities.py();
    let face = py.import("built_core.mesh")?.getattr("Face")?;
    let faces = PySet::empty(py)?;
    for obj in entities.try_iter()? {
        let obj = obj?;
        if obj.is_instance(&face)? {
            faces.add(obj)?;
        }
    }
    let seen = PySet::empty(py)?;
    let out = PyList::empty(py);
    for f in faces.iter() {
        for ring in loops(&f)? {
            let n = ring.len()?;
            for i in 0..n {
                let a = ring.get_item(i)?;
                let b = ring.get_item((i + 1) % n)?;
                for e in a.getattr("edges")?.try_iter()? {
                    let e = e?;
                    let incident = e.getattr("faces")?;
                    if e.call_method1("other", (&a,))?.eq(&b)? && incident.contains(&f)? {
                        if !seen.contains(&e)? {
                            seen.add(&e)?;
                            let mut count = 0;
                            for g in incident.try_iter()? {
                                if faces.contains(g?)? {
                                    count += 1;
                                }
                            }
                            if count == 1 {
                                out.append(e)?;
                            }
                        }
                        break;
                    }
                }
            }
        }
    }
    Ok(out)
}
fn material_key<'p>(f: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = f.py();
    let attrs = f.getattr("attrs")?;
    let get = |k: &str| attrs.call_method1("get", (k,));
    let mat = get("mat")?;
    if mat.is_truthy()? {
        return ("mat", mat).into_pyobject(py).map(|v| v.into_any());
    }
    let color = get("color")?;
    let tex = get("texture")?;
    let path = if tex.is_instance_of::<pyo3::types::PyDict>() {
        tex.call_method1("get", ("path",))?
    } else {
        py.None().into_bound(py)
    };
    if color.is_none() && path.is_none() {
        return Ok(py.None().into_bound(py));
    }
    let col = if color.is_truthy()? {
        let round = py.import("builtins")?.getattr("round")?;
        PyTuple::new(
            py,
            color
                .try_iter()?
                .map(|x| round.call1((x?.extract::<f64>()?, 4)))
                .collect::<PyResult<Vec<_>>>()?,
        )?
        .into_any()
    } else {
        py.None().into_bound(py)
    };
    ("paint", col, path).into_pyobject(py).map(|v| v.into_any())
}
#[pyfunction]
fn same_material<'p>(scene: &Obj<'p>, entities: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let py = scene.py();
    let fc = py.import("built_core.mesh")?.getattr("Face")?;
    let gc = py.import("built_core.group")?.getattr("Group")?;
    let paints = PySet::empty(py)?;
    let materials = PyList::empty(py);
    let out = PyList::empty(py);
    for e in entities.try_iter()? {
        let e = e?;
        if e.is_instance(&fc)? {
            paints.add(material_key(&e)?)?;
        } else if e.is_instance(&gc)? {
            let m = e.getattr("material")?;
            if m.is_truthy()? {
                materials.append(m)?;
            }
        }
    }
    if !paints.is_empty() {
        for f in scene.getattr("faces")?.try_iter()? {
            let f = f?;
            if paints.contains(material_key(&f)?)? {
                out.append(f)?;
            }
        }
    }
    if !materials.is_empty() {
        for g in groups(scene)?.try_iter()? {
            let g = g?;
            if materials.contains(g.getattr("material")?)? {
                out.append(g)?;
            }
        }
    }
    Ok(out)
}
fn matching_layers<'p>(
    scene: &Obj<'p>,
    names: &Bound<PySet>,
    check: bool,
) -> PyResult<Bound<'p, PyList>> {
    let layer = scene.py().import("built_core.layers")?.getattr("layer_of")?;
    let out = PyList::empty(scene.py());
    if names.is_empty() {
        return Ok(out);
    }
    let mut pool = vec![
        scene.getattr("edges")?,
        scene.getattr("faces")?,
        groups(scene)?,
    ];
    if check {
        for attr in ["dimensions", "text_labels", "guides", "image_planes"] {
            if let Some(a) = scene.getattr(attr).ok().filter(|a| !a.is_none()) {
                pool.push(a);
            }
        }
    }
    for part in pool {
        for obj in part.try_iter()? {
            let obj = obj?;
            if names.contains(layer.call1((&obj,))?)?
                && (!check
                    || scene
                        .call_method1("entity_selectable", (&obj,))?
                        .is_truthy()?)
            {
                out.append(obj)?;
            }
        }
    }
    Ok(out)
}
#[pyfunction]
fn same_layer<'p>(scene: &Obj<'p>, entities: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let py = scene.py();
    let mesh = py.import("built_core.mesh")?;
    let types = PyTuple::new(
        py,
        [
            mesh.getattr("Edge")?,
            mesh.getattr("Face")?,
            py.import("built_core.group")?.getattr("Group")?,
        ],
    )?;
    let layer = py.import("built_core.layers")?.getattr("layer_of")?;
    let names = PySet::empty(py)?;
    for obj in entities.try_iter()? {
        let obj = obj?;
        if obj.is_instance(&types)? {
            names.add(layer.call1((obj,))?)?;
        }
    }
    matching_layers(scene, &names, false)
}
#[pyfunction]
fn on_layers<'p>(scene: &Obj<'p>, names: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let selected = PySet::empty(scene.py())?;
    for name in names.try_iter()? {
        selected.add(name?)?;
    }
    matching_layers(scene, &selected, true)
}
#[pyfunction]
fn same_definition<'p>(scene: &Obj<'p>, entities: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let py = scene.py();
    let gc = py.import("built_core.group")?.getattr("Group")?;
    let mut definitions = std::collections::HashSet::new();
    for g in entities.try_iter()? {
        let g = g?;
        if g.is_instance(&gc)? && g.call_method0("is_component")?.is_truthy()? {
            definitions.insert(g.getattr("mesh")?.as_ptr());
        }
    }
    let out = PyList::empty(py);
    if !definitions.is_empty() {
        for g in groups(scene)?.try_iter()? {
            let g = g?;
            if g.call_method0("is_component")?.is_truthy()?
                && definitions.contains(&g.getattr("mesh")?.as_ptr())
            {
                out.append(g)?;
            }
        }
    }
    Ok(out)
}
pub(crate) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    for f in [
        wrap_pyfunction!(all_connected, m)?,
        wrap_pyfunction!(bounding_edges, m)?,
        wrap_pyfunction!(same_material, m)?,
        wrap_pyfunction!(same_layer, m)?,
        wrap_pyfunction!(same_definition, m)?,
        wrap_pyfunction!(on_layers, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
