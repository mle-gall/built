// SPDX-License-Identifier: GPL-3.0-or-later
//! Camera queries and navigation shared by the document presentation hosts.
use crate::document_host::{call_opt, none, opt, vec3, xyz, Obj};
use pyo3::prelude::*;
mod depth;
mod navigation;
mod planes;
mod projection;
mod surface;

fn xy(o: &Obj) -> PyResult<(f64, f64)> {
    Ok((
        o.call_method0("x")?.extract()?,
        o.call_method0("y")?.extract()?,
    ))
}
fn number(o: &Obj, key: &str) -> PyResult<f64> {
    o.getattr(key)?.extract()
}
fn size(v: &Obj) -> PyResult<(f64, f64)> {
    Ok((
        v.call_method0("width")?.extract()?,
        v.call_method0("height")?.extract()?,
    ))
}
pub(crate) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    planes::register(m)?;
    depth::register(m)?;
    projection::register(m)?;
    surface::register(m)?;
    navigation::register(m)
}
