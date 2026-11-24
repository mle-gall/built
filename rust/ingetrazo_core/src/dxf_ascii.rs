// SPDX-License-Identifier: GPL-3.0-or-later
//! UTF-8 DXF model-space records. Keep curve parameters until the user
//! chooses units: the importer's flattening tolerance is in metres.
use super::{infer_unit, prune, unit, Bucket, Drawing, Placement, Ring};
use glam::{DMat4, DQuat, DVec3};
use std::{
    collections::{BTreeMap, HashSet},
    f64::consts::{FRAC_PI_2, TAU},
};

#[derive(Debug)]
struct Entity {
    layer: String,
    shape: Shape,
}
#[derive(Debug)]
enum Shape {
    Line(DVec3, DVec3),
    Polyline(Ring, bool),
    Face(Ring),
    Arc {
        center: DVec3,
        radius: f64,
        start: f64,
        span: f64,
    },
    Insert(String, DMat4),
}
#[derive(Debug)]
struct Block {
    base: DVec3,
    supported: bool,
    entities: Vec<Entity>,
}
#[derive(Debug, Default)]
pub(super) struct Document {
    pub(super) units: i64,
    entities: Vec<Entity>,
    blocks: BTreeMap<String, Block>,
}

struct Tags<'a>(Vec<(i32, &'a str)>);
impl<'a> Tags<'a> {
    fn get(&self, code: i32) -> Option<&'a str> {
        self.0.iter().find_map(|&(c, s)| (c == code).then_some(s))
    }
    fn number(&self, code: i32, default: f64) -> Option<f64> {
        let n = match self.get(code) {
            Some(s) => s.trim().parse().ok()?,
            None => default,
        };
        n.is_finite().then_some(n)
    }
    fn point(&self, code: i32) -> Option<DVec3> {
        Some(DVec3::new(
            self.number(code, f64::NAN)?,
            self.number(code + 10, 0.)?,
            self.number(code + 20, 0.)?,
        ))
    }
    fn name(&self, code: i32) -> Option<String> {
        let s = self.get(code)?;
        // The full decoder handles escaped Unicode and legacy encodings.
        (!s.is_empty() && !s.contains('\\')).then(|| s.to_owned())
    }
    fn entity(&self, kind: &str) -> Option<Entity> {
        if self.number(210, 0.)? != 0. || self.number(220, 0.)? != 0. || self.number(230, 1.)? != 1.
        {
            return None;
        }
        let layer = if self.get(8).is_some() {
            self.name(8)?
        } else {
            "0".into()
        };
        let shape = match kind {
            "LINE" => Shape::Line(self.point(10)?, self.point(11)?),
            "SOLID" | "TRACE" | "3DFACE" => {
                let (a, b, c) = (self.point(10)?, self.point(11)?, self.point(12)?);
                let d = if self.get(13).is_some() {
                    self.point(13)?
                } else {
                    c
                };
                Shape::Face(prune(
                    if kind == "3DFACE" {
                        [a, b, c, d]
                    } else {
                        [a, b, d, c]
                    },
                    false,
                ))
            }
            "CIRCLE" | "ARC" => {
                let center = self.point(10)?;
                let radius = self.number(40, f64::NAN)?;
                if radius <= 0. {
                    return None;
                }
                let (start, span) = if kind == "CIRCLE" {
                    (0., -TAU)
                } else {
                    let a = self.number(50, 0.)?;
                    let b = self.number(51, 360.)?;
                    if a == b {
                        return None;
                    }
                    let span = (b - a).rem_euclid(360.);
                    if span == 0. {
                        return None;
                    }
                    (a.to_radians(), span.to_radians())
                };
                Shape::Arc {
                    center,
                    radius,
                    start,
                    span,
                }
            }
            "LWPOLYLINE" => {
                let mut points = vec![];
                let z = self.number(38, 0.)?;
                for &(c, s) in &self.0 {
                    match c {
                        10 => points.push(DVec3::new(s.trim().parse().ok()?, f64::NAN, z)),
                        20 => points.last_mut()?.y = s.trim().parse().ok()?,
                        42 if s.trim().parse::<f64>().ok()? != 0. => return None,
                        _ => (),
                    }
                }
                if points.len() < 2
                    || points.iter().any(|p| !p.is_finite())
                    || self.get(90)?.trim().parse::<usize>().ok()? != points.len()
                {
                    return None;
                }
                let closed = self.get(70).unwrap_or("0").trim().parse::<i32>().ok()? & 1 != 0;
                Shape::Polyline(points, closed)
            }
            "INSERT" => {
                if self.number(70, 1.)? != 1.
                    || self.number(71, 1.)? != 1.
                    || self.number(66, 0.)? != 0.
                {
                    return None;
                }
                let scale = DVec3::new(
                    self.number(41, 1.)?,
                    self.number(42, 1.)?,
                    self.number(43, 1.)?,
                );
                if scale.abs().min_element() < 1e-15 {
                    return None;
                }
                Shape::Insert(
                    self.name(2)?,
                    DMat4::from_scale_rotation_translation(
                        scale,
                        DQuat::from_rotation_z(self.number(50, 0.)?.to_radians()),
                        self.point(10)?,
                    ),
                )
            }
            _ => return None,
        };
        Some(Entity { layer, shape })
    }
}

