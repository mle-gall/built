// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! A sheet item made by a placement tool (docs/rust-sheets.md step 6):
//! `ComposerWindow.place_tool`'s defaults for the tools a click or a drag
//! places, the record built through its class (built_core.composition) and not
//! yet added: the caller runs it as one step of the history.

use pyo3::prelude::*;
use pyo3::types::PyDict;

fn new_uid(py: Python<'_>) -> PyResult<String> {
    py.import("uuid")?.call_method0("uuid4")?.getattr("hex")?.extract()
}

/// The frame of ``comp`` holding page point (x, y), the topmost first,
/// given a uid when it has none (an item bound to it names it).
fn host<'py>(py: Python<'py>, comp: &Bound<'py, PyAny>, x: f64, y: f64) -> PyResult<Option<Bound<'py, PyAny>>> {
    let frames: Vec<Bound<'py, PyAny>> = comp.getattr("frames")?.try_iter()?.collect::<PyResult<_>>()?;
    for f in frames.iter().rev() {
        let n = |k: &str| -> PyResult<f64> { f.getattr(k)?.extract() };
        let (fx, fy, fw, fh) = (n("x_mm")?, n("y_mm")?, n("w_mm")?, n("h_mm")?);
        if fx <= x && x <= fx + fw && fy <= y && y <= fy + fh {
            if !f.getattr("uid")?.is_truthy()? {
                f.setattr("uid", new_uid(py)?)?;
            }
            return Ok(Some(f.clone()));
        }
    }
    Ok(None)
}

