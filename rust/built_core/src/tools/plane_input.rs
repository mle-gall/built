// SPDX-License-Identifier: GPL-3.0-or-later
//! Host input shared by planar machines and extension tools with PlaneLock.
use crate::math::{xyz, Vec3};
use pyo3::{prelude::*, types::PyDict};

fn anchor(tool: &Bound<'_, PyAny>) -> PyResult<()> {
    if let Some(start) = tool.getattr_opt("start_point")?.filter(|p| !p.is_none()) {
        tool.setattr(
            "work_plane",
            tool.call_method1("locked_work_plane", (start,))?,
        )?;
    }
    Ok(())
}

#[pyfunction]
pub fn plane_input(
    tool: &Bound<'_, PyAny>,
    view: &Bound<'_, PyAny>,
    action: &str,
) -> PyResult<bool> {
    let py = view.py();
    let mut label = None;
    let message = match action {
        "mark" => {
            if !tool.getattr("plane_ref")?.is_none() || !tool.getattr("plane_lock")?.is_none() {
                return Ok(false);
            }
            let mut plane = tool.getattr("work_plane")?;
            if plane.is_none() {
                plane = tool.getattr("hover_plane")?;
            }
            if plane.is_none() {
                return Ok(false);
            }
            tool.setattr(
                "plane_ref",
                (
                    Vec3::from(xyz(&plane.get_item(1)?)?.normalize_or_zero()),
                    "plane",
                ),
            )?;
            None
        }
        "reference" => {
            let message = if tool.getattr("plane_ref")?.is_none() {
                let reference = tool.call_method1("hovered_reference", (view,))?;
                if reference.is_none() {
                    return Ok(true);
                }
                let is_edge = reference.get_item(1)?.extract::<String>()? == "edge";
                tool.setattr("plane_ref", reference)?;
                tool.setattr("plane_lock", py.None())?;
                if is_edge {
                    "Drawing plane locked: perpendicular to the edge"
                } else {
                    "Drawing plane locked: parallel to the face"
                }
            } else {
                tool.setattr("plane_ref", py.None())?;
                "Drawing plane free"
            };
            anchor(tool)?;
            Some(message)
        }
        "x" | "y" | "z" => {
            tool.getattr("_plane_state")?
                .call_method1("toggle", (action,))?;
            anchor(tool)?;
            let axis = tool.getattr("plane_lock")?.extract::<Option<String>>()?;
            Some(match axis.as_deref() {
                Some(a) => {
                    label = Some(match a {
                        "x" => "YZ",
                        "y" => "XZ",
                        _ => "XY",
                    });
                    "Drawing plane locked: {plane}"
                }
                None => "Drawing plane free",
            })
        }
        _ => return Ok(false),
    };
    if let (Some(text), Some(flash)) = (
        message,
        view.getattr_opt("flash_status")?.filter(|f| !f.is_none()),
    ) {
        let args = PyDict::new(py);
        if let Some(label) = label {
            args.set_item("plane", label)?;
        }
        let translated = py
            .import("built_core.i18n")?
            .getattr("tr")?
            .call((text,), Some(&args))?;
        flash.call1((translated,))?;
    }
    if let Some(update) = view.getattr_opt("update")?.filter(|f| !f.is_none()) {
        update.call0()?;
    }
    Ok(true)
}

#[pyfunction]
pub fn cursor_plane_color(normal: &Bound<'_, PyAny>) -> PyResult<Option<(f64, f64, f64, f64)>> {
    let py = normal.py();
    let axes = super::plane::axes_of(Some(&py.import("built_core.axes")?.call_method0("frame")?))?;
    let n = xyz(normal)?.normalize_or_zero();
    for (name, axis) in [("x", axes.x), ("y", axes.y), ("z", axes.z)] {
        if n.dot(axis).abs() > 0.99 {
            let rgb = py
                .import("built_core.snap")?
                .getattr("AXIS_COLORS")?
                .get_item(name)?;
            return Ok(Some((
                rgb.get_item(0)?.extract()?,
                rgb.get_item(1)?.extract()?,
                rgb.get_item(2)?.extract()?,
                1.,
            )));
        }
    }
    Ok(None)
}
