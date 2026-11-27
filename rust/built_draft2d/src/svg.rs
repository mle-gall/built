// SPDX-License-Identifier: GPL-3.0-or-later
//! SVG into a sketch: paths, lines, polylines, polygons, rectangles,
//! circles and ellipses, with group transforms. Y points up in a sketch.
//! Circular arcs stay arcs; Béziers become short lines the cleaner merges.
use crate::geom::{arc, Shape};
use crate::import::{Drawing, Layer};
use glam::{DMat3, DVec2, DVec3};
use std::collections::BTreeMap;
use std::f64::consts::TAU;

/// A length with its unit, in millimetres per user unit.
fn physical(v: &str) -> Option<(f64, f64)> {
    let v = v.trim();
    let split = v.find(|c: char| c.is_ascii_alphabetic() || c == '%').unwrap_or(v.len());
    let n: f64 = v[..split].trim().parse().ok()?;
    let mm = match &v[split..] {
        "mm" => 1.0,
        "cm" => 10.0,
        "in" => 25.4,
        "pt" => 25.4 / 72.0,
        "pc" => 25.4 / 6.0,
        "px" | "" => return Some((n, f64::NAN)),
        _ => return None,
    };
    Some((n, mm))
}

fn numbers(s: &str) -> Vec<f64> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if c.is_ascii_digit() || c == '-' || c == '+' || c == '.' {
            let start = i;
            i += 1;
            let mut dot = c == '.';
            while i < b.len() {
                let d = b[i] as char;
                if d.is_ascii_digit() {
                    i += 1;
                } else if d == '.' && !dot {
                    dot = true;
                    i += 1;
                } else if (d == 'e' || d == 'E') && i + 1 < b.len() {
                    i += 1;
                    if b[i] == b'-' || b[i] == b'+' {
                        i += 1;
                    }
                } else {
                    break;
                }
            }
            if let Ok(n) = s[start..i].parse() {
                out.push(n);
            }
        } else {
            i += 1;
        }
    }
    out
}

fn transform(attr: Option<&str>) -> DMat3 {
    let Some(t) = attr else { return DMat3::IDENTITY };
    let mut m = DMat3::IDENTITY;
    for part in t.split(')') {
        let Some((name, args)) = part.split_once('(') else { continue };
        let a = numbers(args);
        let get = |i: usize, d: f64| a.get(i).copied().unwrap_or(d);
        let step = match name.trim().trim_start_matches(',').trim() {
            "matrix" if a.len() == 6 => DMat3::from_cols(DVec3::new(a[0], a[1], 0.0), DVec3::new(a[2], a[3], 0.0), DVec3::new(a[4], a[5], 1.0)),
            "translate" => DMat3::from_translation(DVec2::new(get(0, 0.0), get(1, 0.0))),
            "scale" => DMat3::from_scale(DVec2::new(get(0, 1.0), get(1, get(0, 1.0)))),
            "rotate" => {
                let (r, cx, cy) = (get(0, 0.0).to_radians(), get(1, 0.0), get(2, 0.0));
                DMat3::from_translation(DVec2::new(cx, cy)) * DMat3::from_angle(r) * DMat3::from_translation(DVec2::new(-cx, -cy))
            }
            "skewX" => DMat3::from_cols(DVec3::X, DVec3::new(get(0, 0.0).to_radians().tan(), 1.0, 0.0), DVec3::Z),
            "skewY" => DMat3::from_cols(DVec3::new(1.0, get(0, 0.0).to_radians().tan(), 0.0), DVec3::Y, DVec3::Z),
            _ => DMat3::IDENTITY,
        };
        m *= step;
    }
    m
}

/// Lines through a Bézier, by subdivision to `tol` (user units).
fn bezier(pts: &[DVec2], tol: f64, out: &mut Vec<DVec2>) {
    let flat = pts.windows(2).map(|w| (w[1] - w[0]).length()).sum::<f64>() - (pts[pts.len() - 1] - pts[0]).length();
    if flat <= tol || out.len() > 4096 {
        out.push(pts[pts.len() - 1]);
        return;
    }
    // de Casteljau halves.
    let mut left = vec![pts[0]];
    let mut right = vec![pts[pts.len() - 1]];
    let mut level = pts.to_vec();
    while level.len() > 1 {
        level = level.windows(2).map(|w| (w[0] + w[1]) * 0.5).collect();
        left.push(level[0]);
        right.push(level[level.len() - 1]);
    }
    right.reverse();
    bezier(&left, tol, out);
    bezier(&right, tol, out);
}