/// The item tool ``tool`` makes for a press at (x0, y0) released at
/// (x1, y1) on sheet ``comp`` (paper mm), in ``scene``; None when it makes
/// none (a title block where there is one). Its z puts it on top.
/// ``template``: the title block template a new title block starts from
/// (its file in ``template_dir``, `load_cajetin_template`).
#[pyfunction]
#[pyo3(signature = (comp, tool, x0, y0, x1, y1, scene=None, template_dir=None, template=None))]
#[allow(clippy::too_many_arguments)]
pub fn sheet_new_item<'py>(py: Python<'py>, comp: &Bound<'py, PyAny>, tool: &str, x0: f64, y0: f64, x1: f64, y1: f64,
                           scene: Option<Bound<'py, PyAny>>, template_dir: Option<std::path::PathBuf>,
                           template: Option<String>) -> PyResult<Option<Bound<'py, PyAny>>> {
    let c = py.import("built_core.composition")?;
    let tr = |s: &str| -> PyResult<String> { py.import("built_core.i18n")?.getattr("tr")?.call1((s,))?.extract() };
    let (w, h, x, y) = ((x1 - x0).abs(), (y1 - y0).abs(), x0.min(x1), y0.min(y1));
    let kw = PyDict::new(py);
    let first_scale = || -> PyResult<f64> {
        let frames = comp.getattr("frames")?;
        if frames.len()? > 0 { frames.get_item(0)?.getattr("scale_n")?.extract() } else { Ok(100.0) }
    };
    let make = |class: &str, kw: &Bound<'py, PyDict>| -> PyResult<Bound<'py, PyAny>> { c.getattr(class)?.call((), Some(kw)) };
    let item = match tool {
        "vista" => {
            kw.set_item("x_mm", x)?;
            kw.set_item("y_mm", y)?;
            kw.set_item("w_mm", w.max(60.0))?;
            kw.set_item("h_mm", h.max(50.0))?;
            kw.set_item("style", c.getattr("NEW_FRAME_STYLE")?)?;
            make("MarcoVista", &kw)?
        }
        "texto" => {
            kw.set_item("x_mm", x0)?;
            kw.set_item("y_mm", y0)?;
            kw.set_item("text", tr("Text")?)?;
            make("TextoItem", &kw)?
        }
        "llamada" => {
            kw.set_item("x_mm", x)?;
            kw.set_item("y_mm", y)?;
            kw.set_item("w_mm", w.max(8.0))?;
            kw.set_item("h_mm", h.max(8.0))?;
            kw.set_item("number", (comp.getattr("llamadas")?.len()? + 1).to_string())?;
            let item = make("LlamadaItem", &kw)?;
            let bubble: f64 = item.getattr("bubble_mm")?.extract()?;
            let iw: f64 = item.getattr("w_mm")?.extract()?;
            let ih: f64 = item.getattr("h_mm")?.extract()?;
            item.setattr("bx_mm", iw + bubble)?;
            item.setattr("by_mm", -bubble * 0.9)?;
            // Bound to the frame it was drawn on: it moves along.
            if let Some(f) = host(py, comp, x + iw / 2.0, y + ih / 2.0)? {
                item.setattr("frame_uid", f.getattr("uid")?)?;
            }
            item
        }
        "cajetin" => {
            if !comp.getattr("cajetin")?.is_none() {
                return Ok(None);
            }
            let item = comp.call_method0("default_cajetin")?;
            if let (Some(dir), Some(name)) = (&template_dir, template.as_deref().filter(|n| !n.is_empty())) {
                if apply_template(py, &item, &dir.join(format!("{name}.json")))? {
                    // Kept docked in the page's corner, whatever its size.
                    let (pw, ph): (f64, f64) = comp.call_method0("page_size_mm")?.extract()?;
                    let m: f64 = comp.getattr("margin_mm")?.extract()?;
                    let (w, h): (f64, f64) = (item.getattr("w_mm")?.extract()?, item.getattr("h_mm")?.extract()?);
                    item.setattr("x_mm", pw - m - w)?;
                    item.setattr("y_mm", ph - m - h)?;
                }
            }
            let today = py.import("datetime")?.getattr("date")?.call_method0("today")?.call_method1("strftime", ("%d/%m/%Y",))?;
            item.call_method1("set_field", ("FECHA", today))?;
            if comp.getattr("frames")?.len()? > 0 {
                let n = super::svg::fmt_g(first_scale()?);
                item.call_method1("set_field", ("ESCALA", format!("1:{n}")))?;
            }
            item
        }
        "escala" => {
            kw.set_item("x_mm", x0)?;
            kw.set_item("y_mm", y0)?;
            kw.set_item("scale_n", first_scale()?)?;
            make("BarraEscala", &kw)?
        }
        "norte" => {
            kw.set_item("x_mm", x0)?;
            kw.set_item("y_mm", y0)?;
            make("FlechaNorte", &kw)?
        }
        "leyenda" => {
            let (rows, swatches): (Bound<'py, PyAny>, Bound<'py, PyAny>) =
                py.import("built_core.layers")?.getattr("legend_rows")?.call1((scene.as_ref(),))?.extract()?;
            kw.set_item("x_mm", x0)?;
            kw.set_item("y_mm", y0)?;
            kw.set_item("rows", rows)?;
            kw.set_item("swatches", swatches)?;
            make("Leyenda", &kw)?
        }
        "perfil" => {
            // The first open path (a road, a canal), else the first.
            let mut index = 0usize;
            if let Some(s) = &scene {
                if let Some(paths) = s.getattr_opt("geo_paths")?.filter(|p| !p.is_none()) {
                    for (i, p) in paths.try_iter()?.enumerate() {
                        let p = p?;
                        let closed = p.getattr("closed").and_then(|c| c.is_truthy()).unwrap_or(false);
                        if !closed && p.getattr("points").and_then(|pts| pts.len()).unwrap_or(0) >= 2 {
                            index = i;
                            break;
                        }
                    }
                }
            }
            kw.set_item("x_mm", x)?;
            kw.set_item("y_mm", y)?;
            kw.set_item("w_mm", w.max(80.0))?;
            kw.set_item("h_mm", h.max(40.0))?;
            kw.set_item("path_index", index)?;
            make("PerfilTerreno", &kw)?
        }
        "linea" | "flecha" | "terreno" => {
            // A line's box may be flat in one axis.
            kw.set_item("kind", tool)?;
            kw.set_item("x_mm", x)?;
            kw.set_item("y_mm", y)?;
            kw.set_item("w_mm", w)?;
            kw.set_item("h_mm", h)?;
            kw.set_item("invert", (x1 < x0) != (y1 < y0))?;
            if tool == "terreno" {
                // A ground line reads as ground: a heavier pen, and the
                // look of the last one.
                kw.set_item("stroke_mm", 0.5)?;
                super::looks::apply(py, "ground", &kw, None)?;
            }
            make("FormaItem", &kw)?
        }
        "etiqueta" => {
            // The first click the pointed-at spot, the second the text.
            kw.set_item("x_mm", x1)?;
            kw.set_item("y_mm", y1)?;
            kw.set_item("ax_mm", x0 - x1)?;
            kw.set_item("ay_mm", y0 - y1)?;
            kw.set_item("text", tr("Label")?)?;
            make("EtiquetaItem", &kw)?
        }
        "nivel" => {
            // The last mark's look, in the document's units.
            super::looks::apply(py, "nivel", &kw, None)?;
            length_style(py, scene.as_ref(), &kw)?;
            kw.set_item("x_mm", x0)?;
            kw.set_item("y_mm", y0)?;
            let item = make("NivelItem", &kw)?;
            // A plan (looking down) takes the plan symbol.
            let frames: Vec<Bound<'py, PyAny>> = comp.getattr("frames")?.try_iter()?.collect::<PyResult<_>>()?;
            for f in frames.iter().rev() {
                let n = |k: &str| -> PyResult<f64> { f.getattr(k)?.extract() };
                if n("x_mm")? <= x0 && x0 <= n("x_mm")? + n("w_mm")? && n("y_mm")? <= y0 && y0 <= n("y_mm")? + n("h_mm")? {
                    if f.getattr("view_key")?.extract::<String>()? == "std:top" {
                        item.setattr("symbol", "circle")?;
                    }
                    break;
                }
            }
            item
        }
        "rect" | "elipse" | "poligono" => {
            kw.set_item("kind", tool)?;
            kw.set_item("x_mm", x)?;
            kw.set_item("y_mm", y)?;
            kw.set_item("w_mm", w.max(2.0))?;
            kw.set_item("h_mm", h.max(2.0))?;
            make("FormaItem", &kw)?
        }
        _ => return Ok(None),
    };
    item.setattr("z", super::display::sheet_next_z(comp)?)?;
    Ok(Some(item))
}

