// SPDX-License-Identifier: GPL-3.0-or-later
mod bands;
mod display_mesh;
mod exact_read;
const DISPLAY_DEFLECTION: f64 = 0.00005;
mod exact_write;
use exact_write::Geometry;
mod import;
mod pcurve;
mod planar_merge;
mod rim_solid;
mod splines;
mod swept;
mod units;
use crate::math::mat;
use crate::store::py::PyMeshStore;
use built_step::{planar::Body, step_io::build::Frame, Definition};
use glam::{DMat4, DVec3};
use pyo3::{exceptions::PyValueError, prelude::*};
use std::{collections::HashMap, path::PathBuf};

fn bad(e: impl ToString) -> PyErr {
    PyValueError::new_err(e.to_string())
}

fn bodies(mesh: &Bound<'_, PyAny>) -> PyResult<Vec<Geometry>> {
    let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
    let borrow = store.try_borrow()?;
    let s = &borrow.inner;
    if let Some(patch) = &s.state.exact.native {
        return Ok(vec![Geometry::Exact(patch.clone())]);
    }
    if s.state.exact.arcs.is_some() {
        let patch = rim_solid::recover(s)
            .or_else(|_| bands::recover(s))
            .map_err(bad)?;
        return Ok(vec![Geometry::Exact(std::sync::Arc::new(patch))]);
    }
    let d = s.dense();
    if d.ef_start.windows(2).any(|w| w[0] == w[1]) {
        return Err(bad("STEP solid contains loose edges"));
    }
    if d.ring_counts.is_empty() {
        return Ok(vec![]);
    }
    let mut points = d.corners.into_iter();
    let mut sizes = d.ring_sizes.into_iter();
    let faces: Vec<Vec<Vec<usize>>> = d
        .ring_counts
        .iter()
        .map(|&n| {
            sizes
                .by_ref()
                .take(n as usize)
                .map(|len| {
                    points
                        .by_ref()
                        .take(len as usize)
                        .map(|i| i as usize)
                        .collect()
                })
                .collect()
        })
        .collect();
    let mut adjacent = vec![Vec::new(); faces.len()];
    for range in d.ef_start.windows(2) {
        let fs = &d.ef_face[range[0] as usize..range[1] as usize];
        if fs.len() > 2 {
            return Err(bad("nonmanifold STEP edge"));
        }
        if let [a, b] = fs {
            adjacent[*a as usize].push(*b as usize);
            adjacent[*b as usize].push(*a as usize);
        }
    }
    let mut soft = vec![Vec::new(); faces.len()];
    for (i, &flags) in d.flags.iter().enumerate() {
        if flags & crate::store::SOFT != 0 && d.ef_start[i + 1] - d.ef_start[i] == 2 {
            soft[d.ef_face[d.ef_start[i] as usize] as usize].push(d.edges[i]);
        }
    }
    let mut seen = vec![false; faces.len()];
    let mut bodies = Vec::new();
    for root in 0..faces.len() {
        if seen[root] {
            continue;
        }
        let mut body = Body::default();
        let mut stack = vec![root];
        let mut vertices = HashMap::new();
        let mut soft_edges = Vec::new();
        while let Some(f) = stack.pop() {
            if std::mem::replace(&mut seen[f], true) {
                continue;
            }
            stack.extend(&adjacent[f]);
            soft_edges.extend_from_slice(&soft[f]);
            body.faces.push(
                faces[f]
                    .iter()
                    .map(|r| {
                        r.iter()
                            .map(|&v| {
                                *vertices.entry(v).or_insert_with(|| {
                                    body.points.push(DVec3::from(
                                        s.state.frame.definition_point(d.pos[v]),
                                    ));
                                    body.points.len() - 1
                                })
                            })
                            .collect()
                    })
                    .collect(),
            );
        }
        if !soft_edges.is_empty() {
            let soft = soft_edges
                .into_iter()
                .map(|[a, b]| {
                    let a = vertices[&(a as usize)];
                    let b = vertices[&(b as usize)];
                    (a.min(b), a.max(b))
                })
                .collect();
            planar_merge::merge(&mut body, &soft);
        }
        bodies.push(Geometry::Facets(body));
    }
    Ok(bodies)
}

