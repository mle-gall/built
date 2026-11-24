//! Trace a ground path, insert measured segments, or pick up an existing node.
use super::{
    add, len, scale, sub, unit, Command, Preview, Reply, Tool, ToolEvent, ToolHost, ToolInfo,
    Value, P3,
};
use serde_json::{json, Value as Json};
static INFO: ToolInfo = ToolInfo::basic(
    "geopath",
    "Path",
    "Y",
    "Trace a path over the base map — a road, a boundary — to profile or measure it; the model is left untouched.",
    false,
    Some("Length"),
);
struct Grab {
    path: u64,
    index: usize,
    orig: P3,
    live: P3,
}
#[derive(Default)]
pub struct GeoPath {
    nodes: Vec<P3>,
    hover: Option<P3>,
    start_z: Option<f64>,
    z: Option<f64>,
    grab: Option<Grab>,
}
fn flat(p: P3) -> P3 {
    [p[0], p[1], 0.]
}
type Node = (u64, usize, P3, [f64; 2]);
fn pick(h: &mut dyn ToolHost, px: [f64; 2]) -> Option<Node> {
    let candidates: Vec<Node> =
        serde_json::from_value(h.call("geo_nodes", Json::Null)).unwrap_or_default();
    let dist = |v: &Node| (v.3[0] - px[0]).hypot(v.3[1] - px[1]);
    candidates
        .into_iter()
        .filter(|v| dist(v) < 9.)
        .min_by(|a, b| dist(a).total_cmp(&dist(b)))
}
impl GeoPath {
    fn push(&mut self, h: &mut dyn ToolHost, p: P3) {
        self.nodes.push(p);
        self.start_z = h.call("geo_elevation", json!(p)).as_f64();
    }
    fn finish(&mut self, h: &mut dyn ToolHost, closed: bool) -> Reply {
        let pts = std::mem::take(&mut self.nodes);
        self.start_z = None;
        if pts.len() >= 2
            && h.run(&Command {
                name: "add_geo_path",
                args: json!({"points":pts,"closed":closed}),
            })
            .ok
        {
            Reply::committed()
        } else {
            Reply::redraw()
        }
    }
    fn restore(&mut self, h: &mut dyn ToolHost) -> Option<Grab> {
        let g = self.grab.take()?;
        h.call("geo_preview", json!([g.path, g.index, g.orig]));
        Some(g)
    }
}
impl Tool for GeoPath {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn busy(&self) -> bool {
        self.grab.is_some() || !self.nodes.is_empty()
    }
    fn state(&self) -> Json {
        let label = self.hover.map(|c| {
            if let Some(&a) = self.nodes.last() {
                let run = len(sub(c, a));
                let rise = self
                    .z
                    .zip(self.start_z)
                    .map(|(z, s)| z - s)
                    .filter(|_| run >= 1e-6);
                json!([
                    scale(add(c, a), 0.5),
                    run,
                    self.z,
                    rise,
                    rise.map(|r| 100. * r / run)
                ])
            } else {
                json!([c, null, self.z, null, null])
            }
        });
        json!({"nodes":self.nodes,"hover_point":self.hover,"elevation":self.z,"label":label})
    }
    fn event(&mut self, ev: &ToolEvent, h: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate | ToolEvent::Deactivate | ToolEvent::Cancel => {
                self.restore(h);
                self.nodes.clear();
                self.start_z = None;
                if matches!(ev, ToolEvent::Activate) {
                    h.call("geo_plan", json!(true));
                }
                if matches!(ev, ToolEvent::Deactivate) {
                    self.hover = None;
                    self.z = None;
                    h.call("geo_highlight", Json::Null);
                    h.call("geo_plan", json!(false));
                }
            }
            ToolEvent::Click(p) => {
                if let Some(g) = self.restore(h) {
                    let path = h.call("geo_index", json!(g.path));
                    if !path.is_null()
                        && h.run(&Command {
                            name: "move_geo_path_node",
                            args: json!({"path":path,"node":g.index,"point":g.live}),
                        })
                        .ok
                    {
                        return Reply::committed();
                    }
                    return Reply::redraw();
                }
                let Some(c) = p.world.map(flat) else {
                    return Reply::ignored();
                };
                if self.nodes.is_empty() {
                    if let Some((path, index, orig, _)) = pick(h, p.px) {
                        self.grab = Some(Grab {
                            path,
                            index,
                            orig,
                            live: orig,
                        });
                        return Reply::redraw();
                    }
                }
                if self.nodes.len() >= 3 {
                    if let Ok([x, y]) = serde_json::from_value::<[f64; 2]>(
                        h.call("geo_project", json!(self.nodes[0])),
                    ) {
                        if (x - p.px[0]).hypot(y - p.px[1]) <= 10. {
                            return self.finish(h, true);
                        }
                    }
                }
                self.push(h, c);
            }
            ToolEvent::Hover(p) => {
                let Some(c) = p.world.map(flat) else {
                    return Reply::ignored();
                };
                if let Some(g) = &mut self.grab {
                    g.live = c;
                    h.call("geo_preview", json!([g.path, g.index, c]));
                } else {
                    self.hover = Some(c);
                    self.z = h.call("geo_elevation", json!(c)).as_f64();
                    let node = if self.nodes.is_empty() {
                        pick(h, p.px).map(|v| (v.0, v.1))
                    } else {
                        None
                    };
                    h.call("geo_highlight", json!(node));
                }
            }
            ToolEvent::DoubleClick(_) => return self.finish(h, false),
            ToolEvent::Key { key, .. } if key == "Enter" => return self.finish(h, false),
            ToolEvent::Value(v) => {
                let Some((&a, b)) = self.nodes.last().zip(self.hover) else {
                    return Reply::ignored();
                };
                let c = match v {
                    Value::Triple(x, y, _) => [a[0] + x, a[1] + y, 0.],
                    Value::Length(w) if *w > 0. && len(sub(b, a)) >= 1e-9 => {
                        flat(add(a, scale(unit(sub(b, a)), *w)))
                    }
                    _ => return Reply::ignored(),
                };
                self.push(h, c);
                self.hover = Some(c);
            }
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
    fn preview(&self) -> Preview {
        let mut p = Preview::default();
        p.lines
            .extend(self.nodes.windows(2).map(|w| Preview::rubber(w[0], w[1])));
        if let Some((&a, b)) = self.nodes.last().zip(self.hover) {
            p.lines.push(Preview::rubber(a, b));
        }
        p
    }
}
