// SPDX-License-Identifier: GPL-3.0-or-later
//! Explode joins only newly copied faces that overlap an existing coplanar face.
//! The history step handles lifted children, selection and scene notifications.
use super::{plane, store_hypot};
use crate::{
    math::Matrix4,
    polygon::raw_newell,
    store::{keys::Kind, py::PyMeshStore, Store},
};
use glam::{DMat4, DVec3};
use pyo3::prelude::*;
use std::collections::HashMap;

const TOL: f64 = 1e-4;
const PARALLEL: f64 = 1. - 1e-6;

pub(crate) struct Exploded {
    pub(crate) added: Vec<u128>,
    pub(crate) copies: Py<PyAny>,
    pub(crate) planes_rebuilt: usize,
    pub(crate) dirty: usize,
}

struct Plane {
    origin: DVec3,
    normal: DVec3,
    old: Vec<u128>,
    new: Vec<u128>,
}

fn outer(store: &Store, f: u32) -> Vec<DVec3> {
    store
        .outer_ring(f)
        .into_iter()
        .map(|v| DVec3::from_array(*store.state.vpos.get(v as usize)))
        .collect()
}

fn unit(points: &[DVec3]) -> Option<DVec3> {
    let n = raw_newell(points);
    let length = n.length();
    (length > 1e-9).then(|| n / length)
}

fn overlapping_planes(
    store: &Store,
    added: &[u128],
    h: crate::store::geometry::Hypot,
) -> Vec<Plane> {
    let added: Vec<_> = added
        .iter()
        .filter_map(|k| store.slot(Kind::Face, *k))
        .collect();
    let (candidates, pairs) = store.plane_merge_candidates(&added, TOL, h);
    let mut planes = Vec::<Plane>::new();
    let mut geometry = HashMap::new();
    for (old, hits) in pairs {
        let points = outer(store, old);
        let Some(old_normal) = unit(&points) else {
            continue;
        };
        let old_key = store.key(Kind::Face, old);
        for j in hits {
            let new = candidates[j as usize];
            let candidate = geometry.entry(new).or_insert_with(|| {
                let points = outer(store, new);
                unit(&points).map(|normal| (points[0], normal))
            });
            let Some((o, n)) = *candidate else {
                continue;
            };
            if old_normal.dot(n).abs() < PARALLEL
                || points.iter().any(|&p| (p - o).dot(n).abs() > TOL)
            {
                continue;
            }
            let i = planes
                .iter()
                .position(|p| {
                    p.normal.dot(n).abs() > PARALLEL && (o - p.origin).dot(p.normal).abs() < TOL
                })
                .unwrap_or_else(|| {
                    planes.push(Plane {
                        origin: o,
                        normal: n,
                        old: Vec::new(),
                        new: Vec::new(),
                    });
                    planes.len() - 1
                });
            let group = &mut planes[i];
            if group.old.last() != Some(&old_key) {
                group.old.push(old_key);
            }
            let key = store.key(Kind::Face, new);
            if !group.new.contains(&key) {
                group.new.push(key);
            }
        }
    }
    planes
}

pub(crate) fn command_explode_geometry(
    mesh: &Bound<'_, PyAny>,
    group_mesh: &Bound<'_, PyAny>,
    transform: Option<[f64; 16]>,
    paint: Option<&Bound<'_, PyAny>>,
) -> PyResult<Option<Exploded>> {
    let hypot = store_hypot(mesh.py())?;
    let h = crate::hypot_of(&hypot)?;
    let start: usize = mesh.getattr("_mut_serial")?.extract()?;
    let matrix = transform.map(|v| DMat4::from_cols_array(&v));
    let inverse = matrix
        .filter(|m| m.determinant() != 0.)
        .map(|m| Matrix4::from(m.inverse()));
    let (added, mut copies) = super::transfer::paste(mesh, group_mesh, matrix.map(Matrix4::from), inverse, paint)?;
    mesh.call_method0("resplit_curves")?;
    let store = mesh.getattr("_store")?;
    let planes = overlapping_planes(&PyMeshStore::read(&store)?.inner, &added, h);
    let planes_rebuilt = planes.len();
    for group in planes {
        let mut keys = group.old;
        keys.extend(group.new);
        plane::rebuild_face_keys(
            mesh,
            &store,
            group.origin.to_array(),
            group.normal.normalize().to_array(),
            &keys,
        )?;
    }
    if planes_rebuilt != 0 {
        copies = copies.call_method1("only_live", (&store,))?;
    }
    let end: usize = mesh.getattr("_mut_serial")?.extract()?;
    Ok(Some(Exploded {
        added,
        copies: copies.unbind(),
        planes_rebuilt,
        dirty: end - start,
    }))
}
