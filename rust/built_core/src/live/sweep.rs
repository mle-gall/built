// SPDX-License-Identifier: GPL-3.0-or-later
use super::{geometry, profile::Profile};
use crate::{
    exact::{
        bound::{self, EdgeBind},
        Curve, Frame,
    },
    facegeom::Hypot,
    store::{edit::Change, keys::Kind, sweep::projected_stations, Store},
};
use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Path {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub definition_space: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chain: Vec<super::Reference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ends: Option<[End; 2]>,
    #[serde(default)]
    pub closed: bool,
    /// The member a part rested on when a junction pinned its anchored end
    /// (docs/junctions.md §5): capture still gathers it with that member.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rests_on: Option<String>,
    /// An extrusion (a slab, a pad): the path runs this far, in millimetres,
    /// from its first point, along `dir` or towards its second point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<f64>,
    /// The extrusion's direction in the parent's frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<[f64; 3]>,
}
/// The shape of an extrusion's edge between its outline and an end face:
/// the profile of a slab's edge, a footing's step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    /// `square`, `chamfer`, `round`, `step` or `downstand`.
    pub shape: String,
    /// How far the edge reaches in from the outline, millimetres (negative:
    /// out from it).
    #[serde(default)]
    pub size: f64,
    /// How far it reaches along the thickness, millimetres (the size when 0).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub depth: f64,
    /// Which end face it meets: `start`, `end` or `both`.
    #[serde(default = "start")]
    pub side: String,
}
fn is_zero(n: &f64) -> bool {
    *n == 0.
}
fn start() -> String {
    "start".into()
}
/// Sized from another member: each field is an expression over the host's
/// dimensions (`host.width`, `host.height`, `host.thickness`, `host.length`,
/// millimetres), written into this member before it is evaluated.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fit {
    /// The host: `{"sweep": "<row uid>", "at": "start" | "end"}`. A preset
    /// has none: the element names its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<super::Reference>,
    /// `profile.params.<name>`, `path.depth`, `shift.x`, `shift.y` or
    /// `rotation` → expression.
    pub values: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged, try_from = "serde_json::Value")]
