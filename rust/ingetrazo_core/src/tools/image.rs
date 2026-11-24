//! Size a reference image in its drawing plane, retaining its aspect by default.
use super::{
    add, dot, json_p3, len, scale, sub, Command, Preview, Reply, Tool, ToolEvent, ToolHost,
    ToolInfo, Value, P3,
};
use serde_json::{json, Value as Json};
static INFO: ToolInfo = ToolInfo::basic("image", "Image", "", "", true, Some("Width"));
const BASIS: [P3; 2] = [[1., 0., 0.], [0., 1., 0.]];
#[derive(Default)]
pub struct ImageTool {
    path: Option<String>,
    name: String,
    aspect: Option<f64>,
    start: Option<P3>,
    hover: Option<P3>,
    axes: Option<[P3; 2]>,
    free: bool,
}
impl ImageTool {
    fn reset(&mut self) {
        self.start = None;
        self.axes = None;
        self.free = false;
    }
    fn spans(&self, a: P3, b: P3, typed: Option<(f64, Option<f64>)>) -> [P3; 2] {
        let [u, v] = self.axes.unwrap_or(BASIS);
        let d = sub(b, a);
        let x = dot(d, u);
        let y = dot(d, v);
        let aspect = self.aspect.unwrap_or(1.);
        let sign = |v: f64| if v < 0. { -1. } else { 1. };
        let (w, h) = if let Some((w, h)) = typed {
            (sign(x) * w, sign(y) * h.unwrap_or(w * aspect))
        } else if self.free || x.abs() < 1e-4 {
            (x, y)
        } else {
            (x, sign(y) * x.abs() * aspect)
        };
        [scale(u, w), scale(v, h)]
    }
    fn place(&mut self, h: &mut dyn ToolHost, a: P3, span: [P3; 2]) -> Reply {
        let answer = h.run(&Command {
            name: "add_image_plane",
            args: json!({"path":self.path,"origin":a,
            "u":span[0],"v":span[1],"aspect":self.aspect.unwrap_or(1.),"name":self.name}),
        });
        if answer.ok {
            self.reset();
            self.path = None;
            Reply::committed()
        } else {
            Reply::ignored()
        }
    }
}
impl Tool for ImageTool {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn busy(&self) -> bool {
        self.start.is_some()
    }
    fn state(&self) -> Json {
        json!({"path":self.path,"aspect":self.aspect.unwrap_or(1.),"label":self.name,
        "start_point":self.start,"hover_point":self.hover,"free":self.free})
    }
    fn set(&mut self, k: &str, v: &Json) -> bool {
        match k {
            "load" => {
                let Ok((path, aspect, name)) =
                    serde_json::from_value::<(String, f64, String)>(v.clone())
                else {
                    return false;
                };
                self.path = Some(path);
                self.aspect = Some(if aspect > 0. { aspect } else { 1. });
                self.name = name;
                self.reset();
            }
            "axes" => self.axes = serde_json::from_value(v.clone()).ok(),
            "start_point" => self.start = json_p3(v),
            "hover_point" => self.hover = json_p3(v),
            "free" => self.free = v.as_bool().unwrap_or(false),
            _ => return false,
        }
        true
    }
    fn query(&self, k: &str, v: &Json) -> Json {
        if k == "frame" {
            if let Ok([a, b]) = serde_json::from_value::<[P3; 2]>(v.clone()) {
                return json!(self.spans(a, b, None));
            }
        }
        Json::Null
    }
    fn event(&mut self, ev: &ToolEvent, h: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate | ToolEvent::Cancel => self.reset(),
            ToolEvent::Deactivate => {
                self.reset();
                self.hover = None;
                self.path = None;
            }
            ToolEvent::Hover(p) => {
                self.hover = p.world;
                self.free = p.mods.shift;
            }
            ToolEvent::Click(p) if self.path.as_ref().is_some_and(|s| !s.is_empty()) => {
                let Some(b) = p.world else {
                    return Reply::ignored();
                };
                self.free = p.mods.shift;
                if let Some(a) = self.start {
                    let span = self.spans(a, b, None);
                    if span.iter().all(|&v| len(v) >= 1e-4) {
                        return self.place(h, a, span);
                    }
                } else {
                    self.start = Some(b);
                }
            }
            ToolEvent::Value(value) => {
                let (Some(a), Some(b), Some(_)) = (self.start, self.hover, &self.path) else {
                    return Reply::ignored();
                };
                let size = match value {
                    Value::Length(w) if *w > 0. => (*w, None),
                    Value::Pair(w, h) if *w > 0. && *h > 0. => (*w, Some(*h)),
                    _ => return Reply::ignored(),
                };
                return self.place(h, a, self.spans(a, b, Some(size)));
            }
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
    fn preview(&self) -> Preview {
        let mut p = Preview::default();
        if let (Some(a), Some(b), Some(_)) = (self.start, self.hover, &self.path) {
            let [u, v] = self.spans(a, b, None);
            let c = [a, add(a, u), add(add(a, u), v), add(a, v)];
            for i in 0..4 {
                p.lines.push(Preview::rubber(c[i], c[(i + 1) % 4]));
            }
        }
        p
    }
}
