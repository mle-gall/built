// SPDX-License-Identifier: GPL-3.0-or-later
//! Queries on the view's Python document records. The machine-facing result
//! stays in Rust; the view only supplies the pick, camera or record.
use super::{
    host_geometry::Projection,
    view_queries::{classify, hit, ViewportToolHost},
    P3,
};
use crate::math::{xyz, Vec3};
use glam::{DMat4, DVec3};
use pyo3::{exceptions::PyValueError, prelude::*, types::PyTuple};
use serde_json::{json, Value};
use std::collections::HashMap;

fn args<T: serde::de::DeserializeOwned>(value: &Value) -> PyResult<T> {
    serde_json::from_value(value.clone()).map_err(|e| PyValueError::new_err(e.to_string()))
}

pub(crate) fn local_plane(
    face: &Bound<'_, PyAny>,
    matrix: Option<&Bound<'_, PyAny>>,
) -> PyResult<(DVec3, DVec3)> {
    if let Some(matrix) = matrix.filter(|m| !m.is_none()) {
        let m = DMat4::from_cols_array(&matrix.call_method0("data")?.extract()?);
        let mut points = Vec::new();
        for p in face.getattr("vertices")?.try_iter()? {
            points.push(m.transform_point3(xyz(&p?)?).to_array());
        }
        let (center, n) = crate::snap_planes::face_plane(points).unwrap_or_default();
        let normal = if n == [0.; 3] {
            m.transform_vector3(xyz(&face.call_method0("normal")?)?)
                .normalize_or_zero()
        } else {
            n.into()
        };
        return Ok((center.into(), normal));
    }
    Ok((
        xyz(&face.call_method0("centroid")?)?,
        xyz(&face.call_method0("normal")?)?,
    ))
}

#[pyfunction]
#[pyo3(signature=(face, xform=None))]
pub fn face_plane_world(
    face: &Bound<'_, PyAny>,
    xform: Option<&Bound<'_, PyAny>>,
) -> PyResult<(Vec3, Vec3)> {
    let (p, n) = local_plane(face, xform)?;
    Ok((p.into(), n.into()))
}

#[pyfunction]
pub fn clicked_back_side(
    viewport: &Bound<'_, PyAny>,
    face: &Bound<'_, PyAny>,
    group: &Bound<'_, PyAny>,
    x: f64,
    y: f64,
) -> PyResult<bool> {
    let Some(ray) = hit(viewport, "_pixel_to_ray", (x, y))? else {
        return Ok(false);
    };
    if ray.get_item(0)?.is_none() || ray.get_item(1)?.is_none() {
        return Ok(false);
    }
    let (_, n) = local_plane(face, group.getattr_opt("xform")?.as_ref())?;
    Ok(n.dot(xyz(&ray.get_item(1)?)?) > 0.)
}