pub enum End {
    Point([f64; 3]),
    Anchored {
        to: super::Reference,
        #[serde(default)]
        offset: f64,
        #[serde(default)]
        extend: bool,
    },
}
// Decode coordinates straight from JSON. The untagged enum visitor otherwise
// buffers arbitrary-precision decimal numbers as maps before trying a variant.
impl TryFrom<serde_json::Value> for End {
    type Error = serde_json::Error;
    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        if value.is_array() {
            return serde_json::from_value(value).map(Self::Point);
        }
        #[derive(Deserialize)]
        struct Anchor {
            to: super::Reference,
            #[serde(default)]
            offset: f64,
            #[serde(default)]
            extend: bool,
        }
        let anchor: Anchor = serde_json::from_value(value)?;
        Ok(Self::Anchored {
            to: anchor.to,
            offset: anchor.offset,
            extend: anchor.extend,
        })
    }
}
impl Path {
    pub fn references(&self) -> impl Iterator<Item = &super::Reference> {
        self.chain
            .iter()
            .chain(self.ends.iter().flatten().filter_map(|e| match e {
                End::Anchored { to, .. } => Some(to),
                _ => None,
            }))
    }
}
impl Sweep {
    /// Every reference the member resolves: its path's, and its host's.
    pub fn references(&self) -> impl Iterator<Item = &super::Reference> {
        self.path.references().chain(self.fit.iter().filter_map(|f| f.host.as_ref()))
    }
    /// The profile point that sits on the path: the attachment, moved by
    /// the section shift (which is given in the path's frame).
    pub fn origin(&self, outline: &super::profile::Outline) -> Result<DVec2, &'static str> {
        let at = outline.attachment(&self.attachment)?;
        if self.shift == [0.; 2] {
            return Ok(at);
        }
        let mut back = DVec2::from_angle(-self.rotation.to_radians()).rotate(DVec2::from(self.shift));
        if self.mirror {
            back.x = -back.x;
        }
        Ok(at - back)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sweep {
    pub profile: Profile,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cut_definitions: Vec<serde_json::Value>,
    pub path: Path,
    #[serde(default = "centre")]
    pub attachment: String,
    #[serde(default)]
    pub rotation: f64,
    #[serde(default = "mitre")]
    pub corner: String,
    #[serde(default)]
    pub mirror: bool,
    #[serde(default = "segments")]
    pub segments: usize,
    #[serde(default)]
    pub preset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset_uid: Option<String>,
    #[serde(default)]
    pub preset_rev: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset_definition: Option<super::presets::Preset>,
    /// The edge of an extrusion (`path.depth`): a slab's edge profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<Edge>,
    /// The section moved off the path, in the path's own frame (across, up),
    /// millimetres.
    #[serde(default, skip_serializing_if = "no_shift")]
    pub shift: [f64; 2],
    /// Sized from a host member (a footing from its column or wall).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit: Option<Fit>,
}
fn no_shift(v: &[f64; 2]) -> bool {
    *v == [0.; 2]
}
fn mitre() -> String {
    "mitre".into()
}
fn centre() -> String {
    "centre".into()
}
fn segments() -> usize {
    24
}

/// The placement frame of a new member: origin at the path start (the
/// profile's attachment point), X along the first span, Y and Z the
/// section's width and height axes as `generate` lays them out. Written in
/// this frame the path starts at the origin along X, so the generated
/// section is the same and the member's box and axes follow it.
pub fn member_frame(points: &[[f64; 3]]) -> Option<glam::DMat4> {
    let start = DVec3::from(*points.first()?);
    let dir = (DVec3::from(*points.get(1)?) - start).try_normalize()?;
    let across = DVec3::Z.cross(dir).try_normalize().unwrap_or(DVec3::X);
    let up = dir.cross(across);
    Some(glam::DMat4::from_cols(
        dir.extend(0.),
        across.extend(0.),
        up.extend(0.),
        start.extend(1.),
    ))
}

pub struct SpanGeometry {
    pub butt_planes: Vec<[super::joints::Plane; 2]>,
    pub zones: Vec<[[f64; 2]; 3]>,
}
/// Cache end sections in local coordinates. Placements with the same section,
/// axis and clipping plane reuse the projected ring and its analytic frame.
fn project_end(
    station: &mut crate::store::sweep::Station,
    at: DVec3,
    axis: DVec3,
    origin: DVec3,
    normal: DVec3,
) {
    use std::{cell::RefCell, collections::HashMap};
    type Station = crate::store::sweep::Station;
    thread_local! {static ENDS: RefCell<HashMap<Vec<u64>, Station>> = RefCell::default();}
    let height = (origin - at).dot(normal);
    let local: Station = station
        .iter()
        .map(|ring| {
            ring.iter()
                .map(|p| (DVec3::from(*p) - at).to_array())
                .collect()
        })
        .collect();
    let mut key: Vec<u64> = axis
        .to_array()
        .into_iter()
        .chain(normal.to_array())
        .chain([height])
        .map(f64::to_bits)
        .collect();
    for ring in &local {
        key.push(ring.len() as u64);
        key.extend(ring.iter().flatten().map(|x| x.to_bits()));
    }
    ENDS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= 256 && !cache.contains_key(&key) {
            cache.clear();
        }
        let projected = cache.entry(key).or_insert_with(|| {
            local
                .into_iter()
                .map(|ring| {
                    ring.into_iter()
                        .map(|v| {
                            let p = DVec3::from(v);
                            (p + axis * ((height - p.dot(normal)) / axis.dot(normal))).to_array()
                        })
                        .collect()
                })
                .collect()
        });
        for (dst, p) in station.iter_mut().flatten().zip(projected.iter().flatten()) {
            *dst = (at + DVec3::from(*p)).to_array();
        }
    });
}

