//! Place a detached component with its front on a picked surface.
use super::{
    add, json_p3, scale, sub, Command, Preview, Reply, Tool, ToolEvent, ToolHost, ToolInfo, P3,
};
use glam::DVec3;
use serde_json::{json, Value as Json};
static INFO: ToolInfo = ToolInfo::basic("place_group", "Place component", "", "", true, None);
fn basis(n: Option<P3>) -> [P3; 3] {
    let Some(n) = n else {
        return [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    };
    let n = DVec3::from(n).normalize_or_zero();
    let hint = if n.z.abs() > 0.99 { DVec3::Y } else { DVec3::Z };
    let r = hint.cross(n).try_normalize().unwrap_or(DVec3::X);
    [r.into(), (-n).into(), n.cross(r).normalize_or_zero().into()]
}
fn rotate(b: [P3; 3], p: P3) -> P3 {
    add(add(scale(b[0], p[0]), scale(b[1], p[1])), scale(b[2], p[2]))
}
#[derive(Default)]
pub struct PlaceGroup {
    template: Option<String>,
    anchor: P3,
    offset: P3,
    align: bool,
    normal: Option<P3>,
    segments: Vec<[P3; 2]>,
}
impl Tool for PlaceGroup {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn state(&self) -> Json {
        json!({"normal":self.normal,"offset":self.offset})
    }
    fn set(&mut self, k: &str, v: &Json) -> bool {
        match k {
            "load" => {
                let Ok((uid, anchor, align, lines)) = serde_json::from_value(v.clone()) else {
                    return false;
                };
                self.template = Some(uid);
                self.anchor = anchor;
                self.align = align;
                self.segments = lines;
            }
            "normal" => self.normal = json_p3(v),
            _ => return false,
        }
        true
    }
    fn query(&self, k: &str, v: &Json) -> Json {
        if k == "basis" {
            json!(basis(json_p3(v)))
        } else {
            Json::Null
        }
    }
    fn event(&mut self, ev: &ToolEvent, h: &mut dyn ToolHost) -> Reply {
        if matches!(ev, ToolEvent::Deactivate | ToolEvent::Cancel) {
            self.template = None;
            self.segments.clear();
            return Reply::redraw();
        }
        let Some(uid) = &self.template else {
            return Reply::ignored();
        };
        let (world, drop) = match ev {
            ToolEvent::Hover(p) | ToolEvent::Click(p) => {
                if self.align {
                    self.normal = json_p3(&h.call("placement_normal", json!(p.px)));
                }
                (p.world, matches!(ev, ToolEvent::Click(_)))
            }
            ToolEvent::PasteAt(p) => (Some(*p), true),
            _ => return Reply::ignored(),
        };
        let Some(at) = world else {
            return Reply::ignored();
        };
        let b = basis(self.normal);
        self.offset = sub(at, rotate(b, self.anchor));
        if drop {
            let ok = h
                .run(&Command {
                    name: "place_group",
                    args: json!({"template":uid,"basis":b,"offset":self.offset,"point":at}),
                })
                .ok;
            if ok {
                self.template = None;
                self.segments.clear();
                return Reply::committed();
            }
        }
        Reply::redraw()
    }
    fn preview(&self) -> Preview {
        let mut p = Preview::default();
        if self.template.is_some() {
            let b = basis(self.normal);
            let map = |v: P3| add(rotate(b, add(self.anchor, v)), self.offset);
            p.lines.extend(
                self.segments
                    .iter()
                    .map(|[a, b]| Preview::rubber(map(*a), map(*b))),
            );
        }
        p
    }
}
