// SPDX-License-Identifier: GPL-3.0-or-later
//! Tool instructions and modifier clauses shown in the native status bar.
use crate::settings_rows::{s, tr, trf};
use pyo3::{prelude::*, types::PyDict};

type Phases = &'static [(&'static str, &'static str)];
const TOOLS: &[(&str, Phases)] = &[
    ("select", &[
        ("idle", "Shift = add/remove, Ctrl = add. Double-click a group to edit it. F3 = search commands."),
    ]),
    ("change_axes", &[
        ("idle", "Click the new origin."),
        ("started", "Click the red direction, then the green one."),
    ]),
    ("scale_manager", &[
        ("idle", "Type the X, Y, Z scale in the window; Enter or leaving the field applies it."),
    ]),
    ("outer_shell", &[
        ("idle", "Click a solid group or component (1)."),
        ("started", "Click another solid (2) — and more to add them."),
    ]),
    ("solid_union", &[
        ("idle", "Click a solid group or component (1)."),
        ("started", "Click another solid (2) — and more to add them."),
    ]),
    ("solid_subtract", &[
        ("idle", "Click the solid that cuts (1)."),
        ("started", "Click the solid to cut (2). The first one goes away."),
    ]),
    ("solid_trim", &[
        ("idle", "Click the solid that cuts (1)."),
        ("started", "Click the solid to cut (2). The first one stays."),
    ]),
    ("solid_intersect", &[
        ("idle", "Click a solid group or component (1)."),
        ("started", "Click another solid (2): only what they share stays."),
    ]),
    ("solid_split", &[
        ("idle", "Click a solid group or component (1)."),
        ("started", "Click another solid (2): three pieces come out."),
    ]),
    ("eraser", &[
        ("idle", "Drag over edges to erase them. Shift = hide instead."),
    ]),
    ("paint", &[
        ("idle", "Click a face to paint it. Alt = sample the material under the cursor."),
    ]),
    ("line", &[
        ("idle", "Click the start point. Arrows lock an axis, Shift locks the inference."),
        ("started", "Click the end point, or type the length and Enter."),
    ]),
    ("freehand", &[
        ("idle", "Press and drag to draw a freehand line."),
    ]),
    ("rectangle", &[
        ("idle", "Click the first corner. Arrows pick the plane."),
        ("started", "Click the opposite corner, or type width;height and Enter."),
    ]),
    ("rotated_rect", &[
        ("idle", "Click the first corner of the base edge."),
        ("started", "Edge end, then width (or type them). Shift holds the direction."),
    ]),
    ("circle", &[
        ("idle", "Click the centre. Arrows pick the plane, Down = perpendicular to an edge. Type the sides first."),
        ("started", "Click the radius, or type it and Enter."),
    ]),
    ("polygon", &[
        ("idle", "Click the centre. Arrows pick the plane, Down = perpendicular to an edge. Type the sides first."),
        ("started", "Click the radius, or type it and Enter."),
    ]),
    ("arc", &[
        ("idle", "Click the start point — on an edge, a tangent arc. Type the segments first."),
        ("end", "End point. Magenta = same distance; double-click rounds."),
        ("bulge", "Click the bulge, or type it. 'Nr' = radius, 'Ns' = segments. Alt keeps the corner."),
    ]),
    ("arc3", &[
        ("idle", "Click the start point. Type the segments first."),
        ("started", "Click a point the arc passes through, then the end."),
    ]),
    ("center_arc", &[
        ("idle", "Click the centre. Type the segments first."),
        ("started", "Click the start of the arc, then its end; or type the angle."),
    ]),
    ("pie", &[
        ("idle", "Click the centre. Type the segments first."),
        ("started", "Click the start of the wedge, then its end; or type the angle."),
    ]),
    ("pushpull", &[
        ("idle", "Click a face and move."),
        ("started", "Move, or type the distance and Enter. Double-click repeats the last."),
    ]),
    ("move", &[
        ("idle", "Click what to move (or select it first). Red + on a group = rotate it."),
        ("started", "Click the destination, or type the distance and Enter."),
    ]),
    ("rotate", &[
        ("idle", "Click the centre of rotation — on a face, the protractor takes its plane."),
        ("started", "Click the start of the angle."),
        ("angle", "Click the end, or type degrees and Enter."),
    ]),
    ("scale", &[
        ("idle", "Select something, then drag a grip: corners uniform, edges along an axis."),
        ("started", "Drag, or type a factor (2) or a size (2m). Ctrl = centre, Shift = uniform."),
    ]),
    ("flip", &[
        ("idle", "Click the plane to mirror the selection across."),
    ]),
    ("followme", &[
        ("idle", "Select the path, then click the profile face. Alt = sweep along the selection."),
        ("started", "Move along the path; click to finish."),
    ]),
    ("offset", &[
        ("idle", "Click a face, or connected edges, to offset."),
        ("started", "Move to set the offset, or type it and Enter."),
    ]),
    ("fillet", &[
        ("idle", "Click an edge (or select edges first); type the size and Enter."),
        ("sizing", "Move to set the size, or type it and Enter; click to apply. 'Ns' = segments, '0.3p' = profile."),
    ]),
    ("tape", &[
        ("idle", "Click a point or an edge to measure. Arrows lock an axis."),
        ("started", "Click the second point, or type the distance and Enter."),
    ]),
    ("protractor", &[
        ("idle", "Click the vertex of the angle."),
        ("started", "Click the start of the angle, then the end; or type the degrees."),
    ]),
    ("north", &[
        ("idle", "Click the centre of the compass."),
        ("started", "Click toward north, or type the degrees from the green axis."),
    ]),
    ("dimension", &[
        ("idle", "Click the first point of the dimension."),
        ("started", "Click the 2nd point, then place it; past an end = linear."),
    ]),
    ("text", &[
        ("idle", "Click a point for a label, or empty space for a screen note."),
        ("started", "Click where the label goes; the text is asked for next."),
    ]),
    ("geopath", &[
        ("idle", "Click the points of the path; Enter or double-click finishes."),
    ]),
    ("section", &[
        ("idle", "Click a face to place the section plane. Shift keeps the orientation."),
    ]),
    ("texture_position", &[
        ("idle", "Drag a pin: red moves, green scales/rotates, blue shears. Enter finishes."),
    ]),
    ("image", &[
        ("idle", "Click the first corner of the image, then the opposite one. Shift frees the aspect."),
    ]),
    ("paste", &[
        ("idle", "Click where the copy goes. Arrows lock an axis."),
    ]),
    ("place_group", &[
        ("idle", "Click where the component goes."),
    ]),
    ("position_camera", &[
        ("idle", "Click where to stand, or drag toward what to look at. Type the eye height."),
    ]),
    ("look_around", &[
        ("idle", "Drag to turn the camera. Type the eye height."),
    ]),
    ("walk", &[
        ("idle", "Click and drag to walk. Ctrl = run, Shift = up/down or sideways, Alt = through walls."),
    ]),
    ("first_person", &[
        ("idle", "W/A/S/D = walk, Q/E = down/up, drag = look. Shift = run, Alt = through walls."),
    ]),
];
const NAVIGATION: &[(&str, &str)] = &[
    (
        "orbit",
        "Drag to orbit. Shift = pan. Wheel = zoom. The middle button orbits from any tool.",
    ),
    ("pan", "Drag to pan. Wheel = zoom."),
    (
        "zoom",
        "Drag up or down to zoom; the wheel zooms at the cursor. Type 35mm or 60deg for the lens.",
    ),
    ("zoom_window", "Drag a box to zoom into it."),
];

