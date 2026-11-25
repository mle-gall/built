//! Paint bucket and one-shot eyedropper. Picking a closed container does not
//! materialise its faces; only a face stroke or a sample needs the real face.
use super::{Command, Reply, Tool, ToolEvent, ToolHost, ToolInfo};
use serde_json::{json, Value as Json};

static INFO: ToolInfo = ToolInfo {
    kind: "paint",
    name: "Paint",
    shortcut: "B",
    description: "Click faces to paint them with the chosen material.",
    uses_snap: false,
    box_select: false,
    vcb_label: None,
    magnetic_axis_deg: None,
    screen_axis_px: None,
    axis_source: false,
    axis_source_empty_only: false,
};
pub struct Paint;
impl Tool for Paint {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }

    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        let (p, click) = match ev {
            ToolEvent::Hover(p) => (p, false),
            ToolEvent::Click(p) => (p, true),
            ToolEvent::Deactivate => {
                host.call("clear_paint_hover", Json::Null);
                return Reply::redraw();
            }
            _ => return Reply::ignored(),
        };
        let sample = p.mods.alt || host.call("sampler_armed", Json::Null) == true;
        let container = !sample && host.call("probe_container", json!(p.px)) == true;
        if !click {
            if container {
                host.call("clear_paint_hover", Json::Null);
            } else {
                host.call("mark_paint_face", json!(p.px));
            }
            return Reply::redraw();
        }
        let has_face = if container {
            true
        } else {
            host.call("pick_paint_face", json!(p.px)) == true
        };
        if sample {
            let image = host.call("sample_image", json!(p.px));
            let material = if !image.is_null() {
                image
            } else if has_face {
                let hit = host.call("sample_face", json!(p.px));
                // A separately painted back owns its own recipe. Only an
                // explicitly two-sided face shares its front on the back.
                let attrs = if hit["back_side"] != true || hit["back"] == true {
                    &hit["front"]
                } else {
                    &hit["back"]
                };
                let plane = if attrs["texture"]["uvw"].is_array() {
                    &hit["plane"]
                } else {
                    &Json::Null
                };
                json!({"attrs":attrs,"plane":plane})
            } else {
                return Reply::ignored();
            };
            host.call("adopt_sample", material);
            return Reply::redraw();
        }
        if !has_face {
            return Reply::ignored();
        }
        let mut args = host.call("brush", Json::Null);
        let targets = host.call("paint_targets", json!([p.px, container]));
        let (Some(dst), Some(src)) = (args.as_object_mut(), targets.as_object()) else {
            return Reply::ignored();
        };
        dst.extend(src.clone());
        let remember = args["default"] != true;
        let result = host.run(&Command {
            name: "paint_material",
            args,
        });
        if !result.ok {
            return Reply::ignored();
        }
        if container {
            host.call("container_painted", Json::Null);
        }
        if remember {
            host.call("remember_paint", Json::Null);
        }
        Reply::committed()
    }
}
