// SPDX-License-Identifier: GPL-3.0-or-later
//! Dimensions and sketch coordinates are millimetres. The store uses metres.
use glam::DVec2;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::LazyLock;
pub const CATALOGUE: &str = include_str!("../../../../resources/profiles/starter.builtprofile");
#[derive(Deserialize)]
pub struct Size {
    pub key: String,
    pub label: String,
    pub family: String,
    pub params: BTreeMap<String, f64>,
}
#[derive(Deserialize)]
struct Catalogue {
    table: Sizes,
}
#[derive(Deserialize)]
struct Sizes {
    sizes: Vec<Size>,
}
static SIZES: LazyLock<Catalogue> =
    LazyLock::new(|| serde_json::from_str(CATALOGUE).expect("starter profiles"));
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sketch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drawing: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub library: String,
    #[serde(default)]
    pub params: BTreeMap<String, f64>,
}
pub struct Node {
    pub p: DVec2,
    /// Centre of a circular edge from this vertex to the next.
    pub arc: Option<DVec2>,
    pub role: String,
}
pub struct Outline {
    pub named: BTreeMap<String, DVec2>,
    pub family: String,
    pub rings: Vec<Vec<Node>>,
    pub width: f64,
    pub height: f64,
    pub web: f64,
    pub flange: f64,
    pub radius: f64,
}
impl Outline {
    pub fn attachment(&self, name: &str) -> Result<DVec2, &'static str> {
        let uv = match name {
            "centre" => [0.5, 0.5],
            "bottom" | "bottom_centre" => [0.5, 0.],
            "top" | "top_centre" => [0.5, 1.],
            "left" | "middle_left" => [0., 0.5],
            "right" | "middle_right" => [1., 0.5],
            "bottom_left" => [0., 0.],
            "bottom_right" => [1., 0.],
            "top_left" => [0., 1.],
            "top_right" => [1., 1.],
            _ => return self.named.get(name).copied().ok_or("attachment_missing"),
        };
        Ok(DVec2::from(uv) * DVec2::new(self.width, self.height))
    }
}
impl Profile {
    pub fn outline(&self, segments: usize) -> Result<Outline, &'static str> {
        if let Some(drawing) = &self.drawing {
            return super::drawn_profile::outline(drawing, segments);
        }
        let size = SIZES
            .table
            .sizes
            .iter()
            .find(|p| p.key == self.library)
            .ok_or("profile_missing")?;
        let get = |k: &str| {
            self.params
                .get(k)
                .or_else(|| size.params.get(k))
                .copied()
                .unwrap_or(0.)
        };
        let (w, h, t, f, r) = (
            get("width"),
            get("height"),
            get("web"),
            get("flange"),
            get("radius"),
        );
        if !(w > 0. && h > 0. && (4..=512).contains(&segments) && r >= 0.) {
            return Err("invalid_profile");
        }
        let fam = size.family.as_str();
        if fam != "rectangle" && !(t > 0. && t < w && t < h) {
            return Err("invalid_profile");
        }
        if matches!(fam, "i" | "channel" | "tee" | "zed") && !(f > 0. && 2. * f < h) {
            return Err("invalid_profile");
        }
        let polygon = |points: &[(f64, f64, f64)]| -> Result<Vec<Node>, &'static str> {
            let mut ring = Vec::new();
            for (i, &(x, y, radius)) in points.iter().enumerate() {
                let p = DVec2::new(x, y);
                if radius == 0. {
                    ring.push(Node {
                        p,
                        arc: None,
                        role: format!("corner/{i}"),
                    });
                    continue;
                }
                let pt = |j: usize| DVec2::new(points[j].0, points[j].1);
                let prev = pt((i + points.len() - 1) % points.len()) - p;
                let next = pt((i + 1) % points.len()) - p;
                if radius >= prev.length()
                    || radius >= next.length()
                    || radius + points[(i + 1) % points.len()].2 > next.length()
                {
                    return Err("invalid_profile");
                }
                let a = prev.normalize();
                let b = next.normalize();
                let centre = p + radius * (a + b);
                let steps = segments.div_ceil(4);
                for k in 0..=steps {
                    let angle = std::f64::consts::FRAC_PI_2 * k as f64 / steps as f64;
                    ring.push(Node {
                        p: centre - radius * (b * angle.cos() + a * angle.sin()),
                        arc: (k < steps).then_some(centre),
                        role: format!("round/{i}/{k}"),
                    });
                }
            }
            Ok(ring)
        };
        let rect = |d: f64, rad: f64| {
            polygon(&[
                (d, d, rad),
                (w - d, d, rad),
                (w - d, h - d, rad),
                (d, h - d, rad),
            ])
        };
        let mid = w / 2.;
        let l = mid - t / 2.;
        let u = mid + t / 2.;
        let rings = match fam {
            "rectangle" => vec![rect(0., 0.)?],
            "i" => vec![polygon(&[
                (0., 0., 0.),
                (w, 0., 0.),
                (w, f, 0.),
                (u, f, r),
                (u, h - f, r),
                (w, h - f, 0.),
                (w, h, 0.),
                (0., h, 0.),
                (0., h - f, 0.),
                (l, h - f, r),
                (l, f, r),
                (0., f, 0.),
            ])?],
            "channel" => vec![polygon(&[
                (0., 0., 0.),
                (w, 0., 0.),
                (w, f, 0.),
                (t, f, r),
                (t, h - f, r),
                (w, h - f, 0.),
                (w, h, 0.),
                (0., h, 0.),
            ])?],
            "angle" => vec![polygon(&[
                (0., 0., 0.),
                (w, 0., 0.),
                (w, t, 0.),
                (t, t, r),
                (t, h, 0.),
                (0., h, 0.),
            ])?],
            "tee" => vec![polygon(&[
                (l, 0., 0.),
                (u, 0., 0.),
                (u, h - f, r),
                (w, h - f, 0.),
                (w, h, 0.),
                (0., h, 0.),
                (0., h - f, 0.),
                (l, h - f, r),
            ])?],
            "zed" => vec![polygon(&[
                (0., 0., 0.),
                (u, 0., 0.),
                (u, h - f, 0.),
                (w, h - f, 0.),
                (w, h, 0.),
                (l, h, 0.),
                (l, f, 0.),
                (0., f, 0.),
            ])?],
            "rhs" if 2. * t < w.min(h) && 2. * r < w.min(h) => {
                vec![rect(0., r)?, rect(t, (r - t).max(0.))?]
            }
            "chs" if 2. * t < w && w == h => {
                let centre = DVec2::splat(mid);
                [mid, mid - t]
                    .into_iter()
                    .map(|rad| {
                        (0..segments)
                            .map(|i| Node {
                                p: centre
                                    + DVec2::from_angle(
                                        std::f64::consts::TAU * i as f64 / segments as f64,
                                    ) * rad,
                                arc: Some(centre),
                                role: format!("circle/{i}"),
                            })
                            .collect()
                    })
                    .collect()
            }
            _ => return Err("invalid_profile"),
        };
        Ok(Outline {
            named: Default::default(),
            family: size.family.clone(),
            rings,
            width: w,
            height: h,
            web: t,
            flange: f,
            radius: r,
        })
    }
}

