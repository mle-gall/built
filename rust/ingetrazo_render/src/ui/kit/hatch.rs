//! Vector geometry of an 8 × 3 mm sample at the tag's angle and paper scale.
//! The catalogue contains clipped material marks and, for ruled patterns, the
//! original line definitions from core.hatches.line_families (PAT format).
use egui::{emath::Rot2, vec2, Vec2};
use serde::Deserialize;
use std::{collections::HashMap, sync::LazyLock};

const SIZE: Vec2 = vec2(8., 3.);
const CORNERS: [Vec2; 4] = [Vec2::ZERO, vec2(8., 0.), SIZE, vec2(0., 3.)];

#[derive(Default, Deserialize)]
#[serde(default)]
pub struct PaperMarks {
    pub(super) lines: Vec<[f32; 4]>,
    pub(super) dots: Vec<[f32; 2]>,
    pub(super) circles: Vec<[f32; 3]>,
    families: Vec<Family>,
}

#[derive(Deserialize)]
struct Family(f32, f32, f32, f32, f32, Vec<f32>);

pub(super) fn standard(key: &str) -> Option<&'static PaperMarks> {
    static MARKS: LazyLock<HashMap<String, PaperMarks>> = LazyLock::new(|| {
        serde_json::from_str(include_str!("../assets/hatches.json")).expect("standard cut patterns")
    });
    MARKS.get(key)
}

pub(in crate::ui) fn hatch_override(cells: &[String]) -> Option<PaperMarks> {
    let key = cells.get(4)?;
    let turn: f32 = cells.get(8)?.parse().ok()?;
    let scale = cells.get(9)?.parse::<f32>().ok()?.clamp(0.05, 20.);
    let imported: Option<Vec<Family>> = cells.get(10).and_then(|s| serde_json::from_str(s).ok());
    if (turn == 0. && scale == 1. && imported.is_none()) || key.is_empty() || key == "solid" {
        return None;
    }
    let base = standard(key);
    let families = imported
        .as_deref()
        .or_else(|| base.map(|m| &m.families[..]))?;
    let rotation = Rot2::from_angle(turn.to_radians());
    let mut out = PaperMarks::default();
    if families.is_empty() {
        repeat_material(base?, rotation, scale, &mut out);
    } else {
        for family in families.iter().take(64) {
            family.sample(rotation, scale, &mut out);
        }
    }
    Some(out)
}

pub(in crate::ui) fn imported_hatch(value: &serde_json::Value) -> Option<PaperMarks> {
    let mut output = PaperMarks::default();
    for raw in value.as_array()?.iter().take(64) {
        if let Ok(family) = serde_json::from_value::<Family>(raw.clone()) {
            family.sample(Rot2::IDENTITY, 1., &mut output);
        }
    }
    Some(output)
}

fn extent(along: Vec2, origin: Vec2) -> (f32, f32) {
    let ts = CORNERS.map(|p| (p - origin).dot(along));
    (
        ts.into_iter().fold(f32::INFINITY, f32::min),
        ts.into_iter().fold(f32::NEG_INFINITY, f32::max),
    )
}

impl Family {
    fn sample(&self, rotation: Rot2, scale: f32, out: &mut PaperMarks) {
        let Self(angle, x, y, along, across, dashes) = self;
        if across.abs() < 0.05 {
            return;
        }
        let u = rotation * Vec2::angled(angle.to_radians());
        let n = vec2(-u.y, u.x);
        let origin = rotation * vec2(*x, *y) * scale;
        let step = (u * *along + n * *across) * scale;
        let (a, b) = extent(n, origin);
        let (a, b) = (a / (scale * across), b / (scale * across));
        let first = a.min(b).ceil();
        let last = a.max(b).floor();
        let dashes = &dashes[..dashes.len().min(32)];
        let period = scale * dashes.iter().map(|d| d.abs()).sum::<f32>();
        for row in 0..((last - first + 1.) as usize).min(4096) {
            if out.lines.len() + out.dots.len() >= 20000 {
                return;
            }
            let start = origin + step * (first + row as f32);
            let (lo, hi) = extent(u, start);
            let segment = |a, b| {
                let p = start + u * a;
                let q = start + u * b;
                [p.x, p.y, q.x, q.y]
            };
            if period <= f32::EPSILON {
                out.lines.push(segment(lo, hi));
                continue;
            }
            let first_dash = (lo / period).floor() * period;
            let repeats = ((hi - first_dash) / period).ceil() as usize;
            if repeats.saturating_mul(dashes.len()) > 4096 {
                continue;
            }
            for repeat in 0..repeats {
                let mut t = first_dash + repeat as f32 * period;
                for d in dashes {
                    let next = t + d.abs() * scale;
                    if *d > 0. && next > lo && t < hi {
                        out.lines.push(segment(t.max(lo), next.min(hi)));
                    } else if *d == 0. && (lo..=hi).contains(&t) {
                        out.dots.push((start + u * t).into());
                    }
                    t = next;
                }
            }
        }
    }
}

// Scattered and curved materials repeat a paper sample. Invert the tag's
// transform to visit only tiles that intersect the swatch's bounding box.
fn repeat_material(base: &PaperMarks, rotation: Rot2, scale: f32, out: &mut PaperMarks) {
    let (xl, xr) = extent(rotation * Vec2::X, Vec2::ZERO);
    let (yt, yb) = extent(rotation * Vec2::Y, Vec2::ZERO);
    for x in (xl / (SIZE.x * scale)).floor() as i32..(xr / (SIZE.x * scale)).ceil() as i32 {
        for y in (yt / (SIZE.y * scale)).floor() as i32..(yb / (SIZE.y * scale)).ceil() as i32 {
            let origin = vec2(x as f32 * SIZE.x, y as f32 * SIZE.y);
            let point = |x, y| rotation * (origin + vec2(x, y)) * scale;
            for &[x, y, u, v] in &base.lines {
                let (p, q) = (point(x, y), point(u, v));
                out.lines.push([p.x, p.y, q.x, q.y]);
            }
            for &[x, y] in &base.dots {
                out.dots.push(point(x, y).into());
            }
            for &[x, y, r] in &base.circles {
                let p = point(x, y);
                out.circles.push([p.x, p.y, r * scale]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_horizontal_family_turns_and_keeps_its_spacing() {
        let mut cells = vec![String::new(); 11];
        cells[4] = "pat:sample".into();
        cells[8] = "90".into();
        cells[9] = "2".into();
        cells[10] = "[[0,0,0,0,1,[]]]".into();
        let marks = hatch_override(&cells).unwrap();
        for x in [2., 4., 6.] {
            assert!(marks
                .lines
                .iter()
                .any(|line| (line[0] - x).abs() < 0.001 && (line[2] - x).abs() < 0.001));
        }
        assert!(marks
            .lines
            .iter()
            .all(|line| (line[0] - line[2]).abs() < 0.001));
    }

    #[test]
    fn every_standard_hatch_has_marks_or_a_solid_fill() {
        for key in super::super::HATCH_KEYS.lines() {
            let marks = standard(key).unwrap();
            assert!(
                key == "solid"
                    || !(marks.lines.is_empty()
                        && marks.circles.is_empty()
                        && marks.dots.is_empty()),
                "{key}"
            );
        }
    }
}
