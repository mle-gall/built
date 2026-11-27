//! Line: click points to draw edges; a loop that closes, or a segment that
//! cuts a face, ends the chain.
//!
//! - The first click starts a chain; each next click commits a segment
//!   (`add_edge`) and chains on from its end.
//! - A segment that makes a face (net face count up) or lands on the
//!   chain's first point (snap kind `close`) ends the chain (#496).
//! - Clicking the start itself draws nothing (#34).
//! - A typed length runs along the rubber band, negative the other way
//!   (#58); `dx;dy;dz` is a delta from the start.
//! - Hot retype (#414): right after a segment, while the cursor still sits
//!   on its end (or once the chain ended), a typed value undoes it and
//!   draws it again from the same start; one undo step however often.
//! - ⌘Z mid-chain undoes the last segment and steps the chain back (#175).

use super::{add, json_p3, len, opt_p3_json, p3_json, scale, sub, unit, Answer, Command, Preview, Reply, Tool,
            ToolEvent, ToolHost, ToolInfo, Value, P3};
use super::preview::ValueAnchor;

static INFO: ToolInfo = ToolInfo {
    kind: "line",
    name: "Line",
    shortcut: "L",
    description: "Draw edges point by point; closing a loop makes a face.",
    uses_snap: true,
    box_select: false,
    vcb_label: Some("Length"),
    // The axis inference magnetises within the soft inference angle
    // (#31): the point returned is ON the axis the label names.
    magnetic_axis_deg: Some(3.0),
    // Axes the work plane cannot hold, found in pixels (#31).
    screen_axis_px: Some(9.0),
    axis_source: true,
    axis_source_empty_only: true,
};

const EPS: f64 = 1e-9;

#[derive(Clone, Debug, PartialEq)]
struct Chain {
    start: P3,
    first: P3,
    vertices: Vec<P3>,
}

/// The segment just drawn, for the hot retype.
#[derive(Clone, Debug, PartialEq)]
struct Retype {
    start: P3,
    end: P3,
    /// The chain as it was before the segment.
    chain: Option<Chain>,
    work_plane: Option<(P3, P3)>,
    ended: bool,
    /// The history token right after the segment: still the top of the
    /// undo stack while it matches.
    token: u64,
}

#[derive(Default)]
pub struct LineTool {
    chain: Option<Chain>,
    hover: Option<P3>,
    work_plane: Option<(P3, P3)>,
    last: Option<Retype>,
    /// Alt: a straight chord instead of a line over a curved face.
    chord: bool,
    /// The previewed line over a curved face, start to hover.
    surface: Option<Vec<P3>>,
}

/// The line over the curved face under `a`, when there is one (the host
/// answers from the exact support; corners on the surface).
fn surface_path(a: P3, b: P3, host: &mut dyn ToolHost) -> Option<Vec<P3>> {
    surface_line(a, b, host).map(|(path, _)| path)
}

/// The polyline to commit and the exact curve to preview.
fn surface_line(a: P3, b: P3, host: &mut dyn ToolHost) -> Option<(Vec<P3>, Vec<P3>)> {
    let v = host.call("surface_path", serde_json::json!({"a": p3_json(a), "b": p3_json(b)}));
    let points = |key: &str| -> Vec<P3> { v[key].as_array().map(|a| a.iter().filter_map(json_p3).collect()).unwrap_or_default() };
    let (path, curve) = (points("path"), points("curve"));
    (path.len() >= 2).then_some((path, curve))
}

fn same(a: P3, b: P3) -> bool { len(sub(a, b)) < EPS }

impl LineTool {
    fn reset(&mut self) {
        self.chain = None;
        self.work_plane = None;
        self.surface = None;
    }

    fn start(&self) -> Option<P3> { self.chain.as_ref().map(|c| c.start) }

    /// `offset`: a typed segment's exact delta, so the end is the start's
    /// stored doubles plus it (the host resolves the start).
    fn add_edge(&self, a: P3, b: P3, offset: Option<P3>, host: &mut dyn ToolHost) -> Answer {
        if offset.is_none() && !self.chord {
            if let Some(path) = surface_path(a, b, host) {
                return Self::add_path(&path, host);
            }
        }
        let mut args = serde_json::json!({"a": p3_json(a), "b": p3_json(b)});
        if self.chord {
            args["chord"] = serde_json::Value::Bool(true);
        }
        if let Some(d) = offset {
            args["offset"] = p3_json(d);
        }
        if let Some(eye) = host.eye() {
            args["eye"] = p3_json(eye);
        }
        host.run(&Command { name: "add_edge", args })
    }

    /// A line over a curved face: its polyline as one step.
    fn add_path(path: &[P3], host: &mut dyn ToolHost) -> Answer {
        let segments: Vec<_> = path.windows(2).map(|w| [p3_json(w[0]), p3_json(w[1])]).collect();
        host.run(&Command { name: "add_edges", args: serde_json::json!({"segments": segments}) })
    }

