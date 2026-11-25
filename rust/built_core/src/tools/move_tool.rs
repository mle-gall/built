//! Move: grab, drag, commit; Ctrl leaves a copy, and right after a copy
//! `3x` / `/3` / `5x10m` lay an array.
//!
//! The rules (from `tools/move.py`):
//! - The first click grabs what the host finds (the selection, or what
//!   lies under the cursor: a group, a vertex, an edge, a face, a guide, a
//!   label, a dimension, a section plane); nothing grabbed, nothing starts.
//! - While dragging, the host previews the move live (not in copy mode:
//!   the original stays and a wireframe shows the copy).
//! - Ctrl toggles the copy on the press; a Ctrl that was part of a chord
//!   takes itself back on release (#183). One operation spends it.
//! - A typed length runs along the drag (negative the other way); a
//!   `dx;dy;dz` is the delta.
//! - Right after a copy, `3x` lays three copies at multiples of the
//!   distance, `/3` three dividing it, `5x10m` five ten metres apart; typed
//!   during a Ctrl-drag, `5x10m` makes the copy there and then (#111). The
//!   window closes at the next action (the history token moves) or Esc.
//!
//! The host owns what is grabbed and the live preview (it is picking and
//! resident rows); the commit is `built_core.move_ops`, the implementation the
//! protocol's `move_selection` runs.

use glam::DVec3;
use serde_json::json;

use super::{Command, Preview, Reply, Tool, ToolEvent, ToolHost, ToolInfo, Value};
use super::preview::ValueAnchor;

static INFO: ToolInfo = ToolInfo {
    kind: "move",
    name: "Move",
    shortcut: "M",
    description: "Move the selection, or what you click, to a new place; Ctrl leaves a copy behind.",
    uses_snap: true,
    box_select: false,
    vcb_label: Some("Distance"),
    // Lock to the nearest axis within 15°: a move stays rigid and level.
    magnetic_axis_deg: Some(15.0),
    // …and the axes the vertical drag plane cannot hold (#42).
    screen_axis_px: Some(9.0),
    axis_source: false,
    axis_source_empty_only: false,
};

/// The copy just made, for the array window.
#[derive(Clone, Debug, PartialEq)]
struct Window { delta: DVec3, token: u64 }

#[derive(Default)]
pub struct MoveTool {
    grab: Option<DVec3>,
    hover: Option<DVec3>,
    copy: bool,
    ctrl_toggled: bool,
    last: Option<Window>,
}

fn d3(p: [f64; 3]) -> DVec3 { DVec3::from_array(p) }
fn j(v: DVec3) -> serde_json::Value { json!([v.x, v.y, v.z]) }

impl MoveTool {
    fn reset(&mut self) {
        self.grab = None;
        self.hover = None;
        self.copy = false; // the Ctrl modifier arms ONE operation
    }

    fn toggle_copy(&mut self, say: bool, host: &mut dyn ToolHost) {
        self.copy = !self.copy;
        if self.copy {
            // The original stops following the cursor.
            host.call("park", json!(null));
        } else if let (Some(g), Some(h)) = (self.grab, self.hover) {
            host.call("preview", j(h - g));
        }
        if say {
            host.message(if self.copy { "Move a copy: on" } else { "Move a copy: off" }, &[]);
        }
    }

    /// Commit a move (or a copy) by `delta`.
    fn commit(&mut self, delta: DVec3, host: &mut dyn ToolHost) {
        host.call("revert", json!(null));
        if self.copy && delta.length() > 1e-9 {
            let token = host.call("copy", json!({"deltas": [j(delta)], "fresh": true}));
            if let Some(t) = token.as_u64() {
                self.last = Some(Window { delta, token: t });
            }
        } else if delta.length() > 1e-9 {
            host.call("move", j(delta));
        }
        self.reset();
    }