/// The SVG arc command (endpoint form) as a centre-form arc, or lines when
/// its radii differ.
fn svg_arc(p: DVec2, q: DVec2, rx: f64, ry: f64, phi: f64, large: bool, sweep: bool) -> Vec<(Shape, bool)> {
    if (q - p).length() < 1e-12 {
        return vec![];
    }
    if rx.abs() < 1e-12 || ry.abs() < 1e-12 {
        return vec![(Shape::Line { a: p, b: q }, false)];
    }
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    let (c, s) = (phi.cos(), phi.sin());
    let d = (p - q) / 2.0;
    let x1 = DVec2::new(c * d.x + s * d.y, -s * d.x + c * d.y);
    let lam = x1.x * x1.x / (rx * rx) + x1.y * x1.y / (ry * ry);
    if lam > 1.0 {
        rx *= lam.sqrt();
        ry *= lam.sqrt();
    }
    let num = (rx * rx * ry * ry - rx * rx * x1.y * x1.y - ry * ry * x1.x * x1.x).max(0.0);
    let den = rx * rx * x1.y * x1.y + ry * ry * x1.x * x1.x;
    let mut k = (num / den).sqrt();
    if large == sweep {
        k = -k;
    }
    let cp = DVec2::new(k * rx * x1.y / ry, -k * ry * x1.x / rx);
    let centre = DVec2::new(c * cp.x - s * cp.y, s * cp.x + c * cp.y) + (p + q) / 2.0;
    let ang = |v: DVec2| v.y.atan2(v.x);
    let t1 = ang(DVec2::new((x1.x - cp.x) / rx, (x1.y - cp.y) / ry));
    let mut dt = ang(DVec2::new((-x1.x - cp.x) / rx, (-x1.y - cp.y) / ry)) - t1;
    if sweep && dt < 0.0 {
        dt += TAU;
    } else if !sweep && dt > 0.0 {
        dt -= TAU;
    }
    if (rx - ry).abs() <= 1e-9 * rx.max(1.0) {
        // In SVG's y-down space a positive sweep is clockwise on screen;
        // the flip to y-up happens with the rest of the drawing.
        let (a0, a1) = if dt > 0.0 { (t1 + phi, t1 + dt + phi) } else { (t1 + dt + phi, t1 + phi) };
        return vec![(arc(centre, rx, a0, a1), dt < 0.0)];
    }
    let n = ((dt.abs() / TAU) * 72.0).ceil().max(2.0) as usize;
    let pt = |t: f64| centre + DVec2::new(c * rx * t.cos() - s * ry * t.sin(), s * rx * t.cos() + c * ry * t.sin());
    let pts: Vec<DVec2> = (0..=n).map(|i| if i == n { q } else { pt(t1 + dt * i as f64 / n as f64) }).collect();
    pts.windows(2).map(|w| (Shape::Line { a: w[0], b: w[1] }, false)).collect()
}

