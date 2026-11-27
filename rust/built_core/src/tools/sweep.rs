// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::live::profile::{label, Profile};
use glam::{DVec2, DVec3};
use serde_json::{json, Value as Json};
const INFO: ToolInfo = ToolInfo {
    axis_source: true,
    axis_source_empty_only: true,
    magnetic_axis_deg: Some(3.),
    screen_axis_px: Some(9.),
    ..ToolInfo::basic(
        "sweep",
        "Sweep",
        "",
        "Draw a profiled member along a path.",
        true,
        Some("Length"),
    )
};
const ATTACH: [(&str, [f64; 2]); 9] = [
    ("centre", [0.5, 0.5]),
    ("bottom", [0.5, 0.]),
    ("bottom_left", [0., 0.]),
    ("left", [0., 0.5]),
    ("top_left", [0., 1.]),
    ("top", [0.5, 1.]),
    ("top_right", [1., 1.]),
    ("right", [1., 0.5]),
    ("bottom_right", [1., 0.]),
];
struct PathEdit {
    row: Json,
    frame: glam::DMat4,
    points: Vec<P3>,
    grip: Option<usize>,
}
impl PathEdit {
    fn from_row(row: Json) -> Option<Self> {
        let m: [f64; 16] = serde_json::from_value(row["parent_frame"].clone()).ok()?;
        let frame = glam::DMat4::from_cols_array(&m);
        let local: Vec<P3> = serde_json::from_value(row["path"]["points"].clone()).ok()?;
        Some(Self {
            row,
            frame,
            points: local
                .iter()
                .map(|p| frame.transform_point3((*p).into()).to_array())
                .collect(),
            grip: None,
        })
    }
    fn click(&mut self, p: &Pointer, host: &mut dyn ToolHost) -> Reply {
        if let Some(i) = self.grip {
            let Some(end) = p.world else {
                return Reply::ignored();
            };
            let mut row = self.row.clone();
            row["path"]["points"][i] = json!(self.frame.inverse().transform_point3(end.into()));
            if host
                .run(&Command {
                    name: "live_set",
                    args: json!({"row":row}),
                })
                .ok
            {
                self.points[i] = end;
                self.row = row;
                self.grip = None;
                return Reply::committed();
            }
        } else {
            let pixels: Vec<Option<[f64; 2]>> =
                serde_json::from_value(host.call("to_px", json!({"points":self.points})))
                    .unwrap_or_default();
            self.grip = pixels
                .iter()
                .enumerate()
                .filter_map(|(i, v)| {
                    let d = DVec2::from((*v)?).distance(p.px.into());
                    (d < 12.).then_some((i, d))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i);
        }
        Reply::redraw()
    }
}
pub struct SweepTool {
    edit: Option<PathEdit>,
    start_anchor: Option<Json>,
    start: Option<P3>,
    hover: Option<P3>,
    profile: Profile,
    attachment: usize,
    rotation: i32,
    mirror: bool,
}
impl Default for SweepTool {
    fn default() -> Self {
        Self {
            edit: None,
            start_anchor: None,
            start: None,
            hover: None,
            profile: Profile {
                library: "en/ipe240".into(),
                params: Default::default(),
            },
            attachment: 0,
            rotation: 0,
            mirror: false,
        }
    }
}
impl SweepTool {
    fn preset(&self) -> &'static str {
        match self.profile.library.as_str() {
            "basic/wall" => "Wall",
            "basic/timber" => "Timber member",
            _ => "Steel member",
        }
    }
    fn commit(
        &self,
        paths: Vec<Vec<P3>>,
        anchors: Option<[Option<Json>; 2]>,
        host: &mut dyn ToolHost,
    ) -> bool {
        if paths.is_empty() {
            return false;
        }
        let specs=paths.iter().map(|points|{
            let mut path=json!({"points":points});
            if let Some(ends)=&anchors{
                if points.len()==2 && ends.iter().any(Option::is_some){
                    path["ends"]=json!(std::array::from_fn::<_,2,_>(|i|match &ends[i]{
                        Some(to)=>json!({"to":to,"extend":true}),None=>json!(points[i])
                    }));
                }
            }
            json!({
                "preset":self.preset(),"profile":self.profile,"path":path,
                "rotation":self.rotation,"mirror":self.mirror,"attachment":ATTACH[self.attachment].0
            })
        }).collect::<Vec<_>>();
        host.run(&Command {
            name: "create_sweeps",
            args: json!({"sweeps":specs}),
        })
        .ok
    }
}
impl Tool for SweepTool {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn busy(&self) -> bool {
        self.start.is_some() || self.edit.is_some()
    }
    fn state(&self) -> Json {
        json!({"start_point":self.start,"hover_point":self.hover,"profile":self.profile.library,"chip":format!("{} · {} · {} · {}°{}",self.preset(),label(&self.profile.library).unwrap_or("Sweep"),ATTACH[self.attachment].0,self.rotation,if self.mirror{" · ↔"}else{""}),"attachment":ATTACH[self.attachment].0,"rotation":self.rotation,"mirror":self.mirror})
    }
    fn set(&mut self, key: &str, value: &Json) -> bool {
        match key {
            "edit" => {
                self.edit = PathEdit::from_row(value.clone());
                return self.edit.is_some();
            }
            "profile" => {
                self.edit = None;
                let Some(k) = value.as_str().filter(|k| label(k).is_some()) else {
                    return false;
                };
                self.profile.library = k.into();
                self.attachment = if k == "basic/wall" { 2 } else { 0 };
            }
            "hover_point" => self.hover = json_p3(value),
            "start_point" => self.start = json_p3(value),
            _ => return false,
        }
        true
    }
    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Cancel | ToolEvent::Deactivate => {
                self.start = None;
                self.start_anchor = None;
                self.hover = None;
                self.edit = None;
            }
            ToolEvent::Hover(p) => self.hover = p.world,
            ToolEvent::Click(p) => {
                if let Some(edit) = self.edit.as_mut() {
                    return edit.click(p, host);
                }
                if let Some(end) = p.world {
                    let anchor = if p.mods.ctrl {
                        let hit = host.call("sweep_face_ref", json!(p.px));
                        if hit.is_null() {
                            return Reply::ignored();
                        }
                        Some(hit)
                    } else {
                        None
                    };
                    if let Some(a) = self.start {
                        if a != end
                            && self.commit(
                                vec![vec![a, end]],
                                Some([self.start_anchor.clone(), anchor.clone()]),
                                host,
                            )
                        {
                            self.start = Some(end);
                            self.start_anchor = anchor;
                            return Reply::committed();
                        }
                    } else {
                        self.start = Some(end);
                        self.start_anchor = anchor;
                    }
                }
            }
            ToolEvent::Value(Value::Length(n)) => {
                if let (Some(a), Some(b)) = (self.start, self.hover) {
                    if let Some(dir) = (DVec3::from(b) - DVec3::from(a)).try_normalize() {
                        let end = (DVec3::from(a) + dir * *n).to_array();
                        if *n != 0.
                            && self.commit(
                                vec![vec![a, end]],
                                Some([self.start_anchor.clone(), None]),
                                host,
                            )
                        {
                            self.start = Some(end);
                            self.start_anchor = None;
                            return Reply::committed();
                        }
                    }
                }
            }
            ToolEvent::Key { key, .. } => match key.to_lowercase().as_str() {
                "tab" => self.attachment = (self.attachment + 1) % 9,
                "r" => self.rotation = (self.rotation + 90) % 360,
                "f" => self.mirror = !self.mirror,
                "l" => {
                    host.call("sweep_picker", Json::Null);
                }
                "enter" | "return" => {
                    if let Ok(paths) = serde_json::from_value(host.call("sweep_edges", Json::Null))
                    {
                        if self.commit(paths, None, host) {
                            return Reply::committed();
                        }
                    }
                }
                _ => return Reply::ignored(),
            },
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
    fn preview(&self) -> Preview {
        let mut out = Preview::default();
        if let Some(edit) = &self.edit {
            let mut points = edit.points.clone();
            if let (Some(i), Some(p)) = (edit.grip, self.hover) {
                points[i] = p;
            }
            for pair in points.windows(2) {
                out.lines.push(Preview::rubber(pair[0], pair[1]));
            }
            let size = points
                .windows(2)
                .map(|p| DVec3::from(p[0]).distance(p[1].into()))
                .fold(f64::INFINITY, f64::min)
                * 0.018;
            for p in &points {
                for axis in [DVec3::X, DVec3::Y, DVec3::Z] {
                    let d = axis * size;
                    out.lines.push(Preview::rubber(
                        (DVec3::from(*p) - d).to_array(),
                        (DVec3::from(*p) + d).to_array(),
                    ));
                }
            }
            return out;
        }
        let Some(b) = self.hover else { return out };
        let dir = self
            .start
            .and_then(|a| (DVec3::from(b) - DVec3::from(a)).try_normalize())
            .unwrap_or(DVec3::X);
        let x = DVec3::Z.cross(dir).try_normalize().unwrap_or(DVec3::X);
        if let Ok(outline) = self.profile.outline(24) {
            let origin =
                DVec2::from(ATTACH[self.attachment].1) * DVec2::new(outline.width, outline.height);
            for ring in outline.rings {
                let pts: Vec<_> = ring
                    .iter()
                    .map(|n| {
                        let mut p = (n.p - origin) / 1000.;
                        if self.mirror {
                            p.x = -p.x
                        }
                        let p = DVec2::from_angle((self.rotation as f64).to_radians()).rotate(p);
                        (DVec3::from(b) + x * p.x + dir.cross(x) * p.y).to_array()
                    })
                    .collect();
                for i in 0..pts.len() {
                    out.lines
                        .push(Preview::rubber(pts[i], pts[(i + 1) % pts.len()]))
                }
            }
        }
        if let Some(a) = self.start {
            out.lines.push(Preview::rubber(a, b));
        }
        out.value = Some(preview::ValueAnchor {
            at: b,
            label: label(&self.profile.library).unwrap_or("Sweep"),
        });
        out
    }
}
