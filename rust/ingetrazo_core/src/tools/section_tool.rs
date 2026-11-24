//! Place a clipping plane facing the camera, with inferred, arrow or Shift orientation.
use super::{
    add, dot, json_p3,
    plane::{plane_axes, Axes},
    scale, sub, Command, Preview, Reply, Tool, ToolEvent, ToolHost, ToolInfo, P3,
};
use serde_json::{json, Value as Json};
static INFO: ToolInfo = ToolInfo::basic(
    "section",
    "Section Plane",
    "",
    "Place a plane that cuts the model open to show its inside.",
    true,
    None,
);
const UP: P3 = [0., 0., 1.];
pub struct Section {
    hover: Option<P3>,
    normal: P3,
    eye: Option<P3>,
    axis: Option<usize>,
    hold: Option<P3>,
    axes: Axes,
}
impl Default for Section {
    fn default() -> Self {
        Self {
            hover: None,
            normal: UP,
            eye: None,
            axis: None,
            hold: None,
            axes: Axes::world(),
        }
    }
}
impl Section {
    fn facing(&self, n: P3) -> P3 {
        if self
            .eye
            .zip(self.hover)
            .is_some_and(|(e, p)| dot(n, sub(e, p)) < 0.)
        {
            scale(n, -1.)
        } else {
            n
        }
    }
    fn current(&self) -> P3 {
        self.hold.unwrap_or_else(|| {
            self.axis
                .map(|i| self.facing(self.axes.axis(i).into()))
                .unwrap_or(self.normal)
        })
    }
}
impl Tool for Section {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn state(&self) -> Json {
        json!({"hover_point":self.hover,"normal":self.normal,"current":self.current(),"axis":self.axis.map(|i|["x","y","z"][i])})
    }
    fn set(&mut self, k: &str, v: &Json) -> bool {
        if k == "normal" {
            if let Some(n) = json_p3(v) {
                self.normal = n;
                return true;
            }
        }
        false
    }
    fn event(&mut self, ev: &ToolEvent, h: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate => {
                self.hover = None;
                self.hold = None;
                self.axis = None;
            }
            ToolEvent::Deactivate => self.hover = None,
            ToolEvent::Frame { axes, .. } => {
                self.axes = Axes {
                    o: axes[0].into(),
                    x: axes[1].into(),
                    y: axes[2].into(),
                    z: axes[3].into(),
                };
            }
            ToolEvent::Hover(p) => {
                self.hover = p.world;
                self.eye = h.eye();
                if p.mods.shift {
                    if self.hold.is_none() {
                        self.hold = Some(self.current());
                    }
                } else {
                    self.hold = None;
                    if self.axis.is_none() {
                        self.normal = json_p3(&h.call("section_normal", json!(p.px)))
                            .unwrap_or_else(|| self.facing(UP));
                    }
                }
            }
            ToolEvent::Key { key, .. } => {
                self.eye = h.eye();
                self.axis = match key.as_str() {
                    "ArrowRight" => Some(0),
                    "ArrowLeft" => Some(1),
                    "ArrowUp" => Some(2),
                    "ArrowDown" => None,
                    _ => return Reply::ignored(),
                };
            }
            ToolEvent::Click(p) => {
                self.eye = h.eye();
                if let Some(point) = p.world {
                    let answer = h.run(&Command {
                        name: "place_section_plane",
                        args: json!({"point":point,"normal":self.current()}),
                    });
                    if answer.ok {
                        return Reply::committed();
                    }
                }
            }
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
    fn preview(&self) -> Preview {
        let mut p = Preview::default();
        let Some(c) = self.hover else { return p };
        let n = self.current();
        let (u, v) = plane_axes(n.into(), &self.axes);
        let u = scale(u.into(), 1.2);
        let v = scale(v.into(), 1.2);
        let q = [
            sub(sub(c, u), v),
            sub(add(c, u), v),
            add(add(c, u), v),
            add(sub(c, u), v),
        ];
        for i in 0..4 {
            p.lines.push(Preview::rubber(q[i], q[(i + 1) % 4]));
        }
        p.lines.push(Preview::rubber(c, add(c, scale(n, 0.6))));
        p
    }
}
