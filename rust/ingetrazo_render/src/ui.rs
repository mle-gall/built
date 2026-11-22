//! Viewport chrome drawn by egui in the frame that draws the model (the
//! UI spike, `ui/egui-spike`). Python forwards pointer events and registers
//! the tools; clicks come back as action strings. Nothing here owns model
//! state: the palette shows what Python says is active.

use std::sync::Arc;
use std::time::Instant;

use egui::{pos2, vec2, Color32, CornerRadius, Margin, Pos2, Rect, Sense, Shadow, Stroke};

// ---- Design tokens (council 2026-11-22; one place, every component
// below draws from these). Text on graphite glass, three levels plus off.
const INK: Color32 = Color32::from_rgb(0xEE, 0xF1, 0xF4);        // primary text, active icons
const INK_2: Color32 = Color32::from_rgb(0xC9, 0xCF, 0xD6);      // idle icons
const DIM: Color32 = Color32::from_rgb(0xA2, 0xA9, 0xB1);        // counts, keys, secondary text
const FAINT: Color32 = Color32::from_rgb(0x85, 0x8C, 0x94);      // captions, placeholders, shortcuts
const OFF: Color32 = Color32::from_rgb(0x5C, 0x63, 0x6A);        // disabled
const ACCENT: Color32 = Color32::from_rgb(0x6A, 0xA5, 0xFF);
const ACTIVE: Color32 = Color32::from_rgba_premultiplied(25, 40, 61, 61);    // accent @ 24 %
const ACCENT_LINE: Color32 = Color32::from_rgba_premultiplied(58, 91, 140, 140); // accent @ 55 %
const HOVER: Color32 = Color32::from_rgba_premultiplied(18, 18, 18, 18);     // white @ 7 %
const PRESS: Color32 = Color32::from_rgba_premultiplied(31, 31, 31, 31);     // white @ 12 %
const EDGE: Color32 = Color32::from_rgba_premultiplied(20, 20, 20, 20);      // hairline, white @ 8 %
const FIELD: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 71);        // inset field, black @ 28 %
const WARN: Color32 = Color32::from_rgb(0xF5, 0xB8, 0x4A);
const WARN_FILL: Color32 = Color32::from_rgba_premultiplied(34, 26, 10, 36);  // warn @ 14 %
const PANEL: Color32 = Color32::from_rgba_premultiplied(29, 31, 36, 245);    // tooltips (opaque)
// Radii: concentric — control 6 inside bars/menus (margin 4) = 10, inside panels (margin 6) = 12.
const R_CONTROL: u8 = 6;
const R_ROW: u8 = 6;
// Type roles (Inter 400 / 500 / 600 when bundled, else the system font).
fn body() -> egui::FontId { egui::FontId::proportional(12.0) }
fn meta() -> egui::FontId { egui::FontId::proportional(11.0) }
fn strong(size: f32) -> egui::FontId { egui::FontId::new(size, egui::FontFamily::Name("medium".into())) }
fn title() -> egui::FontId { egui::FontId::new(13.0, egui::FontFamily::Name("semibold".into())) }

/// The one state painter: every control's fill and line come from here.
#[derive(Clone, Copy, PartialEq)]
enum State { Idle, Hover, Press, On, Off }
fn state_of(resp: &egui::Response, on: bool, enabled: bool) -> State {
    if !enabled { State::Off }
    else if on { State::On }
    else if resp.is_pointer_button_down_on() { State::Press }
    else if resp.hovered() { State::Hover }
    else { State::Idle }
}
fn paint_state(ui: &egui::Ui, rect: Rect, st: State, radius: u8) {
    let r = CornerRadius::same(radius);
    match st {
        State::Hover => { ui.painter().rect_filled(rect, r, HOVER); }
        State::Press => { ui.painter().rect_filled(rect, r, PRESS); }
        State::On => {
            ui.painter().rect_filled(rect, r, ACTIVE);
            ui.painter().rect_stroke(rect.shrink(0.25), r, Stroke::new(0.5_f32, ACCENT_LINE), egui::StrokeKind::Inside);
        }
        _ => {}
    }
}
fn ink_of(st: State) -> Color32 {
    match st { State::Off => OFF, State::On => Color32::WHITE, State::Idle => INK_2, _ => INK }
}

/// Glass surfaces (council 2026-11-22): Panel for data (tree, inspector),
/// Raised for menus, Pill for compact chrome (palette, rows, bars).
#[derive(Clone, Copy)]
enum Surface { Panel, Raised, Pill }
impl Surface {
    fn look(self) -> crate::glass::Look {
        // (tint mix, saturation, peak, lit rim, quiet rim, top, inner shadow)
        let (mix, sat, peak, lit, quiet, top, inner) = match self {
            Surface::Panel => (0.72, 0.30, 0.080, 0.20, 0.045, 0.022, 0.12),
            Surface::Raised => (0.82, 0.16, 0.065, 0.23, 0.055, 0.030, 0.14),
            Surface::Pill => (0.61, 0.45, 0.085, 0.20, 0.045, 0.022, 0.10),
        };
        crate::glass::Look { tint: [0.01444, 0.01764, 0.02315, mix], saturation: sat, rim: [lit, quiet],
                             rim_width: 1.25, glow: top, peak, inner_shadow: inner }
    }
    fn radius(self) -> u8 { match self { Surface::Panel => 12, Surface::Raised | Surface::Pill => 10 } }
    fn shadow(self) -> Shadow {
        let (y, blur, alpha) = match self {
            Surface::Panel => (8, 24, 100),
            Surface::Raised => (12, 32, 128),
            Surface::Pill => (4, 14, 90),
        };
        Shadow { offset: [0, y], blur, spread: 0, color: Color32::from_black_alpha(alpha) }
    }
}

struct Tool {
    id: String,
    label: String,
    key: String,
    icon: Option<egui::TextureHandle>,
}

/// A window action shown in the chrome: (id, label, tooltip, icon key,
/// checked: -1 not checkable / 0 / 1, enabled).
pub type Item = (String, String, String, String, i8, bool);

pub struct Ui {
    ctx: egui::Context,
    painter: egui_wgpu::Renderer,
    start: Instant,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
    groups: Vec<Vec<Tool>>,
    active: String,
    status: String,
    hint: String,
    /// Value at the cursor: caption, value, typing, cursor (points).
    cursor: Option<(String, String, bool, Pos2)>,
    /// Selection bar: summary and (id, label, shortcut) actions.
    selection: Option<(String, Vec<(String, String, String, bool, bool)>)>,
    sel_more: bool,
    /// The open submenu of More (the modeller's secondary menus): its prefix and row.
    sel_sub: Option<(String, Rect)>,
    actions: Vec<String>,
    /// The press started on the chrome: its drag and release stay there.
    captured: bool,
    /// A press that started on the model: the chrome ignores its drag.
    outside_press: bool,
    repaint_ms: Option<f64>,
    pub frame_ms: f64,
    /// Interactive areas as last laid out (points).
    hit: Vec<Rect>,
    icons: std::collections::HashMap<String, egui::TextureHandle>,
    /// Top-right cluster: document actions, latched view toggles, View chip.
    cluster: Vec<Vec<Item>>,
    view_menu: Vec<(String, Vec<Item>)>,
    view_open: bool,
    /// Options strip: (id, label, key, state: 0 off, 1 on, 2 read-only note).
    options: Vec<(String, String, String, u8)>,
    /// Right column: project tree above, inspector below.
    column: Option<Column>,
    glass: crate::glass::Glass,
}

