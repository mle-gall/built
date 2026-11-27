// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::{
    exact::Frame,
    store::{
        keys::{Kind, NONE},
        py::PyMeshStore,
        Store,
    },
};
use glam::DVec3;
use std::collections::{HashMap, HashSet};

fn slots(store: &Store, faces: &Obj<'_>) -> PyResult<Vec<u32>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for k in faces.try_iter()? {
        let key = keys::from_json_key(&k?)?.extract()?;
        let f = store
            .slot(Kind::Face, key)
            .ok_or_else(|| PyValueError::new_err("stale face selection"))?;
        if seen.insert(f) {
            out.push(f)
        }
    }
    if out.is_empty() {
        return Err(PyValueError::new_err("Select faces to process"));
    }
    Ok(out)
}

struct Fit {
    faces: Vec<(u32, Frame)>,
    max_residual: f64,
    volume: Option<f64>,
    reasons: Vec<String>,
}
fn fit(st: &Store, slots: &[u32], tolerance: f64) -> PyResult<Fit> {
    if !tolerance.is_finite() || tolerance <= 0. {
        return Err(PyValueError::new_err(
            "expected a positive tolerance in metres",
        ));
    }
    if st.state.exact.native.is_some() {
        return Err(PyValueError::new_err(
            "Selection already has exact topology",
        ));
    }
    let mut plan = Fit {
        faces: vec![],
        max_residual: 0.,
        volume: None,
        reasons: vec![],
    };
    let mut incidence = HashMap::<(u32, u32), (usize, i32)>::new();
    let mut volume = 0.;
    let mut datum = None;
    let point = |v: u32| {
        DVec3::from(
            st.state
                .frame
                .definition_point(*st.state.vpos.get(v as usize)),
        )
    };
    for &f in slots {
        let rings = st.face_rings(f);
        let outer = &rings[0];
        let p = point(outer[0]);
        let base = *datum.get_or_insert(p);
        let area = |r: &[u32]| {
            let start = point(r[0]);
            r[1..]
                .windows(2)
                .map(|w| (point(w[0]) - start).cross(point(w[1]) - start))
                .sum::<DVec3>()
        };
        let Some(axis) = area(outer).try_normalize() else {
            plan.reasons
                .push(format!("Face {} has zero area", st.pid(Kind::Face, f)));
            continue;
        };
        let basis = if axis.x.abs() < 0.8 {
            DVec3::X
        } else {
            DVec3::Y
        };
        let reference = (basis - axis * axis.dot(basis)).normalize();
        let frame = Frame {
            origin: axis * axis.dot(p),
            axis,
            reference,
        };
        let residual = rings
            .iter()
            .flatten()
            .map(|&v| axis.dot(point(v) - frame.origin).abs())
            .fold(0., f64::max);
        plan.max_residual = plan.max_residual.max(residual);
        for ring in &rings {
            volume += (point(ring[0]) - base).dot(area(ring)) / 6.;
            for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                let row = incidence.entry((a.min(b), a.max(b))).or_default();
                row.0 += 1;
                row.1 += if a < b { 1 } else { -1 };
            }
        }
        if residual > tolerance || st.validate_plane(f, &frame).is_err() {
            plan.reasons.push(format!(
                "Face {} has no verified planar trim",
                st.pid(Kind::Face, f)
            ));
        } else {
            plan.faces.push((f, frame))
        }
    }
    if !incidence.is_empty() && incidence.values().all(|r| *r == (2, 0)) {
        plan.volume = Some(volume)
    }
    Ok(plan)
}

