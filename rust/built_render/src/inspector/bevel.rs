//! Edits to a bevel outline, in cross-section coordinates. The end points
//! are the two cuts on the adjacent faces and cannot be edited.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Point(pub f64, pub f64, pub bool);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Curve(Vec<Point>);

impl Curve {
    pub fn points(&self) -> &[Point] {
        &self.0
    }

    pub fn insert(&mut self, x: f64, y: f64, smooth: bool) -> Option<usize> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let (x, y) = (x.clamp(0., 1.), y.clamp(0., 1.));
        let distance = |pair: &[Point]| {
            let Point(ax, ay, _) = pair[0];
            let Point(bx, by, _) = pair[1];
            let (dx, dy) = (bx - ax, by - ay);
            let length = dx * dx + dy * dy;
            let t = if length == 0. {
                0.
            } else {
                (((x - ax) * dx + (y - ay) * dy) / length).clamp(0., 1.)
            };
            (x - ax - t * dx).hypot(y - ay - t * dy)
        };
        let index = self
            .0
            .windows(2)
            .enumerate()
            .min_by(|(_, a), (_, b)| distance(a).total_cmp(&distance(b)))?
            .0
            + 1;
        self.0.insert(index, Point(x, y, smooth));
        Some(index)
    }

    pub fn interior(&self, i: usize) -> bool {
        (1..self.0.len().saturating_sub(1)).contains(&i)
    }

    pub fn remove(&mut self, i: usize) -> bool {
        if !self.interior(i) {
            return false;
        }
        self.0.remove(i);
        true
    }

    pub fn set(&mut self, i: usize, Point(x, y, smooth): Point) -> bool {
        if !self.interior(i) || !x.is_finite() || !y.is_finite() {
            return false;
        }
        let next = Point(x.clamp(0., 1.), y.clamp(0., 1.), smooth);
        let changed = self.0[i] != next;
        self.0[i] = next;
        changed
    }
}

pub(super) fn shape_name(snapshot: &serde_json::Value) -> &str {
    let shape = &snapshot["shape"];
    if shape.is_array() {
        return "drawn";
    }
    if let Some(name) = shape.as_str() {
        return name;
    }
    match snapshot["profile"].as_f64() {
        Some(0.5) => "round",
        Some(0.25) => "chamfer",
        _ => "profile value",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn diagonal() -> Curve {
        Curve(vec![Point(1., 0., false), Point(0., 1., false)])
    }

    #[test]
    fn adds_toggles_and_removes_an_intermediate_point() {
        let mut c = diagonal();
        let i = c.insert(0.6, 0.6, true).unwrap();
        assert_eq!(i, 1);
        assert!(c.set(i, Point(0.6, 0.6, false)));
        assert!(!c.points()[i].2);
        assert!(c.remove(i));
        assert_eq!(c.points(), diagonal().points());
    }

    #[test]
    fn keeps_ends_and_clamps_dragging() {
        let mut c = diagonal();
        let i = c.insert(0.5, 0.5, true).unwrap();
        assert!(!c.remove(0) && !c.remove(2));
        assert!(!c.set(0, Point(0., 0., true)));
        assert!(c.set(i, Point(1.5, -0.5, true)));
        assert_eq!(c.points()[i], Point(1., 0., true));
    }

    #[test]
    fn inserts_the_chosen_kind_on_the_nearest_segment() {
        let mut c = diagonal();
        c.insert(0.6, 0.6, false);
        c.insert(0.3, 0.8, true);
        assert_eq!(
            c.points(),
            &[
                Point(1., 0., false),
                Point(0.6, 0.6, false),
                Point(0.3, 0.8, true),
                Point(0., 1., false)
            ]
        );
    }
}