/// One tree node as Python sends it, in depth-first order.
pub struct Node {
    key: String,
    parent: Option<usize>,
    depth: usize,
    name: String,
    kind: String,
    count: u32,
    problems: u32,
    active: bool,
    has_children: bool,
}

pub struct Column {
    nodes: Vec<Node>,
    unfiled: u32,
    expanded: std::collections::HashSet<String>,
    filter: String,
    /// "File to…": the next row clicked (or Enter) receives the selection.
    filing: Option<String>,
    focus_filter: bool,
    row_menu: Option<(String, Pos2)>,
    inspector_title: String,
    inspector: Vec<(String, Vec<(String, String)>)>,
    highlighted: Option<String>,
}

/// A glass surface with egui content on it: shadow, then the blurred model
/// through the rounded shape (glass.rs), then the content. Built like an
/// egui Frame so call sites read the same.
struct Glass {
    surface: Surface,
    margin: Margin,
    radius: u8,
    accent: Option<Color32>,
}
fn frame() -> Glass { glass(Surface::Pill) }
fn glass(surface: Surface) -> Glass {
    Glass { surface, margin: Margin::same(4), radius: surface.radius(), accent: None }
}
impl Glass {
    fn inner_margin(mut self, m: Margin) -> Self { self.margin = m; self }
    fn corner_radius(mut self, r: CornerRadius) -> Self { self.radius = r.nw; self }
    /// An accent outline (a field being typed into).
    fn stroke(mut self, s: Stroke) -> Self { if s.color != EDGE { self.accent = Some(s.color); } self }
    fn show<R>(self, ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
        let shadow_at = ui.painter().add(egui::Shape::Noop);
        let glass_at = ui.painter().add(egui::Shape::Noop);
        let mut f = egui::Frame::new().inner_margin(self.margin).corner_radius(CornerRadius::same(self.radius));
        if let Some(c) = self.accent {
            f = f.stroke(Stroke::new(1.0_f32, c));
        }
        let inner = f.show(ui, add);
        let rect = inner.response.rect;
        let contact = Shadow { offset: [0, 1], blur: 2, spread: 0, color: Color32::from_black_alpha(115) };
        ui.painter().set(shadow_at, egui::Shape::Vec(vec![
            egui::Shape::Rect(self.surface.shadow().as_shape(rect, CornerRadius::same(self.radius))),
            egui::Shape::Rect(contact.as_shape(rect, CornerRadius::same(self.radius))),
        ]));
        ui.painter().set(glass_at, crate::glass::pane(rect, self.radius as f32, self.surface.look()));
        inner
    }
}

/// A section caption: small, spaced capitals, faint.
fn caption(ui: &mut egui::Ui, text: &str) {
    let job = egui::text::LayoutJob::single_section(text.to_uppercase(), egui::TextFormat {
        font_id: strong(10.0), color: FAINT, extra_letter_spacing: 0.6, ..Default::default() });
    ui.label(job);
}

