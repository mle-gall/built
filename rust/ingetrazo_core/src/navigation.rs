// SPDX-License-Identifier: GPL-3.0-or-later
//! Binary64 navigation in the model's Z-up frame. Screen deltas are logical pixels.
use crate::math::{xyz, Vec3};
use crate::model::pyfloat::{cos, sin};
use glam::{DQuat, DVec3};
use pyo3::prelude::*;

fn eye_offset(yaw: f64, pitch: f64, distance: f64) -> DVec3 {
    let horizontal = distance * cos(pitch);
    DVec3::new(
        horizontal * cos(yaw),
        horizontal * sin(yaw),
        distance * sin(pitch),
    )
}

#[pyfunction]
pub(crate) fn orbit_angles(yaw: f64, pitch: f64, dx: f64, dy: f64, height: f64) -> (f64, f64) {
    let scale = std::f64::consts::PI / height.max(1.0);
    let limit = 89_f64.to_radians();
    (yaw - dx * scale, (pitch + dy * scale).clamp(-limit, limit))
}

#[pyfunction]
pub(crate) fn orbit_target(
    target: &Vec3,
    pivot: &Bound<'_, PyAny>,
    old_yaw: f64,
    old_pitch: f64,
    yaw: f64,
    pitch: f64,
) -> PyResult<Vec3> {
    let pivot = xyz(pivot)?;
    let target = DVec3::from_array(target.coords());
    if !pivot.is_finite() || ((yaw - old_yaw).abs() < 1e-12 && (pitch - old_pitch).abs() < 1e-12) {
        return Ok(target.into());
    }
    let yaw_half = (yaw - old_yaw) * 0.5;
    let yaw_turn = DQuat::from_xyzw(0., 0., sin(yaw_half), cos(yaw_half));
    let axis = eye_offset(yaw, old_pitch, 1.0).cross(DVec3::Z);
    let turn = if axis.length() < 1e-9 {
        yaw_turn
    } else {
        let pitch_half = (pitch - old_pitch) * 0.5;
        let v = axis.normalize() * sin(pitch_half);
        DQuat::from_xyzw(v.x, v.y, v.z, cos(pitch_half)) * yaw_turn
    };
    Ok((pivot + turn * (target - pivot)).into())
}

#[pyfunction]
pub(crate) fn pan_target(
    target: &Vec3,
    up: &Vec3,
    yaw: f64,
    pitch: f64,
    span: f64,
    fov_deg: f64,
    dx: f64,
    dy: f64,
    height: f64,
) -> Vec3 {
    let forward = -eye_offset(yaw, pitch, 1.0);
    let right = forward
        .cross(DVec3::from_array(up.coords()))
        .normalize_or_zero();
    let screen_up = right.cross(forward).normalize_or_zero();
    let scale = 2.0 * span * (fov_deg.to_radians() * 0.5).tan() / height.max(1.0);
    (DVec3::from_array(target.coords()) - right * (dx * scale) + screen_up * (dy * scale)).into()
}

#[pyfunction]
pub(crate) fn zoom_distance(distance: f64, steps: f64, minimum: f64, maximum: f64) -> f64 {
    (distance * 0.9_f64.powf(steps)).min(maximum).max(minimum)
}

#[pyfunction]
pub(crate) fn zoom_focus(
    target: &Vec3,
    focus: &Bound<'_, PyAny>,
    eye: &Vec3,
    yaw: f64,
    pitch: f64,
    distance: f64,
    steps: f64,
    minimum: f64,
    maximum: f64,
    retreat: f64,
) -> PyResult<(Vec3, f64)> {
    let target = DVec3::from_array(target.coords());
    let focus = xyz(focus)?;
    let eye = DVec3::from_array(eye.coords());
    let mut factor = 0.9_f64.powf(steps);
    let mut next_distance = zoom_distance(distance, steps, minimum, maximum);
    if steps < 0.0 && distance > 0.0 {
        factor = factor.min(next_distance / distance);
    }
    let mut next_target = focus + (target - focus) * factor;
    if steps < 0.0 && retreat > 0.0 && next_distance < maximum {
        let moved = (next_target + eye_offset(yaw, pitch, next_distance)).distance(eye);
        if moved < retreat {
            // The floor is a longer step of the same zoom: the eye and the
            // target scale about the focus together, so the focus keeps its
            // pixel. Adding the shortfall to the distance alone pulled the
            // eye back along the view axis and slid a focus beside the
            // centre of the window toward the centre at every step.
            let reach = eye.distance(focus);
            let longer = 1.0 + retreat / reach;
            if distance > 0.0 && longer.is_finite() {
                let scale = longer.min(maximum / distance);
                next_distance = (distance * scale).min(maximum).max(minimum);
                next_target = focus + (target - focus) * scale;
            } else {
                // The eye is on the focus: there is no direction to scale along.
                next_distance = (next_distance + (retreat - moved)).min(maximum);
            }
        }
    }
    Ok((next_target.into(), next_distance))
}

