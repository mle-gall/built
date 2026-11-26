//! Project tabs in the top bar, beside the window buttons (shell design
//! option B, "agreed for now"; views/wgpu_tabs.py owns the data).
//!
//! The tabs are flat text. The front tab has INK text and a 2 pt accent line
//! on the bar's bottom edge. `•` after a name means unsaved, and `×`
//! replaces it on hover. A middle click closes a tab. Hovering a tab in the
//! back shows its last picture, its path and when it was saved. A link
//! source shows as a ⛓ tab right of its host.
//!
//! When the bar runs short, tabs shrink to 96 pt, then the rest go under
//! `›› n`. That menu also lists the projects in the app's other windows.
//! Right-clicking a tab brings it to the front, then opens the tab menu.
//! Scenes never come here: they keep their own ‹ Scene ▾ › control.
//!
//! Actions: `ptab:activate:<i>`, `ptab:close:<i>`, `ptab:new`,
//! `ptab:menu:<verb>:<i>`, `ptab:goto:<id>` (a project elsewhere).

use super::*;

/// One tab: (id, name, front, unsaved, link source, tip, picture key).
pub type TabData = (String, String, bool, bool, bool, String, String);

#[derive(Default)]
pub struct Strip {
    pub tabs: Vec<TabData>,
    /// Projects in the app's other windows: (id, name, window).
    pub elsewhere: Vec<(String, String, String)>,
    /// "Move to new window" is offered (the native app).
    pub can_move: bool,
    /// The picture on hover is on (Preferences, the tab menu).
    pub pictures: bool,
    /// The tab menu: which tab, and where.
    menu: Option<(usize, Pos2)>,
    /// The ›› n menu, under its button.
    overflow: Option<Rect>,
    /// The tab under the pointer and its rect (the hover card).
    hover: Option<(usize, Rect)>,
    /// Tabs left out of the strip (under ›› n).
    hidden: Vec<usize>,
    /// Owner of the last model image that went through the chrome blur passes.
    pub painted_doc: Option<String>,
    photos: std::collections::HashMap<String, (wgpu::Texture, Option<egui::TextureId>)>,
    /// Each tab's rect as last laid out, by index (None: under ›› n).
    pub rects: Vec<Option<Rect>>,
}

/// A tab's narrowest width before the strip overflows, and its padding:
/// name at 10 from the left, 22 on the right for • or ×.
const TAB_MIN_W: f32 = 96.0;
const PAD_L: f32 = 10.0;
const PAD_R: f32 = 24.0;
const LINK_W: f32 = 16.0;
const PLUS_W: f32 = 28.0;
const MORE_W: f32 = 46.0;
/// The picture's width in the hover card.
const PICTURE_W: f32 = 264.0;

impl Strip {
    pub fn menus_open(&self) -> bool { self.menu.is_some() || self.overflow.is_some() }
    pub fn menu_open(&self) -> bool { self.menu.is_some() }
    pub fn overflow_open(&self) -> bool { self.overflow.is_some() }
    pub fn close_menus(&mut self) { self.menu = None; self.overflow = None; }