impl Ui {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let ctx = egui::Context::default();
        ctx.style_mut(|s| {
            s.interaction.tooltip_delay = 0.25;
            s.visuals = egui::Visuals::dark();
            s.visuals.window_fill = PANEL;
            s.visuals.panel_fill = PANEL;
            s.visuals.override_text_color = Some(INK);
            s.visuals.popup_shadow = Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(90) };
            s.visuals.window_stroke = Stroke::new(1.0_f32, EDGE);
            s.visuals.window_corner_radius = CornerRadius::same(6);
            s.spacing.item_spacing = vec2(2.0, 2.0);
            s.visuals.extreme_bg_color = FIELD;
            s.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, EDGE);
            s.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgba_premultiplied(46, 46, 46, 46));
            s.visuals.widgets.inactive.corner_radius = CornerRadius::same(R_CONTROL);
            s.visuals.widgets.hovered.corner_radius = CornerRadius::same(R_CONTROL);
            s.visuals.widgets.active.corner_radius = CornerRadius::same(R_CONTROL);
            s.visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
            s.visuals.selection.bg_fill = ACTIVE;
            s.visuals.window_corner_radius = CornerRadius::same(R_CONTROL);
            s.spacing.button_padding = vec2(8.0, 5.0);
            s.visuals.text_cursor.stroke = Stroke::new(1.5_f32, Color32::WHITE);
        });
        let mut painter = egui_wgpu::Renderer::new(device, format, None, 1, false);
        let (glass, resources) = crate::glass::Glass::new(device, format);
        painter.callback_resources.insert(resources);
        Self {
            ctx,
            painter,
            glass,
            start: Instant::now(),
            events: Vec::new(),
            modifiers: Default::default(),
            groups: Vec::new(),
            active: String::new(),
            status: String::new(),
            hint: String::new(),
            cursor: None,
            selection: None,
            sel_more: false,
            sel_sub: None,
            actions: Vec::new(),
            captured: false,
            outside_press: false,
            repaint_ms: None,
            frame_ms: 0.,
            hit: Vec::new(),
            icons: Default::default(),
            cluster: Vec::new(),
            view_menu: Vec::new(),
            view_open: false,
            options: Vec::new(),
            column: None,
        }
    }

    /// Regular, medium and semibold faces (Inter); medium/semibold fall
    /// back to regular when missing.
    pub fn set_fonts(&mut self, regular: Vec<u8>, medium: Option<Vec<u8>>, semibold: Option<Vec<u8>>) {
        let mut fonts = egui::FontDefinitions::default();
        // Inter lacks the arrows, checks and pencil the chrome uses: the
        // system faces stand behind it (egui's own fallbacks are not built in).
        for (key, path) in [("sys", "/System/Library/Fonts/SFNS.ttf"),
                            ("symbols", "/System/Library/Fonts/Apple Symbols.ttf")] {
            if let Ok(bytes) = std::fs::read(path) {
                fonts.font_data.insert(key.into(), Arc::new(egui::FontData::from_owned(bytes)));
                for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                    fonts.families.entry(family).or_default().push(key.into());
                }
            }
        }
        let base = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
        fonts.font_data.insert("ui".into(), Arc::new(egui::FontData::from_owned(regular)));
        fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "ui".into());
        for (name, bytes) in [("medium", medium), ("semibold", semibold)] {
            let key = if let Some(b) = bytes {
                fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_owned(b)));
                name.to_owned()
            } else { "ui".to_owned() };
            let mut fam = vec![key];
            fam.extend(base.iter().cloned());
            fonts.families.insert(egui::FontFamily::Name(name.into()), fam);
        }
        self.ctx.set_fonts(fonts);
    }

    /// ``groups``: per group, (id, label, shortcut, icon width, height, RGBA).
    pub fn set_tools(&mut self, groups: Vec<Vec<(String, String, String, usize, usize, Vec<u8>)>>) {
        self.groups = groups
            .into_iter()
            .map(|g| {
                g.into_iter()
                    .map(|(id, label, key, w, h, rgba)| {
                        let icon = (w > 0 && rgba.len() == w * h * 4).then(|| {
                            let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                            self.ctx.load_texture(&id, image, egui::TextureOptions::LINEAR)
                        });
                        Tool { id, label, key, icon }
                    })
                    .collect()
            })
            .collect();
    }

    pub fn set_icons(&mut self, icons: Vec<(String, usize, usize, Vec<u8>)>) {
        for (key, w, h, rgba) in icons {
            if w > 0 && rgba.len() == w * h * 4 {
                let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                let tex = self.ctx.load_texture(&key, image, egui::TextureOptions::LINEAR);
                self.icons.insert(key, tex);
            }
        }
    }
    /// rows: (key, parent index or -1, depth, name, kind, count, problems, active).
    pub fn set_tree(&mut self, rows: Vec<(String, i64, usize, String, String, u32, u32, bool)>, unfiled: u32) {
        let col = self.column.get_or_insert_with(|| Column {
            nodes: Vec::new(), unfiled: 0, expanded: Default::default(), filter: String::new(),
            filing: None, focus_filter: false, row_menu: None, inspector_title: String::new(),
            inspector: Vec::new(), highlighted: None,
        });
        let first = col.nodes.is_empty();
        let mut nodes: Vec<Node> = rows.into_iter().map(|(key, parent, depth, name, kind, count, problems, active)| Node {
            key, parent: (parent >= 0).then_some(parent as usize), depth, name, kind, count, problems, active,
            has_children: false,
        }).collect();
        for i in 0..nodes.len() {
            if let Some(p) = nodes[i].parent { nodes[p].has_children = true; }
        }
        if first {
            // Open the roots, and the way to the active node.
            for n in &nodes { if n.depth == 0 { col.expanded.insert(n.key.clone()); } }
            if let Some(mut i) = nodes.iter().position(|n| n.active) {
                while let Some(p) = nodes[i].parent { col.expanded.insert(nodes[p].key.clone()); i = p; }
            }
        }
        col.nodes = nodes;
        col.unfiled = unfiled;
    }
    pub fn set_inspector(&mut self, title: String, sections: Vec<(String, Vec<(String, String)>)>) {
        if let Some(col) = self.column.as_mut() {
            col.inspector_title = title;
            col.inspector = sections;
        }
    }
    /// "File to…": the tree takes the next pick; the filter gets the keys.
    pub fn start_filing(&mut self, what: String) {
        if let Some(col) = self.column.as_mut() {
            col.filing = Some(what);
            col.focus_filter = true;
            col.filter.clear();
        }
    }
    pub fn set_options(&mut self, options: Vec<(String, String, String, u8)>) { self.options = options; }
    pub fn set_cluster(&mut self, cluster: Vec<Vec<Item>>, view_menu: Vec<(String, Vec<Item>)>) {
        self.cluster = cluster;
        self.view_menu = view_menu;
    }

    /// Keyboard ownership: the chrome takes a key only while one of its
    /// fields has focus, or Escape while one of its popups is open.
    pub fn claims_key(&self, name: &str) -> bool {
        let filing = self.column.as_ref().is_some_and(|c| c.filing.is_some() || c.row_menu.is_some());
        (name == "Escape" && (self.view_open || self.sel_more || filing)) || self.ctx.wants_keyboard_input()
    }
    pub fn key(&mut self, name: &str, pressed: bool, mods: u8) -> bool {
        if name == "Escape" && (self.view_open || self.sel_more) {
            if pressed { self.view_open = false; self.sel_more = false; }
            return true;
        }
        if name == "Escape" {
            if let Some(col) = self.column.as_mut() {
                if col.row_menu.is_some() || col.filing.is_some() {
                    if pressed {
                        if col.row_menu.take().is_none() {
                            col.filing = None;
                            col.filter.clear();
                        }
                        self.ctx.memory_mut(|m| m.stop_text_input());
                    }
                    return true;
                }
            }
        }
        if !self.ctx.wants_keyboard_input() {
            return false;
        }
        if let Some(key) = egui::Key::from_name(name) {
            self.events.push(egui::Event::Key {
                key, physical_key: None, pressed, repeat: false, modifiers: mods_from(mods),
            });
        }
        true
    }
    pub fn text(&mut self, text: String) -> bool {
        if !self.ctx.wants_keyboard_input() {
            return false;
        }
        self.events.push(egui::Event::Text(text));
        true
    }

    pub fn set_active(&mut self, id: String) { self.active = id; }
    pub fn set_status(&mut self, status: String, hint: String) {
        self.status = status;
        self.hint = hint;
    }
    pub fn set_cursor_value(&mut self, caption: String, value: String, typing: bool, x: f32, y: f32) {
        self.cursor = (!caption.is_empty() || typing).then(|| (caption, value, typing, pos2(x, y)));
    }
    /// The canonical measurements box: fixed at the bottom right.
    fn measure_box(&self, ctx: &egui::Context, screen: Rect) {
        let Some((caption, value, typing, _)) = &self.cursor else { return };
        egui::Area::new(egui::Id::new("measure"))
            .pivot(egui::Align2::RIGHT_BOTTOM)
            .fixed_pos(pos2(self.free_right(ctx, screen) - 12.0, screen.max.y - 12.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                let edge = if *typing { ACCENT } else { EDGE };
                glass(Surface::Pill).stroke(Stroke::new(1.0_f32, edge))
                    .inner_margin(Margin::symmetric(10, 5)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(10.0, 0.0);
                        ui.label(egui::RichText::new(caption).size(12.0).color(DIM));
                        let (rect, _) = ui.allocate_exact_size(vec2(110.0, 16.0), Sense::hover());
                        let shown = if value.is_empty() && *typing { "|" } else { value.as_str() };
                        ui.painter().text(rect.right_center(), egui::Align2::RIGHT_CENTER, shown,
                            egui::FontId::monospace(13.0), if *typing { Color32::WHITE } else { INK });
                    });
                });
            });
    }
    pub fn set_selection(&mut self, summary: String, actions: Vec<(String, String, String, bool, bool)>) {
        if summary.is_empty() { self.sel_more = false; }
        self.selection = (!summary.is_empty()).then_some((summary, actions));
    }
    /// Where an area was last laid out, in points (tests and harnesses).
    pub fn area_rect(&self, name: &str) -> Option<(f32, f32, f32, f32)> {
        self.ctx.memory(|m| m.area_rect(egui::Id::new(name))).map(|r| (r.min.x, r.min.y, r.width(), r.height()))
    }
    pub fn take_actions(&mut self) -> Vec<String> { std::mem::take(&mut self.actions) }
    pub fn repaint_after_ms(&self) -> Option<f64> { self.repaint_ms }

    /// Only the interactive chrome takes the pointer; labels pass it through.
    fn over_chrome(&self, pos: Pos2) -> bool { self.hit.iter().any(|r| r.contains(pos)) }

    /// kind: "move" | "press" | "release" | "leave". Coordinates in logical
    /// points. Returns whether the chrome takes the event (the view must
    /// then neither hover, pick, nor navigate).
    pub fn pointer(&mut self, kind: &str, x: f32, y: f32, button: u8, mods: u8) -> bool {
        let pos = pos2(x, y);
        self.modifiers = mods_from(mods);
        let button = match button {
            2 => egui::PointerButton::Secondary,
            3 => egui::PointerButton::Middle,
            _ => egui::PointerButton::Primary,
        };
        match kind {
            "leave" => {
                self.events.push(egui::Event::PointerGone);
                false
            }
            "press" => {
                self.captured = self.over_chrome(pos);
                // A press anywhere else closes the menus, ends typing in a
                // field, and is the model's.
                if !self.captured {
                    self.view_open = false;
                    self.sel_more = false;
                    if let Some(col) = self.column.as_mut() { col.row_menu = None; }
                    self.ctx.memory_mut(|m| m.stop_text_input());
                }
                self.outside_press = !self.captured;
                if self.captured {
                    self.events.push(egui::Event::PointerMoved(pos));
                    self.events.push(egui::Event::PointerButton { pos, button, pressed: true, modifiers: self.modifiers });
                }
                self.captured
            }
            "wheel" => {
                // x, y: position; button carries nothing; deltas come via wheel().
                false
            }
            "release" => {
                self.outside_press = false;
                let was = std::mem::replace(&mut self.captured, false);
                if was {
                    self.events.push(egui::Event::PointerButton { pos, button, pressed: false, modifiers: self.modifiers });
                }
                was
            }
            _ => {
                if self.outside_press {
                    return false;
                }
                self.events.push(egui::Event::PointerMoved(pos));
                self.captured || self.over_chrome(pos)
            }
        }
    }

    /// Wheel over the chrome scrolls it; elsewhere it is the model's.
    pub fn wheel(&mut self, x: f32, y: f32, dx: f32, dy: f32) -> bool {
        let pos = pos2(x, y);
        if !self.over_chrome(pos) {
            return false;
        }
        self.events.push(egui::Event::PointerMoved(pos));
        self.events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point, delta: vec2(dx, dy), modifiers: self.modifiers,
        });
        true
    }

    fn palette(&mut self, ctx: &egui::Context, screen: Rect) {
        let mut clicked = None;
        // Two columns while they fit above the status line; wider, never
        // scrolled or folded, when the window is short (council 2026-11-22).
        let room = screen.height() - 12.0 - 60.0;
        let tall = |cols: usize| self.groups.iter().map(|g| g.len().div_ceil(cols) as f32 * 32.0).sum::<f32>()
            + self.groups.len().saturating_sub(1) as f32 * 9.0 + 8.0;
        let cols = (2..=5).find(|c| tall(*c) <= room).unwrap_or(5);
        egui::Area::new(egui::Id::new("palette"))
            .fixed_pos(pos2(12.0, 12.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                frame().inner_margin(Margin::same(4)).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
                    for (gi, group) in self.groups.iter().enumerate() {
                        if gi > 0 {
                            let (r, _) = ui.allocate_exact_size(vec2(cols as f32 * 32.0 - 2.0, 7.0), Sense::hover());
                            ui.painter().hline(r.x_range().shrink(2.0), r.center().y, Stroke::new(1.0_f32, EDGE));
                        }
                        for pair in group.chunks(cols) {
                            ui.horizontal(|ui| {
                                for tool in pair {
                                    if tool_button(ui, tool, tool.id == self.active) {
                                        clicked = Some(tool.id.clone());
                                    }
                                }
                            });
                        }
                    }
                });
            });
        if let Some(id) = clicked {
            self.active = id.clone();
            self.actions.push(format!("tool:{id}"));
        }
    }

    fn status_line(&self, ctx: &egui::Context, screen: Rect) {
        if self.status.is_empty() && self.hint.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("status"))
            .fixed_pos(pos2(12.0, screen.max.y - 36.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                glass(Surface::Pill).inner_margin(Margin::symmetric(10, 5)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(10.0, 0.0);
                        if !self.status.is_empty() {
                            ui.label(egui::RichText::new(&self.status).font(strong(12.0)).color(INK));
                        }
                        if !self.hint.is_empty() {
                            ui.label(egui::RichText::new(&self.hint).font(body()).color(DIM));
                        }
                    });
                });
            });
    }

    fn selection_bar(&mut self, ctx: &egui::Context, screen: Rect) {
        let Some((summary, actions)) = &self.selection else { return };
        let mut clicked = None;
        let mut toggle_more = false;
        let has_more = actions.iter().any(|a| a.4);
        let bar = egui::Area::new(egui::Id::new("selection"))
            .pivot(egui::Align2::CENTER_BOTTOM)
            .fixed_pos(pos2(screen.center().x, screen.max.y - 52.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Pill).corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(3)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(summary).font(meta()).color(FAINT));
                        ui.add_space(8.0);
                        let (r, _) = ui.allocate_exact_size(vec2(1.0, 28.0), Sense::hover());
                        ui.painter().vline(r.center().x, r.y_range().shrink(6.0), Stroke::new(1.0_f32, EDGE));
                        ui.add_space(4.0);
                        for (id, label, key, enabled, more) in actions {
                            if !*more && text_button(ui, label, key, *enabled) {
                                clicked = Some(id.clone());
                            }
                        }
                        if has_more && text_button(ui, if self.sel_more { "More ▴" } else { "More ▾" }, "", true) {
                            toggle_more = true;
                        }
                    });
                });
            });
        if toggle_more {
            self.sel_more = !self.sel_more;
        }
        if self.sel_more {
            let anchor = bar.response.rect;
            egui::Area::new(egui::Id::new("sel_more"))
                .pivot(egui::Align2::RIGHT_BOTTOM)
                .fixed_pos(pos2(anchor.max.x, anchor.min.y - 6.0))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    glass(Surface::Raised).inner_margin(Margin::same(4)).show(ui, |ui| {
                        ui.set_width(230.0);
                        ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                        let mut seen: Vec<String> = Vec::new();
                        for (id, label, key, enabled, more) in actions {
                            if !*more { continue; }
                            if let Some((prefix, _)) = label.split_once(" ▸ ") {
                                // One row opens the submenu; its items live there.
                                if seen.iter().any(|p| p == prefix) { continue; }
                                seen.push(prefix.to_owned());
                                let open = self.sel_sub.as_ref().is_some_and(|(p, _)| p == prefix);
                                let item = (String::new(), prefix.to_owned(), "›".to_owned(), String::new(), -1i8, true);
                                let (resp, rect) = menu_row_resp(ui, &item);
                                if open {
                                    ui.painter().rect_filled(rect, CornerRadius::same(R_ROW), HOVER);
                                }
                                if resp.hovered() || resp.clicked() {
                                    self.sel_sub = Some((prefix.to_owned(), rect));
                                }
                                continue;
                            }
                            let item = (id.clone(), label.clone(), key.clone(), String::new(), -1i8, *enabled);
                            let (resp, _) = menu_row_resp(ui, &item);
                            if resp.hovered() { self.sel_sub = None; }
                            if resp.clicked() && *enabled {
                                clicked = Some(id.clone());
                            }
                        }
                    });
                });
        }
        if let (true, Some((prefix, row))) = (self.sel_more, self.sel_sub.clone()) {
            egui::Area::new(egui::Id::new("sel_sub"))
                .fixed_pos(pos2(row.max.x + 8.0, row.min.y - 4.0))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    glass(Surface::Raised).inner_margin(Margin::same(4)).show(ui, |ui| {
                        ui.set_width(170.0);
                        ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                        for (id, label, key, enabled, _) in actions {
                            if let Some((p, rest)) = label.split_once(" ▸ ") {
                                if p != prefix { continue; }
                                let item = (id.clone(), rest.to_owned(), key.clone(), String::new(), -1i8, *enabled);
                                if menu_row(ui, &item) && *enabled {
                                    clicked = Some(id.clone());
                                }
                            }
                        }
                    });
                });
        }
        if let Some(id) = clicked {
            self.sel_more = false;
            self.sel_sub = None;
            self.actions.push(format!("cmd:{id}"));
        }
        if !self.sel_more {
            self.sel_sub = None;
        }
    }

    fn top_right(&mut self, ctx: &egui::Context, screen: Rect) {
        if self.cluster.is_empty() {
            return;
        }
        let mut clicked: Option<String> = None;
        let mut toggle_menu = false;
        let chip = egui::Area::new(egui::Id::new("cluster"))
            .pivot(egui::Align2::RIGHT_TOP)
            .fixed_pos(pos2(screen.max.x - 12.0, 12.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                frame().inner_margin(Margin::same(3)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
                        for (gi, group) in self.cluster.iter().enumerate() {
                            if gi > 0 {
                                let (r, _) = ui.allocate_exact_size(vec2(7.0, 26.0), Sense::hover());
                                ui.painter().vline(r.center().x, r.y_range().shrink(5.0), Stroke::new(1.0_f32, EDGE));
                            }
                            for item in group {
                                if item_button(ui, item, self.icons.get(&item.3)) {
                                    clicked = Some(item.0.clone());
                                }
                            }
                        }
                        if !self.view_menu.is_empty() {
                            let label = if self.view_open { "View ▴" } else { "View ▾" };
                            let chip = (String::new(), label.to_owned(), "Views, projection, style".to_owned(),
                                        String::new(), if self.view_open { 1 } else { -1 }, true);
                            if item_button(ui, &chip, None) {
                                toggle_menu = true;
                            }
                        }
                    });
                });
            });
        if toggle_menu {
            self.view_open = !self.view_open;
        }
        if self.view_open {
            let top = chip.response.rect.max.y + 6.0;
            let mut close = false;
            egui::Area::new(egui::Id::new("view_menu"))
                .pivot(egui::Align2::RIGHT_TOP)
                .fixed_pos(pos2(screen.max.x - 12.0, top))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    glass(Surface::Raised).inner_margin(Margin::same(6)).show(ui, |ui| {
                        ui.set_width(236.0);
                        ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
                        for (si, (title, items)) in self.view_menu.iter().enumerate() {
                            if si > 0 {
                                ui.add_space(4.0);
                            }
                            caption(ui, title);
                            let icons_only = items.iter().all(|i| self.icons.contains_key(&i.3));
                            if icons_only {
                                ui.horizontal_wrapped(|ui| {
                                    for item in items {
                                        if item_button(ui, item, self.icons.get(&item.3)) {
                                            clicked = Some(item.0.clone());
                                        }
                                    }
                                });
                            } else {
                                for item in items {
                                    if menu_row(ui, item) {
                                        clicked = Some(item.0.clone());
                                        close = !item.4.is_positive() && item.4 < 0;
                                    }
                                }
                            }
                        }
                    });
                });
            if close {
                self.view_open = false;
            }
        }
        if let Some(id) = clicked {
            self.actions.push(format!("act:{id}"));
        }
    }

    /// The active tool's modes and locks, right of the palette's top row
    /// (council 2026-11-22): left edge fixed, only what matters now.
    fn options_strip(&mut self, ctx: &egui::Context) {
        if self.options.is_empty() {
            return;
        }
        let left = ctx.memory(|m| m.area_rect(egui::Id::new("palette"))).map_or(80.0, |r| r.max.x) + 8.0;
        let mut clicked = None;
        egui::Area::new(egui::Id::new("options"))
            .fixed_pos(pos2(left, 12.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                frame().inner_margin(Margin::same(3)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
                        for (id, label, key, state) in &self.options {
                            if *state == 2 {
                                let (rect, _) = ui.allocate_exact_size(
                                    vec2(ui.painter().layout_no_wrap(label.clone(), egui::FontId::proportional(12.0), INK).size().x + 18.0, 26.0),
                                    Sense::hover());
                                ui.painter().rect_stroke(rect.shrink(1.0), CornerRadius::same(5),
                                    Stroke::new(1.0_f32, Color32::from_rgb(0xD2, 0x96, 0x28)), egui::StrokeKind::Inside);
                                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label,
                                    egui::FontId::proportional(12.0), Color32::from_rgb(0xF0, 0xC0, 0x60));
                                continue;
                            }
                            let text = if key.is_empty() { label.clone() } else { format!("{label}  {key}") };
                            let item = (id.clone(), text, String::new(), String::new(), *state as i8, true);
                            if item_button(ui, &item, None) {
                                clicked = Some(id.clone());
                            }
                        }
                    });
                });
            });
        if let Some(id) = clicked {
            self.actions.push(format!("opt:{id}"));
        }
    }

    /// The right edge of the free viewport (left of the column).
    fn free_right(&self, ctx: &egui::Context, screen: Rect) -> f32 {
        if self.column.is_none() { return screen.max.x; }
        ctx.memory(|m| m.area_rect(egui::Id::new("column"))).map_or(screen.max.x, |r| r.min.x)
    }

    fn right_column(&mut self, ctx: &egui::Context, screen: Rect) {
        let Some(col) = self.column.as_mut() else { return };
        const W: f32 = 300.0;
        const ROW: f32 = 22.0;
        let top = ctx.memory(|m| m.area_rect(egui::Id::new("cluster"))).map_or(12.0, |r| r.max.y + 8.0);
        let height = (screen.max.y - 12.0 - top).max(200.0);
        let mut actions: Vec<String> = Vec::new();
        egui::Area::new(egui::Id::new("column"))
            .fixed_pos(pos2(screen.max.x - 12.0 - W, top))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Panel).inner_margin(Margin::same(8)).show(ui, |ui| {
                    ui.set_width(W - 12.0);
                    ui.set_height(height - 12.0);
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    // Header: title, Unfiled chip.
                    ui.horizontal(|ui| {
                        caption(ui, "Project");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if col.unfiled > 0 {
                                let item = (String::new(), format!("{} unfiled", col.unfiled), "Select the unfiled items".to_owned(),
                                            String::new(), -1, true);
                                if amber_button(ui, &item.1) { actions.push("unfiled".into()); }
                            }
                        });
                    });
                    ui.add_space(4.0);
                    if let Some(what) = &col.filing {
                        let banner = format!("File {what} to…  click a node, or type and press Enter");
                        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
                        ui.painter().rect_filled(r, CornerRadius::same(R_ROW), ACTIVE);
                        ui.painter().text(r.left_center() + vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, banner,
                            egui::FontId::proportional(11.5), Color32::WHITE);
                        ui.add_space(4.0);
                    }
                    let edit = egui::TextEdit::singleline(&mut col.filter)
                        .hint_text(egui::RichText::new(if col.filing.is_some() { "Node name or kind" } else { "Filter nodes" }).color(FAINT))
                        .margin(Margin::symmetric(8, 5))
                        .desired_width(f32::INFINITY)
                        .font(egui::FontId::proportional(12.0));
                    let resp = ui.add(edit);
                    if col.focus_filter {
                        resp.request_focus();
                        col.focus_filter = false;
                    }
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.add_space(4.0);
                    // Rows: filter (with ancestors) or expansion.
                    let needle = col.filter.to_lowercase();
                    let visible: Vec<usize> = if needle.is_empty() {
                        let mut out = Vec::new();
                        let mut hidden_below: Option<usize> = None;
                        for (i, n) in col.nodes.iter().enumerate() {
                            if let Some(d) = hidden_below { if n.depth > d { continue; } hidden_below = None; }
                            out.push(i);
                            if n.has_children && !col.expanded.contains(&n.key) { hidden_below = Some(n.depth); }
                        }
                        out
                    } else {
                        let mut keep = vec![false; col.nodes.len()];
                        for (i, n) in col.nodes.iter().enumerate() {
                            if n.name.to_lowercase().contains(&needle) || n.kind.to_lowercase().contains(&needle) {
                                let mut j = Some(i);
                                while let Some(k) = j { keep[k] = true; j = col.nodes[k].parent; }
                            }
                        }
                        (0..col.nodes.len()).filter(|i| keep[*i]).collect()
                    };
                    if enter {
                        if let (Some(_), Some(i)) = (&col.filing, visible.iter().copied()
                            .find(|i| col.nodes[*i].name.to_lowercase().contains(&needle) || needle.is_empty())) {
                            actions.push(format!("file:{}", col.nodes[i].key));
                            col.filing = None;
                            col.filter.clear();
                        }
                    }
                    let tree_h = ((visible.len().max(1) as f32) * (ROW + 0.0) + 4.0).min(ui.available_height() * 0.6).max(66.0);
                    if col.nodes.is_empty() {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new("No project structure yet. Proposing one from the tags is in the Structure panel (Window menu) for now.")
                            .size(11.5).color(DIM));
                    }
                    egui::ScrollArea::vertical().id_salt("tree").max_height(tree_h).auto_shrink([false, false])
                        .show_rows(ui, ROW, visible.len(), |ui, range| {
                            for &i in &visible[range] {
                                let n = &col.nodes[i];
                                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
                                let lit = col.highlighted.as_deref() == Some(n.key.as_str());
                                let filing = col.filing.is_some();
                                if lit {
                                    // Selected: accent wash and a 2 × 12 mark at the leading edge.
                                    ui.painter().rect_filled(rect, CornerRadius::same(R_ROW), ACTIVE);
                                    ui.painter().rect_filled(Rect::from_center_size(pos2(rect.min.x + 1.0, rect.center().y), vec2(2.0, 12.0)),
                                        CornerRadius::same(1), ACCENT);
                                } else if resp.hovered() {
                                    ui.painter().rect_filled(rect, CornerRadius::same(R_ROW), if filing { ACTIVE } else { HOVER });
                                }
                                let x0 = rect.min.x + 4.0 + n.depth as f32 * 14.0;
                                let tri = Rect::from_min_size(pos2(x0, rect.min.y), vec2(14.0, ROW));
                                if n.has_children {
                                    let open = col.expanded.contains(&n.key) || !needle.is_empty();
                                    ui.painter().text(tri.center(), egui::Align2::CENTER_CENTER, if open { "▾" } else { "▸" }, meta(), FAINT);
                                }
                                // Right: problems (dot + number) and count, each in a fixed 40-pt column.
                                let cy = rect.center().y;
                                let warn_col = rect.max.x - 6.0;
                                if n.problems > 0 {
                                    let g = ui.painter().layout_no_wrap(n.problems.to_string(), meta(), WARN);
                                    let w = g.size().x;
                                    ui.painter().galley(pos2(warn_col - w, cy - g.size().y / 2.0), g, WARN);
                                    ui.painter().circle_filled(pos2(warn_col - w - 6.0, cy), 2.5, WARN);
                                }
                                let count_col = warn_col - 44.0;
                                let g = ui.painter().layout_no_wrap(n.count.to_string(), meta(), DIM);
                                ui.painter().galley(pos2(count_col - g.size().x, cy - g.size().y / 2.0), g, DIM);
                                let right = count_col - 44.0;
                                // Name, then kind, both cut short before the numbers.
                                let ink = if lit || n.active { Color32::WHITE } else { INK };
                                let font = if n.active { strong(12.0) } else { body() };
                                let name = if n.active { format!("✎ {}", n.name) } else { n.name.clone() };
                                let one_line = |text: String, font: egui::FontId, color: Color32, width: f32| {
                                    let mut job = egui::text::LayoutJob::single_section(text, egui::TextFormat {
                                        font_id: font, color, ..Default::default() });
                                    job.wrap.max_width = width.max(10.0);
                                    job.wrap.max_rows = 1;
                                    job.wrap.break_anywhere = true;
                                    job
                                };
                                let g = ui.painter().layout_job(one_line(name, font, ink, right - x0 - 16.0));
                                let name_w = g.size().x;
                                ui.painter().galley(pos2(x0 + 16.0, cy - g.size().y / 2.0), g, ink);
                                let kx = x0 + 16.0 + name_w + 8.0;
                                if !n.kind.is_empty() && kx + 30.0 < right {
                                    let g = ui.painter().layout_job(one_line(n.kind.clone(), meta(), FAINT, right - kx));
                                    ui.painter().galley(pos2(kx, cy - g.size().y / 2.0), g, FAINT);
                                }
                                if resp.clicked() {
                                    let on_tri = n.has_children && resp.interact_pointer_pos().is_some_and(|p| tri.contains(p));
                                    if on_tri {
                                        if !col.expanded.remove(&n.key) { col.expanded.insert(n.key.clone()); }
                                    } else if col.filing.is_some() {
                                        actions.push(format!("file:{}", n.key));
                                        col.filing = None;
                                        col.filter.clear();
                                    } else {
                                        col.highlighted = Some(n.key.clone());
                                        actions.push(format!("node:{}", n.key));
                                    }
                                }
                                if resp.double_clicked() && col.filing.is_none() {
                                    actions.push(format!("isolate:{}", n.key));
                                }
                                if resp.secondary_clicked() {
                                    col.row_menu = resp.interact_pointer_pos().map(|p| (n.key.clone(), p));
                                }
                            }
                        });
                    // Inspector.
                    ui.add_space(6.0);
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0_f32, EDGE));
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().id_salt("inspector").auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing = vec2(0.0, 3.0);
                        if col.inspector_title.is_empty() {
                            ui.label(egui::RichText::new("Nothing selected").font(body()).color(FAINT));
                            return;
                        }
                        ui.label(egui::RichText::new(&col.inspector_title).font(title()).color(INK));
                        ui.add_space(4.0);
                        for (title, rows) in &col.inspector {
                            let folded = title.starts_with("Export");
                            let body = |ui: &mut egui::Ui| {
                                for (label, value) in rows {
                                    // Value right-aligned; the label takes what is left, cut short.
                                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
                                    let v = ui.painter().layout_no_wrap(value.clone(), egui::FontId::proportional(12.0), INK);
                                    let vw = v.size().x.min(r.width() * 0.6);
                                    ui.painter().galley(pos2(r.max.x - 4.0 - vw, r.center().y - v.size().y / 2.0), v, INK);
                                    let mut job = egui::text::LayoutJob::single_section(label.clone(), egui::TextFormat {
                                        font_id: egui::FontId::proportional(11.5), color: DIM, ..Default::default() });
                                    job.wrap.max_width = (r.width() - vw - 14.0).max(20.0);
                                    job.wrap.max_rows = 1;
                                    job.wrap.break_anywhere = true;
                                    let l = ui.painter().layout_job(job);
                                    ui.painter().galley(pos2(r.min.x, r.center().y - l.size().y / 2.0), l, DIM);
                                }
                            };
                            if folded {
                                egui::CollapsingHeader::new(egui::RichText::new(title.to_uppercase()).font(strong(10.0)).color(FAINT))
                                    .id_salt(title).default_open(false).show(ui, body);
                            } else {
                                ui.add_space(8.0);
                                caption(ui, title);
                                body(ui);
                            }
                        }
                    });
                });
            });
        // The row menu, near the pointer.
        if let Some((key, at)) = col.row_menu.clone() {
            let mut close = false;
            egui::Area::new(egui::Id::new("row_menu"))
                .fixed_pos(at + vec2(4.0, 4.0))
                .order(egui::Order::Tooltip)
                .show(ctx, |ui| {
                    glass(Surface::Raised).inner_margin(Margin::same(4)).show(ui, |ui| {
                        ui.set_width(190.0);
                        ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                        for (id, label) in [("active", "Draw into this node"), ("filehere", "File selection here"),
                                            ("node", "Select its items"), ("isolate", "Isolate")] {
                            let item = (id.to_owned(), label.to_owned(), String::new(), String::new(), -1i8, true);
                            if menu_row(ui, &item) {
                                actions.push(format!("{id}:{key}"));
                                close = true;
                            }
                        }
                    });
                });
            if close { col.row_menu = None; }
        }
        for a in actions {
            self.actions.push(format!("tree:{a}"));
        }
    }

    fn cursor_value(&self, ctx: &egui::Context, screen: Rect) {
        let Some((caption, value, typing, at)) = &self.cursor else { return };
        if !screen.contains(*at) {
            return;
        }
        // Below-right of the cursor; flipped when it would leave the view.
        let size = vec2(150.0, 24.0);
        let mut pos = *at + vec2(18.0, 20.0);
        if pos.x + size.x > screen.max.x - 8.0 { pos.x = at.x - 18.0 - size.x; }
        if pos.y + size.y > screen.max.y - 48.0 { pos.y = at.y - 20.0 - size.y; }
        egui::Area::new(egui::Id::new("cursor_value"))
            .fixed_pos(pos)
            .order(egui::Order::Tooltip)
            .interactable(false)
            .show(ctx, |ui| {
                let edge = if *typing { ACCENT } else { EDGE };
                glass(Surface::Pill).stroke(Stroke::new(1.0_f32, edge))
                    .inner_margin(Margin::symmetric(7, 3)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
                        if !caption.is_empty() {
                            ui.label(egui::RichText::new(caption).size(11.0).color(DIM));
                        }
                        let shown = if value.is_empty() && *typing { "|" } else { value.as_str() };
                        ui.label(egui::RichText::new(shown).size(12.5).monospace()
                            .color(if *typing { Color32::WHITE } else { INK }));
                    });
                });
            });
    }

    /// Lay out, tessellate and draw the chrome over ``target`` (already
    /// holding the frame). Called last in the frame, after the overlay.
    pub fn paint(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, e: &mut wgpu::CommandEncoder,
                 frame: &wgpu::Texture, target: &wgpu::TextureView, size: [u32; 2], scale: f32) {
        // The glass behind the chrome: blur the finished model image first.
        if let Some(res) = self.painter.callback_resources.get_mut::<crate::glass::Resources>() {
            self.glass.prepare(device, e, frame, res);
        }
        let begin = Instant::now();
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(size[0] as f32 / scale, size[1] as f32 / scale));
        let mut raw = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(self.start.elapsed().as_secs_f64()),
            modifiers: self.modifiers,
            events: std::mem::take(&mut self.events),
            focused: true,
            ..Default::default()
        };
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(scale);
        let ctx = self.ctx.clone();
        let out = ctx.run(raw, |ctx| {
            self.palette(ctx, screen);
            self.status_line(ctx, screen);
            self.selection_bar(ctx, screen);
            self.top_right(ctx, screen);
            self.options_strip(ctx);
            self.right_column(ctx, screen);
            self.measure_box(ctx, screen);
            if self.cursor.as_ref().is_some_and(|c| c.2) {
                self.cursor_value(ctx, screen);
            }
        });
        let shown = [("palette", !self.groups.is_empty()), ("selection", self.selection.is_some()),
                     ("cluster", !self.cluster.is_empty()), ("view_menu", self.view_open),
                     ("options", !self.options.is_empty()), ("column", self.column.is_some()),
                     ("sel_more", self.sel_more && self.selection.is_some()),
                     ("sel_sub", self.sel_more && self.sel_sub.is_some()),
                     ("row_menu", self.column.as_ref().is_some_and(|c| c.row_menu.is_some()))];
        self.hit = shown.iter()
            .filter(|(_, on)| *on)
            .filter_map(|(n, _)| ctx.memory(|m| m.area_rect(egui::Id::new(*n))))
            .collect();
        self.repaint_ms = out
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|v| v.repaint_delay)
            .filter(|d| d.as_secs() < 60)
            .map(|d| d.as_secs_f64() * 1000.);
        let jobs = ctx.tessellate(out.shapes, out.pixels_per_point);
        for (id, delta) in &out.textures_delta.set {
            self.painter.update_texture(device, queue, *id, delta);
        }
        let screen_desc = egui_wgpu::ScreenDescriptor { size_in_pixels: size, pixels_per_point: out.pixels_per_point };
        let extra = self.painter.update_buffers(device, queue, e, &jobs, &screen_desc);
        if !extra.is_empty() {
            queue.submit(extra);
        }
        {
            let mut pass = e
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("ui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            self.painter.render(&mut pass, &jobs, &screen_desc);
        }
        for id in &out.textures_delta.free {
            self.painter.free_texture(id);
        }
        self.frame_ms = begin.elapsed().as_secs_f64() * 1000.;
    }
}

