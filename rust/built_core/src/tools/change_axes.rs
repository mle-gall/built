//! Group axes from origin, red point and a green direction off the red line.
use super::{
    add, dot, json_p3, len, scale, sub, unit, Command, Preview, Reply, Tool, ToolEvent, ToolHost,
    ToolInfo, P3,
};
use serde_json::{json, Value as Json};
static INFO: ToolInfo = ToolInfo {
    kind: "change_axes",
    name: "Axes",
    shortcut: "Shift+X",
    description: "Place the origin, red axis and green axis.",
    uses_snap: true,
    box_select: false,
    vcb_label: Some("Axes"),
    magnetic_axis_deg: Some(3.0),
    screen_axis_px: None,
    axis_source: true,
    axis_source_empty_only: false,
};
pub fn axes(o: P3, r: P3, g: P3) -> Option<[P3; 3]> {
    if !o.into_iter().chain(r).chain(g).all(f64::is_finite) {
        return None;
    }
    let red = sub(r, o);
    if len(red) < 1e-9 {
        return None;
    }
    let x = unit(red);
    let d = sub(g, o);
    let green = sub(d, scale(x, dot(x, d)));
    if len(green) < 1e-6 {
        return None;
    }
    let y = unit(green);
    let z = glam::DVec3::from(x).cross(y.into()).normalize().into();
    Some([x, y, z])
}
/// Use the first longest boundary edge. The winding determines the blue
/// direction even when the picked occurrence is mirrored.
pub(super) fn face_frame(points: &[glam::DVec3]) -> Option<[glam::DVec3; 3]> {
    let first = *points.first()?;
    let mut longest = glam::DVec3::ZERO;
    let mut start = first;
    let mut normal = glam::DVec3::ZERO;
    for (&a, &b) in points.iter().zip(points.iter().cycle().skip(1)) {
        normal += (a - first).cross(b - first);
        let edge = b - a;
        if edge.length_squared() > longest.length_squared() {
            longest = edge;
            start = a;
        }
    }
    let green = normal.try_normalize()?.cross(longest.try_normalize()?);
    Some([start, start + longest, start + green])
}

#[derive(Default)]
pub struct ChangeAxes {
    origin: Option<P3>,
    red: Option<P3>,
    hover: Option<P3>,
}
impl Tool for ChangeAxes {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn busy(&self) -> bool {
        self.origin.is_some()
    }
    fn state(&self) -> Json {
        json!({"start_point":self.origin,"red_point":self.red,"hover_point":self.hover})
    }
    fn set(&mut self, key: &str, v: &Json) -> bool {
        match key {
            "start_point" => self.origin = json_p3(v),
            "red_point" => self.red = json_p3(v),
            "hover_point" => self.hover = json_p3(v),
            _ => return false,
        }
        true
    }
    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate | ToolEvent::Deactivate | ToolEvent::Cancel => {
                self.origin = None;
                self.red = None;
                if !matches!(ev, ToolEvent::Cancel) {
                    self.hover = None;
                }
            }
            ToolEvent::Hover(p) => self.hover = p.world,
            ToolEvent::Value(value) => {
                let frame = host.call("axes_frame", Json::Null);
                let Some(f) = frame
                    .as_array()
                    .and_then(|a| a.iter().map(json_p3).collect::<Option<Vec<_>>>())
                    .filter(|v| v.len() == 4)
                else {
                    return Reply::ignored();
                };
                let point = match (value, self.origin, self.red) {
                    (super::Value::Triple(x, y, z), None, _) => add(
                        f[0],
                        add(scale(f[1], *x), add(scale(f[2], *y), scale(f[3], *z))),
                    ),
                    (super::Value::Length(deg), Some(o), red) => {
                        let angle = deg.to_radians();
                        let (u, v) = if let Some(r) = red {
                            let x = glam::DVec3::from(unit(sub(r, o)));
                            let up = glam::DVec3::from(f[3]);
                            let y = up.cross(x).try_normalize().unwrap_or(f[2].into());
                            (y.to_array(), x.cross(y).to_array())
                        } else {
                            (f[1], f[2])
                        };
                        add(o, add(scale(u, angle.cos()), scale(v, angle.sin())))
                    }
                    _ => return Reply::ignored(),
                };
                return self.event(
                    &ToolEvent::Click(super::Pointer {
                        world: Some(point),
                        ..Default::default()
                    }),
                    host,
                );
            }
            ToolEvent::Click(p) => {
                let Some(g) = p.world else {
                    return Reply::ignored();
                };
                let target = host.call("axes_target", Json::Null);
                let Some(o) = self.origin else {
                    self.origin = Some(g);
                    return Reply::redraw();
                };
                let Some(r) = self.red else {
                    if len(sub(g, o)) >= 1e-6 {
                        self.red = Some(g);
                    }
                    return Reply::redraw();
                };
                if axes(o, r, g).is_none() {
                    host.call("axes_message", json!("collinear"));
                    return Reply::redraw();
                }
                if host
                    .run(&Command {
                        name: "set_group_axes",
                        args: json!({"placement":target,"origin":o,"red":r,"green":g}),
                    })
                    .ok
                {
                    self.origin = None;
                    self.red = None;
                    host.call("axes_message", json!("done"));
                    return Reply::committed();
                }
            }
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
    fn preview(&self) -> Preview {
        let mut p = Preview::default();
        let (Some(o), Some(h)) = (self.origin, self.hover) else {
            return p;
        };
        if let Some(r) = self.red {
            if let Some(basis) = axes(o, r, h) {
                let length = len(sub(r, o)).max(0.5);
                p.lines
                    .extend(basis.map(|v| Preview::rubber(o, add(o, scale(v, length)))));
            } else {
                p.lines.push(Preview::rubber(o, r));
            }
        } else {
            p.lines.push(Preview::rubber(o, h));
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3 as V;

    #[test]
    fn face_alignment_follows_long_edge_and_winding() {
        let ring = [V::ZERO, V::Y, V::Y + 4. * V::Z, 4. * V::Z];
        let [o, r, g] = face_frame(&ring).unwrap();
        let [x, _, z] = axes(o.into(), r.into(), g.into()).unwrap();
        assert_eq!(x, V::Z.to_array());
        assert_eq!(z, V::X.to_array());
        assert!(face_frame(&[V::X, V::Y]).is_none());
    }
}
