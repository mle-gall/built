// SPDX-License-Identifier: GPL-3.0-or-later
//! Registration boundary for the modelling kernel and its scripting adapters.
pub(crate) mod object_boundary;
pub(crate) mod object_incidence;
pub(crate) mod object_repair;
pub(crate) mod push_queries;
use crate::{object_drawing, object_replay, snap, snap_engine, store};
use pyo3::prelude::*;

pub(crate) fn register_kernel(m: &Bound<'_,PyModule>) -> PyResult<()> {
    object_drawing::register(m)?;
    object_replay::register(m)?;
    push_queries::register(m)?;
    store::planar_api::register(m)?;
    m.add_function(wrap_pyfunction!(snap::inputs::edge_tag,m)?)?;
    m.add_function(wrap_pyfunction!(snap::inputs::edge_columns,m)?)?;
    m.add_function(wrap_pyfunction!(snap::inputs::point_inputs,m)?)?;
    m.add_class::<snap_engine::services::FrameState>()?;
    Ok(())
}
