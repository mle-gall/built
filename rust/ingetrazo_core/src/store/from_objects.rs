// SPDX-License-Identifier: GPL-3.0-or-later
//! Import materialised handles in their public list order. Validate the whole
//! graph before the facade switches representation or exposes a replacement key.
use super::{
    build::{Block, Points},
    keys::Kind,
    Store, Weld, FLAT, HIDDEN, INTERIOR, SOFT,
};
use crate::math::xyz;
use pyo3::{
    exceptions::{PyKeyError, PyValueError},
    prelude::*,
    types::{PyDict, PyInt},
};
use std::collections::HashMap;

type Obj<'py> = Bound<'py, PyAny>;
type Dict<'py> = Bound<'py, PyDict>;
pub(super) fn read<'py>(
    mesh: &Obj<'py>,
    namespace: u64,
    weld: Weld,
) -> PyResult<(Store, Dict<'py>, Dict<'py>)> {
    let py = mesh.py();
    let rows = |name| {
        mesh.getattr(name)?
            .try_iter()?
            .collect::<PyResult<Vec<_>>>()
    };
    let verts = rows("vertices")?;
    let edges = rows("edges")?;
    let faces = rows("faces")?;
    let mut indices = HashMap::new();
    let mut points = Vec::with_capacity(verts.len());
    for (i, v) in verts.iter().enumerate() {
        indices.insert(
            v.getattr(pyo3::intern!(py, "key"))?.extract::<i128>()?,
            i as i64,
        );
        points.push(xyz(&v.getattr(pyo3::intern!(py, "position"))?)?.to_array());
    }
    let index = |v: Obj<'py>| -> PyResult<i64> {
        let k = v.getattr(pyo3::intern!(py, "key"))?.extract::<i128>()?;
        indices
            .get(&k)
            .copied()
            .ok_or_else(|| PyKeyError::new_err(k))
    };
    let mut ends = Vec::with_capacity(edges.len());
    let mut flags = Vec::with_capacity(edges.len());
    let mut curves = Vec::with_capacity(edges.len());
    let mut layers = Vec::with_capacity(edges.len());
    let mut by_name = HashMap::new();
    let mut names = Vec::new();
    for e in &edges {
        ends.push([
            index(e.getattr(pyo3::intern!(py, "v0"))?)?,
            index(e.getattr(pyo3::intern!(py, "v1"))?)?,
        ]);
        let soft = e.getattr(pyo3::intern!(py, "soft"))?.is_truthy()?;
        let mut bits = if soft { SOFT } else { 0 };
        if e.getattr(pyo3::intern!(py, "hidden"))?.is_truthy()? {
            bits |= HIDDEN;
        }
        if soft {
            if let Some(flat) = e.getattr_opt(pyo3::intern!(py, "flat"))? {
                if flat.is_truthy()? {
                    bits |= FLAT;
                }
            }
        }
        flags.push(bits);
        let c = e.getattr(pyo3::intern!(py, "curve"))?;
        curves.push(if c.is_none() {
            -1
        } else {
            py.get_type::<PyInt>().call1((c,))?.extract()?
        });
        let layer = e.getattr(pyo3::intern!(py, "layer"))?;
        let li = if layer.is_none() {
            -1
        } else {
            let name = layer.extract::<String>()?;
            *by_name.entry(name.clone()).or_insert_with(|| {
                let i = names.len() as i64;
                names.push(name);
                i
            })
        };
        layers.push(li);
    }
    let mut corners = Vec::new();
    let mut sizes = Vec::new();
    let mut counts = Vec::with_capacity(faces.len());
    for f in &faces {
        let mut count = 0;
        for ring in std::iter::once(f.getattr(pyo3::intern!(py, "loop")))
            .chain(f.getattr(pyo3::intern!(py, "hole_loops"))?.try_iter()?)
        {
            let ring = ring?;
            sizes.push(ring.len()? as i64);
            for v in ring.try_iter()? {
                corners.push(index(v?)?);
            }
            count += 1;
        }
        counts.push(count);
    }
    let block = Block {
        pos: Points::Double(&points),
        edges: &ends,
        flags: Some(&flags),
        curves: Some(&curves),
        layers: Some(&layers),
        layer_names: names,
        rings: &corners,
        sizes: &sizes,
        counts: &counts,
        sequential: true,
        ..Block::default()
    };
    let mut store = py
        .detach(|| Store::from_block(namespace, &block, weld))
        .map_err(PyValueError::new_err)?;
    let api = py.import("core.store_mesh")?;
    let own = PyDict::new(py);
    let views = PyDict::new(py);
    // id(original) is the compatibility return key. All geometry references
    // and all newly issued handles above and below use the mesh element keys.
    let object_id = py.import("builtins")?.getattr(pyo3::intern!(py, "id"))?;
    for (kind, objects, name) in [
        (Kind::Vertex, &verts, "VertexView"),
        (Kind::Edge, &edges, "EdgeView"),
        (Kind::Face, &faces, "FaceView"),
    ] {
        if store.n(kind) != objects.len() {
            return Err(PyErr::from_value(
                api.getattr(pyo3::intern!(py, "StoreRefusal"))?
                    .call1(("to_store: the store would weld or drop elements of this mesh",))?,
            ));
        }
        let class = api.getattr(name)?;
        for (slot, obj) in objects.iter().enumerate() {
            let key = store.key(kind, slot as u32);
            if kind == Kind::Face {
                if obj.getattr(pyo3::intern!(py, "interior"))?.is_truthy()? {
                    store.set_face_flags(slot as u32, INTERIOR);
                }
                let attrs = obj.getattr(pyo3::intern!(py, "attrs"))?;
                if attrs.is_truthy()? {
                    own.set_item(key, attrs)?;
                }
            }
            views.set_item(object_id.call1((obj,))?, class.call1((mesh, key))?)?;
        }
    }
    Ok((store, own, views))
}
