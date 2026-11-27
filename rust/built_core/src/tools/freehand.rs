//! Freehand stroke sampling and screen-space Ramer–Douglas–Peucker simplification.
use super::{Command, Preview, Reply, Tool, ToolEvent, ToolHost, ToolInfo, P3};
use glam::DVec2;
use serde_json::{json, Value as Json};

static INFO: ToolInfo = ToolInfo {
    kind: "freehand",
    name: "Freehand",
    shortcut: "",
    description: "Drag to sketch a curve; a stroke that ends where it began closes into a face.",
    uses_snap: false,
    box_select: false,
    vcb_label: None,
    magnetic_axis_deg: None,
    screen_axis_px: None,
    axis_source: false,
    axis_source_empty_only: false,
};
const SAMPLE_PX: f64 = 3.;
const SIMPLIFY_PX: f64 = 1.6;
const CLOSE_PX: f64 = 10.;

#[derive(Default)]
pub struct Freehand {
    stroke: Vec<(DVec2, P3)>,
}
impl Freehand {
    fn simplify(&self) -> Vec<P3> {
        let pts = &self.stroke;
        let end = pts.len() - 1;
        let mut keep = vec![false; pts.len()];
        keep[0] = true;
        keep[end] = true;
        let mut spans = vec![(0, end)];
        while let Some((a, b)) = spans.pop() {
            let start = pts[a].0;
            let d = pts[b].0 - start;
            let length = d.length_squared();
            let mut worst = SIMPLIFY_PX;
            let mut split = None;
            for (i, (p, _)) in pts.iter().enumerate().take(b).skip(a + 1) {
                let t = if length < 1e-12 {
                    0.
                } else {
                    ((*p - start).dot(d) / length).clamp(0., 1.)
                };
                let distance = p.distance(start + t * d);
                if distance > worst {
                    worst = distance;
                    split = Some(i);
                }
            }
            if let Some(i) = split {
                keep[i] = true;
                spans.extend([(i, b), (a, i)]);
            }
        }
        let mut result: Vec<_> = pts
            .iter()
            .zip(keep)
            .filter_map(|((_, p), keep)| keep.then_some(*p))
            .collect();
        if pts[0].0.distance_squared(pts[end].0) <= CLOSE_PX * CLOSE_PX && result.len() >= 3 {
            let start = result[0];
            *result.last_mut().unwrap() = start;
        }
        result
    }
}
impl Tool for Freehand {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn busy(&self) -> bool {
        !self.stroke.is_empty()
    }
    fn state(&self) -> Json {
        json!({"start_point":self.stroke.first().map(|(_,p)|p),"stroke":self.busy()})
    }
    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate | ToolEvent::Deactivate | ToolEvent::Cancel => self.stroke.clear(),
            ToolEvent::Click(p) | ToolEvent::Press(p) => {
                let Some(at) = p.world else {
                    return Reply::ignored();
                };
                self.stroke = vec![(p.px.into(), at)];
            }
            ToolEvent::Hover(p) | ToolEvent::Drag(p) => {
                let (Some((last, _)), Some(at)) = (self.stroke.last(), p.world) else {
                    return Reply::ignored();
                };
                let px: DVec2 = p.px.into();
                if last.distance_squared(px) < SAMPLE_PX * SAMPLE_PX {
                    return Reply::ignored();
                }
                self.stroke.push((px, at));
            }
            ToolEvent::Release(_) => {
                if self.stroke.len() < 3 {
                    self.stroke.clear();
                    return Reply::redraw();
                }
                let points = self.simplify();
                self.stroke.clear();
                if host
                    .run(&Command {
                        name: "add_arc",
                        args: json!({"points": points, "exact": false}),
                    })
                    .ok
                {
                    return Reply::committed();
                }
            }
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
    fn preview(&self) -> Preview {
        Preview {
            lines: self
                .stroke
                .windows(2)
                .map(|p| Preview::rubber(p[0].1, p[1].1))
                .collect(),
            ..Default::default()
        }
    }
}
