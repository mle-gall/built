// SPDX-License-Identifier: GPL-3.0-or-later
//! Definition-local f64 supports. Generator references point only into the
//! curve table, making cycles unrepresentable.
pub(crate) mod bound;
pub(crate) mod file;
pub(crate) mod patch;
mod spline;
mod projected;
mod ruled_mesh;
use glam::{DQuat, DVec3, DVec4};
use serde::{Deserialize, Serialize};
pub(crate) use spline::Axis;
use std::f64::consts::FRAC_PI_2;

type Result<T> = std::result::Result<T, &'static str>;
fn check(value: bool, reason: &'static str) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(reason)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Frame {
    pub(crate) origin: DVec3,
    pub(crate) axis: DVec3,
    pub(crate) reference: DVec3,
}
impl Frame {
    pub(crate) fn from_array([origin, axis, reference]: [[f64; 3]; 3]) -> Self {
        Self {
            origin: origin.into(),
            axis: axis.into(),
            reference: reference.into(),
        }
    }
    pub(crate) fn array(&self) -> [[f64; 3]; 3] {
        [
            self.origin.to_array(),
            self.axis.to_array(),
            self.reference.to_array(),
        ]
    }
    pub(crate) fn mapped(&self, point: impl Fn(DVec3) -> DVec3) -> Result<Self> {
        let origin = point(self.origin);
        let x = point(self.origin + self.reference) - origin;
        let y = point(self.origin + self.y()) - origin;
        let out = Self {
            origin,
            axis: x
                .cross(y)
                .try_normalize()
                .ok_or("collapsed plane transform")?,
            reference: x.try_normalize().ok_or("collapsed reference direction")?,
        };
        out.validate()?;
        Ok(out)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        check(
            self.origin.is_finite()
                && (self.axis.length() - 1.).abs() < 1e-12
                && (self.reference.length() - 1.).abs() < 1e-12
                && self.reference.dot(self.axis).abs() < 1e-12,
            "invalid support frame",
        )
    }
    pub(crate) fn y(&self) -> DVec3 {
        self.axis.cross(self.reference)
    }
    fn radial(&self, t: f64) -> DVec3 {
        self.reference * t.cos() + self.y() * t.sin()
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Curve {
    Line {
        origin: DVec3,
        direction: DVec3,
    },
    Circle {
        frame: Frame,
        radius: f64,
    },
    Ellipse {
        frame: Frame,
        major: f64,
        minor: f64,
    },
    Spline {
        axis: Axis,
        controls: Vec<DVec4>,
    },
}
fn radius(r: f64) -> Result<()> {
    check(r > 0. && r.is_finite(), "invalid radius")
}
impl Curve {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Line { origin, direction } => check(
                origin.is_finite() && (direction.length() - 1.).abs() < 1e-12,
                "invalid line",
            ),
            Self::Circle { frame, radius: r } => {
                frame.validate()?;
                radius(*r)
            }
            Self::Ellipse {
                frame,
                major,
                minor,
            } => {
                frame.validate()?;
                radius(*major)?;
                radius(*minor)?;
                check(major >= minor, "invalid ellipse axes")
            }
            Self::Spline { axis, controls } => {
                axis.validate(controls.len())?;
                axis.validate_periodic(controls.len(), |i| controls[i])?;
                spline::controls(controls)
            }
        }
    }
    pub(crate) fn point(&self, t: f64) -> Result<DVec3> {
        check(t.is_finite(), "nonfinite curve parameter")?;
        let p = match self {
            Self::Line { origin, direction } => *origin + *direction * t,
            Self::Circle { frame, radius } => frame.origin + frame.radial(t) * *radius,
            Self::Ellipse {
                frame,
                major,
                minor,
            } => frame.origin + frame.reference * (major * t.cos()) + frame.y() * (minor * t.sin()),
            Self::Spline { axis, controls } => {
                spline::rational(controls, axis.basis(t)?.into_iter())?
            }
        };
        check(p.is_finite(), "nonfinite curve point")?;
        Ok(p)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Surface {
    Plane {
        frame: Frame,
    },
    Cylinder {
        frame: Frame,
        radius: f64,
    },
    Cone {
        frame: Frame,
        radius: f64,
        half_angle: f64,
    },
    Sphere {
        frame: Frame,
        radius: f64,
    },
    Torus {
        frame: Frame,
        major: f64,
        minor: f64,
    },
    Spline {
        u: Axis,
        v: Axis,
        rows: usize,
        cols: usize,
        controls: Vec<DVec4>,
    },
    Extrusion {
        generator: u32,
        vector: DVec3,
    },
    Revolution {
        generator: u32,
        origin: DVec3,
        axis: DVec3,
    },
}

impl Surface {
    fn validate(&self, curves: &[Curve]) -> Result<()> {
        match self {
            Self::Plane { frame } => frame.validate(),
            Self::Sphere { frame, radius: r } | Self::Cylinder { frame, radius: r } => {
                frame.validate()?;
                radius(*r)
            }
            Self::Torus {
                frame,
                major,
                minor,
            } => {
                frame.validate()?;
                radius(*major)?;
                radius(*minor)
            }
            Self::Cone {
                frame,
                radius: r,
                half_angle: a,
            } => {
                frame.validate()?;
                check(
                    r.is_finite() && *r >= 0. && a.abs() > 0. && a.abs() < FRAC_PI_2,
                    "invalid cone",
                )
            }
            Self::Spline {
                u,
                v,
                rows,
                cols,
                controls,
            } => {
                check(
                    rows.checked_mul(*cols) == Some(controls.len()),
                    "invalid spline net dimensions",
                )?;
                u.validate(*rows)?;
                v.validate(*cols)?;
                if u.periodic {
                    for j in 0..*cols {
                        u.validate_periodic(*rows, |i| controls[i * cols + j])?;
                    }
                }
                if v.periodic {
                    for i in 0..*rows {
                        v.validate_periodic(*cols, |j| controls[i * cols + j])?;
                    }
                }
                spline::controls(controls)
            }
            Self::Extrusion { generator, vector } => check(
                (*generator as usize) < curves.len()
                    && vector.length().is_finite()
                    && vector.length() > 0.,
                "invalid extrusion",
            ),
            Self::Revolution {
                generator,
                origin,
                axis,
            } => check(
                (*generator as usize) < curves.len()
                    && origin.is_finite()
                    && (axis.length() - 1.).abs() < 1e-12,
                "invalid revolution",
            ),
        }
    }
    pub(crate) fn point(&self, curves: &[Curve], u: f64, v: f64) -> Result<DVec3> {
        check(
            u.is_finite() && v.is_finite(),
            "nonfinite surface parameter",
        )?;
        let p = match self {
            Self::Plane { frame } => frame.origin + frame.reference * u + frame.y() * v,
            Self::Cylinder { frame, radius } => {
                frame.origin + frame.radial(u) * *radius + frame.axis * v
            }
            Self::Cone {
                frame,
                radius,
                half_angle,
            } => {
                frame.origin
                    + frame.radial(u) * (radius + v * half_angle.sin())
                    + frame.axis * (v * half_angle.cos())
            }
            Self::Sphere { frame, radius } => {
                check(v.abs() <= FRAC_PI_2, "latitude outside sphere")?;
                frame.origin + (frame.radial(u) * v.cos() + frame.axis * v.sin()) * *radius
            }
            Self::Torus {
                frame,
                major,
                minor,
            } => {
                frame.origin
                    + frame.radial(u) * (major + minor * v.cos())
                    + frame.axis * (minor * v.sin())
            }
            Self::Extrusion { generator, vector } => {
                curves[*generator as usize].point(u)? + *vector * v
            }
            Self::Revolution {
                generator,
                origin,
                axis,
            } => {
                *origin
                    + DQuat::from_axis_angle(*axis, v)
                        * (curves[*generator as usize].point(u)? - *origin)
            }
            Self::Spline {
                u: a,
                v: b,
                controls,
                ..
            } => {
                let bu = a.basis(u)?;
                let bv = b.basis(v)?;
                spline::rational(
                    controls,
                    bu.into_iter().flat_map(|x| bv.iter().map(move |y| x * y)),
                )?
            }
        };
        check(p.is_finite(), "nonfinite surface point")?;
        Ok(p)
    }
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rows {
    #[serde(default)]
    pub(crate) topology: Option<patch::Topology>,
    pub(crate) curves: Vec<Curve>,
    pub(crate) surfaces: Vec<Surface>,
}

impl Rows {
    pub(crate) fn validate_pool(&self) -> Result<()> {
        for c in &self.curves {
            c.validate()?;
        }
        for s in &self.surfaces {
            s.validate(&self.curves)?;
        }
        Ok(())
    }
}

use pyo3::{exceptions::PyValueError, prelude::*};
/// Immutable geometry pool for native scripting; indices are local to this pool.
#[pyclass(frozen, module = "built_core.command_protocol")]
pub struct ExactGeometry {
    pub(crate) rows: Rows,
    mesh: std::sync::OnceLock<Result<patch::Mesh>>,
}
#[pymethods]
impl ExactGeometry {
    #[new]
    pub(crate) fn new(json: &str) -> PyResult<Self> {
        // Decode the explicit type tag before deserializing each payload.
        // Serde's internally tagged buffering cannot preserve arbitrary-
        // precision JSON numbers (enabled elsewhere in the document codec).
        let mut value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| PyValueError::new_err(e.to_string()))?;
        for table in ["curves", "surfaces"] {
            let items = value
                .get_mut(table)
                .and_then(|v| v.as_array_mut())
                .ok_or_else(|| PyValueError::new_err("missing geometry table"))?;
            for row in items {
                let tag = row
                    .as_object_mut()
                    .and_then(|o| o.remove("type"))
                    .and_then(|t| t.as_str().map(str::to_owned))
                    .ok_or_else(|| PyValueError::new_err("missing geometry type"))?;
                let payload = row.take();
                *row = serde_json::json!({tag:payload});
            }
        }
        let rows: Rows =
            serde_json::from_value(value).map_err(|e| PyValueError::new_err(e.to_string()))?;
        for c in &rows.curves {
            c.validate().map_err(PyValueError::new_err)?;
        }
        for s in &rows.surfaces {
            s.validate(&rows.curves).map_err(PyValueError::new_err)?;
        }
        if let Some(t) = &rows.topology {
            t.validate(&rows).map_err(PyValueError::new_err)?;
        }
        Ok(Self {
            rows,
            mesh: Default::default(),
        })
    }
    fn tessellate(&self) -> PyResult<String> {
        serde_json::to_string(self.model_mesh()?).map_err(|e| PyValueError::new_err(e.to_string()))
    }

    /// Insert the conforming fallback mesh into the active definition.
    /// The caller owns the enclosing SnapshotMutation.
    fn _materialize(&self, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        let model = self.model_mesh()?;
        let mesh = scene.getattr("mesh")?;
        let before = mesh.call_method0("capture_state")?;
        if let Err(error) = model.append_facets(&mesh) {
            mesh.call_method1("restore_state", (before,))?;
            return Err(error);
        }
        Ok(())
    }

    /// Ordered boundary uses. Seam branches, edge orientation and singular
    /// vertex bounds remain distinct even when they have identical endpoints.
    fn face_boundary(&self, face_id: u64) -> PyResult<String> {
        self.rows
            .topology
            .as_ref()
            .ok_or_else(|| PyValueError::new_err("missing exact topology"))?
            .boundary(face_id)
            .map_err(PyValueError::new_err)
    }

    fn curve_point(&self, index: u32, t: f64) -> PyResult<[f64; 3]> {
        let c = self
            .rows
            .curves
            .get(index as usize)
            .ok_or_else(|| PyValueError::new_err("unknown curve"))?;
        Ok(c.point(t).map_err(PyValueError::new_err)?.to_array())
    }
    fn surface_point(&self, index: u32, u: f64, v: f64) -> PyResult<[f64; 3]> {
        let s = self
            .rows
            .surfaces
            .get(index as usize)
            .ok_or_else(|| PyValueError::new_err("unknown surface"))?;
        Ok(s.point(&self.rows.curves, u, v)
            .map_err(PyValueError::new_err)?
            .to_array())
    }
}

impl ExactGeometry {
    fn model_mesh(&self) -> PyResult<&patch::Mesh> {
        let t = self
            .rows
            .topology
            .as_ref()
            .ok_or_else(|| PyValueError::new_err("missing exact topology"))?;
        self.mesh
            .get_or_init(|| t.mesh(&self.rows))
            .as_ref()
            .map_err(|e| PyValueError::new_err(*e))
    }
}

impl patch::Mesh {
    fn append_facets(&self, mesh: &Bound<'_, PyAny>) -> PyResult<()> {
        use numpy::{ndarray::Array2, IntoPyArray};
        let py = mesh.py();
        let points = Array2::from_shape_fn((self.points.len(), 3), |(i, j)| self.points[i][j])
            .into_pyarray(py);
        let welded = mesh.call_method1("bulk_weld", (points,))?;
        let indices: Vec<usize> = welded.get_item(1)?.extract()?;
        // Coincident uses already share the same model point index. Welding
        // any other pair can join disjoint shells, even when no individual
        // triangle has a repeated corner.
        let mut used = std::collections::HashSet::with_capacity(indices.len());
        if indices.iter().any(|&i| !used.insert(i)) {
            return Err(PyValueError::new_err(
                "store weld collapses distinct model samples",
            ));
        }
        let mut corners = Vec::<i64>::with_capacity(3 * self.triangles.len());
        for tri in &self.triangles {
            let [a, b, c] = tri.map(|i| indices[i]);
            corners.extend([a as i64, b as i64, c as i64]);
        }
        let n = self.triangles.len();
        mesh.call_method1(
            "add_faces_welded",
            (
                welded.get_item(0)?,
                corners.into_pyarray(py),
                vec![3i64; n].into_pyarray(py),
                vec![1i64; n].into_pyarray(py),
            ),
        )?;
        Ok(())
    }
}
pub(crate) use patch::validate_polygon;
