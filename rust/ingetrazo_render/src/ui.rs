//! Viewport chrome drawn by egui in the frame that draws the model (the
//! UI spike, `ui/egui-spike`). Python forwards pointer events and registers
//! the tools; clicks come back as action strings. Nothing here owns model
//! state: the palette shows what Python says is active.

use std::sync::Arc;
use std::time::Instant;

use egui::{pos2, vec2, Color32, CornerRadius, Margin, Pos2, Rect, Sense, Stroke, Vec2};

/// Small widgets shared by the dock and the forms (segmented, tabs).
mod widgets;
/// The chrome's modal forms (views/chrome_forms.py).
pub mod dialog;
/// Sheets, a workspace of the window (views/wgpu_sheets.py).
mod sheets;
mod activity;
mod plugins;
pub use activity::ActivityRow;
/// Project tabs in the top bar (views/wgpu_tabs.py).
mod tabs;
/// The viewport right-click menu (views/context_menu.py, docs/context-menu.md).
mod context_menu;

mod tree_cols;
mod tree_order;
mod drawing;
/// The large sheet over the dimmed model: Library, start page, Settings
/// (views/chrome_sheet.py, docs/ui-patterns.md).
pub mod large_sheet;

pub mod tokens;
pub mod kit;
#[cfg(target_os = "macos")]
mod system_fonts;
mod gallery;
use tokens::*;
use kit::*;

struct Tool {
    enabled: bool,
    id: String,
    label: String,
    key: String,
    icon: Option<egui::TextureHandle>,
}

/// A window action shown in the chrome: (id, label, tooltip, icon key,
/// checked: -1 not checkable / 0 / 1, enabled).
pub type Item = (String, String, String, String, i8, bool);
/// One field of a settings form: (id, kind, label, text, value, min, max,
/// options). Kinds: caption, check, slider, choice, color, button, text.
pub type FormField = (String, String, String, String, f32, f32, f32, Vec<String>);

/// A settings popover (Edit style, Shadows…): Python mirrors a panel's
/// controls into fields and applies what comes back as `form:<id>:<value>`.
pub struct Form {
    title: String,
    fields: Vec<FormField>,
    choice_open: Option<usize>,
}

pub struct Ui {
    plugins: plugins::Panels,
    plugin_document: String,
    background_plugins: std::collections::HashMap<String, plugins::Panels>,
    pub panels: crate::inspector::Panels,
    ctx: egui::Context,
    pub font_picker: Option<FontPicker>,
    gallery: Option<gallery::Gallery>,
    painter: egui_wgpu::Renderer,
    free_after_frame: Vec<egui::TextureId>,
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
    /// Where the focused field wants the input method's candidate window
    /// (egui's IME output, points): (x, y, w, h) of the text cursor.
    ime_rect: Option<(f32, f32, f32, f32)>,
    pub frame_ms: f64,
    /// Interactive areas as last laid out (points).
    hit: Vec<Rect>,
    icons: std::collections::HashMap<String, egui::TextureHandle>,
    /// Top-right cluster: document actions, latched view toggles, View chip.
    cluster: Vec<Vec<Item>>,
    view_menu: Vec<(String, Vec<Item>)>,
    view_open: bool,
    form: Option<Form>,
    /// Options strip: (id, label, key, state: 0 off, 1 on, 2 read-only note).
    options: Vec<(String, String, String, u8)>,
    /// Right column: project tree above, inspector below.
    column: Option<Column>,
    /// Ready-to-deliver: active node path, unfiled count, items to fix.
    delivery: Option<(String, u32, u32)>,
    /// Bottom centre: workspaces (Model, sheet) then scenes, as groups.
    tabs: Vec<Vec<Item>>,
    /// Project tabs at the top left, after the window buttons' room:
    /// (left edge, [(name, active, unsaved)]).
    projects: Option<(f32, Vec<(String, bool, bool)>)>,
    review: bool,
    /// The Paint tool's strip: (id, name, colour hex or "", icon key or "", active).
    palette_items: Vec<(String, String, String, String, bool)>,
    /// The command palette (⌘K / F3): open, query, results (label, path, shortcut, enabled), picked row.
    cmd_open: bool,
    cmd_query: String,
    cmd_results: Vec<(String, String, String, bool)>,
    cmd_at: usize,
    cmd_focus: bool,
    /// The open bar menu: kind ("docs", "work", "scenes") and its button.
    menu: Option<(String, Rect)>,
    view_anchor: Rect,
    sel_more_anchor: Rect,
    top_groups: Vec<Rect>,
    top_right_w: f32,
    units: String,
    glass: crate::glass::Glass,
    /// The app's one question or message (unsaved changes, errors,
    /// confirmations): Python owns its keys and waits for the answer.
    sheet: Option<Sheet>,
    /// A modal form (prompts, Preferences, export options): Python owns
    /// its values; a sheet may still go over it (ui/dialog.rs).
    dialog: Option<dialog::Dialog>,
    /// Work in progress, in the top bar: open, import, save, auto-save.
    jobs: Vec<Job>,
    history: Vec<ActivityRow>,
    activity_open: bool,
    link_banner: String,
    /// The dock's width (points): dragged on its left edge, kept by Python.
    dock_w: f32,
    /// The pointer shape the chrome asks for ("" or "resize_h").
    cursor_shape: &'static str,
    /// The Sheets workspace: while on, the page replaces the model (ui/sheets.rs).
    sheets: sheets::Sheets,
    /// The project tabs (ui/tabs.rs): drawn instead of the document ▾
    /// once Python sends them.
    strip: tabs::Strip,
    /// The right-click menu over the model (ui/context_menu.rs).
    ctx_menu: Option<context_menu::Menu>,
    /// The large sheet (ui/large_sheet.rs): over everything under the top bar.
    lsheet: large_sheet::LargeSheet,
}

/// A sheet: title, body, tone (question, info, warning, error) and its
/// buttons (id, label, key hint, role: 0 plain, 1 default, 2 discard, 3 cancel).
#[derive(Clone)]
pub struct Sheet {
    title: String,
    body: String,
    tone: String,
    buttons: Vec<(String, String, String, u8)>,
}

/// One job: id, what (the file), phase, fraction (< 0: no percentage), cancellable.
pub type Job = (String, String, String, f32, bool);

/// One tree row as Python sends it, in display order (Python owns
/// expansion and filtering; the dock only draws and reports).
#[derive(Default)]
pub struct Node {
    key: String,
    depth: usize,
    name: String,
    kind: String,
    count: u32,
    problems: u32,
    active: bool,
    /// 0 node, 1 item, 2 nested item, 3 filed elsewhere (ghost), 4 loose geometry.
    row: u8,
    badge: String,
    /// -1 none, 0 off, 1 on.
    eye: i8,
    lock: i8,
    expandable: bool,
    expanded: bool,
    selected: u32,
    /// The selection, marked once (tree A): 1 this row holds it (lit), 2 a
    /// row above the one holding it (a quiet dot), 0 neither.
    mark: u8,
    icon: String,
    /// What the kind means, on hover (Python sends "kind\u{1e}tip").
    tip: String,
    /// The active column set's cells (Drawing: colour, line type, weight,
    /// print; Export: class, predefined type).
    cells: Vec<String>,
}

pub struct Column {
    nodes: Vec<Node>,
    sort: tree_order::Sort,
    ordered: Vec<usize>,
    unfiled: u32,
    expanded: std::collections::HashSet<String>,
    filter: String,
    /// "File to…": the next row clicked (or Enter) receives the selection.
    filing: Option<String>,
    focus_filter: bool,
    /// The row menu: row key, row type, where.
    row_menu: Option<(String, u8, Pos2)>,
    /// Inline rename: row key and the text being typed.
    renaming: Option<(String, String)>,
    focus_rename: bool,
    head_menu: Option<Pos2>,
    kind_menu: Option<(String, Pos2)>,
    kinds: Vec<String>,
    /// A confirmation over the dock: title, body, (id, label) buttons.
    prompt: Option<(String, String, Vec<(String, String)>)>,
    /// 0 Filing, 1 Drawing, 2 Export.
    colset: u8,
    /// The colour being picked for a row, and where.
    color_pick: Option<(String, Color32, Pos2)>,
    /// A dock mode replacing the tree ("" = the Project tree, else e.g. "Problems").
    mode: String,
    /// Issues shown (dots in the tree, counts in the header) or kept out of the way.
    issues: bool,
    /// The inspector's editable fields: id → text being typed.
    edits: std::collections::HashMap<String, String>,
    inspector_title: String,
    inspector: Vec<(String, Vec<(String, String)>)>,
    highlighted: Option<String>,
}

