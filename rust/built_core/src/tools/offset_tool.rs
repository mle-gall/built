//! Offset gesture, source edge and undo-revision-checked retype.
use super::{
    add, dot, json_p3, len, scale, sub, unit, Command, Pointer, Preview, Reply, Tool, ToolEvent,
    ToolHost, ToolInfo, Value, P3,
};
use serde_json::{json, Value as Json};

static INFO: ToolInfo = ToolInfo {
    kind:"offset", name:"Offset", shortcut:"F",
    description:"Draw a copy of a face's outline at an even distance inside or outside it — walls with thickness.",
    uses_snap:false, box_select:false, vcb_label:Some("Offset"), magnetic_axis_deg:None,
    screen_axis_px:None, axis_source:false, axis_source_empty_only:false,
};

fn cross(a: P3, b: P3) -> P3 {
    glam::DVec3::from(a).cross(b.into()).into()
}

/// Slide and intersect, preserving the number and order of boundary vertices.
pub fn closed_offset(points: &[P3], normal: P3, d: f64) -> Option<Vec<P3>> {
    let n = points.len();
    if n < 3 {
        return None;
    }
    let normal = unit(normal);
    let mut lines = Vec::with_capacity(n);
    for i in 0..n {
        let edge = sub(points[(i + 1) % n], points[i]);
        if len(edge) < 1e-4 {
            return None;
        }
        let dir = unit(edge);
        lines.push((add(points[i], scale(unit(cross(normal, dir)), d)), dir));
    }
    let corners: Vec<_> = (0..n)
        .map(|i| {
            let (p, u) = lines[(i + n - 1) % n];
            let (q, v) = lines[i];
            let den = dot(cross(u, v), normal);
            if den.abs() < 1e-9 {
                q
            } else {
                add(p, scale(u, dot(cross(sub(q, p), v), normal) / den))
            }
        })
        .collect();
    (0..n)
        .all(|i| {
            let delta = sub(corners[(i + 1) % n], corners[i]);
            len(delta) >= 1e-4 && dot(lines[i].1, unit(delta)) > 0.
        })
        .then_some(corners)
}

#[derive(Clone)]
struct Outline {
    points: Vec<P3>,
    normal: P3,
    face: Option<String>,
    closed: bool,
    origin: Option<P3>,
}
impl Default for Outline {
    fn default() -> Self {
        Self {
            points: vec![],
            normal: [0.; 3],
            face: None,
            closed: true,
            origin: None,
        }
    }
}
impl Outline {
    fn from_host(data: Json) -> Option<Self> {
        let points: Vec<P3> = serde_json::from_value(data.get("points")?.clone()).ok()?;
        if points.len() < 2 {
            return None;
        }
        Some(Self {
            points,
            normal: json_p3(&data["normal"])?,
            face: data["face"].as_str().map(str::to_owned),
            closed: data["closed"].as_bool()?,
            origin: json_p3(&data["origin"]),
        })
    }
    fn cursor(&self, host: &mut dyn ToolHost, p: &Pointer, idle: bool) -> Option<P3> {
        let r = host.call("offset_ray", json!(p.px));
        let o = json_p3(r.get(0)?)?;
        let d = json_p3(r.get(1)?)?;
        let denom = dot(d, self.normal);
        if denom.abs() < 1e-9 {
            return None;
        }
        let plane = if idle {
            self.points.first().copied()
        } else {
            self.origin.or_else(|| self.points.first().copied())
        }?;
        Some(add(o, scale(d, dot(sub(plane, o), self.normal) / denom)))
    }
    fn reference(&self, cursor: Option<P3>) -> Option<(P3, P3, Option<[P3; 3]>)> {
        let n = self.points.len();
        let mut best = f64::INFINITY;
        let mut result = None;
        for i in 0..if self.closed { n } else { n.saturating_sub(1) } {
            let a = self.points[i];
            let b = self.points[(i + 1) % n];
            let edge = sub(b, a);
            let sq = dot(edge, edge);
            let foot = cursor.map(|p| {
                if sq < 1e-12 {
                    a
                } else {
                    add(a, scale(edge, (dot(sub(p, a), edge) / sq).clamp(0., 1.)))
                }
            });
            let dist = foot.zip(cursor).map_or(0., |(q, p)| len(sub(q, p)));
            if dist < best {
                best = dist;
                result = Some((
                    a,
                    unit(cross(unit(self.normal), unit(edge))),
                    foot.map(|q| [a, b, q]),
                ));
            }
        }
        result
    }
    fn segments(&self, d: f64) -> Vec<(P3, P3)> {
        if self.closed {
            let Some(points) = closed_offset(&self.points, self.normal, d) else {
                return vec![];
            };
            return (0..points.len())
                .map(|i| (points[i], points[(i + 1) % points.len()]))
                .collect();
        }
        // Store geometry is binary64; its legacy hypot argument is unused.
        crate::store::offset::outline(
            &self.points,
            self.normal,
            d,
            crate::facegeom::Hypot::LibStdCxx,
            false,
        )
    }
}
#[derive(Clone)]
struct Last {
    outline: Outline,
    sign: f64,
    revision: u64,
}

