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
#[derive(serde::Deserialize)]
struct WallFace {
    uid: String,
    point: P3,
    normal: P3,
    corners: Vec<P3>,
    #[serde(default)]
    occupied: Vec<[P3; 4]>,
}

#[derive(Default)]
pub struct PlaceGroup {
    template: Option<String>,
    anchor: P3,
    offset: P3,
    align: bool,
    normal: Option<P3>,
    segments: Vec<[P3; 2]>,
    hosted: bool,
    window: bool,
    dimensions: Option<[f64; 2]>,
    invalid: bool,
    wall: Option<WallFace>,
    sill: f64,
    reveal: f64,
    flipped: bool,
    opening: Option<[P3; 4]>,
}
impl PlaceGroup {
    fn fit_opening(&mut self) {
        self.opening = None;
        self.invalid = true;
        let Some(wall) = &self.wall else { return };
        if wall.corners.len() != 4 || self.segments.is_empty() {
            return;
        }
        let n = DVec3::from(wall.normal);
        let right = DVec3::Z.cross(n).normalize_or_zero();
        let at = DVec3::from(wall.point);
        let mut left = f64::INFINITY;
        let mut right_bound = f64::NEG_INFINITY;
        let mut base = f64::INFINITY;
        let mut top = f64::NEG_INFINITY;
        for corner in &wall.corners {
            let p = DVec3::from(*corner);
            let u = (p - at).dot(right);
            left = left.min(u);
            right_bound = right_bound.max(u);
            base = base.min(p.z);
            top = top.max(p.z);
        }
        // Restrict hosting to rectangular faces until arbitrary loops support cuts.
        if wall.corners.iter().any(|p| {
            let u = (DVec3::from(*p) - at).dot(right);
            (u - left).abs().min((u - right_bound).abs()) > 1e-5
                || (p[2] - base).abs().min((p[2] - top).abs()) > 1e-5
        }) {
            return;
        }
        let width = self
            .segments
            .iter()
            .flatten()
            .map(|p| p[0].abs())
            .fold(0., f64::max);
        let height = self
            .segments
            .iter()
            .flatten()
            .map(|p| p[2])
            .fold(0., f64::max);
        let [width, height] = self
            .dimensions
            .map(|[w, h]| [w * 0.5, h])
            .unwrap_or([width, height]);
        let bottom = base + self.sill;
        self.invalid = left >= -width - 1e-6
            || right_bound <= width + 1e-6
            || bottom + height >= top - 1e-6
            || wall.occupied.iter().any(|loop_| {
                let mut lo = f64::INFINITY;
                let mut hi = -lo;
                let mut z = f64::INFINITY;
                let mut t = -z;
                for p in loop_ {
                    let u = (DVec3::from(*p) - at).dot(right);
                    lo = lo.min(u);
                    hi = hi.max(u);
                    z = z.min(p[2]);
                    t = t.max(p[2]);
                }
                lo <= width + 1e-6
                    && hi >= -width - 1e-6
                    && z <= bottom + height + 1e-6
                    && t >= bottom - 1e-6
            });
        let point = DVec3::new(at.x, at.y, base + self.sill);
        let a = point - right * width;
        let b = point + right * width;
        self.opening = Some([
            a.into(),
            b.into(),
            (b + DVec3::Z * height).into(),
            (a + DVec3::Z * height).into(),
        ]);
        self.normal = Some(if self.flipped { (-n).into() } else { n.into() });
        self.offset = sub(
            (point - n * self.reveal).into(),
            rotate(basis(self.normal), self.anchor),
        );
    }
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
            "dimensions" => self.dimensions = serde_json::from_value(v.clone()).ok(),
            "wall_family" => {
                self.hosted = true;
                self.window = v == "window";
                self.sill = if self.window { 0.9 } else { 0. };
            }
            "normal" => self.normal = json_p3(v),
            _ => return false,
        }
        true
    }
    fn query(&self, k: &str, v: &Json) -> Json {
        if k == "wall_values" {
            return self
                .opening
                .filter(|_| self.hosted && self.template.is_some())
                .map(|outline| {
                    json!([
                        outline[1],
                        self.sill,
                        self.reveal,
                        DVec3::from(outline[1]).distance(outline[0].into()),
                        DVec3::from(outline[2]).distance(outline[1].into())
                    ])
                })
                .unwrap_or(Json::Null);
        }
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
        let Some(uid) = self.template.clone() else {
            return Reply::ignored();
        };
        if self.hosted {
            match ev {
                ToolEvent::Hover(p) | ToolEvent::Click(p) => {
                    self.wall = serde_json::from_value(h.call("family_wall", json!(p.px))).ok();
                }
                ToolEvent::Key { key, .. } if key == "Tab" => self.flipped = !self.flipped,
                ToolEvent::Value(super::Value::Pair(sill, reveal))
                    if sill.is_finite() && *sill >= 0. && reveal.is_finite() =>
                {
                    self.sill = *sill;
                    self.reveal = *reveal;
                }
                ToolEvent::Value(super::Value::Length(sill)) if sill.is_finite() && *sill >= 0. => {
                    self.sill = *sill
                }
                _ => return Reply::ignored(),
            }
            self.fit_opening();
            let click = matches!(ev, ToolEvent::Click(_));
            h.call(
                "opening_feedback",
                json!([self.invalid, click && self.invalid]),
            );
            if click && self.opening.is_some() && !self.invalid {
                let wall = self.wall.as_ref().unwrap();
                let args = json!({"template":uid,"offset":self.offset,"basis":basis(self.normal),
                    "wall_host":{"uid":wall.uid,"outline":self.opening,"normal":wall.normal,"initial_offset":self.reveal,"sill":self.sill,"offset":self.reveal,"flipped":self.flipped}});
                if h.run(&Command {
                    name: "place_group",
                    args,
                })
                .ok
                {
                    self.template = None;
                    return Reply::committed();
                }
            }
            return Reply::redraw();
        }
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
        if self.hosted && self.template.is_some() {
            let Some(outline) = self.opening else {
                return p;
            };
            for i in 0..4 {
                p.lines
                    .push(Preview::rubber(outline[i], outline[(i + 1) % 4]));
            }
            if let Some(n) = self.normal {
                for [a, b] in crate::hosted_openings::symbol(
                    outline.map(DVec3::from),
                    n.into(),
                    self.window,
                    self.flipped,
                ) {
                    p.lines.push(Preview::rubber(a.into(), b.into()));
                }
            }
            p.value = Some(super::preview::ValueAnchor {
                at: outline[1],
                label: "Sill ; offset",
            });
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hosted_family_fits_above_wall_base_and_flips_about_the_opening() {
        let mut t = PlaceGroup {
            hosted: true,
            template: Some("type".into()),
            sill: 0.8,
            reveal: 0.1,
            segments: vec![[[-0.5, 0., 0.], [0.5, 0., 1.2]]],
            wall: Some(WallFace {
                uid: "wall".into(),
                point: [2., 0., 4.],
                normal: [0., -1., 0.],
                occupied: vec![],
                corners: vec![[0., 0., 3.], [4., 0., 3.], [4., 0., 6.], [0., 0., 6.]],
            }),
            ..Default::default()
        };
        t.fit_opening();
        assert_eq!(t.offset, [2., 0.1, 3.8]);
        let outline = t.opening.unwrap();
        assert_eq!(outline[0], [1.5, 0., 3.8]);
        assert!(t.preview().value.is_some());
        t.flipped = true;
        t.fit_opening();
        assert_eq!(t.normal, Some([0., 1., 0.]));
        assert_eq!(t.opening, Some(outline));
        t.sill = 2.5;
        t.fit_opening();
        assert!(t.invalid && !t.preview().lines.is_empty());
    }
}
