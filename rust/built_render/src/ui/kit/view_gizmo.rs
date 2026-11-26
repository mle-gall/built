//! The axis gizmo in the viewport's top-right corner (UX pass 2, M03): the
//! model's three axes as the camera sees them. A click on an axis end looks
//! along that axis (Top, Front, …; the same end again flips to the
//! opposite view); a drag on the gizmo orbits.
//!
//! Angles are the orbit camera's: radians, Z up, the eye at
//! `target + (cos p cos y, cos p sin y, sin p) · distance`.
use crate::ui::tokens::*;
use egui::{vec2, Align2, Color32, Pos2, Rect, Sense, Stroke, Vec2};

/// The gizmo's circle: what it answers to, and its hover disc.
pub const GIZMO_R: f32 = 40.0;
/// Centre to the middle of an axis end.
const ARM: f32 = 28.0;
const END_R: f32 = 8.0;
const NEG_R: f32 = 6.0;
/// The hit slop around an end, so a small target is easy to take.
const SLOP: f32 = 2.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum End {
    PosX,
    NegX,
    PosY,
    NegY,
    PosZ,
    NegZ,
}

impl End {
    pub const ALL: [End; 6] = [End::PosX, End::NegX, End::PosY, End::NegY, End::PosZ, End::NegZ];
    fn axis(self) -> usize {
        match self {
            End::PosX | End::NegX => 0,
            End::PosY | End::NegY => 1,
            End::PosZ | End::NegZ => 2,
        }
    }
    pub fn positive(self) -> bool {
        matches!(self, End::PosX | End::PosY | End::PosZ)
    }
    fn direction(self) -> [f64; 3] {
        let mut d = [0.; 3];
        d[self.axis()] = if self.positive() { 1. } else { -1. };
        d
    }
    /// The standard view looking from this end towards the model.
    pub fn view(self) -> &'static str {
        match self {
            End::PosX => "right",
            End::NegX => "left",
            End::PosY => "back",
            End::NegY => "front",
            End::PosZ => "top",
            End::NegZ => "bottom",
        }
    }
    pub fn opposite(self) -> End {
        match self {
            End::PosX => End::NegX,
            End::NegX => End::PosX,
            End::PosY => End::NegY,
            End::NegY => End::PosY,
            End::PosZ => End::NegZ,
            End::NegZ => End::PosZ,
        }
    }
    pub fn letter(self) -> &'static str {
        ["X", "Y", "Z"][self.axis()]
    }
    pub fn colour(self) -> Color32 {
        [AXIS_X, AXIS_Y, AXIS_Z][self.axis()]
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The camera's right, up and back (towards the eye) directions.
pub fn basis(yaw: f64, pitch: f64) -> [[f64; 3]; 3] {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let back = [cp * cy, cp * sy, sp];
    let right = [-sy, cy, 0.];
    // up = back × right
    let up = [
        back[1] * right[2] - back[2] * right[1],
        back[2] * right[0] - back[0] * right[2],
        back[0] * right[1] - back[1] * right[0],
    ];
    [right, up, back]
}

/// Each end's offset from the centre (screen points, y down) and its depth
/// towards the viewer (1 faces the eye, -1 points away).
pub fn layout(yaw: f64, pitch: f64) -> [(End, Vec2, f32); 6] {
    let [right, up, back] = basis(yaw, pitch);
    End::ALL.map(|end| {
        let d = end.direction();
        let at = vec2(dot(d, right) as f32, -(dot(d, up) as f32)) * ARM;
        (end, at, dot(d, back) as f32)
    })
}

/// The end under `p` (relative to the centre), the one nearest the eye
/// first; None off every end (a drag there still orbits).
pub fn hit(yaw: f64, pitch: f64, p: Vec2) -> Option<End> {
    layout(yaw, pitch)
        .into_iter()
        .filter(|(end, at, _)| (p - *at).length() <= if end.positive() { END_R } else { NEG_R } + SLOP)
        .max_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(end, _, _)| end)
}

/// Is `p` (relative to the centre) on the gizmo? Outside, the model has it.
pub fn inside(p: Vec2) -> bool {
    p.length() <= GIZMO_R
}

const SAME: f64 = 1e-3;
fn same_angle(a: f64, b: f64) -> bool {
    let d = (a - b).rem_euclid(std::f64::consts::TAU);
    d.min(std::f64::consts::TAU - d) <= SAME
}

