// SPDX-License-Identifier: GPL-3.0-or-later
//! The transient document pose lasts one grab. The copy builder survives the
//! grab only while the machine has an open array window.
use super::{
    transform_pose::TransformPose,
    transform_queries::{transform_grab, CopyWire},
    view_queries::ViewportToolHost,
};
use crate::math::Vec3;
use glam::{DMat4, DVec3};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use serde_json::{json, Value};

#[derive(Default)]
#[pyclass(module = "built_core")]
pub struct MoveGesture {
    pose: Option<TransformPose>,
    delta: DVec3,
    resident: bool,
    builder: Option<Py<PyAny>>,
    repeat_uid: String,
}

#[pymethods]
impl MoveGesture {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    fn finish(&mut self, tool: &Bound<'_, PyAny>, keep_array: bool) -> PyResult<()> {
        self.pose = None;
        self.delta = DVec3::ZERO;
        self.resident = false;
        if !keep_array {
            self.builder = None;
        }
        for name in [
            "_positions",
            "_verts",
            "_groups",
            "_splanes",
            "_images",
            "_labels",
            "_moved_guides",
            "_sel_faces",
            "_sel_edges",
        ] {
            tool.setattr(name, PyList::empty(tool.py()))?;
        }
        tool.setattr("_dims", PyDict::new(tool.py()))?;
        tool.setattr("_base_segments", CopyWire::default())
    }
    fn close_array(&mut self) {
        self.builder = None;
    }
}
impl MoveGesture {
    fn preview(
        &mut self,
        tool: &Bound<'_, PyAny>,
        view: &Bound<'_, PyAny>,
        delta: DVec3,
    ) -> PyResult<()> {
        if (delta - self.delta).length() < 1e-12 {
            return Ok(());
        }
        let scene = view.getattr("scene")?;
        if self.pose.is_none() {
            self.pose = Some(TransformPose::new(tool, &scene, "move", self.resident)?);
        }
        self.pose.as_ref().unwrap().pose(
            &scene,
            (delta != DVec3::ZERO).then(|| DMat4::from_translation(delta)),
        )?;
        if self.resident {
            view.call_method1("set_groups_preview_offset", (Vec3::from(delta),))?;
        }
        self.delta = delta;
        Ok(())
    }
    fn revert(&mut self, view: &Bound<'_, PyAny>, park: bool) -> PyResult<()> {
        if !park {
            if let Some(end) = view.getattr_opt("end_groups_preview")? {
                end.call0()?;
            }
        }
        if let Some(pose) = self.pose.take() {
            if self.delta.length() >= 1e-12 {
                pose.pose(&view.getattr("scene")?, None)?;
            }
        }
        self.delta = DVec3::ZERO;
        if park && self.resident {
            view.call_method1("set_groups_preview_offset", (Vec3::from(DVec3::ZERO),))?;
        }
        Ok(())
    }
}

// Move's picked records and array source already name exact handles. Submit
// the resulting history plan through the same verb as scripted key-based edits.
fn submit_move(
    view: &Bound<'_, PyAny>,
    plan: &Bound<'_, PyAny>,
    delta: DVec3,
    copy: bool,
) -> PyResult<()> {
    let py = view.py();
    let kwargs = PyDict::new(py);
    kwargs.set_item("delta", delta.to_array())?;
    kwargs.set_item("copy", copy)?;
    kwargs.set_item("_plan", plan)?;
    py.import("built_core.command_protocol")?
        .getattr("run")?
        .call((view.getattr("history")?, "move_selection"), Some(&kwargs))?;
    Ok(())
}

