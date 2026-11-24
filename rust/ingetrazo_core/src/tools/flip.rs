//! Flip plane selection, one-shot copy, and mirror commands.
use super::{json_p3, Command, Reply, Tool, ToolEvent, ToolHost, ToolInfo, P3};
use serde_json::{json, Value as Json};
static INFO: ToolInfo = ToolInfo {
    kind: "flip",
    name: "Flip",
    shortcut: "",
    description: "Mirror the selection across its red, green or blue plane.",
    uses_snap: false,
    box_select: false,
    vcb_label: None,
    magnetic_axis_deg: None,
    screen_axis_px: None,
    axis_source: false,
    axis_source_empty_only: false,
};

pub fn quad(axis: &str, c: P3, half: P3, margin: f64) -> [P3; 4] {
    let (u, v) = match axis {
        "x" => (1, 2),
        "y" => (0, 2),
        _ => (0, 1),
    };
    [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)].map(|(su, sv)| {
        let mut p = c;
        p[u] += su * half[u].max(0.3) * margin;
        p[v] += sv * half[v].max(0.3) * margin;
        p
    })
}
#[derive(Default)]
pub struct Flip {
    locked: Option<String>,
    hover: Option<String>,
    point: Option<P3>,
    copy: bool,
    ctrl: bool,
}
impl Flip {
    fn toggle(&mut self, host: &mut dyn ToolHost, say: bool) {
        self.copy = !self.copy;
        host.call("flip_copy", json!([self.copy, say]));
    }
    fn mirror(host: &mut dyn ToolHost, axis: &str, copy: bool) -> Reply {
        if !matches!(axis, "x" | "y" | "z") {
            return Reply::ignored();
        }
        let mut args = host.call("flip_targets", json!(axis));
        if !args.is_object() {
            host.call("empty_flip", Json::Null);
            return Reply::ignored();
        }
        args["copy"] = json!(copy);
        if host
            .run(&Command {
                name: "flip_selection",
                args,
            })
            .ok
        {
            Reply::committed()
        } else {
            Reply::ignored()
        }
    }
}
impl Tool for Flip {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn state(&self) -> Json {
        json!({"lock_axis":self.locked,"hover_axis":self.hover,"hover_point":self.point,"copy":self.copy})
    }
    fn set(&mut self, key: &str, v: &Json) -> bool {
        match key {
            "hover_point" => self.point = json_p3(v),
            "hover_axis" => self.hover = v.as_str().map(str::to_string),
            "lock_axis" => self.locked = v.as_str().map(str::to_string),
            "copy" => self.copy = v == true,
            _ => return false,
        }
        true
    }
    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate => {
                self.locked = None;
                self.hover = None;
                self.copy = false;
                self.ctrl = false;
                if host.call("flip_selected", Json::Null) != true {
                    host.call("empty_flip", Json::Null);
                }
            }
            ToolEvent::Deactivate => self.hover = None,
            ToolEvent::Cancel => self.locked = None,
            ToolEvent::Key { key, .. } if key == "Control" => {
                self.toggle(host, true);
                self.ctrl = true;
            }
            ToolEvent::KeyRelease { key, tapped } if key == "Control" && self.ctrl => {
                self.ctrl = false;
                if !tapped {
                    self.toggle(host, false);
                }
            }
            ToolEvent::Key { key, .. } => {
                let axis = match key.as_str() {
                    "Right" => "x",
                    "Left" => "y",
                    "Up" => "z",
                    _ => return Reply::ignored(),
                };
                self.locked = if self.locked.as_deref() == Some(axis) {
                    None
                } else {
                    Some(axis.to_owned())
                };
            }
            ToolEvent::Hover(p) => {
                self.point = p.world;
                self.hover = host
                    .call("flip_plane_at", json!(p.px))
                    .as_str()
                    .map(str::to_owned);
            }
            ToolEvent::Click(_) => {
                let Some(axis) = self.locked.as_ref().or(self.hover.as_ref()) else {
                    return Reply::ignored();
                };
                let result = Self::mirror(host, axis, self.copy);
                self.copy = false;
                host.call("flip_cursor", json!(false));
                return result;
            }
            ToolEvent::FlipAlong { axis, copy } => return Self::mirror(host, axis, *copy),
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
}
