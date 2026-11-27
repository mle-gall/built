//! Snapshot arrays are borrowed for the duration of the detached build. Callers
//! must not mutate them until return; no Python objects are read by the kernel.
use super::*;
use numpy::{PyReadonlyArray1, PyUntypedArrayMethods};
use rayon::prelude::*;
use std::collections::HashMap;
type Input<'a> = (
    PyReadonlyArray2<'a, f32>,
    PyReadonlyArray2<'a, f64>,
    PyReadonlyArray2<'a, i64>,
    PyReadonlyArray1<'a, bool>,
    PyReadonlyArray1<'a, i64>,
);

/// Build prototypes and their placement BVH in one detached batch.
/// Each input is (triangle vertices f32[N,3], points f64[V,3], endpoint
/// indices i64[E,2], visibility bool[E], curve ids i64[E]); negative curve
/// ids are unnamed. Curve and segment order is first-seen, for snap ties.
/// `detach` is PyO3 0.29's spelling of `allow_threads`.
#[pyfunction]
pub fn build_snap_index(
    py: Python<'_>,
    inputs: Vec<Input<'_>>,
    ids: PyReadonlyArray1<'_, i64>,
    mats: PyReadonlyArray2<'_, f64>,
    lo: PyReadonlyArray2<'_, f64>,
    hi: PyReadonlyArray2<'_, f64>,
) -> PyResult<(Vec<InstanceProto>, InstanceSet)> {
    let bad = || pyo3::exceptions::PyValueError::new_err("snap array shape or index");
    let mut data = Vec::new();
    for (tri, pos, edges, keep, curves) in &inputs {
        if tri.shape()[1] != 3
            || tri.shape()[0] % 3 != 0
            || pos.shape()[1] != 3
            || edges.shape()[1] != 2
            || keep.len() != edges.shape()[0]
            || curves.len() != edges.shape()[0]
        {
            return Err(bad());
        }
        data.push((
            tri.as_slice()?,
            pos.as_slice()?,
            edges.as_slice()?,
            keep.as_slice()?,
            curves.as_slice()?,
        ));
    }
    let (ids, mats, lo, hi) = (
        ids.as_slice()?,
        mats.as_slice()?,
        lo.as_slice()?,
        hi.as_slice()?,
    );
    if mats.len() != ids.len() * 16 || lo.len() != ids.len() * 3 || hi.len() != ids.len() * 3 {
        return Err(bad());
    }
    py.detach(|| {
        let protos: Vec<InstanceProto> = data
            .into_par_iter()
            .map(|d| prepared(d).map(|(a, b, c, segments, curves)| InstanceProto {
                inner: std::sync::Arc::new(instances::Proto::new(a, b, c, segments)),
                curves,
            }))
            .collect::<PyResult<_>>()?;
        let pr: Vec<_> = protos.iter().map(|p| p.inner.clone()).collect();
        let mut places = Vec::with_capacity(ids.len());
        for (i, &id) in ids.iter().enumerate() {
            if id < 0 || id as usize >= pr.len() {
                return Err(bad());
            }
            let m: [f64; 16] = mats[i * 16..i * 16 + 16].try_into().unwrap();
            let (lo, hi) = instances::grown(
                lo[i * 3..i * 3 + 3].try_into().unwrap(),
                hi[i * 3..i * 3 + 3].try_into().unwrap(),
            );
            places.push(instances::Placement {
                proto: id as usize,
                inv: instances::affine_inverse(&m),
                m,
                lo,
                hi,
            });
        }
        let bvh = instances::PlaceBvh::new(&places, |p| pr[p.proto].n_tris() > 0);
        Ok((
            protos,
            InstanceSet {
                protos: pr,
                places,
                bvh,
            },
        ))
    })
}

type Prepared = (Vec<[f64; 3]>, Vec<[f64; 3]>, Vec<[f64; 3]>, Vec<[[f64; 3]; 2]>, Vec<Vec<[f64; 6]>>);

/// One input's triangles (v0, e1, e2), kept hard edges and curves.
fn prepared((tri, pos, edges, keep, curve_ids): (&[f32], &[f64], &[i64], &[bool], &[i64])) -> PyResult<Prepared> {
    let bad = || pyo3::exceptions::PyValueError::new_err("snap array shape or index");
    let mut a = Vec::with_capacity(tri.len() / 9);
    let mut b = Vec::with_capacity(tri.len() / 9);
    let mut c = Vec::with_capacity(tri.len() / 9);
    for t in tri.chunks_exact(9) {
        let v = [t[0] as f64, t[1] as f64, t[2] as f64];
        a.push(v);
        b.push(std::array::from_fn(|i| t[3 + i] as f64 - v[i]));
        c.push(std::array::from_fn(|i| t[6 + i] as f64 - v[i]));
    }
    let mut segments = Vec::new();
    let mut curves: Vec<Vec<[f64; 6]>> = Vec::new();
    let mut named = HashMap::new();
    for (i, e) in edges.chunks_exact(2).enumerate() {
        if !keep[i] {
            continue;
        }
        if e.iter().any(|&v| v < 0 || v as usize >= pos.len() / 3) {
            return Err(bad());
        }
        let aa = std::array::from_fn(|k| pos[e[0] as usize * 3 + k]);
        let bb = std::array::from_fn(|k| pos[e[1] as usize * 3 + k]);
        segments.push([aa, bb]);
        if curve_ids[i] >= 0 {
            let code = *named.entry(curve_ids[i]).or_insert_with(|| {
                curves.push(Vec::new());
                curves.len() - 1
            });
            curves[code].push([aa[0], aa[1], aa[2], bb[0], bb[1], bb[2]]);
        }
    }
    Ok((a, b, c, segments, curves))
}

/// The next bake of `old`'s mesh as a snap prototype (`build_snap_index`'s
/// input for one mesh): `old`'s trees carried over when an edit left most
/// of it as it was (`instances::Proto::patched`), a fresh build otherwise.
/// Its queries answer as a fresh build's would.
#[pyfunction]
pub fn patch_snap_proto(py: Python<'_>, old: PyRef<'_, InstanceProto>, input: Input<'_>) -> PyResult<InstanceProto> {
    let (tri, pos, edges, keep, curves) = &input;
    if tri.shape()[1] != 3
        || tri.shape()[0] % 3 != 0
        || pos.shape()[1] != 3
        || edges.shape()[1] != 2
        || keep.len() != edges.shape()[0]
        || curves.len() != edges.shape()[0]
    {
        return Err(pyo3::exceptions::PyValueError::new_err("snap array shape or index"));
    }
    let data = (tri.as_slice()?, pos.as_slice()?, edges.as_slice()?, keep.as_slice()?, curves.as_slice()?);
    let previous = old.inner.clone();
    drop(old);
    py.detach(|| {
        let (a, b, c, segments, curves) = prepared(data)?;
        let inner = previous.patched(a, b, c, segments);
        Ok(InstanceProto { inner: std::sync::Arc::new(inner), curves })
    })
}
