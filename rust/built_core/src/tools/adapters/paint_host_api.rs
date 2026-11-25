// SPDX-License-Identifier: GPL-3.0-or-later
//! PaintHost callbacks on the live document.
use super::*;

fn container_under<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let group = a.get_item(1)?;
    let vp = a.get_item(0)?;
    let x = a.get_item(2)?;
    let y = a.get_item(3)?;
    if group.is_none() {
        return Ok(none(py));
    }
    let pick_obj = optional(&vp, "pick_group")?;
    Ok(if !pick_obj.is_none() {
        pick_obj.call1((&x, &y))?
    } else {
        none(py)
    })
}
fn every_face_painted<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let mut _m: Obj<'py>;
    let mut g: Obj<'py>;
    let obj = a.get_item(0)?;
    let iter_placements = imported(py, "core.group")?.getattr("iter_placements")?;
    let has_own_material = imported(py, "built_core.materials")?.getattr("has_own_material")?;
    let face_attrs = imported(py, "core.mesh")?.getattr("face_attrs")?;
    let mut seen = false.into_bound_py_any(py)?;
    for item in iter_placements.call1((&obj,))?.try_iter()? {
        [g, _m] = unpack::<2>(&item?)?;
        for item in face_attrs.call1((g.getattr("mesh")?,))?.try_iter()? {
            let attrs = item?;
            seen = true.into_bound_py_any(py)?;
            if !(has_own_material.call1((&attrs,))?.is_truthy()?) {
                return false.into_bound_py_any(py);
            }
        }
    }
    Ok(seen.clone())
}
fn remember_current<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.paint_host")?;
    let cls = a.get_item(0)?;
    let vp = a.get_item(1)?;
    if cls.getattr("current_is_default")?.is_truthy()? {
        return Ok(none(py));
    }
    let before = cls.getattr("recent")?;
    cls.setattr(
        "recent",
        module.call_method1(
            "push_recent",
            (
                cls.getattr("recent")?,
                cls.call_method0("_current_as_material")?,
            ),
        )?,
    )?;
    if cls.getattr("recent")?.eq(&before)? {
        return Ok(none(py));
    }
    let win = if vp.hasattr("window")? {
        vp.call_method0("window")?
    } else {
        none(py)
    };
    let panel = optional(&optional(&win, "tray")?, "materials")?;
    if panel.hasattr("recent_materials_changed")? {
        panel.call_method0("recent_materials_changed")?;
    }
    Ok(none(py))
}
fn current_as_material<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.paint_host")?;
    let cls = a.get_item(0)?;
    module.call_method1(
        "paint_stamp",
        (
            cls.getattr("current_color")?,
            cls.getattr("current_texture")?,
            cls.getattr("current_opacity")?,
            cls.getattr("current_material")?,
        ),
    )
}
fn back_material_for<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.paint_host")?;
    let cls = a.get_item(0)?;
    let face = a.get_item(1)?;
    module.call_method1(
        "paint_stamp",
        (
            cls.getattr("current_color")?,
            cls.getattr("current_texture")?,
            cls.getattr("current_opacity")?,
            cls.getattr("current_material")?,
            cls.getattr("current_texture_plane")?,
            &face,
        ),
    )
}
fn on_hover<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method1("_input", ("hover", &ctx))?;
    Ok(none(py))
}
fn on_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    if tool
        .call_method1("_input", ("click", &ctx))?
        .get_item("redraw")?
        .is_truthy()?
    {
        ctx.getattr("viewport")?.call_method0("update")?;
    }
    Ok(none(py))
}
fn host_brush<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let refresh_library_paint = imported(py, "built_core")?.getattr("refresh_library_paint")?;
    let material_for_paint =
        imported(py, "core.material_commands")?.getattr("material_for_paint")?;
    let brush = tool.get_type().into_any();
    refresh_library_paint.call1((vp.getattr("scene")?, &brush))?;
    if !(brush.getattr("current_is_default")?.is_truthy()?)
        && brush.getattr("current_material")?.is_none()
    {
        brush.setattr("current_material", material_for_paint.call((vp.getattr("scene")?,), Some(&fields!(py; "color" => if !brush.getattr("current_texture")?.is_none() { none(py) } else { brush.getattr("current_color")? }, "texture" => brush.getattr("current_texture")?, "opacity" => brush.getattr("current_opacity")?)))?)?;
    }
    Ok(fields!(py; "material" => if brush.getattr("current_material")?.is_truthy()? { brush.getattr("current_material")?.call_method0("to_dict")? } else { none(py) }, "default" => brush.getattr("current_is_default")?, "color" => brush.getattr("current_color")?, "texture" => brush.getattr("current_texture")?, "opacity" => brush.getattr("current_opacity")?, "plane" => if !brush.getattr("current_texture")?.is_none() { brush.getattr("current_texture_plane")? } else { none(py) }).into_any())
}
fn host_container_painted<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.paint_host")?;
    let name: Obj<'py>;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let flash = optional(&vp, "flash_status")?;
    if flash.is_callable() {
        name = {
            let value = tool
                .getattr("_container")?
                .getattr_opt("name")?
                .unwrap_or(("").into_bound_py_any(py)?);
            if value.is_truthy()? {
                value
            } else {
                module.call_method1("tr", ("Group",))?
            }
        };
        if tool
            .call_method1("_every_face_painted", (tool.getattr("_container")?,))?
            .is_truthy()?
        {
            flash.call1((module.call_method("tr", ("{name}: every face has its own material, so nothing changed — open it and paint the faces",), Some(&fields!(py; "name" => &name)))?, 10000,))?;
        } else {
            flash.call1((module.call_method(
                "tr",
                ("{name} painted as a whole; a face that already had its own material keeps it",),
                Some(&fields!(py; "name" => &name)),
            )?,))?;
        }
    }
    Ok(none(py))
}
fn host_remember_paint<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    tool.call_method1("_remember_current", (&vp,))?;
    Ok(none(py))
}