fn mods_from(mods: u8) -> egui::Modifiers {
    egui::Modifiers {
        alt: mods & 1 != 0,
        ctrl: mods & 2 != 0,
        shift: mods & 4 != 0,
        mac_cmd: mods & 8 != 0,
        command: mods & 8 != 0,
    }
}

/// A cluster or icon-grid button: its icon if it has one, else its label.
/// Checked items stay lit (latched toggles).
/// A bar control (top row, options strip): icon 28 × 28 or text, 28 high.
/// Checked items stay lit (latched toggles), the same way as a current tool.
fn item_button(ui: &mut egui::Ui, item: &Item, icon: Option<&egui::TextureHandle>) -> bool {
    let (_, label, tip, _, checked, enabled) = item;
    let width = match icon {
        Some(_) => 28.0,
        None => ui.painter().layout_no_wrap(label.clone(), body(), INK).size().x + 16.0,
    };
    let sense = if *enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 28.0), sense);
    let st = state_of(&resp, *checked == 1, *enabled);
    paint_state(ui, rect, st, R_CONTROL);
    let ink = ink_of(st);
    match icon {
        Some(tex) => { egui::Image::new(tex).tint(ink).paint_at(ui, rect.shrink(4.0)); }
        None => {
            let ink = if st == State::Idle { INK } else { ink };
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, body(), ink);
        }
    }
    let tip = if tip.is_empty() { label.clone() } else { tip.clone() };
    resp.on_hover_text(tip).clicked()
}

