//! Endpoint capture and label/dimension placement.
use super::{
    add, dot, json_p3, len, scale, sub, unit, Command, Preview, Reply, Tool, ToolEvent, ToolHost,
    ToolInfo, P3,
};
use serde_json::{json, Value as Json};
static TEXT: ToolInfo = ToolInfo::basic(
    "text",
    "Text",
    "X",
    "Place a text with a leader pointing at the model; it suggests the length, area or coordinates of what you click.",
    true,
    None,
);
static DIM: ToolInfo = ToolInfo::basic(
    "dimension",
    "Dimension",
    "D",
    "Place a dimension that shows the distance between two points.",
    true,
    None,
);
#[derive(Default)]
pub struct Text {
    a: Option<P3>,
    hover: Option<P3>,
    attachment: Json,
    description: String,
}
impl Tool for Text {
    fn info(&self) -> &'static ToolInfo {
        &TEXT
    }
    fn state(&self) -> Json {
        json!({"start_point":self.a,"hover_point":self.hover,"description":self.description})
    }
    fn busy(&self) -> bool {
        self.a.is_some()
    }
    fn event(&mut self, ev: &ToolEvent, h: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate | ToolEvent::Cancel | ToolEvent::Deactivate => {
                self.a = None;
                self.attachment = Json::Null;
                self.description.clear();
                if matches!(ev, ToolEvent::Deactivate) {
                    self.hover = None;
                }
            }
            ToolEvent::Hover(p) => self.hover = p.world,
            ToolEvent::Click(p) => {
                let Some(b) = p.world else {
                    return Reply::ignored();
                };
                if let Some(a) = self.a.take() {
                    h.call("text_prompt",json!({"anchor":a,"offset":sub(b,a),"attachment":self.attachment.take(),"text":std::mem::take(&mut self.description)}));
                } else {
                    self.a = Some(b);
                    self.attachment = h.call("annotation_pick", json!(b));
                    self.description = h
                        .call("text_default", Json::Null)
                        .as_str()
                        .unwrap_or_default()
                        .into();
                }
            }
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
    fn preview(&self) -> Preview {
        let mut p = Preview::default();
        if let Some((a, b)) = self.a.zip(self.hover) {
            p.lines.push(Preview::rubber(a, b));
        }
        p
    }
}
const AXES: [P3; 3] = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
const NAMES: [&str; 3] = ["x", "y", "z"];
type Placement = (P3, Option<String>);
fn extension(d: P3, u: P3) -> Option<P3> {
    let e = sub(d, scale(u, dot(d, u)));
    (len(e) >= 1e-6).then(|| unit(e))
}
fn placement(a: P3, b: P3, c: P3, lock: Option<&str>) -> Placement {
    let ab = sub(b, a);
    let d = unit(ab);
    let ac = sub(c, a);
    let aligned = sub(ac, scale(d, dot(ac, d)));
    let lo = 15f64.to_radians().sin();
    if let Some(i) = NAMES.iter().position(|n| Some(*n) == lock) {
        let l = AXES[i];
        let along = scale(l, dot(ac, l));
        for (n, u) in NAMES.iter().zip(AXES) {
            if dot(d, u).abs() >= lo && extension(d, u).is_some_and(|e| dot(e, l).abs() > 0.999) {
                return (along, Some(n.to_string()));
            }
        }
        return (
            if len(sub(l, scale(d, dot(l, d)))) < 0.25 {
                aligned
            } else {
                along
            },
            None,
        );
    }
    let w = sub(c, scale(add(a, b), 0.5));
    if dot(w, d).abs() <= 20f64.to_radians().tan() * len(sub(w, scale(d, dot(w, d)))) {
        return (aligned, None);
    }
    let mut candidate = None;
    for (n, u) in NAMES.iter().zip(AXES) {
        let alignment = dot(d, u).abs();
        if alignment < lo || alignment > 15f64.to_radians().cos() {
            continue;
        }
        let Some(e) = extension(d, u) else { continue };
        let t = dot(ac, e);
        let span = dot(ab, e);
        if (t - span.max(0.)).max(span.min(0.) - t) > 1e-9 {
            if candidate.is_some() {
                return (aligned, None);
            }
            candidate = Some((scale(e, t), Some(n.to_string())));
        }
    }
    candidate.unwrap_or((aligned, None))
}
#[derive(Default)]
pub struct Dimension {
    a: Option<P3>,
    b: Option<P3>,
    hover: Option<P3>,
    attach: [Json; 2],
    free: Option<Placement>,
    hold: Option<Placement>,
    proposal: Option<Placement>,
}
impl Dimension {
    fn reset(&mut self) {
        self.a = None;
        self.b = None;
        self.attach = Default::default();
        self.free = None;
        self.hold = None;
        self.proposal = None;
    }
    fn propose(&mut self, c: P3, lock: Option<&str>, shift: bool) -> Option<Placement> {
        let (a, b) = (self.a?, self.b?);
        let mut result = placement(a, b, c, lock);
        if lock.is_none() {
            if shift {
                if self.hold.is_none() {
                    let (v, ax) = self.free.clone().unwrap_or_else(|| result.clone());
                    if len(v) > 1e-9 {
                        self.hold = Some((unit(v), ax));
                    }
                }
                if let Some((d, ax)) = &self.hold {
                    result = (scale(*d, dot(sub(c, a), *d)), ax.clone());
                }
            } else {
                self.hold = None;
            }
        }
        self.proposal = Some(result.clone());
        Some(result)
    }
}
impl Tool for Dimension {
    fn info(&self) -> &'static ToolInfo {
        &DIM
    }
    fn busy(&self) -> bool {
        self.a.is_some()
    }
    fn state(&self) -> Json {
        json!({"a":self.a,"b":self.b,"hover_point":self.hover,"proposal":self.proposal})
    }
    fn set(&mut self, k: &str, v: &Json) -> bool {
        match k {
            "a" => self.a = json_p3(v),
            "b" => self.b = json_p3(v),
            "hover_point" => self.hover = json_p3(v),
            "free" => self.free = serde_json::from_value(v.clone()).ok(),
            "propose" => {
                let Ok((c, lock, shift)) =
                    serde_json::from_value::<(P3, Option<String>, bool)>(v.clone())
                else {
                    return false;
                };
                self.propose(c, lock.as_deref(), shift);
            }
            _ => return false,
        }
        true
    }
    fn event(&mut self, ev: &ToolEvent, h: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate | ToolEvent::Deactivate | ToolEvent::Cancel => {
                self.reset();
                if matches!(ev, ToolEvent::Deactivate) {
                    self.hover = None;
                }
            }
            ToolEvent::Hover(p) => {
                self.hover = p.world;
                if let (Some(a), Some(b), Some(c)) = (self.a, self.b, p.world) {
                    if !p.mods.shift {
                        self.free = Some(placement(a, b, c, None));
                    }
                }
            }
            ToolEvent::Click(p) => {
                let Some(c) = p.world else {
                    return Reply::ignored();
                };
                match (self.a, self.b) {
                    (None, _) => {
                        if let Ok((a, b, at)) = serde_json::from_value::<(P3, P3, [Json; 2])>(
                            h.call("dimension_edge", Json::Null),
                        ) {
                            if len(sub(b, a)) >= 1e-6 {
                                self.a = Some(a);
                                self.b = Some(b);
                                self.attach = at;
                                return Reply::redraw();
                            }
                        }
                        self.a = Some(c);
                        self.attach[0] = h.call("annotation_pick", json!(c));
                    }
                    (Some(a), None) => {
                        if len(sub(c, a)) >= 1e-6 {
                            self.b = Some(c);
                            self.attach[1] = h.call("annotation_pick", json!(c));
                            h.call("dimension_unlock", Json::Null);
                        }
                    }
                    (Some(a), Some(b)) => {
                        let lock = h.call("dimension_lock", Json::Null);
                        let (off, axis) = self.propose(c, lock.as_str(), p.mods.shift).unwrap();
                        let ok=h.run(&Command{name:"add_dimension",args:json!({"a":a,"b":b,"offset":off,"axis":axis,"attachments":self.attach})}).ok;
                        if ok {
                            self.reset();
                            return Reply::committed();
                        }
                    }
                }
            }
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
}
