// SPDX-License-Identifier: GPL-3.0-or-later
//! Build incidence from a welded corner stream without allocating packed numpy
//! arrays or reordering the first occurrence of an edge.
use numpy::PyReadonlyArray1;
use pyo3::{
    exceptions::{PyIndexError, PyValueError},
    intern,
    prelude::*,
    types::{PyDict, PyList, PySet},
};
use std::collections::HashMap;
type Obj<'py> = Bound<'py, PyAny>;

// NumPy indices are already packed int64 on import. Avoid boxing every index
// into a Python integer. Callers can also supply lists or iterators.
fn indices(values: &Obj<'_>) -> PyResult<Vec<i64>> {
    if let Ok(rows) = values.extract::<PyReadonlyArray1<'_, i64>>() {
        return Ok(rows.as_array().iter().copied().collect());
    }
    values.try_iter()?.map(|item| item?.extract()).collect()
}

fn sizes(values: &Obj<'_>) -> PyResult<Vec<usize>> {
    indices(values)?
        .into_iter()
        .map(|i| {
            usize::try_from(i).map_err(|_| PyValueError::new_err("negative ring size or count"))
        })
        .collect()
}

struct Builder<'py> {
    mesh: Obj<'py>,
    vertices: Vec<Obj<'py>>,
    edges: Bound<'py, PyList>,
    edge_class: Obj<'py>,
    face_class: Obj<'py>,
    next_key: Obj<'py>,
    map: Obj<'py>,
    dict: Option<Bound<'py, PyDict>>,
}
impl<'py> Builder<'py> {
    fn new(mesh: &Obj<'py>, vertices: &Obj<'py>, map: Option<&Obj<'py>>) -> PyResult<Self> {
        let py = mesh.py();
        let api = py.import("core.mesh")?;
        let map = map.cloned().unwrap_or_else(|| PyDict::new(py).into_any());
        let dict = map.cast_exact::<PyDict>().ok().cloned();
        Ok(Self {
            mesh: mesh.clone(),
            vertices: vertices.try_iter()?.collect::<PyResult<_>>()?,
            edges: mesh.getattr("edges")?.cast_into()?,
            edge_class: api.getattr("Edge")?,
            face_class: api.getattr("Face")?,
            next_key: mesh.getattr("_keys")?.getattr("__next__")?,
            map,
            dict,
        })
    }
    fn kwargs(&self) -> PyResult<Bound<'py, PyDict>> {
        let kw = PyDict::new(self.mesh.py());
        kw.set_item(intern!(self.mesh.py(), "key"), self.next_key.call0()?)?;
        Ok(kw)
    }
    fn vertex(&self, i: i64) -> PyResult<&Obj<'py>> {
        let n = self.vertices.len() as i64;
        let j = if i < 0 { n + i } else { i };
        self.vertices
            .get(j as usize)
            .ok_or_else(|| PyIndexError::new_err("vertex index outside weld batch"))
    }
    fn pair(&self, a: i64, b: i64) -> i128 {
        a.min(b) as i128 * self.vertices.len() as i128 + a.max(b) as i128
    }
    fn edge(&self, a: i64, b: i64) -> PyResult<Obj<'py>> {
        let k = self.pair(a, b);
        if let Some(dict) = &self.dict {
            if let Some(e) = dict.get_item(k)? {
                if !e.is_none() {
                    return Ok(e);
                }
            }
        } else {
            let existing = self
                .map
                .call_method1(intern!(self.mesh.py(), "get"), (k,))?;
            if !existing.is_none() {
                return Ok(existing);
            }
        }
        let (v, w) = (self.vertex(a)?, self.vertex(b)?);
        let e = self.edge_class.call((v, w), Some(&self.kwargs()?))?;
        v.getattr(intern!(self.mesh.py(), "edges"))?
            .cast_into::<PySet>()?
            .add(&e)?;
        w.getattr(intern!(self.mesh.py(), "edges"))?
            .cast_into::<PySet>()?
            .add(&e)?;
        self.edges.append(&e)?;
        self.map.set_item(k, &e)?;
        Ok(e)
    }
    fn register_existing(&self) -> PyResult<()> {
        let ids = self
            .vertices
            .iter()
            .enumerate()
            .map(|(i, v)| {
                Ok((
                    v.getattr(intern!(self.mesh.py(), "key"))?
                        .extract::<i128>()?,
                    i as i64,
                ))
            })
            .collect::<PyResult<HashMap<_, _>>>()?;
        for e in self.edges.iter() {
            let key = intern!(self.mesh.py(), "key");
            let a = e
                .getattr(intern!(self.mesh.py(), "v0"))?
                .getattr(key)?
                .extract::<i128>()?;
            let z = e
                .getattr(intern!(self.mesh.py(), "v1"))?
                .getattr(key)?
                .extract::<i128>()?;
            if let (Some(&a), Some(&b)) = (ids.get(&a), ids.get(&z)) {
                self.map
                    .call_method1(intern!(self.mesh.py(), "setdefault"), (self.pair(a, b), e))?;
            }
        }
        Ok(())
    }
}

