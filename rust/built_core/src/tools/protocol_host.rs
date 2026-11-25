// SPDX-License-Identifier: GPL-3.0-or-later
//! Run a machine command through the document protocol. Coordinates for loose
//! drawing verbs are in the open mesh frame; record commands use scene space.
use super::{view_queries::ViewportToolHost, Answer, P3};
use pyo3::{
    exceptions::{PyIndexError, PyKeyError, PyValueError},
    prelude::*,
    types::PyDict,
};
use serde_json::{json, Value};

fn model_point(point: &mut Value, origin: P3) {
    if let Some(p) = super::json_p3(point) {
        *point = json!(super::add(p, origin));
    }
}
fn model_coordinates(name: &str, args: &mut Value, origin: P3) {
    for key in ["a", "b", "eye", "anchor", "center"] {
        if let Some(p) = args.get_mut(key) {
            model_point(p, origin);
        }
    }
    if matches!(name, "add_guide" | "set_group_axes") {
        for key in ["point", "origin", "red", "green"] {
            if let Some(p) = args.get_mut(key) {
                model_point(p, origin);
            }
        }
    }
    for key in ["outer", "points"] {
        if let Some(ring) = args.get_mut(key).and_then(Value::as_array_mut) {
            for p in ring {
                model_point(p, origin);
            }
        }
    }
    if let Some(lines) = args.get_mut("trim").and_then(Value::as_array_mut) {
        for segment in lines {
            if let Some(points) = segment.as_array_mut() {
                for p in points {
                    model_point(p, origin);
                }
            }
        }
    }
}

impl ViewportToolHost {
    pub(super) fn history_token(&self, py: Python<'_>) -> PyResult<u64> {
        let stack = self
            .vp
            .bind(py)
            .getattr("history")?
            .getattr_opt("undo_stack")?;
        let n = stack.as_ref().map(|s| s.len()).transpose()?.unwrap_or(0);
        let last = if n > 0 {
            stack.unwrap().get_item(n - 1)?.as_ptr() as usize
        } else {
            0
        };
        Ok((n, last).into_pyobject(py)?.hash()? as u64 & i64::MAX as u64)
    }
    pub(super) fn command(&self, py: Python<'_>, name: &str, args: &Value) -> PyResult<Answer> {
        let view = self.vp.bind(py);
        let history = view.getattr("history")?;
        let record = matches!(
            name,
            "place_group"
                | "paste_clipboard"
                | "paint_material"
                | "add_dimension"
                | "add_geo_path"
                | "move_geo_path_node"
                | "add_image_plane"
                | "place_section_plane"
                | "set_north"
                | "solid_operation"
                | "erase_stroke"
                | "follow_me"
        );
        let scene = view.getattr("scene")?;
        let mut arguments = std::borrow::Cow::Borrowed(args);
        let (before, faces) = if record {
            (0, 0)
        } else {
            let mesh = scene.getattr("mesh")?;
            if let Some(store) = mesh
                .getattr("__dict__")?
                .cast::<PyDict>()?
                .get_item("_store")?
            {
                if let Some(frame) = store.getattr_opt("frame_origin")? {
                    let origin: P3 = frame.extract()?;
                    if origin != [0.; 3] {
                        model_coordinates(name, arguments.to_mut(), origin);
                    }
                }
            }
            (
                self.history_token(py)?,
                scene.getattr("faces")?.len()? as i64,
            )
        };
        let kw = super::py::json_values(py, &arguments, false)?.cast_into::<PyDict>()?;
        for &(arg, attr) in match name {
            "place_group" => &[("_pending", "_group"), ("attachment", "_attachment")][..],
            "paste_clipboard" => &[("clipboard", "_clip")],
            "bevel_selection" => &[("_plan", "_plan")],
            "paint_material" => &[("_picked_placement", "_container")],
            _ => &[],
        } {
            let value = self
                .tool
                .as_ref()
                .map(|tool| tool.bind(py).getattr_opt(attr))
                .transpose()?
                .flatten();
            kw.set_item(arg, value)?;
        }
        let protocol = py.import("built_core.command_protocol")?;
        match protocol.getattr("run")?.call((history, name), Some(&kw)) {
            Ok(answer) => {
                let revision = self.history_token(py)?;
                let ok = if record {
                    answer.get_item("ok")?.is_truthy()?
                } else {
                    revision != before
                };
                let delta = if record {
                    0
                } else {
                    scene.getattr("faces")?.len()? as i64 - faces
                };
                Ok(Answer {
                    ok,
                    revision,
                    face_delta: delta,
                    error: answer.call_method1("get", ("error",))?.extract()?,
                })
            }
            Err(e) => {
                let stale = match name {
                    "place_group" | "paste_clipboard" => false,
                    "paint_material" => e.is_instance_of::<PyKeyError>(py),
                    "scale_selection" | "rotate_selection" | "offset_outline" => {
                        e.is_instance_of::<PyKeyError>(py) || e.is_instance_of::<PyValueError>(py)
                    }
                    _ => {
                        record
                            && (e.is_instance_of::<PyKeyError>(py)
                                || e.is_instance_of::<PyIndexError>(py))
                    }
                };
                if !stale {
                    return Err(e);
                }
                Ok(Answer {
                    revision: if record {
                        self.history_token(py)?
                    } else {
                        before
                    },
                    error: Some(e.value(py).str()?.to_string()),
                    ..Default::default()
                })
            }
        }
    }
}
