// SPDX-License-Identifier: GPL-3.0-or-later
//! Cache the loose and fixed visible triangles as identity placements.
use super::*;
use pyo3::types::PySlice;

#[pyfunction]
pub fn static_occluders<'py>(view: &Obj<'py>, idx: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    let cached = attr(idx, "_static_occl")?;
    if !cached.is_none() {
        return Ok(cached);
    }
    let module = py
        .import("views.document_interaction")?
        .call_method0("_occlusion_native")?;
    if module.is_none() || !module.hasattr("InstanceSet")? {
        return Ok(none(py));
    }
    let np = py.import("numpy")?;
    let memo = attr(view, "_pick_static_memo")?;
    let fixed = if memo.is_none() {
        none(py)
    } else {
        memo.get_item(1)?.get_item("v0")?
    };
    let v0 = idx.getattr("tri_v0")?;
    let n = v0.len()?;
    let mut nf = if fixed.is_none() { 0 } else { fixed.len()? };
    if nf > n
        || (nf > 0
            && !np
                .call_method1("array_equal", (v0.get_item(n - nf)?, fixed.get_item(0)?))?
                .is_truthy()?)
    {
        nf = 0;
    }
    let vis = idx.getattr("ent_vis")?.get_item(idx.getattr("tri_ent")?)?;
    let slice = |i: usize, j: usize| PySlice::new(py, i as isize, j as isize, 1);
    let proto = |lo: usize, hi: usize| -> PyResult<Obj<'py>> {
        let keep = np
            .call_method1("flatnonzero", (vis.get_item(slice(lo, hi))?,))?
            .add(lo)?;
        let col = |name: &str| {
            np.call_method1(
                "ascontiguousarray",
                (idx.getattr(name)?.get_item(&keep)?, "float64"),
            )
        };
        let p = module.getattr("InstanceProto")?.call1((
            col("tri_v0")?,
            col("tri_e1")?,
            col("tri_e2")?,
            np.call_method1("zeros", ((0, 6),))?,
        ))?;
        Ok((p, keep).into_pyobject(py)?.into_any())
    };
    let mut parts = vec![proto(0, n - nf)?];
    if nf > 0 {
        let cache = attr(view, "_static_occl_memo")?;
        let mask = vis.get_item(slice(n - nf, n))?;
        parts.push(
            if !cache.is_none()
                && cache.get_item(0)?.is(&fixed)
                && np
                    .call_method1("array_equal", (cache.get_item(1)?, &mask))?
                    .is_truthy()?
            {
                cache.get_item(2)?
            } else {
                let part = proto(n - nf, n)?;
                view.setattr(
                    "_static_occl_memo",
                    (fixed, mask.call_method0("copy")?, &part),
                )?;
                part
            },
        );
    }
    let mut protos = Vec::new();
    let mut rows = Vec::new();
    let mut starts = Vec::new();
    let mut end = 0;
    for part in &parts {
        protos.push(part.get_item(0)?);
        starts.push(end);
        let ids = part.get_item(1)?;
        end += ids.len()? as i64;
        rows.push(ids);
    }
    let indices = (0..parts.len() as i64).collect::<Vec<_>>().into_pyarray(py);
    let matrices = np.call_method1(
        "tile",
        (
            np.call_method1("eye", (4,))?
                .call_method1("reshape", (1, 16))?,
            (parts.len(), 1),
        ),
    )?;
    let set = module
        .getattr("InstanceSet")?
        .call1((protos, &indices, matrices))?;
    let result = (
        set,
        indices,
        np.call_method1("concatenate", (rows,))?,
        starts.into_pyarray(py),
    )
        .into_pyobject(py)?
        .into_any();
    idx.setattr("_static_occl", &result)?;
    Ok(result)
}