/// The camera framing a box into part of the window (review council B8,
/// owner decision 2026-11-23: Zoom Extents, Zoom Selection and Review frame
/// the model into the area the panels leave free).
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub right: DVec3,
    pub up: DVec3,
    pub forward: DVec3,
    /// tan(fov / 2) and width / height of the window.
    pub tan_half: f64,
    pub aspect: f64,
    pub perspective: bool,
    /// The camera's frame shift (`shift_x`, `shift_y`, fractions of the frame).
    pub shift: (f64, f64),
}

impl Frame {
    /// The corners' extent in normalised device coordinates for a pose:
    /// (min x, max x, min y, max y), or None when a corner is behind the eye.
    fn extent(&self, corners: &[DVec3; 8], target: DVec3, distance: f64) -> Option<[f64; 4]> {
        let eye = target - self.forward * distance;
        let half_h = distance * self.tan_half;
        let mut out = [f64::MAX, f64::MIN, f64::MAX, f64::MIN];
        for &c in corners {
            let (x, y) = if self.perspective {
                let rel = c - eye;
                let z = rel.dot(self.forward);
                if z <= 1e-9 {
                    return None;
                }
                (rel.dot(self.right) / (z * self.tan_half * self.aspect), rel.dot(self.up) / (z * self.tan_half))
            } else {
                let rel = c - target;
                (rel.dot(self.right) / (half_h * self.aspect), rel.dot(self.up) / half_h)
            };
            let (x, y) = (x - 2.0 * self.shift.0, y - 2.0 * self.shift.1);
            out = [out[0].min(x), out[1].max(x), out[2].min(y), out[3].max(y)];
        }
        Some(out)
    }

    /// Move the pose so the box's projection is centred in `free` (NDC:
    /// x0, y0, x1, y1, y up) and its wider half-span is `1 / margin` of the
    /// free area's: the same rule as a whole-window fit, applied to the
    /// free rectangle. Parallel views settle in one step; perspective ones
    /// walk in like `OrbitCamera._settle_perspective_fit`.
    pub fn frame_into(&self, lo: DVec3, hi: DVec3, mut target: DVec3, mut distance: f64,
                      free: [f64; 4], margin: f64, floor: f64, maximum: f64) -> (DVec3, f64) {
        let corners = std::array::from_fn(|i| DVec3::new(
            if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z }));
        let (fx, fy) = ((free[0] + free[2]) / 2.0, (free[1] + free[3]) / 2.0);
        let (gw, gh) = ((free[2] - free[0]).abs() / 2.0 / margin, (free[3] - free[1]).abs() / 2.0 / margin);
        if gw <= 1e-9 || gh <= 1e-9 {
            return (target, distance);
        }
        let centre = (lo + hi) / 2.0;
        for _ in 0..32 {
            let Some([x0, x1, y0, y1]) = self.extent(&corners, target, distance) else {
                // A corner behind the eye: back off and measure again.
                distance = (distance * 1.5).clamp(floor, maximum);
                continue;
            };
            let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            let scale = ((x1 - x0) / 2.0 / gw).max((y1 - y0) / 2.0 / gh);
            if scale <= 0.0 || !scale.is_finite() {
                break;
            }
            if (cx - fx).abs() < 1e-3 && (cy - fy).abs() < 1e-3 && (scale - 1.0).abs() < 2e-3 {
                break;
            }
            let half_h = distance * self.tan_half;
            if self.perspective {
                // Slide the target so the box's centre lands on the free
                // centre (measured at the box centre's depth, so a tall box
                // seen from above does not overshoot), then walk in or out by
                // how far the nearest corner allows.
                let eye = target - self.forward * distance;
                let depth = corners.iter().map(|c| (c - eye).dot(self.forward)).fold(f64::MAX, f64::min)
                    .max((centre - eye).dot(self.forward) * 0.25).max(1e-6);
                let half_c = depth * self.tan_half;
                target += self.right * ((cx - fx) * half_c * self.aspect) + self.up * ((cy - fy) * half_c);
                let nearest = corners.iter().map(|c| -(c - target).dot(self.forward)).fold(f64::MIN, f64::max);
                let gap = distance - nearest;
                distance = (nearest + gap * scale).clamp(floor, maximum);
            } else {
                let next = (distance * scale).clamp(floor, maximum);
                let next_h = next * self.tan_half;
                target += self.right * (cx * half_h - fx * next_h) * self.aspect + self.up * (cy * half_h - fy * next_h);
                distance = next;
            }
        }
        (target, distance)
    }
}