/// A new sheet from the template file ``path`` (the composer's
/// `_composition_from_template`): its page, border and items, each frame
/// with an identity of its own and the dimensions bound to no model; None
/// for a missing or broken file.
#[pyfunction]
pub fn sheet_from_template<'py>(py: Python<'py>, path: std::path::PathBuf) -> PyResult<Option<Bound<'py, PyAny>>> {
    let Ok(text) = std::fs::read_to_string(&path) else { return Ok(None) };
    let Ok(data) = py.import("json")?.call_method1("loads", (text,)) else { return Ok(None) };
    let Ok(comp) = py.import("built_core.composition")?.getattr("Composicion")?.call_method1("from_dict", (data,)) else {
        return Ok(None);
    };
    let uuid = py.import("uuid")?;
    for f in comp.getattr("frames")?.try_iter()? {
        f?.setattr("uid", uuid.call_method0("uuid4")?.getattr("hex")?)?;
    }
    for ct in comp.getattr("cotas")?.try_iter()? {
        let ct = ct?;
        ct.setattr("anchor_uid", "")?;
        ct.setattr("a_world", py.None())?;
        ct.setattr("b_world", py.None())?;
    }
    Ok(Some(comp))
}

/// The document's length unit and precision a new annotation is born with
/// (`_document_length_style`), and whether a cota shows its unit symbol.
fn length_style<'py>(py: Python<'py>, scene: Option<&Bound<'py, PyAny>>, kw: &Bound<'py, PyDict>) -> PyResult<()> {
    let chosen = py.import("built_core.units")?.getattr("model_units_of")?.call1((scene,))?;
    kw.set_item("units", chosen.get_item("length")?)?;
    kw.set_item("decimals", chosen.get_item("precision")?.call_method0("__int__")?)?;
    Ok(())
}