#[pyfunction]
pub fn static_ray_rows<'py>(
    view: &Obj<'py>,
    idx: &Obj<'py>,
    mask: &Obj<'py>,
    origin: &Obj<'py>,
    direction: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = view.py();
    if !view.hasattr("_lazy_native")? || view.call_method0("_lazy_native")?.is_none() {
        return Ok(none(py));
    }
    let lookup = idx
        .getattr("__dict__")?
        .call_method1("setdefault", ("_mask_in_vis", PyDict::new(py)))?;
    let id = mask.as_ptr() as usize;
    let memo = lookup.call_method1("get", (id,))?;
    let ok = if !memo.is_none() && memo.get_item(0)?.is(mask) {
        memo.get_item(1)?.is_truthy()?
    } else {
        let outside = mask.call_method1(
            "__and__",
            (idx.getattr("ent_vis")?.call_method0("__invert__")?,),
        )?;
        let yes = !outside.call_method0("any")?.is_truthy()?;
        lookup.set_item(id, (mask, yes))?;
        yes
    };
    if !ok {
        return Ok(none(py));
    }
    let stat = view.call_method1("_static_occluders", (idx,))?;
    if stat.is_none() {
        return Ok(none(py));
    }
    let result = stat.get_item(0)?.call_method1(
        "ray_cands",
        (stat.get_item(1)?, origin, direction, f64::INFINITY),
    )?;
    let indices = stat
        .get_item(3)?
        .get_item(result.get_item(0)?)?
        .add(result.get_item(1)?)?;
    py.import("numpy")?
        .call_method1("sort", (stat.get_item(2)?.get_item(indices)?,))
}

#[pyfunction]
pub fn flat_pick_index<'py>(idx: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = idx.py();
    let own = attr(idx, "own_spans")?;
    if !own.is_truthy()? {
        return Ok(idx.clone());
    }
    let cache = attr(idx, "_flat")?;
    if !cache.is_none() {
        return Ok(cache);
    }
    let flat = namespace(py)?;
    flat.getattr("__dict__")?
        .call_method1("update", (idx.getattr("__dict__")?,))?;
    let np = py.import("numpy")?;
    let fixed = !idx.getattr("tri_v0")?.is_none();
    for (i, name) in ["tri_v0", "tri_e1", "tri_e2", "tri_ent"].iter().enumerate() {
        let mut parts = Vec::new();
        if fixed {
            parts.push(idx.getattr(*name)?);
        }
        for row in own.try_iter()? {
            let row = row?;
            let col = row.get_item(i + 1)?;
            parts.push(if i == 3 {
                col.add(row.get_item(5)?)?
            } else {
                col
            });
        }
        flat.setattr(*name, np.call_method1("concatenate", (parts,))?)?;
    }
    flat.setattr("tri_spans", py.None())?;
    flat.setattr("own_spans", PyList::empty(py))?;
    idx.setattr("_flat", &flat)?;
    Ok(flat)
}

#[pyfunction]
pub fn outer_loose_meshes<'py>(view: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    let scene = view.getattr("scene")?;
    let ctx = scene.getattr("edit_group")?;
    let empty = PyTuple::empty(py).into_any();
    if ctx.is_none() {
        return Ok(empty);
    }
    if let Some(f) = view.getattr_opt("_rest_is_hidden")? {
        if f.is_callable() && f.call0()?.is_truthy()? {
            return Ok(empty);
        }
    }
    let outer = attr(&scene, "loose_mesh")?;
    let mesh = scene.getattr("mesh")?;
    if outer.is_none() || outer.is(&mesh) {
        return Ok(empty);
    }
    let key = (
        py.import("views.document_interaction")?
            .call_method1("_content_ver", (view,))?,
        outer.as_ptr() as usize,
        ctx.as_ptr() as usize,
        mesh.as_ptr() as usize,
    )
        .into_pyobject(py)?;
    let memo = attr(view, "_outer_loose_memo")?;
    if !memo.is_none() && memo.get_item(0)?.eq(&key)? {
        return memo.get_item(1);
    }
    let mut meshes = (&outer,).into_pyobject(py)?.into_any();
    for g in view.call_method0("_placements")?.try_iter()? {
        if g?.getattr("mesh")?.is(&outer) {
            meshes = empty;
            break;
        }
    }
    view.setattr("_outer_loose_memo", (key, &meshes))?;
    Ok(meshes)
}

#[pyfunction]
pub fn loose_mesh_wrapper<'py>(view: &Obj<'py>, mesh: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    let mut cache = attr(view, "_loose_wrappers")?;
    if cache.is_none() {
        cache = PyDict::new(py).into_any();
        view.setattr("_loose_wrappers", &cache)?;
    }
    let id = mesh.as_ptr() as usize;
    let cached = cache.call_method1("get", (id,))?;
    if !cached.is_none() {
        return Ok(cached);
    }
    let w = namespace(py)?;
    w.setattr("mesh", mesh)?;
    for name in ["xform", "material", "name", "layer"] {
        w.setattr(name, py.None())?;
    }
    for name in ["billboard", "hidden", "locked"] {
        w.setattr(name, false)?;
    }
    cache.set_item(id, &w)?;
    Ok(w)
}
