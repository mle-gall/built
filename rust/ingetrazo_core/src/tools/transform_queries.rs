// SPDX-License-Identifier: GPL-3.0-or-later
//! Transform target resolution and the immutable wire preview of a copy.
use super::view_queries::{classify, hit};
use crate::{
    math::{xyz, Vec3},
    store::{commands::cell, keys::Kind, py::PyMeshStore},
};
use glam::{DMat4, DVec3};
use pyo3::{prelude::*, types::PyDict};
use std::collections::HashSet;

fn pixel(ctx: &Bound<'_, PyAny>) -> PyResult<(f64, f64)> {
    let xy = ctx.getattr("screen")?;
    Ok((
        xy.call_method0("x")?.extract()?,
        xy.call_method0("y")?.extract()?,
    ))
}

fn entities<'py>(ctx: &Bound<'py, PyAny>, picks: &[&str]) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let view = ctx.getattr("viewport")?;
    let selection = view.getattr("scene")?.getattr("selection")?;
    if selection.is_truthy()? || picks.is_empty() {
        return selection.try_iter()?.collect();
    }
    let px = pixel(ctx)?;
    for name in picks {
        if let Some(entity) = hit(&view, name, px)? {
            return Ok(vec![entity]);
        }
    }
    Ok(Vec::new())
}

#[pyfunction]
pub fn transform_targets(ctx: &Bound<'_, PyAny>) -> PyResult<(Vec<Py<PyAny>>, Vec<Vec3>)> {
    let mut groups = Vec::new();
    let mut points = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |v: Bound<'_, PyAny>| -> PyResult<()> {
        let p = xyz(&v)?;
        if seen.insert(cell(p.to_array())) {
            points.push(Vec3::from(p));
        }
        Ok(())
    };
    for entity in entities(ctx, &["pick_group", "pick_edge", "pick_face"])? {
        match classify(&entity)? {
            "group" => groups.push(entity.unbind()),
            "edge" => {
                add(entity.getattr("a")?)?;
                add(entity.getattr("b")?)?;
            }
            "face" => {
                for p in entity.getattr("vertices")?.try_iter()? {
                    add(p?)?;
                }
            }
            _ => {}
        }
    }
    Ok((groups, points))
}

#[pyfunction]
pub fn transform_records<'py>(
    ctx: &Bound<'py, PyAny>,
    kind: &str,
) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let view = ctx.getattr("viewport")?;
    let scene = view.getattr("scene")?;
    let selection = scene.getattr("selection")?;
    if selection.is_truthy()? {
        let mut records = Vec::new();
        for entity in selection.try_iter()? {
            let entity = entity?;
            if classify(&entity)? == kind {
                records.push(entity);
            }
        }
        return Ok(records);
    }
    let name = format!("pick_{kind}");
    let Some(pick) = view.getattr_opt(name)? else {
        return Ok(Vec::new());
    };
    let px = pixel(ctx)?;
    let kwargs = PyDict::new(ctx.py());
    if kind == "text_label" {
        kwargs.set_item("rect_only", true)?;
    }
    let entity = pick.call(px, Some(&kwargs))?;
    if entity.is_none()
        || (kind == "guide"
            && (!scene
                .call_method1("entity_selectable", (&entity,))?
                .is_truthy()?
                || hit(&view, "pick_edge", px)?.is_some()))
    {
        return Ok(Vec::new());
    }
    Ok(vec![entity])
}

type Segment = [DVec3; 2];

#[pyclass(module = "ingetrazo_core")]
#[derive(Default)]
pub struct CopyWire {
    loose: Vec<Segment>,
    placements: Vec<Segment>,
}