    /// The strip inside the top bar's left group, at most `max_w` wide.
    /// Returns the actions clicked.
    pub fn show(&mut self, ui: &mut egui::Ui, max_w: f32) -> Vec<String> {
        let mut out = Vec::new();
        let n = self.tabs.len();
        let natural: Vec<f32> = self.tabs.iter().map(|t| {
            // Measured in the font it is drawn in (the front tab's is medium).
            let font = if t.2 { strong(TYPE_BODY) } else { body() };
            let w = ui.painter().layout_no_wrap(t.1.clone(), font, INK).size().x;
            w + PAD_L + PAD_R + if t.4 { LINK_W } else { 0.0 }
        }).collect();
        // Fit: natural widths, else shrink each toward 96 pt, else overflow.
        let room = (max_w - PLUS_W).max(TAB_MIN_W);
        let mut widths = natural.clone();
        let total: f32 = natural.iter().sum();
        let mut shown: Vec<usize> = (0..n).collect();
        if total > room {
            let share = room / n.max(1) as f32;
            widths = natural.iter().map(|w| w.min(share.max(TAB_MIN_W))).collect();
            if widths.iter().sum::<f32>() > room {
                let fit = (((room - MORE_W) / TAB_MIN_W).floor() as usize).max(1).min(n);
                widths = natural.iter().map(|w| w.min(TAB_MIN_W.max((room - MORE_W) / fit as f32))).collect();
                shown = (0..fit).collect();
                // The front tab always shows: it takes the last place.
                if let Some(front) = self.tabs.iter().position(|t| t.2) {
                    if front >= fit {
                        shown[fit - 1] = front;
                    }
                }
            }
        }
        self.hidden = (0..n).filter(|i| !shown.contains(i)).collect();
        self.rects = vec![None; n];
        self.hover = None;
        ui.spacing_mut().item_spacing = vec2(1.0, 0.0);
        for &i in &shown {
            let (id, name, front, dirty, link, _tip, _pic) = &self.tabs[i];
            let _ = id;
            let (rect, resp) = ui.allocate_exact_size(vec2(widths[i], CONTROL_H), Sense::click());
            self.rects[i] = Some(rect);
            let hovered = resp.hovered();
            if hovered {
                ui.painter().rect_filled(rect, CornerRadius::same(R_CONTROL), HOVER);
                self.hover = Some((i, rect));
            }
            let ink = if *front { INK } else if hovered { INK } else { DIM };
            let mut x = rect.min.x + PAD_L;
            if *link {
                paint_link(ui, pos2(x + 6.0, rect.center().y), ink);
                x += LINK_W;
            }
            let mut job = egui::text::LayoutJob::single_section(name.clone(), egui::TextFormat {
                font_id: if *front { strong(TYPE_BODY) } else { body() }, color: ink, ..Default::default() });
            job.wrap = egui::text::TextWrapping { max_width: (rect.max.x - PAD_R - x).max(8.0), max_rows: 1,
                                                  break_anywhere: true, overflow_character: Some('…') };
            let g = ui.painter().layout_job(job);
            ui.painter().galley(pos2(x, rect.center().y - g.size().y / 2.0), g, ink);
            // • while unsaved; × on hover, in its place.
            let mark = Rect::from_center_size(pos2(rect.max.x - 13.0, rect.center().y), vec2(18.0, 18.0));
            let over_x = hovered && resp.hover_pos().is_some_and(|p| mark.contains(p));
            if hovered {
                if over_x { ui.painter().rect_filled(mark, CornerRadius::same(R_ROW), PRESS); }
                let c = mark.center();
                let k = 3.5;
                let pen = Stroke::new(1.3_f32, if over_x { INK } else { DIM });
                ui.painter().line_segment([c + vec2(-k, -k), c + vec2(k, k)], pen);
                ui.painter().line_segment([c + vec2(-k, k), c + vec2(k, -k)], pen);
            } else if *dirty {
                ui.painter().circle_filled(mark.center(), 3.0, if *front { INK } else { DIM });
            }
            if *front {
                // The accent line on the bar's bottom edge.
                ui.painter().hline(rect.x_range().shrink(6.0), rect.center().y + (TOP_H - MENU_H) / 2. - 1., Stroke::new(2.0_f32, ACCENT));
            }
            if resp.clicked() {
                out.push(if over_x { format!("ptab:close:{i}") } else { format!("ptab:activate:{i}") });
            } else if resp.middle_clicked() {
                out.push(format!("ptab:close:{i}"));
            } else if resp.secondary_clicked() {
                // The tab comes to the front first, so Save acts on it.
                if !*front { out.push(format!("ptab:activate:{i}")); }
                self.menu = Some((i, pos2(rect.min.x, TOP_H + 4.0)));
                self.overflow = None;
            }
        }
        // ›› n: the tabs left out, and the projects in other windows, so
        // every open project is one list away.
        if !self.hidden.is_empty() || !self.elsewhere.is_empty() {
            let label = format!("›› {}", self.hidden.len() + self.elsewhere.len());
            let (rect, resp) = ui.allocate_exact_size(vec2(MORE_W, CONTROL_H), Sense::click());
            let st = state_of(&resp, self.overflow.is_some(), true);
            paint_state(ui, rect, st, R_CONTROL);
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, body(), DIM);
            if resp.clicked() {
                self.overflow = if self.overflow.is_some() { None } else { Some(rect) };
                self.menu = None;
            }
        }
        let plus = (String::new(), "+".to_owned(), "New model (⌘N)".to_owned(), String::new(), -1i8, true);
        let (rect, resp) = ui.allocate_exact_size(vec2(PLUS_W, CONTROL_H), Sense::click());
        let st = state_of(&resp, false, true);
        paint_state(ui, rect, st, R_CONTROL);
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, &plus.1, heading(), DIM);
        if resp.on_hover_text(&plus.2).clicked() { out.push("ptab:new".into()); }
        out
    }

    fn menu_rows(&self, i: usize) -> Vec<(&'static str, String, &'static str, bool, i8)> {
        let count = self.tabs.len();
        vec![
                ("new", "New model".into(), "⌘N", true, -1),
                ("open", "Open…".into(), "⌘O", true, -1),
                ("save", "Save".into(), "⌘S", true, -1),
                ("save_as", "Save As…".into(), "⇧⌘S", true, -1),
                ("-", String::new(), "", true, -1),
                ("close", "Close model".into(), "⌘W", true, -1),
                ("close_right", "Close models to the right".into(), "", i + 1 < count, -1),
                ("close_left", "Close models to the left".into(), "", i > 0, -1),
                ("close_all", "Close all models".into(), "", true, -1),
                ("-", String::new(), "", true, -1),
                ("move", "Move to new window".into(), "", self.can_move && count > 1, -1),
                ("-", String::new(), "", true, -1),
                ("pictures", "Picture on hover".into(), "", true, if self.pictures { 1 } else { 0 }),
        ]
    }

    /// The hover card, the tab menu and the ›› n menu.
    pub fn popups(&mut self, device: &wgpu::Device, painter: &mut egui_wgpu::Renderer, ctx: &egui::Context, icons: &std::collections::HashMap<String, egui::TextureHandle>) -> Vec<String> {
        let mut out = Vec::new();
        // The hover card: picture (when the tab was left), name, path, saved.
        if self.menu.is_none() && self.overflow.is_none() {
            if let Some((i, rect)) = self.hover {
                if let Some((id, name, front, _, _, tip, pic)) = self.tabs.get(i) {
                    let picture = if !*front && self.pictures {
                        self.photos.get_mut(id).map(|(texture, registered)| {
                            let id = *registered.get_or_insert_with(|| {
                                let view = texture.create_view(&wgpu::TextureViewDescriptor {
                                    format: Some(texture.format().remove_srgb_suffix()), ..Default::default()
                                });
                                painter.register_native_texture(device, &view, wgpu::FilterMode::Linear)
                            });
                            (id, vec2(texture.width() as f32, texture.height() as f32))
                        }).or_else(|| icons.get(pic).map(|tex| (tex.id(), tex.size_vec2())))
                    } else { None };
                    egui::Area::new(egui::Id::new("tab_card"))
                        .fixed_pos(pos2(rect.min.x, TOP_H + 6.0))
                        .order(egui::Order::Tooltip)
                        .interactable(false)
                        .show(ctx, |ui| {
                            glass(Surface::Raised).inner_margin(Margin::same(8)).show(ui, |ui| {
                                ui.set_max_width(PICTURE_W);
                                if let Some((id, s)) = picture {
                                    let h = if s.x > 0.0 { PICTURE_W * s.y / s.x } else { PICTURE_W * 0.6 };
                                    let (r, _) = ui.allocate_exact_size(vec2(PICTURE_W, h), Sense::hover());
                                    egui::Image::new((id, s)).corner_radius(CornerRadius::same(R_ROW)).paint_at(ui, r);
                                    ui.add_space(SPACE_S);
                                }
                                ui.label(egui::RichText::new(name).font(strong(TYPE_BODY)).color(INK));
                                for line in tip.lines() {
                                    ui.label(egui::RichText::new(line).font(meta()).color(DIM));
                                }
                            });
                        });
                }
            }
        }
        if let Some((i, at)) = self.menu {
            let rows = self.menu_rows(i);
            let mut close = false;
            egui::Area::new(egui::Id::new("tab_menu"))
                .fixed_pos(at)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    glass(Surface::Raised).inner_margin(Margin::same(6)).show(ui, |ui| {
                        ui.set_width(236.0);
                        ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                        for (id, label, key, enabled, checked) in &rows {
                            if *id == "-" {
                                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 7.0), Sense::hover());
                                ui.painter().hline(r.x_range().shrink(6.0), r.center().y, Stroke::new(1.0_f32, EDGE));
                                continue;
                            }
                            let item = (id.to_string(), label.clone(), key.to_string(), "check".to_owned(), *checked, *enabled);
                            if menu_row(ui, &item) && *enabled {
                                if *id == "pictures" { self.pictures = !self.pictures; }
                                out.push(format!("ptab:menu:{id}:{i}"));
                                close = true;
                            }
                        }
                    });
                });
            if close { self.menu = None; }
        }
        if let Some(anchor) = self.overflow {
            let mut close = false;
            egui::Area::new(egui::Id::new("tab_overflow"))
                .fixed_pos(pos2(anchor.min.x, TOP_H + 4.0))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    glass(Surface::Raised).inner_margin(Margin::same(6)).show(ui, |ui| {
                        ui.set_width(240.0);
                        ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                        for &i in &self.hidden {
                            let Some((_, name, front, dirty, link, _, _)) = self.tabs.get(i) else { continue };
                            let label = format!("{name}{}{}", if *link { " ↗" } else { "" }, if *dirty { " •" } else { "" });
                            let item = (String::new(), label, String::new(), "check".to_owned(), if *front { 1 } else { 0 }, true);
                            if menu_row(ui, &item) {
                                out.push(format!("ptab:activate:{i}"));
                                close = true;
                            }
                        }
                        if !self.elsewhere.is_empty() {
                            ui.add_space(SPACE_XS);
                            caption_at(ui, "Other windows", 12.0);
                            for (id, name, window) in &self.elsewhere {
                                let item = (id.clone(), name.clone(), window.clone(), "check".to_owned(), 0, true);
                                if menu_row(ui, &item) {
                                    out.push(format!("ptab:goto:{id}"));
                                    close = true;
                                }
                            }
                        }
                    });
                });
            if close { self.overflow = None; }
        }
        out
    }
}

