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
pub struct CatalogueEntry {
    pub name: String,
    #[serde(default)]
    pub series: Vec<(String, f64)>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalogue: Option<CatalogueEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sketch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drawing: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub library: String,
    /// The family of a size that is not in the built-in catalogue, carried
    /// with the row (a detail pack's bundled profile table): `params` then
    /// holds every dimension.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub family: String,
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
/// Section properties of a drawn outline, in millimetres: area, centroid,
/// second moments about the centroidal x (horizontal, "y-y" in EN tables)
/// and y (vertical, "z-z") axes, the product moment, the principal moments
/// and the elastic moduli to the extreme fibres.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Properties {
    pub area: f64,
    pub cx: f64,
    pub cy: f64,
    pub i_xx: f64,
    pub i_yy: f64,
    pub i_xy: f64,
    pub i_u: f64,
    pub i_v: f64,
    pub w_xx: f64,
    pub w_yy: f64,
    pub perimeter: f64,
}
impl Outline {
    /// Exact for the outline's facets (Green's theorem over each ring; the
    /// first ring is the outer boundary, the others are holes).
    pub fn properties(&self) -> Properties {
        let (mut a, mut sx, mut sy, mut ixx, mut iyy, mut ixy, mut perimeter) = (0., 0., 0., 0., 0., 0., 0.);
        for (k, ring) in self.rings.iter().enumerate() {
            let n = ring.len();
            let (mut ra, mut rsx, mut rsy, mut rxx, mut ryy, mut rxy) = (0., 0., 0., 0., 0., 0.);
            for i in 0..n {
                let (p, q) = (ring[i].p, ring[(i + 1) % n].p);
                let c = p.perp_dot(q);
                ra += c / 2.;
                rsx += (p.x + q.x) * c / 6.;
                rsy += (p.y + q.y) * c / 6.;
                rxx += (p.y * p.y + p.y * q.y + q.y * q.y) * c / 12.;
                ryy += (p.x * p.x + p.x * q.x + q.x * q.x) * c / 12.;
                rxy += (p.x * q.y + 2. * p.x * p.y + 2. * q.x * q.y + q.x * p.y) * c / 24.;
                perimeter += p.distance(q);
            }
            // The outer ring counts positive whatever its winding; holes subtract.
            let s = if k == 0 { ra.signum() } else { -ra.signum() };
            a += s * ra;
            sx += s * rsx;
            sy += s * rsy;
            ixx += s * rxx;
            iyy += s * ryy;
            ixy += s * rxy;
        }
        let (cx, cy) = (sx / a, sy / a);
        let i_xx = ixx - a * cy * cy;
        let i_yy = iyy - a * cx * cx;
        let i_xy = ixy - a * cx * cy;
        let mean = (i_xx + i_yy) / 2.;
        let half = (((i_xx - i_yy) / 2.).powi(2) + i_xy * i_xy).sqrt();
        let (mut ymax, mut xmax) = (0f64, 0f64);
        for node in self.rings.iter().flatten() {
            ymax = ymax.max((node.p.y - cy).abs());
            xmax = xmax.max((node.p.x - cx).abs());
        }
        Properties {
            area: a,
            cx,
            cy,
            i_xx,
            i_yy,
            i_xy,
            i_u: mean + half,
            i_v: mean - half,
            w_xx: i_xx / ymax,
            w_yy: i_yy / xmax,
            perimeter,
        }
    }
}
impl Profile {
    pub fn name(&self) -> Option<&str> {
        self.catalogue.as_ref().map(|entry| entry.name.as_str()).or_else(|| label(&self.library))
    }
    pub fn outline(&self, segments: usize) -> Result<Outline, &'static str> {
        if let Some(drawing) = &self.drawing {
            return super::drawn_profile::outline(drawing, segments);
        }
        let size = SIZES.table.sizes.iter().find(|p| p.key == self.library);
        let (family, table) = match size {
            Some(size) => (size.family.as_str(), Some(&size.params)),
            None if is_family(&self.family) => (self.family.as_str(), None),
            None => return Err("profile_missing"),
        };
        let get = |k: &str| {
            self.params
                .get(k)
                .or_else(|| table.and_then(|p| p.get(k)))
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
        // Optional refinements (0 or absent keeps the plain family): the
        // flange toe radius, the inner flange slope in percent with the
        // distance from the flange tip where `flange` is measured, and a
        // tube's own inner corner radius.
        let toe = get("toe");
        let slope = get("slope") / 100.;
        if !(w > 0. && h > 0. && (4..=512).contains(&segments) && r >= 0. && toe >= 0.) {
            return Err("invalid_profile");
        }
        let fam = family;
        if !matches!(fam, "rectangle" | "round") && !(t > 0. && t < w && t < h) {
            return Err("invalid_profile");
        }
        if matches!(fam, "i" | "channel" | "tee" | "zed") && !(f > 0. && 2. * f < h) {
            return Err("invalid_profile");
        }
        // Flange thickness at the tip and at the web face of a sloped flange.
        let outstand = if fam == "channel" { w - t } else { (w - t) / 2. };
        let at = match self.params.get("flange_at").or_else(|| table.and_then(|p| p.get("flange_at"))) {
            Some(&a) => a,
            None => outstand / 2.,
        };
        let f_tip = f - slope * at;
        let f_root = f + slope * (outstand - at);
        if !(slope >= 0. && slope < 1. && (0. ..=outstand).contains(&at)) {
            return Err("invalid_profile");
        }
        if matches!(fam, "i" | "channel" | "tee") && !(f_tip > 0. && 2. * f_root < h) {
            return Err("invalid_profile");
        }
        let polygon = |points: &[(f64, f64, f64)]| -> Result<Vec<Node>, &'static str> {
            let n = points.len();
            let pt = |j: usize| DVec2::new(points[j % n].0, points[j % n].1);
            // A fillet of radius r in a corner of interior angle θ touches
            // both edges at r / tan(θ/2) from the corner.
            let corner = |i: usize| -> Option<(DVec2, DVec2, f64)> {
                let p = pt(i);
                let a = (pt(i + n - 1) - p).try_normalize()?;
                let b = (pt(i + 1) - p).try_normalize()?;
                let theta = a.dot(b).clamp(-1., 1.).acos();
                Some((a, b, theta))
            };
            let tangent = |i: usize| -> Result<f64, &'static str> {
                let radius = points[i % n].2;
                if radius == 0. {
                    return Ok(0.);
                }
                let (_, _, theta) = corner(i).ok_or("invalid_profile")?;
                if !(theta > 1e-6 && theta < std::f64::consts::PI - 1e-6) {
                    return Err("invalid_profile");
                }
                Ok(radius / (theta / 2.).tan())
            };
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
                let (a, b, theta) = corner(i).ok_or("invalid_profile")?;
                let d = tangent(i)?;
                let prev = (pt(i + n - 1) - p).length();
                let next = (pt(i + 1) - p).length();
                if d >= prev || d >= next || d + tangent(i + 1)? > next {
                    return Err("invalid_profile");
                }
                let centre = p + (a + b).normalize() * (radius / (theta / 2.).sin());
                let sweep = std::f64::consts::PI - theta;
                let start = p + a * d - centre;
                let turn = if start.perp_dot(p + b * d - centre) < 0. { -1. } else { 1. };
                let steps = ((segments as f64 * sweep / std::f64::consts::TAU).ceil() as usize).max(1);
                for k in 0..=steps {
                    let angle = turn * sweep * k as f64 / steps as f64;
                    ring.push(Node {
                        p: centre + DVec2::from_angle(angle).rotate(start),
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
        let (ft, fr) = (f_tip, f_root);
        let rings = match fam {
            "rectangle" => vec![rect(0., 0.)?],
            "i" => vec![polygon(&[
                (0., 0., 0.),
                (w, 0., 0.),
                (w, ft, toe),
                (u, fr, r),
                (u, h - fr, r),
                (w, h - ft, toe),
                (w, h, 0.),
                (0., h, 0.),
                (0., h - ft, toe),
                (l, h - fr, r),
                (l, fr, r),
                (0., ft, toe),
            ])?],
            "channel" => vec![polygon(&[
                (0., 0., 0.),
                (w, 0., 0.),
                (w, ft, toe),
                (t, fr, r),
                (t, h - fr, r),
                (w, h - ft, toe),
                (w, h, 0.),
                (0., h, 0.),
            ])?],
            "angle" => vec![polygon(&[
                (0., 0., 0.),
                (w, 0., 0.),
                (w, t, toe),
                (t, t, r),
                (t, h, toe),
                (0., h, 0.),
            ])?],
            "tee" => vec![polygon(&[
                (l, 0., 0.),
                (u, 0., 0.),
                (u, h - fr, r),
                (w, h - ft, toe),
                (w, h, 0.),
                (0., h, 0.),
                (0., h - ft, toe),
                (l, h - fr, r),
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
                let inner = match get("inner") {
                    i if i > 0. => i,
                    _ => (r - t).max(0.),
                };
                vec![rect(0., r)?, rect(t, inner)?]
            }
            "chs" | "round" if 2. * t < w && w == h => {
                let centre = DVec2::splat(mid);
                [mid, mid - t]
                    .into_iter().take(if fam == "round" { 1 } else { 2 })
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
            family: family.to_owned(),
            rings,
            width: w,
            height: h,
            web: t,
            flange: f,
            radius: r,
        })
    }
}

/// The parametric families the generator draws.
pub fn is_family(name: &str) -> bool {
    matches!(
        name,
        "i" | "rhs" | "rectangle" | "channel" | "angle" | "chs" | "round" | "tee" | "zed"
    )
}

/// Whether a size is in the built-in catalogue.
pub fn built_in(key: &str) -> bool {
    SIZES.table.sizes.iter().any(|s| s.key == key)
}

/// Catalogue sizes of `key`'s series (same family, key stem and suffix),
/// ordered by height: what "± 1 catalogue step" counts in a junction rule.
pub fn series(key: &str) -> Vec<(String, f64)> {
    fn split(key: &str) -> (&str, &str) {
        let at = key.find(|c: char| c.is_ascii_digit()).unwrap_or(key.len());
        let rest = &key[at..];
        let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(rest.len());
        (&key[..at], &rest[end..])
    }
    let Some(me) = SIZES.table.sizes.iter().find(|s| s.key == key) else {
        return Vec::new();
    };
    let (stem, suffix) = split(key);
    let mut out: Vec<(String, f64)> = SIZES
        .table
        .sizes
        .iter()
        .filter(|s| s.family == me.family && split(&s.key) == (stem, suffix))
        .map(|s| (s.key.clone(), s.params.get("height").copied().unwrap_or(0.)))
        .collect();
    out.sort_by(|a, b| a.1.total_cmp(&b.1));
    out
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
    let profile = Profile { catalogue: None, sketch: None, drawing: None, library: key.into(), family: String::new(), params: Default::default() };
    sketch_of(&profile)
}
pub fn sketch_of(profile: &Profile) -> Option<serde_json::Value> {
    use built_draft2d::{geom, Shape, Sketch, Use};
    let outline = profile.outline(24).ok()?;
    let mut sketch = Sketch::new(Use::Profile);
    sketch.name = profile.name().unwrap_or(&profile.library).to_owned();
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
        let table = Profile { catalogue: None, sketch: None, drawing: None, library: key.into(), family: String::new(), params: Default::default() }.outline(512).unwrap();
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
