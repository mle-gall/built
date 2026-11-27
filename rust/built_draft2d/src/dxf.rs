// SPDX-License-Identifier: GPL-3.0-or-later
//! DXF into a sketch. Arcs, circles and polyline bulges stay exact arcs
//! (the model importer flattens them; a profile must not). Blocks are
//! expanded; splines and ellipses become short lines the cleaner can merge.
use crate::geom::{arc, Shape};
use crate::import::{Drawing, Layer};
use glam::{DMat3, DVec2, DVec3};
use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};

/// `$INSUNITS` codes to millimetres.
pub fn unit(code: i64) -> Option<(f64, &'static str)> {
    Some(match code {
        1 => (25.4, "in"),
        2 => (304.8, "ft"),
        4 => (1.0, "mm"),
        5 => (10.0, "cm"),
        6 => (1000.0, "m"),
        14 => (100.0, "dm"),
        _ => return None,
    })
}

type Pairs<'a> = Vec<(i32, &'a str)>;

fn pairs(text: &str) -> Pairs<'_> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let (Some(code), Some(value)) = (lines.next(), lines.next()) {
        let Ok(code) = code.trim().parse::<i32>() else { break };
        out.push((code, value.trim()));
    }
    out
}

#[derive(Default, Debug)]
struct Entity<'a> {
    kind: &'a str,
    tags: Vec<(i32, &'a str)>,
    /// POLYLINE vertices (their own tags).
    vertices: Vec<Vec<(i32, &'a str)>>,
}
impl Entity<'_> {
    fn get(&self, code: i32) -> Option<&str> {
        self.tags.iter().find(|t| t.0 == code).map(|t| t.1)
    }
    fn num(&self, code: i32, default: f64) -> f64 {
        self.get(code).and_then(|s| s.parse().ok()).filter(|x: &f64| x.is_finite()).unwrap_or(default)
    }
    fn layer(&self) -> String {
        self.get(8).unwrap_or("0").to_owned()
    }
    fn p(&self, code: i32) -> DVec2 {
        DVec2::new(self.num(code, 0.0), self.num(code + 10, 0.0))
    }
    /// The object coordinate system's flip: extrusion (0, 0, -1) mirrors X.
    fn flipped(&self) -> bool {
        self.num(230, 1.0) < 0.0
    }
}

fn entities<'a>(p: &[(i32, &'a str)]) -> Vec<Entity<'a>> {
    let mut out: Vec<Entity<'a>> = Vec::new();
    let mut i = 0;
    while i < p.len() {
        let (code, value) = p[i];
        i += 1;
        if code != 0 {
            continue;
        }
        if value == "ENDSEC" || value == "ENDBLK" || value == "EOF" {
            break;
        }
        let mut e = Entity { kind: value, ..Default::default() };
        while i < p.len() && p[i].0 != 0 {
            e.tags.push(p[i]);
            i += 1;
        }
        if value == "POLYLINE" {
            while i < p.len() && p[i] == (0, "VERTEX") {
                i += 1;
                let mut v = Vec::new();
                while i < p.len() && p[i].0 != 0 {
                    v.push(p[i]);
                    i += 1;
                }
                e.vertices.push(v);
            }
            if i < p.len() && p[i] == (0, "SEQEND") {
                i += 1;
                while i < p.len() && p[i].0 != 0 {
                    i += 1;
                }
            }
        }
        out.push(e);
    }
    out
}

/// A segment with a bulge: the arc from `a` to `b` turning by 4·atan(bulge).
pub fn bulge_shape(a: DVec2, b: DVec2, bulge: f64) -> Shape {
    if bulge.abs() < 1e-12 {
        return Shape::Line { a, b };
    }
    let theta = 4.0 * bulge.atan();
    let chord = b - a;
    let half = chord.length() / 2.0;
    let r = half / (theta / 2.0).sin().abs();
    // The centre is on the chord's bisector, to the left for a CCW bulge.
    let mid = (a + b) / 2.0;
    let h = (r * r - half * half).max(0.0).sqrt();
    let n = chord.normalize().perp();
    let side = if (theta.abs() < PI) == (theta > 0.0) { 1.0 } else { -1.0 };
    let c = mid + n * h * side;
    let ta = (a - c).y.atan2((a - c).x);
    let tb = (b - c).y.atan2((b - c).x);
    if theta > 0.0 {
        arc(c, r, ta, tb)
    } else {
        arc(c, r, tb, ta)
    }
}