    fn retype_state(&self, start: P3, end: P3) -> Retype {
        Retype { start, end, chain: self.chain.clone(), work_plane: self.work_plane, ended: false, token: 0 }
    }

    /// Commit `start`–`end` and move the chain on. False when the history
    /// refused the segment.
    fn draw_segment(&mut self, start: P3, end: P3, offset: P3, mut last: Retype, ended: bool,
                    host: &mut dyn ToolHost) -> bool {
        let answer = self.add_edge(start, end, Some(offset), host);
        if !answer.ok {
            return false;
        }
        let ended = ended || answer.face_delta > 0;
        last.ended = ended;
        last.token = answer.revision;
        self.last = Some(last);
        if ended {
            self.reset();
        } else if let Some(c) = self.chain.as_mut() {
            c.vertices.push(end);
            c.start = end;
            self.hover = Some(end);
        }
        true
    }

    fn click(&mut self, world: P3, kind: Option<&str>, host: &mut dyn ToolHost) -> Reply {
        self.last = None; // a click ends the retype window
        let Some(start) = self.start() else {
            self.chain = Some(Chain { start: world, first: world, vertices: vec![world] });
            return Reply::redraw();
        };
        if same(world, start) {
            return Reply::redraw();
        }
        // Over a curved face the line follows the surface; its ends sit
        // on the facets, and the chain goes on from where it ended.
        let path = if self.chord { None } else { surface_path(start, world, host) };
        let world = path.as_ref().and_then(|p| p.last().copied()).unwrap_or(world);
        let last = self.retype_state(start, world);
        let answer = match &path {
            Some(p) => Self::add_path(p, host),
            None => self.add_edge(start, world, None, host),
        };
        let ended = kind == Some("close") || answer.face_delta > 0;
        self.last = answer.ok.then(|| Retype { ended, token: answer.revision, ..last });
        if ended {
            self.reset();
        } else if let Some(c) = self.chain.as_mut() {
            c.vertices.push(world);
            c.start = world;
        }
        Reply { consumed: true, redraw: true, committed: answer.ok }
    }

    fn retype_ready(&mut self, host: &mut dyn ToolHost) -> bool {
        let Some(last) = self.last.as_ref() else { return false };
        if host.revision() != last.token {
            self.last = None;
            return false;
        }
        if last.ended {
            return self.chain.is_none();
        }
        self.start().is_some_and(|s| same(s, last.end)) && self.hover.is_none_or(|h| same(h, last.end))
    }

    /// None when the value means nothing here (not consumed); else whether
    /// the new segment was drawn.
    fn retype(&mut self, value: &Value, host: &mut dyn ToolHost) -> Option<bool> {
        let last = self.last.clone().expect("retype_ready");
        let start = last.start;
        let offset = match *value {
            Value::Triple(dx, dy, dz) => [dx, dy, dz],
            Value::Length(v) => {
                let delta = sub(last.end, start);
                if v.abs() < EPS || len(delta) < EPS {
                    return None;
                }
                scale(unit(delta), v)
            }
            _ => return None,
        };
        let end = add(start, offset);
        if same(end, start) {
            return None;
        }
        if !host.run(&Command { name: "undo", args: serde_json::json!({}) }).ok {
            return None;
        }
        self.chain = last.chain.clone();
        self.work_plane = last.work_plane;
        let again = Retype { end, ..last.clone() };
        let drawn = self.draw_segment(start, end, offset, again, last.ended, host);
        if !drawn {
            // Refused: the segment already drawn comes back instead of
            // vanishing with the undo, and the window stays open.
            host.run(&Command { name: "redo", args: serde_json::json!({}) });
            if last.ended {
                self.reset();
            } else if let Some(c) = self.chain.as_mut() {
                c.vertices.push(last.end);
                c.start = last.end;
                self.hover = Some(last.end);
            }
            self.last = Some(Retype { token: host.revision(), ..last });
        }
        Some(drawn)
    }

    fn value(&mut self, value: &Value, host: &mut dyn ToolHost) -> Reply {
        if self.retype_ready(host) {
            return match self.retype(value, host) {
                None => Reply::ignored(),
                Some(drawn) => Reply { consumed: true, redraw: true, committed: drawn },
            };
        }
        let Some(start) = self.start() else { return Reply::ignored() };
        let offset = match *value {
            Value::Triple(dx, dy, dz) => [dx, dy, dz],
            Value::Length(v) => {
                let Some(hover) = self.hover else { return Reply::ignored() };
                if v.abs() < EPS {
                    return Reply::ignored();
                }
                let delta = sub(hover, start);
                if len(delta) < EPS {
                    return Reply::ignored();
                }
                scale(unit(delta), v)
            }
            // A pair is a rectangle's width × height, not a line.
            _ => return Reply::ignored(),
        };
        let end = add(start, offset);
        let last = self.retype_state(start, end);
        let drawn = self.draw_segment(start, end, offset, last, false, host);
        Reply { consumed: true, redraw: true, committed: drawn }
    }