/// A linear dimension from page point ``a`` to ``b`` with its line ``sep``
/// mm off them (``axis`` "h"/"v": forced onto a row or a column), in the
/// look `_new_cota` gives a new one: the document's units, its unit symbol
/// when its dimension style shows it, ISO's text above the middle of the
/// line, aligned with it, a 0.5 mm gap, arrows; at the sheet's first
/// frame's scale. Not yet added; its z puts it on top.
#[pyfunction]
#[pyo3(signature = (comp, a, b, sep=0.0, axis="", scene=None))]
pub fn sheet_new_cota<'py>(py: Python<'py>, comp: &Bound<'py, PyAny>, a: (f64, f64), b: (f64, f64), sep: f64,
                           axis: &str, scene: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyAny>> {
    let c = py.import("built_core.composition")?;
    let kw = PyDict::new(py);
    // The last dimension's look (text, ends, colour, gap), then the
    // document's units, every time.
    super::looks::apply(py, "cota", &kw, None)?;
    length_style(py, scene.as_ref(), &kw)?;
    let shows = match scene.as_ref().map(|s| s.getattr_opt("dimension_style")).transpose()?.flatten() {
        Some(style) if style.is_truthy()? => match style.call_method1("get", ("show_unit", true)) {
            Ok(v) => v.is_truthy()?,
            Err(_) => true,
        },
        _ => true,
    };
    kw.set_item("show_unit", shows)?;
    if !kw.contains("offset_mm")? {
        kw.set_item("offset_mm", 0.5)?;
    }
    // ISO: aligned with the line, above its middle — placed per dimension,
    // never inherited.
    kw.set_item("text_align", "aligned")?;
    kw.set_item("text_pos", "above")?;
    kw.set_item("text_along", "middle")?;
    if !kw.contains("ends")? {
        kw.set_item("ends", "arrow")?;
    }
    let frames = comp.getattr("frames")?;
    let n: f64 = if frames.len()? > 0 { frames.get_item(0)?.getattr("scale_n")?.extract()? } else { 100.0 };
    kw.set_item("x_mm", a.0)?;
    kw.set_item("y_mm", a.1)?;
    kw.set_item("dx_mm", b.0 - a.0)?;
    kw.set_item("dy_mm", b.1 - a.1)?;
    kw.set_item("scale_n", n)?;
    kw.set_item("sep_mm", sep)?;
    kw.set_item("axis", axis)?;
    let item = c.getattr("CotaItem")?.call((), Some(&kw))?;
    item.setattr("z", super::display::sheet_next_z(comp)?)?;
    Ok(item)
}

/// A chain's total (`place_chain_total`): a dimension from its first point
/// ``a`` to its last ``b``, stacked one row (a text height and clearances)
/// beyond the chain's line, which ran ``sep`` off its first two points
/// ``p1`` → ``p2``.
#[pyfunction]
#[pyo3(signature = (comp, a, b, sep, axis, p1, p2, scene=None))]
#[allow(clippy::too_many_arguments)]
pub fn sheet_new_chain_total<'py>(py: Python<'py>, comp: &Bound<'py, PyAny>, a: (f64, f64), b: (f64, f64), sep: f64,
                                  axis: &str, p1: (f64, f64), p2: (f64, f64), scene: Option<Bound<'py, PyAny>>)
    -> PyResult<Bound<'py, PyAny>> {
    let item = sheet_new_cota(py, comp, a, b, 0.0, axis, scene)?;
    let text_mm: f64 = item.getattr("text_mm")?.extract()?;
    let step = text_mm * 2.0 + 2.5;
    let out = if sep >= 0.0 { step } else { -step };
    let total = match axis {
        "h" => (p1.1 + sep - a.1) + out,
        "v" => (p1.0 + sep - a.0) + out,
        _ => {
            let (dx, dy) = (p2.0 - p1.0, p2.1 - p1.1);
            let l = dx.hypot(dy);
            let (nx, ny) = if l > 1e-9 { (-dy / l, dx / l) } else { (0.0, -1.0) };
            let line = (p1.0 + nx * sep, p1.1 + ny * sep);
            let (tx, ty) = (b.0 - a.0, b.1 - a.1);
            let length = tx.hypot(ty);
            if length > 1e-9 {
                let (tnx, tny) = (-ty / length, tx / length);
                (line.0 - a.0) * tnx + (line.1 - a.1) * tny + out
            } else {
                sep + out
            }
        }
    };
    item.setattr("sep_mm", total)?;
    Ok(item)
}

