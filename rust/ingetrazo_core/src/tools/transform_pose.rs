// SPDX-License-Identifier: GPL-3.0-or-later
//! Immutable grab-time pose for annotations and non-resident geometry.
//! A preview frame always starts at this pose, including after copy toggles.
use crate::{
    math::{mat, xyz, Matrix4, Vec3},
    store::py::PyMeshStore,
};
use glam::{DMat4, DVec3};
use pyo3::{prelude::*, types::PyDict};

enum Field {
    Point,
    Direction,
    Normal,
    Offset,
    Slide(DVec3),
}
struct Record {
    item: Py<PyAny>,
    attr: &'static str,
    value: DVec3,
    field: Field,
}
struct VertexBatch {
    mesh: Py<PyAny>,
    vertices: Vec<(Py<PyAny>, DVec3)>,
}
#[pyclass(module = "ingetrazo_core")]
#[derive(Default)]
pub struct TransformPose {
    meshes: Vec<VertexBatch>,
    groups: Vec<(Py<PyAny>, DMat4)>,
    records: Vec<Record>,
    section_only: bool,
    geometry: bool,
    resident: bool,
    mode: String,
}
impl TransformPose {
    fn record(
        &mut self,
        item: &Bound<'_, PyAny>,
        attr: &'static str,
        field: Field,
    ) -> PyResult<()> {
        let value = item.getattr(attr)?;
        if !value.is_none() {
            self.records.push(Record {
                item: item.clone().unbind(),
                attr,
                value: xyz(&value)?,
                field,
            });
        }
        Ok(())
    }
    fn vertices(&mut self, mesh: Bound<'_, PyAny>, verts: &Bound<'_, PyAny>) -> PyResult<()> {
        let vertices = verts
            .try_iter()?
            .map(|v| {
                let v = v?;
                let p = xyz(&v.getattr("position")?)?;
                Ok((v.unbind(), p))
            })
            .collect::<PyResult<Vec<_>>>()?;
        if !vertices.is_empty() {
            self.meshes.push(VertexBatch {
                mesh: mesh.unbind(),
                vertices,
            });
        }
        Ok(())
    }
}
#[pymethods]
impl TransformPose {
    /// Select a resident row preview when the grabbed topology covers the
    /// whole loose row. Scripted hosts may only provide the group API.
    #[staticmethod]
    pub(super) fn begin(view: &Bound<'_, PyAny>, tool: &Bound<'_, PyAny>) -> PyResult<bool> {
        let groups = tool.getattr("_groups")?;
        let vertices = tool.getattr("_verts")?;
        if vertices.is_truthy()? {
            if let Some(begin) = view.getattr_opt("begin_loose_preview")? {
                return begin.call1((groups, vertices))?.is_truthy();
            }
        } else if groups.is_truthy()? {
            if let Some(begin) = view.getattr_opt("begin_groups_preview")? {
                begin.call1((groups,))?;
                return Ok(true);
            }
        }
        Ok(false)
    }
    #[new]
    pub(super) fn new(
        tool: &Bound<'_, PyAny>,
        scene: &Bound<'_, PyAny>,
        mode: &str,
        resident: bool,
    ) -> PyResult<Self> {
        let mut pose = Self {
            resident,
            mode: mode.into(),
            ..Self::default()
        };
        if !resident {
            for g in tool.getattr("_groups")?.try_iter()? {
                let g = g?;
                if let Some(x) = g.getattr_opt("xform")?.filter(|x| !x.is_none()) {
                    pose.groups.push((g.unbind(), mat(&x)?));
                } else {
                    let mesh = g.getattr("mesh")?;
                    let v = mesh.getattr("vertices")?;
                    pose.vertices(mesh, &v)?;
                }
            }
            let verts = tool.getattr("_verts")?;
            if verts.is_truthy()? {
                pose.vertices(scene.getattr("mesh")?, &verts)?;
            }
        }
        pose.geometry = !pose.meshes.is_empty() || !pose.groups.is_empty();
        for im in tool.getattr("_images")?.try_iter()? {
            let im = im?;
            pose.record(&im, "origin", Field::Point)?;
            if mode != "move" {
                pose.record(&im, "u", Field::Direction)?;
                pose.record(&im, "v", Field::Direction)?;
            }
        }
        if mode != "scale" {
            for g in tool.getattr("_moved_guides")?.try_iter()? {
                let g = g?;
                pose.record(&g, "point", Field::Point)?;
                pose.record(&g, "origin", Field::Point)?;
                if mode == "rotate" {
                    pose.record(&g, "direction", Field::Normal)?;
                }
            }
            if mode == "move" {
                for l in tool.getattr("_labels")?.try_iter()? {
                    pose.record(&l?, "offset", Field::Offset)?
                }
                for (dim, mode) in tool.getattr("_dims")?.cast::<PyDict>()?.iter() {
                    if mode.extract::<String>()? == "line" {
                        let d = xyz(&dim.getattr("b")?)? - xyz(&dim.getattr("a")?)?;
                        pose.record(
                            &dim,
                            "offset",
                            Field::Slide(if d.length() > 1e-9 {
                                d.normalize()
                            } else {
                                DVec3::ZERO
                            }),
                        )?;
                    } else {
                        for (a, b, c) in [
                            ("a", "anchor_a", "attachment_a"),
                            ("b", "anchor_b", "attachment_b"),
                        ] {
                            if dim.getattr(b)?.is_none() && dim.getattr(c)?.is_none() {
                                pose.record(&dim, a, Field::Point)?
                            }
                        }
                    }
                }
            }
            let planes = tool.getattr("_splanes")?;
            pose.section_only = !pose.geometry && pose.records.is_empty() && planes.is_truthy()?;
            for plane in planes.try_iter()? {
                let plane = plane?;
                pose.record(&plane, "point", Field::Point)?;
                if mode == "rotate" {
                    pose.record(&plane, "normal", Field::Normal)?;
                }
            }
        }
        Ok(pose)
    }
    #[pyo3(signature=(scene,matrix=None))]
    fn apply(&self, scene: &Bound<'_, PyAny>, matrix: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        self.pose(scene, matrix.map(mat).transpose()?)
    }
    fn translate(&self, scene: &Bound<'_, PyAny>, delta: &Bound<'_, PyAny>) -> PyResult<()> {
        let d = xyz(delta)?;
        self.pose(
            scene,
            (d != DVec3::ZERO).then(|| DMat4::from_translation(d)),
        )
    }
}
impl TransformPose {
    pub(super) fn pose(&self, scene: &Bound<'_, PyAny>, matrix: Option<DMat4>) -> PyResult<()> {
        let py = scene.py();
        let point = |p: DVec3| matrix.map_or(p, |m| m.transform_point3(p));
        for (g, x) in &self.groups {
            g.bind(py)
                .setattr("xform", Matrix4::from(matrix.map_or(*x, |m| m * *x)))?
        }
        for batch in &self.meshes {
            let mesh = batch.mesh.bind(py);
            let store = mesh
                .getattr("__dict__")?
                .cast::<PyDict>()?
                .get_item("_store")?
                .filter(|s| !s.is_none());
            if let Some(s) = store.filter(|s| s.is_instance_of::<PyMeshStore>()) {
                let targets = batch
                    .vertices
                    .iter()
                    .map(|(v, p)| {
                        Ok((
                            v.bind(py).getattr("key")?.extract::<u128>()?,
                            point(*p).to_array(),
                        ))
                    })
                    .collect::<PyResult<Vec<_>>>()?;
                PyMeshStore::place_tool_batch(&s, targets)?;
                mesh.call_method0("_bump")?;
            } else {
                for (v, p) in &batch.vertices {
                    let v = v.bind(py);
                    mesh.call_method1("place_vertex", (v, Vec3::from(point(*p))))?;
                }
            }
        }
        for record in &self.records {
            let p = record.value;
            let value = matrix.map_or(p, |m| match record.field {
                Field::Point => m.transform_point3(p),
                Field::Direction => m.transform_vector3(p),
                Field::Normal => {
                    let n = m.transform_vector3(p);
                    if n.length() > 1e-12 {
                        n.normalize()
                    } else {
                        p
                    }
                }
                Field::Offset => p + m.w_axis.truncate(),
                Field::Slide(dir) => {
                    let d = m.w_axis.truncate();
                    p + d - dir * d.dot(dir)
                }
            });
            record
                .item
                .bind(py)
                .setattr(record.attr, Vec3::from(value))?;
        }
        if !self.resident {
            if self.mode == "move" && self.section_only {
                scene.call_method0("bump_sections")?;
            } else if self.mode != "rotate" || self.geometry {
                let version = scene.getattr("version")?.extract::<u64>()?;
                scene.setattr("version", version + 1)?;
            }
        }
        Ok(())
    }
}

