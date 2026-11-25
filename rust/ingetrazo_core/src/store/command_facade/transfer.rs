// SPDX-License-Identifier: GPL-3.0-or-later
//! Copy-on-write paint table and provenance for an exploded definition.
use crate::math::Matrix4;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::BTreeMap;
type Obj<'py> = Bound<'py, PyAny>;
fn needs_face(side: &Obj<'_>) -> PyResult<bool> {
    let Ok(side) = side.cast::<PyDict>() else {
        return Ok(false);
    };
    if let Some(t) = side.get_item("texture")? {
        if t.is_truthy()? {
            return Ok(!t.call_method1("get", ("uvw",))?.is_truthy()?);
        }
    }
    if let Some(pbr) = side.get_item("pbr")? {
        if pbr.is_truthy()? {
            for key in pbr.try_iter()? {
                if key?.call_method1("endswith", ("_map",))?.is_truthy()? {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}
fn intern(row: &Obj<'_>, rows: &Bound<'_, PyList>, index: &Bound<'_, PyDict>) -> PyResult<usize> {
    if !row.is_truthy()? {
        return Ok(0);
    }
    let d = PyDict::new(row.py());
    for (k, v) in row.cast::<PyDict>()?.iter() {
        if let Ok(v) = v.cast::<PyDict>() {
            d.set_item(k, v.copy()?)?;
        } else {
            d.set_item(k, v)?;
        }
    }
    let key = d.repr()?;
    if let Some(i) = index.get_item(&key)? {
        return Ok(i.extract::<usize>()? + 1);
    }
    let n = rows.len();
    index.set_item(key, n)?;
    rows.append(d)?;
    Ok(n + 1)
}
pub(super) fn paste<'py>(
    mesh: &Obj<'py>,
    src: &Obj<'py>,
    matrix: Option<Matrix4>,
    inverse: Option<Matrix4>,
    paint: Option<&Obj<'py>>,
) -> PyResult<(Vec<u128>, Obj<'py>)> {
    let py = mesh.py();
    let group = crate::model::module(py, "core.group")?;
    let model = crate::model::module(py, "ingetrazo_core.store_mesh")?;
    let st = mesh.getattr("_store")?;
    let sst = src.getattr("_store")?;
    let snap = sst.call_method0("snapshot")?;
    let positions = sst.call_method0("positions")?;
    let mapped = if let Some(m) = matrix {
        let pair = group.call_method1("np_affine", (m,))?;
        positions
            .call_method1("__matmul__", (pair.get_item(0)?.getattr("T")?,))?
            .add(pair.get_item(1)?)?
    } else {
        positions
    };
    let sd = src.getattr("__dict__")?;
    let src_rows = sd.get_item("_attr_table")?;
    let own = sd.get_item("_attr_own")?;
    let keys = sst.call_method1("keys", ("face",))?;
    let n = keys.len()?;
    let rows: Vec<i64> = if n == 0 {
        vec![]
    } else {
        snap.call_method1("face_rows", (&keys,))?
            .call_method0("tolist")?
            .extract()?
    };
    let d = mesh.getattr("__dict__")?;
    let table = d
        .get_item("_attr_table")?
        .cast_into::<PyList>()?
        .get_slice(0, d.get_item("_attr_table")?.len()?);
    let index = PyDict::new(py);
    for (i, attr) in table.iter().enumerate() {
        index.set_item(attr.repr()?, i)?;
    }
    let mut shared: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    let mut individual = BTreeMap::new();
    for (i, r) in rows.into_iter().enumerate() {
        if let Some(o) = own.cast::<PyDict>()?.get_item(keys.get_item(i)?)? {
            individual.insert(i, o);
        } else {
            shared.entry(r).or_default().push(i);
        }
    }
    let effective = crate::model::module(py, "ingetrazo_core.materials")?.getattr("effective_attrs")?;
    let placed = group.getattr("placed_face_paint")?;
    let face = model.getattr("FaceView")?;
    let inverse = inverse.map(|m| Py::new(py, m)).transpose()?;
    let draw = |base: &Obj<'py>| -> PyResult<Obj<'py>> {
        let d = if base.is_truthy()? {
            py.get_type::<PyDict>().call1((base,))?
        } else {
            PyDict::new(py).into_any()
        };
        effective.call1((d, paint))
    };
    let value = |row: Obj<'py>, i: usize| -> PyResult<usize> {
        let row = if let Some(inv) = &inverse {
            placed.call1((row, face.call1((src, keys.get_item(i)?))?, inv))?
        } else {
            row
        };
        intern(&row, &table, &index)
    };
    let mut attrs = vec![0; n];
    for (r, indices) in shared {
        let base = if r < 0 {
            py.None().into_bound(py)
        } else {
            src_rows.get_item(r)?
        };
        let row = draw(&base)?;
        if inverse.is_some()
            && (needs_face(&row)? || needs_face(&row.call_method1("get", ("back",))?)?)
        {
            for i in indices {
                individual.insert(i, base.clone());
            }
        } else {
            let v = value(row, indices[0])?;
            for i in indices {
                attrs[i] = v;
            }
        }
    }
    for (i, base) in individual {
        attrs[i] = value(draw(&base)?, i)?;
    }
    let made = st
        .call_method1(
            "paste_from",
            (snap, mapped, attrs, model.getattr("_hole_filter")?),
        )?
        .get_item(0)?;
    d.set_item("_attr_table", table)?;
    mesh.call_method1("_dirty", (1,))?;
    let cols = PyDict::new(py);
    for (i, kind) in ["vertex", "edge"].iter().enumerate() {
        cols.set_item(
            *kind,
            (sst.call_method1("keys", (*kind,))?, made.get_item(i + 1)?),
        )?;
    }
    cols.set_item("face", (keys, made.get_item(0)?))?;
    let copies = Py::new(py,crate::kernel::store_views::provenance::CopiedKeys::new(cols))?.into_bound(py).into_any();
    Ok((made.get_item(0)?.extract()?, copies))
}
