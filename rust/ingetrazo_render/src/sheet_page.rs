// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The sheet page of the Sheets workspace, native (docs/rust-sheets.md §3–4).
//! It draws the document (paper, ink, the frames' contents): its colours
//! are the drawing's, outside the chrome tokens; the selection, the desk
//! and its labels use the window's tokens.
//!
//! The page snapshot comes from the document (`ingetrazo_core` sheets::page,
//! JSON); the frames' contents come as line sets (the HLR drawing, paper mm,
//! frame-local) or as pictures. Here: the desk, the paper, its margin and
//! border, the guides, every item, the selection and its handles; zoom and
//! pan; picking, moving and resizing items and placing a view. Edits leave
//! as actions (`sheets:rects`, `sheets:place`, `sheets:select`), one per
//! gesture, which views/wgpu_sheets.py runs through the sheet's history.

use crate::ui::tokens::{ACCENT, CONTACT, DESK, FAINT, TYPE_META};
use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Stroke};
use serde_json::Value as J;
use std::collections::HashMap;

/// Points per paper millimetre at 100 % (true size on a 72-dpi point grid,
/// as the composer's `logicalDpiX() / 25.4` on macOS).
pub const PT_PER_MM: f32 = 72.0 / 25.4;
/// Snapping reach in paper mm (core.composition.snap_mm).
const SNAP_MM: f64 = 2.0;
/// Handle half-size in points.
const HANDLE: f32 = 4.0;
/// The ink of the vector pass and of text without its own colour.
const INK_ON_PAPER: Color32 = Color32::from_rgb(0x1e, 0x24, 0x2c);
/// The screen-only guide round a borderless frame and the margin.
const GUIDE_ON_PAPER: Color32 = Color32::from_rgb(0xc4, 0xca, 0xd2);

#[derive(Clone, Debug)]
pub struct Item {
    pub key: u64,
    pub kind: String,
    pub rect: [f64; 4],
    pub locked: bool,
    pub hidden: bool,
    pub editable: bool,
    pub data: J,
    /// The ink box on the page (mm) of an item drawn from its display list.
    pub bounds: Option<[f64; 4]>,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub key: u64,
    pub size: [f64; 2],
    pub margin: f64,
    pub border: J,
    /// The sheet border's display list (sheets::items_c), if any.
    pub raw_border: Option<J>,
    pub guides_v: Vec<f64>,
    pub guides_h: Vec<f64>,
    pub items: Vec<Item>,
    /// The sheet record's own fields (the inspector's "Sheet").
    pub fields: J,
    /// The lists the inspector offers (views, styles), from the document.
    pub choices: J,
}

impl Snapshot {
    pub fn parse(text: &str) -> Option<Snapshot> {
        let v: J = serde_json::from_str(text).ok()?;
        let nums = |j: &J| j.as_array().map(|a| a.iter().filter_map(J::as_f64).collect()).unwrap_or_default();
        let items = v["items"].as_array()?.iter().filter_map(|i| {
            let r = i["rect"].as_array()?;
            Some(Item {
                key: i["key"].as_u64()?,
                kind: i["kind"].as_str()?.to_string(),
                rect: [r.first()?.as_f64()?, r.get(1)?.as_f64()?, r.get(2)?.as_f64()?, r.get(3)?.as_f64()?],
                locked: i["locked"].as_bool().unwrap_or(false),
                hidden: i["hidden"].as_bool().unwrap_or(false),
                editable: i["editable"].as_bool().unwrap_or(false),
                bounds: i["bounds"].as_array().filter(|b| b.len() == 4)
                    .map(|b| [b[0].as_f64().unwrap_or(0.0), b[1].as_f64().unwrap_or(0.0),
                              b[2].as_f64().unwrap_or(0.0), b[3].as_f64().unwrap_or(0.0)]),
                data: i.clone(),
            })
        }).collect();
        Some(Snapshot {
            key: v["key"].as_u64().unwrap_or(0),
            size: [v["size"][0].as_f64()?, v["size"][1].as_f64()?],
            margin: v["margin"].as_f64().unwrap_or(10.0),
            border: v["border"].clone(),
            raw_border: v["border_display"].as_array().filter(|a| !a.is_empty()).map(|_| v["border_display"].clone()),
            guides_v: nums(&v["guides"]["v"]),
            guides_h: nums(&v["guides"]["h"]),
            fields: v["fields"].clone(),
            choices: v["choices"].clone(),
            items,
        })
    }
}

/// The tools a click or a drag on the page places an item with
/// (ingetrazo_core sheets::create PLACE_TOOLS).
pub const PLACE_TOOLS: [&str; 16] = ["vista", "texto", "llamada", "cajetin", "escala", "norte", "leyenda", "perfil",
                                     "linea", "flecha", "terreno", "rect", "elipse", "poligono", "etiqueta", "nivel"];
/// The placement tools a click (press and release on one spot) does not
/// finish: the second click does (the composer's `TOOLS` drag flag).
pub const TWO_POINT_TOOLS: [&str; 10] = ["vista", "etiqueta", "llamada", "perfil", "linea", "flecha", "terreno", "rect",
                                         "elipse", "poligono"];
/// The placement tools whose points snap to the drawing.
pub const SNAP_PLACE_TOOLS: [&str; 8] = ["linea", "flecha", "terreno", "rect", "elipse", "poligono", "etiqueta", "nivel"];

#[derive(Clone, Debug, PartialEq)]
enum Drag {
    /// Moving the selection: where it was grabbed (mm) and the rects it started from.
    Move { grab: [f64; 2], from: Vec<(u64, [f64; 4])> },
    /// One item by a handle (0..8 clockwise from top-left).
    Resize { key: u64, handle: u8, grab: [f64; 2], from: [f64; 4] },
    /// Preview a linear dimension's paper offset, keeping its two model anchors.
    DimensionLine { key: u64, grab: [f64; 2], original: f64, offset: f64, dot: Option<[f64; 2]> },
    /// The View tool's rectangle.
    Place { from: [f64; 2], to: [f64; 2], hit: Option<crate::sheet_tools::Pt>, press: Pos2, waiting: bool },
    /// The hand.
    Pan { last: Pos2 },
    /// A press that has not moved yet (a click selects; moving drags).
    Pending { at: Pos2, key: Option<u64>, add: bool },
}

#[derive(Default)]
pub struct Page {
    pub doc: Option<Snapshot>,
    raw: String,
    /// Points per paper mm, and where the paper's origin is on screen.
    zoom: f32,
    origin: Pos2,
    fit: Option<bool>,
    zoom_to: Option<f32>,
    /// A vector frame's drawing as a display list (frame-local mm).
    ops: HashMap<u64, J>,
    /// A frame's model annotations (sheets::annots), over its content
    /// whatever its style.
    annots: HashMap<u64, J>,
    /// Frames rendered on the GPU straight into a texture (no read-back):
    /// the texture kept alive and its id with the UI painter.
    natives: HashMap<u64, (wgpu::Texture, egui::TextureId)>,
    /// Native ids replaced or dropped, for the UI to free after a frame.
    pub freed: Vec<egui::TextureId>,
    pub selection: Vec<u64>,
    hover: Option<u64>,
    drag: Option<Drag>,
    /// Rects a gesture left, shown until the document says the same.
    preview: HashMap<u64, [f64; 4]>,
    /// Where each text block was laid out last (mm): what a click hits.
    text_h: HashMap<u64, f64>,
    /// The pictures image items show, loaded from their files.
    pictures: crate::sheet_display::Pictures,
    /// The tool from the rail (the composer's mode names).
    pub tool: String,
    /// The cursor the page asks for.
    pub cursor: &'static str,
    /// Where the pointer is on the paper, for the bottom bar.
    pub pointer_mm: Option<[f64; 2]>,
    /// The look Copy style took: the kind and its fields.
    style_clip: Option<(String, serde_json::Map<String, J>)>,
    /// A passing word for the bottom bar (style copied, grouped…).
    flash: Option<(&'static str, std::time::Instant)>,
    /// A text item being typed in on the page: its key and the words so far.
    pub editing: Option<(u64, String)>,
    /// The editor wants the keyboard (it just opened).
    pub edit_focus: bool,
    /// What each frame offers the drawing tools to snap to.
    pub snaps: HashMap<u64, crate::sheet_tools::FrameSnaps>,
    /// The clicks a snapping tool has gathered, and the snap under the cursor.
    pub pick: Option<crate::sheet_tools::Pick>,
    hot: Option<crate::sheet_tools::Pt>,
    /// How far apart a baseline run's rows sit (the document's dimension style).
    pub run_step: f64,
    /// The format painter takes a style on its next click (else pastes).
    pub painter_armed: bool,
}

fn snap(value: f64, targets: &[f64]) -> Option<f64> {
    let mut best: Option<f64> = None;
    for &t in targets {
        let d = (value - t).abs();
        if d <= SNAP_MM && best.is_none_or(|b| d < (value - b).abs()) {
            best = Some(t);
        }
    }
    best
}

impl Page {
    // ---- What Python sends ------------------------------------------------------
    /// A new snapshot; the first one (or another sheet) fits the page.
    pub fn set(&mut self, text: String) {
        if text == self.raw {
            return;
        }
        let Some(doc) = Snapshot::parse(&text) else { return };
        let other = self.doc.as_ref().is_none_or(|d| d.key != doc.key || d.size != doc.size);
        if other {
            self.fit = Some(false);
            self.drag = None;
            self.selection.clear();
            self.preview.clear();
        }
        // A gesture's rects stay until the document agrees (or moved on).
        self.preview.retain(|k, r| doc.items.iter().any(|i| i.key == *k && i.rect != *r));
        let keys: Vec<u64> = doc.items.iter().map(|i| i.key).collect();
        self.selection.retain(|k| keys.contains(k));
        self.ops.retain(|k, _| keys.contains(k));
        self.snaps.retain(|k, _| keys.contains(k));
        let gone: Vec<u64> = self.natives.keys().filter(|k| !keys.contains(k)).copied().collect();
        for k in gone {
            self.drop_native(k);
        }
        self.raw = text;
        self.doc = Some(doc);
    }
    /// A snapping tool's pointer: the snap under the cursor, and a click
    /// taken (snapped when it caught the model). The linear dimension takes
    /// two points (⇧ on the second puts it on a row or a column), then
    /// the click that sets its offset makes it.
    /// The page's linear dimensions' lines (a new one's offset lines up on them).
    fn cota_lines(&self, exclude: Option<u64>) -> Vec<([f64; 2], [f64; 2])> {
        let Some(doc) = self.doc.as_ref() else { return Vec::new() };
        doc.items.iter().filter(|i| i.kind == "dimension" && !i.hidden && Some(i.key) != exclude)
            .filter_map(|i| crate::sheet_tools::cota_line(&i.data["fields"])).collect()
    }
    /// The offset the cursor at `mm` gives the dimension line of a→b, onto a
    /// parallel dimension's line within ~7 px (`snap_sep_to_cotas`), and
    /// the end of the line it took.
    fn offset_at(&self, a: [f64; 2], b: [f64; 2], axis: &str, mm: [f64; 2]) -> (f64, Option<[f64; 2]>) {
        let thr = 7.0 / f64::from(self.zoom.max(1e-6));
        crate::sheet_tools::snap_sep(a, b, axis, mm, &self.cota_lines(None), thr)
    }
    /// The one dimension selected, unlocked: where a chain or a baseline
    /// run carries on from (`_seed_run_from`, AutoCAD's DIMCONTINUE and
    /// DIMBASELINE) — its points, its line's offset, its axis.
    fn run_seed(&self) -> Option<([f64; 2], [f64; 2], f64, String)> {
        let [k] = self.selection.as_slice() else { return None };
        let i = self.item(*k).filter(|i| i.kind == "dimension" && !i.locked)?;
        let f = &i.data["fields"];
        let n = |k: &str| f.get(k).and_then(J::as_f64).unwrap_or(0.0);
        let (x, y) = (n("x_mm"), n("y_mm"));
        Some(([x, y], [x + n("dx_mm"), y + n("dy_mm")], n("sep_mm"),
              f.get("axis").and_then(J::as_str).unwrap_or("").to_string()))
    }

