// SPDX-License-Identifier: GPL-3.0-or-later
//! GeoPathTool callbacks on the live document.
use super::*;

fn host_geo_plan<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let cam: Obj<'py>;
    let enter = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let tr = imported(py, "ingetrazo_core.i18n")?.getattr("tr")?;
    if enter.is_truthy()? && vp.call_method0("has_ground_surface")?.is_truthy()? {
        cam = vp.getattr("camera")?;
        tool.setattr(
            "_saved_view",
            (
                cam.getattr("perspective")?,
                cam.getattr("yaw")?,
                cam.getattr("pitch")?,
            )
                .into_bound_py_any(py)?,
        )?;
        cam.setattr("perspective", false)?;
        cam.call_method1("set_view", ("top",))?;
        vp.call_method1("flash_status", (tr.call1(("Top view, parallel — so the trace lands where you click. Restored when you leave the tool.",))?,))?;
        vp.call_method0("update")?;
    } else {
        if !(enter.is_truthy()?) && !tool.getattr("_saved_view")?.is_none() {
            {
                let [part0, part1, part2] = unpack::<3>(&tool.getattr("_saved_view")?)?;
                vp.getattr("camera")?.setattr("perspective", part0)?;
                vp.getattr("camera")?.setattr("yaw", part1)?;
                vp.getattr("camera")?.setattr("pitch", part2)?;
            }
            tool.setattr("_saved_view", none(py))?;
            vp.call_method0("update")?;
        }
    }
    Ok(none(py))
}
fn on_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let qt = imported(py, "ingetrazo_core")?.getattr("Input")?;
    if !(qt.getattr("Key_Enter")?, qt.getattr("Key_Return")?)
        .into_bound_py_any(py)?
        .contains(&key)?
    {
        return false.into_bound_py_any(py);
    }
    tool.call_method1(
        "_send",
        (
            &vp,
            fields!(py; "type" => "key", "key" => "Enter").into_any(),
        ),
    )?;
    vp.call_method0("update")?;
    true.into_bound_py_any(py)
}
fn on_double_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method1(
        "_send",
        (
            ctx.getattr("viewport")?,
            tool.call_method1("_pointer", ("double_click", &ctx))?,
        ),
    )?;
    ctx.getattr("viewport")?.call_method0("update")?;
    Ok(none(py))
}

fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.geopath_host")?;
    let at: Obj<'py>;
    let elevation: Obj<'py>;
    let grade: Obj<'py>;
    let rise: Obj<'py>;
    let run: Obj<'py>;
    let tool = a.get_item(0)?;
    let mut v: Obj<'py>;
    let fmt_len = imported(py, "core.units")?.getattr("fmt_len")?;
    let label = tool.call_method1("_state", ("label",))?;
    if label.is_none() {
        return Ok(none(py));
    }
    {
        let [part0, part1, part2, part3, part4] = unpack::<5>(&label)?;
        at = part0;
        run = part1;
        elevation = part2;
        rise = part3;
        grade = part4;
    }
    let parts = {
        let items = PyList::empty(py);
        for item in (&run, &elevation).into_bound_py_any(py)?.try_iter()? {
            v = item?;
            if !(!v.is_none()) {
                continue;
            }
            items.append(fmt_len.call1((&v,))?)?;
        }
        items.into_any()
    };
    if !rise.is_none() {
        parts.call_method1(
            "append",
            ([
                (if rise.ge(0)? {
                    ("+").into_bound_py_any(py)?
                } else {
                    ("").into_bound_py_any(py)?
                })
                .call_method1("__format__", ("",))?
                .extract::<String>()?,
                fmt_len
                    .call1((&rise,))?
                    .call_method1("__format__", ("",))?
                    .extract::<String>()?,
                " (".to_owned(),
                grade
                    .call_method1("__format__", ("+.1f",))?
                    .extract::<String>()?,
                "%)".to_owned(),
            ]
            .concat()
            .into_bound_py_any(py)?,),
        )?;
    }
    Ok(if parts.is_truthy()? {
        (
            ("  ·  ")
                .into_bound_py_any(py)?
                .call_method1("join", (&parts,))?,
            module.call_method1("Vec3", positional(&at)?)?,
        )
            .into_bound_py_any(py)?
    } else {
        none(py)
    })
}

fn host_geo_preview<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let [tool, vp, args] = unpack::<3>(a.as_any())?;
    let [key, index, point] = unpack::<3>(&args)?;
    let path = tool.getattr("_handles")?.call_method1("get", (key,))?;
    let scene = vp.getattr("scene")?;
    if scene.getattr("geo_paths")?.contains(&path)? {
        let points = path.getattr("points")?;
        if index.lt(points.len()?)? {
            points.set_item(
                &index,
                imported(py, "tools.geopath_host")?
                    .getattr("Vec3")?
                    .call1(positional(&point)?)?,
            )?;
            scene.setattr("version", scene.getattr("version")?.add(1)?)?;
        }
    }
    Ok(none(py))
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("host_geo_plan", Mode::Method, host_geo_plan, &["self", "vp", "enter"], "", ""),
    ("host_geo_preview", Mode::Method, host_geo_preview, &["self", "vp", "args"], "", ""),
    ("on_key", Mode::Method, on_key, &["self", "vp", "key", "modifiers"], "", ""),
    ("on_double_click", Mode::Method, on_double_click, &["self", "ctx"], "", ""),
    ("value_label", Mode::Method, value_label, &["self"], "", ""),
];