impl ViewportToolHost {
    /// None delegates to the extension or the UI callback supplied by the host.
    pub(super) fn service(
        &self,
        py: Python<'_>,
        verb: &str,
        arg: &Value,
    ) -> PyResult<Option<Value>> {
        let view = self.vp.bind(py);
        Ok(Some(match verb {
            "begin_paste_preview" | "pose_paste_preview" | "end_paste_preview" | "can_stamp" =>
                self.paste_service(py, verb, arg)?,
            "sampler_armed" | "probe_container" | "pick_paint_face" | "clear_paint_hover"
            | "mark_paint_face" | "sample_image" | "sample_face" | "paint_targets"
            | "tape_source" => self.paint_service(py, verb, arg)?,
            "edge_under" | "corner_edges" | "rounded_corners" | "two_edge_corners"
            | "vertex_degree" | "tangent_at" => self.arc_service(py, verb, arg)?,
            "north_pixels" | "to_px" => {
                if !view.hasattr("_world_to_pixel")? {
                    return Ok(Some(Value::Null));
                }
                let camera = Projection::from_view(view)?;
                let points: Vec<P3> = args(if verb == "to_px" { &arg["points"] } else { arg })?;
                let pixels = points
                    .into_iter()
                    .map(|p| camera.pixel(p.into()))
                    .collect::<PyResult<Vec<_>>>()?;
                json!(pixels)
            }
            "geo_project" => {
                json!(Projection::from_view(view)?.pixel(DVec3::from(args::<P3>(arg)?))?)
            }
            "geo_elevation" => {
                let p: P3 = args(arg)?;
                super::py::py_to_json(&view.call_method1("ground_elevation", (p[0], p[1]))?)?
            }
            "geo_nodes" => {
                let camera = Projection::from_view(view)?;
                let mut nodes = Vec::new();
                for path in view.getattr("scene")?.getattr("geo_paths")?.try_iter()? {
                    let path = path?;
                    for (i, p) in path.getattr("points")?.try_iter()?.enumerate() {
                        let p = p?;
                        if let Some(px) = camera.pixel(xyz(&view.call_method1("drape", (&p,))?)?)? {
                            nodes.push(json!([self.id(&path)?, i, xyz(&p)?.to_array(), px]));
                        }
                    }
                }
                Value::Array(nodes)
            }
            "geo_index" => {
                let path = self.object(py, args(arg)?)?;
                let paths = view.getattr("scene")?.getattr("geo_paths")?;
                if paths.contains(&path)? {
                    json!(paths.call_method1("index", (path,))?.extract::<usize>()?)
                } else {
                    Value::Null
                }
            }
            "geo_highlight" => {
                let marker: Option<(u64, usize)> = args(arg)?;
                view.setattr(
                    "_hover_geo_node",
                    marker
                        .map(|(id, i)| self.object(py, id).map(|p| (p, i)))
                        .transpose()?,
                )?;
                Value::Null
            }
            "section_normal" | "placement_normal" => {
                let method = if verb == "placement_normal" && view.hasattr("pick_face_placement")? {
                    "pick_face_placement"
                } else {
                    "pick_face_any"
                };
                let Some(entity) = hit(view, method, args(arg)?)? else {
                    return Ok(Some(Value::Null));
                };
                let (face, placement) = if entity.is_instance_of::<PyTuple>() {
                    (entity.get_item(0)?, entity.get_item(1)?)
                } else {
                    (entity, py.None().into_bound(py))
                };
                if face.is_none() {
                    Value::Null
                } else if verb == "section_normal" {
                    json!(xyz(&face.call_method0("normal")?)?
                        .normalize_or_zero()
                        .to_array())
                } else {
                    json!(
                        local_plane(&face, placement.getattr_opt("xform")?.as_ref())?
                            .1
                            .to_array()
                    )
                }
            }
            "forward" => json!(xyz(&view.getattr("camera")?.call_method0("forward")?)?.to_array()),
            "view_size" => {
                let size = |name, default| -> PyResult<f64> {
                    Ok(view
                        .getattr_opt(name)?
                        .map(|f| f.call0()?.extract())
                        .transpose()?
                        .unwrap_or(default))
                };
                json!([size("width", 1000.)?, size("height", 700.)?])
            }
            "dimension_lock" => match view.getattr_opt("axis_lock")? {
                Some(lock) => super::py::py_to_json(&lock)?,
                None => Value::Null,
            },
            "dimension_unlock" => {
                if view
                    .getattr_opt("_shift_lock")?
                    .map(|s| s.is_truthy())
                    .transpose()?
                    .unwrap_or(false)
                {
                    view.setattr("_shift_lock", py.None())?;
                }
                Value::Null
            }
            "offset_ray" => super::py::py_to_json(
                &view.call_method1("_pixel_to_ray", args::<(f64, f64)>(arg)?)?,
            )?,
            "eraser_targets" => {
                let mut result: HashMap<&str, Vec<Value>> = HashMap::new();
                let scene = view.getattr("scene")?;
                for id in args::<Vec<u64>>(arg)? {
                    let obj = self.object(py, id)?;
                    let (key, value) = match classify(&obj)? {
                        "edge" => ("edges", json!(obj.getattr("key")?.str()?.to_string())),
                        "group" => (
                            "placements",
                            json!(obj.getattr("uid")?.extract::<String>()?),
                        ),
                        kind => {
                            let field = match kind {
                                "guide" => "guides",
                                "geopath" => "geo_paths",
                                "text_label" => "text_labels",
                                "dimension" => "dimensions",
                                _ => continue,
                            };
                            let items = scene.getattr(field)?;
                            if !items.contains(&obj)? {
                                continue;
                            }
                            (
                                field,
                                json!(items.call_method1("index", (&obj,))?.extract::<usize>()?),
                            )
                        }
                    };
                    result.entry(key).or_default().push(value);
                }
                json!(result)
            }
            "curve_pick" => {
                let obj = self.object(py, args(arg)?)?;
                let edges = view
                    .getattr("scene")?
                    .getattr("mesh")?
                    .call_method1("curve_edges", (obj,))?;
                let ids = edges
                    .try_iter()?
                    .map(|e| self.id(&e?))
                    .collect::<PyResult<Vec<_>>>()?;
                json!(ids)
            }
            _ => return Ok(None),
        }))
    }
}

pub(super) const BOX_EDGES: [[usize; 2]; 12] = [
    [0, 1],
    [1, 3],
    [3, 2],
    [2, 0],
    [4, 5],
    [5, 7],
    [7, 6],
    [6, 4],
    [0, 4],
    [1, 5],
    [2, 6],
    [3, 7],
];