pub fn generate(
    store: &mut Store,
    sweep: &Sweep,
    path: &[[f64; 3]],
    planes: &[super::cut::EndPlane],
) -> Result<SpanGeometry, &'static str> {
    let outline = sweep.profile.outline(sweep.segments)?;
    if path.len() < 2 {
        return Err("invalid_path");
    }
    let start = DVec3::from(path[0]);
    let dir = (DVec3::from(path[1]) - start)
        .try_normalize()
        .ok_or("invalid_path")?;
    let across = DVec3::Z.cross(dir).try_normalize().unwrap_or(DVec3::X);
    let up = dir.cross(across);
    let offset = sweep.origin(&outline)?;
    if let Some(edge) = sweep.edge.as_ref().filter(|e| e.shape != "square") {
        return extrude_with_edge(store, sweep, &outline, edge, path, planes, [start, dir, across, up], offset);
    }
    let local = |pt: DVec2| {
        let mut p = (pt - offset) / 1000.;
        if sweep.mirror {
            p.x = -p.x
        }
        let p = DVec2::from_angle(sweep.rotation.to_radians()).rotate(p);
        across * p.x + up * p.y
    };
    let mut first: Vec<Vec<[f64; 3]>> = outline
        .rings
        .iter()
        .map(|ring| {
            ring.iter()
                .map(|n| (start + local(n.p)).to_array())
                .collect()
        })
        .collect();
    // Transport the section frame through exactly the same mitre projections as
    // its vertices. A circular trim on an oblique joint becomes an ellipse.
    first.push(vec![
        start.to_array(),
        (start + across).to_array(),
        (start + up).to_array(),
    ]);
    let (mut stations, directions) =
        projected_stations(first, path, sweep.path.closed, Hypot::LibStdCxx, true)
            .ok_or("invalid_path")?;
    let butt = sweep.corner == "butt" && directions.len() > 1;
    if !matches!(sweep.corner.as_str(), "mitre" | "butt") {
        return Err("invalid_corner");
    }
    let mut trims = Vec::new();
    if butt {
        (stations, trims) =
            super::joints::butt_stations(&stations, &directions, path, sweep.path.closed)?;
    }
    let closed = sweep.path.closed;
    let count = directions.len();
    let separate = butt
        || planes.iter().any(|p| {
            closed
                || if p.end {
                    p.span + 1 < count
                } else {
                    p.span > 0
                }
        });
    if separate && !butt {
        stations = (0..count)
            .flat_map(|i| [stations[i].clone(), stations[(i + 1) % path.len()].clone()])
            .collect();
    }
    let station_index =
        |span: usize, end: bool| (if separate { 2 * span } else { span }) + usize::from(end);
    if !planes.is_empty() {
        if trims.is_empty() {
            let normal = |i: usize| {
                if !closed && (i == 0 || i == count) {
                    directions[i.min(count - 1)]
                } else {
                    (DVec3::from(directions[(i + count - 1) % count])
                        + DVec3::from(directions[i % count]))
                    .normalize()
                    .to_array()
                }
            };
            trims = (0..count)
                .map(|i| {
                    [
                        [path[i], normal(i)],
                        [path[(i + 1) % path.len()], normal(i + 1)],
                    ]
                })
                .collect();
        }
        for plane in planes {
            let end = usize::from(plane.end);
            let axis = DVec3::from(directions[plane.span]);
            let denom = axis.dot(plane.normal);
            if denom.abs() < 1e-9 {
                return Err("end_unresolved");
            }
            project_end(
                &mut stations[station_index(plane.span, plane.end)],
                path[(plane.span + end) % path.len()].into(),
                axis,
                plane.origin,
                plane.normal,
            );
            trims[plane.span][end] = [
                plane.origin.to_array(),
                (plane.normal * denom.signum()).to_array(),
            ];
        }
        for (i, axis) in directions.iter().enumerate() {
            if stations[station_index(i, false)][0]
                .iter()
                .zip(&stations[station_index(i, true)][0])
                .any(|(a, b)| (DVec3::from(*b) - DVec3::from(*a)).dot((*axis).into()) <= 1e-9)
            {
                return Err("empty_cut");
            }
        }
    }
    let zones = super::joints::span_zones(&stations, &directions, path, separate);
    let frames: Vec<_> = stations
        .iter_mut()
        .map(|station| {
            let basis = station.pop().unwrap();
            let centre = DVec3::from(basis[0]);
            let x = DVec3::from(basis[1]) - centre;
            let y = DVec3::from(basis[2]) - centre;
            let map = glam::DMat3::from_cols(x, y, x.cross(y).normalize_or_zero())
                * glam::DMat3::from_cols(across, up, dir).transpose();
            (centre, map)
        })
        .collect();
    let namespace = sweep
        .profile
        .sketch
        .as_ref()
        .map(|id| format!("sketch/{id}"))
        .unwrap_or_else(|| outline.family.clone());
    let mut roles = Vec::new();
    let mut vertices = HashMap::new();
    let mut change = Change::default();
    let mut bindings = crate::store::exact::Curved::default();
    let mut slots = Vec::new();
    for station in &stations {
        let mut rings = Vec::new();
        for ring in station {
            let mut points = Vec::new();
            for p in ring {
                let slot = if separate {
                    store.shell_vertex(*p, &mut change)
                } else {
                    store.vertex(*p, &mut change)
                }
                .map_err(|_| "invalid_profile")?;
                points.push(slot);
            }
            rings.push(points);
        }
        slots.push(rings);
    }
    for (s, station) in slots.iter().enumerate() {
        let start_cap = if separate {
            s % 2 == 0
        } else {
            !sweep.path.closed && s == 0
        };
        let end_cap = if separate {
            s % 2 == 1
        } else {
            !sweep.path.closed && s + 1 == slots.len()
        };
        let span = if separate { s / 2 } else { s };
        let mut cap = Vec::new();
        for (r, points) in station.iter().enumerate() {
            let nodes = &outline.rings[r];
            let reversed = (r > 0) ^ sweep.mirror;
            let mut ring = points.clone();
            if reversed ^ start_cap {
                ring.reverse()
            }
            cap.push(ring);
            for (j, &slot) in points.iter().enumerate() {
                let source_station = if separate && !butt {
                    span + usize::from(end_cap)
                } else {
                    s
                };
                let mut role = format!(
                    "{}/{r}/{}/station/{source_station}",
                    namespace, nodes[j].role
                );
                if separate && !butt && end_cap && (closed || span + 1 < count) {
                    role.push_str("/incoming");
                }
                if vertices.insert(slot, role.clone()).is_some() {
                    return Err("collapsed_profile");
                }
                roles.push((format!("vertex/{role}"), Kind::Vertex, slot));
                if !end_cap {
                    let next = &slots[(s + 1) % slots.len()][r];
                    let k = (j + 1) % points.len();
                    let mut quad = vec![slot, points[k], next[k], next[j]];
                    if reversed {
                        quad.reverse()
                    }
                    let face = store
                        .add_welded_face(&[quad], &mut change)
                        .map_err(|_| "invalid_profile")?;
                    roles.push((format!("side/{role}"), Kind::Face, face));
                    let prev = (j + nodes.len() - 1) % nodes.len();
                    if nodes[j].arc.is_some() || nodes[prev].arc.is_some() {
                        let far = next[j];
                        let seam = store.find_edge(slot, far).ok_or("invalid_profile")?;
                        let flags = if nodes[j].arc == nodes[prev].arc {
                            crate::store::SOFT
                        } else {
                            crate::store::SOFT | crate::store::FLAT
                        };
                        store.set_edge_flags(seam, flags);
                    }
                    if let Some(c) = nodes[j].arc {
                        let radius = (nodes[j].p - c).length() / 1000.;
                        let source = Curve::Circle {
                            frame: Frame {
                                origin: local(c),
                                axis: dir,
                                reference: across,
                            },
                            radius,
                        };
                        for (station, ring) in [(s, points), ((s + 1) % stations.len(), next)] {
                            let (centre, map) = frames[station];
                            let curve = source.mapped(map, centre).ok_or("invalid_profile")?;
                            let e = store.find_edge(ring[j], ring[k]).ok_or("invalid_profile")?;
                            let [p, q] = store
                                .state
                                .ends
                                .get(e as usize)
                                .map(|v| DVec3::from(*store.state.vpos.get(v as usize)));
                            let interval = curve.interval(p, q, None).ok_or("invalid_profile")?;
                            if station == s {
                                bindings.faces.insert(
                                    face,
                                    bound::extrusion(curve.clone(), directions[span].into())
                                        .ok_or("invalid_profile")?,
                                );
                            }
                            bindings.edges.insert(e, EdgeBind { curve, interval });
                        }
                    }
                }
            }
        }
        if start_cap || end_cap {
            let f = store
                .add_welded_face(&cap, &mut change)
                .map_err(|_| "invalid_profile")?;
            roles.push((
                if let Some(p) = planes
                    .iter()
                    .rev()
                    .find(|p| station_index(p.span, p.end) == s)
                {
                    format!("cut/{}/plane", p.uid)
                } else if s == 0 {
                    "cap/start".into()
                } else if s + 1 == slots.len() {
                    "cap/end".into()
                } else {
                    format!("cap/butt/{s}")
                },
                Kind::Face,
                f,
            ));
        }
    }
    for e in store.live_slots(Kind::Edge) {
        let mut ends = store.state.ends.get(e as usize).map(|v| &vertices[&v]);
        ends.sort();
        roles.push((format!("edge/{}/{}", ends[0], ends[1]), Kind::Edge, e));
    }
    if !bindings.edges.is_empty() {
        store.state.exact.curved = Some(Arc::new(bindings));
    }
    geometry::assign_roles(store, roles)?;
    Ok(SpanGeometry {
        butt_planes: trims,
        zones,
    })
}

