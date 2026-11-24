// SPDX-License-Identifier: GPL-3.0-or-later
//! Functions exported for legacy tool callers.
use super::*;
fn run_from_edge<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let edge = a.get_item(0)?;
    if let Some(mesh) = edge.getattr_opt("_m")?.filter(|m| !m.is_none()) {
        let store = mesh.getattr("__dict__")?.call_method1("get", ("_store",))?;
        if !store.is_none() {
            return store.call_method1("edge_run", (edge.getattr("key")?,));
        }
    }
    let vec = py.import("tools.offset_host")?.getattr("QVector3D")?;
    let walk = |mut vertex: Obj<'py>| -> PyResult<(Vec<Obj<'py>>, bool)> {
        let mut previous = edge.clone();
        let mut points = Vec::new();
        loop {
            let incident = vertex.getattr("edges")?;
            if incident.len()? != 2 {
                return Ok((points, false));
            }
            let [first, second] = unpack::<2>(&incident)?;
            let next = if first.ne(&previous)? { first } else { second };
            if next.eq(&edge)? {
                return Ok((points, true));
            }
            vertex = next.call_method1("other", (vertex,))?;
            points.push(vec.call1((vertex.getattr("position")?,))?);
            previous = next;
        }
    };
    let start = edge.getattr("v0")?;
    let end = edge.getattr("v1")?;
    let (mut forward, closed) = walk(end.clone())?;
    let mut points = if closed {
        forward.pop();
        Vec::new()
    } else {
        let (mut back, _) = walk(start.clone())?;
        back.reverse();
        back
    };
    points.push(vec.call1((start.getattr("position")?,))?);
    points.push(vec.call1((end.getattr("position")?,))?);
    points.extend(forward);
    (points, closed).into_bound_py_any(py)
}

#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("_run_from_edge", Mode::Function, run_from_edge, &["edge:Edge"], "", "The run ``edge`` belongs to, walked both ways through vertices\nwhere exactly two edges meet (a corner of a polyline); a junction or a\nfree end stops the walk. ``(points, closed)``."),
];