fn input<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(2)?;
    let kind = a.get_item(1)?;
    let tool = a.get_item(0)?;
    let action = (|| -> Reply<'py> {
        return Ok(tool.call_method1(
            "_send",
            (
                ctx.getattr("viewport")?,
                tool.call_method1("_pointer", (&kind, &ctx))?,
            ),
        )?);
    })();
    {
        let value = none(py);
        tool.setattr("_face", &value)?;
        tool.setattr("_owner", &value)?;
        tool.setattr("_container", &value)?;
    }
    action
}

fn host_adopt_sample<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let [tool, vp, sample] = unpack::<3>(a.as_any())?;
    let cls = tool.get_type();
    let module = imported(py, "tools.paint_host")?;
    let attrs = sample.get_item("attrs")?;
    let attrs = attrs
        .cast::<PyDict>()
        .ok()
        .filter(|d| !d.is_empty())
        .cloned()
        .unwrap_or_else(|| PyDict::new(py));
    let field = |name: &str| Ok::<_, PyErr>(attrs.get_item(name)?.unwrap_or_else(|| none(py)));
    let texture = field("texture")?;
    cls.setattr(
        "current_is_default",
        !imported(py, "built_core.materials")?
            .call_method1("has_own_material", (&attrs,))?
            .is_truthy()?,
    )?;
    cls.setattr("current_texture", &texture)?;
    let plane = sample.get_item("plane")?;
    let plane = if plane.is_truthy()? {
        (
            module.call_method1("Vec3", positional(&plane.get_item(0)?)?)?,
            plane.get_item(1)?,
        )
            .into_bound_py_any(py)?
    } else {
        none(py)
    };
    cls.setattr("current_texture_plane", plane)?;
    if texture.is_none() {
        let color = field("color")?;
        let color = if color.is_truthy()? {
            color
        } else {
            module.getattr("DEFAULT_FACE_COLOR")?
        };
        cls.setattr("current_color", positional(&color)?)?;
    }
    cls.setattr("current_opacity", field("opacity")?)?;
    let mat = field("mat")?;
    let material = if mat.is_truthy()? {
        vp.getattr("scene")?
            .getattr("materials")?
            .call_method1("get", (mat,))?
    } else {
        none(py)
    };
    cls.setattr("current_material", material)?;
    let window = if vp.hasattr("window")? {
        vp.call_method0("window")?
    } else {
        none(py)
    };
    if cls.getattr("sample_armed")?.is_truthy()? {
        cls.setattr("sample_armed", false)?;
        if window.hasattr("release_eyedropper")? {
            window.call_method0("release_eyedropper")?;
        }
    }
    let panel = optional(&optional(&window, "tray")?, "materials")?;
    if panel.hasattr("sync_from_paint")? {
        panel.call_method0("sync_from_paint")?;
    }
    if let Some(flash) = vp.getattr_opt("flash_status")?.filter(|f| f.is_callable()) {
        flash.call1((
            module.call_method1("tr", ("Material sampled — click a face to paint it",))?,
        ))?;
    }
    Ok(none(py))
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("_container_under", Mode::Static, container_under, &["vp", "group", "x:float", "y:float"], "", "The group or component a click at ``(x, y)`` paints as a whole —\na face inside a container reached from OUTSIDE it — or ``None``\nwhen the click paints the face. The hover and the click both ask\nhere, so the marking always shows what the click will paint."),
    ("_every_face_painted", Mode::Static, every_face_painted, &["obj"], "bool", "Whether every face of the group or component — nested\nplacements included — carries its own material, so painting it as\na whole shows no change."),
    ("_remember_current", Mode::Class, remember_current, &["cls", "vp=None"], "None", "Put the material just painted at the head of ``recent`` and let\nthe tray know. The «Default» material is not one: painting with it\ntakes paint off."),
    ("_current_as_material", Mode::Class, current_as_material, &["cls"], "", ""),
    ("_back_material_for", Mode::Class, back_material_for, &["cls", "face"], "", ""),
    ("_input", Mode::Method, input, &["self", "kind", "ctx"], "", ""),
    ("on_hover", Mode::Method, on_hover, &["self", "ctx"], "", ""),
    ("on_click", Mode::Method, on_click, &["self", "ctx"], "", ""),
    ("host_adopt_sample", Mode::Method, host_adopt_sample, &["self", "vp", "sample"], "", ""),
    ("host_brush", Mode::Method, host_brush, &["self", "vp", "_"], "", ""),
    ("host_container_painted", Mode::Method, host_container_painted, &["self", "vp", "_"], "", ""),
    ("host_remember_paint", Mode::Method, host_remember_paint, &["self", "vp", "_"], "", ""),
];
