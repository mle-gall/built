// SPDX-License-Identifier: GPL-3.0-or-later
//! Two questions the Solid Inspector asks of a store-backed mesh before
//! the orientation pass (`core/solid_analysis.py`): which faces are
//! edge-connected (one label per piece), and how many edges two faces walk
//! the same way (a consistently wound skin walks each shared edge once each
//! way). Read from the store's topology, not from Python face objects.

use std::collections::HashMap;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::store::keys::Kind;
use crate::store::py::PyMeshStore;

fn store_of<'py>(mesh: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyMeshStore>> {
    Ok(mesh.getattr("_store")?.cast_into::<PyMeshStore>()?)
}

fn keys(items: &Bound<'_, PyAny>) -> PyResult<Vec<u128>> {
    items.try_iter()?.map(|f| f?.getattr("key")?.extract()).collect()
}

/// `solid_face_components(mesh, faces)`: for each of `faces` (views of
/// `mesh`) the label of its edge-connected piece, equal labels for faces
/// joined through shared edges.
#[pyfunction]
fn solid_face_components(mesh: &Bound<'_, PyAny>, faces: &Bound<'_, PyAny>) -> PyResult<Vec<usize>> {
    let store = store_of(mesh)?;
    let st = &store.borrow().inner;
    let slots: Vec<u32> = keys(faces)?.into_iter()
        .map(|k| st.slot(Kind::Face, k).ok_or_else(|| PyValueError::new_err("a face the store does not hold")))
        .collect::<PyResult<_>>()?;
    let index: HashMap<u32, usize> = slots.iter().enumerate().map(|(i, s)| (*s, i)).collect();
    let mut parent: Vec<usize> = (0..slots.len()).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    for e in st.live_slots(Kind::Edge) {
        let mut on = st.edge_faces(e).into_iter().filter_map(|f| index.get(&f).copied());
        let Some(first) = on.next() else { continue };
        for j in on {
            let (a, b) = (find(&mut parent, first), find(&mut parent, j));
            if a != b {
                parent[a] = b;
            }
        }
    }
    Ok((0..slots.len()).map(|i| find(&mut parent, i)).collect())
}

/// `solid_incoherent_edges(mesh, faces)`: how many edges two of `faces`
/// both walk in the same direction.
#[pyfunction]
fn solid_incoherent_edges(mesh: &Bound<'_, PyAny>, faces: &Bound<'_, PyAny>) -> PyResult<usize> {
    let store = store_of(mesh)?;
    let st = &store.borrow().inner;
    let mut uses: HashMap<(u32, u32), Vec<(u32, u32)>> = HashMap::new();
    for k in keys(faces)? {
        let Some(f) = st.slot(Kind::Face, k) else { continue };
        for ring in st.face_rings(f) {
            let n = ring.len();
            for i in 0..n {
                let (a, b) = (ring[i], ring[(i + 1) % n]);
                uses.entry((a.min(b), a.max(b))).or_default().push((a, b));
            }
        }
    }
    Ok(uses.values().filter(|u| u.len() == 2 && u[0] == u[1]).count())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(solid_face_components, m)?)?;
    m.add_function(wrap_pyfunction!(solid_incoherent_edges, m)?)?;
    Ok(())
}