/// The frame of ``comp`` under page point (x, y), the topmost first.
fn frame_at<'py>(comp: &Bound<'py, PyAny>, x: f64, y: f64) -> PyResult<Option<Bound<'py, PyAny>>> {
    let frames: Vec<Bound<'py, PyAny>> = comp.getattr("frames")?.try_iter()?.collect::<PyResult<_>>()?;
    for f in frames.iter().rev() {
        let n = |k: &str| -> PyResult<f64> { f.getattr(k)?.extract() };
        if n("x_mm")? <= x && x <= n("x_mm")? + n("w_mm")? && n("y_mm")? <= y && y <= n("y_mm")? + n("h_mm")? {
            return Ok(Some(f.clone()));
        }
    }
    Ok(None)
}

/// A radius or diameter dimension (`place_radial`): the centre and a point
/// on the arc (page mm), at the scale of the frame the centre fell in, in
/// the document's units; None for a circle under half a millimetre.
#[pyfunction]
#[pyo3(signature = (comp, centre, arc_point, kind="radius", scene=None))]
pub fn sheet_new_radial<'py>(py: Python<'py>, comp: &Bound<'py, PyAny>, centre: (f64, f64), arc_point: (f64, f64),
                             kind: &str, scene: Option<Bound<'py, PyAny>>) -> PyResult<Option<Bound<'py, PyAny>>> {
    let r = (arc_point.0 - centre.0).hypot(arc_point.1 - centre.1);
    if r < 0.5 {
        return Ok(None);
    }
    let n: f64 = match frame_at(comp, centre.0, centre.1)? {
        Some(f) => f.getattr("scale_n")?.extract()?,
        None => {
            let frames = comp.getattr("frames")?;
            if frames.len()? > 0 { frames.get_item(0)?.getattr("scale_n")?.extract()? } else { 100.0 }
        }
    };
    let kw = PyDict::new(py);
    super::looks::apply(py, "cota", &kw, Some(&["text_mm", "stroke_mm", "color", "text_color", "ends", "text_bg",
                                                "text_bg_opacity", "offset_mm"]))?;
    length_style(py, scene.as_ref(), &kw)?;
    let shows = match scene.as_ref().map(|s| s.getattr_opt("dimension_style")).transpose()?.flatten() {
        Some(style) if style.is_truthy()? => style.call_method1("get", ("show_unit", true)).map_or(Ok(true), |v| v.is_truthy())?,
        _ => true,
    };
    kw.set_item("show_unit", shows)?;
    kw.set_item("x_mm", centre.0)?;
    kw.set_item("y_mm", centre.1)?;
    kw.set_item("radius_mm", r)?;
    kw.set_item("scale_n", n)?;
    kw.set_item("kind", if kind == "diameter" { "diameter" } else { "radius" })?;
    kw.set_item("angle_deg", (arc_point.1 - centre.1).atan2(arc_point.0 - centre.0).to_degrees())?;
    let item = py.import("built_core.composition")?.getattr("CotaRadialItem")?.call((), Some(&kw))?;
    item.setattr("z", super::display::sheet_next_z(comp)?)?;
    Ok(Some(item))
}