impl Ui {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let ctx = egui::Context::default();
        ctx.style_mut(kit::install_style);
        let mut painter = egui_wgpu::Renderer::new(device, format, None, 1, false);
        let (glass, resources) = crate::glass::Glass::new(device, format);
        painter.callback_resources.insert(resources);
        Self {
            panels: Default::default(),
            ctx,
            font_picker: None,
            gallery: (std::env::var("BUILT_GALLERY").as_deref() == Ok("1")).then(gallery::Gallery::default),
            painter,
            free_after_frame: Vec::new(),
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
            ime_rect: None,
            frame_ms: 0.,
            hit: Vec::new(),
            icons: Default::default(),
            cluster: Vec::new(),
            view_menu: Vec::new(),
            view_open: false,
            form: None,
            options: Vec::new(),
            column: None,
            delivery: None,
            tabs: Vec::new(),
            projects: None,
            review: false,
            palette_items: Vec::new(),
            cmd_open: false,
            cmd_query: String::new(),
            cmd_results: Vec::new(),
            cmd_at: 0,
            cmd_focus: false,
            menu: None,
            view_anchor: Rect::NOTHING,
            sel_more_anchor: Rect::NOTHING,
            top_groups: Vec::new(),
            top_right_w: 0.0,
            units: "mm".into(),
            sheet: None,
            dialog: None,
            jobs: Vec::new(),
            history: Vec::new(),
            activity_open: false,
            link_banner: String::new(),
            dock_w: DOCK_W,
            plugins: Default::default(),
            plugin_document: String::new(),
            background_plugins: Default::default(),
            cursor_shape: "",
            sheets: Default::default(),
            strip: Default::default(),
            ctx_menu: None,
            lsheet: Default::default(),
        }
    }

    /// Regular, medium and semibold faces (Inter); medium/semibold fall
    /// back to regular when missing.
    pub fn set_fonts(&mut self, regular: Vec<u8>, medium: Option<Vec<u8>>, semibold: Option<Vec<u8>>) {
        let mut fonts = egui::FontDefinitions::default();
        // Inter lacks the arrows, checks and pencil the chrome uses: the
        // system faces stand behind it (egui's own fallbacks are not built in).
        #[cfg(target_os = "macos")]
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
    pub fn set_tools(&mut self, groups: Vec<Vec<(String, String, String, usize, usize, Vec<u8>)>>, disabled: &[String]) {
        self.groups = groups
            .into_iter()
            .map(|g| {
                g.into_iter()
                    .map(|(id, label, key, w, h, rgba)| {
                        let icon = (w > 0 && rgba.len() == w * h * 4).then(|| {
                            let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                            self.ctx.load_texture(&id, image, egui::TextureOptions::LINEAR)
                        });
                        Tool { enabled: !disabled.contains(&id), id, label, key, icon }
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
    /// rows: ([key, name, kind, badge, icon], [depth, count, problems, active,
    /// row, eye, lock, expandable, expanded, selected]).
    pub fn set_tree(&mut self, rows: Vec<(Vec<String>, Vec<i64>)>, unfiled: u32) {
        let col = self.column.get_or_insert_with(|| Column {
            nodes: Vec::new(), sort: Default::default(), ordered: Vec::new(), unfiled: 0, expanded: Default::default(), filter: String::new(),
            filing: None, focus_filter: false, row_menu: None, renaming: None, focus_rename: false, head_menu: None,
            kind_menu: None, kinds: Vec::new(), prompt: None, colset: 0, color_pick: None, edits: Default::default(), mode: String::new(), issues: false,
            inspector_title: String::new(),
            inspector: Vec::new(), highlighted: None,
        });
        let g = |v: &Vec<i64>, i: usize| v.get(i).copied().unwrap_or(0);
        let s = |v: &Vec<String>, i: usize| v.get(i).cloned().unwrap_or_default();
        col.nodes = rows.into_iter().map(|(t, n)| {
            let raw = s(&t, 2);
            let (kind, tip) = match raw.split_once('\u{1e}') {
                Some((k, tip)) => (k.to_string(), tip.to_string()),
                None => (raw, String::new()),
            };
            Node {
            key: s(&t, 0), name: s(&t, 1), kind, tip, badge: s(&t, 3), icon: s(&t, 4),
            depth: g(&n, 0) as usize, count: g(&n, 1) as u32, problems: g(&n, 2) as u32, active: g(&n, 3) != 0,
            row: g(&n, 4) as u8, eye: g(&n, 5) as i8, lock: g(&n, 6) as i8, expandable: g(&n, 7) != 0,
            expanded: g(&n, 8) != 0, selected: g(&n, 9) as u32, mark: g(&n, 10) as u8,
            cells: t.get(5..).map(|c| c.to_vec()).unwrap_or_default(),
        }}).collect();
        col.ordered = col.sort.order(&col.nodes);
        col.unfiled = unfiled;
    }
    pub fn set_mode(&mut self, mode: String) {
        if let Some(col) = self.column.as_mut() {
            col.mode = mode;
        }
    }
    /// The dock's width as last kept (clamped to what the window allows).
    pub fn set_dock_width(&mut self, w: f32) { self.dock_w = w.clamp(DOCK_MIN_W, DOCK_MAX_W); }
    /// The pointer the chrome wants over it: "resize_h" over the dock's grip.
    pub fn cursor_shape(&self) -> &'static str { self.cursor_shape }
    pub fn set_issues(&mut self, on: bool) {
        if let Some(col) = self.column.as_mut() { col.issues = on; }
    }
    pub fn set_tree_sort(&mut self, index: usize) {
        if let (Some(col), Some(&(sort, _))) = (self.column.as_mut(), tree_order::Sort::OPTIONS.get(index)) {
            if col.sort != sort {
                col.sort = sort;
                col.ordered = sort.order(&col.nodes);
            }
        }
    }
    pub fn set_colset(&mut self, colset: u8) {
        if let Some(col) = self.column.as_mut() { col.colset = colset.min(2); }
    }
    pub fn set_kinds(&mut self, kinds: Vec<String>) {
        if let Some(col) = self.column.as_mut() { col.kinds = kinds; }
    }
    pub fn set_prompt(&mut self, title: String, body: String, buttons: Vec<(String, String)>) {
        if let Some(col) = self.column.as_mut() {
            col.prompt = (!title.is_empty()).then_some((title, body, buttons));
        }
    }
    pub fn set_inspector(&mut self, title: String, sections: Vec<(String, Vec<(String, String)>)>) {
        if let Some(col) = self.column.as_mut() {
            if col.inspector_title != title || col.inspector != sections {
                col.edits.clear();
            }
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
    pub fn set_projects(&mut self, left: f32, docs: Vec<(String, bool, bool)>) {
        self.projects = Some((left, docs));
    }
    pub fn open_commands(&mut self) {
        self.cmd_open = true;
        self.cmd_query.clear();
        self.cmd_at = 0;
        self.cmd_focus = true;
        self.actions.push("cmdq:".into());
    }
    pub fn set_command_results(&mut self, results: Vec<(String, String, String, bool)>) {
        self.cmd_results = results;
        self.cmd_at = self.cmd_at.min(self.cmd_results.len().saturating_sub(1));
    }
    pub fn set_palette(&mut self, items: Vec<(String, String, String, String, bool)>) { self.palette_items = items; }
    /// Opens (or refreshes) the settings form; an empty title closes it.
    pub fn set_form(&mut self, title: String, fields: Vec<FormField>) {
        if title.is_empty() { self.form = None; return; }
        let choice_open = self.form.as_ref().filter(|f| f.title == title).and_then(|f| f.choice_open);
        if self.form.is_none() { self.view_open = false; }
        self.form = Some(Form { title, fields, choice_open });
    }
    pub fn set_units(&mut self, units: String) { self.units = units; }
    /// Shows (or replaces) the sheet; an empty title takes it down. A
    /// sheet closes whatever menu was open: one float at a time.
    pub fn set_sheet(&mut self, title: String, body: String, tone: String, buttons: Vec<(String, String, String, u8)>) {
        if title.is_empty() { self.sheet = None; return; }
        self.view_open = false;
        self.sel_more = false;
        self.menu = None;
        self.cmd_open = false;
        if let Some(col) = self.column.as_mut() {
            col.row_menu = None; col.kind_menu = None; col.head_menu = None; col.color_pick = None;
        }
        self.ctx.memory_mut(|m| m.stop_text_input());
        self.sheet = Some(Sheet { title, body, tone, buttons });
    }
    pub fn plugin_panel(&mut self, raw: &str) -> Result<(), String> { self.plugins.set(raw) }
    pub fn plugin_open(&mut self, id: &str) { self.plugins.open(id); }
    pub fn plugin_document(&mut self, document: String, live: &[String]) {
        if self.plugin_document != document {
            let next = self.background_plugins.remove(&document).unwrap_or_default();
            let previous = std::mem::replace(&mut self.plugins, next);
            let old_id = std::mem::replace(&mut self.plugin_document, document);
            if live.contains(&old_id) {
                self.background_plugins.insert(old_id, previous);
            }
        }
        self.background_plugins.retain(|key, _| live.contains(key));
    }
    pub fn set_jobs(&mut self, jobs: Vec<Job>) { self.jobs = jobs; }
    pub fn set_tabs(&mut self, tabs: Vec<Vec<Item>>) { self.tabs = tabs; }
    pub fn set_delivery(&mut self, active: String, unfiled: u32, problems: u32) {
        self.delivery = Some((active, unfiled, problems));
    }
    pub fn set_options(&mut self, options: Vec<(String, String, String, u8)>) { self.options = options; }
    pub fn set_cluster(&mut self, cluster: Vec<Vec<Item>>, view_menu: Vec<(String, Vec<Item>)>) {
        self.cluster = cluster;
        self.view_menu = view_menu;
    }

    /// Keyboard ownership: the chrome takes a key only while one of its
    /// fields has focus, or Escape while one of its popups is open.
    pub fn claims_key(&self, name: &str) -> bool {
        if self.gallery.is_some() || self.font_picker.is_some() { return true; }
        if name == "Escape" && (self.strip.menus_open() || self.plugin_popup_open() || self.panels.data["terrain"].is_object()) { return true; }
        // The large sheet owns every key while it is up (Python keeps the
        // window's ⌘ shortcuts, views/chrome_sheet.py).
        if self.lsheet.open { return true; }
        if self.sheets.shown { return self.sheets_claims_key(name); }
        let filing = self.column.as_ref().is_some_and(|c| c.filing.is_some() || c.row_menu.is_some() || !c.mode.is_empty()
            || c.kind_menu.is_some() || c.head_menu.is_some() || c.prompt.is_some());
        self.sheet.is_some() || self.dialog.is_some() || self.cmd_open || (name == "Escape" && (self.view_open || self.sel_more || self.menu.is_some() || self.form.is_some() || filing
            || self.ctx_menu.is_some() || self.strip.menus_open()))
            || self.ctx.wants_keyboard_input()
    }
    pub fn key(&mut self, name: &str, pressed: bool, mods: u8) -> bool {
        if self.gallery.is_some() || self.font_picker.is_some() {
            if let Some(key) = egui::Key::from_name(name) {
                self.events.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: mods_from(mods) });
            }
            return true;
        }
        // The sheet's keys are answered in Python (Enter, Esc, ⌘D); none
        // reaches what is under it.
        if self.sheet.is_some() { return true; }
        if self.dialog.is_some() { return self.dialog_key(name, pressed, mods); }
        if name == "Escape" && self.plugin_popup_open() {
            if pressed { self.ctx.memory_mut(|m| m.close_popup()); }
            return true;
        }
        if self.cmd_open && pressed {
            match name {
                "Escape" => { self.cmd_open = false; self.ctx.memory_mut(|m| m.stop_text_input()); return true; }
                "ArrowDown" => { self.cmd_at = (self.cmd_at + 1).min(self.cmd_results.len().saturating_sub(1)); return true; }
                "ArrowUp" => { self.cmd_at = self.cmd_at.saturating_sub(1); return true; }
                "Enter" => {
                    if self.cmd_results.get(self.cmd_at).is_some_and(|r| r.3) {
                        self.actions.push(format!("cmdrun:{}", self.cmd_at));
                    }
                    self.cmd_open = false;
                    self.ctx.memory_mut(|m| m.stop_text_input());
                    return true;
                }
                _ => {}
            }
        }
        if name == "Escape" && (self.ctx_menu.is_some() || self.strip.menus_open()) {
            if pressed { self.ctx_menu = None; self.strip.close_menus(); }
            return true;
        }
        if self.lsheet.open && !self.cmd_open { return self.lsheet_key(name, pressed, mods); }
        if self.sheets.shown { return self.sheets_key(name, pressed, mods); }
        if name == "Escape" && self.panels.data["terrain"].is_object() && !self.ctx.wants_keyboard_input() {
            if pressed { self.actions.push(r#"panel:["terrain.close",null]"#.into()); }
            return true;
        }
        if name == "Escape" && (self.view_open || self.sel_more || self.menu.is_some() || self.form.is_some()) {
            if pressed {
                if self.form.take().is_some() { self.actions.push("form:close".into()); }
                self.view_open = false; self.sel_more = false; self.menu = None;
            }
            return true;
        }
        if name == "Escape" {
            if let Some(col) = self.column.as_mut() {
                if col.row_menu.is_some() || col.filing.is_some() || col.kind_menu.is_some() || col.head_menu.is_some()
                    || col.prompt.is_some() || !col.mode.is_empty() {
                    if pressed {
                        if col.kind_menu.take().is_none() && col.head_menu.take().is_none()
                            && col.prompt.take().is_none() && col.row_menu.take().is_none() {
                            if col.filing.is_some() {
                                col.filing = None;
                                col.filter.clear();
                            } else if !col.mode.is_empty() {
                                col.mode.clear();
                                self.actions.push("tree:mode\u{1f}".into());
                            }
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
    /// An input method's composition for the focused field: kind is
    /// enabled, preedit (the text being composed, drawn underlined in
    /// place by egui), commit or disabled.
    pub fn ime(&mut self, kind: &str, text: String) -> bool {
        if !self.ctx.wants_keyboard_input() {
            return false;
        }
        let event = match kind {
            "enabled" => egui::ImeEvent::Enabled,
            "preedit" => egui::ImeEvent::Preedit(text),
            "commit" => egui::ImeEvent::Commit(text),
            _ => egui::ImeEvent::Disabled,
        };
        self.events.push(egui::Event::Ime(event));
        true
    }
    pub fn ime_rect(&self) -> Option<(f32, f32, f32, f32)> { self.ime_rect }
    pub fn text(&mut self, text: String) -> bool {
        if let Some(picker) = &mut self.font_picker {
            if !self.ctx.wants_keyboard_input() {
                picker.type_filter(&text);
                return true;
            }
        }
        if self.lsheet.open && self.sheet.is_none() && self.dialog.is_none() && !self.cmd_open
            && !self.ctx.wants_keyboard_input() {
            // Type anywhere on the large sheet: it goes to its search.
            self.lsheet.type_text(&text);
            return true;
        }
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
    fn plugin_popup_open(&self) -> bool {
        (self.plugins.showing || self.plugins.utility_open()) && self.ctx.memory(|m| m.any_popup_open())
    }

    fn over_chrome(&self, pos: Pos2) -> bool {
        // A choice or colour picker can extend beyond its parent panel.
        // Its next outside click dismisses it before the model takes input.
        self.font_picker.is_some() || self.plugin_popup_open() || self.hit.iter().any(|r| r.contains(pos))
            || self.ctx.layer_id_at(pos).is_some_and(|layer| self.panels.popup_layers.contains(&layer.id))
    }
    /// Is (x, y) on the interactive chrome (a right-click there is the
    /// chrome's, not the model's)?
    pub fn over_chrome_at(&self, x: f32, y: f32) -> bool { self.over_chrome(pos2(x, y)) }

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
                    self.menu = None;
                    self.cmd_open = false;
                    self.sheets_close_popups();
                    self.ctx_menu = None;
                    self.strip.close_menus();
                    if let Some(col) = self.column.as_mut() {
                        col.row_menu = None;
                        col.kind_menu = None;
                        col.head_menu = None;

                        col.color_pick = None;
                        col.edits.remove("\u{1d}pick");
                    }
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
        let (band_top, band_bottom) = self.band(ctx, screen);
        let room = band_bottom - band_top;
        let tall = |cols: usize| self.groups.iter().map(|g| g.len().div_ceil(cols) as f32 * 32.0).sum::<f32>()
            + self.groups.len().saturating_sub(1) as f32 * 9.0 + 8.0;
        let cols = (2..=5).find(|c| tall(*c) <= room).unwrap_or(5);
        egui::Area::new(egui::Id::new("palette"))
            .fixed_pos(pos2(GUT, band_top))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                frame().corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(4)).show(ui, |ui| {
                    ui.set_min_height(room - 8.0);
                    ui.spacing_mut().item_spacing = vec2(SPACE_XS, SPACE_XS);
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


    // ---- The frame (Round 4, council + Maxence 2026-11-22) -----------------
    // Top and bottom bars docked edge to edge; the tool rail and the project
    // dock float between them; at most one menu floats over the model.

    /// The top bar: window buttons' room, document, workspace, scene |
    /// navigation, view toggles, review, View ▾. Its empty parts move the window.
    fn top_bar(&mut self, ctx: &egui::Context, screen: Rect) {
        let mut clicked: Option<String> = None;
        let mut open: Option<(&'static str, Rect)> = None;
        let mut groups = Vec::new();
        let lights = self.projects.as_ref().map_or(86.0, |p| p.0);
        egui::Area::new(egui::Id::new("top"))
            .fixed_pos(Pos2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Bar).inner_margin(Margin { left: lights as i8, right: 8, top: 6, bottom: 6 }).show(ui, |ui| {
                    ui.set_width(screen.width() - lights - 8.0);
                    ui.set_height(TOP_H - 12.0);
                    ui.spacing_mut().interact_size.y = 28.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(SPACE_XS, 0.0);
                        let left = ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = vec2(SPACE_XS, 0.0);
                            if !self.strip.tabs.is_empty() {
                                // The project tabs (option B): room is left for
                                // the scene control and at least 60 pt to drag.
                                let max_w = (screen.width() - lights - 8.0 - self.top_right_w - 300.0 - 60.0).max(160.0);
                                let acts = self.strip.show(ui, max_w);
                                self.actions.extend(acts);
                                bar_divider(ui);
                            } else if let Some((_, docs)) = &self.projects {
                                let cur = docs.iter().find(|d| d.1).or(docs.first());
                                let label = cur.map_or("Untitled".to_owned(), |d| if d.2 { format!("{} •", d.0) } else { d.0.clone() });
                                let more = if docs.len() > 1 { format!("  {}", docs.len()) } else { String::new() };
                                if let Some(r) = menu_button(ui, &format!("{label}{more}"), self.menu.as_ref().is_some_and(|m| m.0 == "docs")) {
                                    open = Some(("docs", r));
                                }
                                bar_divider(ui);
                            }
                            if self.review {
                                if text_button(ui, "Exit review", "Show the panels again", true) { clicked = Some("review".into()); }
                                bar_divider(ui);
                            }
                            let work = self.tabs.first().and_then(|g| g.iter().find(|i| i.4 == 1)).map_or("Model".to_owned(), |i| i.1.clone());
                            if let Some(r) = menu_button(ui, &work, self.menu.as_ref().is_some_and(|m| m.0 == "work")) {
                                open = Some(("work", r));
                            }
                            if let Some(scenes) = self.tabs.get(1).filter(|g| !g.is_empty()) {
                                let cur = scenes.iter().position(|i| i.4 == 1);
                                let prev = (String::new(), "‹".to_owned(), "Previous scene (⇞)".to_owned(), String::new(), -1i8, true);
                                if item_button(ui, &prev, None) { clicked = Some("scene:prev".into()); }
                                let name = cur.map_or("Scenes".to_owned(), |i| scenes[i].1.clone());
                                if let Some(r) = menu_button(ui, &name, self.menu.as_ref().is_some_and(|m| m.0 == "scenes")) {
                                    open = Some(("scenes", r));
                                }
                                let next = (String::new(), "›".to_owned(), "Next scene (⇟)".to_owned(), String::new(), -1i8, true);
                                if item_button(ui, &next, None) { clicked = Some("scene:next".into()); }
                            }
                        });
                        groups.push(left.response.rect);
                        // The right group, laid out left to right and pushed to
                        // the right edge by its width from the last frame.
                        ui.add_space((ui.available_width() - self.top_right_w).max(0.0));
                        {
                            let right = ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing = vec2(SPACE_XS, 0.0);
                                for (gi, group) in self.cluster.iter().enumerate() {
                                    if gi > 0 { bar_divider(ui); }
                                    for item in group {
                                        if item_button(ui, item, self.icons.get(&item.3)) {
                                            clicked = Some(format!("act:{}", item.0));
                                        }
                                    }
                                }
                                bar_divider(ui);
                                if !self.view_menu.is_empty() {
                                    if let Some(r) = menu_button(ui, "View", self.view_open) {
                                        open = Some(("view", r));
                                    }
                                }
                            });
                            groups.push(right.response.rect);
                        }
                    });
                });
            });
        if let Some(r) = groups.get(1) { self.top_right_w = r.width(); }
        self.top_groups = groups;
        if let Some((kind, r)) = open {
            if kind == "view" {
                self.view_open = !self.view_open;
                if self.view_open && self.form.take().is_some() { self.actions.push("form:close".into()); }
                self.view_anchor = r;
                self.menu = None;
            } else if self.menu.as_ref().is_some_and(|m| m.0 == kind) {
                self.menu = None;
            } else {
                self.menu = Some((kind.to_owned(), r));
                self.view_open = false;
            }
        }
        match clicked.as_deref() {
            Some("review") => { self.review = !self.review; self.actions.push("review".into()); }
            Some(a) => self.actions.push(if a.starts_with("act:") { a.to_owned() } else { format!("tab:{a}") }),
            None => {}
        }
    }

    /// Is ``pos`` on the top bar but on none of its controls (it then moves
    /// the window, as a title bar does)?
    pub fn top_empty(&self, x: f32, y: f32) -> bool {
        let p = pos2(x, y);
        self.gallery.is_none() && y <= TOP_H && !self.top_groups.iter().any(|r| r.contains(p))
            && !self.ctx.memory(|m| m.area_rect(egui::Id::new("jobs"))).is_some_and(|r| (!self.jobs.is_empty() || !self.history.is_empty()) && r.contains(p))
            && !(self.projects.is_none() && x < 86.0)
    }

    /// The bottom bar: units | tool, its options, hint or message | where
    /// new geometry goes.
    fn bottom_bar(&mut self, ctx: &egui::Context, screen: Rect) {
        let mut clicked: Option<String> = None;
        egui::Area::new(egui::Id::new("bottom"))
            .fixed_pos(pos2(0.0, screen.max.y - BOT_H))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Bar).inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
                    ui.set_width(screen.width() - 16.0);
                    ui.set_height(BOT_H - 8.0);
                    ui.spacing_mut().interact_size.y = 28.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(SPACE_XS, 0.0);
                        let (r, _) = ui.allocate_exact_size(vec2(56.0, 28.0), Sense::hover());
                        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, &self.units, meta(), FAINT);
                        bar_divider(ui);
                        ui.add_space(SPACE_S);
                        let icon = self.groups.iter().flatten().find(|t| t.id == self.active).and_then(|t| t.icon.clone())
                            .or_else(|| self.icons.get(&self.active).cloned());
                        if let Some(tex) = icon {
                            let (r, _) = ui.allocate_exact_size(vec2(22.0, 28.0), Sense::hover());
                            egui::Image::new(&tex).tint(INK).paint_at(ui, Rect::from_center_size(r.center(), vec2(22.0, 22.0)));
                            ui.add_space(SPACE_XS);
                        }
                        if !self.status.is_empty() {
                            ui.label(egui::RichText::new(&self.status).font(strong(TYPE_BODY)).color(INK));
                            ui.add_space(SPACE_S);
                        }
                        if self.active == "fillet" && self.panels.data["bevel"].is_object() {
                            self.panels.bevel(ui, &mut self.actions);
                            return;
                        }
                        for (id, label, key, state) in &self.options {
                            if *state == 2 {
                                ui.label(egui::RichText::new(label).font(meta()).color(WARN));
                                ui.add_space(SPACE_S);
                                continue;
                            }
                            let text = if key.is_empty() { label.clone() } else { format!("{label}  {key}") };
                            let item = (id.clone(), text, String::new(), String::new(), *state as i8, true);
                            if item_button(ui, &item, None) { clicked = Some(format!("opt:{id}")); }
                        }
                        if !self.options.is_empty() { bar_divider(ui); }
                        ui.add_space(SPACE_S);
                        let dock_w = if self.review { 0.0 } else { self.dock_w };
                        let room = (ui.available_width() - dock_w - 8.0).max(40.0);
                        let mut job = egui::text::LayoutJob::single_section(self.hint.clone(), egui::TextFormat {
                            font_id: body(), color: DIM, ..Default::default() });
                        job.wrap.max_width = room;
                        job.wrap.max_rows = 1;
                        job.wrap.break_anywhere = true;
                        let g = ui.painter().layout_job(job);
                        let (r, _) = ui.allocate_exact_size(vec2(room, 28.0), Sense::hover());
                        ui.painter().galley(pos2(r.min.x, r.center().y - g.size().y / 2.0), g, DIM);
                        if !self.review {
                            if let Some((active, _, _)) = &self.delivery {
                                if !active.is_empty() {
                                    bar_divider(ui);
                                    if text_button(ui, &format!("✎ {active}"), "New geometry goes here — click to see its items", true) {
                                        clicked = Some("deliver:active".into());
                                    }
                                }
                            }
                        }
                    });
                });
            });
        if let Some(a) = clicked { self.actions.push(a); }
    }

    /// With Paint active: the model's materials as a strip above the bottom
    /// bar, the current one ringed; Library opens the full catalogue in the dock.
    fn paint_strip(&mut self, ctx: &egui::Context, screen: Rect) {
        if self.palette_items.is_empty() { return; }
        let (_, band_bottom) = self.band(ctx, screen);
        let centre = (GUT + 72.0 + self.free_right(ctx, screen)) / 2.0;
        let mut clicked: Option<String> = None;
        egui::Area::new(egui::Id::new("paint_strip"))
            .pivot(egui::Align2::CENTER_BOTTOM)
            .fixed_pos(pos2(centre, band_bottom))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Pill).inner_margin(Margin::same(4)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(SPACE_XS, 0.0);
                        for (id, name, hex, icon, active) in &self.palette_items {
                            // "-": a rule between the model's and the recent ones;
                            // no id: the current material's name.
                            if id == "-" { bar_divider(ui); continue; }
                            if id.is_empty() {
                                ui.add_space(SPACE_XS);
                                ui.label(egui::RichText::new(name).font(body()).color(INK));
                                ui.add_space(SPACE_XS);
                                continue;
                            }
                            let (r, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                            let inner = r.shrink(3.0);
                            if let Some(tex) = self.icons.get(icon) {
                                egui::Image::new(tex).corner_radius(CornerRadius::same(5)).paint_at(ui, inner);
                            } else if let Some(c) = parse_hex(hex) {
                                ui.painter().rect_filled(inner, CornerRadius::same(5), c);
                            }
                            ui.painter().rect_stroke(inner, CornerRadius::same(5), Stroke::new(1.0_f32, EDGE), egui::StrokeKind::Inside);
                            if *active {
                                ui.painter().rect_stroke(r, CornerRadius::same(R_CONTROL), Stroke::new(1.5_f32, ACCENT), egui::StrokeKind::Inside);
                            } else if resp.hovered() {
                                ui.painter().rect_stroke(r, CornerRadius::same(R_CONTROL), Stroke::new(1.0_f32, INK_2), egui::StrokeKind::Inside);
                            }
                            if resp.on_hover_text(name).clicked() { clicked = Some(format!("pal:{id}")); }
                        }
                        if self.palette_items.last().is_some_and(|i| i.0 != "-" && !i.0.is_empty()) { bar_divider(ui); }
                        if text_button(ui, "Browse all…", "Every material: the Library", true) {
                            clicked = Some("libsheet:materials".into());
                        }
                    });
                });
            });
        if let Some(a) = clicked { self.actions.push(a); }
    }

    /// ⌘K / F3: every command of the window by name, at the top of the model.
    fn command_palette(&mut self, ctx: &egui::Context, screen: Rect) {
        if !self.cmd_open { return; }
        let mut run: Option<usize> = None;
        let mut changed = false;
        egui::Area::new(egui::Id::new("commands"))
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(pos2(screen.center().x, TOP_H + 60.0))
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                glass(Surface::Raised).inner_margin(Margin::same(8)).show(ui, |ui| {
                    ui.set_width(520.0);
                    ui.set_max_width(520.0);
                    let r = ui.add(kit::field(&mut self.cmd_query)
                        .hint_text(egui::RichText::new("Search commands").color(FAINT))
                        .font(body()).margin(Margin::symmetric(10, 7))
                        .desired_width(520.0 - 20.0 - 4.0).clip_text(true));
                    if self.cmd_focus { r.request_focus(); self.cmd_focus = false; }
                    if r.changed() { changed = true; self.cmd_at = 0; }
                    ui.add_space(SPACE_S);
                    ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                    for (i, (label, path, key, enabled)) in self.cmd_results.iter().enumerate() {
                        let (rect, resp) = ui.allocate_exact_size(vec2(520.0 - 16.0, CONTROL_H), Sense::click());
                        if i == self.cmd_at { ui.painter().rect_filled(rect, CornerRadius::same(R_ROW), ACTIVE); }
                        else if resp.hovered() { ui.painter().rect_filled(rect, CornerRadius::same(R_ROW), HOVER); }
                        let ink = if *enabled { INK } else { OFF };
                        let g = ui.painter().layout_no_wrap(label.clone(), body(), ink);
                        let lw = g.size().x;
                        ui.painter().galley(pos2(rect.min.x + 10.0, rect.center().y - g.size().y / 2.0), g, ink);
                        ui.painter().text(pos2(rect.min.x + 18.0 + lw, rect.center().y), egui::Align2::LEFT_CENTER, path, meta(), FAINT);
                        ui.painter().text(pos2(rect.max.x - 10.0, rect.center().y), egui::Align2::RIGHT_CENTER, key, meta(), FAINT);
                        if resp.clicked() && *enabled { run = Some(i); }
                        if resp.hovered() && ui.input(|i| i.pointer.delta() != Vec2::ZERO) { self.cmd_at = i; }
                    }
                    if self.cmd_results.is_empty() && !self.cmd_query.is_empty() {
                        ui.label(egui::RichText::new("No command matches").font(body()).color(FAINT));
                    }
                });
            });
        if changed { self.actions.push(format!("cmdq:{}", self.cmd_query)); }
        if let Some(i) = run {
            self.cmd_open = false;
            self.actions.push(format!("cmdrun:{i}"));
        }
    }

    /// The one open menu: documents, workspace or scenes (View has its own).
    fn bar_menu(&mut self, ctx: &egui::Context) {
        let Some((kind, anchor)) = self.menu.clone() else { return };
        let mut clicked: Option<String> = None;
        egui::Area::new(egui::Id::new("menu"))
            .fixed_pos(pos2(anchor.min.x, anchor.max.y + 8.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Raised).inner_margin(Margin::same(6)).show(ui, |ui| {
                    ui.set_width(220.0);
                    ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                    let check = |on: bool| if on { 1i8 } else { 0i8 };
                    match kind.as_str() {
                        "docs" => {
                            if let Some((_, docs)) = &self.projects {
                                for (i, (name, active, dirty)) in docs.iter().enumerate() {
                                    let label = if *dirty { format!("{name} •") } else { name.clone() };
                                    if menu_row(ui, &(String::new(), label, String::new(), "check".into(), check(*active), true)) {
                                        clicked = Some(format!("proj:{i}"));
                                    }
                                }
                                ui.add_space(SPACE_XS);
                                if menu_row(ui, &(String::new(), "New project".into(), "⌘N".into(), "check".into(), -1, true)) {
                                    clicked = Some("proj:new".into());
                                }
                                if let Some(i) = docs.iter().position(|d| d.1) {
                                    if menu_row(ui, &(String::new(), "Close project".into(), "⌘W".into(), "check".into(), -1, true)) {
                                        clicked = Some(format!("proj:close:{i}"));
                                    }
                                }
                                if menu_row(ui, &(String::new(), "Start page".into(), String::new(), "check".into(), -1, true)) {
                                    clicked = Some("start:show".into());
                                }
                            }
                        }
                        "work" | "scenes" => {
                            let group = if kind == "work" { 0 } else { 1 };
                            let mut current = None;
                            if let Some(items) = self.tabs.get(group) {
                                for item in items {
                                    if item.4 == 1 { current = Some(item.1.clone()); }
                                    let row = (item.0.clone(), item.1.clone(), String::new(), "check".into(), check(item.4 == 1), true);
                                    if menu_row(ui, &row) { clicked = Some(format!("tab:{}", item.0)); }
                                }
                            }
                            if kind == "scenes" {
                                // Managing scenes, under the list (council: one place).
                                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 7.0), Sense::hover());
                                ui.painter().hline(r.x_range().shrink(6.0), r.center().y, Stroke::new(1.0_f32, EDGE));
                                let mut rows: Vec<(&str, String)> = vec![("scene:camera", "Camera inspector…".into()), ("scene:add", "Add scene from this view".into())];
                                if let Some(c) = &current {
                                    rows.push(("scene:update", format!("Update “{c}” from this view")));
                                    rows.push(("scene:rename", format!("Rename “{c}”…")));
                                    rows.push(("scene:up", "Move earlier".into()));
                                    rows.push(("scene:down", "Move later".into()));
                                    rows.push(("scene:delete", format!("Delete “{c}”")));
                                }
                                for (id, label) in rows {
                                    if menu_row(ui, &(id.to_owned(), label, String::new(), "check".into(), -1, true)) {
                                        clicked = Some(format!("tab:{id}"));
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                });
            });
        if let Some(a) = clicked {
            self.menu = None;
            self.actions.push(a);
        }
    }

    /// View ▾, under its button at the right of the top bar.
    fn view_popup(&mut self, ctx: &egui::Context) {
        if !self.view_open { return; }
        let mut clicked: Option<String> = None;
        let mut close = false;
        let mut toggle_review = false;
        let a = self.view_anchor;
        egui::Area::new(egui::Id::new("view_menu"))
            .pivot(egui::Align2::RIGHT_TOP)
            .fixed_pos(pos2(a.max.x, a.max.y + 8.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Raised).inner_margin(Margin::same(6)).show(ui, |ui| {
                    ui.set_width(236.0);
                    ui.spacing_mut().item_spacing = vec2(SPACE_XS, SPACE_XS);
                    let review = (String::new(), "Review — hide the panels".to_owned(), String::new(), "check".to_owned(),
                                  if self.review { 1 } else { 0 }, true);
                    if menu_row(ui, &review) { toggle_review = true; }
                    ui.add_space(SPACE_XS);
                    for (si, (title, items)) in self.view_menu.iter().enumerate() {
                        if si > 0 { ui.add_space(SPACE_XS); }
                        caption_at(ui, title, 12.0);
                        let icons_only = items.iter().all(|i| self.icons.contains_key(&i.3));
                        if icons_only {
                            ui.horizontal_wrapped(|ui| {
                                for item in items {
                                    if item_button(ui, item, self.icons.get(&item.3)) { clicked = Some(item.0.clone()); }
                                }
                            });
                        } else {
                            for item in items {
                                if menu_row(ui, item) {
                                    clicked = Some(item.0.clone());
                                    close = item.4 < 0;
                                }
                            }
                        }
                    }
                });
            });
        if close { self.view_open = false; }
        if toggle_review {
            self.review = !self.review;
            self.view_open = false;
        }
        if let Some(id) = clicked { self.actions.push(format!("act:{id}")); }
    }

    /// The settings form, hung under View ▾ like its menu (one floats at a time).
    fn form_popup(&mut self, ctx: &egui::Context) {
        let Some(form) = self.form.as_mut() else { return };
        // Under View ▾ at the window's right edge, over the dock.
        let screen = ctx.screen_rect();
        let at = pos2(screen.max.x - GUT, TOP_H + GUT);
        let mut close = false;
        let mut out: Vec<String> = Vec::new();
        let Form { title, fields, choice_open } = form;
        egui::Area::new(egui::Id::new("form"))
            .pivot(egui::Align2::RIGHT_TOP)
            .fixed_pos(at)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Raised).inner_margin(Margin::same(8)).show(ui, |ui| {
                    ui.set_width(272.0);
                    ui.spacing_mut().item_spacing = vec2(0.0, SPACE_XS);
                    let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
                    ui.painter().text(hr.left_center() + vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, title.as_str(),
                        title_font(), INK);
                    let xr = Rect::from_center_size(hr.right_center() - vec2(12.0, 0.0), vec2(22.0, 22.0));
                    let xresp = ui.interact(xr, ui.id().with("form_close"), Sense::click());
                    if xresp.hovered() { ui.painter().rect_filled(xr, CornerRadius::same(R_ROW as u8), HOVER); }
                    ui.painter().text(xr.center(), egui::Align2::CENTER_CENTER, "×", body(), DIM);
                    if xresp.clicked() { close = true; }
                    let w = ui.available_width();
                    for (i, f) in fields.iter_mut().enumerate() {
                        let (id, kind, label, text) = (f.0.clone(), f.1.clone(), f.2.clone(), f.3.clone());
                        match kind.as_str() {
                            "caption" => { ui.add_space(SPACE_XS); caption_at(ui, &label, 6.0); }
                            "text" => {
                                egui::Frame::NONE.inner_margin(Margin { left: 6, right: 6, top: 2, bottom: 2 }).show(ui, |ui| {
                                    ui.label(egui::RichText::new(&text).font(meta()).color(FAINT));
                                });
                            }
                            "check" => {
                                let item = (id.clone(), label.clone(), text.clone(), "check".to_owned(), if f.4 > 0.5 { 1 } else { 0 }, true);
                                if menu_row(ui, &item) {
                                    f.4 = if f.4 > 0.5 { 0.0 } else { 1.0 };
                                    out.push(format!("form:{id}:{}", f.4 as i32));
                                }
                            }
                            "slider" => {
                                let (r, _) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
                                ui.painter().text(r.left_center() + vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, &label, body(), DIM);
                                ui.painter().text(r.right_center() - vec2(6.0, 0.0), egui::Align2::RIGHT_CENTER, &text, body(), INK);
                                let (sr, resp) = ui.allocate_exact_size(vec2(w, 18.0), Sense::click_and_drag());
                                let track = Rect::from_center_size(sr.center(), vec2(sr.width() - 24.0, 4.0));
                                let span = (f.6 - f.5).max(1e-6);
                                if resp.dragged() || resp.clicked() {
                                    if let Some(p) = resp.interact_pointer_pos() {
                                        let t = ((p.x - track.min.x) / track.width()).clamp(0.0, 1.0);
                                        let v = (f.5 + t * span).round();
                                        if v != f.4 { f.4 = v; out.push(format!("form:{id}:{v}")); }
                                    }
                                }
                                let t = ((f.4 - f.5) / span).clamp(0.0, 1.0);
                                ui.painter().rect_filled(track, CornerRadius::same(2), FIELD);
                                let filled = Rect::from_min_max(track.min, pos2(track.min.x + t * track.width(), track.max.y));
                                ui.painter().rect_filled(filled, CornerRadius::same(2), ACCENT);
                                let knob = pos2(track.min.x + t * track.width(), track.center().y);
                                ui.painter().circle_filled(knob, if resp.hovered() || resp.dragged() { 7.0 } else { 6.0 }, INK);
                            }
                            "choice" => {
                                let (r, resp) = ui.allocate_exact_size(vec2(w, CONTROL_H), Sense::click());
                                if resp.hovered() { ui.painter().rect_filled(r, CornerRadius::same(R_ROW as u8), HOVER); }
                                ui.painter().text(r.left_center() + vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, &label, body(), DIM);
                                let open = *choice_open == Some(i);
                                ui.painter().text(r.right_center() - vec2(6.0, 0.0), egui::Align2::RIGHT_CENTER,
                                    format!("{text}  {}", if open { "▴" } else { "▾" }), body(), INK);
                                if resp.clicked() { *choice_open = if open { None } else { Some(i) }; }
                                if open {
                                    for (j, opt) in f.7.iter().enumerate() {
                                        let item = (j.to_string(), format!("   {opt}"), String::new(), "check".to_owned(),
                                                    if j as f32 == f.4 { 1 } else { -1 }, true);
                                        if menu_row(ui, &item) {
                                            f.4 = j as f32;
                                            f.3 = opt.clone();
                                            out.push(format!("form:{id}:{j}"));
                                            *choice_open = None;
                                        }
                                    }
                                }
                            }
                            "color" => {
                                let (r, resp) = ui.allocate_exact_size(vec2(w, CONTROL_H), Sense::click());
                                if resp.hovered() { ui.painter().rect_filled(r, CornerRadius::same(R_ROW as u8), HOVER); }
                                ui.painter().text(r.left_center() + vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, &label, body(), DIM);
                                let sw = Rect::from_center_size(r.right_center() - vec2(22.0, 0.0), vec2(32.0, 16.0));
                                if let Some(c) = parse_hex(&text) { ui.painter().rect_filled(sw, CornerRadius::same(4), c); }
                                ui.painter().rect_stroke(sw, CornerRadius::same(4), Stroke::new(1.0_f32, EDGE), egui::StrokeKind::Inside);
                                if resp.clicked() { out.push(format!("form:{id}:pick")); }
                            }
                            "button" => {
                                ui.add_space(SPACE_XS);
                                ui.horizontal(|ui| {
                                    ui.add_space(SPACE_S);
                                    if chip_button(ui, &label) { out.push(format!("form:{id}:")); }
                                });
                            }
                            _ => {}
                        }
                    }
                });
            });
        if close {
            self.form = None;
            out.push("form:close".into());
        }
        self.actions.extend(out);
    }

    /// More ▾ of the selection actions (opened from the inspector), with
    /// the modeller's secondary menus as submenus; both open leftwards, into
    /// the model, since the dock sits at the right.
    fn selection_more(&mut self, ctx: &egui::Context) {
        let Some((_, actions)) = &self.selection else { self.sel_more = false; return };
        if !self.sel_more { self.sel_sub = None; return; }
        let mut clicked = None;
        let a = self.sel_more_anchor;
        let menu = egui::Area::new(egui::Id::new("sel_more"))
            .pivot(egui::Align2::RIGHT_TOP)
            .fixed_pos(pos2(a.max.x, a.max.y + 6.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Raised).inner_margin(Margin::same(6)).show(ui, |ui| {
                    ui.set_width(230.0);
                    ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                    let mut seen: Vec<String> = Vec::new();
                    for (id, label, key, enabled, more) in actions {
                        if !*more { continue; }
                        if let Some((prefix, _)) = label.split_once(" ▸ ") {
                            if seen.iter().any(|p| p == prefix) { continue; }
                            seen.push(prefix.to_owned());
                            let open = self.sel_sub.as_ref().is_some_and(|(p, _)| p == prefix);
                            let item = (String::new(), prefix.to_owned(), "‹".to_owned(), String::new(), -1i8, true);
                            let (resp, rect) = menu_row_resp(ui, &item);
                            if open { ui.painter().rect_filled(rect, CornerRadius::same(R_ROW), HOVER); }
                            if resp.hovered() || resp.clicked() { self.sel_sub = Some((prefix.to_owned(), rect)); }
                            continue;
                        }
                        let item = (id.clone(), label.clone(), key.clone(), String::new(), -1i8, *enabled);
                        let (resp, _) = menu_row_resp(ui, &item);
                        if resp.hovered() { self.sel_sub = None; }
                        if resp.clicked() && *enabled { clicked = Some(id.clone()); }
                    }
                });
            });
        let left = menu.response.rect.min.x;
        if let Some((prefix, row)) = self.sel_sub.clone() {
            egui::Area::new(egui::Id::new("sel_sub"))
                .pivot(egui::Align2::RIGHT_TOP)
                .fixed_pos(pos2(left - 6.0, row.min.y - 6.0))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    glass(Surface::Raised).inner_margin(Margin::same(6)).show(ui, |ui| {
                        ui.set_width(170.0);
                        ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                        for (id, label, key, enabled, _) in actions {
                            if let Some((p, rest)) = label.split_once(" ▸ ") {
                                if p != prefix { continue; }
                                let item = (id.clone(), rest.to_owned(), key.clone(), String::new(), -1i8, *enabled);
                                if menu_row(ui, &item) && *enabled { clicked = Some(id.clone()); }
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
    }
    /// Top and bottom of the band between the bars, measured from where the
    /// bars were laid out (so the side panes keep the same 8 pt above and
    /// below, whatever the bars' content).
    fn band(&self, ctx: &egui::Context, screen: Rect) -> (f32, f32) {
        let top = ctx.memory(|m| m.area_rect(egui::Id::new("top"))).map_or(TOP_H, |r| r.max.y);
        let bottom = if self.review { screen.max.y } else {
            ctx.memory(|m| m.area_rect(egui::Id::new("bottom"))).map_or(screen.max.y - BOT_H, |r| r.min.y)
        };
        (top + GUT, bottom - GUT)
    }

    /// The right edge of the free viewport (left of the column).
    fn free_right(&self, ctx: &egui::Context, screen: Rect) -> f32 {
        if self.column.is_none() { return screen.max.x; }
        ctx.memory(|m| m.area_rect(egui::Id::new("column"))).map_or(screen.max.x, |r| r.min.x)
    }

    fn right_column(&mut self, ctx: &egui::Context, screen: Rect) {
        let delivery = self.delivery.clone();
        let selection = self.selection.clone();
        let sel_more = self.sel_more;
        let mut cmds: Vec<String> = Vec::new();
        let mut more_at: Option<Rect> = None;
        let (top, bottom) = self.band(ctx, screen);
        let self_icons = self.icons.clone();
        let Some(col) = self.column.as_mut() else { return };
        #[allow(non_snake_case)]
        let W: f32 = self.dock_w.min((screen.width() * 0.45).max(DOCK_MIN_W));
        let height = (bottom - top).max(200.0);
        let mut actions: Vec<String> = Vec::new();
        egui::Area::new(egui::Id::new("column"))
            .fixed_pos(pos2(screen.max.x - GUT - W, top))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Panel).inner_margin(Margin::same(10)).show(ui, |ui| {
                    ui.set_width(W - 20.0);
                    ui.set_max_width(W - 20.0);
                    ui.set_height(height - 20.0);
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    // Header: Project, its mode (Problems) with its way back,
                    // then the issues toggle and the structure menu. (The
                    // Library is a sheet now, ui/large_sheet.rs.)
                    let in_library = col.mode == "Library";
                    if self.panels.data["camera"].is_object() {
                        egui::ScrollArea::vertical().id_salt("camera inspector").show(ui, |ui| self.panels.camera(ui,&mut cmds));
                        return;
                    }
                    ui.horizontal(|ui| {
                        ui.set_min_height(28.0);
                        // Project, or Plugins when there are some (the Library is
                        // a sheet now, ui/large_sheet.rs).
                        let has_plugins = self.plugins.available();
                        if has_plugins {
                            let at = usize::from(self.plugins.showing);
                            match widgets::segmented(ui, &["Project", "Plugins"], at, 150.) {
                                Some(0) => { self.plugins.showing = false; actions.push("mode\u{1f}".into()); }
                                Some(1) => self.plugins.showing = true,
                                _ => {}
                            }
                        } else {
                            ui.add_space(SPACE_XS);
                            let (pr, presp) = ui.allocate_exact_size(vec2(58.0, 28.0), Sense::click());
                            ui.painter().text(pr.left_center(), egui::Align2::LEFT_CENTER, "Project", title_font(), INK);
                            if presp.clicked() && !col.mode.is_empty() { actions.push("mode\u{1f}".into()); }
                        }
                        if self.plugins.showing { return; }
                        if !col.mode.is_empty() && !in_library && widgets::caption_mode(ui, &col.mode) {
                            actions.push("mode\u{1f}".into());
                        }
                        if in_library { return; }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let dots = (String::new(), "⋯".to_owned(), "Structure: new node, propose from tags…".to_owned(), String::new(), -1i8, true);
                            let before = ui.cursor();
                            if item_button(ui, &dots, None) {
                                col.head_menu = Some(pos2(before.max.x, before.min.y + 30.0));
                            }
                            // What stands between the model and delivery (one home: here).
                            let total = if col.mode.is_empty() && col.issues {
                                delivery.as_ref().map_or(0, |d| d.2) + col.unfiled
                            } else { 0 };
                            if total > 0 && amber_button(ui, &format!("{total} issues")) { cmds.push("deliver:problems".into()); }
                            if col.mode.is_empty() {
                                let tip = if col.issues { "Hide issues" } else { "Show issues: problem dots and what blocks delivery" };
                                let t = (String::new(), "Issues".to_owned(), tip.to_owned(), String::new(), if col.issues { 1 } else { 0 }, true);
                                if item_button(ui, &t, None) { actions.push("issues\u{1f}".into()); }
                            }
                        });
                    });
                    if self.plugins.showing {
                        self.plugins.dock(ui, &mut cmds);
                        return;
                    }
                    if !self.link_banner.is_empty() {
                        ui.add_space(SPACE_S);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new(&self.link_banner).font(body()).color(WARN));
                            if ui.small_button("Locate…").clicked() { cmds.push("linkbanner:locate".into()); }
                            if ui.small_button("×").on_hover_text("Dismiss").clicked() { cmds.push("linkbanner:close".into()); }
                        });
                    }
                    ui.add_space(SPACE_S);
                    if col.mode.is_empty() {
                        // Column sets: second-level tabs over the same rows.
                        let sets: Vec<(String, String, bool)> = ["Filing", "Drawing", "Deliver"].iter()
                            .map(|l| (l.to_string(), String::new(), true)).collect();
                        if let Some(i) = widgets::underline_tabs(ui, &sets, col.colset as usize) {
                            if col.colset != i as u8 {
                                col.colset = i as u8;
                                actions.push(format!("colset\u{1f}{i}"));
                            }
                        }
                    }
                    ui.add_space(SPACE_S);
                    // The Library takes the tree's place (the tree code below
                    // stays as it was, in a block it can leave).
                    'tree: {
                    if col.mode.is_empty() && col.colset == 2 {
                        ui.horizontal(|ui| {
                            caption(ui, "Deliveries");
                            if chip_button(ui, "+ New delivery") { cmds.push("deliver:new".into()); }
                        });
                        ui.add_space(SPACE_S);
                        egui::ScrollArea::vertical().id_salt("deliveries")
                            .max_height(ui.available_height() * 0.42).show(ui, |ui| {
                                for n in &col.nodes {
                                    ui.horizontal(|ui| {
                                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width() - 56.0, 40.0), Sense::click());
                                        paint_state(ui, r, state_of(&resp, n.active, true), R_ROW);
                                        ui.painter().text(pos2(r.min.x + 6.0, r.center().y), egui::Align2::LEFT_CENTER,
                                            &n.kind, caption_font(), ACCENT);
                                        for (label, y, font, color) in [(&n.name, 12.0, strong(TYPE_BODY), INK), (&n.badge, 28.0, meta(), DIM)] {
                                            let mut line = egui::text::LayoutJob::simple_singleline(label.clone(), font, color);
                                            line.wrap.max_width = (r.width() - 50.0).max(12.0);
                                            line.wrap.max_rows = 1;
                                            let galley = ui.painter().layout_job(line);
                                            ui.painter().galley(pos2(r.min.x + 44.0, r.min.y + y - galley.size().y / 2.0), galley, color);
                                        }
                                        if resp.on_hover_text(&n.name).clicked() { cmds.push(format!("deliver:select:{}", n.key)); }
                                        if chip_button(ui, "▶ Run") { cmds.push(format!("deliver:run:{}", n.key)); }
                                    });
                                }
                            });
                        break 'tree;
                    }
                    if let Some(what) = &col.filing {
                        // Two short lines, never cut off at the dock's edge.
                        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
                        ui.painter().rect_filled(r, CornerRadius::same(R_ROW), ACTIVE);
                        ui.painter().text(pos2(r.min.x + 8.0, r.min.y + 11.0), egui::Align2::LEFT_CENTER,
                            format!("File {what} to…"), caption_font(), BRIGHT);
                        ui.painter().text(pos2(r.min.x + 8.0, r.max.y - 10.0), egui::Align2::LEFT_CENTER,
                            "Click a node, or type a name and Enter", meta(), INK_2);
                        ui.add_space(SPACE_XS);
                    }
                    let edit = kit::field(&mut col.filter)
                        .hint_text(egui::RichText::new(if col.filing.is_some() { "Node name or kind" }
                            else if col.mode == "Problems" { "Filter problems" } else { "Filter nodes" }).color(FAINT))
                        .margin(Margin::symmetric(8, 5))
                        .desired_width((W - 20.0 - 16.0 - 4.0).max(40.0))
                        .font(body());
                    let resp = ui.add(edit);
                    if col.focus_filter {
                        resp.request_focus();
                        col.focus_filter = false;
                    }
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if resp.changed() {
                        actions.push(format!("filter\u{1f}{}", col.filter));
                    }
                    ui.add_space(SPACE_XS);
                    // Rows, as Python sends them (it owns expansion and filtering).
                    if enter {
                        if col.filing.is_some() {
                            if let Some(n) = col.ordered.iter().map(|&i| &col.nodes[i]).find(|n| n.row == 0 && n.key.len() > 2 && !n.key.starts_with("n:link:")) {
                                actions.push(format!("file\u{1f}{}", n.key));
                                col.filing = None;
                                col.filter.clear();
                                actions.push("filter\u{1f}".into());
                            }
                        }
                    }
                    // A dock mode with nothing to inspect gives the whole dock to its list.
                    let room = if !col.mode.is_empty() && col.inspector.is_empty() { ui.available_height() - 44.0 } else { ui.available_height() * 0.6 };
                    if col.nodes.is_empty() {
                        ui.add_space(SPACE_S);
                        ui.horizontal_wrapped(|ui| {
                            ui.add_space(SPACE_S);
                            ui.label(egui::RichText::new("No project structure yet.").font(body()).color(DIM));
                        });
                        ui.add_space(SPACE_S);
                        ui.horizontal(|ui| {
                            ui.add_space(SPACE_S);
                            if chip_button(ui, "Propose from tags…") { actions.push("propose\u{1f}".into()); }
                            if chip_button(ui, "New node") { actions.push("new\u{1f}".into()); }
                        });
                    }
                    let nodes_len = col.nodes.len();
                    // The column set as a table (tree C): one layout for the
                    // header and every row, names never crushed.
                    let row_left = ui.cursor().min.x;
                    let cols = tree_cols::Cols::plan(col.colset, row_left, row_left + ui.available_width());
                    let row_room = if col.mode.is_empty() && nodes_len > 0 {
                        let header = cols.header(ui, row_left);
                        drawing::drop_on(ui, &header, "", &mut actions);
                        room - ROW_H
                    } else { room };
                    let tree_h = (nodes_len as f32 * ROW_H + SPACE_XS).min(row_room).max(3.0 * ROW_H);
                    egui::ScrollArea::vertical().id_salt("tree").max_height(tree_h).auto_shrink([false, false])
                        .show_rows(ui, ROW_H, nodes_len, |ui, range| {
                            for i in range {
                                let index = if col.mode.is_empty() { col.ordered[i] } else { i };
                                let n = &col.nodes[index];
                                let editable = n.row == 0 && n.key.starts_with("n:") && n.key.len() > 2 && !n.key.starts_with("n:link:");
                                let sense = if editable { Sense::click_and_drag() } else { Sense::click() };
                                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), sense);
                                if editable {
                                    if resp.drag_started() { resp.dnd_set_drag_payload(drawing::DraggedNode(n.key.clone())); }
                                    drawing::drop_on(ui, &resp, n.key.strip_prefix("n:").unwrap_or(""), &mut actions);
                                }
                                // Items in the selection are lit like a picked row; nodes count them.
                                let lit = col.highlighted.as_deref() == Some(n.key.as_str()) || n.mark == 1;
                                let file_state = if n.row == 0 { tree_cols::file_state(&n.icon) } else { None };
                                let cols = tree_cols::Cols::plan(col.colset, rect.min.x, rect.max.x);
                                let linked = n.row == 0 && n.key.starts_with("n:link");
                                let filing = col.filing.is_some() && n.row == 0 && !linked;
                                if lit {
                                    ui.painter().rect_filled(rect, CornerRadius::same(R_ROW), ACTIVE);
                                    ui.painter().rect_filled(Rect::from_center_size(pos2(rect.min.x + 1.0, rect.center().y), vec2(2.0, 12.0)),
                                        CornerRadius::same(1), ACCENT);
                                } else if resp.hovered() {
                                    ui.painter().rect_filled(rect, CornerRadius::same(R_ROW), if filing { ACTIVE } else { HOVER });
                                }
                                let cy = rect.center().y;
                                let x0 = rect.min.x + 4.0 + n.depth as f32 * 14.0;
                                let tri = Rect::from_min_size(pos2(x0, rect.min.y), vec2(14.0, ROW_H));
                                if n.expandable {
                                    ui.painter().text(tri.center(), egui::Align2::CENTER_CENTER, if n.expanded { "▾" } else { "▸" },
                                        meta(), if n.row == 0 { FAINT } else { OFF });
                                }
                                // Right columns: problems, count, then eye and lock —
                                // or the Drawing / Export cells.

                                let mut cells_left = f32::MAX;
                                let warn_col = rect.max.x - 6.0;
                                if col.colset != 0 {
                                    let cells = &n.cells;
                                    let cell = |i: usize| cells.get(i).cloned().unwrap_or_default();
                                    let inherited = n.row != 0;
                                    let ink_c = if inherited { OFF } else { DIM };
                                    if col.colset == 1 && !cells.is_empty() {
                                        for (slot, index) in [(cols.weight, 2), (cols.line, 1), (cols.hatch, 4)] {
                                            if let Some((start, end)) = slot {
                                                ui.painter().with_clip_rect(Rect::from_min_max(pos2(start, rect.top()), pos2(end, rect.bottom()))).text(pos2(end, cy), egui::Align2::RIGHT_CENTER, cell(index), meta(), ink_c);
                                            }
                                        }
                                        if let Some((_, end)) = cols.print {
                                            ui.painter().text(pos2(end, cy), egui::Align2::RIGHT_CENTER, if cell(3) == "1" { "✓" } else { "–" }, meta(), ink_c);
                                        }
                                        if let Some((a,b)) = cols.swatch {
                                            let sw = Rect::from_center_size(pos2((a+b)*0.5,cy),vec2(12.0,12.0));
                                            if let Some(color) = parse_hex(&cell(0)) { ui.painter().rect_filled(sw, CornerRadius::same(R_INSET), color); }
                                            ui.painter().rect_stroke(sw, CornerRadius::same(R_INSET), Stroke::new(1.0, EDGE), egui::StrokeKind::Inside);
                                        }
                                    } else if col.colset == 2 {
                                        // Quantity at the edge, the export class (and type) before it.
                                        let qty = cell(2);
                                        let (qa, qb) = cols.qty.unwrap_or((warn_col - 64.0, warn_col));
                                        if !qty.is_empty() {
                                            let mut job = egui::text::LayoutJob::single_section(qty, egui::TextFormat {
                                                font_id: meta(), color: if n.row == 0 { INK_2 } else { DIM }, ..Default::default() });
                                            job.wrap.max_width = qb - qa;
                                            job.wrap.max_rows = 1;
                                            job.wrap.break_anywhere = true;
                                            let g = ui.painter().layout_job(job);
                                            ui.painter().galley(pos2(qb - g.size().x, cy - g.size().y / 2.0), g, DIM);
                                        }
                                        let text = if cell(1).is_empty() { cell(0) } else { format!("{} · {}", cell(0), cell(1)) };
                                        let text = text.strip_prefix("Ifc").map(|t| t.to_string()).unwrap_or(text);
                                        let (ca, cb) = cols.class.unwrap_or((qa - 100.0, qa - 8.0));
                                        let mut job = egui::text::LayoutJob::single_section(text, egui::TextFormat {
                                            font_id: meta(), color: FAINT, ..Default::default() });
                                        job.wrap.max_width = cb - ca;
                                        job.wrap.max_rows = 1;
                                        job.wrap.break_anywhere = true;
                                        let g = ui.painter().layout_job(job);
                                        cells_left = ca;
                                        ui.painter().galley(pos2(ca, cy - g.size().y / 2.0), g, FAINT);
                                    }
                                }
                                let warn_col = if col.colset != 0 { rect.max.x + 1000.0 } else { warn_col };
                                if n.problems > 0 && (col.issues || !col.mode.is_empty()) {
                                    let g = ui.painter().layout_no_wrap(n.problems.to_string(), meta(), WARN);
                                    let w = g.size().x;
                                    ui.painter().galley(pos2(warn_col - w, cy - g.size().y / 2.0), g, WARN);
                                    ui.painter().circle_filled(pos2(warn_col - w - 6.0, cy), 2.5, WARN);
                                }
                                let count_col = warn_col - 40.0;
                                if n.count > 0 || n.row == 0 {
                                    let g = ui.painter().layout_no_wrap(n.count.to_string(), meta(), DIM);
                                    ui.painter().galley(pos2(count_col - g.size().x, cy - g.size().y / 2.0), g, DIM);
                                }
                                let show_ctl = resp.hovered() || n.eye == 0 || n.lock == 1;
                                let control_edge = cols.flags.map_or(count_col, |(a, _)| a + 74.0);
                                let (eye_r, lock_r) = tree_slots(ui, control_edge, cy, n.eye, n.lock, show_ctl || col.colset == 1);
                                // A row with no eye or lock (problems, library) keeps its width for the name.
                                let bare = n.eye < 0 && n.lock < 0 && n.problems == 0;
                                let right = if col.colset == 1 { cols.name_right } else if col.colset == 2 { cells_left.min(cols.name_right) - 8.0 }
                                    else if (bare || !show_ctl) && (n.count > 0 || n.row == 0) { count_col - 34.0 }
                                    else if bare { rect.max.x - 8.0 } else { eye_r.min.x - 6.0 };
                                // Item icon (a material row shows its swatch).
                                let mut tx = x0 + 16.0;
                                if n.row == 5 {
                                    let sw = Rect::from_center_size(pos2(tx + 8.0, cy), vec2(16.0, 16.0));
                                    if let Some(tex) = self_icons.get(&n.icon).filter(|_| !n.icon.is_empty()) {
                                        egui::Image::new(tex).corner_radius(CornerRadius::same(3)).paint_at(ui, sw);
                                    } else if let Some(c) = parse_hex(&n.badge) {
                                        ui.painter().rect_filled(sw, CornerRadius::same(3), c);
                                    } else {
                                        ui.painter().text(sw.center(), egui::Align2::CENTER_CENTER, "◇", meta(), DIM);
                                    }
                                    ui.painter().rect_stroke(sw, CornerRadius::same(3), Stroke::new(1.0_f32, EDGE), egui::StrokeKind::Inside);
                                    if n.active {
                                        ui.painter().rect_stroke(sw.expand(2.0), CornerRadius::same(4), Stroke::new(1.5_f32, ACCENT), egui::StrokeKind::Outside);
                                    }
                                    tx += 22.0;
                                } else if file_state.is_some() {
                                    // A linked file is a different kind of row: a page icon.
                                    tree_cols::paint_file(ui.painter(), pos2(tx + 6.0, cy), if lit { BRIGHT } else { DIM });
                                    tx += 16.0;
                                } else if n.row != 0 && n.row != 4 {
                                    let glyph = match n.icon.as_str() { "component" => "◇", "section" => "⊟", "image" => "▨", "link" => "⛓", _ => "▢" };
                                    ui.painter().text(pos2(tx + 6.0, cy), egui::Align2::CENTER_CENTER, glyph, meta(),
                                        if n.row == 3 { OFF } else { FAINT });
                                    tx += 16.0;
                                }
                                let one_line = |text: String, font: egui::FontId, color: Color32, width: f32| {
                                    let mut job = egui::text::LayoutJob::single_section(text, egui::TextFormat {
                                        font_id: font, color, ..Default::default() });
                                    job.wrap.max_width = width.max(10.0);
                                    job.wrap.max_rows = 1;
                                    job.wrap.break_anywhere = true;
                                    job
                                };
                                let renaming_here = col.renaming.as_ref().is_some_and(|(k, _)| *k == n.key);
                                if renaming_here {
                                    let buf = &mut col.renaming.as_mut().unwrap().1;
                                    let field = kit::field(buf).font(body()).margin(Margin::symmetric(4, 1))
                                        .desired_width((right - tx - 16.0).max(30.0)).clip_text(true);
                                    let r = ui.put(Rect::from_min_max(pos2(tx - 4.0, rect.min.y + 1.0), pos2(right, rect.max.y - 1.0)), field);
                                    if col.focus_rename { r.request_focus(); col.focus_rename = false; }
                                    if r.lost_focus() {
                                        let (key, name) = col.renaming.take().unwrap();
                                        if !ui.input(|i| i.key_pressed(egui::Key::Escape)) && !name.trim().is_empty() && name != n.name {
                                            actions.push(format!("rename\u{1f}{key}\u{1f}{}", name.trim()));
                                        }
                                    }
                                    continue;
                                }
                                let (ink, font) = match n.row {
                                    0 => (if lit || n.active { BRIGHT } else { INK },
                                          if n.active || file_state.is_some() { strong(TYPE_BODY) } else { body() }),
                                    3 | 4 => (FAINT, meta()),
                                    _ => (if lit { BRIGHT } else { INK_2 }, body()),
                                };
                                // The selection is marked once (tree A): lit where it
                                // is held, a quiet dot on the rows above (after the name).
                                let marks_w = if file_state.is_some() { 10.0 } else { 0.0 } + if n.mark == 2 { 10.0 } else { 0.0 };
                                let right = right - marks_w;
                                let name = if n.active && n.row != 5 { format!("✎ {}", n.name) } else { n.name.clone() };
                                let full = name.clone();
                                let g = ui.painter().layout_job(one_line(name, font, ink, right - tx));
                                let name_w = g.size().x;
                                // A name cut at the column shows whole on hover.
                                let cut = g.elided || name_w >= right - tx - 1.0;
                                let resp = if cut { resp.on_hover_text(full) }
                                    else if n.mark == 1 && n.row == 0 && n.selected > 1 { resp.on_hover_text(format!("{} selected here", n.selected)) }
                                    else { resp };
                                ui.painter().galley(pos2(tx, cy - g.size().y / 2.0), g, ink);
                                let mut kx = tx + name_w + 8.0;
                                if let Some(state) = file_state {
                                    let c = pos2(kx + 2.0, cy);
                                    dot(ui.painter(), c, tree_cols::state_color(state));
                                    let tip = match state { "resolved" => "Up to date", "modified" => "Changed on disk: reload to see the changes",
                                        "missing" => "Not found: relink it", _ => "Packed in this file" };
                                    ui.interact(Rect::from_center_size(c, vec2(10.0, 10.0)), ui.id().with(("file_dot", i)), Sense::hover())
                                        .on_hover_text(tip);
                                    kx += 10.0;
                                }
                                if n.mark == 2 {
                                    tree_cols::paint_partial(ui.painter(), pos2(kx + 2.0, cy));
                                    kx += 10.0;
                                }
                                let right = right + marks_w;
                                if !n.badge.is_empty() && n.row != 5 && kx + 20.0 < right {
                                    let col_b = if n.row == 3 { ACCENT } else { FAINT };
                                    let g = ui.painter().layout_job(one_line(n.badge.clone(), meta(), col_b, right - kx));
                                    let w = g.size().x;
                                    ui.painter().galley(pos2(kx, cy - g.size().y / 2.0), g, col_b);
                                    kx += w + 8.0;
                                }
                                if !n.kind.is_empty() && kx + 30.0 < right {
                                    let g = ui.painter().layout_job(one_line(n.kind.clone(), meta(), FAINT, right - kx));
                                    let at = pos2(kx, cy - g.size().y / 2.0);
                                    let kind_rect = Rect::from_min_size(at, g.size());
                                    ui.painter().galley(at, g, FAINT);
                                    // What "Reference", "Drawing only" and the like mean.
                                    if !n.tip.is_empty() {
                                        ui.interact(kind_rect, ui.id().with(("kind_tip", i)), Sense::hover())
                                            .on_hover_text(n.tip.clone());
                                    }
                                } else if !n.tip.is_empty() && resp.hovered() && n.kind.is_empty() {
                                    // A row with no kind word (Linked files): the tip on the name.
                                    let r = Rect::from_min_max(pos2(tx, rect.min.y), pos2(kx, rect.max.y));
                                    ui.interact(r, ui.id().with(("kind_tip", i)), Sense::hover()).on_hover_text(n.tip.clone());
                                }
                                if resp.clicked() {
                                    let p = resp.interact_pointer_pos().unwrap_or(rect.center());
                                    if n.expandable && tri.contains(p) {
                                        actions.push(format!("toggle\u{1f}{}", n.key));
                                    } else if n.row == 6 {
                                        continue;
                                    } else if n.eye >= 0 && eye_r.expand(2.0).contains(p) {
                                        actions.push(format!("eye\u{1f}{}", n.key));
                                    } else if n.lock >= 0 && lock_r.expand(2.0).contains(p) {
                                        actions.push(format!("lock\u{1f}{}", n.key));
                                    } else if col.filing.is_some() {
                                        if n.row == 0 && !linked {
                                            actions.push(format!("file\u{1f}{}", n.key));
                                            col.filing = None;
                                            col.filter.clear();
                                            actions.push("filter\u{1f}".into());
                                        }
                                    } else {
                                        col.highlighted = Some(n.key.clone());
                                        let verb = match n.row { 0 => "node", 3 => "reveal", 4 => "loose", 5 => "mat", _ => "item" };
                                        actions.push(format!("{verb}\u{1f}{}", n.key));
                                    }
                                }
                                if resp.double_clicked() && n.row != 6 && col.filing.is_none() {
                                    let verb = if n.row == 0 { "isolate" } else { "open" };
                                    actions.push(format!("{verb}\u{1f}{}", n.key));
                                }
                                if resp.secondary_clicked() && n.row != 6 && n.row != 4 && (n.row != 5 || n.key.starts_with("cm:")) {
                                    col.row_menu = resp.interact_pointer_pos().map(|p| (n.key.clone(), n.row, p));
                                }
                            }
                        });
                    } // 'tree
                    // A delivery owns the inspector even when geometry is selected.
                    let selection = if col.colset == 2 && col.mode.is_empty() { None } else { selection.as_ref() };
                    // Inspector.
                    ui.add_space(SPACE_S);
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0_f32, EDGE));
                    ui.add_space(SPACE_S);
                    egui::ScrollArea::vertical().id_salt("inspector").auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing = vec2(0.0, SPACE_XS);
                        if col.mode.is_empty() && col.colset != 2 {
                            self.panels.inspector(ui, &mut cmds);
                        }
                        if col.inspector_title.is_empty() {
                            ui.horizontal(|ui| { ui.add_space(SPACE_S); empty_state(ui, "Nothing selected"); });
                            return;
                        }
                        {
                            // Title and selection count on one line, painted (a
                            // Label would widen the panel to fit a long name).
                            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
                            let mut right = r.max.x - 8.0;
                            if let Some((summary, _)) = &selection {
                                let g = ui.painter().layout_no_wrap(summary.clone(), meta(), FAINT);
                                right -= g.size().x;
                                ui.painter().galley(pos2(right, r.center().y - g.size().y / 2.0), g, FAINT);
                                right -= 10.0;
                            }
                            let mut job = egui::text::LayoutJob::single_section(col.inspector_title.clone(), egui::TextFormat {
                                font_id: title(), color: INK, ..Default::default() });
                            job.wrap.max_width = (right - r.min.x - 8.0).max(40.0);
                            job.wrap.max_rows = 1;
                            job.wrap.break_anywhere = true;
                            let g = ui.painter().layout_job(job);
                            ui.painter().galley(pos2(r.min.x + 8.0, r.center().y - g.size().y / 2.0), g, INK);
                        }
                        // The selection's actions: the selection bar's job, in its one home.
                        if let Some((_, acts)) = &selection {
                            ui.add_space(SPACE_S);
                            egui::Frame::NONE.inner_margin(Margin { left: 8, ..Default::default() }).show(ui, |ui| ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = vec2(SPACE_S, SPACE_S);
                                for (id, label, _key, enabled, more) in acts {
                                    if !*more && *enabled && chip_button(ui, label) { cmds.push(format!("cmd:{id}")); }
                                }
                                if acts.iter().any(|a| a.4) {
                                    let label = if sel_more { "More ▴" } else { "More ▾" };
                                    let before = ui.cursor().min;
                                    if chip_button(ui, label) {
                                        more_at = Some(Rect::from_min_size(before, vec2(70.0, 24.0)));
                                    }
                                }
                            }));
                        }
                        ui.add_space(SPACE_XS);
                        let native_export = self.panels.has_records() && col.mode.is_empty() && col.colset != 2;
                        if native_export {
                            ui.collapsing("Export details", |ui| self.panels.export(ui, &mut cmds));
                        }
                        let sections = col.inspector.clone();
                        for (title, rows) in &sections {
                            if title == "Drawing" {
                                if let Some(n) = rows.first().and_then(|(_, key)| col.nodes.iter().find(|n| &n.key == key)) {
                                    drawing::inspector(ui, n, rows, &mut actions, &mut col.color_pick);
                                }
                                continue;
                            }
                            if title == "Pre-run checks" {
                                ui.add_space(SPACE_S);
                                for (label, count) in rows {
                                    if amber_button(ui, &format!("⚠ {count} {label} · Show")) {
                                        cmds.push("deliver:problems".into());
                                    }
                                }
                                continue;
                            }
                            if title == "Actions" {
                                // What to do next, as commands: (id, label).
                                ui.add_space(SPACE_S);
                                egui::Frame::NONE.inner_margin(Margin { left: 8, ..Default::default() }).show(ui, |ui| ui.horizontal_wrapped(|ui| {
                                                                        ui.spacing_mut().item_spacing = vec2(SPACE_S, SPACE_S);
                                    for (id, label) in rows {
                                        let at = ui.cursor().min;
                                        if chip_button(ui, label) {
                                            if let Some(key) = id.strip_prefix("kindmenu:") {
                                                col.kind_menu = Some((key.to_owned(), pos2(at.x, at.y + 28.0)));
                                            } else {
                                                actions.push(format!("insp:{id}"));
                                            }
                                        }
                                    }
                                }));
                                continue;
                            }
                            if title == "Export details" && native_export { continue; }
                            if title.starts_with("Export") {
                                ui.add_space(SPACE_XS);
                                kit::disclosure(ui, title, title, false,
                                    |ui| inspector_rows(ui, rows, &mut col.edits, &mut actions));
                            } else {
                                ui.add_space(SPACE_S);
                                caption(ui, title);
                                inspector_rows(ui, rows, &mut col.edits, &mut actions);
                            }
                        }
                    });
                });
            });
        // The row menu, near the pointer: node verbs or item verbs.
        if let Some((key, row, at)) = col.row_menu.clone() {
            let mut close = false;
            let node_menu: &[(&str, &str)] = &[("active", "Draw into this node"), ("filehere", "File selection here"),
                ("node", "Select its items"), ("isolate", "Isolate"), ("hidemembers", "Hide its items"),
                ("-", ""), ("rename", "Rename"), ("newchild", "New child node"), ("kind", "Kind  ›"),
                ("-", ""), ("delete", "Delete node")];
            let item_menu: &[(&str, &str)] = &[("item", "Select"), ("open", "Open for editing"), ("rename", "Rename"),
                ("-", ""), ("hideitem", "Hide"), ("lockitem", "Lock / unlock"), ("instances", "Select instances"),
                ("unique", "Make unique"), ("zoom", "Zoom to"), ("-", ""), ("fileto", "File to…")];
            let comp_menu: &[(&str, &str)] = &[("cplace", "Place a copy"), ("cselect", "Select instances"),
                ("creplace", "Replace selection with this"), ("-", ""), ("ckeep", "Keep in file"), ("crename", "Rename…")];
            // A linked file's rows are read-only here: no filing, kind or rename.
            let link_menu: &[(&str, &str)] = &[("node", "Select"), ("contents", "Select contents"),
                ("isolate", "Isolate"), ("zoom", "Zoom to"), ("-", ""), ("eye", "Show / hide"), ("lock", "Lock / unlock"),
                ("-", ""), ("linkopen", "Open source"), ("linkreload", "Reload")];
            let menu = if row == 0 && key.starts_with("n:link") { link_menu } else if row == 0 { node_menu }
                else if row == 5 { comp_menu } else { item_menu };
            let resp = egui::Area::new(egui::Id::new("row_menu"))
                .fixed_pos(at + vec2(4.0, 4.0))
                .order(egui::Order::Tooltip)
                .show(ctx, |ui| {
                    glass(Surface::Raised).inner_margin(Margin::same(6)).show(ui, |ui| {
                        ui.set_width(200.0);
                        ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                        for (id, label) in menu {
                            if *id == "-" {
                                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 7.0), Sense::hover());
                                ui.painter().hline(r.x_range().shrink(6.0), r.center().y, Stroke::new(1.0_f32, EDGE));
                                continue;
                            }
                            let item = (id.to_string(), label.to_string(), String::new(), String::new(), -1i8, true);
                            let (r, rect) = menu_row_resp(ui, &item);
                            if r.clicked() {
                                match *id {
                                    "rename" => {
                                        let name = col.nodes.iter().find(|n| n.key == key).map(|n| n.name.clone()).unwrap_or_default();
                                        col.renaming = Some((key.clone(), name));
                                        col.focus_rename = true;
                                    }
                                    "kind" => { col.kind_menu = Some((key.clone(), pos2(rect.max.x + 4.0, rect.min.y))); }
                                    _ => actions.push(format!("{id}\u{1f}{key}")),
                                }
                                close = true;
                            }
                        }
                    });
                });
            let _ = resp;
            if close { col.row_menu = None; }
        }
        if let Some((key, at)) = col.kind_menu.clone() {
            let mut close = false;
            egui::Area::new(egui::Id::new("kind_menu"))
                .fixed_pos(at)
                .order(egui::Order::Tooltip)
                .show(ctx, |ui| {
                    glass(Surface::Raised).inner_margin(Margin::same(6)).show(ui, |ui| {
                        ui.set_width(190.0);
                        ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                        for k in &col.kinds {
                            if menu_row(ui, &(k.clone(), k.clone(), String::new(), String::new(), -1i8, true)) {
                                actions.push(format!("kind\u{1f}{key}\u{1f}{k}"));
                                close = true;
                            }
                        }
                    });
                });
            if close { col.kind_menu = None; }
        }
        if let Some((key, mut c, at)) = col.color_pick.clone() {
            let mut done = false;
            egui::Area::new(egui::Id::new("color_pick"))
                .fixed_pos(at)
                .order(egui::Order::Tooltip)
                .show(ctx, |ui| {
                    glass(Surface::Raised).inner_margin(Margin::same(10)).show(ui, |ui| {
                        egui::widgets::color_picker::color_picker_color32(ui, &mut c, egui::widgets::color_picker::Alpha::Opaque);
                        ui.add_space(SPACE_S);
                        ui.horizontal(|ui| {
                            if chip_button(ui, "Apply") {
                                actions.push(format!("color\u{1f}{key}\u{1f}#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b()));
                                done = true;
                            }
                            if chip_button(ui, "Automatic") {
                                actions.push(format!("color\u{1f}{key}\u{1f}"));
                                done = true;
                            }
                        });
                    });
                });
            col.color_pick = if done { None } else { Some((key, c, at)) };
        }
        if let Some(at) = col.head_menu {
            let mut close = false;
            egui::Area::new(egui::Id::new("head_menu"))
                .pivot(egui::Align2::RIGHT_TOP)
                .fixed_pos(at)
                .order(egui::Order::Tooltip)
                .show(ctx, |ui| {
                    glass(Surface::Raised).inner_margin(Margin::same(6)).show(ui, |ui| {
                        ui.set_width(220.0);
                        ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                        if col.mode.is_empty() && col.colset != 2 {
                            let options = tree_order::Sort::OPTIONS;
                            let old = options.iter().position(|(sort, _)| *sort == col.sort).unwrap_or(0);
                            let mut index = old;
                            ui.horizontal(|ui| {
                                kit::caption_plain(ui, "Sort by");
                                kit::choice(ui, "project_sort", &mut index, &options.map(|(_, label)| label));
                            });
                            if index != old {
                                col.sort = options[index].0;
                                col.ordered = col.sort.order(&col.nodes);
                                actions.push(format!("sort\u{1f}{index}"));
                                close = true;
                            }
                        }
                        for (id, label) in [("new", "New node"), ("propose", "Propose structure from tags…"),
                                            ("expandall", "Expand all"), ("collapseall", "Collapse all"), ("showall", "Show all"),
                                            ("measure", "Measure quantities"), ("exportcsv", "Export quantities CSV…")] {
                            if menu_row(ui, &(id.to_owned(), label.to_owned(), String::new(), String::new(), -1i8, true)) {
                                actions.push(format!("{id}\u{1f}"));
                                close = true;
                            }
                        }
                    });
                });
            if close { col.head_menu = None; }
        }
        if let Some((title, body_text, buttons)) = col.prompt.clone() {
            let mut answered = false;
            egui::Area::new(egui::Id::new("prompt"))
                .pivot(egui::Align2::CENTER_CENTER)
                .fixed_pos(pos2(screen.max.x - GUT - W / 2.0, top + 160.0))
                .order(egui::Order::Tooltip)
                .show(ctx, |ui| {
                    glass(Surface::Raised).inner_margin(Margin::same(14)).show(ui, |ui| {
                        ui.set_width(W - 40.0);
                        ui.label(egui::RichText::new(&title).font(title_font()).color(INK));
                        ui.add_space(SPACE_S);
                        // A body written "\u{1d}edit\u{1d}<id>\u{1d}<text>" is a field to type in.
                        let field = body_text.strip_prefix("\u{1d}edit\u{1d}").map(|r| {
                            let (id, text) = r.split_once('\u{1d}').unwrap_or((r, ""));
                            (id.to_owned(), text.to_owned())
                        });
                        let mut enter = false;
                        if let Some((id, text)) = &field {
                            let buf = col.edits.entry(format!("prompt:{id}")).or_insert_with(|| text.clone());
                            let r = ui.add(kit::field(buf).font(body()).margin(Margin::symmetric(6, 4))
                                .desired_width((W - 40.0 - 40.0).max(60.0)).clip_text(true));
                            r.request_focus();
                            enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        } else {
                            ui.label(egui::RichText::new(&body_text).font(body()).color(DIM));
                        }
                        ui.add_space(SPACE_M);
                        let typed = field.as_ref().map(|(id, _)| col.edits.get(&format!("prompt:{id}")).cloned().unwrap_or_default());
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = vec2(SPACE_S, 0.0);
                            for (i, (id, label)) in buttons.iter().enumerate() {
                                if chip_button(ui, label) || (enter && i == 0) {
                                    match &typed {
                                        Some(t) => actions.push(format!("prompt\u{1f}{id}\u{1f}{t}")),
                                        None => actions.push(format!("prompt\u{1f}{id}")),
                                    }
                                    answered = true;
                                }
                            }
                        });
                    });
                });
            if answered {
                col.prompt = None;
                col.edits.retain(|k, _| !k.starts_with("prompt:"));
            }
        }
        // The grip on the dock's left edge: drag to resize, double-click resets.
        let mut shape = "";
        if let Some(r) = ctx.memory(|m| m.area_rect(egui::Id::new("column"))) {
            let dock_w = &mut self.dock_w;
            egui::Area::new(egui::Id::new("dock_grip"))
                .fixed_pos(pos2(r.min.x - 6.0, r.min.y + 14.0))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let (g, resp) = ui.allocate_exact_size(vec2(9.0, (r.height() - 28.0).max(20.0)), Sense::click_and_drag());
                    if resp.hovered() || resp.dragged() {
                        shape = "resize_h";
                        ui.painter().vline(g.center().x + 1.0, g.y_range().shrink(g.height() * 0.3), Stroke::new(2.0_f32, ACCENT_LINE));
                    }
                    let max_w = (screen.width() * 0.45).clamp(DOCK_MIN_W, DOCK_MAX_W);
                    if resp.dragged() {
                        *dock_w = (*dock_w - resp.drag_delta().x).clamp(DOCK_MIN_W, max_w);
                    }
                    if resp.double_clicked() { *dock_w = DOCK_W; }
                    if resp.drag_stopped() || resp.double_clicked() {
                        cmds.push(format!("dockw:{}", dock_w.round()));
                    }
                });
        }
        self.cursor_shape = shape;
        for a in actions {
            self.actions.push(format!("tree:{a}"));
        }
        self.actions.extend(cmds);
        if let Some(r) = more_at {
            self.sel_more = !self.sel_more;
            self.sel_more_anchor = r;
        }
    }

    /// The sheet: hung under the top bar over the free model area, a scrim
    /// over everything else taking every click (Python makes it modal).
    /// Discard sits apart on the left; the default is the accent button.
    fn sheet_popup(&mut self, ctx: &egui::Context, screen: Rect) {
        let Some(sheet) = self.sheet.clone() else { return };
        let mut clicked: Option<String> = None;
        let left = if self.review { GUT } else { GUT + 72.0 + GUT };
        let right = if self.review { screen.max.x - GUT } else { self.free_right(ctx, screen) - GUT };
        let w = 400.0_f32.min((right - left).max(280.0)).min(screen.width() - 2.0 * GUT);
        let x = ((left + right) / 2.0 - w / 2.0).clamp(GUT, (screen.max.x - GUT - w).max(GUT));
        egui::Area::new(egui::Id::new("sheet"))
            .fade_in(false)
            .fixed_pos(Pos2::ZERO)
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                // The scrim: every click under the sheet is the sheet's.
                let (all, _) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
                ui.painter().rect_filled(Rect::from_min_max(pos2(all.min.x, TOP_H), all.max), CornerRadius::ZERO,
                                         SCRIM);
                let at = Rect::from_min_size(pos2(x, TOP_H), vec2(w, screen.height() - TOP_H - 56.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(at), |ui| {
                    glass(Surface::Raised).inner_margin(Margin::same(14)).show(ui, |ui| {
                        ui.set_width(w - 28.0);
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = vec2(SPACE_S, 0.0);
                            let (glyph, ink) = match sheet.tone.as_str() {
                                "error" => ("!", ERROR),
                                "warning" => ("!", WARN),
                                "info" => ("i", ACCENT),
                                _ => ("?", ACCENT),
                            };
                            let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                            ui.painter().circle_stroke(r.center(), 8.5, Stroke::new(1.4_f32, ink));
                            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, glyph, caption_font(), ink);
                            ui.add(egui::Label::new(egui::RichText::new(&sheet.title).font(title_font()).color(INK)).wrap());
                        });
                        if !sheet.body.is_empty() {
                            ui.add_space(SPACE_S);
                            ui.horizontal(|ui| {
                                ui.add_space(28.0);
                                ui.add(egui::Label::new(egui::RichText::new(&sheet.body).font(body()).color(DIM)).wrap());
                            });
                        }
                        if sheet.buttons.iter().any(|b| b.3 >= 4) {
                            egui::ScrollArea::vertical().max_height(screen.height() * 0.45).show(ui, |ui| {
                                for (id, label, _, role) in sheet.buttons.iter().filter(|b| b.3 >= 4) {
                                    let mut checked = *role == 5;
                                    if ui.push_id(id, |ui| ui.checkbox(&mut checked, label)).inner.changed() {
                                        clicked = Some(id.clone());
                                    }
                                }
                            });
                        }
                        ui.add_space(SPACE_L);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = vec2(SPACE_S, 0.0);
                            for (id, label, hint, role) in sheet.buttons.iter().filter(|b| b.3 == 2) {
                                if sheet_button(ui, label, hint, *role) { clicked = Some(id.clone()); }
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                for (id, label, hint, role) in sheet.buttons.iter().rev().filter(|b| b.3 != 2 && b.3 < 4) {
                                    if sheet_button(ui, label, hint, *role) { clicked = Some(id.clone()); }
                                }
                            });
                        });
                    });
                });
            });
        if let Some(id) = clicked { self.actions.push(format!("sheet:{id}")); }
    }

    /// The value field on the geometry it measures (Maxence 2026-11-22):
    /// Python anchors it (middle of the rubber band, beside a pulled face);
    /// it stays inside the free area. Typing lights it up; Enter applies.
    fn cursor_value(&self, ctx: &egui::Context, screen: Rect) {
        let Some((caption, value, typing, at)) = &self.cursor else { return };
        if !screen.contains(*at) || (value.is_empty() && !*typing) {
            return;
        }
        let left = if self.review { GUT } else { GUT + 72.0 + GUT };
        let right = if self.review { screen.max.x - GUT } else { self.free_right(ctx, screen) - GUT };
        let bottom = if self.review { screen.max.y - GUT } else { screen.max.y - BOT_H - GUT };
        let size = vec2(170.0, 30.0);
        let pos = pos2((at.x - size.x / 2.0).clamp(left, (right - size.x).max(left)),
                       (at.y - size.y / 2.0).clamp(TOP_H + GUT, (bottom - size.y).max(TOP_H + GUT)));
        egui::Area::new(egui::Id::new("cursor_value"))
            .fixed_pos(pos)
            .order(egui::Order::Tooltip)
            .interactable(false)
            .show(ctx, |ui| {
                let edge = if *typing { ACCENT } else { EDGE };
                glass(Surface::Raised).stroke(Stroke::new(1.0_f32, edge)).corner_radius(CornerRadius::same(R_CONTROL))
                    .inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(SPACE_S, 0.0);
                        if !caption.is_empty() {
                            ui.label(egui::RichText::new(caption).font(meta()).color(FAINT));
                        }
                        let shown = if *typing { format!("{value}▏") } else { value.clone() };
                        ui.label(egui::RichText::new(shown).font(title_font())
                            .color(if *typing { BRIGHT } else { INK }));
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
        let zoom = if self.gallery.is_some() { gallery::Gallery::fit(&mut raw, screen) } else { 1.0 };
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(scale * zoom);
        let ctx = self.ctx.clone();
        let out = ctx.run(raw, |ctx| {
            if let Some(page) = &mut self.gallery { page.show(ctx); return; }
            if self.sheets.shown {
                // The Sheets workspace: the window's top bar, the sheet's chrome under it.
                self.top_bar(ctx, screen);
                self.sheets_frame(ctx, screen);
                self.bar_menu(ctx);
                self.view_popup(ctx);
                self.form_popup(ctx);
                let acts = self.strip.popups(ctx, &self.icons);
                self.actions.extend(acts);
            } else {
            self.panels.popup_layers.clear();
            self.top_bar(ctx, screen);
            if !self.review {
                self.palette(ctx, screen);
                self.right_column(ctx, screen);
                self.plugins.utility(ctx, screen, self.dock_w, &mut self.actions);
                self.bottom_bar(ctx, screen);
                self.paint_strip(ctx, screen);
                if self.panels.data["terrain"].is_object() {
                    let bounds = Rect::from_min_max(pos2(12.,0.),pos2(self.free_right(ctx,screen)-8.,screen.bottom()-BOT_H-4.));
                    self.panels.terrain.draw(ctx,bounds,&self.panels.data["terrain"],&self.panels.data["projected"],&mut self.actions);
                }
                self.panels.frame_guide(ctx, screen);
            }
            self.cursor_value(ctx, screen);
            self.bar_menu(ctx);
            self.view_popup(ctx);
            self.form_popup(ctx);
            let acts = self.strip.popups(ctx, &self.icons);
            self.actions.extend(acts);
            self.context_menu(ctx, screen);
            }
            self.large_sheet(ctx, screen);
            self.command_palette(ctx, screen);
            if !self.review && !self.sheets.shown && !self.lsheet.open { self.selection_more(ctx); }
            self.dialog_popup(ctx, screen);
            self.jobs_chip(ctx, screen);
            self.sheet_popup(ctx, screen);
            if let Some(picker) = &mut self.font_picker {
                if let Some(apply) = picker.popup(ctx) {
                    let payload = if apply { serde_json::to_string(&picker.choice).unwrap() } else { String::new() };
                    self.actions.push(format!("font:{payload}"));
                    self.font_picker = None;
                    ctx.memory_mut(|m| m.stop_text_input());
                }
            }
        });
        let work = !self.review;
        // The model's own areas are not drawn in the Sheets workspace: their
        // last rects must not take the pointer there.
        let model = !self.sheets.shown;
        let shown = [("top", true), ("palette", work && model && !self.groups.is_empty()),
                     ("view_menu", self.view_open), ("menu", self.menu.is_some()), ("form", self.form.is_some()),
                     ("plugin_utility", work && model && self.plugins.utility_open()), ("column", work && model && self.column.is_some()), ("bottom", work),
                     ("sel_more", work && model && self.sel_more && self.selection.is_some()),
                     ("paint_strip", work && model && !self.palette_items.is_empty()),
                     ("commands", self.cmd_open),
                     ("sel_sub", work && model && self.sel_more && self.sel_sub.is_some()),
                     ("row_menu", model && self.column.as_ref().is_some_and(|c| c.row_menu.is_some())),
                     ("kind_menu", model && self.column.as_ref().is_some_and(|c| c.kind_menu.is_some())),
                     ("head_menu", model && self.column.as_ref().is_some_and(|c| c.head_menu.is_some())),
                     ("prompt", model && self.column.as_ref().is_some_and(|c| c.prompt.is_some())),
                     ("color_pick", model && self.column.as_ref().is_some_and(|c| c.color_pick.is_some())),
                     ("terrain_profile", work && model && self.panels.data["terrain"].is_object()),
                     ("jobs", !self.jobs.is_empty() || !self.history.is_empty()),
                     ("dialog", self.dialog.is_some()),
                     ("tab_menu", self.strip.menu_open()),
                     ("tab_overflow", self.strip.overflow_open()),
                     ("ctx_menu", model && self.ctx_menu.is_some()),
                     ("sheet", self.sheet.is_some()),
                     ("dock_grip", work && model && self.column.is_some()),
                     ("insp_color", work && model && self.column.as_ref().is_some_and(|c| c.edits.contains_key("\u{1d}pick")))];
        let sheet_areas = self.sheets_areas();
        let large = self.lsheet_areas();
        self.hit = shown.iter().chain(sheet_areas.iter()).chain(large.iter())
            .filter(|(_, on)| *on)
            .filter_map(|(n, _)| ctx.memory(|m| m.area_rect(egui::Id::new(*n))))
            .collect();
        if self.gallery.is_some() { self.hit = vec![screen]; }
        self.repaint_ms = out
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|v| v.repaint_delay)
            .filter(|d| d.as_secs() < 60)
            .map(|d| d.as_secs_f64() * 1000.);
        self.ime_rect = out.platform_output.ime.map(|ime| {
            let r = ime.cursor_rect;
            (r.min.x * zoom, r.min.y * zoom, (r.width() * zoom).max(1.0), (r.height() * zoom).max(1.0))
        });
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
        // The caller submits this encoder after paint returns. Textures
        // released during this pass can still occur in its draw commands.
        for id in std::mem::replace(&mut self.free_after_frame, out.textures_delta.free) {
            self.painter.free_texture(&id);
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
        None => ui.painter().layout_no_wrap(label.clone(), body(), INK).size().x + 24.0,
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

/// A project tab: capsule, name (and • while unsaved), × on the right when
/// current or hovered. Some(true): close clicked; Some(false): tab clicked.
fn doc_tab(ui: &mut egui::Ui, name: &str, active: bool, dirty: bool) -> Option<bool> {
    let label = if dirty { format!("{name} •") } else { name.to_owned() };
    let g = ui.painter().layout_no_wrap(label, body(), INK);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 12.0 + 26.0, 28.0), Sense::click());
    let st = state_of(&resp, active, true);
    paint_state(ui, rect, st, R_CONTROL);
    let ink = if active { BRIGHT } else if resp.hovered() { INK } else { INK_2 };
    ui.painter().galley(pos2(rect.min.x + 12.0, rect.center().y - g.size().y / 2.0), g, ink);
    let x = Rect::from_center_size(pos2(rect.max.x - 14.0, rect.center().y), vec2(18.0, 18.0));
    let over_x = resp.hover_pos().is_some_and(|p| x.contains(p));
    if active || resp.hovered() {
        if over_x {
            ui.painter().circle_filled(x.center(), 9.0, PRESS);
        }
        let c = x.center();
        let k = 3.5;
        let pen = Stroke::new(1.3_f32, if over_x { INK } else { DIM });
        ui.painter().line_segment([c + vec2(-k, -k), c + vec2(k, k)], pen);
        ui.painter().line_segment([c + vec2(-k, k), c + vec2(k, -k)], pen);
    }
    if resp.clicked() {
        let on_x = resp.interact_pointer_pos().is_some_and(|p| x.contains(p));
        return Some(on_x && (active || resp.hovered()));
    }
    None
}

/// Inspector rows: label left, value right; a value written
/// "\u{1d}edit\u{1d}<id>\u{1d}<text>" is an editable field (Enter applies).
fn inspector_rows(ui: &mut egui::Ui, rows: &[(String, String)], edits: &mut std::collections::HashMap<String, String>,
                  actions: &mut Vec<String>) {
    for (label, value) in rows {
        // "\u{1d}choice\u{1d}<id>\u{1d}A|B\u{1d}<at>": a switch (the material's base).
        if let Some(rest) = value.strip_prefix("\u{1d}choice\u{1d}") {
            let parts: Vec<&str> = rest.split('\u{1d}').collect();
            let id = parts.first().copied().unwrap_or_default();
            let labels: Vec<&str> = parts.get(1).copied().unwrap_or_default().split('|').collect();
            let at = parts.get(2).and_then(|a| a.parse::<usize>().ok()).unwrap_or(0);
            ui.horizontal(|ui| {
                ui.add_space(SPACE_S);
                let (lr, _) = ui.allocate_exact_size(vec2(88.0, CONTROL_H), Sense::hover());
                ui.painter().text(lr.left_center(), egui::Align2::LEFT_CENTER, label, meta(), DIM);
                let w = (ui.available_width() - 8.0).max(60.0);
                if let Some(i) = widgets::segmented(ui, &labels, at, w) {
                    if i != at { actions.push(format!("edit\u{1f}{id}\u{1f}{}", labels[i])); }
                }
            });
            continue;
        }
        // "\u{1d}color\u{1d}<id>\u{1d}#rrggbb": a swatch; a click opens the picker.
        if let Some(rest) = value.strip_prefix("\u{1d}color\u{1d}") {
            let (id, hex) = rest.split_once('\u{1d}').unwrap_or((rest, ""));
            let mut swatch_rect = Rect::NOTHING;
            ui.horizontal(|ui| {
                ui.add_space(SPACE_S);
                let (lr, _) = ui.allocate_exact_size(vec2(88.0, CONTROL_H), Sense::hover());
                ui.painter().text(lr.left_center(), egui::Align2::LEFT_CENTER, label, meta(), DIM);
                let (r, resp) = ui.allocate_exact_size(vec2((ui.available_width() - 8.0).max(60.0), CONTROL_H), Sense::click());
                paint_state(ui, r, state_of(&resp, false, true), R_ROW);
                let sw = Rect::from_min_size(r.min + vec2(3.0, 3.0), vec2(36.0, 18.0));
                ui.painter().rect_filled(sw, CornerRadius::same(4), parse_hex(hex).unwrap_or(SWATCH_FALLBACK));
                ui.painter().rect_stroke(sw, CornerRadius::same(4), Stroke::new(1.0_f32, EDGE), egui::StrokeKind::Inside);
                ui.painter().text(pos2(sw.max.x + 8.0, r.center().y), egui::Align2::LEFT_CENTER, hex.to_uppercase(), body(), INK);
                swatch_rect = r;
                if resp.on_hover_text("Pick a colour").clicked() {
                    edits.insert("\u{1d}pick".into(), id.to_owned());
                    edits.insert("\u{1d}pickc".into(), hex.to_owned());
                }
            });
            if edits.get("\u{1d}pick").map(String::as_str) == Some(id) {
                let mut c = edits.get("\u{1d}pickc").and_then(|h| parse_hex(h)).unwrap_or(SWATCH_FALLBACK);
                let mut done = false;
                egui::Area::new(egui::Id::new("insp_color"))
                    .pivot(egui::Align2::RIGHT_TOP)
                    .fixed_pos(pos2(swatch_rect.max.x, swatch_rect.max.y + 4.0))
                    .order(egui::Order::Tooltip)
                    .show(ui.ctx(), |ui| {
                        glass(Surface::Raised).inner_margin(Margin::same(10)).show(ui, |ui| {
                            egui::widgets::color_picker::color_picker_color32(ui, &mut c, egui::widgets::color_picker::Alpha::Opaque);
                            ui.add_space(SPACE_S);
                            ui.horizontal(|ui| {
                                if chip_button(ui, "Apply") {
                                    actions.push(format!("edit\u{1f}{id}\u{1f}#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b()));
                                    done = true;
                                }
                                if chip_button(ui, "Cancel") { done = true; }
                            });
                        });
                    });
                if done {
                    edits.remove("\u{1d}pick");
                    edits.remove("\u{1d}pickc");
                } else {
                    edits.insert("\u{1d}pickc".into(), format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b()));
                }
            }
            continue;
        }
        if let Some(rest) = value.strip_prefix("\u{1d}edit\u{1d}") {
            let (id, text) = rest.split_once('\u{1d}').unwrap_or((rest, ""));
            let buf = edits.entry(id.to_owned()).or_insert_with(|| text.to_owned());
            ui.horizontal(|ui| {
                ui.add_space(SPACE_S);
                let (lr, _) = ui.allocate_exact_size(vec2(88.0, CONTROL_H), Sense::hover());
                ui.painter().text(lr.left_center(), egui::Align2::LEFT_CENTER, label, meta(), DIM);
                // desired_width is the text's width: leave room for the field's
                // own margins, or the panel grows a little every frame.
                let w = (ui.available_width() - 8.0 - 12.0 - 4.0).max(40.0);
                let r = ui.add(kit::field(buf).font(body()).margin(Margin::symmetric(6, 3))
                    .desired_width(w).clip_text(true));
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && buf.as_str() != text {
                    actions.push(format!("edit\u{1f}{id}\u{1f}{buf}"));
                }
            });
            continue;
        }
        // Value right-aligned; the label takes what is left, cut short.
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), PROP_H), Sense::hover());
        let v = ui.painter().layout_no_wrap(value.clone(), body(), INK);
        let vw = v.size().x.min(r.width() * 0.6);
        ui.painter().galley(pos2(r.max.x - 8.0 - vw, r.center().y - v.size().y / 2.0), v, INK);
        let mut job = egui::text::LayoutJob::single_section(label.clone(), egui::TextFormat {
            font_id: meta(), color: DIM, ..Default::default() });
        job.wrap.max_width = (r.width() - vw - 26.0).max(20.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let l = ui.painter().layout_job(job);
        ui.painter().galley(pos2(r.min.x + 8.0, r.center().y - l.size().y / 2.0), l, DIM);
    }
}

