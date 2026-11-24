// SPDX-License-Identifier: GPL-3.0-or-later
//! UTF-8 DXF model-space records. Keep curve parameters until the user
//! chooses units: the importer's flattening tolerance is in metres.
use super::{infer_unit, prune, unit, Bucket, Drawing, Placement, Ring};
use glam::{DMat4, DQuat, DVec3};
use std::{
    collections::{BTreeMap, HashSet},
    f64::consts::{FRAC_PI_2, TAU},
};

#[path = "dxf_curves.rs"]
mod curves;

#[derive(Debug)]
struct Entity {
    layer: String,
    shape: Shape,
}
#[derive(Debug)]
enum Shape {
    Ignored(String),
    Line(DVec3, DVec3),
    Curve(Vec<curves::Cubic>, Vec<f64>),
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
    fn numbers(&self, code: i32) -> Option<Vec<f64>> {
        self.0
            .iter()
            .filter(|(k, _)| *k == code)
            .map(|(_, s)| {
                let v: f64 = s.trim().parse().ok()?;
                v.is_finite().then_some(v)
            })
            .collect()
    }
    fn points(&self, code: i32) -> Option<Vec<DVec3>> {
        let mut out = vec![];
        for &(k, s) in &self.0 {
            if k == code || k == code + 10 || k == code + 20 {
                let n: f64 = s.trim().parse().ok()?;
                if !n.is_finite() {
                    return None;
                }
                if k == code {
                    out.push(DVec3::new(n, 0., 0.));
                } else if k == code + 10 {
                    out.last_mut()?.y = n;
                } else {
                    out.last_mut()?.z = n;
                }
            }
        }
        Some(out)
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
            "SPLINE" => Shape::Curve(curves::spline(self)?, vec![]),
            "ELLIPSE" => {
                let major = self.point(11)?;
                let minor = DVec3::Z.cross(major).try_normalize()?
                    * major.length()
                    * self.number(40, 1.)?;
                let start = self.number(41, 0.)?;
                let end = self.number(42, TAU)?;
                let span = (end - start).rem_euclid(TAU);
                let span = if span.abs() < 1e-12 { -TAU } else { span };
                Shape::Curve(
                    curves::elliptic(self.point(10)?, major, minor, start, span),
                    vec![],
                )
            }
            "LWPOLYLINE" => {
                let z = self.number(38, 0.)?;
                let mut points = self.points(10)?;
                for p in &mut points {
                    p.z = z;
                }
                if points.len() < 2 || self.get(90)?.trim().parse::<usize>().ok()? != points.len() {
                    return None;
                }
                let mut bulges = vec![];
                for &(k, s) in &self.0 {
                    if k == 10 {
                        bulges.push(0.);
                    } else if k == 42 {
                        *bulges.last_mut()? = s.trim().parse::<f64>().ok()?;
                    }
                }
                let closed = self.get(70).unwrap_or("0").trim().parse::<i32>().ok()? & 1 != 0;
                if bulges.iter().all(|&b| b == 0.) {
                    Shape::Polyline(points, closed)
                } else {
                    let lengths = points.windows(2).map(|p| p[0].distance(p[1])).collect();
                    Shape::Curve(curves::polyline(&points, &bulges, closed), lengths)
                }
            }
            "INSERT" => {
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
            "POLYLINE" | "VERTEX" | "MESH" | "SEQEND" => return None,
            _ => Shape::Ignored(kind.to_owned()),
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
    let mut tags_read = 0usize;
    let mut eof = false;
    let mut modern = false;
    loop {
        tags_read += 1;
        if tags_read % 4096 == 0 && crate::file_open::cancelled() {
            return None;
        }
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
                b.entities.push(tags.entity(k)?);
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
                if out.len() % 4096 == 0 && crate::file_open::cancelled() {
                    return None;
                }
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

// Nested virtual INSERTs preserve their orthogonal axes and per-axis scale.
fn virtual_insert(parent: DMat4, child: DMat4) -> DMat4 {
    let (scale, rotation, origin) = child.to_scale_rotation_translation();
    let z = parent.transform_vector3(rotation * DVec3::Z).normalize();
    let x = parent.transform_vector3(rotation * DVec3::X).normalize();
    let axes = DMat4::from_cols(
        x.extend(0.),
        z.cross(x).extend(0.),
        z.extend(0.),
        parent.transform_point3(origin).extend(1.),
    );
    let lengths = DVec3::new(
        parent.x_axis.truncate().length(),
        parent.y_axis.truncate().length() * parent.determinant().signum(),
        parent.z_axis.truncate().length(),
    );
    axes * DMat4::from_scale(scale * lengths)
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
                Shape::Curve(_, lengths) => samples.extend_from_slice(lengths),
                Shape::Polyline(p, _) => samples.extend(p.windows(2).map(|w| w[0].distance(w[1]))),
                _ => (),
            }
        }
        infer_unit(unit(self.units), samples)
    }
    pub(super) fn drawing(&self, sagitta: f64) -> Option<Drawing> {
        let mut drawing = Drawing::default();
        for (index, e) in self.entities.iter().enumerate() {
            if index % 256 == 0 && crate::file_open::cancelled() {
                return None;
            }
            if let Shape::Insert(name, m) = &e.shape {
                self.expand(name, &e.layer, *m, 0, &mut drawing.placements)?;
                drawing.entities += 1;
            } else {
                drawing.entities += e.shape.install(
                    drawing.layers.entry(e.layer.clone()).or_default(),
                    sagitta,
                    DMat4::IDENTITY,
                    &mut drawing.skipped,
                )? as usize;
            }
        }
        let mut seen = HashSet::new();
        for p in &drawing.placements {
            if seen.insert(&p.name) {
                let mut bucket = Bucket::default();
                if let Some(b) = self.block(&p.name) {
                    self.prototype(
                        b,
                        DMat4::IDENTITY,
                        0,
                        sagitta,
                        &mut bucket,
                        &mut drawing.skipped,
                    )?;
                    drawing.blocks.push((p.name.clone(), bucket));
                }
            }
        }
        Some(drawing)
    }
    fn block(&self, name: &str) -> Option<&Block> {
        self.blocks.get(name).or_else(|| {
            self.blocks
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, b)| b)
        })
    }
    fn expand(
        &self,
        name: &str,
        layer: &str,
        m: DMat4,
        depth: usize,
        out: &mut Vec<Placement>,
    ) -> Option<()> {
        if crate::file_open::cancelled() {
            return None;
        }
        let Some(block) = self.block(name) else {
            return Some(());
        };
        let matrix = m * DMat4::from_translation(-block.base);
        let mut children = block
            .entities
            .iter()
            .filter(|e| !matches!(&e.shape,Shape::Ignored(k) if k=="ATTDEF"))
            .peekable();
        if depth < 8
            && children.peek().is_some()
            && children
                .clone()
                .all(|c| matches!(c.shape, Shape::Insert(..)))
        {
            for c in children {
                if let Shape::Insert(id, local) = &c.shape {
                    self.expand(
                        id,
                        if c.layer == "0" { layer } else { &c.layer },
                        matrix * *local,
                        depth + 1,
                        out,
                    )?;
                }
            }
        } else {
            out.push(Placement {
                name: name.to_owned(),
                layer: layer.to_owned(),
                m: matrix,
            });
        }
        Some(())
    }
    fn prototype(
        &self,
        block: &Block,
        matrix: DMat4,
        depth: usize,
        tol: f64,
        bucket: &mut Bucket,
        skipped: &mut BTreeMap<String, usize>,
    ) -> Option<()> {
        for (i, item) in block.entities.iter().enumerate() {
            if i % 256 == 0 && crate::file_open::cancelled() {
                return None;
            }
            if let Shape::Insert(name, m) = &item.shape {
                if depth < 8 {
                    if let Some(child) = self.block(name) {
                        self.prototype(
                            child,
                            virtual_insert(matrix, *m) * DMat4::from_translation(-child.base),
                            depth + 1,
                            tol,
                            bucket,
                            skipped,
                        )?;
                    }
                } else {
                    *skipped.entry("INSERT".into()).or_default() += 1;
                }
            } else {
                item.shape.install(bucket, tol, matrix, skipped)?;
            }
        }
        Some(())
    }
}
impl Shape {
    fn install(
        &self,
        b: &mut Bucket,
        tol: f64,
        m: DMat4,
        skipped: &mut BTreeMap<String, usize>,
    ) -> Option<bool> {
        if let Self::Ignored(k) = self {
            if !matches!(
                k.as_str(),
                "ATTDEF" | "VIEWPORT" | "XLINE" | "RAY" | "ATTRIB"
            ) {
                *skipped.entry(k.clone()).or_default() += 1;
            }
            return Some(false);
        }
        if m == DMat4::IDENTITY {
            return self.add(b, tol);
        }
        let map = |p: DVec3| m.transform_point3(p);
        let transform_curves =
            |cs: &[curves::Cubic]| cs.iter().map(|c| c.map(map)).collect::<Vec<_>>();
        match self {
            Self::Arc {
                center,
                radius,
                start,
                span,
            } => Self::Curve(
                transform_curves(&curves::elliptic(
                    *center,
                    *radius * DVec3::X,
                    *radius * DVec3::Y,
                    *start,
                    *span,
                )),
                vec![],
            )
            .add(b, tol),
            Self::Curve(cs, _) => Self::Curve(transform_curves(cs), vec![]).add(b, tol),
            Self::Face(p) => Self::Face(p.iter().copied().map(map).collect()).add(b, tol),
            Self::Line(a, z) => Self::Line(map(*a), map(*z)).add(b, tol),
            Self::Polyline(p, closed) => {
                Self::Polyline(p.iter().copied().map(map).collect(), *closed).add(b, tol)
            }
            _ => None,
        }
    }
    fn add(&self, b: &mut Bucket, tol: f64) -> Option<bool> {
        match self {
            Self::Line(a, z) => b.runs.push((vec![*a, *z], false)),
            Self::Curve(curves, _) => {
                if curves.is_empty() {
                    return Some(false);
                }
                b.runs.push((curves::flatten(curves, tol)?, true));
            }
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
            Self::Insert(..) | Self::Ignored(_) => return None,
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
    fn assembly_matches_reference_edges_in_world_space() {
        let model = decode(include_str!(
            "../../../tests/fixtures/dxf/mixed-assembly.dxf"
        ))
        .unwrap()
        .drawing(0.01)
        .unwrap();
        let expected = [
            ("Leaf", "Span", vec![[0., 10., -1., 10.]]),
            ("Leaf", "Bolts", vec![[2., 7., 2., 8.]]),
            (
                "Frame",
                "0",
                vec![[0., 0., 0., 1.], [8., 4., 8., 6.], [2., 2., 4., 2.]],
            ),
            (
                "Frame",
                "0",
                vec![[15., 0., 15., 1.], [23., 4., 23., 6.], [17., 2., 19., 2.]],
            ),
        ];
        assert_eq!(model.placements.len(), expected.len());
        assert_eq!(model.blocks.len(), 2);
        for (p, (name, layer, edges)) in model.placements.iter().zip(expected) {
            assert_eq!((&*p.name, &*p.layer), (name, layer));
            let (_, b) = model.blocks.iter().find(|(id, _)| id == name).unwrap();
            assert_eq!(b.runs.len(), edges.len());
            for ((pts, _), [x, y, u, v]) in b.runs.iter().zip(edges) {
                assert!(p.m.transform_point3(pts[0]).distance(DVec3::new(x, y, 0.)) < 1e-9);
                assert!(p.m.transform_point3(pts[1]).distance(DVec3::new(u, v, 0.)) < 1e-9);
            }
        }
    }
    #[test]
    fn paths_match_all_ezdxf_vertices_at_two_tolerances() {
        let doc = decode(include_str!(
            "../../../tests/fixtures/dxf/curve-records.dxf"
        ))
        .unwrap();
        let mut bytes =
            include_bytes!("../../../tests/fixtures/dxf/curve-records.points").as_slice();
        for tol in [0.01, 0.0001] {
            let drawing = doc.drawing(tol).unwrap();
            for name in ["bulge", "cubic", "fit", "weighted", "ellipse"] {
                let (pts, is_curve) = &drawing.layers[name].runs[0];
                assert!(is_curve);
                let (count, tail) = bytes.split_at(4);
                bytes = tail;
                assert_eq!(
                    pts.len(),
                    u32::from_le_bytes(count.try_into().unwrap()) as usize
                );
                for p in pts {
                    for x in p.to_array() {
                        let (raw, tail) = bytes.split_at(8);
                        bytes = tail;
                        let expected = f64::from_le_bytes(raw.try_into().unwrap());
                        assert!(
                            (x - expected).abs() < 1e-9,
                            "{name}: {x} vs {expected} at {tol}"
                        );
                    }
                }
            }
        }
        assert!(bytes.is_empty());
    }
    #[test]
    fn unsupported_curves_leave_the_complete_drawing_to_the_compatibility_reader() {
        assert!(decode(&FIXTURE.replace("\nCIRCLE\n", "\nSPLINE\n")).is_none());
    }
}