fn menu_row(ui: &mut egui::Ui, item: &Item) -> bool {
    menu_row_resp(ui, item).0.clicked()
}

/// A menu row, 24 high: optional check lane (only when the menu has checks:
/// the caller marks that with item.3 == "check"), label, shortcut or › right.
fn menu_row_resp(ui: &mut egui::Ui, item: &Item) -> (egui::Response, Rect) {
    let (_, label, tip, lane, checked, enabled) = item;
    let sense = if *enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), sense);
    let st = state_of(&resp, false, *enabled);
    paint_state(ui, rect, st, R_ROW);
    let ink = if *enabled { INK } else { OFF };
    let x = if lane == "check" || *checked >= 0 { 24.0 } else { 10.0 };
    if *checked == 1 {
        ui.painter().text(pos2(rect.min.x + 12.0, rect.center().y), egui::Align2::CENTER_CENTER, "✓", body(), ACCENT);
    }
    ui.painter().text(pos2(rect.min.x + x, rect.center().y), egui::Align2::LEFT_CENTER, label, body(), ink);
    if !tip.is_empty() {
        ui.painter().text(pos2(rect.max.x - 10.0, rect.center().y), egui::Align2::RIGHT_CENTER, tip, meta(), FAINT);
    }
    (resp, rect)
}