    fn pick_pointer(&mut self, kind: &str, p: Pos2, mods: u8, out: &mut Vec<String>) {
        use crate::sheet_tools::{snap_at, Pt};
        if kind == "release" {
            return;                       // a click is its press; the release adds nothing
        }
        let mm = self.to_mm(p);
        let thr = 7.0 / f64::from(self.zoom.max(1e-6));
        let seed = if matches!(kind, "press" | "double") { self.run_seed() } else { None };
        let pick = self.pick.get_or_insert_with(|| crate::sheet_tools::Pick { tool: self.tool.clone(), ..Default::default() });
        if pick.tool != self.tool {
            *pick = crate::sheet_tools::Pick { tool: self.tool.clone(), ..Default::default() };
        }
        let offsetting = match pick.tool.as_str() {
            "cota_ang" => pick.pts.len() == 3,
            "cota_radio" => false,
            _ => pick.sep.is_none() && pick.pts.len() == 2,
        };
        let sep_dot = match self.pick.as_ref() {
            Some(pk) if offsetting && pk.tool != "cota_ang" => self.offset_at(pk.pts[0].p, pk.pts[1].p, &pk.axis, mm).1,
            _ => None,
        };
        // Offsetting, the dot shows the parallel dimension the line lines up on.
        self.hot = if offsetting { sep_dot.map(Pt::free) } else { snap_at(&self.snaps, mm, thr) };
        self.cursor = "crosshair";
        if !matches!(kind, "press" | "double") {
            return;
        }
        let mut pt = self.hot.clone().unwrap_or_else(|| Pt::free(mm));
        let sep_here = match self.pick.as_ref() {
            Some(pk) if pk.pts.len() >= 2 => self.offset_at(pk.pts[0].p, pk.pts[1].p, &pk.axis, mm).0,
            _ => 0.0,
        };
        let Some(pick) = self.pick.as_mut() else { return };
        if matches!(pick.tool.as_str(), "cota_cadena" | "cota_base") && pick.pts.is_empty() {
            if let Some((a, b, sep, axis)) = seed {
                // Carrying on from the selected dimension: this click is
                // already the run's next point.
                pick.pts = vec![Pt::free(a), Pt::free(b)];
                pick.sep = Some(sep);
                pick.axis = axis;
                pick.placed = 1;
                if offsetting || self.hot.is_none() {
                    self.hot = snap_at(&self.snaps, mm, thr);
                }
                pt = self.hot.clone().unwrap_or_else(|| Pt::free(mm));
            }
        }
        if pick.tool == "cota_ang" && mods & 4 != 0 && !pick.pts.is_empty() && pick.pts.len() < 3 {
            pt = Pt::free(crate::sheet_tools::ang_step(pt.p, &pick.pts));
        }
        match (pick.tool.as_str(), pick.pts.len()) {
            ("cota_ang", n) if n < 3 => pick.pts.push(pt),
            ("cota_ang", _) => {
                let v = pick.pts[0].p;
                let r = (mm[0] - v[0]).hypot(mm[1] - v[1]);
                let body = serde_json::json!({"v": v, "a": pick.pts[1].p, "b": pick.pts[2].p, "r": r});
                out.push(format!("sheets:angular\u{1f}{body}"));
                self.pick = None;
                self.hot = None;
            }
            ("cota_radio", n) => {
                // One click on the arc: the circle lends its centre and its
                // radius, the click says which way the line leaves. Else the
                // centre, then a point on the arc. ⌘ makes it a diameter.
                let kind = if mods & 2 != 0 { "diameter" } else { "radius" };
                let ring = if n == 0 { crate::sheet_tools::circle_at(&self.snaps, mm, thr) } else { None };
                let (c, q) = if let Some([cx, cy, r]) = ring {
                    let a = (mm[1] - cy).atan2(mm[0] - cx);
                    ([cx, cy], [cx + r * a.cos(), cy + r * a.sin()])
                } else if n == 0 {
                    pick.pts.push(pt);
                    return;
                } else {
                    (pick.pts[0].p, pt.p)
                };
                let body = serde_json::json!({"c": c, "p": q, "kind": kind});
                out.push(format!("sheets:radial\u{1f}{body}"));
                self.pick = None;
                self.hot = None;
            }
            ("cota", 0) => pick.pts.push(pt),
            ("cota", 1) => {
                let a = pick.pts[0].p;
                pick.axis = if mods & 4 != 0 {
                    if (pt.p[0] - a[0]).abs() >= (pt.p[1] - a[1]).abs() { "h".into() } else { "v".into() }
                } else {
                    String::new()
                };
                pick.pts.push(pt);
            }
            ("cota", _) => {
                let (a, b) = (&pick.pts[0], &pick.pts[1]);
                let sep = sep_here;
                let body = serde_json::json!({"a": a.json(), "b": b.json(), "sep": sep, "axis": pick.axis});
                out.push(format!("sheets:cota\u{1f}{body}"));
                self.pick = None;
                self.hot = None;
            }
            (_, n) => {
                // A chain or a baseline. A click back on the last point ends it.
                let near = 4.0 / f64::from(self.zoom.max(1e-6));
                if n > 0 && (pt.p[0] - pick.pts[n - 1].p[0]).abs() + (pt.p[1] - pick.pts[n - 1].p[1]).abs() < near {
                    self.finish_run(out);
                    return;
                }
                if n < 2 {
                    if n == 1 && mods & 4 != 0 {
                        pick.axis = crate::sheet_tools::axis_out(pick.pts[0].p, pt.p, pt.p, "");
                    }
                    pick.pts.push(pt);
                    return;
                }
                let one = |a: &Pt, b: &Pt, sep: f64, axis: &str| {
                    let body = serde_json::json!({"a": a.json(), "b": b.json(), "sep": sep, "axis": axis});
                    format!("sheets:cota\u{1f}{body}")
                };
                let Some(run_sep) = pick.sep else {
                    let (a, b) = (pick.pts[0].p, pick.pts[1].p);
                    let sep = if mods & 4 != 0 {
                        let was = pick.axis.clone();
                        pick.axis = crate::sheet_tools::axis_out(a, b, mm, &was);
                        if pick.axis == was { sep_here } else { crate::sheet_tools::sep_of(a, b, &pick.axis, mm) }
                    } else {
                        sep_here
                    };
                    pick.sep = Some(sep);
                    pick.placed = 1;
                    out.push(one(&pick.pts[0], &pick.pts[1], sep, &pick.axis));
                    return;
                };
                let base = pick.tool == "cota_base";
                let prev = if base { pick.pts[0].clone() } else { pick.pts[n - 1].clone() };
                let sep = if base {
                    let step = self.run_step * pick.placed as f64;
                    run_sep + if run_sep >= 0.0 { step } else { -step }
                } else {
                    crate::sheet_tools::chain_sep_for(pick.pts[0].p, pick.pts[1].p, run_sep, &pick.axis, prev.p, pt.p)
                };
                out.push(one(&prev, &pt, sep, &pick.axis));
                pick.pts.push(pt);
                pick.placed += 1;
            }
        }
    }

    /// End a chain or baseline run: a chain of two or more stacks its total
    /// (`sheets:cota_total`: first and last point, the run's line).
    fn finish_run(&mut self, out: &mut Vec<String>) {
        if let Some(pick) = self.pick.take() {
            if pick.tool == "cota_cadena" && pick.placed >= 2 {
                if let (Some(sep), Some(first), Some(last)) = (pick.sep, pick.pts.first(), pick.pts.last()) {
                    let body = serde_json::json!({"a": first.json(), "b": last.json(), "sep": sep, "axis": pick.axis,
                                                  "p1": pick.pts[0].p, "p2": pick.pts[1].p});
                    out.push(format!("sheets:cota_total\u{1f}{body}"));
                }
            }
        }
        self.hot = None;
    }