pub(super) fn decode(text: &str) -> Option<Document> {
    let mut doc = Document::default();
    let mut lines = text.lines();
    let mut kind = None;
    let mut tags = Tags(vec![]);
    let mut section = "";
    let mut seen = HashSet::new();
    let mut model_owner = None;
    let mut block = None;
    let mut eof = false;
    let mut modern = false;
    loop {
        let pair = match lines.next() {
            Some(c) => Some((c.trim().parse::<i32>().ok()?, lines.next()?)),
            None => None,
        };
        if let Some((code, value)) = pair {
            if code != 0 {
                if kind.is_none() || eof {
                    return None;
                }
                tags.0.push((code, value));
                continue;
            }
        }
        match kind {
            None => (),
            Some("SECTION") if section.is_empty() => {
                section = tags.get(2)?;
                if !seen.insert(section) {
                    return None;
                }
                if section == "HEADER" {
                    for pair in tags.0.windows(2) {
                        if pair[0] == (9, "$ACADVER") {
                            modern = pair[1].0 == 1
                                && matches!(pair[1].1, "AC1021" | "AC1024" | "AC1027" | "AC1032")
                        }
                        if pair[0] == (9, "$INSUNITS") {
                            if pair[1].0 != 70 {
                                return None;
                            }
                            doc.units = pair[1].1.trim().parse().ok()?
                        }
                    }
                }
            }
            Some("ENDSEC") if !section.is_empty() && block.is_none() => section = "",
            Some("EOF") if section.is_empty() && !eof => eof = true,
            Some("BLOCK_RECORD")
                if section == "TABLES"
                    && tags
                        .get(2)
                        .is_some_and(|n| n.eq_ignore_ascii_case("*Model_Space")) =>
            {
                if model_owner.replace(tags.get(5)?).is_some() {
                    return None;
                }
            }
            Some("BLOCK") if section == "BLOCKS" && block.is_none() => {
                let name = tags.name(2)?;
                let base = tags.point(10)?;
                if doc
                    .blocks
                    .insert(
                        name.clone(),
                        Block {
                            base,
                            supported: true,
                            entities: vec![],
                        },
                    )
                    .is_some()
                {
                    return None;
                }
                block = Some(name)
            }
            Some("ENDBLK") if section == "BLOCKS" && block.take().is_some() => (),
            Some(k) if section == "BLOCKS" => {
                let b = doc.blocks.get_mut(block.as_ref()?)?;
                if b.supported {
                    match tags.entity(k) {
                        Some(entity) if !matches!(entity.shape, Shape::Insert(..)) => {
                            b.entities.push(entity)
                        }
                        _ => {
                            b.supported = false;
                            b.entities.clear();
                        }
                    }
                }
            }
            Some(k) if section == "ENTITIES" => {
                let owner = tags.get(330)?;
                let paper = tags.number(67, 0.)?;
                if paper == 0. {
                    if Some(owner) != model_owner {
                        return None;
                    }
                    doc.entities.push(tags.entity(k)?);
                } else if Some(owner) == model_owner {
                    return None;
                }
            }
            Some(_) if section.is_empty() || matches!(kind, Some("SECTION" | "EOF" | "ENDSEC")) => {
                return None
            }
            _ => (),
        }
        tags.0.clear();
        match pair {
            Some((_, v)) if !eof => kind = Some(v),
            None => break,
            _ => return None,
        }
    }
    if !eof || !modern || !seen.contains("ENTITIES") || model_owner.is_none() {
        return None;
    }
    for e in &doc.entities {
        if let Shape::Insert(name, _) = &e.shape {
            if !doc.blocks.get(name).is_some_and(|b| b.supported) {
                return None;
            }
        }
    }
    Some(doc)
}

