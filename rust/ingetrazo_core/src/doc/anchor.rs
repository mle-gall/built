// SPDX-License-Identifier: GPL-3.0-or-later
//! Affine coordinates of a picked point on a keyed vertex, edge or face.
use crate::math::Matrix4;
use glam::{DMat4, DVec3};
use pyo3::prelude::*;

#[pyclass(module = "ingetrazo_core", frozen)]
pub struct ElementAnchor {
    #[pyo3(get)]
    kind: String,
    #[pyo3(get)]
    key: u128,
    #[pyo3(get)]
    path: Vec<String>,
    weights: Vec<(u128, f64)>,
}

#[pymethods]
impl ElementAnchor {
    #[staticmethod]
    fn capture(
        kind: &str,
        key: u128,
        path: Vec<String>,
        vertices: Vec<(u128, [f64; 3])>,
        point: [f64; 3],
    ) -> Option<Self> {
        let origin = DVec3::from(vertices.first()?.1);
        let delta = DVec3::from(point) - origin;
        let weights = match kind {
            "vertex" => vec![(vertices[0].0, 1.0)],
            "edge" => {
                let axis = DVec3::from(vertices.get(1)?.1) - origin;
                let t = delta.dot(axis) / axis.length_squared();
                vec![(vertices[0].0, 1.0 - t), (vertices[1].0, t)]
            }
            "face" => {
                // Choose the best conditioned triangle in a fan about the
                // first vertex. Affine coordinates also represent points
                // outside that triangle (including concave polygon faces).
                let (tri, normal) = vertices
                    .get(1..)?
                    .windows(2)
                    .map(|tri| {
                        let u = DVec3::from(tri[0].1) - origin;
                        let v = DVec3::from(tri[1].1) - origin;
                        (tri, u.cross(v))
                    })
                    .max_by(|a, b| a.1.length_squared().total_cmp(&b.1.length_squared()))?;
                let u = DVec3::from(tri[0].1) - origin;
                let v = DVec3::from(tri[1].1) - origin;
                let a = delta.cross(v).dot(normal) / normal.length_squared();
                let b = u.cross(delta).dot(normal) / normal.length_squared();
                vec![(vertices[0].0, 1.0 - a - b), (tri[0].0, a), (tri[1].0, b)]
            }
            _ => return None,
        };
        weights.iter().all(|(_, w)| w.is_finite()).then(|| Self {
            kind: kind.into(),
            key,
            path,
            weights,
        })
    }

    /// Archives recreate element keys on open. Encode the existing weights
    /// by vertex index so reading and saving never refits a picked point.
    fn indexed_weights(&self, keys: Vec<u128>) -> Option<Vec<(usize, f64)>> {
        self.weights
            .iter()
            .map(|(key, weight)| Some((keys.iter().position(|k| k == key)?, *weight)))
            .collect()
    }

    #[staticmethod]
    fn restore(
        kind: &str,
        key: u128,
        path: Vec<String>,
        keys: Vec<u128>,
        weights: Vec<(usize, f64)>,
    ) -> Option<Self> {
        let count = match kind {
            "vertex" => 1,
            "edge" => 2,
            "face" => 3,
            _ => return None,
        };
        if weights.len() != count || weights.iter().any(|(_, w)| !w.is_finite()) {
            return None;
        }
        let restored = weights
            .iter()
            .map(|(i, w)| Some((*keys.get(*i)?, *w)))
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            kind: kind.into(),
            key,
            path,
            weights: restored,
        })
    }

    fn position(&self, vertices: Vec<(u128, [f64; 3])>) -> Option<[f64; 3]> {
        let mut point = DVec3::ZERO;
        for (key, weight) in &self.weights {
            let (_, xyz) = vertices.iter().find(|(k, _)| k == key)?;
            point += DVec3::from(*xyz) * *weight;
        }
        Some(point.to_array())
    }

    /// A sign is expressed in its parent's frame. A face supplies a stable
    /// tangent and normal as well as a point. A vertex or edge pins its point.
    fn placement_frame(
        &self,
        vertices: Vec<(u128, [f64; 3])>,
        transform: [f64; 16],
    ) -> Option<Vec<f64>> {
        let space = DMat4::from_cols_array(&transform);
        let point = space.transform_point3(DVec3::from(self.position(vertices.clone())?));
        let mut frame = DMat4::from_translation(point);
        if self.kind == "face" {
            let xyz = |i: usize| {
                let key = self.weights.get(i)?.0;
                let v = vertices.iter().find(|(k, _)| *k == key)?;
                Some(space.transform_point3(DVec3::from(v.1)))
            };
            let a = xyz(0)?;
            let x = (xyz(1)? - a).try_normalize()?;
            let z = x.cross(xyz(2)? - a).try_normalize()?;
            frame.x_axis = x.extend(0.);
            frame.y_axis = z.cross(x).extend(0.);
            frame.z_axis = z.extend(0.);
        }
        Some(frame.to_cols_array().to_vec())
    }

    #[staticmethod]
    fn follow_placement(
        previous: [f64; 16],
        current: [f64; 16],
        pose: [f64; 16],
    ) -> Option<Matrix4> {
        let previous = DMat4::from_cols_array(&previous);
        let current = DMat4::from_cols_array(&current);
        if current.abs_diff_eq(previous, 1e-10) {
            return None;
        }
        Some((current * previous.inverse() * DMat4::from_cols_array(&pose)).into())
    }

    fn __deepcopy__(slf: PyRef<'_, Self>, _memo: &Bound<'_, PyAny>) -> Py<Self> {
        slf.into()
    }
}
