// SPDX-License-Identifier: GPL-3.0-or-later
//! PlaneLock callbacks on the live document.
use super::*;

fn plane_state<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let mut state = tool
        .getattr("__dict__")?
        .call_method1("get", ("_plane_state_rs",))?;
    if state.is_none() {
        let _state = py.import("ingetrazo_core")?.getattr("PlaneLock")?;
        {
            let value = _state.call0()?;
            state = value.clone();
            tool.getattr("__dict__")?
                .set_item("_plane_state_rs", value.clone())?;
        }
    }
    Ok(state.clone())
}
fn plane_lock<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_plane_state")?.getattr("plane_lock")
}
fn set_plane_lock<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let axis = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.getattr("_plane_state")?.setattr("plane_lock", &axis)?;
    Ok(none(py))
}
fn plane_ref<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_plane_state")?.getattr("plane_ref")
}
fn set_plane_ref<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let reference = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.getattr("_plane_state")?
        .setattr("plane_ref", &reference)?;
    Ok(none(py))
}
fn hover_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_plane_state")?.getattr("hover_plane")
}
fn set_hover_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let plane = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.getattr("_plane_state")?
        .setattr("hover_plane", &plane)?;
    Ok(none(py))
}
fn note_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let plane = optional(&viewport, "_last_work_plane")?;
    if !plane.is_none() {
        tool.setattr("hover_plane", &plane)?;
    }
    Ok(none(py))
}
fn drawing_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    let plane = tool.call_method0("_own_plane")?;
    let snapped = tool.call_method1("snapped_plane", (&plane,))?;
    Ok(if !snapped.is_none() {
        snapped.clone()
    } else {
        plane.clone()
    })
}
fn own_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let axes = py.import("core.axes")?.into_any();
    tool.getattr("_plane_state")?.call_method1(
        "own_plane",
        (tool.getattr("work_plane")?, axes.call_method0("frame")?),
    )
}
fn plane_points<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    Ok(PyList::empty(py).into_any())
}
fn snapped_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.base")?;
    let plane = a.get_item(1)?;
    let tool = a.get_item(0)?;
    let axes = py.import("core.axes")?.into_any();
    let names_point = if tool.getattr("snap_kind")?.is_none() {
        none(py)
    } else {
        (module
            .getattr("POINT_SNAP_KINDS")?
            .contains(tool.getattr("snap_kind")?)?)
        .into_bound_py_any(py)?
    };
    tool.getattr("_plane_state")?.call_method1(
        "snapped",
        (
            &plane,
            tool.call_method0("plane_points")?,
            &names_point,
            axes.call_method0("frame")?,
        ),
    )
}
fn adopt_snapped_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let snapped = tool.call_method1("snapped_plane", (tool.call_method0("_own_plane")?,))?;
    if !snapped.is_none() {
        tool.setattr("work_plane", &snapped)?;
    }
    Ok(none(py))
}
fn preview_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.base")?;
    let point = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let locked = tool.call_method1("locked_work_plane", (&point,))?;
    if !locked.is_none() {
        return Ok(locked.clone());
    }
    let plane = optional(&viewport, "_last_work_plane")?;
    let axes = py.import("core.axes")?.into_any();
    if !plane.is_none() {
        return (
            module.call_method1("QVector3D", (&point,))?,
            plane.get_item(1)?,
        )
            .into_bound_py_any(py);
    }
    (
        module.call_method1("QVector3D", (&point,))?,
        axes.call_method1("axis", ("z",))?,
    )
        .into_bound_py_any(py)
}
fn plane_lock_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.base")?;
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if !optional(&tool, "start_point")?.is_none()
        && !(tool
            .getattr("plane_lock_after_the_first_point")?
            .is_truthy()?)
    {
        return false.into_bound_py_any(py);
    }
    if py.get_type::<pyo3::types::PyInt>().call1((&key,))?.eq(py
        .get_type::<pyo3::types::PyInt>()
        .call1((module.getattr("Qt")?.getattr("Key_Down")?,))?)?
    {
        return tool.call_method1("reference_lock_key", (&viewport,));
    }
    let axis = module.getattr("PLANE_LOCK_KEYS")?.call_method1(
        "get",
        (py.get_type::<pyo3::types::PyInt>().call1((&key,))?,),
    )?;
    if axis.is_none() {
        return false.into_bound_py_any(py);
    }
    module.call_method1("plane_input", (&tool, &viewport, &axis))
}
fn reference_lock_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.base")?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    module.call_method1("plane_input", (&tool, &viewport, "reference"))
}
fn mark_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.base")?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    module.call_method1("plane_input", (&tool, &viewport, "mark"))
}
fn clear_plane_lock<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    tool.getattr("_plane_state")?.call_method0("clear")?;
    Ok(none(py))
}
fn locked_work_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let point = a.get_item(1)?;
    let tool = a.get_item(0)?;
    let axes = py.import("core.axes")?.into_any();
    tool.getattr("_plane_state")?
        .call_method1("locked", (&point, axes.call_method0("frame")?))
}