// Model-space circular curves use cubic spans of at most 90 degrees.
// Subdivide each eighth span to the distance tolerance at its midpoint.
fn arc(center: DVec3, r: f64, start: f64, span: f64, tol: f64) -> Option<Ring> {
    let sections = (span.abs() / FRAC_PI_2).ceil() as usize;
    let step = span / sections as f64;
    let arm = (step * 0.25).tan() * 4. / 3.;
    let point = |a: f64| DVec3::new(a.cos(), a.sin(), 0.);
    let tangent = |a: f64| DVec3::new(-a.sin(), a.cos(), 0.);
    let mut out = vec![center + r * point(start)];
    for i in 0..sections {
        let a = start + i as f64 * step;
        let b = a + step;
        let p = point(a);
        let q = point(b);
        let c = p + arm * tangent(a);
        let d = q - arm * tangent(b);
        let at = |t: f64| {
            let u = 1. - t;
            r * (u * u * u * p + 3. * u * u * t * c + 3. * u * t * t * d + t * t * t * q)
        };
        for j in 0..8 {
            let lo = j as f64 / 8.;
            let hi = (j + 1) as f64 / 8.;
            let mut work = vec![(lo, hi, at(lo), at(hi))];
            while let Some((lo, hi, p, q)) = work.pop() {
                let mid = (lo + hi) * 0.5;
                let m = at(mid);
                if m.distance((p + q) * 0.5) <= tol {
                    out.push(center + q)
                } else {
                    if mid == lo || mid == hi || out.len() > 1_000_000 {
                        return None;
                    }
                    work.push((mid, hi, m, q));
                    work.push((lo, mid, p, m))
                }
            }
        }
    }
    Some(out)
}

impl Document {
    pub(super) fn suggestion(&self) -> Option<f64> {
        let mut samples = vec![];
        for e in &self.entities {
            if samples.len() >= 4000 {
                break;
            }
            match &e.shape {
                Shape::Line(a, b) => samples.push(a.distance(*b)),
                Shape::Arc { radius, .. } => samples.push(2. * radius),
                Shape::Polyline(p, _) => samples.extend(p.windows(2).map(|w| w[0].distance(w[1]))),
                _ => (),
            }
        }
        infer_unit(unit(self.units), samples)
    }
    pub(super) fn drawing(&self, sagitta: f64) -> Option<Drawing> {
        let mut drawing = Drawing::default();
        let mut placed = HashSet::new();
        for e in &self.entities {
            match &e.shape {
                Shape::Insert(name, m) => {
                    let b = self.blocks.get(name)?;
                    drawing.placements.push(Placement {
                        name: name.clone(),
                        layer: e.layer.clone(),
                        m: *m * DMat4::from_translation(-b.base),
                    });
                    if placed.insert(name) {
                        let mut bucket = Bucket::default();
                        for item in &b.entities {
                            item.shape.add(&mut bucket, sagitta)?;
                        }
                        drawing.blocks.push((name.clone(), bucket));
                    }
                    drawing.entities += 1;
                }
                s => {
                    drawing.entities +=
                        s.add(drawing.layers.entry(e.layer.clone()).or_default(), sagitta)? as usize
                }
            }
        }
        Some(drawing)
    }
}
impl Shape {
    fn add(&self, b: &mut Bucket, tol: f64) -> Option<bool> {
        match self {
            Self::Line(a, z) => b.runs.push((vec![*a, *z], false)),
            Self::Polyline(pts, closed) => {
                let mut pts = pts.clone();
                if *closed && pts.first() != pts.last() {
                    pts.push(pts[0])
                }
                b.runs.push((pts, false));
            }
            Self::Face(p) => {
                if p.len() < 3 {
                    return Some(false);
                }
                b.faces.push(p.clone());
            }
            Self::Arc {
                center,
                radius,
                start,
                span,
            } => b
                .runs
                .push((arc(*center, *radius, *start, *span, tol)?, true)),
            Self::Insert(..) => return None,
        }
        Some(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE: &str = include_str!("../../../tests/fixtures/dxf/block-and-circle.dxf");
    #[test]
    fn r2018_modelspace_matches_the_original_decoder() {
        let source = decode(FIXTURE).unwrap();
        assert_eq!(source.suggestion(), Some(1.));
        let model = source.drawing(0.01).unwrap();
        assert_eq!(model.entities, 4);
        assert_eq!(model.blocks.len(), 1);
        assert_eq!(model.placements.len(), 2);
        let placement = &model.placements[0];
        assert_eq!(placement.name, "panel");
        let local = DVec3::new(2., 2., 0.);
        assert!(
            placement
                .m
                .transform_point3(local)
                .distance(DVec3::new(10., 22., 0.))
                < 1e-9
        );
        assert_eq!(model.blocks[0].1.faces[0].len(), 4);
        assert_eq!(model.layers["axis"].runs.len(), 1);
        let (circle, curved) = &model.layers["markers"].runs[0];
        assert!(*curved);
        assert_eq!(circle.len(), 33);
        assert!(circle[1].distance(DVec3::new(1.3918734216769113, 1.9193860482616198, 0.)) < 1e-12);
        assert!(circle[0].distance(*circle.last().unwrap()) < 1e-12);
        assert!(
            source.drawing(0.00001).unwrap().layers["markers"].runs[0]
                .0
                .len()
                > 33
        );
    }
    #[test]
    fn unsupported_curves_leave_the_complete_drawing_to_the_compatibility_reader() {
        assert!(decode(&FIXTURE.replace("\nCIRCLE\n", "\nSPLINE\n")).is_none());
    }
}
