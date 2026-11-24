//! A stroke latches hide/erase at press and applies one history command at release.
use super::{Command, Reply, Tool, ToolEvent, ToolHost, ToolInfo};
use serde_json::{json, Value as Json};
use std::collections::BTreeSet;
static INFO: ToolInfo = ToolInfo::basic(
    "eraser",
    "Eraser",
    "E",
    "Click or drag over edges to erase them, together with the faces they bound.",
    false,
    None,
);
#[derive(Default)]
pub struct Eraser {
    stroke: bool,
    hide: bool,
    marked: BTreeSet<u64>,
}
impl Tool for Eraser {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn state(&self) -> Json {
        json!({"stroke":self.stroke,"hide":self.hide,"marked":self.marked})
    }
    fn set(&mut self, k: &str, v: &Json) -> bool {
        match k {
            "hide" => self.hide = v.as_bool().unwrap_or(false),
            "stroke" => self.stroke = v.as_bool().unwrap_or(false),
            "marked" => self.marked = serde_json::from_value(v.clone()).unwrap_or_default(),
            _ => return false,
        }
        true
    }
    fn event(&mut self, ev: &ToolEvent, h: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Click(p) | ToolEvent::Hover(p) => {
                if matches!(ev, ToolEvent::Click(_)) {
                    self.stroke = true;
                    self.hide = p.mods.shift;
                }
                let targets = if !self.stroke {
                    &["glyph", "edge"][..]
                } else if self.hide {
                    &["edge", "group"][..]
                } else {
                    &[
                        "glyph",
                        "edge",
                        "group",
                        "guide",
                        "dimension",
                        "label",
                        "path",
                    ][..]
                };
                let mut picked = vec![];
                for target in targets {
                    picked = serde_json::from_value::<Vec<u64>>(
                        h.call("eraser_pick", json!([target, p.px, self.stroke])),
                    )
                    .unwrap_or_default();
                    if !picked.is_empty() {
                        break;
                    }
                }
                if self.stroke {
                    self.marked.extend(picked);
                } else {
                    h.call("eraser_hover", json!(picked.first()));
                }
            }
            ToolEvent::Activate | ToolEvent::Deactivate | ToolEvent::Cancel => {
                *self = Self::default()
            }
            ToolEvent::Release(_) if self.stroke => {
                self.stroke = false;
                let ids = std::mem::take(&mut self.marked);
                if !ids.is_empty() {
                    let mut args = h.call("eraser_targets", json!(ids));
                    args["hide"] = json!(self.hide);
                    if h.run(&Command {
                        name: "erase_stroke",
                        args,
                    })
                    .ok
                    {
                        return Reply::committed();
                    }
                }
            }
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
}
