// SPDX-License-Identifier: GPL-3.0-or-later
//! The interval over which the topology of a staged extrusion stays valid.
//! Each moving vertex advances on the intersection of its fixed wall planes.
use crate::{
    math::xyz,
    store::{
        geometry::Hypot,
        keys::Kind,
        py::{PyMeshStore, PyStoreSnapshot},
        Store,
    },
};
use glam::DVec3;
use pyo3::prelude::*;
use std::collections::{HashMap, HashSet};

fn slide_rate(normals: &[DVec3], n: DVec3) -> Option<DVec3> {
    let nn = n.normalize_or_zero();
    let mut planes: Vec<DVec3> = vec![];
    for &m in normals {
        if planes.iter().all(|o| m.cross(*o).length() > 1e-3) {
            planes.push(m);
        }
    }
    if planes.iter().all(|m| m.dot(nn).abs() < 1e-3) {
        return Some(n);
    }
    let m = planes[0];
    let line = if planes.len() == 1 {
        nn - m * nn.dot(m)
    } else {
        m.cross(planes[1])
    };
    let along = line.dot(nn);
    if line.length() < 1e-9 || along.abs() < 1e-3 * line.length() {
        return None;
    }
    let rate = line * (n.dot(nn) / along);
    planes
        .iter()
        .all(|m| m.dot(rate).abs() <= 1e-6 * rate.length())
        .then_some(rate)
}
struct Mover {
    key: u128,
    point: DVec3,
    rate: DVec3,
}

#[pyclass(module = "ingetrazo_core")]
pub struct PushSlide {
    movers: Vec<Mover>,
    #[pyo3(get)]
    interval: (f64, f64),
    #[pyo3(get)]
    faces: Vec<u128>,
    #[pyo3(get)]
    edges: Vec<u128>,
}
#[pymethods]
impl PushSlide {
    #[staticmethod]
    #[allow(clippy::too_many_arguments)]
    fn resolve(
        store: &Bound<'_, PyAny>,
        before: PyRef<'_, PyStoreSnapshot>,
        anchor: &Bound<'_, PyAny>,
        normal: &Bound<'_, PyAny>,
        distance: f64,
        mut levels: Vec<f64>,
        cap: &Bound<'_, PyAny>,
        tolerance: f64,
    ) -> PyResult<Option<Self>> {
        let st = PyMeshStore::read(store)?;
        let s = &st.inner;
        let (anchor, n) = (xyz(anchor)?, xyz(normal)?);
        let pos = |v: u32| DVec3::from(*s.state.vpos.get(v as usize));
        let height = |p: DVec3| (p - anchor).dot(n);
        let made = |k| s.new_slots_since(&before.inner, k);
        let created = made(Kind::Vertex);
        let moving: Vec<_> = created
            .iter()
            .copied()
            .filter(|&v| (height(pos(v)) - distance).abs() < tolerance)
            .collect();
        let moving_set: HashSet<_> = moving.iter().copied().collect();
        let mut walls: HashMap<u32, Vec<DVec3>> = HashMap::new();
        for f in s.faces_around(&moving) {
            let vertices: Vec<_> = s.face_rings(f).into_iter().flatten().collect();
            if vertices.iter().all(|v| moving_set.contains(v)) {
                continue;
            }
            let normal: DVec3 = <[f64; 3]>::from(s.face_normal(f, Hypot::LibStdCxx)).into();
            for v in vertices {
                if moving_set.contains(&v) {
                    walls.entry(v).or_default().push(normal);
                } else {
                    levels.push(height(pos(v)));
                }
            }
        }
        let mut result = Self {
            movers: vec![],
            interval: (distance, distance),
            faces: vec![],
            edges: vec![],
        };
        if !levels.iter().any(|&d| (d - distance).abs() <= tolerance) {
            if moving.is_empty() {
                let mut spread = 0_f64;
                for p in cap.try_iter()? {
                    spread = spread.max(height(xyz(&p?)?).abs());
                }
                if created
                    .iter()
                    .any(|&v| (height(pos(v)) - distance).abs() <= spread + tolerance)
                {
                    return Ok(None);
                }
            }
            for v in moving {
                let Some(rate) =
                    slide_rate(walls.get(&v).map(Vec::as_slice).unwrap_or_default(), n)
                else {
                    return Ok(None);
                };
                result.movers.push(Mover {
                    key: s.key(Kind::Vertex, v),
                    point: pos(v),
                    rate,
                });
            }
            result.interval = (
                levels
                    .iter()
                    .copied()
                    .filter(|&d| d < distance)
                    .fold(f64::NEG_INFINITY, f64::max),
                levels
                    .iter()
                    .copied()
                    .filter(|&d| d > distance)
                    .fold(f64::INFINITY, f64::min),
            );
        }
        let keys = |k| made(k).into_iter().map(|i| s.key(k, i)).collect();
        result.faces = keys(Kind::Face);
        result.edges = keys(Kind::Edge);
        Ok(Some(result))
    }
    #[getter]
    fn vertices(&self) -> Vec<u128> {
        self.movers.iter().map(|m| m.key).collect()
    }

    fn apply(&self, mesh: &Bound<'_, PyAny>, offset: f64) -> PyResult<()> {
        let store = mesh.getattr("_store")?;
        let positions: Vec<_> = self
            .movers
            .iter()
            .map(|m| (m.key, (m.point + m.rate * offset).to_array()))
            .collect();
        PyMeshStore::place_tool_batch(&store, positions)?;
        mesh.call_method0("_bump")?;
        Ok(())
    }
}

impl Store {
    pub(crate) fn new_slots_since(&self, before: &crate::store::Snapshot, kind: Kind) -> Vec<u32> {
        let old = &before.state.slots[kind as usize];
        let current = &self.state.slots[kind as usize];
        self.live_slots(kind)
            .into_iter()
            .filter(|&v| {
                before.state.ns != self.state.ns || !old.alive(v) || old.gen(v) != current.gen(v)
            })
            .collect()
    }
}
