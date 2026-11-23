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
const PT_TO_MM: f32 = 25.4 / 72.0;
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
            guides_v: nums(&v["guides"]["v"]),
            guides_h: nums(&v["guides"]["h"]),
            fields: v["fields"].clone(),
            choices: v["choices"].clone(),
            items,
        })
    }
}

/// What a frame shows: its line drawing, paper mm in the frame, with a
/// line class each (0 edge, 1 profile, 2 cut, 3 hidden), or a picture.
#[derive(Default)]
pub struct Lines {
    pub segs: Vec<[f32; 4]>,
    pub kinds: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
enum Drag {
    /// Moving the selection: where it was grabbed (mm) and the rects it started from.
    Move { grab: [f64; 2], from: Vec<(u64, [f64; 4])> },
    /// One item by a handle (0..8 clockwise from top-left).
    Resize { key: u64, handle: u8, grab: [f64; 2], from: [f64; 4] },
    /// The View tool's rectangle.
    Place { from: [f64; 2], to: [f64; 2] },
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
    lines: HashMap<u64, Lines>,
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
}

fn hex(s: &str, fallback: Color32) -> Color32 {
    let h = s.trim_start_matches('#');
    if h.len() != 6 {
        return fallback;
    }
    match u32::from_str_radix(h, 16) {
        Ok(v) => Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8),
        Err(_) => fallback,
    }
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
            self.selection.clear();
            self.preview.clear();
        }
        // A gesture's rects stay until the document agrees (or moved on).
        self.preview.retain(|k, r| doc.items.iter().any(|i| i.key == *k && i.rect != *r));
        let keys: Vec<u64> = doc.items.iter().map(|i| i.key).collect();
        self.selection.retain(|k| keys.contains(k));
        self.lines.retain(|k, _| keys.contains(k));
        let gone: Vec<u64> = self.natives.keys().filter(|k| !keys.contains(k)).copied().collect();
        for k in gone {
            self.drop_native(k);
        }
        self.raw = text;
        self.doc = Some(doc);
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
        self.lines.remove(&key);
        if let Some((_, old)) = self.natives.insert(key, (texture, id)) {
            self.freed.push(old);
        }
    }
    pub fn set_lines(&mut self, key: u64, segs: Vec<[f32; 4]>, kinds: Vec<u8>) {
        self.drop_native(key);
        self.lines.insert(key, Lines { segs, kinds });
    }
    pub fn forget_content(&mut self, key: u64) {
        self.lines.remove(&key);
        self.drop_native(key);
    }
    pub fn has_content(&self, key: u64) -> bool {
        self.lines.contains_key(&key) || self.natives.contains_key(&key)
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
            (Some(Drag::Place { .. }), _) => "Release to place the view, at the scale that fits.",
            (_, "pan") => "Drag to move the sheet. Scroll pans, ⌘-scroll zooms.",
            (_, "zoom") | (_, "zoom_ventana") => "Click to zoom in, ⌥-click to zoom out.",
            (_, "vista") => "Drag a rectangle to place a model view.",
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
        let tool = self.tool.as_str();
        match kind {
            "leave" => {
                self.hover = None;
                self.cursor = "";
            }
            "press" | "double" => {
                if button == 3 || tool == "pan" {
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
                if tool == "vista" {
                    self.drag = Some(Drag::Place { from: mm, to: mm });
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
                Some(Drag::Place { from, .. }) => {
                    self.drag = Some(Drag::Place { from, to: self.to_mm(p) });
                    self.cursor = "crosshair";
                }
                _ => {
                    self.hover = self.hit(self.to_mm(p));
                    self.cursor = match (tool, self.handle_at(p)) {
                        ("pan", _) => "grab",
                        ("vista", _) => "crosshair",
                        ("zoom" | "zoom_ventana", _) => "zoom",
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
                    Some(Drag::Resize { key, from, .. }) => {
                        if let Some(r) = self.preview.get(&key).copied().filter(|r| *r != from) {
                            out.push(Self::rects_action(&[(key, r)]));
                        }
                    }
                    Some(Drag::Place { from, to }) => {
                        out.push(format!("sheets:place\u{1f}{:.3}\u{1f}{:.3}\u{1f}{:.3}\u{1f}{:.3}",
                                         from[0], from[1], to[0], to[1]));
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
        let z = self.zoom;
        let m = doc.margin;
        dashed_rect(&painter, self.screen_rect([m, m, doc.size[0] - 2.0 * m, doc.size[1] - 2.0 * m]), GUIDE_ON_PAPER);
        if doc.border["on"].as_bool().unwrap_or(false) {
            let w = (doc.border["mm"].as_f64().unwrap_or(0.5) as f32 * z).max(1.0);
            let c = hex(doc.border["color"].as_str().unwrap_or(""), INK_ON_PAPER);
            let r = (doc.border["radius_mm"].as_f64().unwrap_or(0.0) as f32 * z) as u8;
            let rect = self.screen_rect([m, m, doc.size[0] - 2.0 * m, doc.size[1] - 2.0 * m]);
            painter.rect_stroke(rect, r, Stroke::new(w, c), egui::StrokeKind::Middle);
            if doc.border["style"].as_str() == Some("double") {
                painter.rect_stroke(rect.shrink(w * 3.0), r, Stroke::new(w * 0.5, c), egui::StrokeKind::Middle);
            }
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
        for item in doc.items.iter().filter(|i| !i.hidden) {
            let r = self.rect_of(item);
            let rect = self.screen_rect(r);
            match item.kind.as_str() {
                "frame" => self.paint_frame(&painter, item, rect),
                "text" => self.paint_text(&painter, ctx, item, rect),
                "titleblock" => self.paint_title_block(&painter, item, rect),
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
        if let Some(Drag::Place { from, to }) = &self.drag {
            let r = Rect::from_two_pos(self.to_screen(from[0], from[1]), self.to_screen(to[0], to[1]));
            painter.rect_filled(r, 0, ACCENT.gamma_multiply(0.08));
            painter.rect_stroke(r, 0, Stroke::new(1.0_f32, ACCENT), egui::StrokeKind::Middle);
        }
    }

    fn paint_frame(&self, painter: &egui::Painter, item: &Item, rect: Rect) {
        let f = &item.data["frame"];
        let z = self.zoom;
        let inner = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        if let Some(&(_, id)) = self.natives.get(&item.key) {
            inner.image(id, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        } else if let Some(lines) = self.lines.get(&item.key) {
            let pens: Vec<f32> = f["pens"].as_array().map(|a| a.iter().map(|p| p.as_f64().unwrap_or(0.18) as f32).collect())
                .unwrap_or_else(|| vec![0.18, 0.35, 0.5, 0.18]);
            let o = rect.min;
            for (s, k) in lines.segs.iter().zip(&lines.kinds) {
                let pen = pens.get(*k as usize).copied().unwrap_or(0.18);
                let stroke = Stroke::new((pen * z).max(0.6), INK_ON_PAPER);
                let a = o + vec2(s[0] * z, s[1] * z);
                let b = o + vec2(s[2] * z, s[3] * z);
                if *k == 3 {
                    // Hidden lines: dashed 1.5 / 0.8 mm, as the sheet inks them.
                    inner.extend(egui::Shape::dashed_line(&[a, b], stroke, 1.5 * z, 0.8 * z));
                } else {
                    inner.line_segment([a, b], stroke);
                }
            }
        }
        if f["border"].as_bool().unwrap_or(false) {
            let w = (f["border_mm"].as_f64().unwrap_or(0.3) as f32 * z).max(1.0);
            painter.rect_stroke(rect, 0, Stroke::new(w, hex(f["border_color"].as_str().unwrap_or(""), INK_ON_PAPER)),
                                egui::StrokeKind::Middle);
        } else {
            dashed_rect(painter, rect, GUIDE_ON_PAPER);
        }
        if !self.has_content(item.key) {
            painter.text(rect.left_top() + vec2(6.0, 6.0), Align2::LEFT_TOP, "Updating the view…",
                         FontId::proportional(TYPE_META), FAINT);
        }
        if f["show_title"].as_bool().unwrap_or(false) {
            let size = (f["title_mm"].as_f64().unwrap_or(4.0) as f32 * z).max(4.0);
            let mut title = f["title"].as_str().unwrap_or("").to_string();
            if f["title_scale"].as_bool().unwrap_or(true) {
                title = format!("{title}    ESC. {}", f["scale"].as_str().unwrap_or(""));
            }
            let above = f["title_pos"].as_str() == Some("above");
            let y = if above { rect.top() - size * 0.4 } else { rect.bottom() + size * 0.4 };
            let anchor = if above { Align2::LEFT_BOTTOM } else { Align2::LEFT_TOP };
            painter.text(pos2(rect.left(), y), anchor, title, FontId::proportional(size), INK_ON_PAPER);
            let rule_y = if above { y + 0.15 * size } else { y + size * 1.3 };
            painter.hline(rect.x_range(), rule_y, Stroke::new((0.25 * z).max(0.6), INK_ON_PAPER));
        }
    }

    fn paint_text(&mut self, painter: &egui::Painter, ctx: &egui::Context, item: &Item, rect: Rect) {
        let t = &item.data["text"];
        let z = self.zoom;
        let size = (t["size_pt"].as_f64().unwrap_or(14.0) as f32 * PT_TO_MM * z).max(1.0);
        let colour = hex(t["color"].as_str().unwrap_or(""), INK_ON_PAPER);
        let mut job = egui::text::LayoutJob::default();
        let format = egui::TextFormat {
            font_id: FontId::proportional(size),
            color: colour,
            italics: t["italic"].as_bool().unwrap_or(false),
            underline: if t["underline"].as_bool().unwrap_or(false) { Stroke::new((size / 14.0).max(0.6), colour) } else { Stroke::NONE },
            ..Default::default()
        };
        job.append(t["text"].as_str().unwrap_or(""), 0.0, format);
        job.wrap.max_width = rect.width().max(1.0);
        job.halign = match t["align"].as_str() {
            Some("center") => egui::Align::Center,
            Some("right") => egui::Align::RIGHT,
            _ => egui::Align::LEFT,
        };
        let galley = ctx.fonts(|f| f.layout_job(job));
        let h = galley.size().y.max(size);
        self.text_h.insert(item.key, (h / z.max(1e-6)) as f64);
        let block = Rect::from_min_size(rect.min, vec2(rect.width(), h));
        let bg = t["bg_color"].as_str().unwrap_or("");
        if !bg.is_empty() {
            let a = t["bg_opacity"].as_f64().unwrap_or(1.0).clamp(0.0, 1.0) as f32;
            painter.rect_filled(block.expand(1.0 * z), 0, hex(bg, Color32::WHITE).gamma_multiply(a));
        }
        let x = match t["align"].as_str() {
            Some("center") => block.center().x,
            Some("right") => block.right(),
            _ => block.left(),
        };
        painter.galley(pos2(x, block.top()), galley.clone(), colour);
        if t["bold"].as_bool().unwrap_or(false) {
            // Faux bold: the font set has one weight.
            painter.galley(pos2(x + (size / 28.0).max(0.4), block.top()), galley, colour);
        }
    }

    fn paint_title_block(&self, painter: &egui::Painter, item: &Item, rect: Rect) {
        let t = &item.data["titleblock"];
        let z = self.zoom;
        let rows: Vec<(String, String)> = t["rows"].as_array().map(|a| a.iter().map(|r| (
            r[0].as_str().unwrap_or("").to_string(), r[1].as_str().unwrap_or("").to_string())).collect()).unwrap_or_default();
        let line = hex(t["line_color"].as_str().unwrap_or(""), INK_ON_PAPER);
        let label_c = hex(t["label_color"].as_str().unwrap_or(""), FAINT);
        let text_c = hex(t["text_color"].as_str().unwrap_or(""), INK_ON_PAPER);
        let border = Stroke::new((t["border_mm"].as_f64().unwrap_or(0.5) as f32 * z).max(1.0), line);
        let grid = Stroke::new((t["line_mm"].as_f64().unwrap_or(0.2) as f32 * z).max(0.6), line);
        let fill = t["fill_color"].as_str().unwrap_or("");
        let minimal = t["layout"].as_str() == Some("minimal");
        let cols = t["columns"].as_i64().unwrap_or(1).max(1) as usize;
        let per_col = rows.len().div_ceil(cols).max(1);
        let col_w = rect.width() / cols as f32;
        let row_h = rect.height() / per_col as f32;
        let label_w = match t["label_mm"].as_f64().unwrap_or(0.0) as f32 {
            w if w > 0.0 => w * z,
            _ => col_w * 0.3,
        };
        let size = (row_h * 0.42).min(3.2 * z).max(1.0);
        for (n, (label, value)) in rows.iter().enumerate() {
            let (c, r) = (n / per_col, n % per_col);
            let cell = Rect::from_min_size(pos2(rect.left() + c as f32 * col_w, rect.top() + r as f32 * row_h), vec2(col_w, row_h));
            let lab = Rect::from_min_max(cell.min, pos2(cell.left() + label_w, cell.bottom()));
            if !fill.is_empty() {
                painter.rect_filled(lab, 0, hex(fill, Color32::WHITE));
            }
            if !minimal {
                painter.rect_stroke(cell, 0, grid, egui::StrokeKind::Middle);
                painter.vline(lab.right(), cell.y_range(), grid);
            } else if r + 1 < per_col {
                painter.hline(cell.x_range(), cell.bottom(), grid);
            }
            painter.text(pos2(lab.left() + 1.0 * z, lab.center().y), Align2::LEFT_CENTER, label,
                         FontId::proportional(size * 0.8), label_c);
            painter.text(pos2(lab.right() + 1.5 * z, cell.center().y), Align2::LEFT_CENTER, value,
                         FontId::proportional(size), text_c);
        }
        painter.rect_stroke(rect, 0, border, egui::StrokeKind::Middle);
        if t["double_border"].as_bool().unwrap_or(false) {
            painter.rect_stroke(rect.shrink(1.2 * z), 0, Stroke::new(grid.width, line), egui::StrokeKind::Middle);
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
                     {"key":2,"kind":"text","rect":[20,190,180,6],"locked":false,"hidden":false,"editable":true,"text":{}},
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
        assert_eq!(out, vec!["sheets:place\u{1f}300.000\u{1f}200.000\u{1f}380.000\u{1f}260.000".to_string()]);
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
            .replace("\"text\":{}}", "\"text\":{},\"fields\":{\"group_id\":\"g\"}}");
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
    fn the_document_catching_up_drops_the_preview() {
        let mut p = page();
        p.preview.insert(1, [30.0, 20.0, 190.0, 150.0]);
        let moved = p.raw.replace("[20,20,190,150]", "[30,20,190,150]");
        p.set(moved);
        assert!(p.preview.is_empty());
        assert_eq!(p.doc.as_ref().unwrap().items[0].rect, [30.0, 20.0, 190.0, 150.0]);
    }
}
