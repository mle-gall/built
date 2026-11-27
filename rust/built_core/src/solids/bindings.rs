// SPDX-License-Identifier: GPL-3.0-or-later
//! The manifold face id names the donor even when an operand is subtracted.
use super::*;
use crate::{
    exact::{
        bound::{EdgeBind, Support},
        Curve, Surface as ExactSurface,
    },
    store::{exact::ChunkMap, keys::Kind, py::PyMeshStore, Store},
};
use glam::{DMat3, DMat4};

#[derive(Default)]
pub(super) struct Donor {
    pub support: Option<Support>,
    rails: Vec<EdgeBind>,
    pub native_smooth: bool,
}

fn source(st: &Store, f: u32) -> Donor {
    if let Some(native) = &st.state.exact.native {
        let topo = native.topology();
        let face = &topo.faces[f as usize];
        let surface = &native.rows.surfaces[face.surface as usize];
        let support = Support::from_pool(&native.rows.curves, surface)
            .filter(|s| !matches!(s.surface, ExactSurface::Plane { .. }));
        let rails = face
            .loops
            .iter()
            .flatten()
            .filter_map(|c| {
                let edge = &topo.edges[c.edge? as usize];
                Some(EdgeBind {
                    curve: native.rows.curves[edge.curve as usize].clone(),
                    interval: edge.interval,
                })
            })
            .collect();
        return Donor {
            native_smooth: support.is_some(),
            support,
            rails,
        };
    }
    let support = st.curved_face(f).cloned().or_else(|| {
        let strip = st.source_strip(f)?;
        let surface = strip.swept.unwrap_or_else(|| {
            if strip.semi_angle.abs() < 1e-13 {
                ExactSurface::Cylinder {
                    frame: strip.frame,
                    radius: strip.radius,
                }
            } else {
                ExactSurface::Cone {
                    frame: strip.frame,
                    radius: strip.radius,
                    half_angle: strip.semi_angle,
                }
            }
        });
        Some(Support::new(surface))
    });
    let rails = st
        .boundary_edges(f)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|(e, _)| {
            st.curved_edge(e).cloned().or_else(|| {
                let arc = st.state.exact.arcs.as_ref()?.get(&e)?;
                Some(EdgeBind {
                    curve: Curve::Circle {
                        frame: (*arc.frame).clone(),
                        radius: arc.radius,
                    },
                    interval: arc.interval,
                })
            })
        })
        .collect();
    Donor {
        support,
        rails,
        native_smooth: false,
    }
}

pub(super) fn donors(mesh: &Obj, placement: Option<DMat4>) -> PyResult<Vec<Donor>> {
    let Some(handle) = mesh.getattr_opt("_store")? else {
        return Ok(vec![]);
    };
    let handle = handle.cast_into::<PyMeshStore>()?;
    let borrow = handle.try_borrow()?;
    let st = &borrow.inner;
    if st.state.exact.curved.is_none()
        && st.state.exact.arcs.is_none()
        && st.state.exact.native.is_none()
    {
        return Ok(vec![]);
    }
    Ok(st
        .live_slots(Kind::Face)
        .into_iter()
        .filter(|&f| st.state.fflags.get(f as usize) & crate::store::INTERIOR == 0)
        .map(|f| {
            let mut donor = source(st, f);
            if let Some(matrix) = placement.filter(|&m| m != DMat4::IDENTITY) {
                let m = DMat3::from_mat4(matrix);
                let t = matrix.w_axis.truncate();
                donor.support = donor.support.and_then(|s| s.mapped(m, t));
                donor.rails.retain_mut(|b| {
                    let Some(c) = b.curve.mapped(m, t) else {
                        return false;
                    };
                    let [Some(a), Some(z)] = b.interval.map(|u| b.curve.point(u).ok()) else {
                        return false;
                    };
                    let Some(interval) = c.interval(m * a + t, m * z + t, Some(b.interval)) else {
                        return false;
                    };
                    *b = EdgeBind { curve: c, interval };
                    true
                });
            }
            donor
        })
        .collect())
}

pub(super) fn apply(mesh: &Obj, ids: &Bound<PyDict>, donors: &[Donor]) -> PyResult<()> {
    if !donors
        .iter()
        .any(|d| d.support.is_some() || !d.rails.is_empty())
    {
        return Ok(());
    }
    let handle = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
    let mut borrow = handle.try_borrow_mut()?;
    let st = &mut borrow.inner;
    let mut edges = ChunkMap::default();
    let mut faces = ChunkMap::default();
    for (key, owner) in ids.iter() {
        let Some(f) = st.slot(Kind::Face, key.extract()?) else {
            continue;
        };
        let Some(d) = owner
            .extract::<Option<usize>>()?
            .and_then(|i| donors.get(i))
        else {
            continue;
        };
        if let Some(s) = &d.support {
            faces.insert(f, s.clone());
        }
        for (e, _) in st.boundary_edges(f).into_iter().flatten().flatten() {
            if edges.contains_key(&e) {
                continue;
            }
            if let Some(b) = d.rails.iter().find_map(|rail| {
                let b = st.edge_holds(e, rail)?;
                let [u, v] = rail.interval;
                // Periodic parameters can differ by a whole turn.
                let shift = if b.curve.periodic() {
                    ((u + v - b.interval[0] - b.interval[1]) / (2. * std::f64::consts::TAU)).round()
                        * std::f64::consts::TAU
                } else {
                    0.
                };
                b.interval
                    .iter()
                    .all(|w| (u.min(v) - 1e-8..=u.max(v) + 1e-8).contains(&(w + shift)))
                    .then_some(b)
            }) {
                edges.insert(e, b);
            }
        }
    }
    st.sections(&st.live_slots(Kind::Edge), &faces, &mut edges);
    st.install_curved_slots(
        edges.iter().map(|(&e, b)| (e, b.clone())).collect(),
        faces.iter().map(|(&f, s)| (f, s.clone())).collect(),
    );
    Ok(())
}
