//! Follow a preselected path, drag a chain, or trace a face perimeter.
use super::{
    dot, len, sub, unit, Command, Preview, Reply, Tool, ToolEvent, ToolHost, ToolInfo, P3,
};
use serde_json::{json, Value as Json};

static INFO: ToolInfo = ToolInfo {
    kind: "followme",
    name: "Follow Me",
    shortcut: "W",
    description: "Sweep a face along a path of edges — mouldings, pipes, turned shapes.",
    uses_snap: false,
    box_select: false,
    vcb_label: None,
    magnetic_axis_deg: None,
    screen_axis_px: None,
    axis_source: false,
    axis_source_empty_only: false,
};

#[derive(Default)]
pub struct FollowMe {
    preset: Option<(Vec<P3>, bool)>,
    hovered: Option<String>,
    drag: Option<Drag>,
}

#[derive(Default)]
struct Drag {
    face: String,
    press: [f64; 2],
    last: [f64; 2],
    // Mesh vertex keys, followed by their world coordinates.
    chain: Vec<(String, P3)>,
    crossing: Option<P3>,
    perimeter: Option<Vec<P3>>,
}
impl Drag {
    fn path(&self) -> (Vec<P3>, bool) {
        if let Some(pts) = &self.perimeter {
            return (pts.clone(), true);
        }
        let mut pts: Vec<_> = self
            .crossing
            .into_iter()
            .chain(self.chain.iter().map(|v| v.1))
            .collect();
        let closed = self.crossing.is_none()
            && self.chain.len() >= 4
            && self.chain.first() == self.chain.last();
        if closed {
            pts.pop();
        }
        (pts, closed)
    }
}
fn orient(mut pts: Vec<P3>, closed: bool, center: P3, normal: P3) -> Vec<P3> {
    if pts.len() < 2 {
        return pts;
    }
    let distance = |p: P3| len(sub(p, center));
    if !closed {
        if distance(*pts.last().unwrap()) < distance(pts[0]) {
            pts.reverse();
        }
    } else if pts.len() >= 3 {
        let nearest = (0..pts.len())
            .min_by(|&a, &b| distance(pts[a]).total_cmp(&distance(pts[b])))
            .unwrap();
        pts.rotate_left(nearest);
        let a = sub(pts[1], pts[0]);
        let b = sub(*pts.last().unwrap(), pts[0]);
        if len(a) > 1e-9
            && len(b) > 1e-9
            && dot(unit(b), normal).abs() > dot(unit(a), normal).abs() + 1e-9
        {
            pts[1..].reverse();
        }
    }
    pts
}
impl FollowMe {
    fn commit(
        &mut self,
        host: &mut dyn ToolHost,
        face: String,
        path: Vec<P3>,
        closed: bool,
    ) -> Reply {
        self.hovered = None;
        let answer = host.run(&Command {
            name: "follow_me",
            args: json!({"face":face, "path":path, "closed":closed}),
        });
        host.call("sweep_done", json!(answer.ok));
        if answer.ok {
            Reply::committed()
        } else {
            Reply::redraw()
        }
    }
    fn finish(&mut self, host: &mut dyn ToolHost) -> Reply {
        let Some(d) = self.drag.take() else {
            return Reply::ignored();
        };
        let (pts, closed) = d.path();
        host.call("sweep_clear_preview", Json::Null);
        if pts.len() < 2 {
            host.call("sweep_message", json!("touch"));
            return Reply::redraw();
        }
        self.commit(host, d.face, pts, closed)
    }
}
impl Tool for FollowMe {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn state(&self) -> Json {
        let path = self.drag.as_ref().map(Drag::path).unwrap_or_default();
        json!({"path":self.preset.as_ref().map(|x|&x.0), "hovered":self.hovered, "profile":self.drag.as_ref().map(|d| &d.face),
               "chain":self.drag.as_ref().map(|d|&d.chain), "points":path.0,"closed":path.1})
    }
    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate => {
                *self = Self::default();
                let selected = host.call("sweep_selection", Json::Null);
                self.preset = serde_json::from_value(selected.clone()).ok();
                if self.preset.is_none() {
                    host.call(
                        "sweep_message",
                        json!(if selected.is_null() {
                            "intro"
                        } else {
                            "invalid"
                        }),
                    );
                }
            }
            ToolEvent::Cancel | ToolEvent::Deactivate => {
                self.drag = None;
                if matches!(ev, ToolEvent::Deactivate) {
                    self.hovered = None;
                }
                host.call("sweep_highlight", Json::Null);
                host.call("sweep_clear_preview", Json::Null);
            }
            ToolEvent::Hover(p) => {
                let Some(d) = &mut self.drag else {
                    self.hovered =
                        serde_json::from_value(host.call("sweep_pick_face", json!(p.px))).ok();
                    host.call("sweep_highlight", json!(self.hovered));
                    return Reply::redraw();
                };
                d.last = p.px;
                if p.mods.alt {
                    let face = host.call("sweep_pick_face", json!(p.px));
                    if face.is_null() || face == json!(d.face) {
                        return Reply::ignored();
                    }
                    let ring: Vec<P3> =
                        serde_json::from_value(host.call("sweep_ring", face)).unwrap_or_default();
                    let (c, n) = serde_json::from_value(host.call("sweep_plane", json!(d.face)))
                        .unwrap_or_default();
                    let pts = orient(ring, true, c, n);
                    if d.perimeter.as_ref() == Some(&pts) {
                        return Reply::ignored();
                    }
                    d.perimeter = Some(pts);
                } else {
                    let was_alt = d.perimeter.take().is_some();
                    let next = host.call(
                        "sweep_chain",
                        json!({"px":p.px,"world":p.world,"profile":d.face,
                            "chain":d.chain.iter().map(|v|&v.0).collect::<Vec<_>>() }),
                    );
                    match serde_json::from_value(next) {
                        Ok((crossing, chain)) => {
                            if d.chain.is_empty() {
                                d.crossing = crossing;
                            }
                            d.chain = chain;
                        }
                        Err(_) if !was_alt => return Reply::ignored(),
                        _ => {}
                    }
                }
                let (pts, closed) = d.path();
                host.call("sweep_preview", json!([d.face, pts, closed]));
            }
            ToolEvent::Click(p) => {
                if self.drag.is_some() {
                    return self.finish(host);
                }
                let Some(face) = self.hovered.clone() else {
                    return Reply::ignored();
                };
                if let Some((pts, closed)) = &self.preset {
                    let (c, n) = serde_json::from_value(host.call("sweep_plane", json!(face)))
                        .unwrap_or_default();
                    return self.commit(host, face, orient(pts.clone(), *closed, c, n), *closed);
                }
                self.drag = Some(Drag {
                    face,
                    press: p.px,
                    last: p.px,
                    ..Drag::default()
                });
                host.call("sweep_highlight", Json::Null);
                host.call("sweep_message", json!("drag"));
            }
            ToolEvent::Release(_) => {
                if let Some(d) = &self.drag {
                    if !d.path().0.is_empty()
                        && (d.last[0] - d.press[0]).hypot(d.last[1] - d.press[1]) >= 6.0
                    {
                        return self.finish(host);
                    }
                }
                return Reply::ignored();
            }
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
    fn preview(&self) -> Preview {
        let mut p = Preview::default();
        if let Some(d) = &self.drag {
            let (pts, closed) = d.path();
            p.lines
                .extend(pts.windows(2).map(|w| Preview::rubber(w[0], w[1])));
            if closed && pts.len() >= 2 {
                p.lines.push(Preview::rubber(*pts.last().unwrap(), pts[0]));
            }
        }
        p
    }
}
