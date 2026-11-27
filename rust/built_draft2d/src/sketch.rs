// SPDX-License-Identifier: GPL-3.0-or-later
//! A sketch: curves with persistent sketch element ids, named points,
//! driving dimensions and its record (use, plane, extrude). The same JSON
//! is a live `sketch` row's body and a `.builtprofile` file's sketch.
use crate::geom::{Shape, EPS};
use glam::DVec2;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Use {
    /// A section to sweep along a path (the Sweep picker's list).
    #[default]
    Profile,
    /// A cutter section (the Cut picker's list).
    Section,
}

/// One entity. `id` is kept for life: trimming keeps it on the first
/// piece, new pieces and fillets take fresh ids. Generated faces take their
/// roles from it, so a sweep side face survives an edit of its profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    pub id: u32,
    #[serde(flatten)]
    pub shape: Shape,
    /// Facets per full turn for round parts (facets are geometry).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segments: Option<u32>,
}

/// A named point: an extra attachment point (a shear centre) or a datum.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NamedPoint {
    pub name: String,
    pub at: DVec2,
}

/// Where on a curve a dimension measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum At {
    Start,
    End,
    Mid,
    Centre,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointRef {
    pub curve: u32,
    pub at: At,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DimKind {
    /// A line's length.
    Length,
    /// Between two points, along `axis`.
    Distance,
    /// An arc's or a circle's radius.
    Radius,
    /// Between two lines, degrees.
    Angle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    #[default]
    Aligned,
    Horizontal,
    Vertical,
}

/// A driving dimension. Typing its value moves the free end of what it
/// measures (the later-drawn entity) and, one step on, the ends that met it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dim {
    pub id: u32,
    pub kind: DimKind,
    /// Length, Radius: the curve. Distance: the first point's curve.
    /// Angle: the first line.
    pub a: PointRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b: Option<PointRef>,
    #[serde(default)]
    pub axis: Axis,
    /// Millimetres, or degrees for angles.
    pub value: f64,
    /// Where the dimension line sits off its geometry (display only).
    #[serde(default)]
    pub offset: f64,
}

/// How a section cuts (kept on the sketch so a Library section carries a
/// default; the cut row owns the real values).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Extrude {
    /// `through`, `depth`, `both` or `to_face`.
    pub extent: String,
    #[serde(default)]
    pub depth: f64,
    /// `subtract`, `intersect`, `union` or `trim`.
    #[serde(default = "subtract")]
    pub operation: String,
}
fn subtract() -> String {
    "subtract".into()
}
impl Default for Extrude {
    fn default() -> Self {
        Self { extent: "through".into(), depth: 0.0, operation: subtract() }
    }
}

