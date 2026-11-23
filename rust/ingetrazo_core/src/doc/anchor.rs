// SPDX-License-Identifier: GPL-3.0-or-later
//! Affine coordinates of a picked point on a keyed vertex, edge or face.
use glam::DVec3;
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

    fn position(&self, vertices: Vec<(u128, [f64; 3])>) -> Option<[f64; 3]> {
        let mut point = DVec3::ZERO;
        for (key, weight) in &self.weights {
            let (_, xyz) = vertices.iter().find(|(k, _)| k == key)?;
            point += DVec3::from(*xyz) * *weight;
        }
        Some(point.to_array())
    }

    fn __deepcopy__(slf: PyRef<'_, Self>, _memo: &Bound<'_, PyAny>) -> Py<Self> {
        slf.into()
    }
}
