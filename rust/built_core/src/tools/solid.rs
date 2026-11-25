//! Ordered boolean-tool picks, selection shortcuts and union chaining.
use super::{Command, Reply, Tool, ToolEvent, ToolHost, ToolInfo};
use serde_json::{json, Value as Json};

const TOOLS: [ToolInfo; 6] = [
    ToolInfo::basic(
        "outer_shell",
        "Outer Shell",
        "",
        "Merge the selected solids into one, keeping only their outer skin.",
        false,
        None,
    ),
    ToolInfo::basic(
        "solid_union",
        "Union",
        "",
        "Join solids into a single solid.",
        false,
        None,
    ),
    ToolInfo::basic(
        "solid_subtract",
        "Subtract",
        "",
        "Cut the first solid clicked out of the second, and remove the first.",
        false,
        None,
    ),
    ToolInfo::basic(
        "solid_trim",
        "Trim",
        "",
        "Cut the first solid clicked out of the second, and keep the first.",
        false,
        None,
    ),
    ToolInfo::basic(
        "solid_intersect",
        "Intersect",
        "",
        "Keep only the part where the solids overlap.",
        false,
        None,
    ),
    ToolInfo::basic(
        "solid_split",
        "Split",
        "",
        "Split two overlapping solids into their separate parts and their common part.",
        false,
        None,
    ),
];
pub struct Solid {
    info: &'static ToolInfo,
    first: Option<String>,
    badge: &'static str,
}
impl Solid {
    pub fn new(kind: &str) -> Option<Self> {
        Some(Self {
            info: TOOLS.iter().find(|t| t.kind == kind)?,
            first: None,
            badge: "no",
        })
    }
    fn op(&self) -> &'static str {
        self.info.kind.trim_start_matches("solid_")
    }
    fn execute(&self, host: &mut dyn ToolHost, groups: Vec<String>) -> Option<String> {
        let answer = host.run(&Command {
            name: "solid_operation",
            args: json!({"operation":self.op(),"placements":groups}),
        });
        if !answer.ok {
            host.call("solid_error", json!(answer.error));
            return None;
        }
        serde_json::from_value(host.call("solid_result", Json::Null)).ok()
    }
}
impl Tool for Solid {
    fn info(&self) -> &'static ToolInfo {
        self.info
    }
    fn busy(&self) -> bool {
        self.first.is_some()
    }
    fn state(&self) -> Json {
        json!({"first":self.first,"badge":self.badge})
    }
    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate => {
                self.first = None;
                self.badge = "no";
                let groups: Vec<(String, bool)> =
                    serde_json::from_value(host.call("solid_selection", Json::Null))
                        .unwrap_or_default();
                let ready = match self.op() {
                    "union" | "outer_shell" | "intersect" => groups.len() >= 2,
                    "split" => groups.len() == 2,
                    _ => false,
                };
                if ready
                    && groups.iter().all(|g| g.1)
                    && self
                        .execute(host, groups.iter().map(|g| g.0.clone()).collect())
                        .is_some()
                {
                    return Reply::committed();
                }
                if !groups.is_empty() && matches!(self.op(), "subtract" | "trim") {
                    host.call("solid_cutter_hint", Json::Null);
                }
                host.call("solid_select", Json::Null);
            }
            ToolEvent::Cancel | ToolEvent::Deactivate => {
                self.first = None;
                if matches!(ev, ToolEvent::Cancel) {
                    host.call("solid_select", Json::Null);
                }
            }
            ToolEvent::Click(p) | ToolEvent::Hover(p) => {
                let hit: Option<(String, bool)> =
                    serde_json::from_value(host.call("solid_pick", json!(p.px))).ok();
                if matches!(ev, ToolEvent::Hover(_)) {
                    let badge = match hit {
                        Some((ref g, true)) if Some(g) != self.first.as_ref() => {
                            if self.first.is_some() {
                                "2"
                            } else {
                                "1"
                            }
                        }
                        _ => "no",
                    };
                    if self.badge != badge {
                        self.badge = badge;
                        return Reply::redraw();
                    }
                    return Reply::ignored();
                }
                let Some((g, valid)) = hit else {
                    return Reply::ignored();
                };
                if self.first.as_ref() == Some(&g) {
                    return Reply::ignored();
                }
                if !valid {
                    host.call("solid_error", json!("not a solid"));
                    return Reply::ignored();
                }
                let Some(first) = self.first.take() else {
                    host.call("solid_select", json!(g));
                    self.first = Some(g);
                    return Reply::redraw();
                };
                let made = self.execute(host, vec![first, g]);
                if made.is_none() {
                    host.call("solid_select", Json::Null);
                }
                if matches!(self.op(), "outer_shell" | "union") {
                    self.first = made;
                }
                self.badge = "no";
            }
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
}
