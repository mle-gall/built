//! Paste placement. Clipboard templates and the renderer's group preview are
//! host resources; the anchor, cursor offset and stamp gesture live here.
use super::{add, json_p3, sub, Command, Preview, Reply, Tool, ToolEvent, ToolHost, ToolInfo, P3};
use serde_json::{json, Value as Json};

static INFO: ToolInfo = ToolInfo {
    kind: "paste",
    name: "Paste",
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
#[derive(Default)]
pub struct Paste {
    reference: Option<P3>,
    offset: P3,
    lines: Vec<[P3; 2]>,
}
impl Paste {
    fn stamp(&self, offset: P3, host: &mut dyn ToolHost) -> Reply {
        if self.reference.is_none()
            || !offset.iter().all(|v| v.is_finite())
            || host.call("can_stamp", Json::Null) != true
        {
            return Reply::ignored();
        }
        host.call("end_paste_preview", Json::Null);
        if host
            .run(&Command {
                name: "paste_clipboard",
                args: json!({"offset":offset}),
            })
            .ok
        {
            Reply::committed()
        } else {
            Reply::ignored()
        }
    }
}
impl Tool for Paste {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn state(&self) -> Json {
        json!({"start_point":self.reference,"offset":self.offset})
    }
    fn set(&mut self, key: &str, v: &Json) -> bool {
        match key {
            "clipboard" => {
                self.reference = json_p3(&v["ref"]);
                self.lines.clear();
                if let Some(loops) = v["rings"].as_array() {
                    for ring in loops {
                        let Ok(pts) = serde_json::from_value::<Vec<P3>>(ring.clone()) else {
                            continue;
                        };
                        self.lines.extend(
                            pts.iter()
                                .zip(pts.iter().cycle().skip(1))
                                .map(|(a, b)| [*a, *b]),
                        );
                    }
                }
                if let Some(edges) = v["edges"].as_array() {
                    self.lines.extend(
                        edges
                            .iter()
                            .filter_map(|v| serde_json::from_value::<[P3; 2]>(v.clone()).ok()),
                    );
                }
            }
            "offset" => {
                let Some(off) = json_p3(v) else { return false };
                self.offset = off;
            }
            _ => return false,
        }
        true
    }
    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate => {
                self.offset = [0.; 3];
                host.call("begin_paste_preview", Json::Null);
            }
            ToolEvent::Deactivate | ToolEvent::Cancel => {
                host.call("end_paste_preview", Json::Null);
                self.reference = None;
                self.lines.clear();
            }
            ToolEvent::Hover(p) => {
                let (Some(at), Some(reference)) = (p.world, self.reference) else {
                    return Reply::ignored();
                };
                self.offset = sub(at, reference);
                host.call("pose_paste_preview", json!(self.offset));
            }
            ToolEvent::Click(p) => {
                let (Some(at), Some(reference)) = (p.world, self.reference) else {
                    return Reply::ignored();
                };
                return self.stamp(sub(at, reference), host);
            }
            ToolEvent::PasteAt(offset) => return self.stamp(*offset, host),
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
    fn preview(&self) -> Preview {
        Preview {
            lines: self
                .lines
                .iter()
                .map(|[a, b]| Preview::rubber(add(*a, self.offset), add(*b, self.offset)))
                .collect(),
            ..Default::default()
        }
    }
}