    fn array(&mut self, count: i64, mode: &str, step: Option<f64>, host: &mut dyn ToolHost) -> bool {
        if let Some(step) = step {
            if let Some(grab) = self.grab {
                let Some(hover) = self.hover.filter(|_| self.copy) else { return false };
                let dir = hover - grab;
                if dir.length() < 1e-9 {
                    return false;
                }
                self.commit(dir.normalize() * step, host);
                if self.last.is_none() {
                    return false;
                }
                return self.array(count, mode, None, host);
            }
            if let Some(w) = self.last.as_mut() {
                if w.delta.length() < 1e-9 {
                    return false;
                }
                w.delta = w.delta.normalize() * step;
            }
        }
        if self.grab.is_some() || count < 1 {
            return false;
        }
        let Some(w) = self.last.clone() else { return false };
        if w.token != host.revision() {
            self.last = None;
            return false;
        }
        let deltas: Vec<serde_json::Value> = (1..=count).map(|k| {
            let f = if mode == "/" { k as f64 / count as f64 } else { k as f64 };
            j(w.delta * f)
        }).collect();
        host.run(&Command { name: "undo", args: json!({}) });
        let token = host.call("copy", json!({"deltas": deltas, "fresh": false}));
        let Some(t) = token.as_u64() else {
            self.last = None;
            return false;
        };
        self.last = Some(Window { token: t, ..w });
        host.message("{n} copies", &[("n", count.to_string())]);
        true
    }

    fn value(&mut self, value: &Value, host: &mut dyn ToolHost) -> bool {
        let Some(grab) = self.grab else { return false };
        let delta = match *value {
            Value::Triple(x, y, z) => DVec3::new(x, y, z),
            Value::Length(v) => {
                let Some(hover) = self.hover else { return false };
                let dir = hover - grab;
                if dir.length() < 1e-9 {
                    return false;
                }
                dir.normalize() * v
            }
            // A pair is a rectangle's width × height, not a move.
            _ => return false,
        };
        self.commit(delta, host);
        true
    }
}