    /// The tool's clicks so far and the snap dot, over the page.
    fn paint_pick(&self, painter: &egui::Painter) {
        use crate::sheet_tools::line_ends;
        let stroke = Stroke::new(1.0_f32, ACCENT);
        if let (Some(pick), Some(cur)) = (&self.pick, self.pointer_mm) {
            let scr = |q: [f64; 2]| self.to_screen(q[0], q[1]);
            let run = pick.tool == "cota_cadena" || pick.tool == "cota_base";
            match (pick.tool.as_str(), pick.pts.len()) {
                ("cota_ang", n) if n >= 1 => {
                    let v = scr(pick.pts[0].p);
                    for q in pick.pts.iter().skip(1) {
                        painter.line_segment([v, scr(q.p)], stroke);
                    }
                    let to = self.hot.as_ref().map_or(cur, |h| h.p);
                    if n < 3 {
                        painter.extend(egui::Shape::dashed_line(&[v, scr(to)], stroke, 4.0, 3.0));
                    } else {
                        painter.circle_stroke(v, (scr(cur) - v).length(), Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.6)));
                    }
                }
                ("cota_radio", 1) => {
                    let c = scr(pick.pts[0].p);
                    let to = scr(self.hot.as_ref().map_or(cur, |h| h.p));
                    painter.line_segment([c, to], stroke);
                    painter.circle_stroke(c, (to - c).length(), Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.6)));
                }
                (_, n) if run && n >= 2 && pick.sep.is_some() => {
                    let base = pick.tool == "cota_base";
                    let prev = if base { pick.pts[0].p } else { pick.pts[n - 1].p };
                    let to = self.hot.as_ref().map_or(cur, |h| h.p);
                    let run_sep = pick.sep.unwrap_or(0.0);
                    let sep = if base {
                        let step = self.run_step * pick.placed as f64;
                        run_sep + if run_sep >= 0.0 { step } else { -step }
                    } else {
                        crate::sheet_tools::chain_sep_for(pick.pts[0].p, pick.pts[1].p, run_sep, &pick.axis, prev, to)
                    };
                    let (a2, b2) = line_ends(prev, to, sep, &pick.axis);
                    for (p, q) in [(prev, a2), (to, b2), (a2, b2)] {
                        painter.extend(egui::Shape::dashed_line(&[scr(p), scr(q)], stroke, 4.0, 3.0));
                    }
                }
                (_, 1) => {
                    let to = self.hot.as_ref().map_or(cur, |h| h.p);
                    painter.extend(egui::Shape::dashed_line(&[scr(pick.pts[0].p), scr(to)], stroke, 4.0, 3.0));
                }
                (_, 2) => {
                    let (a, b) = (pick.pts[0].p, pick.pts[1].p);
                    let (a2, b2) = line_ends(a, b, self.offset_at(a, b, &pick.axis, cur).0, &pick.axis);
                    for (p, q) in [(a, a2), (b, b2), (a2, b2)] {
                        painter.extend(egui::Shape::dashed_line(&[scr(p), scr(q)], stroke, 4.0, 3.0));
                    }
                }
                _ => {}
            }
        }
        if let Some(h) = &self.hot {
            let c = self.to_screen(h.p[0], h.p[1]);
            if h.segment.is_some() {
                painter.circle_stroke(c, 4.5, Stroke::new(1.5_f32, ACCENT));
            } else {
                painter.circle_filled(c, 4.5, ACCENT);
            }
        }
    }

    /// Another tool armed: what the last one gathered is dropped; the
    /// format painter starts by taking a style.
    pub fn set_tool(&mut self, tool: String) {
        self.pick = None;
        self.hot = None;
        if matches!(self.drag, Some(Drag::Place { .. } | Drag::DimensionLine { .. })) {
            self.drag = None;
        }
        self.painter_armed = tool == "estilo";
        self.tool = tool;
    }
    /// Where a placement tool's point lands: on the drawing for the tools
    /// that snap to it (the dot shows), level or plumb from `from` with ⇧
    /// for the lines.
    fn place_point(&mut self, mm: [f64; 2], from: Option<[f64; 2]>, mods: u8) -> crate::sheet_tools::Pt {
        let tool = self.tool.as_str();
        let thr = 7.0 / f64::from(self.zoom.max(1e-6));
        self.hot = if SNAP_PLACE_TOOLS.contains(&tool) { crate::sheet_tools::snap_at(&self.snaps, mm, thr) } else { None };
        let mut pt = self.hot.clone().unwrap_or_else(|| crate::sheet_tools::Pt::free(mm));
        if let (Some(a), true) = (from, mods & 4 != 0 && matches!(tool, "linea" | "flecha" | "terreno")) {
            pt = if (pt.p[0] - a[0]).abs() >= (pt.p[1] - a[1]).abs() {
                crate::sheet_tools::Pt::free([pt.p[0], a[1]])
            } else {
                crate::sheet_tools::Pt::free([a[0], pt.p[1]])
            };
        }
        pt
    }
    /// `sheets:place␟x0␟y0␟x1␟y1␟tool[␟first point's catch]`.
    fn place_action(&self, from: [f64; 2], to: [f64; 2], hit: Option<&crate::sheet_tools::Pt>) -> String {
        let mut a = format!("sheets:place\u{1f}{:.3}\u{1f}{:.3}\u{1f}{:.3}\u{1f}{:.3}\u{1f}{}",
                            from[0], from[1], to[0], to[1], self.tool);
        if let Some(h) = hit {
            a += &format!("\u{1f}{}", h.json());
        }
        a
    }

    /// Start typing in text item `key` (its words as the document has them).
    pub fn edit_text(&mut self, key: u64) {
        let text = self.item(key).and_then(|i| i.data["fields"]["text"].as_str().map(str::to_string));
        if let Some(t) = text {
            self.selection = vec![key];
            self.editing = Some((key, t));
            self.edit_focus = true;
        }
    }
    /// The words typed: one edit of the document, unless nothing changed.
    pub fn commit_text(&mut self, out: &mut Vec<String>) {
        if let Some((key, text)) = self.editing.take() {
            let was = self.item(key).and_then(|i| i.data["fields"]["text"].as_str().map(str::to_string));
            if was.as_deref() != Some(text.as_str()) {
                out.push(format!("sheets:set\u{1f}{key}\u{1f}{}", serde_json::json!({"text": text})));
            }
        }
    }
    /// The words over the item, at its size, on paper; leaving the field
    /// (or ⌘↩, or a press on the page) is one edit, Esc puts them back.
    pub fn text_editor(&mut self, ctx: &egui::Context, out: &mut Vec<String>) {
        let Some(key) = self.editing.as_ref().map(|e| e.0) else { return };
        let Some((rect, size)) = self.text_box(key) else {
            self.editing = None;
            return;
        };
        let focus = std::mem::take(&mut self.edit_focus);
        let (mut commit, mut cancel) = (false, false);
        egui::Area::new(egui::Id::new("sheet_text_editor"))
            .fixed_pos(rect.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let Some((_, text)) = self.editing.as_mut() else { return };
                egui::Frame::new().fill(Color32::WHITE).stroke(Stroke::new(1.0_f32, ACCENT)).show(ui, |ui| {
                    let edit = egui::TextEdit::multiline(text)
                        .font(FontId::proportional(size))
                        .text_color(INK_ON_PAPER)
                        .background_color(Color32::WHITE)
                        .frame(false)
                        .margin(egui::Margin::ZERO)
                        .desired_width(rect.width().max(60.0))
                        .desired_rows(1);
                    let resp = ui.add(edit);
                    if focus {
                        resp.request_focus();
                    }
                    let (escape, submit) = ui.input(|i| (i.key_pressed(egui::Key::Escape),
                                                         i.key_pressed(egui::Key::Enter) && i.modifiers.command));
                    if escape {
                        cancel = true;
                    } else if submit || (resp.lost_focus() && !focus) {
                        commit = true;
                    }
                });
            });
        if cancel {
            self.editing = None;
        } else if commit {
            self.commit_text(out);
        }
    }
    /// The screen box a text item is typed in, and its type size in points.
    pub fn text_box(&self, key: u64) -> Option<(Rect, f32)> {
        let i = self.item(key)?;
        let size = i.data["fields"]["size_pt"].as_f64().unwrap_or(14.0) * 25.4 / 72.0 * 0.75;
        Some((self.screen_rect(self.rect_of(i)), (size as f32 * self.zoom).max(6.0)))
    }
    pub fn clear(&mut self) {
        let mut freed = std::mem::take(&mut self.freed);
        freed.extend(self.natives.drain().map(|(_, (_, id))| id));
        *self = Page { tool: std::mem::take(&mut self.tool), freed, ..Default::default() };
    }
    fn drop_native(&mut self, key: u64) {
        if let Some((_, id)) = self.natives.remove(&key) {
            self.freed.push(id);
        }
    }
    /// A frame's picture as a GPU texture already registered with the UI
    /// painter (``id``); the one it replaces is freed after the next frame.
    pub fn set_native(&mut self, key: u64, texture: wgpu::Texture, id: egui::TextureId) {
        self.ops.remove(&key);
        if let Some((_, old)) = self.natives.insert(key, (texture, id)) {
            self.freed.push(old);
        }
    }
    /// A vector frame's drawing (sheets::items_d `paint_vector_content`).
    pub fn set_ops(&mut self, key: u64, ops: J) {
        self.drop_native(key);
        self.ops.insert(key, ops);
    }
    /// A frame's model annotations (sheets::annots `paint_annots`).
    pub fn set_annots(&mut self, key: u64, ops: J) {
        self.annots.insert(key, ops);
    }
    pub fn forget_content(&mut self, key: u64) {
        self.ops.remove(&key);
        self.annots.remove(&key);
        self.drop_native(key);
    }
    pub fn has_content(&self, key: u64) -> bool {
        self.ops.contains_key(&key) || self.natives.contains_key(&key)
    }
    pub fn set_selection(&mut self, keys: Vec<u64>) {
        self.selection = keys;
    }
    pub fn request_fit(&mut self, width_only: bool) {
        self.fit = Some(width_only);
    }
    pub fn request_zoom(&mut self, percent: f32) {
        self.zoom_to = Some(percent.clamp(1.0, 1600.0));
    }
    /// What the page does now, for the bottom bar.
    pub fn hint(&self) -> &'static str {
        if let Some((text, at)) = self.flash {
            if at.elapsed().as_secs_f32() < 3.0 {
                return text;
            }
        }
        match (&self.drag, self.tool.as_str()) {
            (Some(Drag::Move { .. }), _) => "Edges snap to the margins, guides and other items; ⌥ moves freely.",
            (Some(Drag::Resize { .. }), _) => "Edges snap to the margins, guides and other items; ⌥ resizes freely.",
            (Some(Drag::Place { waiting: true, .. }), _) => "Click the second point.",
            (Some(Drag::Place { .. }), "vista") => "Release to place the view, at the scale that fits.",
            (_, "pan") => "Drag to move the sheet. Scroll pans, ⌘-scroll zooms.",
            (_, "zoom") | (_, "zoom_ventana") => "Click to zoom in, ⌥-click to zoom out.",
            (_, "vista") => "Drag a rectangle to place a model view.",
            (_, t) if PLACE_TOOLS.contains(&t) => "Click to place it, or drag its box.",
            (_, "cota_cadena") | (_, "cota_base") => match self.pick.as_ref().map_or(0, |p| p.pts.len()) {
                0 => "Click the first point of the run: it snaps to the drawing.",
                1 => "Click the second point; ⇧ keeps the run level or plumb.",
                2 if self.pick.as_ref().is_some_and(|p| p.sep.is_none()) => "Click where the dimension line goes.",
                _ => "Click the next point. Click the last one again, or Esc, to end the run.",
            },
            (_, "cota_ang") => match self.pick.as_ref().map_or(0, |p| p.pts.len()) {
                0 => "Click the angle's vertex: it snaps to the drawing.",
                1 => "Click a point on the first arm; ⇧ steps it by 15°.",
                2 => "Click a point on the second arm; ⇧ steps the angle by 15°.",
                _ => "Click where the arc goes.",
            },
            (_, "cota_radio") => match self.pick.as_ref().map_or(0, |p| p.pts.len()) {
                0 => "Click on a circle or an arc, or its centre; ⌘ for a diameter.",
                _ => "Click a point on the arc; ⌘ for a diameter.",
            },
            (_, "cota") => match self.pick.as_ref().map_or(0, |p| p.pts.len()) {
                0 => "Click the first point: it snaps to the drawing's corners, middles and edges.",
                1 => "Click the second point; ⇧ keeps the dimension level or plumb.",
                _ => "Click where the dimension line goes. Esc starts again.",
            },
            _ if !self.selection.is_empty() => "Drag to move, the handles resize. Arrows nudge 1 mm, ⇧ 10 mm, ⌥ 0.1 mm. Esc clears.",
            _ => "Click an item to select it, ⇧-click to add. Drag to move it.",
        }
    }
    pub fn zoom_percent(&self) -> f32 {
        self.zoom / PT_PER_MM * 100.0
    }

    // ---- Paper ↔ screen ---------------------------------------------------------
    fn to_screen(&self, x: f64, y: f64) -> Pos2 {
        pos2(self.origin.x + x as f32 * self.zoom, self.origin.y + y as f32 * self.zoom)
    }
    fn to_mm(&self, p: Pos2) -> [f64; 2] {
        let z = self.zoom.max(1e-6);
        [((p.x - self.origin.x) / z) as f64, ((p.y - self.origin.y) / z) as f64]
    }
    fn screen_rect(&self, r: [f64; 4]) -> Rect {
        Rect::from_min_max(self.to_screen(r[0], r[1]), self.to_screen(r[0] + r[2], r[1] + r[3]))
    }
    /// Zoom by ``factor`` keeping the paper point under ``at`` where it is.
    pub fn zoom_about(&mut self, at: Pos2, factor: f32) {
        let mm = self.to_mm(at);
        let z = (self.zoom * factor).clamp(0.01 * PT_PER_MM, 16.0 * PT_PER_MM);
        self.zoom = z;
        self.origin = pos2(at.x - mm[0] as f32 * z, at.y - mm[1] as f32 * z);
    }
    pub fn pan_by(&mut self, dx: f32, dy: f32) {
        self.origin += vec2(dx, dy);
    }

    fn item(&self, key: u64) -> Option<&Item> {
        self.doc.as_ref()?.items.iter().find(|i| i.key == key)
    }
    /// The rect an item shows now (a gesture's preview first).
    fn rect_of(&self, i: &Item) -> [f64; 4] {
        if let Some(r) = self.preview.get(&i.key) {
            return *r;
        }
        let mut r = i.rect;
        if i.kind == "text" {
            if let Some(h) = self.text_h.get(&i.key) {
                r[3] = *h;
            }
        }
        r
    }

    /// What an item covers on the page now: its ink when its painter says,
    /// else its box (a gesture's preview moves either).
    fn ink_of(&self, i: &Item) -> [f64; 4] {
        let r = self.rect_of(i);
        match i.bounds {
            Some(b) => [b[0] + r[0] - i.rect[0], b[1] + r[1] - i.rect[1], b[2], b[3]],
            None => r,
        }
    }

    /// The topmost visible item under ``mm``.
    fn hit(&self, mm: [f64; 2]) -> Option<u64> {
        let doc = self.doc.as_ref()?;
        let pad = (3.0 / self.zoom.max(1e-6)) as f64;
        doc.items.iter().rev().filter(|i| !i.hidden).find(|i| {
            let r = self.ink_of(i);
            let (x0, x1) = (r[0].min(r[0] + r[2]) - pad, r[0].max(r[0] + r[2]) + pad);
            let (y0, y1) = (r[1].min(r[1] + r[3]) - pad, r[1].max(r[1] + r[3]) + pad);
            mm[0] >= x0 && mm[0] <= x1 && mm[1] >= y0 && mm[1] <= y1
        }).map(|i| i.key)
    }

    fn handles(&self, r: [f64; 4]) -> [Pos2; 8] {
        let s = self.screen_rect(r);
        let (x0, x1, y0, y1, xm, ym) = (s.min.x, s.max.x, s.min.y, s.max.y, s.center().x, s.center().y);
        [pos2(x0, y0), pos2(xm, y0), pos2(x1, y0), pos2(x1, ym), pos2(x1, y1), pos2(xm, y1), pos2(x0, y1), pos2(x0, ym)]
    }

    /// The handle of the single selected, editable, unlocked item under ``p``.
    fn handle_at(&self, p: Pos2) -> Option<(u64, u8)> {
        let [key] = self.selection[..] else { return None };
        let i = self.item(key)?;
        if i.locked || !i.editable {
            return None;
        }
        let only_width = i.kind == "text";
        self.handles(self.rect_of(i)).iter().enumerate()
            .filter(|(h, _)| !only_width || *h == 3 || *h == 7)
            .find(|(_, c)| (p - **c).length() <= HANDLE + 3.0)
            .map(|(h, _)| (key, h as u8))
    }

    fn dimension_handle(&self) -> Option<(u64, Pos2, Pos2)> {
        let [key] = self.selection[..] else { return None };
        let item = self.item(key)?;
        if item.kind != "dimension" || item.locked || item.hidden { return None; }
        let (a, b) = crate::sheet_tools::cota_line(&item.data["fields"])?;
        Some((key, self.to_screen(a[0], a[1]), self.to_screen(b[0], b[1])))
    }

    fn dimension_line_at(&self, p: Pos2) -> Option<u64> {
        let (key, a, b) = self.dimension_handle()?;
        let ab = b - a;
        let t = ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0);
        (p.distance(a + t * ab) <= HANDLE + 2.0).then_some(key)
    }

    fn dragged_offset(&self, key: u64, grab: [f64; 2], original: f64, p: Pos2) -> Option<(f64, Option<[f64; 2]>)> {
        let (a, b, current, axis) = self.run_seed()?;
        if self.selection != [key] || current != original { return None; }
        let [_, _, nx, ny] = crate::sheet_tools::sep_frame(a, b, &axis)?;
        let correction = original - crate::sheet_tools::sep_of(a, b, &axis, grab);
        let mm = self.to_mm(p);
        let at = [mm[0] + nx * correction, mm[1] + ny * correction];
        Some(crate::sheet_tools::snap_sep(a, b, &axis, at, &self.cota_lines(Some(key)),
                                         7.0 / f64::from(self.zoom.max(1e-6))))
    }

    fn cursor_for_handle(h: u8) -> &'static str {
        match h {
            0 | 4 => "resize_nwse",
            2 | 6 => "resize_nesw",
            1 | 5 => "resize_v",
            _ => "resize_h",
        }
    }

    /// Edges the moving rects snap to: the paper, its margin, the guides
    /// and the other items' edges and centres.
    fn targets(&self, moving: &[u64]) -> (Vec<f64>, Vec<f64>) {
        let Some(doc) = self.doc.as_ref() else { return (vec![], vec![]) };
        let (w, h, m) = (doc.size[0], doc.size[1], doc.margin);
        let mut xs = vec![0.0, m, w / 2.0, w - m, w];
        let mut ys = vec![0.0, m, h / 2.0, h - m, h];
        xs.extend(&doc.guides_v);
        ys.extend(&doc.guides_h);
        for i in doc.items.iter().filter(|i| !i.hidden && !moving.contains(&i.key)) {
            let r = self.rect_of(i);
            xs.extend([r[0], r[0] + r[2] / 2.0, r[0] + r[2]]);
            ys.extend([r[1], r[1] + r[3] / 2.0, r[1] + r[3]]);
        }
        (xs, ys)
    }

    // ---- Input --------------------------------------------------------------------
    /// A pointer event on the page (what the chrome did not take): kind
    /// "move" | "press" | "release" | "leave" | "double", logical points.
    pub fn pointer(&mut self, kind: &str, p: Pos2, button: u8, mods: u8, out: &mut Vec<String>) {
        let shift = mods & 4 != 0;
        let alt = mods & 1 != 0;
        self.pointer_mm = (kind != "leave").then(|| self.to_mm(p));
        if self.doc.is_none() {
            return;
        }
        if matches!(kind, "press" | "double") && self.editing.is_some() {
            // A press elsewhere on the page ends the typing.
            self.commit_text(out);
        }
        if crate::sheet_tools::PICK_TOOLS.contains(&self.tool.as_str()) && kind != "leave"
            && (kind == "move" || button == 1) && !matches!(self.drag, Some(Drag::Pan { .. })) {
            self.pick_pointer(kind, p, mods, out);
            return;
        }
        let tool = self.tool.as_str();
        match kind {
            "leave" => {
                self.hover = None;
                self.cursor = "";
            }
            "press" | "double" => {
                if kind == "double" && button == 1 && !PLACE_TOOLS.contains(&tool) {
                    // A text block: typed in where it is.
                    let mm = self.to_mm(p);
                    if let Some(k) = self.hit(mm) {
                        if self.item(k).is_some_and(|i| i.kind == "text" && !i.locked) {
                            self.edit_text(k);
                            return;
                        }
                    }
                }
                if button == 3 || (tool == "pan" && button == 1) {
                    self.drag = Some(Drag::Pan { last: p });
                    self.cursor = "grabbing";
                    return;
                }
                if button != 1 {
                    return;
                }
                if tool == "zoom" || tool == "zoom_ventana" {
                    self.zoom_about(p, if alt { 1.0 / 1.5 } else { 1.5 });
                    return;
                }
                let mm = self.to_mm(p);
                if tool == "estilo" {
                    // The format painter: the first click takes an item's
                    // style, the next ones paste it on items of its kind.
                    if let Some(k) = self.hit(mm) {
                        self.selection = vec![k];
                        let verb = if self.painter_armed { "copy_style" } else { "paste_style" };
                        if self.command(verb, out) && verb == "copy_style" {
                            self.painter_armed = false;
                        }
                    }
                    return;
                }
                if PLACE_TOOLS.contains(&tool) {
                    let snapped = self.place_point(mm, None, mods);
                    if let Some(Drag::Place { from, hit, waiting: true, .. }) = self.drag.take() {
                        // The second click of click-move-click.
                        let to = self.place_point(mm, Some(from), mods);
                        out.push(self.place_action(from, to.p, hit.as_ref()));
                        self.hot = None;
                        return;
                    }
                    self.drag = Some(Drag::Place { from: snapped.p, to: snapped.p, hit: snapped.world.is_some().then_some(snapped),
                                                   press: p, waiting: false });
                    return;
                }
                if let Some(key) = self.dimension_line_at(p) {
                    let original = self.item(key).unwrap().data["fields"]["sep_mm"].as_f64().unwrap_or(0.0);
                    self.drag = Some(Drag::DimensionLine { key, grab: mm, original, offset: original, dot: None });
                    self.cursor = "move";
                    return;
                }
                if let Some((key, handle)) = self.handle_at(p) {
                    let from = self.item(key).map(|i| self.rect_of(i)).unwrap_or_default();
                    self.drag = Some(Drag::Resize { key, handle, grab: mm, from });
                    return;
                }
                let key = self.hit(mm);
                self.drag = Some(Drag::Pending { at: p, key, add: shift });
                if let Some(k) = key {
                    if !self.selection.contains(&k) && !shift {
                        self.selection = self.with_groups(vec![k]);
                        out.push(self.select_action());
                    }
                }
            }
            "move" => match self.drag.clone() {
                Some(Drag::Pan { last }) => {
                    self.pan_by(p.x - last.x, p.y - last.y);
                    self.drag = Some(Drag::Pan { last: p });
                }
                Some(Drag::Pending { at, key: Some(key), add: false }) if (p - at).length() > 3.0 => {
                    let movable: Vec<(u64, [f64; 4])> = self.selection.iter()
                        .filter_map(|k| self.item(*k))
                        .filter(|i| !i.locked && i.editable)
                        .map(|i| (i.key, self.rect_of(i))).collect();
                    if movable.iter().any(|(k, _)| *k == key) {
                        self.drag = Some(Drag::Move { grab: self.to_mm(at), from: movable });
                        self.pointer("move", p, button, mods, out);
                    } else {
                        self.drag = None;
                    }
                }
                Some(Drag::Move { grab, from }) => {
                    let mm = self.to_mm(p);
                    let (mut dx, mut dy) = (mm[0] - grab[0], mm[1] - grab[1]);
                    if !alt {
                        let keys: Vec<u64> = from.iter().map(|f| f.0).collect();
                        let (xs, ys) = self.targets(&keys);
                        let (x0, y0) = (from.iter().map(|f| f.1[0]).fold(f64::INFINITY, f64::min),
                                        from.iter().map(|f| f.1[1]).fold(f64::INFINITY, f64::min));
                        let (x1, y1) = (from.iter().map(|f| f.1[0] + f.1[2]).fold(f64::NEG_INFINITY, f64::max),
                                        from.iter().map(|f| f.1[1] + f.1[3]).fold(f64::NEG_INFINITY, f64::max));
                        let best = |edges: [f64; 3], targets: &[f64]| -> Option<f64> {
                            edges.iter().filter_map(|e| snap(*e, targets).map(|t| t - e))
                                .min_by(|a, b| a.abs().partial_cmp(&b.abs()).unwrap_or(std::cmp::Ordering::Equal))
                        };
                        if let Some(s) = best([x0 + dx, (x0 + x1) / 2.0 + dx, x1 + dx], &xs) { dx += s; }
                        if let Some(s) = best([y0 + dy, (y0 + y1) / 2.0 + dy, y1 + dy], &ys) { dy += s; }
                    }
                    for (k, r) in &from {
                        self.preview.insert(*k, [r[0] + dx, r[1] + dy, r[2], r[3]]);
                    }
                    self.cursor = "move";
                }
                Some(Drag::DimensionLine { key, grab, original, .. }) => {
                    self.drag = self.dragged_offset(key, grab, original, p).map(|(offset, dot)|
                        Drag::DimensionLine { key, grab, original, offset, dot });
                }
                Some(Drag::Resize { key, handle, grab, from }) => {
                    let mm = self.to_mm(p);
                    let (dx, dy) = (mm[0] - grab[0], mm[1] - grab[1]);
                    let [mut x0, mut y0, w, h] = from;
                    let (mut x1, mut y1) = (x0 + w, y0 + h);
                    let (xs, ys) = self.targets(&[key]);
                    let sx = |v: f64| if alt { v } else { snap(v, &xs).unwrap_or(v) };
                    let sy = |v: f64| if alt { v } else { snap(v, &ys).unwrap_or(v) };
                    if matches!(handle, 0 | 6 | 7) { x0 = sx(x0 + dx).min(x1 - 1.0); }
                    if matches!(handle, 2 | 3 | 4) { x1 = sx(x1 + dx).max(x0 + 1.0); }
                    if matches!(handle, 0 | 1 | 2) { y0 = sy(y0 + dy).min(y1 - 1.0); }
                    if matches!(handle, 4 | 5 | 6) { y1 = sy(y1 + dy).max(y0 + 1.0); }
                    self.preview.insert(key, [x0, y0, x1 - x0, y1 - y0]);
                    self.cursor = Self::cursor_for_handle(handle);
                }
                Some(Drag::Place { from, hit, press, waiting, .. }) => {
                    let to = self.place_point(self.to_mm(p), Some(from), mods).p;
                    self.drag = Some(Drag::Place { from, to, hit, press, waiting });
                    self.cursor = "crosshair";
                }
                None if SNAP_PLACE_TOOLS.contains(&tool) => {
                    self.place_point(self.to_mm(p), None, mods);
                    self.cursor = "crosshair";
                }
                _ => {
                    self.hover = self.hit(self.to_mm(p));
                    self.cursor = match (tool, self.handle_at(p)) {
                        ("pan", _) => "grab",
                        (t, _) if PLACE_TOOLS.contains(&t) => "crosshair",
                        ("zoom" | "zoom_ventana", _) => "zoom",
                        _ if self.dimension_line_at(p).is_some() => "move",
                        (_, Some((_, h))) => Self::cursor_for_handle(h),
                        _ if self.hover.is_some_and(|k| self.selection.contains(&k)) => "move",
                        _ => "",
                    };
                }
            },
            "release" => {
                match self.drag.take() {
                    Some(Drag::Pending { key, add, .. }) => {
                        let before = self.selection.clone();
                        match key {
                            Some(k) if add => {
                                if let Some(at) = self.selection.iter().position(|s| *s == k) {
                                    self.selection.remove(at);
                                } else {
                                    self.selection.push(k);
                                }
                            }
                            Some(k) => self.selection = self.with_groups(vec![k]),
                            None if !add => self.selection.clear(),
                            None => {}
                        }
                        if self.selection != before {
                            out.push(self.select_action());
                        }
                    }
                    Some(Drag::Move { from, .. }) => {
                        let moved: Vec<(u64, [f64; 4])> = from.iter()
                            .filter_map(|(k, r)| self.preview.get(k).filter(|n| *n != r).map(|n| (*k, *n))).collect();
                        if !moved.is_empty() {
                            out.push(Self::rects_action(&moved));
                        }
                    }
                    Some(Drag::DimensionLine { key, grab, original, .. }) => {
                        if p.distance(self.to_screen(grab[0], grab[1])) > 0.5 {
                            if let Some((offset, _)) = self.dragged_offset(key, grab, original, p) {
                                if (offset - original).abs() > 1e-8 {
                                    if let Some(action) = self.doc.as_ref().and_then(|doc|
                                        crate::sheet_inspector::edit(doc, &format!("{key}:sep_mm"), &offset.to_string())) {
                                        out.push(format!("sheets:{action}"));
                                    }
                                }
                            }
                        }
                    }
                    Some(Drag::Resize { key, from, .. }) => {
                        if let Some(r) = self.preview.get(&key).copied().filter(|r| *r != from) {
                            out.push(Self::rects_action(&[(key, r)]));
                        }
                    }
                    Some(Drag::Place { from, to, hit, press, waiting }) => {
                        if !waiting && TWO_POINT_TOOLS.contains(&self.tool.as_str()) && (p - press).length() < 4.0 {
                            // A click, not a drag: the second click finishes it.
                            self.drag = Some(Drag::Place { from, to, hit, press, waiting: true });
                            return;
                        }
                        out.push(self.place_action(from, to, hit.as_ref()));
                        self.hot = None;
                    }
                    _ => {}
                }
                self.cursor = "";
            }
            _ => {}
        }
    }

    /// Arrows nudge the selection by 1 mm (⇧ 10 mm, ⌥ 0.1 mm); Escape clears it.
    pub fn key(&mut self, name: &str, mods: u8, out: &mut Vec<String>) -> bool {
        let step = if mods & 4 != 0 { 10.0 } else if mods & 1 != 0 { 0.1 } else { 1.0 };
        let (dx, dy) = match name {
            "ArrowLeft" => (-step, 0.0),
            "ArrowRight" => (step, 0.0),
            "ArrowUp" => (0.0, -step),
            "ArrowDown" => (0.0, step),
            "Escape" => {
                if self.pick.as_ref().is_some_and(|p| p.placed > 0) {
                    self.finish_run(out);
                    return true;
                }
                if self.pick.take().is_some() {
                    self.hot = None;
                    return true;
                }
                if self.drag.take().is_some() {
                    self.preview.clear();
                    return true;
                }
                if !self.selection.is_empty() {
                    self.selection.clear();
                    out.push(self.select_action());
                    return true;
                }
                return false;
            }
            _ => return false,
        };
        let moved: Vec<(u64, [f64; 4])> = self.selection.iter().filter_map(|k| self.item(*k))
            .filter(|i| !i.locked && i.editable)
            .map(|i| { let r = self.rect_of(i); (i.key, [r[0] + dx, r[1] + dy, r[2], r[3]]) }).collect();
        if moved.is_empty() {
            return false;
        }
        for (k, r) in &moved {
            self.preview.insert(*k, *r);
        }
        out.push(Self::rects_action(&moved));
        true
    }

    /// A fresh group id (32 hex digits).
    fn new_group_id() -> String {
        use std::hash::{BuildHasher, Hasher};
        let mut out = String::new();
        for i in 0..2u64 {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u64(i);
            h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
            out.push_str(&format!("{:016x}", h.finish()));
        }
        out
    }

    fn set_many(edits: &[(u64, J)]) -> String {
        let parts: Vec<String> = edits.iter().map(|(k, v)| format!("{k}\u{1f}{v}")).collect();
        format!("sheets:set_many\u{1f}{}", parts.join("\u{1f}"))
    }

    /// The selection's sheet commands: ``group``, ``ungroup``,
    /// ``copy_style``, ``paste_style``. Edits leave as one ``set_many``.
    pub fn command(&mut self, name: &str, out: &mut Vec<String>) -> bool {
        let Some(doc) = self.doc.as_ref() else { return false };
        let selected: Vec<&Item> = self.selection.iter().filter_map(|k| doc.items.iter().find(|i| i.key == *k)).collect();
        let field = |i: &Item, f: &str| i.data["fields"][f].clone();
        match name {
            "edit_text" => {
                if let [item] = selected.as_slice() {
                    if !item.locked && item.kind == "text" { self.edit_text(item.key); }
                }
            }
            "lock" if !selected.is_empty() => {
                let on = !selected.iter().all(|i| i.locked);
                out.push(Self::set_many(&selected.iter().filter(|i| i.locked != on)
                    .map(|i| (i.key, serde_json::json!({"locked": on}))).collect::<Vec<_>>()));
            }
            "group" => {
                let members: Vec<&Item> = selected.iter().copied().filter(|i| i.kind != "titleblock").collect();
                if members.len() < 2 {
                    self.flash = Some(("Select two or more items to group them.", std::time::Instant::now()));
                    return true;
                }
                let gid = Self::new_group_id();
                out.push(Self::set_many(&members.iter().map(|i| (i.key, serde_json::json!({"group_id": gid}))).collect::<Vec<_>>()));
                self.flash = Some(("Grouped: they select and move together.", std::time::Instant::now()));
            }
            "ungroup" => {
                let gids: Vec<J> = selected.iter().map(|i| field(i, "group_id")).filter(|g| g.as_str().is_some_and(|s| !s.is_empty())).collect();
                if gids.is_empty() {
                    return true;
                }
                let members: Vec<(u64, J)> = doc.items.iter().filter(|i| gids.contains(&field(i, "group_id")))
                    .map(|i| (i.key, serde_json::json!({"group_id": ""}))).collect();
                out.push(Self::set_many(&members));
                self.flash = Some(("Ungrouped.", std::time::Instant::now()));
            }
            "copy_style" => {
                let Some(i) = selected.first() else { return false };
                let fields = crate::sheet_inspector::style_fields(&i.kind);
                if fields.is_empty() {
                    self.flash = Some(("This item has no style to copy.", std::time::Instant::now()));
                    return true;
                }
                let look = fields.iter().filter(|f| i.data["fields"].get(**f).is_some())
                    .map(|f| (f.to_string(), field(i, f))).collect();
                self.style_clip = Some((i.kind.clone(), look));
                self.flash = Some(("Style copied: select items of the same kind and paste it.", std::time::Instant::now()));
            }
            "paste_style" => {
                let Some((kind, look)) = &self.style_clip else { return false };
                let edits: Vec<(u64, J)> = selected.iter().filter(|i| &i.kind == kind && !i.locked).filter_map(|i| {
                    let changes: serde_json::Map<String, J> = look.iter()
                        .filter(|(k, v)| &i.data["fields"][k.as_str()] != *v).map(|(k, v)| (k.clone(), v.clone())).collect();
                    (!changes.is_empty()).then(|| (i.key, J::Object(changes)))
                }).collect();
                if edits.is_empty() {
                    if selected.iter().all(|i| &i.kind != kind) && !selected.is_empty() {
                        self.flash = Some(("That item is of another kind: the style does not apply.", std::time::Instant::now()));
                    }
                    return true;
                }
                out.push(Self::set_many(&edits));
            }
            _ => return false,
        }
        true
    }

    /// Right-click keeps an existing selection or selects the item under the pointer.
    pub fn context_at(&mut self, at: Pos2, out: &mut Vec<String>) -> Option<(String, Vec<crate::verbs::Row>)> {
        let hit = self.hit(self.to_mm(at));
        if !hit.is_some_and(|k| self.selection.contains(&k)) {
            self.selection = self.with_groups(hit.into_iter().collect());
            out.push(self.select_action());
        }
        Some(crate::verbs::sheet_menu(self.doc.as_ref()?, &self.selection,
                                      self.style_clip.as_ref().map(|(kind, _)| kind.as_str())))
    }

    /// The selection with each picked item's group around it.
    fn with_groups(&self, keys: Vec<u64>) -> Vec<u64> {
        let Some(doc) = self.doc.as_ref() else { return keys };
        let gids: Vec<&str> = keys.iter().filter_map(|k| doc.items.iter().find(|i| i.key == *k))
            .filter_map(|i| i.data["fields"]["group_id"].as_str()).filter(|g| !g.is_empty()).collect();
        let mut out = keys.clone();
        for i in &doc.items {
            if !out.contains(&i.key) && i.data["fields"]["group_id"].as_str().is_some_and(|g| gids.contains(&g)) {
                out.push(i.key);
            }
        }
        out
    }

    fn select_action(&self) -> String {
        let keys: Vec<String> = self.selection.iter().map(u64::to_string).collect();
        format!("sheets:select\u{1f}{}", keys.join(","))
    }

    fn rects_action(rects: &[(u64, [f64; 4])]) -> String {
        let parts: Vec<String> = rects.iter()
            .map(|(k, r)| format!("{k}:{:.4}:{:.4}:{:.4}:{:.4}", r[0], r[1], r[2], r[3])).collect();
        format!("sheets:rects\u{1f}{}", parts.join(";"))
    }

    // ---- Painting -----------------------------------------------------------------
    /// The page under the chrome; ``free`` is the part of the window the
    /// chrome leaves it (fit and zoom work in it).
    pub fn paint(&mut self, ctx: &egui::Context, screen: Rect, free: Rect) {
        let painter = ctx.layer_painter(egui::LayerId::background());
        painter.rect_filled(screen, 0.0, DESK);
        let Some(doc) = self.doc.clone() else { return };
        let (pw, ph) = (doc.size[0] as f32, doc.size[1] as f32);
        if let Some(width_only) = self.fit.take() {
            let room = free.shrink(16.0);
            let mut z = room.width() / pw.max(1.0);
            if !width_only {
                z = z.min(room.height() / ph.max(1.0));
            }
            self.zoom = z.max(1e-3);
            let top = if width_only { room.top() } else { room.center().y - ph * self.zoom / 2.0 };
            self.origin = pos2(room.center().x - pw * self.zoom / 2.0, top);
        }
        if let Some(pct) = self.zoom_to.take() {
            let factor = pct / 100.0 * PT_PER_MM / self.zoom.max(1e-6);
            self.zoom_about(free.center(), factor);
        }
        let paper = self.screen_rect([0.0, 0.0, doc.size[0], doc.size[1]]);
        painter.rect_filled(paper.translate(vec2(0.0, 2.0)).expand(1.0), 2.0, CONTACT);
        painter.rect_filled(paper, 0.0, Color32::WHITE);
        let clip = paper.intersect(screen);
        let painter = painter.with_clip_rect(clip);
        let m = doc.margin;
        dashed_rect(&painter, self.screen_rect([m, m, doc.size[0] - 2.0 * m, doc.size[1] - 2.0 * m]), GUIDE_ON_PAPER);
        // The sheet's border: its display list (sheets::items_c).
        if let Some(ops) = doc.raw_border.as_ref() {
            let place = crate::sheet_display::Place { origin: self.to_screen(0.0, 0.0), zoom: self.zoom };
            crate::sheet_display::paint(&painter, ctx, &mut self.pictures, ops, place);
        }
        let guide = ACCENT.gamma_multiply(0.45);
        for x in &doc.guides_v {
            let sx = self.to_screen(*x, 0.0).x;
            painter.vline(sx, paper.y_range(), Stroke::new(1.0_f32, guide));
        }
        for y in &doc.guides_h {
            let sy = self.to_screen(0.0, *y).y;
            painter.hline(paper.x_range(), sy, Stroke::new(1.0_f32, guide));
        }
        let typing = self.editing.as_ref().map(|e| e.0);
        for item in doc.items.iter().filter(|i| !i.hidden && Some(i.key) != typing) {
            let r = self.rect_of(item);
            let rect = self.screen_rect(r);
            match item.kind.as_str() {
                "frame" => {
                    self.paint_frame(&painter, item, rect);
                    let place = crate::sheet_display::Place { origin: rect.min, zoom: self.zoom };
                    let inner = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
                    for layer in [&self.ops, &self.annots] {
                        if let Some(ops) = layer.get(&item.key) {
                            crate::sheet_display::paint(&inner, ctx, &mut self.pictures, ops, place);
                        }
                    }
                    crate::sheet_display::paint(&painter, ctx, &mut self.pictures, &item.data["display"], place);
                }
                _ => {
                    let place = crate::sheet_display::Place { origin: rect.min, zoom: self.zoom };
                    crate::sheet_display::paint(&painter, ctx, &mut self.pictures, &item.data["display"], place);
                }
            }
        }
        // Hover, selection, handles: the window's accent, on top.
        let painter = ctx.layer_painter(egui::LayerId::background()).with_clip_rect(screen);
        if let Some(h) = self.hover.filter(|h| !self.selection.contains(h)) {
            if let Some(i) = self.item(h) {
                painter.rect_stroke(self.screen_rect(self.ink_of(i)), 0, Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.5)), egui::StrokeKind::Outside);
            }
        }
        let single = self.selection.len() == 1;
        for k in &self.selection {
            let Some(i) = self.item(*k) else { continue };
            let r = self.screen_rect(self.ink_of(i));
            painter.rect_stroke(r, 0, Stroke::new(1.5_f32, ACCENT), egui::StrokeKind::Outside);
            if single && !i.locked && i.editable {
                let only_width = i.kind == "text";
                for (n, c) in self.handles(self.rect_of(i)).iter().enumerate() {
                    if only_width && n != 3 && n != 7 {
                        continue;
                    }
                    let h = Rect::from_center_size(*c, vec2(HANDLE * 2.0, HANDLE * 2.0));
                    painter.rect_filled(h, 1.5, Color32::WHITE);
                    painter.rect_stroke(h, 1.5, Stroke::new(1.0_f32, ACCENT), egui::StrokeKind::Inside);
                }
            }
        }
        if let Some((_, mut a, mut b)) = self.dimension_handle() {
            if let Some(Drag::DimensionLine { offset, dot, .. }) = &self.drag {
                if let Some((p, q, _, axis)) = self.run_seed() {
                    let (v, w) = crate::sheet_tools::line_ends(p, q, *offset, &axis);
                    a = self.to_screen(v[0], v[1]);
                    b = self.to_screen(w[0], w[1]);
                    for line in [[self.to_screen(p[0], p[1]), a], [a, b], [b, self.to_screen(q[0], q[1])]] {
                        painter.line_segment(line, Stroke::new(1.5, ACCENT));
                    }
                    if let Some(d) = dot { painter.circle_filled(self.to_screen(d[0], d[1]), 3.5, ACCENT); }
                }
            }
            let mid = a + (b - a) * 0.5;
            painter.circle_filled(mid, HANDLE, Color32::WHITE);
            painter.circle_stroke(mid, HANDLE, Stroke::new(1.5, ACCENT));
        }
        if let Some(Drag::Place { from, to, .. }) = &self.drag {
            let r = Rect::from_two_pos(self.to_screen(from[0], from[1]), self.to_screen(to[0], to[1]));
            painter.rect_filled(r, 0, ACCENT.gamma_multiply(0.08));
            painter.rect_stroke(r, 0, Stroke::new(1.0_f32, ACCENT), egui::StrokeKind::Middle);
        }
        self.paint_pick(&painter);
    }

    fn paint_frame(&self, painter: &egui::Painter, item: &Item, rect: Rect) {
        let f = &item.data["frame"];
        let inner = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        if let Some(&(_, id)) = self.natives.get(&item.key) {
            inner.image(id, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        if !f["border"].as_bool().unwrap_or(false) {
            // A borderless frame still reads as a frame on screen (never printed).
            dashed_rect(painter, rect, GUIDE_ON_PAPER);
        }
        if !self.has_content(item.key) {
            painter.text(rect.left_top() + vec2(6.0, 6.0), Align2::LEFT_TOP, "Updating the view…",
                         FontId::proportional(TYPE_META), FAINT);
        }
    }

}

fn dashed_rect(painter: &egui::Painter, r: Rect, colour: Color32) {
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    painter.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.0_f32, colour), 4.0, 3.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> Page {
        let mut p = Page { tool: "select".into(), ..Default::default() };
        p.set(r#"{"key":7,"size":[420,297],"margin":10,"border":{"on":false},"guides":{"v":[100],"h":[]},
            "items":[{"key":1,"kind":"frame","rect":[20,20,190,150],"locked":false,"hidden":false,"editable":true,"frame":{}},
                     {"key":2,"kind":"text","rect":[20,190,180,6],"locked":false,"hidden":false,"editable":true,"text":{},
                      "fields":{"text":"Planta","size_pt":14}},
                     {"key":3,"kind":"frame","rect":[230,20,150,150],"locked":true,"hidden":false,"editable":true,"frame":{}}]}"#.into());
        p.zoom = 2.0;
        p.origin = pos2(100.0, 50.0);
        p
    }

    fn at(p: &Page, x: f64, y: f64) -> Pos2 {
        p.to_screen(x, y)
    }

    #[test]
    fn a_click_selects_the_item_under_it() {
        let mut p = page();
        let mut out = vec![];
        let q = at(&p, 50.0, 50.0);
        p.pointer("press", q, 1, 0, &mut out);
        p.pointer("release", q, 1, 0, &mut out);
        assert_eq!(p.selection, vec![1]);
        assert_eq!(out, vec!["sheets:select\u{1f}1".to_string()]);
        let e = at(&p, 300.0, 250.0);
        out.clear();
        p.pointer("press", e, 1, 0, &mut out);
        p.pointer("release", e, 1, 0, &mut out);
        assert!(p.selection.is_empty());
        assert_eq!(out, vec!["sheets:select\u{1f}".to_string()]);
    }

    #[test]
    fn a_drag_moves_the_view_and_commits_once_with_snapping() {
        let mut p = page();
        let mut out = vec![];
        p.pointer("press", at(&p, 50.0, 50.0), 1, 0, &mut out);
        for k in 1..=10 {
            p.pointer("move", at(&p, 50.0 + 3.0 * k as f64, 50.0 + 0.5 * k as f64), 1, 0, &mut out);
        }
        p.pointer("release", at(&p, 80.0, 55.0), 1, 0, &mut out);
        let rects: Vec<_> = out.iter().filter(|a| a.starts_with("sheets:rects")).collect();
        assert_eq!(rects.len(), 1);
        // 20 + 30 = 50 mm, the left edge; the right edge 240 lands within
        // 2 mm of nothing, the centre 145 neither: x stays 50; y 25 → 25.
        let r = p.preview[&1];
        assert!((r[0] - 50.0).abs() < 1e-6, "{r:?}");
        assert!((r[1] - 25.0).abs() < 1e-6, "{r:?}");
        assert_eq!(&r[2..], &[190.0, 150.0]);
    }

    #[test]
    fn snapping_to_the_margin_and_guides() {
        let mut p = page();
        let mut out = vec![];
        p.pointer("press", at(&p, 50.0, 50.0), 1, 0, &mut out);
        p.pointer("move", at(&p, 41.5, 50.0), 1, 0, &mut out);
        p.pointer("move", at(&p, 41.0, 50.0), 1, 0, &mut out);
        // Left edge 11 mm: the margin (10) is within 2 mm.
        assert!((p.preview[&1][0] - 10.0).abs() < 1e-6);
        p.pointer("move", at(&p, 41.0, 50.0), 1, 1, &mut out);
        // ⌥ turns snapping off.
        assert!((p.preview[&1][0] - 11.0).abs() < 1e-6);
    }

    #[test]
    fn a_handle_resizes_and_a_locked_item_stays() {
        let mut p = page();
        let mut out = vec![];
        p.set_selection(vec![1]);
        let corner = at(&p, 210.0, 170.0);
        p.pointer("press", corner, 1, 0, &mut out);
        p.pointer("move", at(&p, 160.0, 120.0), 1, 0, &mut out);
        p.pointer("release", at(&p, 160.0, 120.0), 1, 0, &mut out);
        assert_eq!(out.last().unwrap(), "sheets:rects\u{1f}1:20.0000:20.0000:140.0000:100.0000");
        out.clear();
        p.set_selection(vec![3]);
        p.pointer("press", at(&p, 250.0, 50.0), 1, 0, &mut out);
        p.pointer("move", at(&p, 280.0, 50.0), 1, 0, &mut out);
        p.pointer("release", at(&p, 280.0, 50.0), 1, 0, &mut out);
        assert!(out.iter().all(|a| !a.starts_with("sheets:rects")));
    }

    #[test]
    fn the_view_tool_places_a_frame_and_arrows_nudge() {
        let mut p = page();
        p.tool = "vista".into();
        let mut out = vec![];
        p.pointer("press", at(&p, 300.0, 200.0), 1, 0, &mut out);
        p.pointer("move", at(&p, 380.0, 260.0), 1, 0, &mut out);
        p.pointer("release", at(&p, 380.0, 260.0), 1, 0, &mut out);
        assert_eq!(out, vec!["sheets:place\u{1f}300.000\u{1f}200.000\u{1f}380.000\u{1f}260.000\u{1f}vista".to_string()]);
        out.clear();
        p.set_selection(vec![2]);
        assert!(p.key("ArrowRight", 4, &mut out));
        assert_eq!(out, vec!["sheets:rects\u{1f}2:30.0000:190.0000:180.0000:6.0000".to_string()]);
        assert!(p.key("Escape", 0, &mut out));
        assert!(p.selection.is_empty());
    }

    #[test]
    fn group_selects_together_and_style_pastes_on_its_kind() {
        let mut p = page();
        let mut out = vec![];
        p.set_selection(vec![1, 2]);
        assert!(p.command("group", &mut out));
        let a = out.pop().unwrap();
        assert!(a.starts_with("sheets:set_many\u{1f}1\u{1f}{\"group_id\":\"") && a.contains("\u{1f}2\u{1f}"), "{a}");
        // The document says they are grouped: a click on one takes both.
        let grouped = p.raw.replacen("\"editable\":true,\"frame\":{}}", "\"editable\":true,\"frame\":{},\"fields\":{\"group_id\":\"g\",\"style\":\"vectorial\"}}", 1)
            .replace("\"size_pt\":14}", "\"size_pt\":14,\"group_id\":\"g\"}");
        p.set(grouped);
        p.set_selection(vec![]);
        let q = at(&p, 50.0, 50.0);
        p.pointer("press", q, 1, 0, &mut out);
        p.pointer("release", q, 1, 0, &mut out);
        assert_eq!(p.selection, vec![1, 2]);
        out.clear();
        assert!(p.command("ungroup", &mut out));
        assert!(out[0].contains("{\"group_id\":\"\"}"));
        // Copy a frame's look, paste it on the other frame (locked: no), on a text (another kind: no).
        p.set_selection(vec![1]);
        assert!(p.command("copy_style", &mut out));
        out.clear();
        p.set_selection(vec![2]);
        assert!(p.command("paste_style", &mut out));
        assert!(out.is_empty());
        assert!(p.hint().contains("another kind"));
    }

    #[test]
    fn a_double_click_types_in_a_text_and_an_item_tool_places_its_kind() {
        let mut p = page();
        let mut out = vec![];
        p.pointer("double", at(&p, 30.0, 192.0), 1, 0, &mut out);
        assert_eq!(p.editing, Some((2, "Planta".to_string())));
        p.commit_text(&mut out);
        assert!(out.is_empty(), "unchanged words are no edit");
        p.edit_text(2);
        p.editing.as_mut().unwrap().1.push_str(" baja");
        p.commit_text(&mut out);
        assert_eq!(out, vec![format!("sheets:set\u{1f}2\u{1f}{}", serde_json::json!({"text": "Planta baja"}))]);
        out.clear();
        p.tool = "escala".into();
        p.pointer("press", at(&p, 300.0, 250.0), 1, 0, &mut out);
        p.pointer("release", at(&p, 300.0, 250.0), 1, 0, &mut out);
        assert_eq!(out, vec!["sheets:place\u{1f}300.000\u{1f}250.000\u{1f}300.000\u{1f}250.000\u{1f}escala".to_string()]);
    }

    #[test]
    fn the_dimension_tool_snaps_two_points_and_its_offset_click_makes_it() {
        let mut p = page();
        p.tool = "cota".into();
        p.snaps.insert(1, crate::sheet_tools::FrameSnaps {
            points: vec![[50.0, 60.0], [90.0, 60.0]],
            world: vec![[1.0, 2.0, 0.0], [5.0, 2.0, 0.0]],
            ..Default::default()
        });
        let mut out = vec![];
        p.pointer("move", at(&p, 50.4, 60.3), 0, 0, &mut out);
        assert!(p.hot.is_some());
        // Clicks: a press and a release each, as the shell sends them.
        for (x, y) in [(50.4, 60.3), (89.8, 60.2)] {
            p.pointer("press", at(&p, x, y), 1, 0, &mut out);
            p.pointer("release", at(&p, x, y), 1, 0, &mut out);
        }
        p.pointer("move", at(&p, 70.0, 52.0), 0, 0, &mut out);
        assert!(p.hot.is_none(), "the offset does not snap");
        p.pointer("press", at(&p, 70.0, 52.0), 1, 0, &mut out);
        p.pointer("release", at(&p, 70.0, 52.0), 1, 0, &mut out);
        assert_eq!(out.len(), 1);
        let body: serde_json::Value = serde_json::from_str(out[0].strip_prefix("sheets:cota\u{1f}").unwrap()).unwrap();
        assert_eq!(body["a"]["p"], serde_json::json!([50.0, 60.0]));
        assert_eq!(body["b"]["world"], serde_json::json!([5.0, 2.0, 0.0]));
        assert_eq!(body["a"]["frame"], 1);
        assert!((body["sep"].as_f64().unwrap() - -8.0).abs() < 1e-9);
        assert!(p.pick.is_none());
        p.pointer("press", at(&p, 10.0, 10.0), 1, 0, &mut out);
        assert!(p.key("Escape", 0, &mut out) && p.pick.is_none());
    }

    /// The page with a dimension from (100, 60) to (140, 60), its line 10 mm above.
    fn page_with_a_dimension() -> Page {
        let mut p = page();
        p.set(r#"{"key":7,"size":[420,297],"margin":10,"border":{"on":false},"guides":{"v":[],"h":[]},
            "items":[{"key":1,"kind":"frame","rect":[20,20,190,150],"locked":false,"hidden":false,"editable":true,"frame":{}},
                     {"key":4,"kind":"dimension","rect":[100,50,40,10],"locked":false,"hidden":false,"editable":false,
                      "fields":{"x_mm":100.0,"y_mm":60.0,"dx_mm":40.0,"dy_mm":0.0,"sep_mm":-10.0,"axis":""}}]}"#.into());
        p
    }

    #[test]
    fn line_drag_skips_itself_and_commits_only_the_offset_on_release() {
        let mut p = page_with_a_dimension();
        p.zoom = 2.0;
        let doc = p.doc.as_mut().unwrap();
        let mut target = doc.items.last().unwrap().clone();
        target.key = 5;
        target.data["fields"]["x_mm"] = serde_json::json!(20.0);
        target.data["fields"]["sep_mm"] = serde_json::json!(-25.0);
        doc.items.push(target);
        p.selection = vec![4];
        let mut out = Vec::new();
        let start = at(&p, 120.0, 50.0);
        let end = at(&p, 120.0, 36.5);
        p.pointer("press", start, 1, 0, &mut out);
        p.pointer("move", at(&p, 120.0, 49.0), 1, 0, &mut out);
        assert!(matches!(p.drag, Some(Drag::DimensionLine { offset: -11.0, dot: None, .. })));
        p.pointer("move", end, 1, 0, &mut out);
        assert!(matches!(p.drag, Some(Drag::DimensionLine { offset: -25.0, dot: Some(_), .. })));
        assert!(out.is_empty());
        p.key("Escape", 0, &mut out);
        p.pointer("release", end, 1, 0, &mut out);
        assert!(out.is_empty());
        p.pointer("press", start, 1, 0, &mut out);
        p.pointer("release", end, 1, 0, &mut out);
        assert_eq!(out, vec!["sheets:set\x1f4\x1f{\"sep_mm\":-25.0}"]);
        p.doc.as_mut().unwrap().items[1].locked = true;
        assert!(p.dimension_line_at(start).is_none());
    }

    #[test]
    fn context_hit_preserves_a_multi_selection_and_offers_the_dock_edit() {
        let mut p = page_with_a_dimension();
        let mut out = Vec::new();
        p.selection = vec![1, 4];
        let hit = at(&p, 120.0, 53.0);
        let (title, _) = p.context_at(hit, &mut out).unwrap();
        assert_eq!(title, "2 items");
        assert!(out.is_empty());
        p.selection.clear();
        let (title, rows) = p.context_at(hit, &mut out).unwrap();
        assert_eq!((title.as_str(), &p.selection[..]), ("Dimension", &[4][..]));
        let edit = crate::sheet_inspector::edit(p.doc.as_ref().unwrap(), "4:show_unit", "1").unwrap();
        assert!(rows.iter().flat_map(|row| &row.1).any(|entry| entry.0 == format!("sheets:{edit}")));
        let blank = at(&p, 300.0, 250.0);
        let (title, rows) = p.context_at(blank, &mut out).unwrap();
        assert!(p.selection.is_empty());
        assert_eq!(title, "Sheet");
        assert!(rows.iter().flat_map(|row| &row.1).any(|entry| entry.0 == "sheets:fit"));
    }

    fn cota_of(action: &str) -> serde_json::Value {
        serde_json::from_str(action.strip_prefix("sheets:cota\u{1f}").unwrap()).unwrap()
    }

    #[test]
    fn a_dimension_s_offset_lines_up_on_a_parallel_one() {
        let mut p = page_with_a_dimension();
        p.tool = "cota".into();
        let mut out = vec![];
        for (x, y) in [(50.0, 60.0), (90.0, 60.0)] {
            p.pointer("press", at(&p, x, y), 1, 0, &mut out);
            p.pointer("release", at(&p, x, y), 1, 0, &mut out);
        }
        // 1.5 mm off the other's line (3.5 mm is ~7 px at this zoom): on it,
        // the dot at its nearer end.
        p.pointer("move", at(&p, 70.0, 51.5), 0, 0, &mut out);
        assert_eq!(p.hot.as_ref().map(|h| h.p), Some([100.0, 50.0]));
        p.pointer("press", at(&p, 70.0, 51.5), 1, 0, &mut out);
        assert!((cota_of(&out[0])["sep"].as_f64().unwrap() - -10.0).abs() < 1e-9, "{out:?}");
        // Far from it: free.
        out.clear();
        for (x, y) in [(50.0, 60.0), (90.0, 60.0), (70.0, 45.0)] {
            p.pointer("press", at(&p, x, y), 1, 0, &mut out);
            p.pointer("release", at(&p, x, y), 1, 0, &mut out);
        }
        assert!((cota_of(&out[0])["sep"].as_f64().unwrap() - -15.0).abs() < 1e-9);
    }

    #[test]
    fn a_run_carries_on_from_the_selected_dimension() {
        let mut p = page_with_a_dimension();
        p.set_selection(vec![4]);
        p.tool = "cota_cadena".into();
        let mut out = vec![];
        // The first click is already the next point: from the selected
        // one's second end, on its line.
        p.pointer("press", at(&p, 170.0, 60.0), 1, 0, &mut out);
        p.pointer("release", at(&p, 170.0, 60.0), 1, 0, &mut out);
        assert_eq!(out.len(), 1, "{out:?}");
        let c = cota_of(&out[0]);
        assert_eq!(c["a"]["p"], serde_json::json!([140.0, 60.0]));
        assert_eq!(c["b"]["p"], serde_json::json!([170.0, 60.0]));
        assert!((c["sep"].as_f64().unwrap() - -10.0).abs() < 1e-9);
        // A baseline carries on from its first end, a row further out.
        let mut p = page_with_a_dimension();
        p.set_selection(vec![4]);
        p.tool = "cota_base".into();
        out.clear();
        p.pointer("press", at(&p, 170.0, 60.0), 1, 0, &mut out);
        let c = cota_of(&out[0]);
        assert_eq!(c["a"]["p"], serde_json::json!([100.0, 60.0]));
        assert!((c["sep"].as_f64().unwrap() - -(10.0 + p.run_step)).abs() < 1e-9);
        // Nothing selected: a run starts from scratch.
        let mut p = page_with_a_dimension();
        p.tool = "cota_cadena".into();
        out.clear();
        p.pointer("press", at(&p, 170.0, 60.0), 1, 0, &mut out);
        assert!(out.is_empty() && p.pick.as_ref().unwrap().pts.len() == 1);
    }

    #[test]
    fn a_chain_places_on_the_offset_click_then_one_per_point_and_ends_with_its_total() {
        let mut p = page();
        p.tool = "cota_cadena".into();
        p.run_step = 8.0;
        let mut out = vec![];
        for (x, y) in [(50.0, 260.0), (80.0, 260.0), (65.0, 250.0), (120.0, 260.0)] {
            p.pointer("press", at(&p, x, y), 1, 0, &mut out);
        }
        assert_eq!(out.len(), 2, "{out:?}");
        let second: serde_json::Value = serde_json::from_str(out[1].strip_prefix("sheets:cota\u{1f}").unwrap()).unwrap();
        assert_eq!(second["a"]["p"], serde_json::json!([80.0, 260.0]));
        assert!((second["sep"].as_f64().unwrap() - -10.0).abs() < 1e-9, "on the chain's line");
        p.pointer("press", at(&p, 120.0, 260.0), 1, 0, &mut out);
        assert!(out[2].starts_with("sheets:cota_total\u{1f}") && p.pick.is_none());
    }

    #[test]
    fn a_click_on_an_arc_dimensions_it_and_an_angle_takes_four_clicks() {
        let mut p = page();
        p.snaps.insert(1, crate::sheet_tools::FrameSnaps { circles: vec![[100.0, 100.0, 10.0]], ..Default::default() });
        p.tool = "cota_radio".into();
        let mut out = vec![];
        p.pointer("press", at(&p, 100.0, 110.4), 1, 2, &mut out);
        let body: serde_json::Value = serde_json::from_str(out[0].strip_prefix("sheets:radial\u{1f}").unwrap()).unwrap();
        assert_eq!(body["c"], serde_json::json!([100.0, 100.0]));
        assert_eq!(body["kind"], "diameter");
        assert!((body["p"][1].as_f64().unwrap() - 110.0).abs() < 1e-9);
        out.clear();
        p.tool = "cota_ang".into();
        for (x, y) in [(50.0, 250.0), (70.0, 251.0), (50.0, 230.0), (58.0, 250.0)] {
            p.pointer("press", at(&p, x, y), 1, if x == 70.0 { 4 } else { 0 }, &mut out);
        }
        let body: serde_json::Value = serde_json::from_str(out[0].strip_prefix("sheets:angular\u{1f}").unwrap()).unwrap();
        // ⇧ put the arm on 0°, its length kept.
        assert!((body["a"][1].as_f64().unwrap() - 250.0).abs() < 1e-9);
        assert!((body["a"][0].as_f64().unwrap() - (50.0 + 20f64.hypot(1.0))).abs() < 1e-9);
        assert!((body["r"].as_f64().unwrap() - 8.0).abs() < 1e-9);
    }

    #[test]
    fn two_clicks_place_a_shape_snapped_and_the_painter_copies_then_pastes() {
        let mut p = page();
        p.snaps.insert(1, crate::sheet_tools::FrameSnaps { points: vec![[40.0, 40.0]], world: vec![[0.0, 0.0, 0.0]], ..Default::default() });
        p.set_tool("rect".into());
        let mut out = vec![];
        p.pointer("press", at(&p, 40.3, 40.2), 1, 0, &mut out);
        p.pointer("release", at(&p, 40.3, 40.2), 1, 0, &mut out);
        assert!(out.is_empty(), "a click waits for the second");
        p.pointer("move", at(&p, 70.0, 60.0), 0, 0, &mut out);
        p.pointer("press", at(&p, 70.0, 60.0), 1, 0, &mut out);
        assert_eq!(out[0], "sheets:place\u{1f}40.000\u{1f}40.000\u{1f}70.000\u{1f}60.000\u{1f}rect\u{1f}{\"frame\":1,\"p\":[40.0,40.0],\"segment\":null,\"world\":[0.0,0.0,0.0]}");
        out.clear();
        p.set_tool("estilo".into());
        assert!(p.painter_armed);
        p.pointer("press", at(&p, 50.0, 50.0), 1, 0, &mut out);
        assert!(!p.painter_armed, "the first click took the frame's style");
    }

    #[test]
    fn the_document_catching_up_drops_the_preview() {
        let mut p = page();
        p.preview.insert(1, [30.0, 20.0, 190.0, 150.0]);
        let moved = p.raw.replace("[20,20,190,150]", "[30,20,190,150]");
        p.set(moved);
        assert!(p.preview.is_empty());
        assert_eq!(p.doc.as_ref().unwrap().items[0].rect, [30.0, 20.0, 190.0, 150.0]);
    }
}