/// A 2D affine map (block inserts), with whether it keeps circles round.
#[derive(Clone, Copy)]
struct Map(DMat3);
impl Map {
    fn apply(&self, p: DVec2) -> DVec2 {
        self.0.transform_point2(p)
    }
    fn similar(&self) -> Option<(f64, f64, bool)> {
        let (x, y) = (self.0.x_axis.truncate(), self.0.y_axis.truncate());
        let (sx, sy) = (x.length(), y.length());
        let mirrored = x.perp_dot(y) < 0.0;
        ((sx - sy).abs() <= 1e-9 * sx.max(1.0) && x.dot(y).abs() <= 1e-9 * sx * sy).then_some((sx, x.y.atan2(x.x), mirrored))
    }
    fn shape(&self, s: Shape) -> Vec<Shape> {
        match (s, self.similar()) {
            (Shape::Line { a, b }, _) => vec![Shape::Line { a: self.apply(a), b: self.apply(b) }],
            (Shape::Circle { c, r }, Some((k, _, _))) => vec![Shape::Circle { c: self.apply(c), r: r * k }],
            (Shape::Arc { c, r, a0, a1 }, Some((k, rot, mirrored))) => {
                if mirrored {
                    // Mirroring runs the arc the other way: swap its ends.
                    let (p, q) = (self.apply(crate::geom::polar(c, r, a1)), self.apply(crate::geom::polar(c, r, a0)));
                    let cc = self.apply(c);
                    vec![arc(cc, r * k, (p - cc).y.atan2((p - cc).x), (q - cc).y.atan2((q - cc).x))]
                } else {
                    vec![arc(self.apply(c), r * k, a0 + rot, a1 + rot)]
                }
            }
            (other, None) => {
                let pts: Vec<DVec2> = other.polyline(64).into_iter().map(|p| self.apply(p)).collect();
                pts.windows(2).map(|w| Shape::Line { a: w[0], b: w[1] }).collect()
            }
        }
    }
}

fn shapes(e: &Entity<'_>, skipped: &mut BTreeMap<String, usize>) -> Vec<Shape> {
    let flip = |s: Shape| -> Shape {
        if !e.flipped() {
            return s;
        }
        Map(DMat3::from_cols(DVec3::new(-1.0, 0.0, 0.0), DVec3::Y, DVec3::Z)).shape(s)[0]
    };
    let mut out = Vec::new();
    match e.kind {
        "LINE" => {
            let (a, b) = (e.p(10), e.p(11));
            if (b - a).length() > 1e-12 {
                out.push(Shape::Line { a, b });
            }
        }
        "CIRCLE" => {
            let r = e.num(40, 0.0);
            if r > 0.0 {
                out.push(flip(Shape::Circle { c: e.p(10), r }));
            }
        }
        "ARC" => {
            let r = e.num(40, 0.0);
            if r > 0.0 {
                out.push(flip(arc(e.p(10), r, e.num(50, 0.0).to_radians(), e.num(51, 360.0).to_radians())));
            }
        }
        "LWPOLYLINE" => {
            let closed = (e.num(70, 0.0) as i64) & 1 == 1;
            // Vertices: each 10/20 pair, with the bulge (42) that follows it.
            let mut pts: Vec<(DVec2, f64)> = Vec::new();
            let mut x = None;
            for &(code, v) in &e.tags {
                let n: f64 = v.parse().unwrap_or(0.0);
                match code {
                    10 => x = Some(n),
                    20 => {
                        if let Some(xv) = x.take() {
                            pts.push((DVec2::new(xv, n), 0.0));
                        }
                    }
                    42 => {
                        if let Some(last) = pts.last_mut() {
                            last.1 = n;
                        }
                    }
                    _ => {}
                }
            }
            out.extend(poly(&pts, closed).into_iter().map(flip));
        }
        "POLYLINE" => {
            let flags = e.num(70, 0.0) as i64;
            if flags & (16 | 64) != 0 {
                *skipped.entry("POLYLINE (mesh)".into()).or_default() += 1;
                return out;
            }
            let pts: Vec<(DVec2, f64)> = e
                .vertices
                .iter()
                .map(|v| {
                    let get = |c: i32| v.iter().find(|t| t.0 == c).and_then(|t| t.1.parse::<f64>().ok()).unwrap_or(0.0);
                    (DVec2::new(get(10), get(20)), get(42))
                })
                .collect();
            out.extend(poly(&pts, flags & 1 == 1).into_iter().map(flip));
        }
        "ELLIPSE" => {
            let c = e.p(10);
            let major = e.p(11);
            let ratio = e.num(40, 1.0);
            let (t0, t1) = (e.num(41, 0.0), e.num(42, TAU));
            let r = major.length();
            if (ratio - 1.0).abs() < 1e-9 {
                let rot = major.y.atan2(major.x);
                out.push(flip(if (t1 - t0 - TAU).abs() < 1e-9 { Shape::Circle { c, r } } else { arc(c, r, t0 + rot, t1 + rot) }));
            } else {
                let minor = major.perp() * ratio;
                let span = (t1 - t0).rem_euclid(TAU);
                let span = if span < 1e-12 { TAU } else { span };
                let n = ((span / TAU) * 72.0).ceil().max(4.0) as usize;
                let pts: Vec<DVec2> = (0..=n).map(|i| {
                    let t = t0 + span * i as f64 / n as f64;
                    c + major * t.cos() + minor * t.sin()
                }).collect();
                out.extend(pts.windows(2).map(|w| flip(Shape::Line { a: w[0], b: w[1] })));
            }
        }
        "SPLINE" => {
            // Fit points when present, else the control polygon: lines the
            // cleaner can merge into arcs where they lie on one.
            let mut fit: Vec<DVec2> = Vec::new();
            let mut ctrl: Vec<DVec2> = Vec::new();
            let mut x = (None, None);
            for &(code, v) in &e.tags {
                let n: f64 = v.parse().unwrap_or(0.0);
                match code {
                    10 => x.0 = Some(n),
                    20 => {
                        if let Some(xv) = x.0.take() {
                            ctrl.push(DVec2::new(xv, n));
                        }
                    }
                    11 => x.1 = Some(n),
                    21 => {
                        if let Some(xv) = x.1.take() {
                            fit.push(DVec2::new(xv, n));
                        }
                    }
                    _ => {}
                }
            }
            let pts = if fit.len() >= 2 { fit } else { ctrl };
            if pts.len() >= 2 {
                out.extend(pts.windows(2).filter(|w| (w[1] - w[0]).length() > 1e-12).map(|w| flip(Shape::Line { a: w[0], b: w[1] })));
                *skipped.entry("SPLINE (as lines)".into()).or_default() += 1;
            }
        }
        "POINT" | "TEXT" | "MTEXT" | "DIMENSION" | "HATCH" | "SOLID" | "3DFACE" | "VIEWPORT" | "ATTDEF" | "ATTRIB" | "LEADER" | "MLEADER" | "INSERT" => {
            if e.kind != "INSERT" {
                *skipped.entry(e.kind.to_owned()).or_default() += 1;
            }
        }
        other => {
            *skipped.entry(other.to_owned()).or_default() += 1;
        }
    }
    out
}