/// The rings of an edge treatment at one end, from the end face inwards:
/// `(inset mm, distance from the end plane mm, smooth)` for each extra ring,
/// then where the full outline starts.
fn edge_levels(edge: &Edge, segments: usize) -> Result<(Vec<(f64, f64, bool)>, f64), &'static str> {
    let size = edge.size;
    let depth = if edge.depth == 0. { size.abs() } else { edge.depth };
    if !(size.is_finite() && depth.is_finite()) || size == 0. || depth <= 0. {
        return Err("invalid_edge");
    }
    Ok(match edge.shape.as_str() {
        "chamfer" => (vec![(size, 0., false)], depth),
        "round" => {
            let n = segments.div_ceil(4).max(2);
            (
                (0..n)
                    .map(|k| {
                        let a = std::f64::consts::FRAC_PI_2 * k as f64 / n as f64;
                        (size * (1. - a.sin()), depth * (1. - a.cos()), k > 0)
                    })
                    .collect(),
                depth,
            )
        }
        "step" => (vec![(size, 0., false), (size, depth, false)], depth),
        // An edge beam under a slab: past the end face by `depth`, `size` wide.
        "downstand" if size > 0. => (vec![(size, 0., false), (size, -depth, false)], -depth),
        _ => return Err("invalid_edge"),
    })
}

