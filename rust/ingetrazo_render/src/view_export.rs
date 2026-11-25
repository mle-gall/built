// SPDX-License-Identifier: GPL-3.0-or-later
//! Current-view SVG export and the cached drawing shared by preview and output.
use crate::settings_rows::{tr, trf};
use pyo3::{
    prelude::*,
    types::{IntoPyDict, PyDict, PyList, PyTuple},
};

mod active;
mod builder;
mod scopes;
mod sheet;

type Obj<'p> = Bound<'p, PyAny>;
type ResultObj<'p> = PyResult<Obj<'p>>;
const PREVIEW_WIDTH: usize = 900;
const FILE_WIDTH: usize = 3200;
const NO_FRAME: (f64, f64, f64) = (0., 0., 1.);
fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
fn render(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("ingetrazo_render")
}
fn rect(cam: &Obj) -> PyResult<(f64, f64, f64, f64)> {
    cam.py()
        .import("core.vista_caras")?
        .call_method1("view_rect", (cam,))?
        .extract()
}
/// Run a context manager with the actual exception triple so progress jobs and
/// camera/visibility changes are unwound on cancellation and failed readback.
fn scoped<'p, T>(ctx: &Obj<'p>, work: impl FnOnce() -> PyResult<T>) -> PyResult<T> {
    let py = ctx.py();
    ctx.call_method0("__enter__")?;
    let result = work();
    match &result {
        Ok(_) => ctx.call_method1("__exit__", (py.None(), py.None(), py.None()))?,
        Err(e) => ctx.call_method1("__exit__", (e.get_type(py), e.value(py), e.traceback(py)))?,
    };
    result
}
pub(crate) fn register(parent: &Bound<PyModule>) -> PyResult<()> {
    let py = parent.py();
    let name = "ingetrazo_render.view_export";
    let m = PyModule::new(py, name)?;
    m.add_class::<builder::SvgBuilder>()?;
    m.add_function(wrap_pyfunction!(scopes::widened, &m)?)?;
    m.add_function(wrap_pyfunction!(scopes::framed, &m)?)?;
    m.add_function(wrap_pyfunction!(scopes::selection_only, &m)?)?;
    m.add_function(wrap_pyfunction!(active::active_view_builder, &m)?)?;
    m.add_function(wrap_pyfunction!(active::view_line_groups, &m)?)?;
    m.add_function(wrap_pyfunction!(sheet::open_view_export, &m)?)?;
    m.add("PREVIEW_IMAGE_PX", PREVIEW_WIDTH)?;
    m.add("EXPORT_IMAGE_PX", FILE_WIDTH)?;
    m.add("NO_FRAME", NO_FRAME)?;
    parent.add("view_export", &m)?;
    py.import("sys")?.getattr("modules")?.set_item(name, m)
}
