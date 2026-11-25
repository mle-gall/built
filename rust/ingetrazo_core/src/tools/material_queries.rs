// SPDX-License-Identifier: GPL-3.0-or-later
//! Paint's pick, hover and sample queries. Reading the material on a lazy
//! placement must not materialise its topology.
use super::{
    py::py_to_json,
    record_queries::clicked_back_side,
    view_queries::{classify, hit, ViewportToolHost},
};
use crate::math::{xyz, Vec3};
use glam::DVec3;
use pyo3::{prelude::*, types::PyDict};
use serde_json::{json, Value};

impl ViewportToolHost {
    pub(super) fn paint_service(&self, py: Python<'_>, verb: &str, arg: &Value) -> PyResult<Value> {
        let view = self.vp.bind(py);
        let tool = self
            .tool
            .as_ref()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("missing paint tool"))?
            .bind(py);
        let px = |v: &Value| {
            serde_json::from_value::<(f64, f64)>(v.clone())
                .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
        };
        let face = || tool.getattr("_face");
        let owner = || tool.getattr("_owner");
        Ok(match verb {
            "sampler_armed" => json!(
                tool.getattr("sample_armed")?.is_truthy()?
                    || view
                        .getattr_opt("_alt_down")?
                        .is_some_and(|v| v.is_truthy().unwrap_or(false))
            ),
            "probe_container" | "pick_paint_face" => {
                let px = px(arg)?;
                let pick = || view.call_method1("pick_face_any", px);
                let pair = if verb == "probe_container" {
                    let guard = py
                        .import("views.array_bake")?
                        .call_method0("reading_only")?;
                    guard.call_method0("__enter__")?;
                    let result = pick();
                    guard.call_method1("__exit__", (py.None(), py.None(), py.None()))?;
                    result?
                } else {
                    pick()?
                };
                let (face, owner) = (pair.get_item(0)?, pair.get_item(1)?);
                tool.setattr("_face", &face)?;
                tool.setattr("_owner", &owner)?;
                if verb == "probe_container" {
                    let container = if owner.is_none() {
                        None
                    } else {
                        hit(view, "pick_group", px)?
                    };
                    let ok = container.is_some() && !face.is_none();
                    tool.setattr("_container", container)?;
                    json!(ok)
                } else {
                    json!(!face.is_none())
                }
            }
            "clear_paint_hover" => {
                view.call_method1("set_hover", (py.None(),))?;
                Value::Null
            }
            "mark_paint_face" => {
                let px = px(arg)?;
                if let Some(f) = view
                    .getattr_opt("pick_face_placement")?
                    .filter(|p| p.is_callable())
                {
                    let pair = f.call1(px)?;
                    view.call_method1("set_hover", (pair.get_item(0)?, pair.get_item(1)?))?;
                } else {
                    let pair = view.call_method1("pick_face_any", px)?;
                    view.call_method1("set_hover", (pair.get_item(0)?,))?;
                }
                Value::Null
            }
            "sample_image" => {
                let (x, y) = px(arg)?;
                let face = face()?;
                let im = if face.is_none() {
                    hit(view, "image_plane_at", (x, y))?
                } else if let Some(f) = view.getattr_opt("image_in_front_of")? {
                    Some(f.call1((&face, x, y))?).filter(|v| !v.is_none())
                } else {
                    None
                };
                let Some(im) = im else { return Ok(Value::Null) };
                let tex = im.call_method0("texture")?;
                let texture = py_to_json(&tex)?;
                let normal = xyz(&im.call_method0("normal")?)?;
                let opacity = im
                    .getattr_opt("opacity")?
                    .map(|v| v.extract::<f64>())
                    .transpose()?
                    .unwrap_or(1.);
                let plane = if tex.call_method1("get", ("uvw",))?.is_truthy()? {
                    Some((normal.to_array(), normal.dot(xyz(&im.getattr("origin")?)?)))
                } else {
                    None
                };
                json!({"attrs":{"texture":texture,"opacity":(opacity<1.).then_some(opacity)},"plane":plane})
            }
            "sample_face" => {
                let (x, y) = px(arg)?;
                let face = face()?;
                let owner = owner()?;
                let attrs = face.getattr("attrs")?;
                let inherited = py
                    .import("core.group")?
                    .call_method1("effective_material", (&owner,))?;
                let front = py
                    .import("ingetrazo_core.materials")?
                    .call_method1("effective_attrs", (&attrs, inherited))?;
                let recipe = |d: &Bound<'_, PyAny>| -> PyResult<Value> {
                    if let Ok(d) = d.cast::<PyDict>() {
                        let mut fields = serde_json::Map::new();
                        for key in ["color", "texture", "opacity", "pbr", "mat"] {
                            if let Some(v) = d.get_item(key)? {
                                fields.insert(key.into(), py_to_json(&v)?);
                            }
                        }
                        Ok(Value::Object(fields))
                    } else {
                        py_to_json(d)
                    }
                };
                let n = xyz(&face.call_method0("normal")?)?;
                let vs = face.getattr("vertices")?;
                let v = if vs.is_truthy()? {
                    xyz(&vs.get_item(0)?)?
                } else {
                    DVec3::ZERO
                };
                json!({"front":recipe(&front)?,"back":recipe(&attrs.call_method1("get",("back",))?)?,"back_side":clicked_back_side(view,&face,&owner,x,y)?,"plane":[n.to_array(),n.dot(v)]})
            }
            "paint_targets" => {
                if arg[1] == true {
                    return Ok(
                        json!({"placement":tool.getattr("_container")?.getattr("uid")?.extract::<String>()?}),
                    );
                }
                let (x, y) = px(&arg[0])?;
                let face = face()?;
                let owner = owner()?;
                let scene = view.getattr("scene")?;
                let mesh = if owner.is_none() {
                    scene.getattr("mesh")?
                } else {
                    owner.getattr("mesh")?
                };
                let selection = scene.getattr("selection")?;
                let mut ids = Vec::new();
                if selection.contains(&face)? {
                    for f in selection.try_iter()? {
                        let f = f?;
                        if classify(&f)? == "face" {
                            ids.push(f.getattr("key")?.str()?.to_string())
                        }
                    }
                } else {
                    for f in mesh.call_method1("surface_of", (&face,))?.try_iter()? {
                        ids.push(f?.getattr("key")?.str()?.to_string());
                    }
                }
                let key = py
                    .import("ingetrazo_core.command_protocol")?
                    .call_method1("mesh_key", (&mesh,))?
                    .extract::<String>()?;
                json!({"mesh":key,"seed":face.getattr("key")?.str()?.to_string(),"faces":ids,"back":clicked_back_side(view,&face,&owner,x,y)?})
            }
            "tape_source" => {
                let px = px(&arg[0])?;
                let mut edge = if view
                    .getattr_opt("pick_edge_any")?
                    .is_some_and(|f| f.is_callable())
                {
                    hit(view, "pick_edge_any", px)?
                } else {
                    hit(view, "pick_edge", px)?
                };
                if edge.is_none() {
                    edge = hit(view, "pick_guide", px)?.filter(|g| {
                        g.getattr_opt("is_line")
                            .ok()
                            .flatten()
                            .is_some_and(|l| l.is_truthy().unwrap_or(false))
                    });
                }
                let mut point = None;
                if edge.is_none() && arg[2] != true {
                    let snap = tool.getattr("_snap")?;
                    let axes = py.import("ingetrazo_core.axes")?.getattr("AXES")?;
                    let mut axis = if arg[1] == "on_axis" {
                        snap.getattr_opt("axis")?.filter(|a| !a.is_none())
                    } else {
                        None
                    };
                    if axis
                        .as_ref()
                        .is_some_and(|a| axes.contains(a).unwrap_or(false))
                    {
                        point = Some(xyz(&snap.getattr("point")?)?);
                    } else {
                        axis = hit(view, "pick_axis", px)?;
                        if let Some(a) = axis.as_ref().filter(|a| axes.contains(a).unwrap_or(false))
                        {
                            let d = xyz(&axes.get_item(a)?)?;
                            point = Some(d * xyz(&tool.getattr("_click_point")?)?.dot(d));
                        }
                    }
                    if let Some(a) = axis.filter(|a| axes.contains(a).unwrap_or(false)) {
                        edge = Some(
                            py.import("ingetrazo_core.guide")?
                                .getattr("Guide")?
                                .call1((Vec3::from(DVec3::ZERO), axes.get_item(a)?))?,
                        );
                    }
                }
                tool.setattr("_source", &edge)?;
                if let Some(e) = edge {
                    json!({"a":xyz(&e.getattr("a")?)?.to_array(),"b":xyz(&e.getattr("b")?)?.to_array(),"point":point.map(|p|p.to_array())})
                } else {
                    Value::Null
                }
            }
            _ => unreachable!(),
        })
    }
}
