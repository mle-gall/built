// SPDX-License-Identifier: GPL-3.0-or-later
//! Geometry operations on detached plugin elements.
use pyo3::prelude::*;

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "object_replay")?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::vertex_faces, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::face_vertices, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::face_holes, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::reverse_rings, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::insert_between, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::collapse_collinear, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::curve_edges, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::dominant_attrs, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::mesh_key, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::ring_normal, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::orient_union, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::loop_area, &sub)?)?;
    m.add_submodule(&sub)
}