pub fn label(key: &str) -> Option<&'static str> {
    SIZES
        .table
        .sizes
        .iter()
        .find(|s| s.key == key)
        .map(|s| s.label.as_str())
}

/// A catalogue size as an exact drawing (lines, arcs, circles in mm), so a
/// member's size can be opened in the Sketch workspace. Runs of facets
/// about one centre become one arc; a ring about one centre is a circle.
pub fn catalogue_sketch(key: &str) -> Option<serde_json::Value> {
    use built_draft2d::{geom, Shape, Sketch, Use};
    let outline = Profile { sketch: None, drawing: None, library: key.into(), params: Default::default() }.outline(24).ok()?;
    let mut sketch = Sketch::new(Use::Profile);
    sketch.name = label(key)?.to_owned();
    for ring in &outline.rings {
        let n = ring.len();
        if n >= 3 && ring.iter().all(|node| node.arc.is_some_and(|c| c.distance(ring[0].arc.unwrap()) < 1e-9)) {
            let c = ring[0].arc.unwrap();
            sketch.add(Shape::Circle { c, r: ring[0].p.distance(c) });
            continue;
        }
        // Start on a node that begins a run (its edge differs from the one before).
        let same = |a: Option<DVec2>, b: Option<DVec2>| match (a, b) {
            (Some(a), Some(b)) => a.distance(b) < 1e-9,
            (None, None) => false,
            _ => false,
        };
        let Some(start) = (0..n).find(|&i| !same(ring[(i + n - 1) % n].arc, ring[i].arc)) else { return None };
        let mut i = 0;
        while i < n {
            let k = (start + i) % n;
            let node = &ring[k];
            match node.arc {
                None => {
                    let b = ring[(k + 1) % n].p;
                    if node.p.distance(b) > 1e-9 {
                        sketch.add(Shape::Line { a: node.p, b });
                    }
                    i += 1;
                }
                Some(c) => {
                    let mut j = i + 1;
                    while j < n && same(ring[(start + j) % n].arc, Some(c)) {
                        j += 1;
                    }
                    let (p, q) = (node.p, ring[(start + j) % n].p);
                    let m = ring[(k + 1) % n].p;
                    let r = p.distance(c);
                    // The ring runs p → q; an arc is counter-clockwise from a0 to a1.
                    let ccw = (m - c).perp_dot(p - c) < 0.;
                    let (from, to) = if ccw { (p, q) } else { (q, p) };
                    let a0 = geom::angle_from(c, from, 0.);
                    sketch.add(geom::arc(c, r, a0, geom::angle_from(c, to, a0)));
                    i = j;
                }
            }
        }
    }
    Some(sketch.to_json())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(key: &str) -> (f64, f64) {
        let drawn = catalogue_sketch(key).unwrap();
        let s = built_draft2d::Sketch::from_json(&drawn).unwrap();
        let p = built_draft2d::profile(&s);
        assert!(p.ok(), "{key}: {:?}", p.problems);
        let table = Profile { sketch: None, drawing: None, library: key.into(), params: Default::default() }.outline(512).unwrap();
        let ring_area = |ring: &Vec<Node>| {
            ring.iter().enumerate().map(|(i, n)| n.p.perp_dot(ring[(i + 1) % ring.len()].p) / 2.).sum::<f64>().abs()
        };
        let a = ring_area(&table.rings[0]) - table.rings[1..].iter().map(ring_area).sum::<f64>();
        (p.area, a)
    }

    #[test]
    fn catalogue_sizes_open_as_exact_closed_drawings() {
        for size in &SIZES.table.sizes {
            let (drawn, faceted) = area(&size.key);
            assert!((drawn - faceted).abs() / faceted < 1e-3, "{}: {drawn} vs {faceted}", size.key);
        }
    }
}