/// An angular dimension (`place_angular`): the vertex, a point on each arm
/// (page mm) and the arc's radius (2 mm at least).
#[pyfunction]
pub fn sheet_new_angular<'py>(py: Python<'py>, comp: &Bound<'py, PyAny>, vertex: (f64, f64), a: (f64, f64), b: (f64, f64),
                              radius_mm: f64) -> PyResult<Option<Bound<'py, PyAny>>> {
    if (a.0 - vertex.0).hypot(a.1 - vertex.1) < 1e-6 || (b.0 - vertex.0).hypot(b.1 - vertex.1) < 1e-6 {
        return Ok(None);
    }
    let kw = PyDict::new(py);
    super::looks::apply(py, "cota", &kw, Some(&["text_mm", "stroke_mm", "color", "text_color"]))?;
    kw.set_item("x_mm", vertex.0)?;
    kw.set_item("y_mm", vertex.1)?;
    kw.set_item("ax_mm", a.0 - vertex.0)?;
    kw.set_item("ay_mm", a.1 - vertex.1)?;
    kw.set_item("bx_mm", b.0 - vertex.0)?;
    kw.set_item("by_mm", b.1 - vertex.1)?;
    kw.set_item("radius_mm", radius_mm.max(2.0))?;
    let item = py.import("built_core.composition")?.getattr("CotaAngularItem")?.call((), Some(&kw))?;
    item.setattr("z", super::display::sheet_next_z(comp)?)?;
    Ok(Some(item))
}

/// An image item for picture file ``path``, 60 mm wide in its proportions,
/// at the sheet's top-left margin (`_on_add_image`); None for a file that
/// is no picture.
#[pyfunction]
pub fn sheet_new_image<'py>(py: Python<'py>, comp: &Bound<'py, PyAny>, path: String) -> PyResult<Option<Bound<'py, PyAny>>> {
    let Ok((w, h)) = image::image_dimensions(&path) else { return Ok(None) };
    let margin: f64 = comp.getattr("margin_mm")?.extract()?;
    let kw = PyDict::new(py);
    kw.set_item("x_mm", margin + 4.0)?;
    kw.set_item("y_mm", margin + 4.0)?;
    kw.set_item("w_mm", 60.0)?;
    kw.set_item("h_mm", if w > 0 { 60.0 * f64::from(h) / f64::from(w) } else { 40.0 })?;
    kw.set_item("path", path)?;
    let item = py.import("built_core.composition")?.getattr("ImagenItem")?.call((), Some(&kw))?;
    item.setattr("z", super::display::sheet_next_z(comp)?)?;
    Ok(Some(item))
}

/// A title block template's fields on ``item`` (`load_cajetin_template`):
/// only what a title block has, never its place, z, lock or group; the
/// rows as text pairs. False when there is no such template.
fn apply_template(py: Python<'_>, item: &Bound<'_, PyAny>, path: &std::path::Path) -> PyResult<bool> {
    let Ok(text) = std::fs::read_to_string(path) else { return Ok(false) };
    let json = py.import("json")?;
    let Ok(raw) = json.call_method1("loads", (text,)) else { return Ok(false) };
    let Ok(raw) = raw.cast_into::<PyDict>() else { return Ok(false) };
    let fields: Vec<String> = item.getattr("_record_fields")?.extract()?;
    let mut any = false;
    for (k, v) in raw.iter() {
        let k: String = k.extract()?;
        if !fields.contains(&k) || matches!(k.as_str(), "x_mm" | "y_mm" | "z" | "locked" | "group_id") {
            continue;
        }
        let v = if k == "campos" {
            let rows = pyo3::types::PyList::empty(py);
            for r in v.try_iter()? {
                let r = r?;
                if (r.is_instance_of::<pyo3::types::PyList>() || r.is_instance_of::<pyo3::types::PyTuple>()) && r.len()? >= 2 {
                    rows.append(vec![r.get_item(0)?.str()?, r.get_item(1)?.str()?])?;
                }
            }
            rows.into_any()
        } else {
            v
        };
        item.setattr(k.as_str(), v)?;
        any = true;
    }
    Ok(any)
}