/// The standard view a camera at (`yaw`, `pitch`) shows, if any.
pub fn aligned_view(yaw: f64, pitch: f64) -> Option<&'static str> {
    use std::f64::consts::FRAC_PI_2;
    if pitch >= FRAC_PI_2 - SAME {
        return Some("top");
    }
    if pitch <= -FRAC_PI_2 + SAME {
        return Some("bottom");
    }
    if pitch.abs() <= SAME {
        for (deg, name) in [(-90., "front"), (90., "back"), (0., "right"), (180., "left")] {
            if same_angle(yaw, f64::to_radians(deg)) {
                return Some(name);
            }
        }
    }
    if (pitch.abs() - 30f64.to_radians()).abs() <= SAME
        && [-45., 45., 135., 225.].iter().any(|d: &f64| same_angle(yaw, d.to_radians()))
    {
        return Some("iso");
    }
    None
}

/// The view a click on `end` asks for: its own, or the opposite one when
/// the camera already looks from there.
pub fn click_view(end: End, aligned: Option<&str>) -> &'static str {
    if aligned == Some(end.view()) { end.opposite().view() } else { end.view() }
}

/// "Front · Parallel", "Perspective", "Top · Parallel · Hidden line": the
/// view (when standard), the projection and a style other than the default.
pub fn view_label(view: Option<&str>, projection: &str, style: &str, tr: impl Fn(&str) -> String) -> String {
    let view = view.map(|v| match v {
        "top" => "Top",
        "bottom" => "Bottom",
        "front" => "Front",
        "back" => "Back",
        "right" => "Right",
        "left" => "Left",
        _ => "Isometric",
    });
    let projection = match projection {
        "parallel" => "Parallel",
        "two_point" => "Two-point",
        _ => "Perspective",
    };
    let style = (!style.is_empty() && style != "Default").then_some(style);
    view.into_iter().chain([projection]).chain(style).map(tr).collect::<Vec<_>>().join(" · ")
}

/// What the gizmo did this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GizmoEvent {
    View(&'static str),
    Orbit(Vec2),
}