#[pyclass(frozen)]
struct Edit {
    arcs: Vec<Vec<(u32, crate::store::source_arcs::ArcBinding)>>,
    revision: u64,
    slots: Vec<u32>,
    planes: Option<Vec<(u32, Frame)>>,
}
#[pymethods]
impl Edit {
    fn __call__(&self, scene: &Obj<'_>) -> PyResult<()> {
        let mesh = scene.getattr("mesh")?;
        let handle = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
        let mut held = handle.try_borrow_mut()?;
        if held.inner.revision != self.revision {
            return Err(PyValueError::new_err(
                "Selection changed after recovery preview",
            ));
        }
        let mut st = held.inner.clone();
        for ring in &self.arcs {
            let group = mesh.call_method0("next_curve_id")?.extract()?;
            for (e, arc) in ring {
                st.bind_source_arc(*e, arc.clone())
                    .map_err(PyValueError::new_err)?;
                st.set_edge_curve(*e, group);
            }
        }
        if let Some(planes) = &self.planes {
            for (f, plane) in planes {
                st.bind_plane(*f, plane.clone())
                    .map_err(PyValueError::new_err)?
            }
        } else if st.state.exact.native.is_some() {
            if self.slots.len() != st.n(Kind::Face) {
                return Err(PyValueError::new_err(
                    "Select all faces of the exact patch to make it faceted",
                ));
            }
            st = st.facet_view().map_err(PyValueError::new_err)?;
        } else {
            for &f in &self.slots {
                for (e, _) in st.boundary_edges(f).into_iter().flatten().flatten() {
                    st.state.exact.alloc(Kind::Edge, e)
                }
                st.state.exact.inherit(Kind::Face, f, NONE)
            }
            st.revision += 1;
        }
        held.inner = st;
        drop(held);
        mesh.call_method0("_bump")?;
        Ok(())
    }
}
#[pyfunction]
#[pyo3(signature=(history,faces,tolerance=0.0001,preview=true))]
fn recover_exact_geometry<'p>(
    history: &Obj<'p>,
    faces: &Obj<'p>,
    tolerance: f64,
    preview: bool,
) -> Reply<'p> {
    let py = history.py();
    let h = history
        .getattr("scene")?
        .getattr("mesh")?
        .getattr("_store")?
        .cast_into::<PyMeshStore>()?;
    let st = h.try_borrow()?;
    let slots = slots(&st.inner, faces)?;
    let mut remaining: HashSet<_> = slots.iter().copied().collect();
    let mut parts = Vec::new();
    let mut planes = Vec::new();
    let mut arcs = Vec::new();
    let mut reasons = Vec::new();
    let mut count = 0;
    let mut max_residual = 0f64;
    let mut total_input = Some(0.);
    let mut total_output = Some(0.);
    let mut methods = Vec::new();
    for &root in &slots {
        if !remaining.remove(&root) {
            continue;
        }
        let mut stack = vec![root];
        let mut component = Vec::new();
        while let Some(f) = stack.pop() {
            component.push(f);
            for (edge, _) in st.inner.boundary_edges(f).into_iter().flatten().flatten() {
                stack.extend(
                    st.inner
                        .edge_faces(edge)
                        .into_iter()
                        .filter(|n| remaining.remove(n)),
                );
            }
        }
        let mut part = fit(&st.inner, &component, tolerance)?;
        let mut method = "Planar supports";
        let mut output = part.volume;
        let faces = part.faces.len();
        let mut curved = 0;
        if let Some(round) = part
            .volume
            .and_then(|v| super::recover_round::cylinder(&st.inner, &component, tolerance, v))
        {
            part.faces.retain(|(f, _)| !round.curved.contains(f));
            method = round.method;
            output = Some(round.volume);
            part.max_residual = round.residual;
            arcs.extend(round.rings);
            curved = round.supports;
        }
        let row = PyDict::new(py);
        row.set_item("first_face", st.inner.pid(Kind::Face, root))?;
        row.set_item("method", method)?;
        row.set_item("faces", faces)?;
        row.set_item("planar_faces", part.faces.len())?;
        row.set_item("curved_supports", curved)?;
        row.set_item("max_residual_m", part.max_residual)?;
        row.set_item("input_volume_m3", part.volume)?;
        row.set_item("output_volume_m3", output)?;
        row.set_item("untouched", &part.reasons)?;
        parts.push(row);
        if !methods.contains(&method) {
            methods.push(method);
        }
        total_input = total_input.zip(part.volume).map(|(a, b)| a + b);
        total_output = total_output.zip(output).map(|(a, b)| a + b);
        planes.extend(part.faces);
        reasons.extend(part.reasons);
        count += faces;
        max_residual = max_residual.max(part.max_residual);
    }
    let edit = Edit {
        slots,
        arcs,
        revision: st.inner.revision,
        planes: Some(planes),
    };
    drop(st);
    let report = if preview {
        PyDict::new(py)
    } else {
        if count == 0 {
            return Err(PyValueError::new_err("No verified planar faces"));
        }
        execute(
            history,
            &history_command(py, "SnapshotMutation", (Bound::new(py, edit)?,))?,
        )?
    };
    report.set_item("parts", parts)?;
    report.set_item("method", methods.join(", "))?;
    report.set_item("faces", count)?;
    report.set_item("max_residual_m", max_residual)?;
    report.set_item("input_volume_m3", total_input)?;
    report.set_item("output_volume_m3", total_output)?;
    report.set_item("untouched", reasons)?;
    Ok(report)
}
#[pyfunction]
fn make_faceted<'p>(history: &Obj<'p>, faces: &Obj<'p>) -> Reply<'p> {
    let py = history.py();
    let h = history
        .getattr("scene")?
        .getattr("mesh")?
        .getattr("_store")?
        .cast_into::<PyMeshStore>()?;
    let st = h.try_borrow()?;
    let edit = Edit {
        revision: st.inner.revision,
        slots: slots(&st.inner, faces)?,
        planes: None,
        arcs: vec![],
    };
    drop(st);
    execute(
        history,
        &history_command(py, "SnapshotMutation", (Bound::new(py, edit)?,))?,
    )
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(recover_exact_geometry, m)?)?;
    m.add_function(wrap_pyfunction!(make_faceted, m)?)?;
    Ok(())
}