/// The nine implicit attachment points, by name.
pub const ATTACH: [&str; 9] = [
    "top_left",
    "top_centre",
    "top_right",
    "middle_left",
    "centre",
    "middle_right",
    "bottom_left",
    "bottom_centre",
    "bottom_right",
];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Sketch {
    #[serde(rename = "use", default)]
    pub usage: Use,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Where it sits in the model: an anchor (`{"ref", "frame", "offset"}`,
    /// general-base §2.3). Library profiles have none.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub plane: serde_json::Value,
    #[serde(default)]
    pub curves: Vec<Curve>,
    /// The sketch's origin: where a sweep path runs through, (0, 0) unless moved.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub origin: DVec2,
    /// Extra named points (the nine attachment points are implicit).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<NamedPoint>,
    /// The default attachment point: one of ATTACH or a named point.
    #[serde(default = "centre", skip_serializing_if = "is_centre")]
    pub attach: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dims: Vec<Dim>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extrude: Option<Extrude>,
    /// Next fresh element id (ids are never reused inside a sketch).
    #[serde(default)]
    pub next_id: u32,
}
fn is_zero(p: &DVec2) -> bool {
    *p == DVec2::ZERO
}
fn centre() -> String {
    "centre".into()
}
fn is_centre(s: &String) -> bool {
    s == "centre"
}

impl Sketch {
    pub fn new(usage: Use) -> Self {
        Self { usage, attach: centre(), next_id: 1, ..Default::default() }
    }
    pub fn fresh_id(&mut self) -> u32 {
        let floor = self
            .curves
            .iter()
            .map(|c| c.id)
            .chain(self.dims.iter().map(|d| d.id))
            .max()
            .unwrap_or(0)
            + 1;
        self.next_id = self.next_id.max(floor);
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    pub fn add(&mut self, shape: Shape) -> u32 {
        let id = self.fresh_id();
        self.curves.push(Curve { id, shape, segments: None });
        id
    }
    pub fn curve(&self, id: u32) -> Option<&Curve> {
        self.curves.iter().find(|c| c.id == id)
    }
    pub fn curve_mut(&mut self, id: u32) -> Option<&mut Curve> {
        self.curves.iter_mut().find(|c| c.id == id)
    }
    pub fn remove(&mut self, id: u32) -> Option<Curve> {
        let i = self.curves.iter().position(|c| c.id == id)?;
        self.dims.retain(|d| d.a.curve != id && d.b.is_none_or(|b| b.curve != id));
        Some(self.curves.remove(i))
    }
    pub fn point(&self, r: PointRef) -> Option<DVec2> {
        let s = &self.curve(r.curve)?.shape;
        match r.at {
            At::Start => s.start().or(s.centre()),
            At::End => s.end().or(s.centre()),
            At::Mid => Some(s.mid()),
            At::Centre => s.centre().or(Some(s.mid())),
        }
    }
    /// Exact bounds of every curve, or None when empty.
    pub fn bounds(&self) -> Option<(DVec2, DVec2)> {
        self.curves.iter().map(|c| c.shape.bounds()).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
    }
    /// An attachment point by name: the nine of the bounding box, a named
    /// point, or the origin.
    pub fn attachment(&self, name: &str) -> Option<DVec2> {
        if name == "origin" {
            return Some(self.origin);
        }
        if let Some(p) = self.points.iter().find(|p| p.name == name) {
            return Some(p.at);
        }
        let i = ATTACH.iter().position(|n| *n == name)?;
        let (lo, hi) = self.bounds()?;
        let x = [lo.x, (lo.x + hi.x) * 0.5, hi.x][i % 3];
        let y = [hi.y, (lo.y + hi.y) * 0.5, lo.y][i / 3];
        Some(DVec2::new(x, y))
    }
    /// Scale every length about the origin (Scale by edge, unit fixes).
    pub fn scale(&mut self, k: f64) {
        let o = self.origin;
        for c in &mut self.curves {
            c.shape = c.shape.scaled(o, k);
        }
        for p in &mut self.points {
            p.at = o + (p.at - o) * k;
        }
        for d in &mut self.dims {
            if d.kind != DimKind::Angle {
                d.value *= k;
                d.offset *= k;
            }
        }
    }
    pub fn translate(&mut self, by: DVec2) {
        for c in &mut self.curves {
            c.shape = c.shape.translated(by);
        }
        for p in &mut self.points {
            p.at += by;
        }
    }
    /// Curve ends within `tol` of `p` (id, which end).
    pub fn ends_at(&self, p: DVec2, tol: f64) -> Vec<(u32, At)> {
        let mut out = Vec::new();
        for c in &self.curves {
            if let Some([s, e]) = c.shape.ends() {
                if (s - p).length() <= tol {
                    out.push((c.id, At::Start));
                }
                if (e - p).length() <= tol {
                    out.push((c.id, At::End));
                }
            }
        }
        out
    }
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("sketch serialises")
    }
    pub fn from_json(v: &serde_json::Value) -> Result<Self, String> {
        let mut s: Sketch = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
        s.validate()?;
        Ok(s)
    }
    /// Refuse what the geometry code cannot hold: non-finite numbers,
    /// zero-length lines, zero radii, repeated ids.
    pub fn validate(&mut self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for c in &self.curves {
            if !seen.insert(c.id) || c.id == 0 {
                return Err(format!("repeated or zero curve id {}", c.id));
            }
            let ok = match c.shape {
                Shape::Line { a, b } => a.is_finite() && b.is_finite() && (b - a).length() > EPS,
                Shape::Arc { c, r, a0, a1 } => c.is_finite() && r > EPS && r.is_finite() && a0.is_finite() && a1 > a0 && a1 - a0 <= std::f64::consts::TAU + 1e-9,
                Shape::Circle { c, r } => c.is_finite() && r > EPS && r.is_finite(),
            };
            if !ok {
                return Err(format!("curve {} is degenerate", c.id));
            }
        }
        if !self.origin.is_finite() || self.points.iter().any(|p| !p.at.is_finite()) {
            return Err("non-finite point".into());
        }
        let floor = self.curves.iter().map(|c| c.id).chain(self.dims.iter().map(|d| d.id)).max().unwrap_or(0) + 1;
        self.next_id = self.next_id.max(floor);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json_round_trip_keeps_ids_and_the_design_field_names() {
        let mut s = Sketch::new(Use::Section);
        s.add(Shape::Line { a: DVec2::ZERO, b: DVec2::new(60., 0.) });
        s.add(crate::geom::arc(DVec2::ZERO, 10., 0., 1.0));
        s.extrude = Some(Extrude { extent: "depth".into(), depth: 80., operation: "subtract".into() });
        let v = s.to_json();
        assert_eq!(v["use"], "section");
        assert_eq!(v["curves"][0]["kind"], "line");
        assert_eq!(v["curves"][1]["kind"], "arc");
        assert_eq!(v["curves"][1]["r"], 10.0);
        let back = Sketch::from_json(&v).unwrap();
        assert_eq!(back, s);
        let mut bad = v.clone();
        bad["curves"][1]["id"] = 1.into();
        assert!(Sketch::from_json(&bad).is_err());
    }
    #[test]
    fn nine_attachment_points_come_from_the_exact_bounds() {
        let mut s = Sketch::new(Use::Profile);
        s.add(Shape::Circle { c: DVec2::new(10., 20.), r: 5. });
        assert_eq!(s.attachment("top_centre"), Some(DVec2::new(10., 25.)));
        assert_eq!(s.attachment("bottom_left"), Some(DVec2::new(5., 15.)));
        s.points.push(NamedPoint { name: "shear_centre".into(), at: DVec2::new(1., 2.) });
        assert_eq!(s.attachment("shear_centre"), Some(DVec2::new(1., 2.)));
        assert_eq!(s.attachment("nowhere"), None);
    }
}