/// A small chain mark for a link source's tab: two interlocked links.
fn paint_link(ui: &egui::Ui, c: Pos2, ink: Color32) {
    let pen = Stroke::new(1.3_f32, ink);
    let r = CornerRadius::same(2);
    let a = Rect::from_center_size(c + vec2(-2.5, 1.5), vec2(7.0, 4.5));
    let b = Rect::from_center_size(c + vec2(2.5, -1.5), vec2(7.0, 4.5));
    ui.painter().rect_stroke(a, r, pen, egui::StrokeKind::Middle);
    ui.painter().rect_stroke(b, r, pen, egui::StrokeKind::Middle);
}

impl Ui {
    /// Python's tabs for the strip (views/wgpu_tabs.py): the tabs, the
    /// projects in other windows, and the tab menu's two switches.
    pub fn set_project_tabs(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, tabs: Vec<TabData>, elsewhere: Vec<(String, String, String)>,
                            can_move: bool, pictures: bool) {
        let background = |id: &str| tabs.iter().any(|t| t.0 == id && !t.2);
        self.strip.photos.retain(|id, (_, texture)| {
            if background(id) { return true; }
            if let Some(texture) = texture { self.free_after_frame.push(*texture); }
            false
        });
        if let Some(id) = self.strip.painted_doc.as_ref() {
            if !tabs.iter().any(|t| &t.0 == id && t.2) {
                if background(id) {
                    if let Some(texture) = self.glass.tab_picture(device, queue) {
                        self.strip.photos.insert(id.clone(), (texture, None));
                    }
                }
                self.strip.painted_doc = None;
            }
        }
        if !self.strip.tabs.iter().map(|t| &t.0).eq(tabs.iter().map(|t| &t.0)) {
            self.strip.close_menus();
        }
        self.strip.tabs = tabs;
        self.strip.elsewhere = elsewhere;
        self.strip.can_move = can_move;
        self.strip.pictures = pictures;
    }

