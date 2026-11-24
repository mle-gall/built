// SPDX-License-Identifier: GPL-3.0-or-later
//! EraserHost callbacks on the live document.
use super::*;

fn mark<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.eraser_host")?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let x = a.get_item(2)?;
    let y = a.get_item(3)?;
    tool.call_method1("_send", (&vp, fields!(py; "type" => "click", "px" => (&x, &y,).into_bound_py_any(py)?, "mods" => if tool.getattr("_hide")?.is_truthy()? { module.getattr("Qt")?.getattr("ShiftModifier")?.getattr("value")? } else { 0.into_bound_py_any(py)? }).into_any(),))?;
    Ok(none(py))
}
fn on_release<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    tool.call_method1("_send", (&vp, fields!(py; "type" => "release").into_any()))?;
    vp.call_method0("update")?;
    Ok(none(py))
}

fn send<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.eraser_host")?;
    let event = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    tool.setattr("_viewport", &vp)?;
    let reply = imported(py, "builtins")?
        .getattr("super")?
        .call1((module.getattr("EraserHost")?, &tool))?
        .call_method1("_send", (&vp, &event))?;
    if ("activate", "deactivate", "cancel", "release")
        .into_bound_py_any(py)?
        .contains(event.get_item("type")?)?
        || (event.get_item("type")?.eq("hover")? && !(tool.getattr("_stroke")?.is_truthy()?))
    {
        tool.getattr("_handles")?.call_method0("clear")?;
    }
    Ok(reply.clone())
}
fn rubber_band_lines<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.eraser_host")?;
    let mut i: Obj<'py>;
    let mut m: Obj<'py>;
    let mut pts: Obj<'py>;
    let tool = a.get_item(0)?;
    let segs = PyList::empty(py).into_any();
    for item in tool.getattr("marked")?.try_iter()? {
        m = item?;
        if m.is_instance(&module.getattr("Edge")?)? {
            segs.call_method1(
                "append",
                ((m.getattr("a")?, m.getattr("b")?).into_bound_py_any(py)?,),
            )?;
        } else {
            if m.is_instance(&module.getattr("Group")?)? {
                segs.call_method1(
                    "extend",
                    (tool.call_method1("_group_box_segments", (&m,))?,),
                )?;
            } else {
                if m.is_instance(&module.getattr("Guide")?)? {
                    segs.call_method1("append", (m.call_method0("segment")?,))?;
                } else {
                    if m.getattr_opt("is_circle")?
                        .unwrap_or(false.into_bound_py_any(py)?)
                        .is_truthy()?
                    {
                        pts = m.call_method1("sample_points", (24,))?;
                        segs.call_method1(
                            "extend",
                            ({
                                let collected = PyList::empty(py);
                                for item in imported(py, "builtins")?
                                    .getattr("range")?
                                    .call1((pts.len()?.into_bound_py_any(py)?,))?
                                    .try_iter()?
                                {
                                    i = item?;
                                    collected.append(
                                        (
                                            pts.get_item(&i)?,
                                            pts.get_item(
                                                i.add(1)?.rem(pts.len()?.into_bound_py_any(py)?)?,
                                            )?,
                                        )
                                            .into_bound_py_any(py)?,
                                    )?;
                                }
                                collected.into_any()
                            },),
                        )?;
                    } else {
                        if m.is_instance(&module.getattr("Dimension")?)? {
                            segs.call_method1("append", (m.call_method0("line_points")?,))?;
                        } else {
                            if m.is_instance(&module.getattr("GeoPath")?)? {
                                segs.call_method1("extend", (m.call_method0("segments")?,))?;
                            } else if m.is_instance(&module.getattr("TextLabel")?)? {
                                segs.call_method1(
                                    "append",
                                    ((m.getattr("anchor")?, m.call_method0("position")?)
                                        .into_bound_py_any(py)?,),
                                )?;
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(segs.clone())
}

fn group_box_segments<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let [tool, group] = unpack::<2>(a.as_any())?;
    let viewport = optional(&tool, "_viewport")?;
    let output = PyList::empty(py);
    if let Some(obb) = viewport.getattr_opt("_group_obb")?.filter(|v| !v.is_none()) {
        let corners = imported(py, "core.group")?.getattr("oriented_box_corners")?;
        if let Ok(c) = obb
            .call1((group,))
            .and_then(|b| corners.call1(positional(&b)?))
        {
            if c.len()? == 8 {
                for (i, j) in [
                    (0, 1),
                    (1, 3),
                    (3, 2),
                    (2, 0),
                    (4, 5),
                    (5, 7),
                    (7, 6),
                    (6, 4),
                    (0, 4),
                    (1, 5),
                    (2, 6),
                    (3, 7),
                ] {
                    output.append((c.get_item(i)?, c.get_item(j)?))?;
                }
            }
        }
    }
    Ok(output.into_any())
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("_mark", Mode::Method, mark, &["self", "vp", "x", "y"], "", ""),
    ("_send", Mode::Method, send, &["self", "vp", "event"], "", ""),
    ("on_release", Mode::Method, on_release, &["self", "vp"], "", ""),
    ("rubber_band_lines", Mode::Method, rubber_band_lines, &["self"], "", "The marked stroke, drawn in the tool's red wireframe colour."),
    ("_group_box_segments", Mode::Method, group_box_segments, &["self", "group"], "list", "The twelve edges of the group's oriented box — its stroke mark,\nthe same box Select draws around it."),
];
