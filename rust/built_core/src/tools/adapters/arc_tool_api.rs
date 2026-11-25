// SPDX-License-Identifier: GPL-3.0-or-later
//! ArcTool callbacks on the live document.
use super::*;

fn magnet_on<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (tool.getattr("end_point")?.is_none()).into_bound_py_any(py)
}
fn equidistant<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let tool = a.get_item(0)?;
    let e = tool.call_method1("_state", ("equidistant",))?;
    Ok(if e.is_none() {
        none(py)
    } else {
        (
            module.call_method1("Vec3", positional(&e.get_item(0)?)?)?,
            module.call_method1("Vec3", positional(&e.get_item(1)?)?)?,
            tool.getattr("_handles")?
                .call_method1("get", (e.get_item(2)?,))?,
        )
            .into_bound_py_any(py)?
    })
}
fn start_edge<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let tool = a.get_item(0)?;
    let e = tool.call_method1("_state", ("start_edge",))?;
    Ok(if e.is_none() {
        none(py)
    } else {
        (
            tool.getattr("_handles")?
                .call_method1("get", (e.get_item(0)?,))?,
            module.call_method1("Vec3", positional(&e.get_item(1)?)?)?,
        )
            .into_bound_py_any(py)?
    })
}
fn tangent_at_start<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if tool.getattr("start_point")?.is_none() {
        return Ok(none(py));
    }
    let t = module
        .call_method1("_Host", (&viewport, tool.getattr("_handles")?, &tool))?
        .call_method1(
            "arc_tangent",
            (tool.getattr("start_point")?.call_method0("toTuple")?,),
        )?;
    Ok(if t.is_none() {
        none(py)
    } else {
        module.call_method1("Vec3", positional(&t)?)?
    })
}
fn bulge_kind<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("kind",))
}
fn snap_bulge<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("snap_bulge",))
}
fn tangent_dir<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_point", ("tangent_dir",))
}
fn vcb_caption<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if tool.getattr("start_point")?.is_none() {
        return ("Segments").into_bound_py_any(py);
    }
    Ok(if !tool.getattr("_fillet")?.is_none() {
        ("Radius").into_bound_py_any(py)?
    } else {
        ("Bulge").into_bound_py_any(py)?
    })
}
fn on_double_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method1("note_plane", (ctx.getattr("viewport")?,))?;
    tool.setattr("_viewport", ctx.getattr("viewport")?)?;
    tool.call_method1(
        "_send_framed",
        (
            ctx.getattr("viewport")?,
            tool.call_method1("_pointer", ("double_click", &ctx))?,
        ),
    )?;
    tool.call_method1("_after", (ctx.getattr("viewport")?,))?;
    Ok(none(py))
}
fn on_radius_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let radius = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.setattr("_viewport", &viewport)?;
    let consumed = tool.call_method1("_send_framed", (&viewport, fields!(py; "type" => "radius", "value" => py.get_type::<pyo3::types::PyFloat>().call1((&radius,))?).into_any(),))?.get_item("consumed")?;
    tool.call_method1("_after", (&viewport,))?;
    Ok(consumed.clone())
}
fn rubber_band_lines<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let tool = a.get_item(0)?;
    module
        .getattr("RustTool")?
        .call_method1("rubber_band_lines", (&tool,))
}
fn plane_points<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if tool.getattr("start_point")?.is_none() {
        return Ok(PyList::empty(py).into_any());
    }
    Ok(PyList::new(
        py,
        [
            tool.getattr("start_point")?,
            (if !tool.getattr("end_point")?.is_none() {
                tool.getattr("end_point")?
            } else {
                tool.getattr("hover_point")?
            }),
        ],
    )?
    .into_any())
}
fn points<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let bulge = a.get_item(2)?;
    let cursor = a.get_item(1)?;
    let tool = a.get_item(0)?;
    return Ok({
        let items = PyList::empty(py);
        for item in tool
            .call_method1(
                "_query",
                (
                    "points",
                    (module.call_method1("_tuple", (&cursor,))?, &bulge).into_bound_py_any(py)?,
                ),
            )?
            .try_iter()?
        {
            let p = item?;
            items.append(module.call_method1("Vec3", positional(&p)?)?)?;
        }
        items.into_any()
    });
}
fn bulge_for<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let cursor = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method1(
        "_query",
        (
            "bulge_for",
            (module.call_method1("_tuple", (&cursor,))?,).into_bound_py_any(py)?,
        ),
    )
}
fn tangent_bulge<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_query", ("tangent_bulge",))
}

