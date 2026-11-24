//! Group axes from origin, red point and a green direction off the red line.
use super::{
    add, dot, json_p3, len, scale, sub, unit, Command, Preview, Reply, Tool, ToolEvent, ToolHost,
    ToolInfo, P3,
};
use serde_json::{json, Value as Json};
static INFO: ToolInfo = ToolInfo {
    kind: "change_axes",
    name: "Change Axes",
    shortcut: "",
    description: "",
    uses_snap: true,
    box_select: false,
    vcb_label: None,
    magnetic_axis_deg: None,
    screen_axis_px: None,
    axis_source: false,
    axis_source_empty_only: false,
};
pub fn axes(o: P3, r: P3, g: P3) -> Option<[P3; 3]> {
    if !o.into_iter().chain(r).chain(g).all(f64::is_finite) {
        return None;
    }
    let red = sub(r, o);
    if len(red) < 1e-9 {
        return None;
    }
    let x = unit(red);
    let d = sub(g, o);
    let green = sub(d, scale(x, dot(x, d)));
    if len(green) < 1e-6 {
        return None;
    }
    let y = unit(green);
    let z = glam::DVec3::from(x).cross(y.into()).normalize().into();
    Some([x, y, z])
}
#[derive(Default)]
pub struct ChangeAxes {
    origin: Option<P3>,
    red: Option<P3>,
    hover: Option<P3>,
}
impl Tool for ChangeAxes {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn busy(&self) -> bool {
        self.origin.is_some()
    }
    fn state(&self) -> Json {
        json!({"start_point":self.origin,"red_point":self.red,"hover_point":self.hover})
    }
    fn set(&mut self, key: &str, v: &Json) -> bool {
        match key {
            "start_point" => self.origin = json_p3(v),
            "red_point" => self.red = json_p3(v),
            "hover_point" => self.hover = json_p3(v),
            _ => return false,
        }
        true
    }
    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate | ToolEvent::Deactivate | ToolEvent::Cancel => {
                self.origin = None;
                self.red = None;
                if !matches!(ev, ToolEvent::Cancel) {
                    self.hover = None;
                }
                if matches!(ev, ToolEvent::Activate)
                    && host.call("axes_target", Json::Null).is_null()
                {
                    host.call("axes_message", json!("empty"));
                }
            }
            ToolEvent::Hover(p) => self.hover = p.world,
            ToolEvent::Click(p) => {
                let Some(g) = p.world else {
                    return Reply::ignored();
                };
                let target = host.call("axes_target", Json::Null);
                if target.is_null() {
                    return Reply::ignored();
                }
                let Some(o) = self.origin else {
                    self.origin = Some(g);
                    host.call("axes_message", json!("red"));
                    return Reply::redraw();
                };
                let Some(r) = self.red else {
                    if len(sub(g, o)) >= 1e-6 {
                        self.red = Some(g);
                        host.call("axes_message", json!("green"));
                    }
                    return Reply::redraw();
                };
                if axes(o, r, g).is_none() {
                    host.call("axes_message", json!("collinear"));
                    return Reply::redraw();
                }
                if host
                    .run(&Command {
                        name: "set_group_axes",
                        args: json!({"placement":target,"origin":o,"red":r,"green":g}),
                    })
                    .ok
                {
                    self.origin = None;
                    self.red = None;
                    host.call("axes_message", json!("done"));
                    return Reply::committed();
                }
            }
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
    fn preview(&self) -> Preview {
        let mut p = Preview::default();
        let (Some(o), Some(h)) = (self.origin, self.hover) else {
            return p;
        };
        if let Some(r) = self.red {
            if let Some(basis) = axes(o, r, h) {
                let length = len(sub(r, o)).max(0.5);
                p.lines
                    .extend(basis.map(|v| Preview::rubber(o, add(o, scale(v, length)))));
            } else {
                p.lines.push(Preview::rubber(o, r));
            }
        } else {
            p.lines.push(Preview::rubber(o, h));
        }
        p
    }
}