#[pymethods]
impl CopyWire {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    #[staticmethod]
    fn gather(
        ctx: &Bound<'_, PyAny>,
        guides: &Bound<'_, PyAny>,
        pick_loose: bool,
    ) -> PyResult<(Vec<Py<PyAny>>, Vec<Py<PyAny>>, Self)> {
        let mut wire = Self::default();
        let (mut faces, mut edges) = (Vec::new(), Vec::new());
        let camera = ctx.getattr("viewport")?.getattr_opt("camera")?;
        let reach = camera
            .map(|c| c.getattr_opt("distance"))
            .transpose()?
            .flatten()
            .map(|v| v.extract::<f64>())
            .transpose()?
            .unwrap_or(10.)
            .max(1e-3);
        for guide in guides.try_iter()? {
            let guide = guide?;
            let p = xyz(&guide.getattr("point")?)?;
            let axis = guide.getattr("direction")?;
            if !axis.is_none() {
                let d = xyz(&axis)? * reach;
                wire.loose.push([p - d, p + d]);
            } else {
                for axis in [DVec3::X, DVec3::Y, DVec3::Z] {
                    let d = axis * reach * 0.02;
                    wire.loose.push([p - d, p + d]);
                }
                let origin = guide.getattr("origin")?;
                if !origin.is_none() {
                    wire.loose.push([xyz(&origin)?, p]);
                }
            }
        }
        let picks: &[&str] = if pick_loose {
            &["pick_edge", "pick_face"]
        } else {
            &[]
        };
        for entity in entities(ctx, picks)? {
            match classify(&entity)? {
                "face" => {
                    let holes = entity.getattr("holes")?;
                    for ring in
                        std::iter::once(Ok(entity.getattr("vertices")?)).chain(holes.try_iter()?)
                    {
                        let points = ring?
                            .try_iter()?
                            .map(|p| xyz(&p?))
                            .collect::<PyResult<Vec<_>>>()?;
                        wire.loose.extend(
                            points
                                .iter()
                                .zip(points.iter().cycle().skip(1))
                                .map(|(&a, &b)| [a, b]),
                        );
                    }
                    faces.push(entity.unbind());
                }
                "edge" => {
                    wire.loose
                        .push([xyz(&entity.getattr("a")?)?, xyz(&entity.getattr("b")?)?]);
                    edges.push(entity.unbind());
                }
                _ => {}
            }
        }
        Ok((faces, edges, wire))
    }

    #[pyo3(signature=(mesh, matrix=None))]
    fn add_placement(
        &mut self,
        mesh: &Bound<'_, PyAny>,
        matrix: Option<[f64; 16]>,
    ) -> PyResult<()> {
        let m = matrix
            .as_ref()
            .map(DMat4::from_cols_array)
            .unwrap_or(DMat4::IDENTITY);
        let d = mesh.getattr("__dict__")?;
        if let Some(store) = d.cast::<PyDict>()?.get_item("_store")? {
            let store = store.extract::<PyRef<PyMeshStore>>()?;
            for slot in store.inner.live_slots(Kind::Edge) {
                let segment =
                    store.inner.state.ends.get(slot as usize).map(|v| {
                        m.transform_point3((*store.inner.state.vpos.get(v as usize)).into())
                    });
                self.placements.push(segment);
            }
        } else {
            for edge in mesh.getattr("edges")?.try_iter()? {
                let edge = edge?;
                self.placements.push([
                    m.transform_point3(xyz(&edge.getattr("a")?)?),
                    m.transform_point3(xyz(&edge.getattr("b")?)?),
                ]);
            }
        }
        Ok(())
    }
    #[pyo3(signature=(matrix=None, delta=None))]
    fn lines(&self, matrix: Option<[f64; 16]>, delta: Option<[f64; 3]>) -> Vec<(Vec3, Vec3)> {
        let m = matrix
            .as_ref()
            .map(DMat4::from_cols_array)
            .unwrap_or_else(|| DMat4::from_translation(delta.unwrap_or_default().into()));
        self.placements
            .iter()
            .chain(&self.loose)
            .map(|&[a, b]| (m.transform_point3(a).into(), m.transform_point3(b).into()))
            .collect()
    }
    fn __len__(&self) -> usize {
        self.placements.len() + self.loose.len()
    }
}