impl Tool for MoveTool {
    fn info(&self) -> &'static ToolInfo { &INFO }

    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        let done = |ok: bool| Reply { consumed: ok, redraw: true, committed: ok };
        match ev {
            ToolEvent::Activate => {
                self.reset();
                self.last = None;
                Reply::redraw()
            }
            ToolEvent::Deactivate => {
                host.call("revert", json!(null));
                self.reset();
                self.last = None;
                Reply::redraw()
            }
            ToolEvent::Cancel => {
                host.call("revert", json!(null));
                self.reset();
                self.last = None;
                Reply::redraw()
            }
            ToolEvent::Click(p) | ToolEvent::DoubleClick(p) | ToolEvent::TripleClick(p) => {
                let Some(world) = p.world.map(d3) else { return Reply::ignored() };
                match self.grab {
                    None => {
                        if host.call("grab", j(world)).as_bool() == Some(true) {
                            self.last = None; // a new move ends the array window
                            self.grab = Some(world);
                        }
                        Reply::redraw()
                    }
                    Some(grab) => {
                        self.commit(world - grab, host);
                        Reply::committed()
                    }
                }
            }
            ToolEvent::Hover(p) => {
                let Some(world) = p.world.map(d3) else { return Reply::ignored() };
                self.hover = Some(world);
                if let Some(grab) = self.grab.filter(|_| !self.copy) {
                    host.call("preview", j(world - grab));
                }
                Reply::redraw()
            }
            ToolEvent::Value(v) => done(self.value(v, host)),
            ToolEvent::Array { count, mode, step } => done(self.array(*count, mode, *step, host)),
            ToolEvent::Key { key, .. } if key == "Control" => {
                self.toggle_copy(true, host);
                self.ctrl_toggled = true;
                Reply::redraw()
            }
            ToolEvent::KeyRelease { key, tapped } if key == "Control" => {
                if !self.ctrl_toggled {
                    return Reply::ignored();
                }
                self.ctrl_toggled = false;
                if !tapped {
                    self.toggle_copy(false, host);
                }
                Reply::redraw()
            }
            ToolEvent::Key { key, .. } if key == "ToggleCopy" || key == "ToggleCopyQuiet" => {
                // Ctrl held as the click lands (the view's own check).
                self.toggle_copy(key == "ToggleCopy", host);
                Reply::redraw()
            }
            ToolEvent::Key { key, .. } if key == "CloseArray" => {
                self.last = None;
                Reply::ignored()
            }
            _ => Reply::ignored(),
        }
    }

    fn preview(&self) -> Preview {
        let mut out = Preview::default();
        if let (Some(g), Some(h)) = (self.grab, self.hover) {
            out.lines.push(Preview::rubber(g.to_array(), h.to_array()));
            out.value = Some(ValueAnchor { at: ((g + h) * 0.5).to_array(), label: "Distance" });
        }
        out
    }

    fn busy(&self) -> bool { self.grab.is_some() }

    fn state(&self) -> serde_json::Value {
        let pt = |v: Option<DVec3>| v.map(j).unwrap_or(serde_json::Value::Null);
        json!({
            "start_point": pt(self.grab),
            "grab": pt(self.grab),
            "hover_point": pt(self.hover),
            "copy": self.copy,
            "window": self.last.as_ref().map(|w| json!({"delta": j(w.delta), "token": w.token})),
        })
    }

    fn set(&mut self, key: &str, value: &serde_json::Value) -> bool {
        let point = super::json_p3(value).map(d3);
        match key {
            "start_point" | "grab" => self.grab = point,
            "hover_point" => self.hover = point,
            "copy" => self.copy = value.as_bool().unwrap_or(false),
            "reset" => self.reset(),
            _ => return false,
        }
        true
    }

    fn query(&self, what: &str, _arg: &serde_json::Value) -> serde_json::Value {
        match what {
            "window_token" => self.last.as_ref().map(|w| json!(w.token)).unwrap_or_default(),
            _ => serde_json::Value::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{Answer, Pointer};

    #[derive(Default)]
    struct Host { log: Vec<String>, token: u64, grabbable: bool }

    impl ToolHost for Host {
        fn run(&mut self, cmd: &Command) -> Answer {
            self.log.push(cmd.name.to_string());
            self.token += 1;
            Answer { ok: true, revision: self.token, ..Answer::default() }
        }
        fn revision(&mut self) -> u64 { self.token }
        fn call(&mut self, verb: &str, args: serde_json::Value) -> serde_json::Value {
            self.log.push(format!("{verb} {args}"));
            match verb {
                "grab" => json!(self.grabbable),
                "copy" => { self.token += 1; json!(self.token) }
                "move" => { self.token += 1; json!(self.token) }
                _ => serde_json::Value::Null,
            }
        }
    }

    fn at(x: f64, y: f64) -> Pointer { Pointer { world: Some([x, y, 0.0]), ..Pointer::default() } }

    #[test]
    fn grab_drag_commit_and_the_array_window() {
        let mut t = MoveTool::default();
        let mut h = Host { grabbable: true, ..Host::default() };
        t.event(&ToolEvent::Key { key: "Control".into(), mods: Default::default() }, &mut h);
        t.event(&ToolEvent::KeyRelease { key: "Control".into(), tapped: true }, &mut h);
        t.event(&ToolEvent::Click(at(0.0, 0.0)), &mut h);
        t.event(&ToolEvent::Hover(at(0.0, 2.0)), &mut h);
        assert!(!h.log.iter().any(|l| l.starts_with("preview")), "a copy does not drag the original");
        t.event(&ToolEvent::Click(at(0.0, 2.0)), &mut h);
        assert!(h.log.last().unwrap().starts_with("copy"));
        assert!(t.event(&ToolEvent::Array { count: 3, mode: "x".into(), step: None }, &mut h).consumed);
        assert!(h.log.iter().any(|l| l.contains("[0.0,6.0,0.0]")), "{:?}", h.log);
        h.token += 5; // something else happened
        assert!(!t.event(&ToolEvent::Array { count: 2, mode: "x".into(), step: None }, &mut h).consumed);
    }

    #[test]
    fn nothing_grabbed_nothing_starts_and_a_typed_length_runs_along_the_drag() {
        let mut t = MoveTool::default();
        let mut h = Host::default();
        t.event(&ToolEvent::Click(at(0.0, 0.0)), &mut h);
        assert!(!t.busy());
        h.grabbable = true;
        t.event(&ToolEvent::Click(at(0.0, 0.0)), &mut h);
        t.event(&ToolEvent::Hover(at(3.0, 0.0)), &mut h);
        assert!(t.event(&ToolEvent::Value(Value::Length(-2.0)), &mut h).consumed);
        assert!(h.log.last().unwrap().starts_with("move [-2.0,"), "{:?}", h.log);
        assert!(!t.busy());
    }
}