#[pyfunction]
#[pyo3(signature = (lo, hi, target, distance, right, up, forward, fov_deg, aspect, perspective, shift, free, margin, floor, maximum))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn frame_box(
    lo: &Bound<'_, PyAny>, hi: &Bound<'_, PyAny>, target: &Bound<'_, PyAny>, distance: f64,
    right: &Bound<'_, PyAny>, up: &Bound<'_, PyAny>, forward: &Bound<'_, PyAny>,
    fov_deg: f64, aspect: f64, perspective: bool, shift: (f64, f64), free: [f64; 4],
    margin: f64, floor: f64, maximum: f64,
) -> PyResult<(Vec3, f64)> {
    let frame = Frame {
        right: xyz(right)?.normalize_or_zero(), up: xyz(up)?.normalize_or_zero(), forward: xyz(forward)?.normalize_or_zero(),
        tan_half: (fov_deg.to_radians() / 2.0).tan(), aspect: if aspect > 1e-6 { aspect } else { 1.0 }, perspective, shift,
    };
    let (t, d) = frame.frame_into(xyz(lo)?, xyz(hi)?, xyz(target)?, distance, free, margin, floor, maximum);
    Ok((t.into(), d))
}

pub fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(frame_box, module)?)?;
    module.add_function(wrap_pyfunction!(orbit_angles, module)?)?;
    module.add_function(wrap_pyfunction!(orbit_target, module)?)?;
    module.add_function(wrap_pyfunction!(pan_target, module)?)?;
    module.add_function(wrap_pyfunction!(zoom_distance, module)?)?;
    module.add_function(wrap_pyfunction!(zoom_focus, module)?)?;
    Ok(())
}

#[cfg(test)]
mod frame_tests {
    use super::*;

    fn iso(perspective: bool) -> Frame {
        let forward = DVec3::new(-1.0, 1.0, -1.0).normalize();
        let right = forward.cross(DVec3::Z).normalize();
        Frame { right, up: right.cross(forward), forward, tan_half: (30f64.to_radians()).tan(), aspect: 1.6,
                perspective, shift: (0.0, 0.0) }
    }

    fn top() -> Frame {
        Frame { right: DVec3::X, up: DVec3::Y, forward: -DVec3::Z, tan_half: (30f64.to_radians()).tan(), aspect: 1.6,
                perspective: true, shift: (0.0, 0.0) }
    }

    #[test]
    fn the_box_fills_the_free_area_not_the_window() {
        // A window with the rail on the left and the dock on the right: the
        // free area is x in [-0.8, 0.4] in device coordinates.
        let free = [-0.8, -0.85, 0.4, 0.9];
        let boxed = (DVec3::new(0.0, 0.0, 0.0), DVec3::new(10.0, 4.0, 3.0));
        let tower = (DVec3::ZERO, DVec3::new(5.0, 5.0, 40.0));
        for (perspective, f, (lo, hi)) in [(false, iso(false), boxed), (true, iso(true), boxed), (true, top(), tower)] {
            // From a whole-window fit's pose (the target at the box centre).
            let (t, d) = f.frame_into(lo, hi, (lo + hi) / 2.0, 120.0, free, 1.1, 0.01, 1e4);
            let corners = std::array::from_fn(|i| DVec3::new(
                if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z }));
            let [x0, x1, y0, y1] = f.extent(&corners, t, d).unwrap();
            assert!(x0 >= free[0] - 1e-3 && x1 <= free[2] + 1e-3, "{perspective}: x {x0}..{x1}");
            assert!(y0 >= free[1] - 1e-3 && y1 <= free[3] + 1e-3, "{perspective}: y {y0}..{y1}");
            let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            assert!((cx - -0.2).abs() < 2e-3 && (cy - 0.025).abs() < 2e-3, "{perspective}: centre {cx} {cy}");
            // The binding side spans 1 / margin of the free area.
            let fill = ((x1 - x0) / 1.2).max((y1 - y0) / 1.75);
            assert!((fill - 1.0 / 1.1).abs() < 5e-3, "{perspective}: fill {fill}");
        }
    }
}
