// SPDX-License-Identifier: GPL-3.0-or-later
//! Bridge clipboard records to the machine and keep a detached group preview
//! on the same display from activation through commit or cancellation.
use super::{plane::vec, view_queries::ViewportToolHost, P3};
use crate::math::xyz;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use serde_json::{json, Value};

pub(super) fn geometry(clip: Option<&Bound<'_, PyDict>>) -> PyResult<Value> {
    let Some(clip) = clip else {
        return Ok(Value::Null);
    };
    let mut faces = vec![];
    let mut edges = vec![];
    if let Some(rows) = clip.get_item("faces")? {
        for row in rows.try_iter()? {
            let row = row?;
            let ring = |points: Bound<'_, PyAny>| {
                points
                    .try_iter()?
                    .map(|p| Ok(xyz(&p?)?.to_array()))
                    .collect::<PyResult<Vec<P3>>>()
            };
            let mut rings = vec![ring(row.get_item(0)?)?];
            for hole in row.get_item(1)?.try_iter()? {
                rings.push(ring(hole?)?);
            }
            faces.push(rings);
        }
    }
    if let Some(rows) = clip.get_item("edges")? {
        for row in rows.try_iter()? {
            let row = row?;
            edges.push([
                xyz(&row.get_item(0)?)?.to_array(),
                xyz(&row.get_item(1)?)?.to_array(),
            ]);
        }
    }
    let anchor = clip
        .get_item("ref")?
        .map(|p| xyz(&p))
        .transpose()?
        .unwrap_or_default();
    Ok(json!({"ref":anchor.to_array(),"faces":faces,"edges":edges}))
}

impl ViewportToolHost {
    pub(super) fn paste_service(&self, py: Python<'_>, verb: &str, arg: &Value) -> PyResult<Value> {
        let vp = self.vp.bind(py);
        let tool = self
            .tool
            .as_ref()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("paste without a tool"))?
            .bind(py);
        let target = tool
            .getattr_opt("_preview_target")?
            .filter(|v| !v.is_none());
        match verb {
            "can_stamp" => {
                if let Some(clip) = tool.getattr("_clip")?.cast::<PyDict>().ok() {
                    for key in ["faces", "edges", "groups"] {
                        if clip
                            .get_item(key)?
                            .is_some_and(|v| v.len().unwrap_or(0) > 0)
                        {
                            return Ok(Value::Bool(true));
                        }
                    }
                }
                return Ok(Value::Bool(false));
            }
            "begin_paste_preview" => {
                let clip = tool.getattr("_clip")?;
                if let Ok(dict) = clip.cast::<PyDict>() {
                    if let Some(groups) = dict
                        .get_item("groups")?
                        .filter(|g| g.len().unwrap_or(0) > 0)
                    {
                        // Tool activation runs on the document controller. Pointer input
                        // and deactivation can arrive on different adapters to its display.
                        let mut owner = vp.clone();
                        if let Some(display) =
                            vp.getattr_opt("keys_stand_in")?.filter(|d| !d.is_none())
                        {
                            if display
                                .getattr_opt("renderer")?
                                .is_some_and(|r| !r.is_none())
                            {
                                if let Some(access) = display.getattr_opt("tools")? {
                                    owner = access;
                                }
                            }
                        }
                        if let Some(begin) = owner
                            .getattr_opt("begin_groups_preview")?
                            .filter(|m| m.is_callable())
                        {
                            let kw = PyDict::new(py);
                            kw.set_item("external", true)?;
                            begin.call((groups,), Some(&kw))?;
                            tool.setattr("_preview_target", owner)?;
                        }
                    }
                }
            }
            "pose_paste_preview" => {
                if let (Some(owner), Some(offset)) = (target, super::json_p3(arg)) {
                    owner.call_method1("set_groups_preview_offset", (vec(offset.into()),))?;
                } else {
                    vp.call_method0("update")?;
                }
            }
            "end_paste_preview" => {
                if let Some(owner) = target {
                    tool.setattr("_preview_target", py.None())?;
                    owner.call_method0("end_groups_preview")?;
                }
            }
            _ => unreachable!(),
        }
        Ok(Value::Null)
    }
}

/// Materialize an extension-compatible Face only when the painter asks.
/// Pin a planar image in the grab frame and reuse the document's UV mapper.
pub(super) fn preview_faces<'py>(
    py: Python<'py>,
    surfaces: &[Vec<Vec<P3>>],
    rows: &Bound<'py, PyAny>,
    offset: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyList>> {
    let out = PyList::empty(py);
    let face = py.import("core.geometry")?.getattr("Face")?;
    let translated = py.import("ingetrazo_core")?.getattr("group_translated_attrs")?;
    for (i, surface) in surfaces.iter().enumerate() {
        let Some((outer, holes)) = surface.split_first() else {
            continue;
        };
        let ring = |points: &Vec<P3>| points.iter().map(|p| vec((*p).into())).collect::<Vec<_>>();
        let holes: Vec<_> = holes.iter().map(ring).collect();
        let f = face.call1((ring(outer), (!holes.is_empty()).then_some(holes)))?;
        let row = rows.get_item(i)?;
        if row.len()? > 2 {
            let attrs = row.get_item(2)?;
            if let Ok(dict) = attrs.cast::<PyDict>() {
                let attrs = dict.copy()?;
                if let Some(t) = attrs
                    .get_item("texture")?
                    .and_then(|t| t.cast_into::<PyDict>().ok())
                {
                    let truth = |key| -> PyResult<bool> {
                        Ok(t.get_item(key)?
                            .map(|v| v.is_truthy())
                            .transpose()?
                            .unwrap_or(false))
                    };
                    if truth("path")? && !truth("uvw")? {
                        let axes = py
                            .import("core.texture")?
                            .call_method1("face_uv_axes", (&t, f.call_method0("normal")?))?;
                        let mut uvw = vec![];
                        for axis in [0, 2] {
                            uvw.extend(xyz(&axes.get_item(axis)?)?.to_array());
                            uvw.push(axes.get_item(axis + 1)?.extract()?);
                        }
                        let t = t.copy()?;
                        t.set_item("uvw", uvw)?;
                        attrs.set_item("texture", t)?;
                    }
                }
                if !attrs.is_empty() {
                    f.setattr("attrs", translated.call1((attrs, offset))?)?;
                }
            }
        }
        out.append(f)?;
    }
    Ok(out)
}