fn path(d: &str, tol: f64) -> Vec<Shape> {
    let mut out = Vec::new();
    let mut cmds: Vec<(char, Vec<f64>)> = Vec::new();
    let mut cur: Option<(char, String)> = None;
    for ch in d.chars() {
        if ch.is_ascii_alphabetic() && ch != 'e' && ch != 'E' {
            if let Some((c, s)) = cur.take() {
                cmds.push((c, numbers(&s)));
            }
            cur = Some((ch, String::new()));
        } else if let Some((_, s)) = cur.as_mut() {
            s.push(ch);
        }
    }
    if let Some((c, s)) = cur {
        cmds.push((c, numbers(&s)));
    }
    let (mut p, mut start) = (DVec2::ZERO, DVec2::ZERO);
    let mut last_ctrl: Option<(char, DVec2)> = None;
    let line = |out: &mut Vec<Shape>, a: DVec2, b: DVec2| {
        if (b - a).length() > 1e-12 {
            out.push(Shape::Line { a, b });
        }
    };
    for (cmd, a) in cmds {
        let rel = cmd.is_ascii_lowercase();
        let base = |p: DVec2| if rel { p } else { DVec2::ZERO };
        let up = cmd.to_ascii_uppercase();
        let arity = match up {
            'M' | 'L' | 'T' => 2,
            'H' | 'V' => 1,
            'C' => 6,
            'S' | 'Q' => 4,
            'A' => 7,
            _ => 0,
        };
        if up == 'Z' {
            line(&mut out, p, start);
            p = start;
            last_ctrl = None;
            continue;
        }
        if arity == 0 {
            continue;
        }
        for (k, g) in a.chunks_exact(arity).enumerate() {
            let o = base(p);
            match up {
                'M' => {
                    let q = o + DVec2::new(g[0], g[1]);
                    if k == 0 {
                        p = q;
                        start = q;
                    } else {
                        line(&mut out, p, q);
                        p = q;
                    }
                    last_ctrl = None;
                }
                'L' => {
                    let q = o + DVec2::new(g[0], g[1]);
                    line(&mut out, p, q);
                    p = q;
                    last_ctrl = None;
                }
                'H' => {
                    let q = DVec2::new(if rel { p.x + g[0] } else { g[0] }, p.y);
                    line(&mut out, p, q);
                    p = q;
                    last_ctrl = None;
                }
                'V' => {
                    let q = DVec2::new(p.x, if rel { p.y + g[0] } else { g[0] });
                    line(&mut out, p, q);
                    p = q;
                    last_ctrl = None;
                }
                'C' | 'S' | 'Q' | 'T' => {
                    let ctrl: Vec<DVec2> = match up {
                        'C' => vec![o + DVec2::new(g[0], g[1]), o + DVec2::new(g[2], g[3]), o + DVec2::new(g[4], g[5])],
                        'S' => {
                            let c1 = match last_ctrl {
                                Some(('C', c)) => p * 2.0 - c,
                                _ => p,
                            };
                            vec![c1, o + DVec2::new(g[0], g[1]), o + DVec2::new(g[2], g[3])]
                        }
                        'Q' => vec![o + DVec2::new(g[0], g[1]), o + DVec2::new(g[2], g[3])],
                        _ => {
                            let c1 = match last_ctrl {
                                Some(('Q', c)) => p * 2.0 - c,
                                _ => p,
                            };
                            vec![c1, o + DVec2::new(g[0], g[1])]
                        }
                    };
                    let mut pts = vec![p];
                    pts.extend(&ctrl);
                    let mut flat = vec![p];
                    bezier(&pts, tol, &mut flat);
                    for w in flat.windows(2) {
                        line(&mut out, w[0], w[1]);
                    }
                    last_ctrl = Some((if matches!(up, 'C' | 'S') { 'C' } else { 'Q' }, ctrl[ctrl.len() - 2]));
                    p = ctrl[ctrl.len() - 1];
                }
                'A' => {
                    let q = o + DVec2::new(g[5], g[6]);
                    for (s, _) in svg_arc(p, q, g[0], g[1], g[2].to_radians(), g[3] != 0.0, g[4] != 0.0) {
                        out.push(s);
                    }
                    p = q;
                    last_ctrl = None;
                }
                _ => {}
            }
        }
    }
    out
}

