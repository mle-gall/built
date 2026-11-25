// SPDX-License-Identifier: GPL-3.0-or-later
//! Geometry query and editing bindings shared by mesh views and drawing tools.
use crate::math::xyz;
use crate::store::commands::{cell, Cell};
use pyo3::prelude::*;
type Obj<'py> = Bound<'py, PyAny>;
fn cells(ring: &Obj<'_>) -> PyResult<Vec<Cell>> {
    ring.try_iter()?
        .map(|p| Ok(cell(xyz(&p?)?.to_array())))
        .collect()
}
pub(crate) fn outline_key(ring: &Obj<'_>) -> PyResult<Vec<Cell>> {
    let mut k = cells(ring)?;
    k.sort_unstable();
    k.dedup();
    Ok(k)
}
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "object_drawing")?;
    sub.add_function(wrap_pyfunction!(crate::object_planes::coplanar_plane, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_planes::has_plane_edge, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_planes::face_rebuild_plane, &sub)?)?;
    sub.add_class::<crate::draw_overlay::ScanOverlay>()?;
    sub.add_class::<crate::drawing_queries::Nearby>()?;
    sub.add_function(wrap_pyfunction!(crate::drawing_queries::touching_segments, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::drawing_queries::find_duplicate_edge, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_curves::point_on_segment, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_curves::tag_curve, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_curves::resplit_curves, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_edge_edits::order_chain, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_edge_edits::divide_edges, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_edge_edits::join_edges, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_edge_edits::collapsible, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_edge_edits::collapse_vertex, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_edge_edits::dedupe_faces, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::draw_plan::plan_segments, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::draw_plan::graph_adjacency, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::draw_plan::graph_insert, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::draw_plan::graph_remove, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::draw_plan::smallest_cycle, &sub)?)?;
    crate::kernel::object_boundary::register(&sub)?;
    crate::kernel::object_repair::register(&sub)?;
    m.add_submodule(&sub)
}
