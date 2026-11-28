// SPDX-License-Identifier: GPL-3.0-or-later
//! Thumbnails of drawn sketches for the Library (Sections, drawn Profiles):
//! rendered from the sketch itself, never from a stored picture, so a tile
//! always shows the drawing as it is now.
//!
//! A tile's thumbnail key is `sketch:<hash of the drawing>`. The same
//! drawing keeps its key, so its pixels and its texture are made once and
//! reused (the cache below, and the view's own set of uploaded icons); an
//! edited drawing has another key and is rendered again.
use built_draft2d::{Shape, Sketch};
use glam::DVec2;
use serde_json::Value;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

/// The drawings behind the keys handed out, and the pixels made for them.
#[derive(Default)]
struct Cache {
    sketches: HashMap<String, Value>,
    pixels: HashMap<(String, usize), Vec<u8>>,
    rendered: u64,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);
const LIMIT: usize = 512;

/// Only what is drawn: a model row carries more than a sketch reads.
fn drawn(sketch: &Value) -> Option<Sketch> {
    Sketch::from_json(&serde_json::json!({
        "use": sketch["use"].as_str().unwrap_or("profile"),
        "curves": sketch["curves"],
        "next_id": sketch["next_id"].as_u64().unwrap_or(1_000_000),
    }))
    .ok()
}

/// What a drawing looks like: its curves, nothing of its name, plane or
/// bookkeeping, and not how its numbers happen to be written (40 or 40.0).
fn fingerprint(sketch: &Value) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    sketch["use"].as_str().unwrap_or("profile").hash(&mut h);
    match drawn(sketch) {
        Some(s) => {
            for curve in &s.curves {
                let numbers: Vec<f64> = match curve.shape {
                    Shape::Line { a, b } => vec![0.0, a.x, a.y, b.x, b.y],
                    Shape::Arc { c, r, a0, a1 } => vec![1.0, c.x, c.y, r, a0, a1],
                    Shape::Circle { c, r } => vec![2.0, c.x, c.y, r],
                };
                for n in numbers {
                    // -0.0 and 0.0 draw the same.
                    (n + 0.0).to_bits().hash(&mut h);
                }
            }
        }
        None => sketch["curves"].to_string().hash(&mut h),
    }
    h.finish()
}

/// The thumbnail key of a sketch (a row body or a profile file's `sketch`);
/// None when it has no curves to draw.
pub fn key(sketch: &Value) -> Option<String> {
    if !sketch["curves"].as_array().is_some_and(|c| !c.is_empty()) {
        return None;
    }
    let key = format!("sketch:{:016x}", fingerprint(sketch));
    let mut guard = CACHE.lock().ok()?;
    let cache = guard.get_or_insert_with(Cache::default);
    if !cache.sketches.contains_key(&key) {
        if cache.sketches.len() >= LIMIT {
            cache.sketches.clear();
            cache.pixels.clear();
        }
        cache.sketches.insert(key.clone(), sketch.clone());
    }
    Some(key)
}

/// The pixels of a key from `key()`: `px` × `px`, RGBA, straight alpha.
pub fn pixels(key: &str, px: usize) -> Option<Vec<u8>> {
    let sketch = {
        let mut guard = CACHE.lock().ok()?;
        let cache = guard.get_or_insert_with(Cache::default);
        if let Some(hit) = cache.pixels.get(&(key.to_owned(), px)) {
            return Some(hit.clone());
        }
        cache.sketches.get(key)?.clone()
    };
    let out = render(&sketch, px)?;
    if let Ok(mut guard) = CACHE.lock() {
        let cache = guard.get_or_insert_with(Cache::default);
        cache.rendered += 1;
        cache.pixels.insert((key.to_owned(), px), out.clone());
    }
    Some(out)
}

/// How many thumbnails were rendered (not served from the cache) so far.
pub fn rendered() -> u64 {
    CACHE.lock().ok().and_then(|g| g.as_ref().map(|c| c.rendered)).unwrap_or(0)
}