    fn undo(&mut self, host: &mut dyn ToolHost) -> Reply {
        self.last = None; // the segment it would retype is gone
        let Some(chain) = self.chain.as_mut() else { return Reply::ignored() };
        if chain.vertices.len() > 1 {
            if !host.run(&Command { name: "undo", args: serde_json::json!({}) }).ok {
                self.reset();
                return Reply::redraw();
            }
            let chain = self.chain.as_mut().unwrap();
            chain.vertices.pop();
            chain.start = *chain.vertices.last().unwrap();
            if chain.vertices.len() == 1 {
                chain.first = chain.start;
            }
        } else {
            self.reset();
        }
        Reply::redraw()
    }
}

impl Tool for LineTool {
    fn info(&self) -> &'static ToolInfo { &INFO }

    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate => {
                self.reset();
                self.last = None;
                Reply::redraw()
            }
            ToolEvent::Deactivate => {
                self.reset();
                self.last = None;
                self.hover = None;
                Reply::redraw()
            }
            ToolEvent::Cancel => {
                self.reset();
                self.last = None;
                Reply::redraw()
            }
            ToolEvent::Hover(p) => {
                if let Some(w) = p.world {
                    self.hover = Some(w);
                }
                self.surface = match (self.start(), self.hover) {
                    (Some(a), Some(b)) if !self.chord && !same(a, b) => {
                        surface_line(a, b, host).map(|(path, curve)| if curve.len() >= 2 { curve } else { path })
                    }
                    _ => None,
                };
                Reply::redraw()
            }
            ToolEvent::KeyRelease { key, tapped: true } if key == "Alt" => {
                self.chord = !self.chord;
                self.surface = None;
                host.message(
                    if self.chord {
                        "Straight chord across curved faces (Alt: follow the surface)"
                    } else {
                        "Lines follow curved faces (Alt: straight chord)"
                    },
                    &[],
                );
                Reply::redraw()
            }
            ToolEvent::Click(p) | ToolEvent::DoubleClick(p) | ToolEvent::TripleClick(p) => match p.world {
                Some(w) => self.click(w, p.snap.as_ref().map(|s| s.kind.as_str()), host),
                None => Reply::ignored(),
            },
            ToolEvent::Value(v) => self.value(v, host),
            ToolEvent::Undo => self.undo(host),
            ToolEvent::WorkPlane(plane) => {
                self.work_plane = *plane;
                Reply::ignored()
            }
            _ => Reply::ignored(),
        }
    }

    fn preview(&self) -> Preview {
        let mut out = Preview::default();
        if let (Some(start), Some(hover)) = (self.start(), self.hover) {
            match &self.surface {
                Some(path) => out.lines.extend(path.windows(2).map(|w| Preview::rubber(w[0], w[1]))),
                None => out.lines.push(Preview::rubber(start, hover)),
            }
            out.value = Some(ValueAnchor { at: scale(add(start, hover), 0.5), label: "Length" });
        }
        out
    }

    fn busy(&self) -> bool { self.chain.is_some() }

    fn state(&self) -> serde_json::Value {
        let plane = self.work_plane.map(|(p, n)| serde_json::json!([p3_json(p), p3_json(n)]));
        serde_json::json!({
            "start_point": opt_p3_json(self.start()),
            "hover_point": opt_p3_json(self.hover),
            "chain_first_point": opt_p3_json(self.chain.as_ref().map(|c| c.first)),
            "chain_vertices": self.chain.as_ref().map(|c| c.vertices.iter().map(|v| p3_json(*v)).collect::<Vec<_>>())
                .unwrap_or_default(),
            "work_plane": plane,
            "retype": self.last.is_some(),
            "chord": self.chord,
            "surface_path": self.surface.as_ref().map(|p| p.iter().map(|v| p3_json(*v)).collect::<Vec<_>>()),
        })
    }

    fn set(&mut self, key: &str, value: &serde_json::Value) -> bool {
        let point = json_p3(value);
        match key {
            "start_point" => match (point, self.chain.as_mut()) {
                (None, _) => self.chain = None,
                (Some(p), Some(c)) => c.start = p,
                (Some(p), None) => self.chain = Some(Chain { start: p, first: p, vertices: vec![p] }),
            },
            "chain_first_point" => {
                if let (Some(p), Some(c)) = (point, self.chain.as_mut()) {
                    c.first = p;
                }
            }
            "chain_vertices" => {
                if let Some(c) = self.chain.as_mut() {
                    c.vertices = value.as_array().map(|a| a.iter().filter_map(json_p3).collect()).unwrap_or_default();
                }
            }
            "hover_point" => self.hover = point,
            "chord" => self.chord = value.as_bool().unwrap_or(false),
            "work_plane" => {
                self.work_plane = value.as_array().and_then(|a| Some((json_p3(a.first()?)?, json_p3(a.get(1)?)?)));
            }
            _ => return false,
        }
        true
    }
}