impl ViewportToolHost {
    pub(super) fn move_service(
        &self,
        py: Python<'_>,
        verb: &str,
        arg: &Value,
    ) -> PyResult<Option<Value>> {
        let Some(tool) = self.tool.as_ref().map(|t| t.bind(py)) else {
            return Ok(None);
        };
        let Some(gesture) = tool.getattr_opt("_move_gesture")? else {
            return Ok(None);
        };
        let gesture = gesture.cast_into::<MoveGesture>()?;
        let view = self.vp.bind(py);
        let point = |arg: &Value| {
            super::json_p3(arg)
                .map(DVec3::from)
                .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("move expects a 3D delta"))
        };
        Ok(Some(match verb {
            "grab" => {
                let ctx = tool.getattr("_ctx")?;
                let grabbed = super::move_hover::grip_grab(tool, "move")?
                    || transform_grab(tool, &ctx, "move")?;
                if grabbed {
                    gesture.borrow_mut().resident = TransformPose::begin(view, tool)?;
                    gesture.borrow_mut().close_array();
                    CopyWire::capture(tool, &ctx)?;
                }
                json!(grabbed)
            }
            "preview" => {
                gesture.borrow_mut().preview(tool, view, point(arg)?)?;
                Value::Null
            }
            "revert" | "park" => {
                gesture.borrow_mut().revert(view, verb == "park")?;
                Value::Null
            }
            "move" => {
                let kwargs = PyDict::new(py);
                for (name, attr) in [
                    ("groups", "_groups"),
                    ("positions", "_positions"),
                    ("labels", "_labels"),
                    ("guides", "_moved_guides"),
                    ("dims", "_dims"),
                    ("splanes", "_splanes"),
                    ("images", "_images"),
                ] {
                    kwargs.set_item(name, tool.getattr(attr)?)?;
                }
                let cmds = py
                    .import("built_core.move_ops")?
                    .getattr("move_commands")?
                    .call((Vec3::from(point(arg)?),), Some(&kwargs))?;
                let command = match cmds.len()? {
                    0 => None,
                    1 => Some(cmds.get_item(0)?),
                    _ => Some(
                        py.import("built_core.history")?
                            .call_method1("CompoundCommand", (&cmds,))?,
                    ),
                };
                if let Some(command) = command {
                    submit_move(view, &command, point(arg)?, false)?;
                }
                tool.call_method1("_tell_join", (view, cmds))?;
                Value::Null
            }
            "copy" => {
                if arg["fresh"] == true || gesture.borrow().builder.is_none() {
                    tool.setattr("_copy_notice", py.None())?;
                    let builder =
                        tool.call_method1("_make_copy_builder", (view.getattr("scene")?,))?;
                    let mut state = gesture.borrow_mut();
                    state.repeat_uid = crate::element_ids::new_uid();
                    state.builder = Some(builder.unbind());
                }
                let deltas = arg["deltas"]
                    .as_array()
                    .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("copy needs offsets"))?
                    .iter()
                    .map(|v| point(v).map(Vec3::from))
                    .collect::<PyResult<Vec<_>>>()?;
                let builder = gesture.borrow().builder.as_ref().unwrap().clone_ref(py);
                if let (Some(run), Some(rule)) = (arg.get("run"), arg.get("rule")) {
                    if let Some(source) = builder
                        .bind(py)
                        .cast::<super::copy_history::MoveCopies>()
                        .ok()
                        .and_then(|b| b.borrow().repeat_source(py))
                    {
                        let cmd = crate::live::repeat_command(
                            py,
                            source.bind(py),
                            run.clone(),
                            rule.clone(),
                            gesture.borrow().repeat_uid.clone(),
                        )?;
                        let result = crate::protocol::execute(&view.getattr("history")?, &cmd)?;
                        return Ok(Some(
                            if result
                                .get_item("ok")?
                                .is_some_and(|v| v.is_truthy().unwrap_or(false))
                            {
                                json!(self.history_token(py)?)
                            } else {
                                Value::Null
                            },
                        ));
                    }
                }
                let cmd = builder.call1(py, (deltas,))?;
                let history = view.getattr("history")?;
                if cmd.is_none(py) {
                    Value::Null
                } else {
                    submit_move(view, cmd.bind(py), point(&arg["deltas"][0])?, true)?;
                    let stack = history.getattr_opt("undo_stack")?;
                    if let Some(stack) = stack.filter(|s| s.len().unwrap_or(0) > 0) {
                        if stack.get_item(-1)?.is(cmd.bind(py)) {
                            return Ok(Some(json!(self.history_token(py)?)));
                        }
                    }
                    Value::Null
                }
            }
            _ => return Ok(None),
        }))
    }
}