fn element(n: roxmltree::Node<'_, '_>, tol: f64) -> Vec<Shape> {
    let f = |k: &str| n.attribute(k).and_then(|v| physical(v)).map(|x| x.0).unwrap_or(0.0);
    match n.tag_name().name() {
        "path" => n.attribute("d").map(|d| path(d, tol)).unwrap_or_default(),
        "line" => {
            let (a, b) = (DVec2::new(f("x1"), f("y1")), DVec2::new(f("x2"), f("y2")));
            if (b - a).length() > 1e-12 {
                vec![Shape::Line { a, b }]
            } else {
                vec![]
            }
        }
        "polyline" | "polygon" => {
            let v = numbers(n.attribute("points").unwrap_or(""));
            let pts: Vec<DVec2> = v.chunks_exact(2).map(|c| DVec2::new(c[0], c[1])).collect();
            let mut out: Vec<Shape> = pts.windows(2).filter(|w| (w[1] - w[0]).length() > 1e-12).map(|w| Shape::Line { a: w[0], b: w[1] }).collect();
            if n.tag_name().name() == "polygon" && pts.len() > 2 && (pts[0] - pts[pts.len() - 1]).length() > 1e-12 {
                out.push(Shape::Line { a: pts[pts.len() - 1], b: pts[0] });
            }
            out
        }
        "rect" => {
            let (x, y, w, h) = (f("x"), f("y"), f("width"), f("height"));
            if w <= 0.0 || h <= 0.0 {
                return vec![];
            }
            let (mut rx, mut ry) = (f("rx"), f("ry"));
            if rx == 0.0 {
                rx = ry;
            }
            if ry == 0.0 {
                ry = rx;
            }
            let r = rx.min(ry).min(w / 2.0).min(h / 2.0);
            if r <= 1e-12 {
                let p = [DVec2::new(x, y), DVec2::new(x + w, y), DVec2::new(x + w, y + h), DVec2::new(x, y + h)];
                return (0..4).map(|i| Shape::Line { a: p[i], b: p[(i + 1) % 4] }).collect();
            }
            use std::f64::consts::{FRAC_PI_2, PI};
            let mut out = Vec::new();
            let corners = [(x + w - r, y + r, -FRAC_PI_2), (x + w - r, y + h - r, 0.0), (x + r, y + h - r, FRAC_PI_2), (x + r, y + r, PI)];
            for (i, (cx, cy, a0)) in corners.iter().enumerate() {
                out.push(arc(DVec2::new(*cx, *cy), r, *a0, a0 + FRAC_PI_2));
                let next = corners[(i + 1) % 4];
                let end = crate::geom::polar(DVec2::new(*cx, *cy), r, a0 + FRAC_PI_2);
                let start = crate::geom::polar(DVec2::new(next.0, next.1), r, next.2);
                if (start - end).length() > 1e-12 {
                    out.push(Shape::Line { a: end, b: start });
                }
            }
            out
        }
        "circle" => {
            let r = f("r");
            if r > 0.0 {
                vec![Shape::Circle { c: DVec2::new(f("cx"), f("cy")), r }]
            } else {
                vec![]
            }
        }
        "ellipse" => {
            let (c, rx, ry) = (DVec2::new(f("cx"), f("cy")), f("rx"), f("ry"));
            if rx <= 0.0 || ry <= 0.0 {
                return vec![];
            }
            if (rx - ry).abs() < 1e-12 {
                return vec![Shape::Circle { c, r: rx }];
            }
            let pts: Vec<DVec2> = (0..=72).map(|i| {
                let t = TAU * i as f64 / 72.0;
                c + DVec2::new(rx * t.cos(), ry * t.sin())
            }).collect();
            pts.windows(2).map(|w| Shape::Line { a: w[0], b: w[1] }).collect()
        }
        _ => vec![],
    }
}

/// Map a shape through an affine map that may mirror or stretch.
fn mapped(s: Shape, m: DMat3) -> Vec<Shape> {
    let (x, y) = (m.x_axis.truncate(), m.y_axis.truncate());
    let (sx, sy) = (x.length(), y.length());
    let similar = (sx - sy).abs() <= 1e-9 * sx.max(1.0) && x.dot(y).abs() <= 1e-9 * sx * sy;
    let mirrored = x.perp_dot(y) < 0.0;
    let ap = |p: DVec2| m.transform_point2(p);
    match s {
        Shape::Line { a, b } => vec![Shape::Line { a: ap(a), b: ap(b) }],
        Shape::Circle { c, r } if similar => vec![Shape::Circle { c: ap(c), r: r * sx }],
        Shape::Arc { c, r, a0, a1 } if similar => {
            let cc = ap(c);
            let (p, q) = (ap(crate::geom::polar(c, r, a0)), ap(crate::geom::polar(c, r, a1)));
            let ang = |v: DVec2| (v - cc).y.atan2((v - cc).x);
            vec![if mirrored { arc(cc, r * sx, ang(q), ang(p)) } else { arc(cc, r * sx, ang(p), ang(q)) }]
        }
        other => {
            let pts: Vec<DVec2> = other.polyline(72).into_iter().map(ap).collect();
            pts.windows(2).map(|w| Shape::Line { a: w[0], b: w[1] }).collect()
        }
    }
}

