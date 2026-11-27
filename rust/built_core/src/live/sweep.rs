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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chain: Vec<super::Reference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ends: Option<[End; 2]>,
    #[serde(default)]
    pub closed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sweep {
    pub profile: Profile,
    pub path: Path,
    #[serde(default = "centre")]
    pub attachment: String,
    #[serde(default)]
    pub rotation: f64,
    #[serde(default)]
    pub mirror: bool,
    #[serde(default = "segments")]
    pub segments: usize,
    #[serde(default)]
    pub preset: Option<String>,
}
fn centre() -> String {
    "centre".into()
}
fn segments() -> usize {
    24
}

pub fn generate(store: &mut Store, sweep: &Sweep, path: &[[f64; 3]]) -> Result<(), &'static str> {
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
    let offset = outline.attachment(&sweep.attachment)?;
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
    let mut roles = Vec::new();
    let mut vertices = HashMap::new();
    let mut change = Change::default();
    let mut bindings = crate::store::exact::Curved::default();
    for (s, station) in stations.iter().enumerate() {
        let mut cap = Vec::new();
        for (r, points) in station.iter().enumerate() {
            let nodes = &outline.rings[r];
            let reversed = (r > 0) ^ sweep.mirror;
            let mut ring = points.clone();
            if reversed ^ (s == 0) {
                ring.reverse()
            }
            cap.push(ring);
            for (j, p) in points.iter().enumerate() {
                let slot = store
                    .vertex(*p, &mut change)
                    .map_err(|_| "invalid_profile")?;
                let role = format!("{}/{r}/{}/station/{s}", outline.family, nodes[j].role);
                if vertices.insert(slot, role.clone()).is_some() {
                    return Err("collapsed_profile");
                }
                roles.push((format!("vertex/{role}"), Kind::Vertex, slot));
                if sweep.path.closed || s + 1 < stations.len() {
                    let next = &stations[(s + 1) % stations.len()][r];
                    let k = (j + 1) % points.len();
                    let mut quad = vec![*p, points[k], next[k], next[j]];
                    if reversed {
                        quad.reverse()
                    }
                    let face = store
                        .add_face(&[quad], &mut change)
                        .map_err(|_| "invalid_profile")?;
                    roles.push((format!("side/{role}"), Kind::Face, face));
                    let prev = (j + nodes.len() - 1) % nodes.len();
                    if nodes[j].arc.is_some() || nodes[prev].arc.is_some() {
                        let far = store
                            .vertex(next[j], &mut change)
                            .map_err(|_| "invalid_profile")?;
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
                            let a = store
                                .vertex(ring[j], &mut change)
                                .map_err(|_| "invalid_profile")?;
                            let b = store
                                .vertex(ring[k], &mut change)
                                .map_err(|_| "invalid_profile")?;
                            let e = store.find_edge(a, b).ok_or("invalid_profile")?;
                            let [p, q] = store
                                .state
                                .ends
                                .get(e as usize)
                                .map(|v| DVec3::from(*store.state.vpos.get(v as usize)));
                            let interval = curve.interval(p, q, None).ok_or("invalid_profile")?;
                            if station == s {
                                bindings.faces.insert(
                                    face,
                                    bound::extrusion(curve.clone(), directions[s].into())
                                        .ok_or("invalid_profile")?,
                                );
                            }
                            bindings.edges.insert(e, EdgeBind { curve, interval });
                        }
                    }
                }
            }
        }
        if !sweep.path.closed && (s == 0 || s + 1 == stations.len()) {
            let f = store
                .add_face(&cap, &mut change)
                .map_err(|_| "invalid_profile")?;
            roles.push((
                if s == 0 { "cap/start" } else { "cap/end" }.into(),
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
    geometry::assign_roles(store, roles)
}