/// Paint the gizmo centred on `centre` and read its input. `names` gives
/// the hovered end's tooltip (the view's translated name).
pub fn view_gizmo(
    ui: &mut egui::Ui,
    centre: Pos2,
    yaw: f64,
    pitch: f64,
    names: impl Fn(&str) -> String,
) -> Option<GizmoEvent> {
    let rect = Rect::from_center_size(centre, Vec2::splat(2. * GIZMO_R));
    let response = ui.interact(rect, ui.id().with("view gizmo"), Sense::click_and_drag());
    let pointer = response.hover_pos().or(response.interact_pointer_pos()).map(|p| p - centre);
    let over = pointer.is_some_and(inside) || response.dragged();
    let hovered = pointer.filter(|p| inside(*p) && !response.dragged()).and_then(|p| hit(yaw, pitch, p));
    let painter = ui.painter_at(rect.expand(2.));
    if over {
        painter.circle_filled(centre, GIZMO_R, GIZMO_DISC);
    }
    let mut ends = layout(yaw, pitch);
    ends.sort_by(|a, b| a.2.total_cmp(&b.2));
    for (end, at, depth) in ends {
        // Ends facing away fade a little: the gizmo reads in depth.
        let fade = 0.6 + 0.4 * (depth + 1.) / 2.;
        let colour = end.colour().gamma_multiply(fade);
        let p = centre + at;
        let lit = hovered == Some(end);
        if end.positive() {
            painter.line_segment([centre, p], Stroke::new(2.0_f32, colour));
            painter.circle_filled(p, END_R, colour);
            if lit {
                painter.circle_stroke(p, END_R + 1., Stroke::new(1.5_f32, BRIGHT));
            }
            painter.text(p, Align2::CENTER_CENTER, end.letter(), strong(TYPE_META), BRIGHT);
        } else {
            painter.circle_filled(p, NEG_R, if lit { colour } else { colour.gamma_multiply(0.25) });
            painter.circle_stroke(p, NEG_R, Stroke::new(1.5_f32, colour));
            if lit {
                painter.text(p, Align2::CENTER_CENTER, format!("−{}", end.letter()), meta(), BRIGHT);
            }
        }
    }
    if let Some(end) = hovered {
        response.clone().on_hover_text_at_pointer(names(end.view()));
    }
    if response.dragged() {
        let d = response.drag_delta();
        if d != Vec2::ZERO {
            return Some(GizmoEvent::Orbit(d));
        }
    } else if response.clicked() {
        let at = response.interact_pointer_pos()? - centre;
        if let Some(end) = inside(at).then(|| hit(yaw, pitch, at)).flatten() {
            return Some(GizmoEvent::View(click_view(end, aligned_view(yaw, pitch))));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    const FREE: (f64, f64) = (-0.785398, 0.523599);
    const FRONT: (f64, f64) = (-std::f64::consts::FRAC_PI_2, 0.);
    const TOP: (f64, f64) = (-std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);

    fn at(view: (f64, f64), end: End) -> Vec2 {
        layout(view.0, view.1).into_iter().find(|e| e.0 == end).unwrap().1
    }

    #[test]
    fn the_front_view_shows_x_right_and_z_up() {
        let x = at(FRONT, End::PosX);
        let z = at(FRONT, End::PosZ);
        assert!((x - vec2(ARM, 0.)).length() < 1e-4, "{x:?}");
        assert!((z - vec2(0., -ARM)).length() < 1e-4, "{z:?}");
        // Y points away from a front view, at the centre.
        let (_, y, depth) = layout(FRONT.0, FRONT.1)[2];
        assert!(y.length() < 1e-4 && depth < -0.99);
    }

    #[test]
    fn every_end_is_hit_where_it_is_drawn() {
        for end in End::ALL {
            assert_eq!(hit(FREE.0, FREE.1, at(FREE, end)), Some(end), "{end:?}");
            // A little off its centre still takes it.
            assert_eq!(hit(FREE.0, FREE.1, at(FREE, end) + vec2(3., -2.)), Some(end));
        }
        // Between the ends, nothing (a drag there orbits).
        assert_eq!(hit(FREE.0, FREE.1, vec2(0., 0.)), None);
    }

    #[test]
    fn the_end_nearer_the_eye_wins_where_two_overlap() {
        // From the top, +Z faces the eye over -Z at the centre.
        assert_eq!(hit(TOP.0, TOP.1, Vec2::ZERO), Some(End::PosZ));
        assert_eq!(hit(FRONT.0, FRONT.1, Vec2::ZERO), Some(End::NegY));
    }

    #[test]
    fn each_end_names_its_view_and_the_aligned_end_flips() {
        let views: Vec<_> = End::ALL.iter().map(|e| click_view(*e, None)).collect();
        assert_eq!(views, ["right", "left", "back", "front", "top", "bottom"]);
        assert_eq!(click_view(End::PosZ, Some("top")), "bottom");
        assert_eq!(click_view(End::NegY, Some("front")), "back");
        assert_eq!(click_view(End::PosX, Some("top")), "right");
    }

    #[test]
    fn clicks_outside_the_circle_pass_through() {
        assert!(inside(vec2(GIZMO_R - 1., 0.)));
        // The square's corner is the model's.
        assert!(!inside(vec2(GIZMO_R - 2., GIZMO_R - 2.)));
        assert!(!inside(vec2(0., GIZMO_R + 1.)));
    }

    #[test]
    fn standard_angles_name_their_view() {
        assert_eq!(aligned_view(FRONT.0, FRONT.1), Some("front"));
        assert_eq!(aligned_view(TOP.0, TOP.1), Some("top"));
        assert_eq!(aligned_view(0.3, -std::f64::consts::FRAC_PI_2), Some("bottom"));
        assert_eq!(aligned_view(std::f64::consts::PI, 0.), Some("left"));
        assert_eq!(aligned_view(-std::f64::consts::PI, 0.), Some("left"));
        assert_eq!(aligned_view(FREE.0, FREE.1), Some("iso"));
        assert_eq!(aligned_view(0.2, 0.4), None);
        // An orbit stops a degree short of the pole: not Top any more.
        assert_eq!(aligned_view(TOP.0, 89f64.to_radians()), None);
    }

    #[test]
    fn the_label_names_view_projection_and_a_non_default_style() {
        let id = |s: &str| s.to_owned();
        assert_eq!(view_label(Some("front"), "parallel", "Default", id), "Front · Parallel");
        assert_eq!(view_label(None, "perspective", "", id), "Perspective");
        assert_eq!(view_label(Some("top"), "parallel", "Hidden line", id), "Top · Parallel · Hidden line");
        assert_eq!(view_label(Some("iso"), "two_point", "Default", |s| s.to_uppercase()), "ISOMETRIC · TWO-POINT");
    }
}
