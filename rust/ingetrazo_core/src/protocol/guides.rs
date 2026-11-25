// SPDX-License-Identifier: GPL-3.0-or-later
//! Guide, clipboard and axis tools in the current mesh frame.
use super::annotations::vec3;
use super::placements::one;
use super::transforms::{finite, vector};
use super::*;
fn local<'py>(history: &Obj<'py>, point: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = history.py();
    let mesh = history.getattr("scene")?.getattr("mesh")?;
    let st = mesh
        .getattr("__dict__")?
        .cast::<PyDict>()?
        .get_item("_store")?;
    if let Some(s) = st.filter(|s| !s.is_none()) {
        py.import("ingetrazo_core.coordinate_frame")?
            .call_method1("point_in_frame", (point, s.getattr("frame_origin")?))
    } else {
        vec3(point)
    }
}
#[pyfunction]
fn set_group_axes<'py>(
    history: &Obj<'py>,
    placement: &Obj<'py>,
    origin: &Obj<'py>,
    red: &Obj<'py>,
    green: &Obj<'py>,
) -> Reply<'py> {
    let py = history.py();
    let group = one(history, placement)?;
    let basis = py
        .import("ingetrazo_core")?
        .getattr("ToolMachine")?
        .call_method1("group_axes_basis", (origin, red, green))?;
    if basis.is_none() {
        return Err(PyValueError::new_err(
            "the red and green points must define a nondegenerate frame",
        ));
    }
    let frame = py.import("core.axes")?.call_method1(
        "frame_matrix",
        (
            local(history, origin)?,
            vec3(&basis.get_item(0)?)?,
            vec3(&basis.get_item(1)?)?,
            vec3(&basis.get_item(2)?)?,
        ),
    )?;
    execute(
        history,
        &history_command(py, "ChangeAxesCommand", (group, frame))?,
    )
}
#[pyfunction]
#[pyo3(signature=(history,offset,*,clipboard))]
fn paste_clipboard<'py>(history: &Obj<'py>, offset: &Obj<'py>, clipboard: &Obj<'py>) -> Reply<'py> {
    let d = finite(offset, "paste offset must be a finite vector")?;
    let before = history.getattr("revision")?;
    let ok = clipboard.is_truthy()? && super::paste::stamp(history, clipboard, d)?;
    answer(history, &before, ok)
}
#[pyfunction]
#[pyo3(signature=(history,point,direction=None,origin=None))]
fn add_guide<'py>(
    history: &Obj<'py>,
    point: &Obj<'py>,
    direction: Option<&Obj<'py>>,
    origin: Option<&Obj<'py>>,
) -> Reply<'py> {
    let py = history.py();
    let g = py.import("core.guide")?.call_method1(
        "Guide",
        (
            local(history, point)?,
            direction.map(vec3).transpose()?,
            origin.map(|v| local(history, v)).transpose()?,
        ),
    )?;
    if g.call_method0("is_finite")?.is_truthy()?
        && (direction.is_none() || g.getattr("is_line")?.is_truthy()?)
    {
        execute(history, &history_command(py, "AddGuideCommand", (g,))?)
    } else {
        answer(history, &history.getattr("revision")?, false)
    }
}
#[pyfunction]
fn change_guide<'py>(history: &Obj<'py>, index: &Obj<'py>, direction: &Obj<'py>) -> Reply<'py> {
    let py = history.py();
    let items = history.getattr("scene")?.getattr("guides")?;
    let len = items.len()?;
    let i = index
        .extract::<usize>()
        .ok()
        .filter(|i| *i < len && index.is_exact_instance_of::<pyo3::types::PyInt>())
        .ok_or_else(|| PyValueError::new_err("unknown guide index"))?;
    let g = items.get_item(i)?;
    if !g.is_instance(&py.import("core.guide")?.getattr("Guide")?)?
        || !g.getattr("is_line")?.is_truthy()?
    {
        return Err(PyValueError::new_err("this guide is not a line"));
    }
    let err = "guide direction must be a finite nonzero vector";
    let d = finite(direction, err)?;
    if d.length() < 1e-12 {
        return Err(PyValueError::new_err(err));
    }
    execute(
        history,
        &history_command(py, "ChangeGuideCommand", (g, vector(py, d)?))?,
    )
}
#[pyfunction]
fn add_guide_circle<'py>(
    history: &Obj<'py>,
    center: &Obj<'py>,
    normal: &Obj<'py>,
    radius: &Obj<'py>,
) -> Reply<'py> {
    let py = history.py();
    let radius = py
        .get_type::<pyo3::types::PyFloat>()
        .call1((radius,))?
        .extract::<f64>()?;
    if radius >= 1e-9 {
        let g = py.import("core.guide")?.call_method1(
            "GuideCircle",
            (local(history, center)?, vec3(normal)?, radius),
        )?;
        if g.call_method0("is_finite")?.is_truthy()? {
            return execute(history, &history_command(py, "AddGuideCommand", (g,))?);
        }
    }
    answer(history, &history.getattr("revision")?, false)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(set_group_axes, m)?)?;
    m.add_function(wrap_pyfunction!(paste_clipboard, m)?)?;
    m.add_function(wrap_pyfunction!(add_guide, m)?)?;
    m.add_function(wrap_pyfunction!(change_guide, m)?)?;
    m.add_function(wrap_pyfunction!(add_guide_circle, m)?)?;
    Ok(())
}