/// Absolute matrix previews for Scale and Rotate. Scale captures the original
/// on grab. Rotate defers it until the reference arm is set, including when
/// Move hands its box grip to Rotate. Copy parks the row without releasing it.
#[pyclass(module = "ingetrazo_core")]
pub struct MatrixGesture {
    original: Option<TransformPose>,
    resident: bool,
    factors: [f64; 3],
    #[pyo3(get)]
    angle: f64,
}
#[pymethods]
impl MatrixGesture {
    #[new]
    fn new() -> Self {
        Self {
            original: None,
            resident: false,
            factors: [1.; 3],
            angle: 0.,
        }
    }
    #[getter]
    fn factors(&self) -> (f64, f64, f64) {
        self.factors.into()
    }

    fn begin(
        &mut self,
        tool: &Bound<'_, PyAny>,
        view: &Bound<'_, PyAny>,
        scale: bool,
    ) -> PyResult<()> {
        *self = Self::new();
        self.resident =
            (!scale || !tool.getattr("_images")?.is_truthy()?) && TransformPose::begin(view, tool)?;
        if scale && !self.resident {
            self.original = Some(TransformPose::new(
                tool,
                &view.getattr("scene")?,
                "scale",
                false,
            )?);
        }
        Ok(())
    }
    fn scale(
        &mut self,
        tool: &Bound<'_, PyAny>,
        view: &Bound<'_, PyAny>,
        factors: [f64; 3],
    ) -> PyResult<()> {
        if factors
            .iter()
            .zip(self.factors)
            .all(|(a, b)| (a - b).abs() < 1e-12)
        {
            return Ok(());
        }
        let matrix = view.py().import("ingetrazo_core")?.call_method1(
            "scale_matrix",
            (tool.getattr("_anchor")?, factors, tool.getattr("_axes")?),
        )?;
        if self.resident {
            view.call_method1("set_groups_preview_matrix", (matrix,))?;
        } else if let Some(original) = &self.original {
            let identity = factors.iter().all(|f| (f - 1.).abs() < 1e-12);
            original.apply(&view.getattr("scene")?, (!identity).then_some(&matrix))?;
        }
        self.factors = factors;
        Ok(())
    }
    fn rotate(
        &mut self,
        tool: &Bound<'_, PyAny>,
        view: &Bound<'_, PyAny>,
        angle: f64,
    ) -> PyResult<()> {
        let scene = view.getattr("scene")?;
        let matrix = view.py().import("ingetrazo_core")?.call_method1(
            "rotation_matrix",
            (
                tool.getattr("start_point")?,
                tool.call_method0("_axis")?,
                angle,
            ),
        )?;
        if self.original.is_none() {
            self.original = Some(TransformPose::new(tool, &scene, "rotate", self.resident)?);
        }
        self.original
            .as_ref()
            .unwrap()
            .apply(&scene, Some(&matrix))?;
        if self.resident {
            view.call_method1("set_groups_preview_matrix", (matrix,))?;
        }
        self.angle = angle;
        Ok(())
    }
    fn revert(&mut self, view: &Bound<'_, PyAny>, park: bool) -> PyResult<()> {
        if let Some(original) = self.original.take() {
            if original.mode != "scale" || self.factors.iter().any(|f| (f - 1.).abs() > 1e-12) {
                original.pose(&view.getattr("scene")?, None)?;
            }
        }
        if self.resident {
            if park {
                view.call_method1(
                    "set_groups_preview_matrix",
                    (Matrix4::from(DMat4::IDENTITY),),
                )?;
            } else {
                view.call_method0("end_groups_preview")?;
                self.resident = false;
            }
        }
        self.angle = 0.;
        self.factors = [1.; 3];
        Ok(())
    }
}
