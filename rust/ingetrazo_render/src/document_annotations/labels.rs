// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
struct Style<'py> {
    data: Obj<'py>,
    ink: Obj<'py>,
    selected: Obj<'py>,
    font: Obj<'py>,
}
impl<'py> Style<'py> {
    fn new(c: &Paint<'py>, scene: &Obj<'py>) -> PyResult<Self> {
        let data = scene
            .getattr_opt("dimension_style")?
            .unwrap_or_else(|| PyDict::new(scene.py()).into_any());
        let rgb = data
            .call_method1("get", ("color", [45, 55, 75]))?
            .extract()?;
        let ink = c.rgb(rgb)?;
        let selected = c.rgb([243, 115, 41])?;
        let font = c
            .styles
            .call_method1("QFont", (data.call_method1("get", ("font_family", ""))?,))?;
        let size = scene
            .py()
            .import("builtins")?
            .call_method1("int", (data.call_method1("get", ("font_size", 9))?,))?;
        font.call_method1("setPointSize", (size,))?;
        for (method, key, default) in [
            ("setBold", "font_bold", true),
            ("setItalic", "font_italic", false),
        ] {
            font.call_method1(method, (data.call_method1("get", (key, default))?,))?;
        }
        Ok(Self {
            data,
            ink,
            selected,
            font,
        })
    }
    fn pen(&self, c: &Paint, sel: &Obj, entity: &Obj) -> PyResult<&Obj<'py>> {
        Ok(
            if sel.contains(entity)? || entity.is(&c.v.getattr("_hover_entity")?) {
                &self.selected
            } else {
                &self.ink
            },
        )
    }
}
#[pyfunction]
fn text_block_x(p: Point, anchor: Option<Point>, width: f64) -> f64 {
    if anchor.is_some_and(|a| p.0 < a.0) {
        p.0 - 6. - width
    } else {
        p.0 + 6.
    }
}
#[pyfunction]
fn draw_text_labels(v: &Obj, p: &Obj) -> PyResult<()> {
    let scene = v.getattr("scene")?;
    let labels = opt(&scene, "text_labels")?;
    if !labels.is_truthy()? {
        return Ok(());
    }
    let c = Paint::new(v, p)?;
    let style = Style::new(&c, &scene)?;
    let sel = scene.getattr("selection")?;
    p.call_method1("setFont", (&style.font,))?;
    let metrics = p.call_method0("fontMetrics")?;
    let shadow = c.rgba([255, 255, 255, 230])?;
    for lab in labels.try_iter()? {
        let lab = lab?;
        if !scene.call_method1("entity_visible", (&lab,))?.is_truthy()? {
            continue;
        }
        let ink = style.pen(&c, &sel, &lab)?;
        let pos = lab.call_method0("position")?;
        let a = lab.getattr("anchor")?;
        let anchor = c.project(&a)?;
        let Some(at) = c.project(&pos)? else { continue };
        c.pen(ink, 1.2)?;
        v.call_method1("_draw_occluded_segment", (p, &a, pos))?;
        if let Some(q) = anchor {
            if !v.call_method1("_occluded_on_screen", (a,))?.is_truthy()? {
                c.brush(ink)?;
                c.circle(q, 2.5)?;
                c.no_brush()?
            }
        }
        let mut lines = lab
            .getattr("text")?
            .call_method0("splitlines")?
            .extract::<Vec<String>>()?;
        if lines.is_empty() {
            lines.push(String::new())
        }
        let mut width = 0.;
        for s in &lines {
            width = f64::max(
                width,
                metrics.call_method1("horizontalAdvance", (s,))?.extract()?,
            );
        }
        let x = v
            .call_method1("_text_block_x", (at, anchor, width))?
            .extract::<f64>()?;
        let height = metrics.call_method0("height")?.extract::<f64>()?;
        for (i, line) in lines.iter().enumerate() {
            c.shadow_text((x, at.1 - 4. + i as f64 * height), line, ink, &shadow)?
        }
    }
    Ok(())
}
#[pyfunction]
fn draw_dimensions(v: &Obj, p: &Obj) -> PyResult<()> {
    let scene = v.getattr("scene")?;
    let dims = opt(&scene, "dimensions")?;
    if !dims.is_truthy()? {
        return Ok(());
    }
    let c = Paint::new(v, p)?;
    let style = Style::new(&c, &scene)?;
    let sel = scene.getattr("selection")?;
    let riding = opt(v, "_preview_groups")?;
    let carry = if riding.is_truthy()? {
        v.call_method0("_groups_preview_transform")?
    } else {
        none(v.py())
    };
    let shadow = c.rgba([255, 255, 255, 230])?;
    for dim in dims.try_iter()? {
        let mut dim = dim?;
        if !scene.call_method1("entity_visible", (&dim,))?.is_truthy()? {
            continue;
        }
        let ink = style.pen(&c, &sel, &dim)?;
        if riding.is_truthy()? {
            let ends = dim.call_method1("ends_riding", (&riding,))?;
            if ends.is_truthy()? {
                dim = dim.call_method1("carried", (&carry, ends))?
            }
        }
        let (a, b): (Obj, Obj) = dim.call_method0("line_points")?.extract()?;
        let (Some(pa), Some(pb)) = (c.project(&a)?, c.project(&b)?) else {
            continue;
        };
        c.pen(ink, 1.)?;
        v.call_method1("_draw_occluded_segment", (p, dim.getattr("a")?, &a))?;
        v.call_method1("_draw_occluded_segment", (p, dim.getattr("b")?, &b))?;
        c.pen(ink, 1.5)?;
        v.call_method1("_draw_occluded_segment", (p, &a, &b))?;
        let dx = pb.0 - pa.0;
        let dy = pb.1 - pa.1;
        let len = dx.hypot(dy);
        let ends = style.data.call_method1("get", ("ends", "arrow"))?;
        if len > 1e-6 && !ends.eq("none")? {
            let (ux, uy) = (dx / len, dy / len);
            for ((x, y), world, sign) in [(pa, &a, 1.), (pb, &b, -1.)] {
                if v.call_method1("_occluded_on_screen", (world,))?
                    .is_truthy()?
                {
                    continue;
                }
                if ends.eq("tick")? {
                    let k = 4. * std::f64::consts::FRAC_1_SQRT_2;
                    let (ox, oy) = ((ux + uy) * k, (uy - ux) * k);
                    c.line((x - ox, y - oy), (x + ox, y + oy))?
                } else {
                    let (bx, by) = (x + sign * ux * 10., y + sign * uy * 10.);
                    p.call_method0("save")?;
                    c.brush(ink)?;
                    c.set_pen(&c.flag("NoPen")?)?;
                    c.polygon(&[
                        (x, y),
                        (bx - uy * 2.2, by + ux * 2.2),
                        (bx + uy * 2.2, by - ux * 2.2),
                    ])?;
                    p.call_method0("restore")?;
                }
            }
        }
        let mid = dim.call_method0("midpoint")?;
        if let Some((x, y)) = c.project(&mid)? {
            if !v.call_method1("_occluded_on_screen", (mid,))?.is_truthy()? {
                let text = dim
                    .call_method1(
                        "display_text",
                        (v.call_method1(
                            "_format_dim_value",
                            (dim.call_method0("value")?, &style.data),
                        )?,),
                    )?
                    .extract::<String>()?;
                p.call_method1("setFont", (&style.font,))?;
                c.shadow_text((x + 4., y - 5.), &text, ink, &shadow)?
            }
        }
    }
    Ok(())
}
#[pyfunction]
fn length_label_at<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let tool = v.getattr("active_tool")?;
    let no = none(py);
    if tool.is_none() {
        return Ok(no);
    }
    let provider = opt(&tool, "value_label")?;
    let (mut text, mid) = if provider.is_callable() {
        let val = provider.call0()?;
        if !(val.cast::<PyTuple>().is_ok() || val.cast::<PyList>().is_ok())
            || val.len()? != 2
            || val.get_item(1)?.is_none()
        {
            return Ok(no);
        }
        (val.get_item(0)?, val.get_item(1)?)
    } else {
        let seg = tool.call_method0("rubber_band_lines")?;
        if seg.len()? != 1 {
            return Ok(no);
        }
        let (a, b): (Obj, Obj) = seg.get_item(0)?.extract()?;
        let (a, b) = (xyz(&a)?, xyz(&b)?);
        (
            host(py)?.call_method1("_fmt_len", ((b - a).length(),))?,
            vec3(py, (a + b) * 0.5)?,
        )
    };
    let buffer = v.getattr("_value_buffer")?;
    let typing = buffer.is_truthy()?;
    if typing {
        text = if call_opt(&tool, "value_is_unitless")?.is_truthy()? {
            buffer
        } else {
            host(py)?.call_method1("_typed_value_text", (buffer,))?
        }
    }
    Ok((text, mid, typing).into_pyobject(py)?.into_any())
}
#[pyfunction]
fn measurement_text<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let tool = v.getattr("active_tool")?;
    if !tool.is_none() {
        let label = call_opt(&tool, "value_label")?;
        if !label.is_none() {
            return label.get_item(0);
        }
        let seg = tool.call_method0("rubber_band_lines")?;
        if seg.len()? == 1 {
            let (a, b): (Obj, Obj) = seg.get_item(0)?.extract()?;
            return host(py)?.call_method1("_fmt_len", ((xyz(&b)? - xyz(&a)?).length(),));
        }
    }
    Ok("".into_pyobject(py)?.into_any())
}
#[pyfunction]
fn draw_length_label(v: &Obj, p: &Obj) -> PyResult<()> {
    if truth(v, "_chrome_owns_labels")? {
        return Ok(());
    }
    let label = v.call_method0("_length_label_at")?;
    if label.is_none() {
        return Ok(());
    }
    let (text, mid, typing): (String, Obj, bool) = label.extract()?;
    let c = Paint::new(v, p)?;
    if let Some((x, y)) = c.project(&mid)? {
        c.font(10, true, false)?;
        let shadow = c.rgba(if typing {
            [255, 220, 130, 235]
        } else {
            [255, 255, 255, 220]
        })?;
        c.shadow_text((x + 11., y - 8.), &text, &c.rgb([15, 20, 27])?, &shadow)?
    }
    Ok(())
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(draw_text_labels, m)?)?;
    m.add_function(wrap_pyfunction!(draw_dimensions, m)?)?;
    m.add_function(wrap_pyfunction!(text_block_x, m)?)?;
    m.add_function(wrap_pyfunction!(length_label_at, m)?)?;
    m.add_function(wrap_pyfunction!(measurement_text, m)?)?;
    m.add_function(wrap_pyfunction!(draw_length_label, m)?)?;
    Ok(())
}