fn placement(m: DMat4) -> PyResult<Frame> {
    let [x, y, z, t] = m.to_cols_array_2d().map(|a| DVec3::new(a[0], a[1], a[2]));
    if !m.is_finite()
        || m.row(3) != glam::DVec4::W
        || (x.cross(y) - z).length() > 1e-12
        || (y - z.cross(x)).length() > 1e-12
    {
        return Err(bad("STEP occurrence requires a rigid placement"));
    }
    Ok(Frame {
        origin: t.to_array(),
        axis: z.to_array(),
        ref_dir: x.to_array(),
    })
}
type BranchKey = (usize, Vec<(usize, [u64; 9])>);
fn children(
    groups: &Bound<'_, PyAny>,
    defs: &mut Vec<Definition<Geometry>>,
    known: &mut HashMap<BranchKey, usize>,
    depth: u32,
) -> PyResult<Vec<(usize, Frame)>> {
    if depth > 256 {
        return Err(bad("STEP assembly nesting limit exceeded"));
    }
    let mut placements = Vec::new();
    for g in groups.try_iter()? {
        let g = g?;
        let mesh = g.getattr("mesh")?;
        let nested = children(&g.getattr("children")?, defs, known, depth + 1)?;
        let key = (
            mesh.as_ptr() as usize,
            nested
                .iter()
                .map(|(child, frame)| {
                    let mut bits = [0; 9];
                    for (i, x) in frame
                        .origin
                        .into_iter()
                        .chain(frame.axis)
                        .chain(frame.ref_dir)
                        .enumerate()
                    {
                        bits[i] = if x == 0. { 0 } else { x.to_bits() };
                    }
                    (*child, bits)
                })
                .collect(),
        );
        let id = if let Some(&id) = known.get(&key) {
            id
        } else {
            let id = defs.len();
            defs.push(Definition {
                name: g.getattr("name")?.extract()?,
                bodies: bodies(&mesh)?,
                children: nested,
            });
            known.insert(key, id);
            id
        };
        let xf = g.getattr("xform")?;
        let m = if xf.is_none() {
            DMat4::IDENTITY
        } else {
            mat(&xf)?
        };
        placements.push((id, placement(m)?));
    }
    Ok(placements)
}
#[pyfunction]
fn save_step(scene: &Bound<'_, PyAny>, path: PathBuf) -> PyResult<()> {
    let mut defs = vec![Definition {
        name: "Model".into(),
        bodies: bodies(&match scene.getattr_opt("loose_mesh")? {
            Some(m) => m,
            None => scene.getattr("mesh")?,
        })?,
        children: vec![],
    }];
    defs[0].children = children(&scene.getattr("groups")?, &mut defs, &mut HashMap::new(), 0)?;
    // An imported assembly already has its product root. Promote a single
    // identity placement to avoid adding a wrapper on every round trip.
    if let [(_, at)] = defs[0].children.as_slice() {
        if defs[0].bodies.is_empty()
            && at.origin == [0.; 3]
            && at.axis == [0., 0., 1.]
            && at.ref_dir == [1., 0., 0.]
        {
            defs.remove(0);
            for d in &mut defs {
                for (id, _) in &mut d.children {
                    *id -= 1;
                }
            }
        }
    }
    scene
        .py()
        .detach(|| {
            let tol = defs
                .iter()
                .flat_map(|d| &d.bodies)
                .filter_map(|g| match g {
                    Geometry::Exact(p) => Some(p.topology().tolerance),
                    _ => None,
                })
                .fold(1e-10_f64, f64::max);
            built_step::write_checked(
                &defs,
                &path,
                tol * 1000.,
                |g, b, p, name| g.add(b, p, name, tol),
                |m, s, scale| {
                    exact_write::reopened(m, s, scale, tol).map(built_step::Measure::Volume)
                },
                |m, s, sense, scale| {
                    exact_write::reopened_sheet(m, s, sense, scale, tol)
                        .map(built_step::Measure::Area)
                },
            )
        })
        .map_err(bad)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(import::prepare_step, m)?)?;
    m.add_function(wrap_pyfunction!(import::apply_step, m)?)?;
    m.add_function(wrap_pyfunction!(save_step, m)?)?;
    Ok(())
}
