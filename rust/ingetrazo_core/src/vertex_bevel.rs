// SPDX-License-Identifier: GPL-3.0-or-later
//! Build the tool preview from the same vertex-rounding plan that the store applies.
use pyo3::{prelude::*, types::PyDict};
use std::collections::HashSet;

#[pyfunction]
#[pyo3(signature=(mesh,vertices,size,segments=8,profile=0.5))]
pub(crate) fn plan_vertex_bevel<'p>(
    mesh: &Bound<'p, PyAny>,
    vertices: &Bound<'p, PyAny>,
    size: f64,
    segments: i32,
    profile: f64,
) -> PyResult<Bound<'p, PyAny>> {
    let py = mesh.py();
    let mut seen = HashSet::new();
    let mut keys = Vec::new();
    for vertex in vertices.try_iter()? {
        let key = vertex?.getattr("key")?.extract::<u128>()?;
        if seen.insert(key) {
            keys.push(key);
        }
    }
    let refusal = if keys.is_empty() {
        Some("Pick a corner to bevel.")
    } else if size <= 0. {
        Some("The radius must be positive.")
    } else if !(profile > 0. && profile < 1.) {
        Some("The profile must be between 0 and 1.")
    } else {
        None
    };
    if let Some(reason) = refusal {
        return py.import("core.i18n")?.call_method1("tr", (reason,));
    }
    let segments = segments.max(1);
    let fillet = py.import("core.fillet")?;
    let plan = fillet.call_method1("FilletPlan", (size, segments, profile, "offset", "round"))?;
    let planner = mesh.getattr("_store")?.getattr("plan_vertex_bevel")?;
    let hypot = py.import("core._native")?.call_method0("qt_hypot")?;
    let kw = PyDict::new(py);
    kw.set_item("vertices", true)?;
    fillet.getattr("_store_round_plan")?.call(
        (mesh, plan, planner, keys, size, segments, profile, hypot),
        Some(&kw),
    )
}