fn poly(pts: &[(DVec2, f64)], closed: bool) -> Vec<Shape> {
    let n = pts.len();
    if n < 2 {
        return vec![];
    }
    let count = if closed { n } else { n - 1 };
    (0..count)
        .filter_map(|i| {
            let (a, bulge) = pts[i];
            let b = pts[(i + 1) % n].0;
            ((b - a).length() > 1e-12).then(|| bulge_shape(a, b, bulge))
        })
        .collect()
}

/// Read DXF text (ASCII; other encodings read lossily) into layers of
/// shapes in drawing units, with the declared unit.
pub fn read(text: &str) -> Result<Drawing, String> {
    let p = pairs(text);
    if p.is_empty() {
        return Err("not a DXF file".into());
    }
    // Header units.
    let mut declared = None;
    for w in p.windows(2) {
        if w[0] == (9, "$INSUNITS") && w[1].0 == 70 {
            declared = w[1].1.parse::<i64>().ok().and_then(unit);
        }
    }
    // Sections.
    let mut blocks: BTreeMap<String, (DVec2, Vec<Entity<'_>>)> = BTreeMap::new();
    let mut model: Vec<Entity<'_>> = Vec::new();
    let mut i = 0;
    while i + 1 < p.len() {
        if p[i] == (0, "SECTION") && p[i + 1].0 == 2 {
            let name = p[i + 1].1;
            i += 2;
            let end = (i..p.len()).find(|&k| p[k] == (0, "ENDSEC")).unwrap_or(p.len());
            let body = &p[i..end];
            match name {
                "ENTITIES" => model = entities(body),
                "BLOCKS" => {
                    let mut k = 0;
                    while k < body.len() {
                        if body[k] == (0, "BLOCK") {
                            let head_end = (k + 1..body.len()).find(|&j| body[j].0 == 0).unwrap_or(body.len());
                            let head = &body[k + 1..head_end];
                            let bname = head.iter().find(|t| t.0 == 2).map(|t| t.1.to_owned()).unwrap_or_default();
                            let get = |c: i32| head.iter().find(|t| t.0 == c).and_then(|t| t.1.parse::<f64>().ok()).unwrap_or(0.0);
                            let base = DVec2::new(get(10), get(20));
                            let stop = (head_end..body.len()).find(|&j| body[j] == (0, "ENDBLK")).unwrap_or(body.len());
                            blocks.insert(bname, (base, entities(&body[head_end..stop])));
                            k = stop + 1;
                        } else {
                            k += 1;
                        }
                    }
                }
                _ => {}
            }
            i = end + 1;
        } else {
            i += 1;
        }
    }
    let mut layers: BTreeMap<String, Vec<Shape>> = BTreeMap::new();
    let mut skipped = BTreeMap::new();
    fn walk<'a>(
        list: &[Entity<'a>],
        m: Map,
        depth: usize,
        inherited: Option<&str>,
        blocks: &BTreeMap<String, (DVec2, Vec<Entity<'a>>)>,
        layers: &mut BTreeMap<String, Vec<Shape>>,
        skipped: &mut BTreeMap<String, usize>,
    ) {
        for e in list {
            let mut layer = e.layer();
            if layer == "0" {
                if let Some(parent) = inherited {
                    layer = parent.to_owned();
                }
            }
            if e.kind == "INSERT" {
                if depth > 8 {
                    continue;
                }
                let Some((base, inner)) = e.get(2).and_then(|n| blocks.get(n)) else { continue };
                let at = e.p(10);
                let (sx, sy) = (e.num(41, 1.0), e.num(42, 1.0));
                let rot = e.num(50, 0.0).to_radians();
                let local = DMat3::from_translation(at) * DMat3::from_angle(rot) * DMat3::from_scale(DVec2::new(sx, sy)) * DMat3::from_translation(-*base);
                walk(inner, Map(m.0 * local), depth + 1, Some(&layer), blocks, layers, skipped);
                continue;
            }
            for s in shapes(e, skipped) {
                layers.entry(layer.clone()).or_default().extend(m.shape(s));
            }
        }
    }
    walk(&model, Map(DMat3::IDENTITY), 0, None, &blocks, &mut layers, &mut skipped);
    Ok(Drawing {
        layers: layers.into_iter().map(|(name, shapes)| Layer { name, shapes }).collect(),
        unit: declared.map(|u| u.1.to_owned()),
        to_mm: declared.map(|u| u.0),
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn v(x: f64, y: f64) -> DVec2 {
        DVec2::new(x, y)
    }
    #[test]
    fn a_bulge_of_one_is_a_half_circle_to_the_left() {
        let s = bulge_shape(v(0., 0.), v(10., 0.), 1.0);
        let Shape::Arc { c, r, .. } = s else { panic!() };
        assert!((c - v(5., 0.)).length() < 1e-9 && (r - 5.).abs() < 1e-9);
        // CCW from (0,0) to (10,0) with bulge > 0 goes below the chord.
        assert!(s.mid().y < 0.0);
        let q = bulge_shape(v(0., 0.), v(10., 0.), -1.0);
        assert!(q.mid().y > 0.0);
        let small = bulge_shape(v(0., 0.), v(10., 0.), (PI / 16.0).tan());
        assert!((small.length() - small.radius().unwrap() * PI / 4.0).abs() < 1e-9);
    }
    #[test]
    fn entities_units_and_blocks_read() {
        let text = "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n2\nHOLE\n10\n0\n20\n0\n0\nCIRCLE\n8\n0\n10\n0\n20\n0\n40\n2\n0\nENDBLK\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nLINE\n8\nContour\n10\n0\n20\n0\n11\n10\n21\n0\n0\nARC\n8\nContour\n10\n0\n20\n0\n40\n5\n50\n0\n51\n90\n0\nLWPOLYLINE\n8\nContour\n90\n2\n70\n0\n10\n0\n20\n0\n42\n1\n10\n10\n20\n0\n0\nINSERT\n8\nHoles\n2\nHOLE\n10\n50\n20\n20\n41\n2\n42\n2\n0\nTEXT\n8\nNotes\n1\nhi\n0\nENDSEC\n0\nEOF\n";
        let d = read(text).unwrap();
        assert_eq!(d.unit.as_deref(), Some("mm"));
        let contour = d.layers.iter().find(|l| l.name == "Contour").unwrap();
        assert_eq!(contour.shapes.len(), 3);
        assert!(matches!(contour.shapes[1], Shape::Arc { .. }));
        assert!(matches!(contour.shapes[2], Shape::Arc { .. }));
        let holes = d.layers.iter().find(|l| l.name == "Holes").unwrap();
        assert_eq!(holes.shapes, vec![Shape::Circle { c: v(50., 20.), r: 4. }]);
        assert_eq!(d.skipped.get("TEXT"), Some(&1));
    }
}