    /// Each tab's rect as last laid out (x, y, w, h), or None when it is
    /// under ›› n (tests and harnesses).
    pub fn project_tab_rects(&self) -> Vec<Option<(f32, f32, f32, f32)>> {
        self.strip.rects.iter().map(|r| r.map(|r| (r.min.x, r.min.y, r.width(), r.height()))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(name: &str, front: bool) -> TabData {
        (String::new(), name.to_owned(), front, false, false, String::new(), String::new())
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        // The app's Inter for every family the chrome names (this crate
        // builds egui without its bundled fonts).
        let inter = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/fonts/inter/Inter-Regular.ttf"))
            .expect("resources/fonts/inter");
        let mut fonts = egui::FontDefinitions::empty();
        fonts.font_data.insert("inter".into(), std::sync::Arc::new(egui::FontData::from_owned(inter)));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace,
                       egui::FontFamily::Name("medium".into()), egui::FontFamily::Name("semibold".into())] {
            fonts.families.insert(family, vec!["inter".into()]);
        }
        ctx.set_fonts(fonts);
        ctx
    }

    fn frame(ctx: &egui::Context, strip: &mut Strip, width: f32, events: Vec<egui::Event>)
        -> (egui::FullOutput, Vec<String>)
    {
        let mut commands = Vec::new();
        let input = egui::RawInput { events, ..Default::default() };
        let output = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal(|ui| commands = strip.show(ui, width));
            });
        });
        (output, commands)
    }

    fn lay_out(strip: &mut Strip, width: f32) -> egui::FullOutput {
        let ctx = context();
        frame(&ctx, strip, width, Vec::new());
        frame(&ctx, strip, width, Vec::new()).0
    }

    #[test]
    fn right_edge_closes_the_pointed_tab_and_middle_click_closes_its_body() {
        use egui::{Event, PointerButton};
        let ctx = context();
        let mut strip = Strip {
            tabs: vec![tab("Drawing", true), tab("مخطط", false)], ..Default::default()
        };
        frame(&ctx, &mut strip, 700.0, Vec::new());
        for front in [0, 1] {
            strip.tabs[0].2 = front == 0;
            strip.tabs[1].2 = front == 1;
            frame(&ctx, &mut strip, 700.0, Vec::new());
            for index in [0, 1] {
                let rect = strip.rects[index].unwrap();
                let at_edge = pos2(rect.right() - 7.0, rect.center().y);
                for (pos, button, verb) in [
                    (at_edge, PointerButton::Primary, "close"),
                    (rect.center(), PointerButton::Primary, "activate"),
                    (rect.center(), PointerButton::Middle, "close"),
                ] {
                    let event = |pressed| Event::PointerButton { pos, button, pressed, modifiers: Default::default() };
                    frame(&ctx, &mut strip, 700.0, vec![Event::PointerMoved(pos), event(true)]);
                    let (_, actions) = frame(&ctx, &mut strip, 700.0, vec![event(false)]);
                    assert!(actions.contains(&format!("ptab:{verb}:{index}")), "{actions:?}");
                }
            }
        }
    }

    #[test]
    fn right_click_activates_the_target_before_its_file_menu() {
        let ctx = context();
        let mut strip = Strip { tabs: vec![tab("A", true), tab("B", false)], ..Default::default() };
        frame(&ctx, &mut strip, 700., vec![]);
        let pos = strip.rects[1].unwrap().center();
        for pressed in [true, false] {
            let click = egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary,
                pressed, modifiers: Default::default() };
            let (_, actions) = frame(&ctx, &mut strip, 700., vec![egui::Event::PointerMoved(pos), click]);
            if !pressed {
                assert_eq!(actions, ["ptab:activate:1"]);
                assert_eq!(strip.menu.unwrap().0, 1);
            }
        }
        for i in 0..2 {
            for pictures in [true, false] {
                strip.pictures = pictures;
                let rows = strip.menu_rows(i);
                for verb in ["new", "open", "save", "save_as", "close"] {
                    assert!(rows.iter().any(|r| r.0 == verb && !r.2.is_empty() && r.3));
                }
                for (verb, enabled) in [("close_left", i == 1), ("close_right", i == 0), ("close_all", true)] {
                    assert!(rows.iter().any(|r| r.0 == verb && r.3 == enabled));
                }
                assert!(rows.iter().any(|r| r.0 == "pictures" && r.4 == i8::from(pictures)));
            }
        }
    }

    #[test]
    fn active_underline_and_new_button_follow_the_tabs() {
        let mut strip = Strip {
            tabs: vec![tab("Drawing", true), tab("Elevation", false)], ..Default::default()
        };
        for front in [0, 1] {
            strip.tabs[0].2 = front == 0;
            strip.tabs[1].2 = front == 1;
            let frame = lay_out(&mut strip, 700.0);
            let active = strip.rects[front].unwrap();
            let last = strip.rects[1].unwrap();
            assert_eq!(active.height(), CONTROL_H);
            let mut accents = Vec::new();
            let mut plus = None;
            for s in frame.shapes {
                match s.shape {
                    egui::Shape::LineSegment { points, stroke } if stroke.color == ACCENT => {
                        accents.push(points);
                    }
                    egui::Shape::Text(t) if t.galley.text() == "+" => {
                        plus = Some(t.pos + t.galley.size() / 2.0);
                    }
                    _ => {}
                }
            }
            assert_eq!(accents.len(), 1);
            let [left, right] = accents[0];
            assert!(left.x >= active.left() && right.x <= active.right());
            // The bottom of the tab bar: the laid-out strip has no menu bar above it.
            assert_eq!(left.y, TOP_H - MENU_H - 1.0);
            let plus = plus.expect("new tab button");
            assert!((plus.x - (last.right() + 1.0 + PLUS_W / 2.0)).abs() <= 1.0);
            assert!((plus.y - last.center().y).abs() <= 1.0);
        }
    }

    #[test]
    fn full_names_leave_room_for_close_on_either_tab() {
        let mut strip = Strip {
            tabs: vec![tab("casa-pestanas.igz", true), tab("مخطط", false)], ..Default::default()
        };
        for front in [0, 1] {
            strip.tabs[0].2 = front == 0;
            strip.tabs[1].2 = front == 1;
            let frame = lay_out(&mut strip, 700.0);
            for (index, tab) in strip.tabs.iter().enumerate() {
                let rect = strip.rects[index].unwrap();
                let text = frame.shapes.iter().find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == tab.1 => Some(t),
                    _ => None,
                }).unwrap();
                assert!(!text.galley.elided);
                assert!(text.pos.x + text.galley.size().x <= rect.right() - 22.0);
            }
        }
    }

    #[test]
    fn tabs_fit_at_their_width_when_there_is_room() {
        let mut strip = Strip { tabs: vec![tab("Site-plan.igz", true), tab("Catwalk-L2.igz", false)], ..Default::default() };
        lay_out(&mut strip, 900.0);
        assert!(strip.rects.iter().all(|r| r.is_some()));
        assert!(strip.hidden.is_empty());
    }

    #[test]
    fn a_short_bar_shrinks_tabs_to_96_then_overflows_keeping_the_front_tab() {
        let names = ["Alpha-long-name.igz", "Bravo-long-name.igz", "Charlie-long-name.igz",
                     "Delta-long-name.igz", "Echo-long-name.igz", "Foxtrot-long-name.igz"];
        let mut strip = Strip { tabs: names.iter().enumerate().map(|(i, n)| tab(n, i == 5)).collect(), ..Default::default() };
        lay_out(&mut strip, 420.0);
        let shown: Vec<usize> = (0..6).filter(|&i| strip.rects[i].is_some()).collect();
        assert!(!strip.hidden.is_empty(), "some tabs go under ›› n");
        assert!(shown.contains(&5), "the front tab always shows");
        for i in &shown {
            assert!(strip.rects[*i].unwrap().width() >= TAB_MIN_W - 0.5);
        }
    }

    #[test]
    fn projects_in_other_windows_are_one_list_away() {
        let mut strip = Strip { tabs: vec![tab("A.igz", true)], elsewhere: vec![("1:0".into(), "B.igz".into(), "Window 2".into())],
                                ..Default::default() };
        lay_out(&mut strip, 900.0);
        assert!(strip.hidden.is_empty());
        assert_eq!(strip.elsewhere.len(), 1);
    }
}