/// A warning chip: amber text on an amber wash, no outline.
fn amber_button(ui: &mut egui::Ui, label: &str) -> bool {
    let g = ui.painter().layout_no_wrap(label.to_owned(), strong(11.0), WARN);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 16.0, 20.0), Sense::click());
    let fill = if resp.is_pointer_button_down_on() {
        Color32::from_rgba_premultiplied(54, 40, 16, 56)
    } else if resp.hovered() {
        Color32::from_rgba_premultiplied(44, 33, 13, 46)
    } else { WARN_FILL };
    ui.painter().rect_filled(rect, CornerRadius::same(10), fill);
    ui.painter().galley(rect.center() - g.size() / 2.0, g, WARN);
    resp.clicked()
}

/// A text command in a bar, 28 high, 8 pt padding.
fn text_button(ui: &mut egui::Ui, label: &str, key: &str, enabled: bool) -> bool {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), body(), INK);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 16.0, 28.0), sense);
    let st = state_of(&resp, false, enabled);
    paint_state(ui, rect, st, R_CONTROL);
    let ink = if enabled { INK } else { OFF };
    ui.painter().galley(pos2(rect.min.x + 8.0, rect.center().y - galley.size().y / 2.0), galley, ink);
    let resp = if key.is_empty() { resp } else { resp.on_hover_text(key) };
    resp.clicked()
}

/// A palette tool: 30 × 30 cell, glyph about 18 pt (the icon art carries
/// its own margin), idle INK_2, hover INK, current white on accent.
fn tool_button(ui: &mut egui::Ui, tool: &Tool, active: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
    let st = state_of(&resp, active, true);
    paint_state(ui, rect, st, R_CONTROL);
    let tint = ink_of(st);
    match &tool.icon {
        Some(tex) => {
            egui::Image::new(tex).tint(tint).paint_at(ui, rect.shrink(3.0));
        }
        None => {
            let letter: String = tool.label.chars().take(2).collect();
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, letter, meta(), tint);
        }
    }
    let tip = if tool.key.is_empty() { tool.label.clone() } else { format!("{}    {}", tool.label, tool.key) };
    resp.on_hover_text(tip).clicked()
}