#[derive(Default)]
pub struct Offset {
    source: Outline,
    armed: bool,
    dragging: bool,
    distance: f64,
    reference: Option<[P3; 3]>,
    anchor: Option<P3>,
    inward: Option<P3>,
    last: Option<Last>,
}
impl Offset {
    fn reset(&mut self, host: &mut dyn ToolHost) {
        self.source = Outline::default();
        self.dragging = false;
        self.distance = 0.;
        self.armed = false;
        self.reference = None;
        self.anchor = None;
        self.inward = None;
        host.call("clear_offset_picks", Json::Null);
    }
    fn measure(&mut self, host: &mut dyn ToolHost, p: &Pointer) {
        if let (Some(a), Some(n)) = (self.anchor, self.inward) {
            let cursor = p
                .snap
                .as_ref()
                .map(|s| s.point)
                .or_else(|| self.source.cursor(host, p, false));
            if let Some(q) = cursor {
                self.distance = dot(sub(q, a), n);
            }
        }
    }
    fn commit(&mut self, host: &mut dyn ToolHost) -> Reply {
        let args = match &self.source.face {
            Some(key) => json!({"face":key,"distance":self.distance}),
            None => {
                json!({"points":self.source.points,"normal":self.source.normal,"closed":self.source.closed,
                          "distance":self.distance,"eye":host.eye()})
            }
        };
        let ans = host.run(&Command {
            name: "offset_outline",
            args,
        });
        if !ans.ok {
            host.call(
                "offset_refusal",
                json!([
                    ans.error,
                    self.distance,
                    self.source.points,
                    self.source.normal
                ]),
            );
        }
        self.last = ans.ok.then(|| Last {
            outline: std::mem::take(&mut self.source),
            sign: self.distance.signum(),
            revision: ans.revision,
        });
        self.reset(host);
        if ans.ok {
            Reply::committed()
        } else {
            Reply::redraw()
        }
    }
}
impl Tool for Offset {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn busy(&self) -> bool {
        self.dragging
    }
    fn state(&self) -> Json {
        json!({"distance":self.distance,"dragging":self.dragging,"armed":self.armed,
        "chain":self.source.face.is_none() && !self.source.points.is_empty(),"closed":self.source.closed,
        "ref_point":self.anchor,"ref_inward":self.inward,"reference":self.reference})
    }
    fn set(&mut self, key: &str, value: &Json) -> bool {
        match key {
            "distance" => self.distance = value.as_f64().unwrap_or(0.),
            "dragging" => self.dragging = value == true,
            "ref_point" => self.anchor = json_p3(value),
            "ref_inward" => self.inward = json_p3(value),
            "points" => {
                self.source.points = serde_json::from_value(value.clone()).unwrap_or_default()
            }
            "normal" => self.source.normal = json_p3(value).unwrap_or([0.; 3]),
            "face" => self.source.face = value.as_str().map(str::to_owned),
            "closed" => self.source.closed = value == true,
            _ => return false,
        }
        true
    }
    fn query(&self, what: &str, _: &Json) -> Json {
        match what {
            "points" => json!(self.source.points),
            "normal" => json!(self.source.normal),
            "label" if self.dragging => self
                .anchor
                .map(|a| {
                    json!([
                        self.distance.abs(),
                        add(
                            self.reference.map_or(a, |r| r[2]),
                            scale(self.inward.unwrap_or([0.; 3]), self.distance * 0.5)
                        )
                    ])
                })
                .unwrap_or(Json::Null),
            _ => Json::Null,
        }
    }
    fn preview(&self) -> Preview {
        let mut out = Preview::default();
        if self.dragging && self.distance.abs() >= 1e-6 {
            out.lines = self
                .source
                .segments(self.distance)
                .into_iter()
                .map(|(a, b)| Preview::rubber(a, b))
                .collect();
        }
        out
    }
    fn event(&mut self, event: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match event {
            ToolEvent::Activate => {
                self.reset(host);
                self.last = None;
                if let Some(run) = Outline::from_host(host.call("selected_outline", Json::Null)) {
                    self.source = run;
                    self.armed = true;
                }
            }
            ToolEvent::Cancel | ToolEvent::Deactivate => {
                host.call("clear_offset_hover", Json::Null);
                self.reset(host);
                self.last = None;
            }
            ToolEvent::Hover(p) if !self.dragging => {
                if !self.armed {
                    self.source = Outline::from_host(host.call("hover_outline", json!(p.px)))
                        .unwrap_or_default();
                }
                self.reference = if self.source.points.is_empty() {
                    None
                } else {
                    self.source
                        .cursor(host, p, true)
                        .and_then(|c| self.source.reference(Some(c)))
                        .and_then(|(_, _, r)| r)
                };
            }
            ToolEvent::Click(p) if !self.dragging => {
                self.last = None;
                if !self.armed {
                    let Some(src) = Outline::from_host(host.call("picked_outline", Json::Null))
                    else {
                        return Reply::ignored();
                    };
                    self.source = src;
                }
                let cursor = self.source.cursor(host, p, false);
                let Some((a, n, r)) = self.source.reference(cursor) else {
                    return Reply::ignored();
                };
                self.anchor = Some(a);
                self.inward = Some(n);
                self.reference = r;
                self.distance = 0.;
                self.dragging = true;
                self.armed = false;
                host.call("clear_offset_hover", Json::Null);
            }
            ToolEvent::Hover(p) => self.measure(host, p),
            ToolEvent::Click(p) => {
                self.measure(host, p);
                if self.distance.abs() < 1e-6 {
                    return Reply::ignored();
                }
                return self.commit(host);
            }
            ToolEvent::Key { key, .. } if key == "CommitOffset" => return self.commit(host),
            ToolEvent::Value(Value::Length(v)) if *v != 0. => {
                if self.dragging && !self.source.points.is_empty() {
                    self.distance = if self.distance < 0. { -*v } else { *v };
                    return self.commit(host);
                }
                let Some(mut last) = self.last.take() else {
                    return Reply::ignored();
                };
                if last.revision != host.revision()
                    || !host
                        .run(&Command {
                            name: "undo",
                            args: json!({}),
                        })
                        .ok
                {
                    return Reply::ignored();
                }
                self.source = last.outline.clone();
                self.distance = last.sign * v;
                let result = self.commit(host);
                if !result.committed {
                    let redo = host.run(&Command {
                        name: "redo",
                        args: json!({}),
                    });
                    if redo.ok {
                        last.revision = redo.revision;
                        self.last = Some(last);
                    }
                }
                return result;
            }
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
}
