//! One-shot launcher for a placement's absolute, per-axis scale editor.
use super::{Reply, Tool, ToolEvent, ToolHost, ToolInfo};
use serde_json::Value;
static INFO: ToolInfo = ToolInfo::basic("scale_manager", "Scale Manager", "", "", true, None);
#[derive(Default)]
pub struct ScaleManager {
    target: Option<String>,
}
impl Tool for ScaleManager {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn state(&self) -> Value {
        serde_json::json!({"target":self.target})
    }
    fn set(&mut self, key: &str, v: &Value) -> bool {
        if key != "target" {
            return false;
        }
        self.target = v.as_str().map(str::to_owned);
        true
    }
    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate => {
                host.call("scale_editor", serde_json::json!(self.target.take()));
            }
            ToolEvent::Deactivate => self.target = None,
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
}