/// Read SVG text into layers (top-level groups by id or label) in mm when
/// the root states a physical size, else in px at 96 per inch (doubtful).
pub fn read(text: &str) -> Result<Drawing, String> {
    let doc = roxmltree::Document::parse(text).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg" {
        return Err("not an SVG file".into());
    }
    // User units to mm: from width/height and the viewBox.
    let vb = root.attribute("viewBox").map(numbers).filter(|v| v.len() == 4);
    let width = root.attribute("width").and_then(physical);
    let (to_mm, unit) = match (width, &vb) {
        (Some((w, mm)), Some(vb)) if mm.is_finite() && vb[2] > 0.0 => (w * mm / vb[2], Some("mm")),
        (Some((_, mm)), None) if mm.is_finite() => (mm, Some("mm")),
        (Some((w, _)), Some(vb)) if vb[2] > 0.0 => (w / vb[2] * 25.4 / 96.0, None),
        _ => (25.4 / 96.0, None),
    };
    let tol = 0.05 / to_mm.max(1e-9);
    let mut layers: BTreeMap<String, Vec<Shape>> = BTreeMap::new();
    let mut skipped = BTreeMap::new();
    // Flip to y-up and scale to mm at the end.
    let world = DMat3::from_scale(DVec2::new(to_mm, -to_mm));
    fn visit(n: roxmltree::Node<'_, '_>, m: DMat3, layer: &str, tol: f64, layers: &mut BTreeMap<String, Vec<Shape>>, skipped: &mut BTreeMap<String, usize>, depth: usize) {
        for child in n.children().filter(|c| c.is_element()) {
            let name = child.tag_name().name();
            if matches!(name, "defs" | "metadata" | "title" | "desc" | "style" | "clipPath" | "mask" | "symbol" | "pattern" | "marker") {
                continue;
            }
            if child.attribute("display") == Some("none") {
                continue;
            }
            let cm = m * transform(child.attribute("transform"));
            if name == "g" || name == "svg" || name == "a" {
                let label = if depth == 0 {
                    child.attributes().find(|a| a.name() == "label").map(|a| a.value()).or(child.attribute("id")).unwrap_or(layer).to_owned()
                } else {
                    layer.to_owned()
                };
                visit(child, cm, &label, tol, layers, skipped, depth + 1);
                continue;
            }
            let shapes = element(child, tol);
            if shapes.is_empty() && !matches!(name, "path" | "line" | "polyline" | "polygon" | "rect" | "circle" | "ellipse") {
                *skipped.entry(name.to_owned()).or_default() += 1;
                continue;
            }
            let out = layers.entry(layer.to_owned()).or_default();
            for s in shapes {
                out.extend(mapped(s, cm));
            }
        }
    }
    visit(root, DMat3::IDENTITY, "Drawing", tol, &mut layers, &mut skipped, 0);
    let layers = layers
        .into_iter()
        .map(|(name, shapes)| Layer { name, shapes: shapes.into_iter().flat_map(|s| mapped(s, world)).collect() })
        .collect();
    Ok(Drawing { layers, unit: unit.map(str::to_owned), to_mm: Some(1.0), skipped })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_rounded_rect_and_a_circle_in_mm_read_true_size_y_up() {
        let text = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="50mm" viewBox="0 0 100 50">
            <g id="Contour"><rect x="0" y="0" width="100" height="50" rx="5"/></g>
            <g id="Holes"><circle cx="20" cy="10" r="4"/></g><text>x</text></svg>"#;
        let d = read(text).unwrap();
        assert_eq!(d.unit.as_deref(), Some("mm"));
        let holes = d.layers.iter().find(|l| l.name == "Holes").unwrap();
        assert_eq!(holes.shapes, vec![Shape::Circle { c: DVec2::new(20., -10.), r: 4. }]);
        let contour = d.layers.iter().find(|l| l.name == "Contour").unwrap();
        assert_eq!(contour.shapes.iter().filter(|s| matches!(s, Shape::Arc { .. })).count(), 4);
        let mut sk = crate::sketch::Sketch::new(crate::sketch::Use::Profile);
        for s in &contour.shapes {
            sk.add(*s);
        }
        let p = crate::regions::profile(&sk);
        assert!(p.ok(), "{:?}", p.problems);
        assert!((p.area - (5000. - (100. - 25. * std::f64::consts::PI))).abs() < 1e-6, "{}", p.area);
    }
    #[test]
    fn path_arcs_stay_arcs_and_px_files_doubt_their_unit() {
        let text = r#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M 0 0 L 10 0 A 5 5 0 0 1 10 10 L 0 10 Z"/></svg>"#;
        let d = read(text).unwrap();
        assert_eq!(d.unit, None);
        let shapes = &d.layers[0].shapes;
        assert_eq!(shapes.len(), 4);
        let a = shapes.iter().find(|s| matches!(s, Shape::Arc { .. })).unwrap();
        assert!((a.radius().unwrap() - 5. * 25.4 / 96.).abs() < 1e-9);
        // The arc bulges outward (+x), whatever the y flip.
        assert!(a.mid().x > 10. * 25.4 / 96.);
    }
}