fn truth(tool: &Bound<PyAny>, field: &str, default: bool) -> PyResult<bool> {
    tool.getattr_opt(field)?
        .map_or(Ok(default), |v| v.is_truthy())
}

#[pyfunction]
fn phase_of(key: &str, tool: &Bound<PyAny>) -> PyResult<&'static str> {
    let started =
        |attr| -> PyResult<bool> { Ok(tool.getattr_opt(attr)?.is_some_and(|p| !p.is_none())) };
    Ok(match key {
        "rotate" if started("ref_point")? => "angle",
        "arc" if started("end_point")? => "bulge",
        "arc" if started("start_point")? => "end",
        "arc" => "idle",
        "fillet" if truth(tool, "sizing", false)? => "sizing",
        "fillet" => "idle",
        _ if started("start_point")?
            || truth(tool, "dragging", false)?
            || truth(tool, "nodes", false)? =>
        {
            "started"
        }
        _ => "idle",
    })
}
#[pyfunction]
fn alt_inference_hint(py: Python, mode: Option<&str>) -> PyResult<String> {
    let state = match mode.unwrap_or("all") {
        "all" | "" => "all",
        "off" => "none",
        "parallel_perp" => "parallel / perpendicular",
        _ => return Ok(String::new()),
    };
    trf(
        py,
        "Alt = inferences ({state})",
        &[("state", s(py, &tr(py, state)?))],
    )
}

