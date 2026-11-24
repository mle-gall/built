// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! A sheet item made by a placement tool (docs/rust-sheets.md step 6):
//! `ComposerWindow.place_tool`'s defaults for the tools a click or a drag
//! places, the record built through its class (core.composition) and not
//! yet added: the caller runs it as one step of the history.

use pyo3::prelude::*;
use pyo3::types::PyDict;

/// The tools ``sheet_new_item`` makes an item for.
pub const PLACE_TOOLS: [&str; 13] = ["vista", "texto", "llamada", "cajetin", "escala", "norte", "leyenda", "perfil",
                                     "linea", "flecha", "terreno", "rect", "elipse"];

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
#[pyfunction]
#[pyo3(signature = (comp, tool, x0, y0, x1, y1, scene=None))]
#[allow(clippy::too_many_arguments)]
pub fn sheet_new_item<'py>(py: Python<'py>, comp: &Bound<'py, PyAny>, tool: &str, x0: f64, y0: f64, x1: f64, y1: f64,
                           scene: Option<Bound<'py, PyAny>>) -> PyResult<Option<Bound<'py, PyAny>>> {
    let c = py.import("core.composition")?;
    let tr = |s: &str| -> PyResult<String> { py.import("core.i18n")?.getattr("tr")?.call1((s,))?.extract() };
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
                py.import("core.layers")?.getattr("legend_rows")?.call1((scene.as_ref(),))?.extract()?;
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
                kw.set_item("stroke_mm", 0.5)?;
            }
            make("FormaItem", &kw)?
        }
        "rect" | "elipse" => {
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
