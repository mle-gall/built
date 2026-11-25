// SPDX-License-Identifier: GPL-3.0-or-later
//! Group overlapping outer rings before applying one rebuild per plane.
use crate::{math::Vec3, polygon::raw_newell};
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet},
};
use std::collections::HashSet;

struct Ring {
    key: u128,
    points: Vec<DVec3>,
    normal: DVec3,
    lo: DVec3,
    hi: DVec3,
}
impl Ring {
    fn read(store: &Bound<'_, PyAny>, key: u128) -> PyResult<Option<Self>> {
        let points: Vec<[f64; 3]> = store
            .call_method1("face_positions", (key,))?
            .get_item(0)?
            .extract()?;
        let points: Vec<_> = points.into_iter().map(DVec3::from_array).collect();
        let n = raw_newell(&points);
        let length = n.length();
        if length <= 1e-9 {
            return Ok(None);
        }
        let (lo, hi) = points.iter().fold(
            (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
            |(lo, hi), p| (lo.min(*p), hi.max(*p)),
        );
        Ok(Some(Self {
            key,
            points,
            normal: n / length,
            lo,
            hi,
        }))
    }
    fn overlaps(&self, other: &Self, tol: f64) -> bool {
        (0..3).all(|i| {
            (self.lo[i] < other.hi[i] - tol && other.lo[i] < self.hi[i] - tol)
                || ((self.hi[i] - self.lo[i]).abs() < tol
                    && (other.hi[i] - other.lo[i]).abs() < tol)
        })
    }
}

struct Plane {
    origin: DVec3,
    normal: DVec3,
    old: Vec<u128>,
    new: Vec<u128>,
}

pub(super) fn merge<'py>(
    scene: &Bound<'py, PyAny>,
    mesh: &Bound<'py, PyAny>,
    batch: &[Bound<'py, PyAny>],
    pending: &HashSet<u128>,
    checkpoint: impl FnOnce() -> PyResult<()>,
) -> PyResult<()> {
    let py = scene.py();
    let st = mesh.getattr("_store")?;
    let api = py.import("core.history")?;
    let class = api.getattr("RebuildPlaneFacesCommand")?;
    let tol = class.getattr("_TOL")?.extract::<f64>()?;
    let mut added = Vec::new();
    let mut ids = HashSet::new();
    for face in batch.iter().filter(|f| !f.is_none()) {
        let key = face.getattr("key")?.extract::<u128>()?;
        if st.call_method1("is_live", (key,))?.is_truthy()? {
            ids.insert(key);
            if let Some(row) = Ring::read(&st, key)? {
                added.push(row);
            }
        }
    }
    if added.is_empty() {
        return Ok(());
    }
    let mut planes: Vec<Plane> = Vec::new();
    let (lo, hi) = added.iter().fold(
        (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
        |(lo, hi), r| (lo.min(r.lo), hi.max(r.hi)),
    );
    for key in st.call_method1("keys", ("face",))?.extract::<Vec<u128>>()? {
        if ids.contains(&key) || pending.contains(&key) {
            continue;
        }
        let Some(old) = Ring::read(&st, key)? else {
            continue;
        };
        if (0..3).any(|i| old.lo[i] > hi[i] + tol || lo[i] > old.hi[i] + tol) {
            continue;
        }
        for new in &added {
            let origin = new.points[0];
            if !old.overlaps(new, tol)
                || old.normal.dot(new.normal).abs() < 1. - 1e-6
                || old
                    .points
                    .iter()
                    .any(|p| (*p - origin).dot(new.normal).abs() > tol)
            {
                continue;
            }
            let plane = if let Some(i) = planes.iter().position(|p| {
                p.normal.dot(new.normal).abs() > 1. - 1e-6
                    && (origin - p.origin).dot(p.normal).abs() < tol
            }) {
                &mut planes[i]
            } else {
                planes.push(Plane {
                    origin,
                    normal: new.normal,
                    old: Vec::new(),
                    new: Vec::new(),
                });
                planes.last_mut().unwrap()
            };
            if plane.old.last() != Some(&key) {
                plane.old.push(key);
            }
            if !plane.new.contains(&new.key) {
                plane.new.push(new.key);
            }
        }
    }
    if planes.is_empty() {
        return Ok(());
    }
    checkpoint()?;
    let keep = py
        .import("core.element_set")?
        .call_method1("ElementSet", (scene.getattr("selection")?,))?;
    let view = py.import("core.store_mesh")?.getattr("FaceView")?;
    for plane in planes {
        let faces = PyList::empty(py);
        for key in plane.old.into_iter().chain(plane.new) {
            faces.append(view.call1((mesh, key))?)?;
        }
        let kwargs = PyDict::new(py);
        kwargs.set_item("faces", faces)?;
        let cmd = class.call(
            (Vec3::from(plane.origin), Vec3::from(plane.normal)),
            Some(&kwargs),
        )?;
        cmd.setattr("take_snapshot", false)?;
        cmd.call_method1("do", (scene,))?;
    }
    scene.setattr("selection", py.get_type::<PySet>().call1((keep,))?)?;
    crate::doc::history_support::drop_dead_elements(scene, mesh)?;
    Ok(())
}
