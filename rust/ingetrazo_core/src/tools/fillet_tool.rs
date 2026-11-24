//! Interactive solid bevel. The host binds resident edges, the preview plan
//! and the shared panel. The machine chooses when to size, replan and commit.
use super::{
    dot, json_p3, len, scale, sub, Command, Reply, Tool, ToolEvent, ToolHost, ToolInfo, Value, P3,
};
use serde_json::{json, Value as Json};

static INFO: ToolInfo = ToolInfo {
    kind: "fillet",
    name: "Fillet 3D",
    shortcut: "Shift+F",
    description:
        "Round or bevel the edges of a solid; Ctrl changes the measure, «0.3p» the profile.",
    uses_snap: false,
    box_select: false,
    vcb_label: Some("Radius"),
    magnetic_axis_deg: None,
    screen_axis_px: None,
    axis_source: false,
    axis_source_empty_only: false,
};

#[derive(Default)]
pub struct Fillet {
    sizing: bool,
}

fn radius(world: P3, targets: &Json) -> Option<f64> {
    let nearest = targets
        .as_array()?
        .iter()
        .filter_map(|target| {
            let a = json_p3(&target[0])?;
            let off = sub(world, a);
            let Some(b) = json_p3(&target[1]) else {
                return Some(len(off));
            };
            let d = sub(b, a);
            let n = len(d);
            if n < 1e-9 {
                return None;
            }
            let axis = scale(d, 1. / n);
            Some(len(sub(off, scale(axis, dot(off, axis)))))
        })
        .reduce(f64::min)?;
    (nearest >= 1e-4).then_some(nearest)
}

impl Fillet {
    fn commit(&mut self, host: &mut dyn ToolHost) -> Reply {
        let args = host.call("fillet_plan", Json::Null);
        if !args.is_object() {
            return Reply::redraw();
        }
        let result = host.run(&Command {
            name: "bevel_selection",
            args,
        });
        if !result.ok {
            return Reply::redraw();
        }
        self.sizing = false;
        host.call("finish_bevel", Json::Null);
        Reply::committed()
    }
}
impl Tool for Fillet {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn busy(&self) -> bool {
        self.sizing
    }
    fn state(&self) -> Json {
        json!({"sizing":self.sizing})
    }
    fn set(&mut self, key: &str, value: &Json) -> bool {
        if key != "sizing" {
            return false;
        }
        let Some(sizing) = value.as_bool() else {
            return false;
        };
        self.sizing = sizing;
        true
    }
    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate | ToolEvent::Deactivate | ToolEvent::Cancel => {
                self.sizing = false;
                host.call("reset_bevel", json!(matches!(ev, ToolEvent::Activate)));
            }
            ToolEvent::Click(p) => {
                if self.sizing {
                    return self.commit(host);
                }
                if host.call("pick_bevel", json!(p.world)) != true {
                    return Reply::ignored();
                }
                self.sizing = true;
                host.call("replan", Json::Null);
            }
            ToolEvent::Hover(p) => {
                if self.sizing {
                    if let Some(r) = p
                        .world
                        .and_then(|w| radius(w, &host.call("bevel_geometry", Json::Null)))
                    {
                        host.call("panel_value", json!(["radius", r]));
                    }
                    host.call("replan", Json::Null);
                } else {
                    host.call("hover_bevel", json!(p.world));
                }
            }
            ToolEvent::Value(Value::Length(v)) if v.is_finite() && *v > 0. => {
                host.call("panel_value", json!(["radius", v]));
                if host.call("bevel_picked", Json::Null) == true {
                    return self.commit(host);
                }
                host.call("announce_size", Json::Null);
                host.call("replan", Json::Null);
            }
            ToolEvent::Segments(n) if *n >= 1 => {
                host.call("panel_value", json!(["segments", n]));
                host.call("replan", Json::Null);
            }
            ToolEvent::Profile(p) => {
                if !(0. < *p && *p < 1.) {
                    host.call("bad_profile", Json::Null);
                    return Reply::ignored();
                }
                host.call("panel_value", json!(["profile", p]));
                host.call("replan", Json::Null);
            }
            ToolEvent::Key { key, .. } if key == "Apply" => return self.commit(host),
            ToolEvent::Key { key, .. } if key == "AffectEdges" => self.sizing = false,
            ToolEvent::Key { key, .. } if key == "Control" => (),
            ToolEvent::KeyRelease { key, tapped: true } if key == "Control" => {
                if let Some(measure) = host.call("bevel_measure", Json::Null).as_str() {
                    let next = match measure {
                        "radius" => "offset",
                        "offset" => "width",
                        "width" => "depth",
                        _ => "radius",
                    };
                    host.call("measure", json!(next));
                }
            }
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
}