/// The outer ring moved in by `inset` millimetres (mitred corners).
fn inset_ring(ring: &[super::profile::Node], inset: f64) -> Result<Vec<DVec2>, &'static str> {
    let n = ring.len();
    if inset == 0. {
        return Ok(ring.iter().map(|node| node.p).collect());
    }
    let normal = |i: usize| {
        let d = ring[(i + 1) % n].p - ring[i].p;
        d.try_normalize().map(|d| DVec2::new(-d.y, d.x))
    };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (Some(a), Some(b)) = (normal((i + n - 1) % n), normal(i)) else {
            return Err("invalid_profile");
        };
        let den = 1. + a.dot(b);
        if den < 1e-6 {
            return Err("edge_too_large");
        }
        out.push(ring[i].p + (a + b) * (inset / den));
    }
    // An edge that turned round means the inset ate a side.
    for i in 0..n {
        let k = (i + 1) % n;
        if (out[k] - out[i]).dot(ring[k].p - ring[i].p) <= 1e-9 {
            return Err("edge_too_large");
        }
    }
    Ok(out)
}

/// An extrusion whose outline changes near its end faces (`Sweep::edge`):
/// a stack of rings, the full outline between them. Holes run straight
/// from one end face to the other.
#[allow(clippy::too_many_arguments)]
fn extrude_with_edge(
    store: &mut Store,
    sweep: &Sweep,
    outline: &super::profile::Outline,
    edge: &Edge,
    path: &[[f64; 3]],
    planes: &[super::cut::EndPlane],
    [start, dir, across, up]: [DVec3; 4],
    offset: DVec2,
) -> Result<SpanGeometry, &'static str> {
    if path.len() != 2 || sweep.path.closed || !planes.is_empty() {
        return Err("edge_needs_extrusion");
    }
    let length = (DVec3::from(path[1]) - start).length();
    let (extras, main) = edge_levels(edge, sweep.segments)?;
    // (role, inset mm, station along the path in metres, smooth ring)
    let mut levels: Vec<(String, f64, f64, bool)> = Vec::new();
    let (at_start, at_end) = match edge.side.as_str() {
        "start" => (true, false),
        "end" => (false, true),
        "both" => (true, true),
        _ => return Err("invalid_edge"),
    };
    let mut first = 0.;
    let mut last = length;
    if at_start {
        for (i, (inset, z, smooth)) in extras.iter().enumerate() {
            levels.push((format!("s{i}"), *inset, z / 1000., *smooth));
        }
        first = main / 1000.;
    }
    levels.push(("0".into(), 0., first, false));
    if at_end {
        last = length - main / 1000.;
    }
    levels.push(("1".into(), 0., last, false));
    if at_end {
        for (i, (inset, z, smooth)) in extras.iter().enumerate().rev() {
            levels.push((format!("e{i}"), *inset, length - z / 1000., *smooth));
        }
    }
    if last - first <= 1e-9 {
        return Err("edge_too_large");
    }
    let local = |pt: DVec2| {
        let mut p = (pt - offset) / 1000.;
        if sweep.mirror {
            p.x = -p.x
        }
        let p = DVec2::from_angle(sweep.rotation.to_radians()).rotate(p);
        across * p.x + up * p.y
    };
    let namespace = sweep
        .profile
        .sketch
        .as_ref()
        .map(|id| format!("sketch/{id}"))
        .unwrap_or_else(|| outline.family.clone());
    let mut change = Change::default();
    let mut roles = Vec::new();
    let mut vertices = HashMap::new();
    let mut ring_slots = |store: &mut Store, r: usize, tag: &str, points: Vec<DVec2>, z: f64| -> Result<Vec<u32>, &'static str> {
        let mut slots = Vec::with_capacity(points.len());
        for (j, p) in points.into_iter().enumerate() {
            let slot = store
                .vertex((start + dir * z + local(p)).to_array(), &mut change)
                .map_err(|_| "invalid_profile")?;
            let role = format!("{namespace}/{r}/{}/station/{tag}", outline.rings[r][j].role);
            if vertices.insert(slot, role.clone()).is_some() {
                return Err("edge_too_large");
            }
            roles.push((format!("vertex/{role}"), Kind::Vertex, slot));
            slots.push(slot);
        }
        Ok(slots)
    };
    // The outer ring at every level; holes at the two end faces only.
    let mut outer = Vec::new();
    for (tag, inset, z, _) in &levels {
        outer.push(ring_slots(store, 0, tag, inset_ring(&outline.rings[0], *inset)?, *z)?);
    }
    let (z0, z1) = (levels[0].2, levels[levels.len() - 1].2);
    let mut holes = Vec::new();
    for r in 1..outline.rings.len() {
        let points = || outline.rings[r].iter().map(|n| n.p).collect::<Vec<_>>();
        holes.push([ring_slots(store, r, "0", points(), z0)?, ring_slots(store, r, "1", points(), z1)?]);
    }
    drop(ring_slots);
    let mut faces = Vec::new();
    let mut wall = |store: &mut Store, r: usize, tag: &str, a: &[u32], b: &[u32]| -> Result<(), &'static str> {
        let nodes = &outline.rings[r];
        let reversed = (r > 0) ^ sweep.mirror;
        for j in 0..a.len() {
            let k = (j + 1) % a.len();
            let mut quad = vec![a[j], a[k], b[k], b[j]];
            if reversed {
                quad.reverse()
            }
            let face = store.add_welded_face(&[quad], &mut change).map_err(|_| "edge_too_large")?;
            faces.push((format!("side/{namespace}/{r}/{}/station/{tag}", nodes[j].role), face));
            let prev = (j + nodes.len() - 1) % nodes.len();
            if nodes[j].arc.is_some() || nodes[prev].arc.is_some() {
                let seam = store.find_edge(a[j], b[j]).ok_or("invalid_profile")?;
                let flags = if nodes[j].arc == nodes[prev].arc { crate::store::SOFT } else { crate::store::SOFT | crate::store::FLAT };
                store.set_edge_flags(seam, flags);
            }
        }
        Ok(())
    };
    let mut ledges = Vec::new();
    for (i, pair) in outer.windows(2).enumerate() {
        if (levels[i].2 - levels[i + 1].2).abs() < 1e-12 {
            ledges.push(i);
        } else {
            wall(store, 0, &levels[i].0, &pair[0], &pair[1])?;
        }
    }
    for (r, [a, b]) in holes.iter().enumerate() {
        wall(store, r + 1, "0", a, b)?;
    }
    drop(wall);
    // Two rings at one station: a flat ledge, one face with the inner ring
    // as its hole, facing the way the solid narrows.
    for i in ledges {
        let (a, mut b) = (outer[i].clone(), outer[i + 1].clone());
        let forward = levels[i + 1].1 > levels[i].1;
        b.reverse();
        let mut rings = if forward { vec![a, b] } else { vec![b, a] };
        if sweep.mirror {
            rings.iter_mut().for_each(|r| r.reverse());
        }
        let face = store.add_welded_face(&rings, &mut change).map_err(|_| "edge_too_large")?;
        faces.push((format!("ledge/{namespace}/{}", levels[i].0), face));
    }
    // A rounded edge shades as one surface.
    for (i, (_, _, _, smooth)) in levels.iter().enumerate() {
        if *smooth {
            let ring = &outer[i];
            for j in 0..ring.len() {
                if let Some(e) = store.find_edge(ring[j], ring[(j + 1) % ring.len()]) {
                    store.set_edge_flags(e, crate::store::SOFT);
                }
            }
        }
    }
    for (name, is_start) in [("cap/start", true), ("cap/end", false)] {
        let mut cap = Vec::new();
        let level = if is_start { 0 } else { outer.len() - 1 };
        for r in 0..outline.rings.len() {
            let mut ring = if r == 0 { outer[level].clone() } else { holes[r - 1][usize::from(!is_start)].clone() };
            if ((r > 0) ^ sweep.mirror) ^ is_start {
                ring.reverse()
            }
            cap.push(ring);
        }
        let f = store.add_welded_face(&cap, &mut change).map_err(|_| "edge_too_large")?;
        faces.push((name.into(), f));
    }
    roles.extend(faces.into_iter().map(|(role, f)| (role, Kind::Face, f)));
    for e in store.live_slots(Kind::Edge) {
        let mut ends = store.state.ends.get(e as usize).map(|v| &vertices[&v]);
        ends.sort();
        roles.push((format!("edge/{}/{}", ends[0], ends[1]), Kind::Edge, e));
    }
    geometry::assign_roles(store, roles)?;
    Ok(SpanGeometry {
        butt_planes: Vec::new(),
        zones: vec![[[0., 0.], [0., length], [length, length]]],
    })
}