#[pyfunction]
#[pyo3(signature=(mesh, vertices, ia, ib, flags=None))]
pub(crate) fn add_edges_welded<'py>(
    mesh: &Obj<'py>,
    vertices: &Obj<'py>,
    ia: &Obj<'py>,
    ib: &Obj<'py>,
    flags: Option<&Obj<'py>>,
) -> PyResult<Obj<'py>> {
    let (ia, ib) = (indices(ia)?, indices(ib)?);
    let b = Builder::new(mesh, vertices, None)?;
    if ib.len() < ia.len() || flags.is_some_and(|f| f.len().is_ok_and(|n| n < ia.len())) {
        return Err(PyIndexError::new_err("missing edge endpoint or flags"));
    }
    for (i, (&a, &z)) in ia.iter().zip(&ib).enumerate() {
        if b.vertex(a)?.is(b.vertex(z)?) {
            continue;
        }
        let edge = b.edge(a, z)?;
        if let Some(flags) = flags {
            let row = flags.get_item(i)?;
            if row.len()? < 3 {
                return Err(PyValueError::new_err(
                    "edge flags require soft, curve and layer",
                ));
            }
            for (idx, name) in ["soft", "curve", "layer", "hidden", "flat"]
                .iter()
                .enumerate()
                .take(row.len()?.min(5))
            {
                let value = row.get_item(idx)?;
                if (idx == 1 || idx == 2) && !value.is_none() {
                    edge.setattr(*name, &value)?;
                }
                if (idx == 0 || idx >= 3) && value.is_truthy()? {
                    edge.setattr(*name, true)?;
                }
            }
        }
    }
    Ok(b.map)
}

#[pyfunction]
#[pyo3(signature=(mesh, vertices, inverse, ring_sizes, counts, attrs=None, edge_map=None))]
pub(crate) fn add_faces_welded<'py>(
    mesh: &Obj<'py>,
    vertices: &Obj<'py>,
    inverse: &Obj<'py>,
    ring_sizes: &Obj<'py>,
    counts: &Obj<'py>,
    attrs: Option<&Obj<'py>>,
    edge_map: Option<&Obj<'py>>,
) -> PyResult<Bound<'py, PyList>> {
    let py = mesh.py();
    let result = PyList::empty(py);
    let counts = sizes(counts)?;
    if counts.is_empty() {
        return Ok(result);
    }
    let inverse = indices(inverse)?;
    let sizes = sizes(ring_sizes)?;
    if counts.contains(&0)
        || counts.iter().try_fold(0usize, |s, &n| s.checked_add(n)) != Some(sizes.len())
        || sizes.iter().try_fold(0usize, |s, &n| s.checked_add(n)) != Some(inverse.len())
    {
        return Err(PyValueError::new_err(
            "ring sizes and face counts must describe all welded corners",
        ));
    }
    let b = Builder::new(mesh, vertices, edge_map)?;
    for &i in &inverse {
        b.vertex(i)?;
    }
    if edge_map.is_none() && !b.edges.is_empty() {
        b.register_existing()?;
    }
    let mut start = 0;
    let mut rings = Vec::with_capacity(sizes.len());
    let mut incidence = Vec::with_capacity(inverse.len());
    let mut seen = HashMap::new();
    for size in sizes {
        let ring = &inverse[start..start + size];
        start += size;
        rings.push(PyList::new(
            py,
            ring.iter()
                .map(|&j| b.vertex(j).cloned())
                .collect::<PyResult<Vec<_>>>()?,
        )?);
        for (&a, &z) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            let k = b.pair(a, z);
            let edge = match seen.entry(k) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => entry.insert(
                    b.edge(a, z)?
                        .getattr(intern!(py, "faces"))?
                        .cast_into::<PyList>()?,
                ),
            };
            incidence.push(edge.clone());
        }
    }
    let faces = mesh.getattr("faces")?.cast_into::<PyList>()?;
    let mut radial = incidence.into_iter();
    let mut ring = 0;
    for (i, count) in counts.into_iter().enumerate() {
        let face = b.face_class.call(
            (
                &rings[ring],
                PyList::new(py, &rings[ring + 1..ring + count])?,
            ),
            Some(&b.kwargs()?),
        )?;
        if let Some(rows) = attrs {
            let a = rows.get_item(i)?;
            if a.is_truthy()? {
                face.getattr("attrs")?.call_method1("update", (a,))?;
            }
        }
        faces.append(&face)?;
        result.append(&face)?;
        for r in &rings[ring..ring + count] {
            for edge_faces in radial.by_ref().take(r.len()) {
                edge_faces.append(&face)?;
            }
        }
        ring += count;
    }
    Ok(result)
}