fn lock_color<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if !tool.getattr("plane_ref")?.is_none() {
        let color_reference = py.import("core.snap")?.getattr("COLOR_REFERENCE")?;
        return Ok({
            let values = PyList::empty(py);
            values.call_method1("extend", (&color_reference,))?;
            values.append(1.0)?;
            positional(values.as_any())?.into_any()
        });
    }
    if !tool.getattr("plane_lock")?.is_none() {
        let plane_lock_axes = py.import("tools.base")?.getattr("PLANE_LOCK_AXES")?;
        return Ok(tool.call_method1(
            "plane_color",
            (plane_lock_axes.get_item(tool.getattr("plane_lock")?)?,),
        )?);
    }
    if !tool.getattr("hover_plane")?.is_none() {
        return Ok(tool.call_method1("plane_color", (tool.getattr("hover_plane")?.get_item(1)?,))?);
    }
    Ok(none(py))
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("_plane_state", Mode::Property, plane_state, &["self"], "", ""),
    ("plane_lock", Mode::Property, plane_lock, &["self"], "", ""),
    ("plane_ref", Mode::Property, plane_ref, &["self"], "", ""),
    ("hover_plane", Mode::Property, hover_plane, &["self"], "", ""),
    ("note_plane", Mode::Method, note_plane, &["self", "viewport"], "None", "Remember the plane the viewport just hit — call on every hover\nand click before using the point."),
    ("drawing_plane", Mode::Method, drawing_plane, &["self"], "", "``(point, normal)`` the shape is laid out on: the captured or\nlocked plane — unless a snapped point says otherwise (see\n``snapped_plane``) — else the plane of the last hit, else the\nground."),
    ("_own_plane", Mode::Method, own_plane, &["self"], "", "The captured plane, else the last hit's, else the context's ground."),
    ("plane_points", Mode::Method, plane_points, &["self"], "list", "The points the shape must pass through, first click first — the\nclicked ones and the one under the cursor. A tool opts in to\n``snapped_plane`` by listing them; the default opts out."),
    ("snapped_plane", Mode::Method, snapped_plane, &["self", "plane"], "", "The plane the shape's points ask for when a SNAPPED one lies off\n``plane``, or ``None`` to keep ``plane``.\n\nFilling a window opening (2026-09-29): first point on the midpoint\nof one jamb's depth edge, the next on the midpoint of the other.\nThe first click captured the jamb face, where those two points\nshare a vertical line: a rectangle «0.00 × 2.41 m», a circle whose\nrim missed the point, arcs that ended on the wrong jamb. Both lie on\nthe wall's middle plane, and that is the plane they get:\n\n- two points: the axis plane that holds both, when exactly one\n  does (the span has no component along its normal);\n- three points not in a line: the plane through them.\n\nAn arrow-key lock was asked for explicitly and never yields. A\ncursor whose snap kind positively names a face (not a point of the\nmodel, ``core.snap.POINT_SNAP_KINDS``) keeps that face's plane\n(#545); no kind known keeps the older rule: the points decide.\nThe rule is Rust's (``PlaneLock.snapped``)."),
    ("adopt_snapped_plane", Mode::Method, adopt_snapped_plane, &["self"], "None", "Make the snapped plane the captured one — call when a point that\nis NOT the last is clicked, so the cursor rays for the next point\nland on the new plane instead of the old one."),
    ("preview_plane", Mode::Method, preview_plane, &["self", "viewport", "point:QVector3D"], "", "The plane the shape WOULD take at ``point`` before the first\nclick: the arrow-key lock, else the face under the cursor, else\nthe view's — vertical facing the camera near the horizon, flat\notherwise. The classic tool shows this plane on the cursor (a coloured\nsquare / ring) so a lock is visible before you commit to it."),
    ("lock_color", Mode::Method, lock_color, &["self"], "", "The rubber band colour for a locked or camera-selected plane."),
    ("plane_lock_key", Mode::Method, plane_lock_key, &["self", "viewport", "key:int"], "bool", ""),
    ("reference_lock_key", Mode::Method, reference_lock_key, &["self", "viewport"], "bool", ""),
    ("mark_plane", Mode::Method, mark_plane, &["self", "viewport"], "bool", ""),
    ("clear_plane_lock", Mode::Method, clear_plane_lock, &["self"], "None", "Both locks off — the shape spent them, or Esc / deactivation. A\nheld Shift's mark (``plane_ref``) goes with them; its own release\nthen has nothing left to do."),
    ("locked_work_plane", Mode::Method, locked_work_plane, &["self", "point:QVector3D"], "", "``(point, normal)`` of the locked plane through *point* — the\narrow-key axis plane or the Down-arrow reference — or ``None``\nwithout a lock."),
    ("plane_lock", Mode::Setter, set_plane_lock, &["self", "axis"], "", ""),
    ("plane_ref", Mode::Setter, set_plane_ref, &["self", "ref"], "", ""),
    ("hover_plane", Mode::Setter, set_hover_plane, &["self", "plane"], "", ""),
];
