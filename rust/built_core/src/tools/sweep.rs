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
        let resolved = row.get("resolved_points");
        let local: Vec<P3> =
            serde_json::from_value(resolved.unwrap_or(&row["path"]["points"]).clone()).ok()?;
        let display_frame = if resolved.is_some() {
            glam::DMat4::from_cols_array(&serde_json::from_value(row["owner_frame"].clone()).ok()?)
        } else {
            frame
        };
        Some(Self {
            row,
            frame,
            points: local
                .iter()
                .map(|p| display_frame.transform_point3((*p).into()).to_array())
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
            let pos = self.frame.inverse().transform_point3(end.into());
            if row["path"]["depth"].is_number() {
                // An extrusion: its far end sets the thickness, its near
                // end moves it (unless a host holds it).
                let (a, b) = (DVec3::from(self.points[0]), DVec3::from(self.points[1]));
                let Some(dir) = (b - a).try_normalize() else {
                    return Reply::ignored();
                };
                if i == 1 {
                    let depth = (DVec3::from(end) - a).dot(dir) * 1000.;
                    if depth < 1. {
                        return Reply::ignored();
                    }
                    row["path"]["depth"] = json!(depth);
                } else if row["path"]["points"][0].is_array() {
                    row["path"]["points"][0] = json!(pos);
                    if row["path"]["points"][1].is_array() {
                        let reach = self.frame.inverse().transform_vector3(b - a);
                        row["path"]["points"][1] = json!((pos + reach).to_array());
                    }
                } else {
                    return Reply::ignored();
                }
            } else if row["path"]["ends"].is_array() && row["path"]["ends"][i].is_object() {
                let Some(dir) =
                    (DVec3::from(self.points[1 - i]) - DVec3::from(self.points[i])).try_normalize()
                else {
                    return Reply::ignored();
                };
                let distance = (DVec3::from(end) - DVec3::from(self.points[i])).dot(dir) * 1000.;
                let anchor = &mut row["path"]["ends"][i];
                anchor["offset"] = json!(anchor["offset"].as_f64().unwrap_or(0.) + distance);
            } else {
                row["path"]["points"][i] = json!(pos);
                if row["path"]["ends"].is_array() {
                    row["path"]["ends"][i] = json!(pos);
                }
            }
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
    inset: bool,
    chosen: Option<Json>,
    rotation: f64,
    mirror: bool,
    /// An extrusion preset's thickness in millimetres: the pointer then
    /// draws the outline, or picks the face or the host that gives it.
    depth: Option<f64>,
    /// The preset's edge profile and its sized-from expressions.
    edge: Json,
    fit: Json,
    /// What the pointer is over (`sweep_target`): a host member, a face.
    target: Json,
    /// The plane of the outline being drawn (its normal).
    normal: Option<P3>,
}
impl Default for SweepTool {
    fn default() -> Self {
        Self {
            edit: None,
            start_anchor: None,
            start: None,
            hover: None,
            profile: Profile {
                catalogue: None,
                sketch: None,
                drawing: None,
                library: "basic/i".into(),
                params: Default::default(),
                family: String::new(),
            },
            attachment: 0,
            inset: false,
            chosen: None,
            rotation: 0.,
            mirror: false,
            depth: None,
            edge: Json::Null,
            fit: Json::Null,
            target: Json::Null,
            normal: None,
        }
    }
}
/// The section frame the generator gives a path direction (live/sweep.rs).
fn section_frame(dir: DVec3) -> (DVec3, DVec3) {
    let across = DVec3::Z.cross(dir).try_normalize().unwrap_or(DVec3::X);
    (across, dir.cross(across))
}
fn vec3(v: &Json) -> Option<DVec3> {
    json_p3(v).map(DVec3::from)
}
impl SweepTool {
    /// The base of every member this tool makes: the preset and the section.
    fn spec(&self, path: Json) -> Json {
        let mut spec = json!({
            "preset":self.preset(),"preset_definition":self.chosen,"preset_uid":self.chosen.as_ref().map(|p|&p["uid"]),"profile":self.profile,"path":path,
            "rotation":self.rotation,"mirror":self.mirror,"attachment":self.attachment_name()
        });
        if self.edge.is_object() {
            spec["edge"] = self.edge.clone();
        }
        spec
    }
    fn run_specs(&self, specs: Vec<Json>, host: &mut dyn ToolHost) -> bool {
        !specs.is_empty()
            && host
                .run(&Command {
                    name: "create_sweeps",
                    args: if self.profile.library == "basic/wall" { json!({"sweeps":specs,"wall_junctions":true}) } else { json!({"sweeps":specs}) },
                })
                .ok
    }
    /// The way an extrusion goes from a plane seen from `normal`: into it
    /// (a slab below the outline), or out of it once flipped (F).
    fn extrude_dir(&self, normal: DVec3) -> DVec3 {
        if self.mirror { normal } else { -normal }
    }
    fn rectangular(&self) -> bool {
        self.profile.drawing.is_none() && self.profile.outline(24).is_ok_and(|o| o.family == "rectangle")
    }
    /// An extrusion from two opposite corners on a plane.
    fn rectangle_spec(&self, a: DVec3, c: DVec3, normal: DVec3) -> Option<Json> {
        let dir = self.extrude_dir(normal);
        let (across, up) = section_frame(dir);
        let (dx, dy) = ((c - a).dot(across), (c - a).dot(up));
        if dx.abs() < 1e-3 || dy.abs() < 1e-3 {
            return None;
        }
        let mut spec = self.spec(json!({"points":[a.to_array()],"depth":self.depth?,"dir":dir.to_array()}));
        spec["profile"] = json!({"library":self.profile.library,"params":{"width":dx.abs()*1000.,"height":dy.abs()*1000.}});
        spec["attachment"] = json!(match (dx >= 0., dy >= 0.) {
            (true, true) => "bottom_left",
            (false, true) => "bottom_right",
            (true, false) => "top_left",
            (false, false) => "top_right",
        });
        spec["rotation"] = json!(0.);
        spec["mirror"] = json!(false);
        Some(spec)
    }
    /// An extrusion of a face's outline, holes included, from where it is.
    fn face_spec(&self, face: &Json) -> Option<Json> {
        let rings: Vec<Vec<P3>> = serde_json::from_value(face["rings"].clone()).ok()?;
        let normal = vec3(&face["normal"])?;
        let dir = self.extrude_dir(normal);
        let (across, up) = section_frame(dir);
        let origin = DVec3::from(*rings.first()?.first()?);
        let mut curves = Vec::new();
        for ring in &rings {
            let xy = |p: &P3| { let v = DVec3::from(*p) - origin; [v.dot(across) * 1000., v.dot(up) * 1000.] };
            for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                curves.push(json!({"id":curves.len()+1,"kind":"line","a":xy(a),"b":xy(b)}));
            }
        }
        let name = format!("{} outline", self.preset());
        let mut spec = self.spec(json!({"points":[origin.to_array()],"depth":self.depth?,"dir":dir.to_array()}));
        spec["profile"] = json!({"drawing":{"name":name,"use":"profile","curves":curves,"next_id":curves.len()+1,"origin":[0.,0.]}});
        spec["attachment"] = json!("origin");
        spec["rotation"] = json!(0.);
        spec["mirror"] = json!(false);
        Some(spec)
    }
    /// The preset's profile as it is, extruded at a point.
    fn point_spec(&self, at: DVec3, normal: DVec3) -> Option<Json> {
        let dir = self.extrude_dir(normal);
        let mut spec = self.spec(json!({"points":[at.to_array()],"depth":self.depth?,"dir":dir.to_array()}));
        // Under a point the mirror key means the side, not the section.
        spec["mirror"] = json!(false);
        Some(spec)
    }
    /// A member sized from `host` (`sweep_target`): an extrusion under its
    /// lower end (a pad), else a run along its axis under its section (a
    /// strip). The relation is kept: `fit` and the anchored path.
    fn host_spec(&self, host: &Json) -> Option<Json> {
        let points: Vec<P3> = serde_json::from_value(host["points"].clone()).ok()?;
        let (hx, hy, centre) = (vec3(&host["x"])?, vec3(&host["y"])?, vec3(&host["centre"])?);
        let row = host["row"].as_str()?;
        let mut values = self.fit["values"].clone();
        let turn = |across: DVec3, up: DVec3| hx.dot(up).atan2(hx.dot(across)).to_degrees();
        let mut spec;
        if let Some(depth) = self.depth {
            let (first, last) = (DVec3::from(*points.first()?), DVec3::from(*points.last()?));
            let at = if first.z <= last.z { "start" } else { "end" };
            let dir = self.extrude_dir(DVec3::Z);
            let (across, up) = section_frame(dir);
            let anchor = json!({"to":{"sweep":row,"at":at}});
            spec = self.spec(json!({"ends":[anchor, anchor],"depth":depth,"dir":dir.to_array()}));
            spec["attachment"] = json!("centre");
            spec["rotation"] = json!(turn(across, up));
            spec["shift"] = json!([centre.dot(across) * 1000., centre.dot(up) * 1000.]);
            spec["fit"] = json!({"host":{"sweep":row,"at":at},"values":values});
        } else {
            let dir = (DVec3::from(*points.get(1)?) - DVec3::from(points[0])).try_normalize()?;
            let (across, up) = section_frame(dir);
            let height = host["dims"]["height"].as_f64()?;
            let width = host["dims"]["width"].as_f64()?;
            // Under the section: its centre, down half its height.
            let under = centre - hy * (height / 2000.);
            let shift = [under.dot(across) * 1000., under.dot(up) * 1000.];
            // Resting on one of the nine points with a square section, the
            // shift follows the host's size as well.
            if let (Some(uv), true) = (serde_json::from_value::<[f64; 2]>(host["attach"].clone()).ok(), host["rotation"].as_f64() == Some(0.)) {
                let mx = if host["mirror"] == true { -1. } else { 1. };
                for (field, k, dim, now) in [("shift.x", (0.5 - uv[0]) * mx, "width", shift[0]), ("shift.y", -uv[1], "height", shift[1])] {
                    let size = if dim == "width" { width } else { height };
                    if k != 0. && (k * size - now).abs() < 1e-6 && values.is_object() {
                        values[field] = json!(format!("host.{dim} * {k}"));
                    }
                }
            }
            let resolved = self.fitted(host);
            let reach = resolved["profile.params.width"].as_f64().map_or(0., |w| ((w - width) / 2.).max(0.));
            let path = if points.len() == 2 {
                json!({"ends":[{"to":{"sweep":row,"at":"start"},"offset":-reach,"extend":true},{"to":{"sweep":row,"at":"end"},"offset":-reach,"extend":true}]})
            } else {
                json!({"points":points})
            };
            spec = self.spec(path);
            spec["attachment"] = json!("top");
            spec["rotation"] = json!(turn(across, up));
            spec["mirror"] = json!(false);
            spec["shift"] = json!(shift);
            spec["fit"] = json!({"host":{"sweep":row,"at":"start"},"values":values});
        }
        Some(spec)
    }
    /// The preset's fitted fields for this host, as numbers.
    fn fitted(&self, host: &Json) -> Json {
        let dims: std::collections::BTreeMap<String, f64> = serde_json::from_value(host["dims"].clone()).unwrap_or_default();
        crate::live::fit::resolve(&self.fit["values"], &json!(dims))
    }
    /// Ghost lines of what a click would make: an extrusion's two outlines
    /// and their risers, or a run's section along its host.
    fn ghost(&self, spec: &Json, out: &mut Preview) {
        let host = &self.target["host"];
        let fitted = if spec["fit"].is_object() { self.fitted(host) } else { Json::Null };
        let mut profile = spec["profile"].clone();
        for (field, n) in fitted.as_object().into_iter().flatten() {
            if let Some(name) = field.strip_prefix("profile.params.") {
                profile["params"][name] = n.clone();
            }
        }
        let Ok(profile) = serde_json::from_value::<Profile>(profile) else { return };
        let Ok(outline) = profile.outline(24) else { return };
        let host_points: Vec<P3> = serde_json::from_value(host["points"].clone()).unwrap_or_default();
        let depth = spec["path"]["depth"].as_f64();
        // The run: from the first outline to the second.
        let (start, run) = if let (Some(depth), Some(dir)) = (depth, vec3(&spec["path"]["dir"])) {
            let start = json_p3(&spec["path"]["points"][0]).map(DVec3::from).or_else(|| {
                let (first, last) = (DVec3::from(*host_points.first()?), DVec3::from(*host_points.last()?));
                Some(if first.z <= last.z { first } else { last })
            });
            let Some(start) = start else { return };
            (start, dir * depth / 1000.)
        } else if host_points.len() == 2 {
            let (a, b) = (DVec3::from(host_points[0]), DVec3::from(host_points[1]));
            let Some(dir) = (b - a).try_normalize() else { return };
            let reach = -spec["path"]["ends"][0]["offset"].as_f64().unwrap_or(0.) / 1000.;
            (a - dir * reach, b - a + dir * (2. * reach))
        } else {
            return;
        };
        let Some(dir) = run.try_normalize() else { return };
        let (across, up) = section_frame(dir);
        let mut origin = outline.attachment(spec["attachment"].as_str().unwrap_or("centre")).unwrap_or_default();
        if spec["attachment"] == "origin" {
            origin = outline.named.get("origin").copied().unwrap_or(origin);
        }
        let turn = DVec2::from_angle(spec["rotation"].as_f64().unwrap_or(0.).to_radians());
        let mut shift = serde_json::from_value::<[f64; 2]>(spec["shift"].clone()).unwrap_or_default();
        for (i, key) in ["shift.x", "shift.y"].into_iter().enumerate() {
            if let Some(n) = fitted[key].as_f64() {
                shift[i] = n;
            }
        }
        for ring in &outline.rings {
            let pts: Vec<DVec3> = ring.iter().map(|n| {
                let p = turn.rotate((n.p - origin) / 1000.) + DVec2::from(shift) / 1000.;
                start + across * p.x + up * p.y
            }).collect();
            let sharp = pts.len() <= 16;
            for i in 0..pts.len() {
                let k = (i + 1) % pts.len();
                out.lines.push(Preview::rubber(pts[i].to_array(), pts[k].to_array()));
                out.lines.push(Preview::rubber((pts[i] + run).to_array(), (pts[k] + run).to_array()));
                if sharp {
                    out.lines.push(Preview::rubber(pts[i].to_array(), (pts[i] + run).to_array()));
                }
            }
        }
    }
    /// What a click would make, here and now, in the extrusion and
    /// sized-from modes (None: the path mode's own points).
    fn pending(&self) -> Option<Json> {
        let host = &self.target["host"];
        if self.fit.is_object() && host.is_object() && self.start.is_none() {
            return self.host_spec(host);
        }
        self.depth?;
        let hover = DVec3::from(self.hover?);
        if let Some(a) = self.start {
            return self.rectangle_spec(a.into(), hover, self.normal.map_or(DVec3::Z, DVec3::from));
        }
        let face = &self.target["face"];
        if !self.fit.is_object() && face.is_object() && face["live"] != true {
            return self.face_spec(face);
        }
        if self.rectangular() && !self.fit.is_object() {
            return None;
        }
        self.point_spec(hover, DVec3::Z)
    }
}
impl SweepTool {
    fn attachment_name(&self) -> &str {
        ATTACH
            .get(self.attachment)
            .map(|a| a.0)
            .or_else(|| {
                self.extra_attachments()
                    .nth(self.attachment.saturating_sub(9))
            })
            .unwrap_or("centre")
    }
    fn extra_attachments(&self) -> impl Iterator<Item = &str> {
        self.profile.drawing.iter().flat_map(|d| {
            std::iter::once("origin").chain(
                d["points"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p["name"].as_str()),
            )
        })
    }
    /// The profile the chip names: a drawing by its name, a size by its
    /// label, a size the preset re-dimensions by its own width and height.
    fn profile_name(&self) -> std::borrow::Cow<'_, str> {
        if self.profile.drawing.is_none() {
            if let (Some(w), Some(h)) = (self.profile.params.get("width"), self.profile.params.get("height")) {
                return format!("{w:.0} × {h:.0}").into();
            }
        }
        self.profile
            .drawing
            .as_ref()
            .and_then(|d| d["name"].as_str())
            .filter(|n| !n.is_empty())
            .or_else(|| self.profile.name())
            .unwrap_or(if self.profile.drawing.is_some() { "Drawn profile" } else { "Sweep" })
            .into()
    }
    /// *preset · profile · attachment · rotation*; a profile size picked
    /// from the Library is its own preset, so its name shows once.
    fn chip(&self) -> String {
        let (preset, profile) = (self.preset(), self.profile_name());
        let named = if preset == profile { preset.to_owned() } else { format!("{preset} · {profile}") };
        format!("{named} · {} · {}°{}", self.attachment_name(), self.rotation, if self.mirror { " · ↔" } else { "" })
    }
    fn preset(&self) -> &str {
        if let Some(name) = self.chosen.as_ref().and_then(|p| p["name"].as_str()) {
            return name;
        }
        match self.profile.library.as_str() {
            "" => "Drawn member",
            "basic/wall" => "Wall",
            "basic/timber" => "Timber member",
            _ => "Member",
        }
    }
    fn run_paths(&self, paths: Vec<Json>, host: &mut dyn ToolHost) -> bool {
        self.run_specs(paths.into_iter().map(|path| self.spec(path)).collect(), host)
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
        let paths = paths
            .iter()
            .map(|points| {
                let mut path = json!({"points":points});
                if let Some(ends) = &anchors {
                    if points.len() == 2 {
                        let dir = DVec3::from(points[1]) - DVec3::from(points[0]);
                        let ends = std::array::from_fn::<_, 2, _>(|i| {
                            if let Some(hit) = &ends[i] {
                                if json_p3(&hit["normal"])
                                    .is_some_and(|n| DVec3::from(n).dot(dir).abs() > 1e-9)
                                {
                                    return json!({"to":hit["to"],"extend":true});
                                }
                            }
                            json!(points[i])
                        });
                        if ends.iter().any(Json::is_object) {
                            path["ends"] = json!(ends);
                        }
                    }
                }
                path
            })
            .collect();
        self.run_paths(paths, host)
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
        let inset = if self.inset && self.edit.is_none() {
            self.profile.outline(24).ok().map(|outline| {
                let centre = DVec2::new(outline.width, outline.height) * 0.5;
                let scale = 48. / outline.width.max(outline.height);
                outline
                    .rings
                    .iter()
                    .map(|ring| {
                        ring.iter()
                            .map(|node| {
                                let mut p = (node.p - centre) * scale;
                                if self.mirror {
                                    p.x = -p.x;
                                }
                                let p = DVec2::from_angle(self.rotation.to_radians()).rotate(p);
                                [p.x, -p.y]
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            })
        } else {
            None
        };
        let extruding = self.depth.is_some();
        // The start is the tool's own while an outline is drawn: the
        // inference must not pull the opposite corner onto an axis from it.
        let length = match self.depth {
            Some(depth) => Some(depth / 1000.),
            None => self.start.zip(self.hover).map(|(a, b)| DVec3::from(a).distance(b.into())),
        };
        let chip = if extruding || (self.fit.is_object() && self.target["host"].is_object()) {
            let what = if self.target["host"].is_object() && self.fit.is_object() && self.start.is_none() {
                "sized from the member".to_owned()
            } else if self.start.is_some() {
                match self.pending() {
                    Some(spec) => format!("{:.0} × {:.0} mm", spec["profile"]["params"]["width"].as_f64().unwrap_or(0.), spec["profile"]["params"]["height"].as_f64().unwrap_or(0.)),
                    None => "opposite corner".to_owned(),
                }
            } else if !self.fit.is_object() && self.target["face"].is_object() && self.target["face"]["live"] != true {
                "from this face".to_owned()
            } else if self.rectangular() && !self.fit.is_object() {
                "first corner, or a face".to_owned()
            } else if self.rectangular() {
                // A free pad: its catalogue size.
                let o = self.profile.outline(24).ok();
                o.map_or_else(|| self.profile_name().into_owned(), |o| format!("{:.0} × {:.0} mm", o.width, o.height))
            } else {
                self.profile_name().into_owned()
            };
            let edge = match self.edge["shape"].as_str() {
                Some("chamfer") => " · chamfered edge",
                Some("round") => " · rounded edge",
                Some("step") => " · stepped edge",
                Some("downstand") => " · edge beam",
                _ => "",
            };
            format!("{} · {}{}{}", self.preset(), what, edge, if self.mirror && extruding { " · above" } else { "" })
        } else {
            self.chip()
        };
        json!({"section_inset":inset,"length":length,"start_point":if extruding { None } else { self.start },"corner_point":if extruding { self.start } else { None },"hover_point":self.hover,"profile":self.profile.library,"chip":chip,"attachment":self.attachment_name(),"rotation":self.rotation,"mirror":self.mirror,
            "caption":if extruding { "Thickness" } else { "Length" },"depth":self.depth,"extruding":extruding,"sized":self.fit.is_object() && self.edit.is_none(),"target":if self.target["host"].is_object() && self.fit.is_object() { "host" } else if self.target["face"].is_object() { "face" } else { "" }})
    }
    fn set(&mut self, key: &str, value: &Json) -> bool {
        match key {
            "preset" => {
                let fields = &value["values"];
                let Ok(profile) = serde_json::from_value::<Profile>(fields["profile"].clone())
                else {
                    return false;
                };
                if value["primitive"] != "sweep" || profile.outline(24).is_err() {
                    return false;
                }
                self.profile = profile;
                self.edit = None;
                self.rotation = fields["rotation"].as_f64().unwrap_or(0.);
                self.mirror = fields["mirror"] == true;
                let index = ATTACH
                    .iter()
                    .map(|a| a.0)
                    .chain(self.extra_attachments())
                    .position(|s| fields["attachment"] == s)
                    .unwrap_or(0);
                self.attachment = index;
                self.chosen = Some(value.clone());
                self.depth = fields["depth"].as_f64().filter(|d| *d > 0.);
                self.edge = if fields["edge"].is_object() { fields["edge"].clone() } else { Json::Null };
                self.fit = if fields["fit"]["values"].is_object() { fields["fit"].clone() } else { Json::Null };
                self.start = None;
                self.normal = None;
                self.target = Json::Null;
            }
            "depth" => {
                let Some(depth) = value.as_f64().filter(|d| *d > 0.) else { return false };
                if self.depth.is_none() {
                    return false;
                }
                self.depth = Some(depth);
            }
            "target" => self.target = value.clone(),
            "edit" => {
                self.edit = PathEdit::from_row(value.clone());
                return self.edit.is_some();
            }
            "sketch" => {
                let profile = Profile {
                    catalogue: None,
                    library: String::new(),
                    sketch: value["uid"].as_str().map(str::to_owned),
                    drawing: Some(value.clone()),
                    params: Default::default(),
                    family: String::new(),
                };
                if profile.outline(24).is_err() {
                    return false;
                }
                self.edit = None;
                self.chosen = None;
                (self.depth, self.edge, self.fit) = (None, Json::Null, Json::Null);
                self.profile = profile;
                let name = match value["attach"].as_str().unwrap_or("centre") {
                    "middle_left" => "left",
                    "middle_right" => "right",
                    "bottom_centre" => "bottom",
                    "top_centre" => "top",
                    n => n,
                };
                let index = ATTACH
                    .iter()
                    .map(|a| a.0)
                    .chain(self.extra_attachments())
                    .position(|n| n == name)
                    .unwrap_or(0);
                self.attachment = index;
            }
            "dimensions" => {
                let Ok(profile) = serde_json::from_value::<Profile>(value.clone()) else { return false };
                if profile.outline(24).is_err() { return false; }
                self.edit = None;
                self.chosen = None;
                self.attachment = if profile.library == "basic/wall" { 2 } else { 0 };
                self.profile = profile;
            }
            "profile" => {
                self.edit = None;
                let Some(k) = value.as_str().filter(|k| label(k).is_some()) else {
                    return false;
                };
                self.chosen = None;
                (self.depth, self.edge, self.fit) = (None, Json::Null, Json::Null);
                self.profile = Profile {
                    catalogue: None,
                    library: k.into(),
                    sketch: None,
                    drawing: None,
                    params: Default::default(),
                    family: String::new(),
                };
                self.attachment = if k == "basic/wall" { 2 } else { 0 };
            }
            "hover_point" => self.hover = json_p3(value),
            "start_point" | "corner_point" => self.start = json_p3(value),
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
                self.normal = None;
                self.target = Json::Null;
            }
            ToolEvent::DoubleClick(_) => {
                self.start = None;
                self.start_anchor = None;
            }
            ToolEvent::Activate => {
                let p = host.call("sweep_default", Json::Null);
                if !p.is_null() {
                    self.set("preset", &p);
                }
            }
            ToolEvent::Hover(p) => {
                self.hover = p.world;
                self.inset = false;
                let aware = self.depth.is_some() || self.fit.is_object();
                if aware && self.edit.is_none() && self.start.is_none() {
                    self.target = host.call("sweep_target", json!({"px":p.px}));
                }
                if self.edit.is_none() && self.hover.is_some() && self.depth.is_none() {
                    let pre = self.preview();
                    let section: Vec<_> = pre
                        .lines
                        .iter()
                        .take(
                            pre.lines
                                .len()
                                .saturating_sub(usize::from(self.start.is_some())),
                        )
                        .map(|line| line.a)
                        .collect();
                    if let Ok(pixels) = serde_json::from_value::<Vec<Option<[f64; 2]>>>(
                        host.call("to_px", json!({"points":section})),
                    ) {
                        let mut lo = DVec2::splat(f64::INFINITY);
                        let mut hi = -lo;
                        for p in pixels.into_iter().flatten() {
                            lo = lo.min(p.into());
                            hi = hi.max(p.into());
                        }
                        self.inset = (hi - lo).min_element() < 22.;
                    }
                }
            }
            ToolEvent::Click(p) => {
                if let Some(edit) = self.edit.as_mut() {
                    return edit.click(p, host);
                }
                if self.depth.is_some() || self.fit.is_object() {
                    if self.start.is_none() {
                        self.target = host.call("sweep_target", json!({"px":p.px}));
                        // Alt keeps a plain point: the face only gives its plane.
                        if p.mods.alt && self.target["face"].is_object() {
                            self.target["face"]["live"] = json!(true);
                            self.target["host"] = Json::Null;
                        }
                    }
                    self.hover = p.world.or(self.hover);
                    let hosted = self.fit.is_object() && self.target["host"].is_object();
                    let from_face = self.depth.is_some() && !self.fit.is_object() && self.target["face"].is_object() && self.target["face"]["live"] != true;
                    let drawn = self.depth.is_some() && self.rectangular() && !self.fit.is_object();
                    if self.start.is_some() || hosted || from_face || (self.depth.is_some() && !drawn) {
                        if let Some(spec) = self.pending() {
                            if self.run_specs(vec![spec], host) {
                                self.start = None;
                                self.normal = None;
                                self.target = Json::Null;
                                return Reply::committed();
                            }
                        }
                        return Reply::redraw();
                    }
                    if self.depth.is_some() {
                        let Some(at) = p.world else { return Reply::ignored() };
                        self.start = Some(at);
                        self.normal = json_p3(&self.target["face"]["normal"]);
                        return Reply::redraw();
                    }
                }
                if p.mods.alt && self.start.is_none() {
                    let sketch = host.call("sweep_face_profile", json!(p.px));
                    if self.set("sketch", &sketch) {
                        return Reply::redraw();
                    }
                }
                if let Some(mut end) = p.world {
                    let plate = self.chosen.as_ref().is_some_and(|c| c["record"]["class"] == "IfcPlate");
                    let anchor = if p.mods.alt {
                        None
                    } else {
                        let hit = host.call("sweep_face_ref", json!({"px":p.px,"at":end,"plate":plate && self.start.is_none()}));
                        (!hit.is_null()).then_some(hit)
                    };
                    if let Some(snap) = anchor.as_ref().and_then(|h| json_p3(&h["snap"])) {
                        end = snap;
                    }
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
            // An extrusion: a typed length is its thickness, a typed pair
            // the outline's two sides from the first corner.
            ToolEvent::Value(Value::Length(n)) if self.depth.is_some() => {
                if *n <= 0. {
                    return Reply::ignored();
                }
                self.depth = Some(*n * 1000.);
            }
            ToolEvent::Value(Value::Pair(w, h)) if self.depth.is_some() => {
                let (Some(a), Some(b)) = (self.start, self.hover) else { return Reply::ignored() };
                let normal = self.normal.map_or(DVec3::Z, DVec3::from);
                let (across, up) = section_frame(self.extrude_dir(normal));
                let v = DVec3::from(b) - DVec3::from(a);
                let sign = |x: f64| if x < 0. { -1. } else { 1. };
                let c = DVec3::from(a) + across * (sign(v.dot(across)) * w.abs()) + up * (sign(v.dot(up)) * h.abs());
                if let Some(spec) = self.rectangle_spec(a.into(), c, normal) {
                    if self.run_specs(vec![spec], host) {
                        self.start = None;
                        self.normal = None;
                        return Reply::committed();
                    }
                }
            }
            ToolEvent::Value(Value::Length(n)) => {
                // A plate started on a face: its thickness goes along the
                // face's normal, wherever the pointer is.
                let plate = self.chosen.as_ref().is_some_and(|p| p["record"]["class"] == "IfcPlate");
                let normal = self.start_anchor.as_ref().and_then(|h| json_p3(&h["normal"])).map(DVec3::from).and_then(|n| n.try_normalize());
                if let (Some(a), Some(n_dir), true) = (self.start, normal, plate) {
                    let end = (DVec3::from(a) + n_dir * *n).to_array();
                    if *n != 0. && self.commit(vec![vec![a, end]], Some([self.start_anchor.clone(), None]), host) {
                        self.start = None;
                        self.start_anchor = None;
                        return Reply::committed();
                    }
                }
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
                "tab" => {
                    self.attachment = (self.attachment + 1) % (9 + self.extra_attachments().count())
                }
                "r" => self.rotation = (self.rotation + 90.) % 360.,
                "f" => self.mirror = !self.mirror,
                "l" => {
                    host.call("sweep_picker", Json::Null);
                }
                // Draw the profile now: the Sketch workspace, then back here
                // with it in the chip (sketch_workspace.rs, `sketch:sweep`).
                "d" if self.start.is_none() && self.edit.is_none() => {
                    host.call("sweep_draw", Json::Null);
                }
                "enter" | "return" => {
                    if let Ok(paths) = serde_json::from_value(host.call("sweep_edges", Json::Null))
                    {
                        if self.run_paths(paths, host) {
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
        if self.depth.is_some() || (self.fit.is_object() && self.target["host"].is_object() && self.start.is_none()) {
            if let Some(spec) = self.pending() {
                self.ghost(&spec, &mut out);
            }
            if self.depth.is_some() || !out.lines.is_empty() {
                if let Some(b) = self.hover {
                    out.value = Some(preview::ValueAnchor { at: b, label: if self.depth.is_some() { "Thickness" } else { "Sweep" } });
                }
                return out;
            }
        }
        let Some(b) = self.hover else { return out };
        let dir = self
            .start
            .and_then(|a| (DVec3::from(b) - DVec3::from(a)).try_normalize())
            .unwrap_or(DVec3::X);
        let x = DVec3::Z.cross(dir).try_normalize().unwrap_or(DVec3::X);
        if let Ok(outline) = self.profile.outline(24) {
            let origin = outline
                .attachment(self.attachment_name())
                .unwrap_or_default();
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
