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
        if matches!(verb, "grab" | "preview" | "park" | "revert" | "move" | "copy") {
            if let Some(result) = self.move_service(py, verb, arg)? {
                return Ok(Some(result));
            }
        }
        Ok(Some(match verb {
            "begin_paste_preview" | "pose_paste_preview" | "end_paste_preview" | "can_stamp" =>
                self.paste_service(py, verb, arg)?,
            "sampler_armed" | "probe_container" | "pick_paint_face" | "clear_paint_hover"
            | "mark_paint_face" | "sample_image" | "sample_face" | "paint_targets"
            | "tape_source" => self.paint_service(py, verb, arg)?,
            "edge_under" | "corner_edges" | "rounded_corners" | "two_edge_corners"
            | "vertex_degree" | "tangent_at" => self.arc_service(py, verb, arg)?,
            "sweep_face_profile"=>{
                let px:(f64,f64)=args(arg)?;
                let Some(pair)=hit(view,"pick_face_placement",px)? else{return Ok(Some(Value::Null))};
                let face=pair.get_item(0)?;
                if face.is_none(){return Ok(Some(Value::Null));}
                let placement=pair.get_item(1)?;
                let matrix=placement.getattr_opt("xform")?.filter(|m|!m.is_none()).map(|m|m.call_method0("data")?.extract::<[f64;16]>()).transpose()?.map(|a|DMat4::from_cols_array(&a)).unwrap_or(DMat4::IDENTITY);
                let mut curves=Vec::new();
                let mut basis=None;
                for ring in std::iter::once(Ok(face.getattr("loop")?)).chain(face.getattr("holes")?.try_iter()?){
                    let vertices=ring?.try_iter()?.map(|v|Ok(matrix.transform_point3(DVec3::from(v?.call_method0("position_tuple")?.extract::<P3>()?)))).collect::<PyResult<Vec<_>>>()?;
                    if vertices.len()<3{return Ok(Some(Value::Null));}
                    if basis.is_none(){
                        let Some((_,n))=crate::snap_planes::face_plane(vertices.iter().map(|p|p.to_array()).collect()) else{return Ok(Some(Value::Null));};
                        let x=(vertices[1]-vertices[0]).normalize();
                        basis=Some((vertices[0],x,DVec3::from(n).cross(x)));
                    }
                    let (o,x,y)=basis.unwrap();
                    let xy=|v:DVec3|[(v-o).dot(x)*1000.,(v-o).dot(y)*1000.];
                    for (a,b) in vertices.iter().zip(vertices.iter().cycle().skip(1)){
                        curves.push(json!({"id":curves.len()+1,"kind":"line","a":xy(*a),"b":xy(*b)}));
                    }
                }
                json!({"name":"Face profile","use":"profile","curves":curves,"next_id":curves.len()+1})
            }
            "repeat_edge" => {
                let px: (f64, f64) = args(arg)?;
                let Some(edge) = hit(view, "pick_edge_any", px)? else {
                    return Ok(Some(Value::Null));
                };
                let scene = view.getattr("scene")?;
                let group = edge.getattr_opt("group")?.filter(|g| !g.is_none());
                let (mesh, path) = if let Some(g) = group {
                    let g = g.getattr_opt("source_occurrence")?.unwrap_or(g);
                    (
                        g.getattr("mesh")?,
                        scene
                            .call_method1("occurrence_path", (&g,))?
                            .extract::<Vec<String>>()?,
                    )
                } else {
                    (scene.getattr("mesh")?, Vec::new())
                };
                let Some(slot) = edge.getattr_opt("edge_id")? else {
                    return Ok(Some(Value::Null));
                };
                let store = mesh.getattr("_store")?;
                let Ok(key) = store.call_method1("key_at", ("edge", slot)) else {
                    return Ok(Some(Value::Null));
                };
                let id = store.call_method1("id_of", ("edge", key))?.extract()?;
                json!({"path":path,"id":crate::element_ids::to_text('e',id)})
            }
            "sweep_face_ref"=>{
                let px:(f64,f64)=args(&arg["px"])?;
                let Some(pair)=hit(view,"pick_face_inference",px)? else{return Ok(Some(Value::Null))};
                let face=pair.get_item(0)?;let placement=pair.get_item(1)?;
                if face.is_none(){return Ok(Some(Value::Null))}
                let (centre,normal)=local_plane(&face,placement.getattr_opt("xform")?.as_ref())?;
                let at:DVec3=args(&arg["at"])?;
                if (at-centre).dot(normal).abs()>1e-6{return Ok(Some(Value::Null))}
                let scene=view.getattr("scene")?;
                let (mesh,path)=if placement.is_none(){(scene.getattr("mesh")?,Vec::new())}else{
                    let owner=placement.getattr_opt("source_occurrence")?.unwrap_or(placement);
                    (owner.getattr("mesh")?,scene.call_method1("occurrence_path",(&owner,))?.extract::<Vec<String>>()?)
                };
                let id=mesh.getattr("_store")?.call_method1("id_of",("face",face.getattr("key")?))?.extract()?;
                json!({"to":{"path":path,"id":crate::element_ids::to_text('f',id)},"normal":normal})
            }
            "cut_error" => {view.call_method1("flash_status",(arg.as_str().unwrap_or("Cut failed"),))?; Value::Null}
            "cut_sections" => {
                let rows=py.import("built_core.command_protocol")?.call_method1("live_rows",(view.getattr("scene")?,))?;
                let mut ids=Vec::new();
                for row in rows.try_iter()? {
                    let row=row?;
                    if row.get_item("kind")?.eq("sketch")? && row.get_item("use").is_ok_and(|v|v.eq("section").unwrap_or(false)) && row.get_item("state")?.eq("ok")?{
                        ids.push(row.get_item("uid")?.extract::<String>()?);
                    }
                }
                json!(ids)
            }
            "cut_grip" => {
                let items=crate::live::live_cut_display(&view.getattr("scene")?)?;
                if items.len()?==0 {return Ok(Some(Value::Null));}
                let camera=Projection::from_view(view)?;
                let cursor=glam::DVec2::from(args::<[f64;2]>(arg)?);
                let mut nearest=9.;
                let mut hit=Value::Null;
                for item in items.try_iter()? {
                    let item=item?;
                    for grip in item.get_item("grips")?.try_iter()?{
                        let grip=grip?;
                        let at=grip.get_item("at")?.extract::<P3>()?;
                        if let Some(px)=camera.pixel(at.into())?{
                            let distance=cursor.distance(px.into());
                            if distance<nearest{
                                nearest=distance;
                                hit=json!({"kind":grip.get_item("kind")?.extract::<String>()?, "at":at,"row":super::py::py_to_json(&item.get_item("row")?)?,"axis":item.get_item("axis")?.extract::<P3>()?,"radius":item.get_item("radius")?.extract::<f64>()?});
                            }
                        }
                    }
                }
                if hit.is_null() {
                    // Project the visible wires without a depth query. A foreground
                    // face cannot swallow a pick on the cutter behind it.
                    for item in items.try_iter()? {
                        let item=item?;
                        for line in item.get_item("lines")?.extract::<Vec<[P3;2]>>()? {
                            let (Some(a),Some(b))=(camera.pixel(line[0].into())?,camera.pixel(line[1].into())?) else {continue;};
                            let a=glam::DVec2::from(a);
                            let ab=glam::DVec2::from(b)-a;
                            let u=((cursor-a).dot(ab)/ab.length_squared().max(1e-9)).clamp(0.,1.);
                            let d=cursor.distance(a+u*ab);
                            if d>=nearest.min(5.) {continue;}
                            let row=item.get_item("row")?;
                            let at=item.get_item("centre")?.extract::<P3>()?;
                            let grips=item.get_item("grips")?;
                            if grips.len()? > 0 {
                                nearest=d;
                                hit=json!({"kind":"offset","row":super::py::py_to_json(&row)?,"at":at,"axis":item.get_item("axis")?.extract::<P3>()?,"radius":item.get_item("radius")?.extract::<f64>()?});
                            } else if let Ok(body)=row.get_item("cutter")?.get_item("body") {
                                nearest=d;
                                hit=json!({"placement":body.extract::<String>()?});
                            }
                        }
                    }
                }
                hit
            }
            "cut_pick" => {
                let px:(f64,f64)=args(arg)?;
                let Some(pair)=hit(view,"pick_face_placement",px)? else{return Ok(Some(Value::Null));};
                let face=pair.get_item(0)?;let placed=pair.get_item(1)?;
                if face.is_none() || placed.is_none(){return Ok(Some(Value::Null));}
                let matrix=placed.getattr("xform")?;
                let xf=if matrix.is_none(){DMat4::IDENTITY}else{DMat4::from_cols_array(&matrix.call_method0("data")?.extract()?)};
                let group=placed.getattr_opt("source_occurrence")?.unwrap_or(placed);
                let scene=view.getattr("scene")?;
                let path:Vec<String>=scene.call_method1("occurrence_path",(&group,))?.extract()?;
                let id=group.getattr("mesh")?.getattr("_store")?.call_method1("id_of",("face",face.getattr("key")?))?.extract()?;
                let ring=face.getattr("vertices")?.try_iter()?.map(|v|Ok(xf.transform_point3(xyz(&v?)?).to_array())).collect::<PyResult<Vec<_>>>()?;
                let local=xyz(&face.call_method0("centroid")?)?;
                let ends=group.getattr("ext")?.call_method1("get",("built.live",))?;
                let points=ends.get_item("path").and_then(|p|p.extract::<Vec<[f64;3]>>()).unwrap_or_default();
                let repeat = group.getattr("ext")?.get_item("built.repeat").ok()
                    .and_then(|m|m.get_item("row").ok()).and_then(|id|id.extract::<String>().ok());
                let nearest = points.windows(2).enumerate().map(|(i,ab)| {
                    let a=DVec3::from(ab[0]);let b=DVec3::from(ab[1]);let axis=b-a;
                    let t=((local-a).dot(axis)/axis.length_squared()).clamp(0.,1.);
                    (i,t<0.5,local.distance_squared(a+t*axis))
                }).min_by(|a,b|a.2.total_cmp(&b.2));
                let (span,start,_) = nearest.unwrap_or_default();
                json!({"repeat":repeat,"placement":group.getattr("uid")?.extract::<String>()?,"ring":ring,"face":{"path":path,"id":crate::element_ids::to_text('f',id)},"span":span,"end":if start {"start"}else{"end"}})
            }
            "cut_selection" => {
                let selected=view.getattr("scene")?.getattr("selection")?;
                if selected.len()?==1 {
                    let g=selected.try_iter()?.next().transpose()?;
                    json!(g.and_then(|g|g.getattr("uid").ok()).and_then(|v|v.extract::<String>().ok()))
                } else {Value::Null}
            }
            "cut_draw_section" => {
                let win=view.call_method0("window")?;
                let workspace=py.import("built_render.sketch_workspace")?;
                if let (Some(path),Some(id))=(arg["face"]["path"].as_array(),arg["face"]["id"].as_str()){
                    let path=path.iter().filter_map(|p|p.as_str()).collect::<Vec<_>>().join("/");
                    workspace.call_method1("enter",(win,"section",format!("{path}|{id}")))?;
                }else{
                    workspace.call_method1("enter",(win,"new","section"))?;
                }
                Value::Null
            }
            "sweep_default" => crate::live::default_preset(&view.getattr("scene")?)?,
            "cut_picker" => {
                let canvas=view.call_method0("window")?.getattr("_host")?.getattr("view")?;
                canvas.getattr("renderer")?.call_method1("ui_open_commands",("", "Cut presets"))?;
                canvas.call_method0("update")?;
                Value::Null
            }
            "sweep_picker" => {
                let canvas=view.call_method0("window")?.getattr("_host")?.getattr("view")?;
                canvas.getattr("renderer")?.call_method1("ui_open_commands",("", "Sweep presets"))?;
                canvas.call_method0("update")?;
                Value::Null
            }
            "sweep_edges" => {
                let scene=view.getattr("scene")?;
                let owner=scene.getattr("edit_group")?;
                let path=if owner.is_none(){Vec::new()}else{scene.call_method1("occurrence_path",(&owner,))?.extract::<Vec<String>>()?};
                let store=scene.getattr("mesh")?.getattr("_store")?;
                let mut paths=Vec::new();
                for edge in scene.getattr("selection")?.try_iter()?{
                    let edge=edge?;
                    if edge.hasattr("v0")? && edge.hasattr("v1")? {
                        let id=store.call_method1("id_of",("edge",edge.getattr("key")?))?.extract()?;
                        paths.push(json!({"chain":[{"path":path,"id":crate::element_ids::to_text('e',id)}]}));
                    }
                }
                json!(paths)
            }
            "surface_path" => {
                // A line over a curved face follows its surface (world
                // points in and out; the store keeps them from its origin).
                let a: P3 = serde_json::from_value(arg["a"].clone()).map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
                let b: P3 = serde_json::from_value(arg["b"].clone()).map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
                let store = view
                    .getattr_opt("scene")?
                    .and_then(|s| s.getattr_opt("mesh").ok().flatten())
                    .and_then(|m| m.getattr_opt("_store").ok().flatten())
                    .filter(|s| !s.is_none());
                let Some(store) = store else { return Ok(Some(Value::Null)) };
                let Ok(st) = store.cast::<crate::store::py::PyMeshStore>() else { return Ok(Some(Value::Null)) };
                let st = st.borrow();
                let o: [f64; 3] = store.getattr("frame_origin")?.extract()?;
                let local = |p: P3| [p[0] - o[0], p[1] - o[1], p[2] - o[2]];
                let world = |v: &Vec<[f64; 3]>| v.iter().map(|p| [p[0] + o[0], p[1] + o[1], p[2] + o[2]]).collect::<Vec<_>>();
                match st.inner.surface_path(local(a), local(b)) {
                    Some(p) => json!({"path": world(&p.path), "curve": world(&p.curve)}),
                    None => Value::Null,
                }
            }
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
            "opening_feedback" => {
                let (invalid,warn):(bool,bool)=args(arg)?;
                if let Some(tool)=&self.tool {
                    tool.bind(py).setattr("wireframe_color",if invalid {(0.85,0.16,0.16,1.)}else{(0.13,0.17,0.23,1.)})?;
                }
                if warn { view.call_method0("window")?.call_method1("notify",("warning",py.import("built_core.i18n")?.call_method1("tr",(crate::hosted_openings::INVALID,))?))?; }
                Value::Null
            }
            "family_wall" => {
                let ray = view.call_method1("_pixel_to_ray", args::<(f64,f64)>(arg)?)?;
                let origin = xyz(&ray.get_item(0)?)?;
                let dir = xyz(&ray.get_item(1)?)?;
                let Some(pair) = hit(view, "pick_face_placement", args(arg)?)? else { return Ok(Some(Value::Null)); };
                let face = pair.get_item(0)?;
                let placement = pair.get_item(1)?;
                let owner = placement.getattr_opt("source_occurrence")?.unwrap_or_else(|| placement.clone());
                let record = owner.getattr_opt("ifc")?.filter(|r| !r.is_none());
                let wall = record.and_then(|r| r.get_item("class").ok()).and_then(|v| v.extract::<String>().ok());
                if face.is_none() || !matches!(wall.as_deref(), Some("IfcWall" | "IfcWallStandardCase"))
                    || owner.getattr_opt("locked")?.is_some_and(|v| v.is_truthy().unwrap_or(true)) {
                    return Ok(Some(crate::hosted_openings::aperture_at(&view.getattr("scene")?,origin,dir)?.unwrap_or(Value::Null)));
                }
                let matrix = placement.getattr_opt("xform")?.filter(|m| !m.is_none());
                let (center, normal) = local_plane(&face, matrix.as_ref())?;
                if normal.z.abs() > 0.001 { return Ok(Some(Value::Null)); }
                let dot = dir.dot(normal);
                if dot.abs() < 1e-8 { return Ok(Some(Value::Null)); }
                let point = origin + dir * (center-origin).dot(normal)/dot;
                if !view.getattr("scene")?.getattr("groups")?.contains(&owner)? {return Ok(Some(Value::Null));}
                crate::hosted_openings::pick(&owner,point,normal)?.unwrap_or(Value::Null)

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