/// Draw `sketch` into `px` × `px`: its outline, its closed regions filled.
pub fn render(sketch: &Value, px: usize) -> Option<Vec<u8>> {
    if !(8..=1024).contains(&px) {
        return None;
    }
    let s = drawn(sketch)?;
    let mut segs: Vec<[DVec2; 2]> = Vec::new();
    for curve in &s.curves {
        match curve.shape {
            Shape::Line { a, b } => segs.push([a, b]),
            Shape::Arc { c, r, a0, a1 } => arc_segments(&mut segs, c, r, a0, a1),
            Shape::Circle { c, r } => arc_segments(&mut segs, c, r, 0.0, std::f64::consts::TAU),
        }
    }
    if segs.is_empty() {
        return None;
    }
    let (mut lo, mut hi) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
    for p in segs.iter().flatten() {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    // Fitted to a band wider than it is tall: the detail pane shows the
    // middle of the tile, so a tall drawing must not lose its ends there.
    let size = (hi - lo).max(DVec2::splat(1e-9));
    let scale = (px as f64 * 0.76 / size.x).min(px as f64 * 0.52 / size.y);
    let mid = (lo + hi) * 0.5;
    let half = px as f64 / 2.0;
    // Paper space: x right, y down, the drawing centred.
    let to_px = |p: DVec2| DVec2::new(half + (p.x - mid.x) * scale, half - (p.y - mid.y) * scale);
    let lines: Vec<[DVec2; 2]> = segs.iter().map(|[a, b]| [to_px(*a), to_px(*b)]).collect();
    let closed = built_draft2d::profile(&s).ok();
    let stroke = (px as f64 / 96.0).max(1.0);
    const INK: [u8; 3] = [196, 200, 203];
    let mut out = vec![0u8; px * px * 4];
    for y in 0..px {
        let cy = y as f64 + 0.5;
        // The spans this row crosses, for the even-odd fill.
        let mut crossings: Vec<f64> = Vec::new();
        if closed {
            for [a, b] in &lines {
                if (a.y > cy) != (b.y > cy) {
                    crossings.push(a.x + (cy - a.y) / (b.y - a.y) * (b.x - a.x));
                }
            }
            crossings.sort_by(f64::total_cmp);
        }
        // Only the segments near this row can ink it.
        let near: Vec<&[DVec2; 2]> = lines.iter().filter(|[a, b]| a.y.min(b.y) - stroke - 1.0 <= cy && cy <= a.y.max(b.y) + stroke + 1.0).collect();
        for x in 0..px {
            let c = DVec2::new(x as f64 + 0.5, cy);
            let mut d = f64::INFINITY;
            for [a, b] in &near {
                let ab = *b - *a;
                let t = if ab.length_squared() > 0.0 { ((c - *a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
                d = d.min(c.distance(*a + ab * t));
            }
            let line = (stroke * 0.5 + 0.5 - d).clamp(0.0, 1.0);
            let inside = closed && crossings.iter().filter(|cx| **cx < c.x).count() % 2 == 1;
            let fill = if inside { 0.2 } else { 0.0 };
            let alpha = line + (1.0 - line) * fill;
            if alpha > 0.0 {
                let i = (y * px + x) * 4;
                out[i..i + 3].copy_from_slice(&INK);
                out[i + 3] = (alpha * 255.0).round() as u8;
            }
        }
    }
    Some(out)
}

fn arc_segments(out: &mut Vec<[DVec2; 2]>, c: DVec2, r: f64, a0: f64, a1: f64) {
    let n = (((a1 - a0).abs() / std::f64::consts::TAU * 72.0).ceil() as usize).clamp(2, 72);
    let at = |i: usize| c + DVec2::from_angle(a0 + (a1 - a0) * i as f64 / n as f64) * r;
    for i in 0..n {
        out.push([at(i), at(i + 1)]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rect(w: f64, h: f64) -> Value {
        let pts = [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]];
        let curves: Vec<Value> = (0..4).map(|i| json!({"id": i + 1, "kind": "line", "a": pts[i], "b": pts[(i + 1) % 4]})).collect();
        json!({"use": "section", "curves": curves, "next_id": 5})
    }

    fn alpha(p: &[u8], px: usize, x: usize, y: usize) -> u8 {
        p[(y * px + x) * 4 + 3]
    }

    #[test]
    fn a_closed_section_is_outlined_and_filled_and_keeps_its_proportions() {
        let p = render(&rect(200.0, 100.0), 96).unwrap();
        // The outline: 200 wide fills 76 % of the tile, half as high.
        assert_eq!(alpha(&p, 96, 48, 30), 255, "top edge");
        // A tall drawing is fitted to the band the detail pane shows.
        let tall = render(&rect(100.0, 400.0), 96).unwrap();
        assert_eq!(alpha(&tall, 96, 48, 20), 0, "above the band");
        assert!(alpha(&tall, 96, 48, 23) > 100, "its top edge, inside the band");
        assert_eq!(alpha(&p, 96, 12, 48), 255, "left edge");
        assert_eq!(alpha(&p, 96, 48, 48), 51, "filled inside");
        assert_eq!(alpha(&p, 96, 48, 12), 0, "empty above");
        assert_eq!(alpha(&p, 96, 3, 48), 0, "empty beside");
    }

    #[test]
    fn an_open_drawing_is_only_its_lines() {
        let mut open = rect(100.0, 100.0);
        open["curves"].as_array_mut().unwrap().pop();
        let p = render(&open, 64).unwrap();
        assert_eq!(alpha(&p, 64, 32, 32), 0);
        assert!(p.chunks(4).any(|c| c[3] == 255));
    }

    #[test]
    fn the_same_drawing_is_rendered_once_and_an_edit_renders_again() {
        let a = rect(310.0, 120.0);
        let key_a = key(&a).unwrap();
        let before = rendered();
        let first = pixels(&key_a, 64).unwrap();
        assert_eq!(pixels(&key_a, 64).unwrap(), first);
        // Renamed or saved again unchanged: the same key, nothing rendered.
        let mut renamed = a.clone();
        renamed["name"] = json!("Notch");
        assert_eq!(key(&renamed).unwrap(), key_a);
        assert_eq!(rendered() - before, 1);
        // Edited: another key, other pixels.
        let mut edited = a.clone();
        edited["curves"][1]["b"] = json!([310.0, 300.0]);
        edited["curves"][2]["a"] = json!([310.0, 300.0]);
        let key_b = key(&edited).unwrap();
        assert_ne!(key_b, key_a);
        assert_ne!(pixels(&key_b, 64).unwrap(), first);
        assert_eq!(rendered() - before, 2);
        assert!(key(&json!({"curves": []})).is_none());
    }

    #[test]
    fn arcs_and_circles_are_drawn_round() {
        let hole = json!({"use": "section", "curves": [{"id": 1, "kind": "circle", "c": [0.0, 0.0], "r": 50.0}], "next_id": 2});
        let p = render(&hole, 96).unwrap();
        assert_eq!(alpha(&p, 96, 48, 48), 51);
        assert_eq!(alpha(&p, 96, 14, 14), 0, "the corner outside the circle");
        assert!(alpha(&p, 96, 48, 12) > 200, "the top of the circle");
    }
}