#[pyfunction]
#[pyo3(signature=(key, tool, nav_mode=None, linear_mode=None))]
pub(crate) fn hint_for(
    py: Python,
    key: Option<&str>,
    tool: &Bound<PyAny>,
    nav_mode: Option<&str>,
    linear_mode: Option<&str>,
) -> PyResult<String> {
    if let Some((_, hint)) = NAVIGATION.iter().find(|(k, _)| Some(*k) == nav_mode) {
        return tr(py, hint);
    }
    let key = key.unwrap_or_default();
    let Some((_, phases)) = TOOLS.iter().find(|(k, _)| *k == key) else {
        return match tool.getattr_opt("name")?.filter(|v| !v.is_none()) {
            Some(v) if v.is_truthy()? => trf(
                py,
                "{name}: Esc cancels.",
                &[("name", s(py, &tr(py, &v.extract::<String>()?)?))],
            ),
            _ => Ok(String::new()),
        };
    };
    let phase = phase_of(key, tool)?;
    let instruction = phases
        .iter()
        .find(|(k, _)| *k == phase)
        .unwrap_or(&phases[0])
        .1;
    let text = tr(py, instruction)?;
    let clause = if phase == "idle" && matches!(key, "rectangle" | "pushpull" | "move") {
        String::new()
    } else if phase == "idle" {
        match tool
            .getattr_opt("status_clause")?
            .filter(|v| v.is_callable())
        {
            Some(f) => f.call0()?.extract::<Option<String>>()?.unwrap_or_default(),
            None => String::new(),
        }
    } else if linear_mode.is_some()
        && truth(tool, "uses_snap", false)?
        && truth(tool, "allow_inferences", true)?
    {
        alt_inference_hint(py, linear_mode)?
    } else {
        String::new()
    };
    Ok(if clause.is_empty() {
        text
    } else {
        format!("{text} | {clause}")
    })
}

pub(crate) fn register(parent: &Bound<PyModule>) -> PyResult<()> {
    let py = parent.py();
    let m = PyModule::new(py, "built_render.status_hints")?;
    let hints = PyDict::new(py);
    for &(tool, phases) in TOOLS {
        let entries = PyDict::new(py);
        for &(phase, text) in phases {
            entries.set_item(phase, text)?;
        }
        hints.set_item(tool, entries)?;
    }
    let navigation = PyDict::new(py);
    for &(key, hint) in NAVIGATION {
        navigation.set_item(key, hint)?;
    }
    m.add("HINTS", hints)?;
    m.add("NAV_HINTS", navigation)?;
    m.add_function(wrap_pyfunction!(hint_for, &m)?)?;
    m.add_function(wrap_pyfunction!(phase_of, &m)?)?;
    m.add_function(wrap_pyfunction!(alt_inference_hint, &m)?)?;
    py.import("sys")?
        .getattr("modules")?
        .set_item(m.name()?, &m)?;
    parent.add("status_hints", m)
}