/// A bar button that opens a menu: label and ▾; returns its rect when clicked.
fn menu_button(ui: &mut egui::Ui, label: &str, open: bool) -> Option<Rect> {
    let g = ui.painter().layout_no_wrap(label.to_owned(), body(), INK);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 36.0, 28.0), Sense::click());
    let st = state_of(&resp, false, true);
    paint_state(ui, rect, if open { State::Press } else { st }, R_CONTROL);
    ui.painter().galley(pos2(rect.min.x + 10.0, rect.center().y - g.size().y / 2.0), g, INK);
    ui.painter().text(pos2(rect.max.x - 12.0, rect.center().y), egui::Align2::CENTER_CENTER, "▾", meta(), FAINT);
    resp.clicked().then_some(rect)
}

/// A hairline between groups in a bar.
fn bar_divider(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(9.0, 28.0), Sense::hover());
    ui.painter().vline(r.center().x, r.y_range().shrink(7.0), Stroke::new(1.0_f32, EDGE));
}

/// A text command in a bar, 28 high, 8 pt padding.
fn text_button(ui: &mut egui::Ui, label: &str, key: &str, enabled: bool) -> bool {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), body(), INK);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 24.0, 28.0), sense);
    let st = state_of(&resp, false, enabled);
    paint_state(ui, rect, st, R_CONTROL);
    let ink = if enabled { INK } else { OFF };
    ui.painter().galley(pos2(rect.min.x + 12.0, rect.center().y - galley.size().y / 2.0), galley, ink);
    let resp = if key.is_empty() { resp } else { resp.on_hover_text(key) };
    resp.clicked()
}

/// A palette tool: 30 × 30 cell, glyph about 18 pt (the icon art carries
/// its own margin), idle INK_2, hover INK, current white on accent.
fn tool_button(ui: &mut egui::Ui, tool: &Tool, active: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), if tool.enabled { Sense::click() } else { Sense::hover() });
    let st = state_of(&resp, active, tool.enabled);
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
    let tip = if !tool.enabled { format!("{} — Open in classic app", tool.label) } else if tool.key.is_empty() { tool.label.clone() } else { format!("{}    {}", tool.label, tool.key) };
    resp.on_hover_text(tip).clicked()
}