fn fillet<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let mut p: Obj<'py>;
    let tool = a.get_item(0)?;
    let f = tool.call_method1("_state", ("fillet",))?;
    return Ok(if f.is_none() {
        none(py)
    } else {
        py.get_type::<pyo3::types::PyTuple>().call1(({
            let collected = PyList::empty(py);
            for item in f.try_iter()? {
                p = item?;
                collected.append(module.call_method1("Vec3", positional(&p)?)?)?;
            }
            collected.into_any()
        },))?
    });
}
fn cue_colour<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let tool = a.get_item(0)?;
    let color_midpoint = imported(py, "core.snap")?.getattr("COLOR_MIDPOINT")?;
    let color_tangent = imported(py, "core.snap")?.getattr("COLOR_TANGENT")?;
    let cue = tool.call_method1("_state", ("cue",))?;
    let colour = fields!(py; "tangent" => &color_tangent, "fillet" => module.getattr("COLOR_FILLET")?, "half" => &color_midpoint).into_any().call_method1("get", (&cue,))?;
    Ok(if !colour.is_none() {
        {
            let values = PyList::empty(py);
            values.call_method1("extend", (&colour,))?;
            values.append(1.0)?;
            positional(values.as_any())?.into_any()
        }
    } else {
        tool.call_method0("lock_color")?
    })
}

fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let label = a.get_item(0)?.call_method1("_state", ("label",))?;
    if label.is_none() {
        return Ok(none(py));
    }
    let [kind, value, point] = unpack::<3>(&label)?;
    let tr = imported(py, "built_core.i18n")?.getattr("tr")?;
    let length = || imported(py, "core.units")?.call_method1("fmt_len", (&value,));
    let key: String = kind.extract()?;
    let text = match key.as_str() {
        "tangent_edge" => tr.call1(("Tangent to edge",))?,
        "tangent" => tr
            .call1(("Tangent at vertex",))?
            .add("  ")?
            .add(length()?)?,
        "fillet" | "half" => {
            let text = if key == "half" {
                "Half circle"
            } else {
                "Tangent to edge"
            };
            tr.call1((text,))?.add("  ")?.add(length()?)?
        }
        "bulge" => "Bulge ".into_bound_py_any(py)?.add(length()?)?,
        _ => return Err(pyo3::exceptions::PyKeyError::new_err(key)),
    };
    (
        text,
        imported(py, "tools.rust_tools")?.call_method1("Vec3", positional(&point)?)?,
    )
        .into_bound_py_any(py)
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("value_label", Mode::Method, value_label, &["self"], "", ""),
    ("magnet_on", Mode::Method, magnet_on, &["self"], "bool", ""),
    ("_fillet", Mode::Property, fillet, &["self"], "", ""),
    ("_equidistant", Mode::Property, equidistant, &["self"], "", ""),
    ("_start_edge", Mode::Property, start_edge, &["self"], "", ""),
    ("_tangent_at_start", Mode::Method, tangent_at_start, &["self", "viewport"], "", "The direction an existing arc leaves the start (``host_tangent_at``)."),
    ("_bulge_kind", Mode::Property, bulge_kind, &["self"], "", ""),
    ("_snap_bulge", Mode::Property, snap_bulge, &["self"], "", ""),
    ("_tangent_dir", Mode::Property, tangent_dir, &["self"], "", ""),
    ("_cue_colour", Mode::Method, cue_colour, &["self"], "", ""),
    ("vcb_caption", Mode::Method, vcb_caption, &["self"], "str", ""),
    ("on_double_click", Mode::Method, on_double_click, &["self", "ctx"], "None", ""),
    ("on_radius_value", Mode::Method, on_radius_value, &["self", "viewport", "radius"], "bool", "«2r»: the radius instead of the bulge."),
    ("rubber_band_lines", Mode::Method, rubber_band_lines, &["self"], "", ""),
    ("plane_points", Mode::Method, plane_points, &["self"], "", ""),
    ("_points", Mode::Method, points, &["self", "cursor", "bulge=None"], "", ""),
    ("_bulge_for", Mode::Method, bulge_for, &["self", "cursor"], "", ""),
    ("_tangent_bulge", Mode::Method, tangent_bulge, &["self"], "", ""),
];
