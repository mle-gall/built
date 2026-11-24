//! Viewport chrome drawn by egui in the frame that draws the model (the
//! UI spike, `ui/egui-spike`). Python forwards pointer events and registers
//! the tools; clicks come back as action strings. Nothing here owns model
//! state: the palette shows what Python says is active.

use std::sync::Arc;
use std::time::Instant;

use egui::{pos2, vec2, Color32, CornerRadius, Margin, Pos2, Rect, Sense, Stroke, Vec2};

/// Small widgets shared by the dock and the forms (segmented, tabs).
mod widgets;
/// The chrome's modal forms (rust/ingetrazo_render/src/forms_host.rs).
pub mod dialog;
pub mod operation_form;
/// Sheets, a workspace of the window (views/wgpu_sheets.py).
mod sheets;
mod activity;
mod plugins;
pub use activity::ActivityRow;
/// Project tabs in the top bar (views/wgpu_tabs.py).
mod tabs;
/// The viewport right-click menu (viewport_menu.rs, docs/context-menu.md).
mod context_menu;

mod lens;
use lens::Lens;
mod tree_order;
mod tree_filter;
mod tree_keys;
mod tree_rows;
mod drawing;
mod drawing_catalog;
mod drawing_hatches;
mod drawing_cuts;
mod export;
/// The large sheet over the dimmed model: Library, start page, Settings
/// (sheet_runtime.rs, docs/ui-patterns.md).
pub mod large_sheet;

pub mod tokens;
pub mod kit;
#[cfg(target_os = "macos")]
mod system_fonts;
#[cfg(any(not(target_os = "macos"), test))]
mod host_fonts;
#[cfg(not(target_os = "macos"))]
use host_fonts as system_fonts;
mod gallery;
mod scene_folders;
pub mod command_palette;
/// The tool rail's deterministic layout (two columns, three, then scroll).
mod rail;
/// The value field on the geometry and its readback after Enter (B1).
mod value_field;
mod site_map;
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

#[cfg(test)]
fn test_context()->egui::Context{
    let ctx=egui::Context::default();
    let mut fonts=egui::FontDefinitions::empty();
    let data=include_bytes!("../../../resources/fonts/inter/Inter-Regular.ttf");
    fonts.font_data.insert("inter".into(),std::sync::Arc::new(egui::FontData::from_static(data)));
    for family in [egui::FontFamily::Proportional,egui::FontFamily::Monospace,egui::FontFamily::Name("medium".into()),egui::FontFamily::Name("semibold".into())]{
        fonts.families.insert(family,vec!["inter".into()]);
    }
    ctx.set_fonts(fonts);
    ctx
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
    model_textures: std::collections::HashMap<usize, egui::TextureId>,
    model_panes: Vec<(egui::TextureId, Rect, bool, String)>,
    pub(super) model_guides: Vec<[f32; 4]>,
    free_after_frame: Vec<egui::TextureId>,
    start: Instant,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
    groups: Vec<Vec<Tool>>,
    /// The model rail's outer width as last laid out (points).
    rail_w: f32,
    /// The window as last painted (points).
    screen: Rect,
    active: String,
    status: String,
    hint: String,
    /// The value field on the geometry and its readback (ui/value_field.rs).
    cursor: value_field::ValueField,
    /// Selection bar: summary and (id, label, shortcut) actions.
    selection: Option<(String, Vec<(String, String, String, bool, bool)>)>,
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
    scene_browser: scene_folders::Browser,
    /// Project tabs at the top left, after the window buttons' room:
    /// (left edge, [(name, active, unsaved)]).
    projects: Option<(f32, Vec<(String, bool, bool)>)>,
    review: bool,
    /// The Paint tool's strip: (id, name, colour hex or "", icon key or "", active).
    palette_items: Vec<(String, String, String, String, bool)>,
    /// The command palette (⌘K / F3): open, query, results (label, path, shortcut, enabled), picked row.
    cmd_open: bool,
    cmd_query: String,
    cmd_scope: String,
    cmd_anchor: Option<Pos2>,
    cmd_extra: Vec<(String, bool, bool)>,
    cmd_results: Vec<(String, String, String, bool)>,
    cmd_at: usize,
    cmd_focus: bool,
    cmd_scroll: bool,
    /// The open bar menu: kind ("docs", "work", "scenes") and its button.
    menu: Option<(String, Rect)>,
    view_anchor: Rect,
    top_groups: Vec<Rect>,
    pub cluster_buttons: Vec<(String, (f32, f32))>,
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
    /// The page of the Sheets workspace (ui/page.rs).
    pub page: crate::sheet_page::Page,
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
    dim: bool,
    local_placement: bool,
    control_tips: [String; 2],
    /// The selection, marked once (tree A): 1 this row holds it (lit), 2 a
    /// row above the one holding it (a quiet dot), 0 neither.
    mark: u8,
    icon: String,
    /// What the kind means, on hover (Python sends "kind\u{1e}tip").
    tip: String,
    /// The active column set's cells (Drawing: colour, line type, weight,
    /// print; Export: class, predefined type).
    cells: Vec<String>,
    hatch: Option<Box<kit::PaperMarks>>,
}

pub struct Column {
    nodes: Vec<Node>,
    path: Vec<(String, String)>,
    host: (String, Vec<crate::host_context::Tag>),
    styles: drawing::StyleEditor,
    overview: lens::Overview,
    proposal_available: bool,
    proposal_dismissed: bool,
    reveal: Option<String>,
    sort: tree_order::Sort,
    ordered: Vec<usize>,
    unfiled: u32,
    browser_picks: usize,
    issue_at: Option<usize>,
    export_focus: (Option<String>, std::collections::HashSet<String>),
    filter: String,
    /// "File to…": the next row clicked (or Enter) receives the selection.
    filing: Option<String>,
    focus_filter: bool,
    group_rows: Vec<(String, String, String)>,
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
    lens: Lens,
    /// Legacy Library overlay state.
    mode: String,
    /// Issues shown (dots in the tree, counts in the header) or kept out of the way.
    issues: bool,
    /// The inspector's editable fields: id → text being typed.
    edits: std::collections::HashMap<String, String>,
    identity: Vec<(String, String, String, Option<bool>)>,
    inspector_title: String,
    inspector: Vec<(String, Vec<(String, String)>)>,
    highlighted: Option<String>,
    focused_row: Option<String>,
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
            model_textures: Default::default(),
            model_panes: Vec::new(),
            model_guides: Vec::new(),
            free_after_frame: Vec::new(),
            glass,
            start: Instant::now(),
            events: Vec::new(),
            modifiers: Default::default(),
            groups: Vec::new(),
            rail_w: rail::width(2),
            screen: Rect::NOTHING,
            active: String::new(),
            status: String::new(),
            hint: String::new(),
            cursor: Default::default(),
            selection: None,
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
            scene_browser: Default::default(),
            projects: None,
            review: false,
            palette_items: Vec::new(),
            cmd_open: false,
            cmd_query: String::new(),
            cmd_scope: String::new(),
            cmd_anchor: None,
            cmd_extra: Vec::new(),
            cmd_results: Vec::new(),
            cmd_at: 0,
            cmd_focus: false,
            cmd_scroll: false,
            menu: None,
            view_anchor: Rect::NOTHING,
            top_groups: Vec::new(),
            cluster_buttons: Vec::new(),
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
            page: Default::default(),
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
    pub fn set_tree_group_rows(&mut self, key: String, rows: Vec<(String, String, String)>) {
        if let Some(col) = self.column.as_mut() {
            if col.row_menu.as_ref().is_some_and(|(k, _, _)| k == &key) { col.group_rows = rows; }
        }
    }

    /// rows: ([key, name, kind, badge, icon], [depth, count, problems, active,
    /// row, eye, lock, expandable, expanded, selected]).
    pub fn set_tree(&mut self, rows: Vec<(Vec<String>, Vec<i64>)>, unfiled: u32) {
        let col = self.column.get_or_insert_with(|| Column {
            nodes: Vec::new(), path: Vec::new(), host: Default::default(), overview: Default::default(), styles: Default::default(), reveal: None, proposal_available: false, proposal_dismissed: false, sort: Default::default(), ordered: Vec::new(), unfiled: 0, browser_picks: 0, issue_at: None, export_focus: Default::default(), filter: String::new(),
            filing: None, focus_filter: false, group_rows: Vec::new(), row_menu: None, renaming: None, focus_rename: false, head_menu: None,
            kind_menu: None, kinds: Vec::new(), prompt: None, lens: Lens::Filing, edits: Default::default(), mode: String::new(), issues: false,
            identity: Vec::new(),
            inspector_title: String::new(),
            inspector: Vec::new(), highlighted: None, focused_row: None,
        });
        col.nodes = tree_rows::reconcile(std::mem::take(&mut col.nodes), rows);
        if col.highlighted.as_ref().is_some_and(|k| !k.starts_with("n:") && !k.starts_with("preview-tag:") &&
            !col.nodes.iter().any(|n| &n.key == k && n.selected > 0)) {
            col.highlighted = None;
        }
        if col.lens == Lens::Drawing { col.styles.refresh(&col.nodes); }
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
    pub fn set_browser_picks(&mut self, count: usize) {
        if let Some(col) = self.column.as_mut() { col.browser_picks = count; }
    }
    pub fn rename_browser_row(&mut self, id: &str) -> bool {
        let Some(col) = self.column.as_mut() else { return false };
        let Some(node) = col.nodes.iter().find(|n| n.local_placement && n.key.split_once(':').is_some_and(|(_, k)| k == id)) else { return false };
        col.renaming = Some((node.key.clone(), node.name.clone()));
        col.reveal = Some(node.key.clone());
        col.focus_rename = true;
        true
    }
    pub fn reveal_row(&mut self, key: String) {
        self.plugins.showing = false;
        if let Some(col) = self.column.as_mut() {
            col.filter.clear();
            col.highlighted = None;
            col.filing = None;
            col.reveal = Some(key);
        }
    }
    pub fn set_catalog_info(&mut self, filename:String, tree:&pyo3::Bound<'_,pyo3::PyAny>, counts:&pyo3::Bound<'_,pyo3::PyAny>)->pyo3::PyResult<()> {
        if let Some(col)=self.column.as_mut(){col.overview=lens::Overview::read(filename,tree,counts)?;}
        Ok(())
    }
    pub fn style_catalog(&mut self, owner: usize, lines: Vec<String>, patterns: String, scene: Option<&pyo3::Bound<'_,pyo3::PyAny>>) -> pyo3::PyResult<()> {
        if let Some(col)=self.column.as_mut() {
            if col.styles.owner != owner { col.styles = Default::default(); col.styles.owner=owner; col.renaming=None; col.proposal_dismissed=false; col.highlighted=None; col.focused_row=None; col.issue_at=None; }
            col.styles.lines=lines;
            col.styles.set_patterns(patterns);
            if let Some(scene)=scene { col.styles.sync_catalog(scene)?; col.styles.hatches.sync(scene)?; }
        }
        Ok(())
    }
    pub fn open_line_type(&mut self,name:&str,tags:Vec<String>)->bool {
        let Some(col)=self.column.as_mut() else{return false;};
        col.lens=Lens::Drawing;
        self.plugins.showing=false;
        col.mode.clear();
        col.styles.catalog.open_line(name,tags)
    }
    pub fn hatch_preference(&mut self,py:pyo3::Python<'_>,verb:&str,value:&str)->pyo3::PyResult<()> {
        if let Some(col)=self.column.as_mut(){col.styles.hatches.preference(py,verb,value)?;}
        Ok(())
    }
    pub fn tick_tag_counts(&mut self, history: &pyo3::Bound<'_, pyo3::PyAny>) -> pyo3::PyResult<bool> {
        if let Some(col) = self.column.as_mut() {
            if col.lens == Lens::Drawing { return col.styles.tick_counts(history); }
        }
        Ok(false)
    }

    pub fn drawing_edit_applied(&mut self,verb:&str,key:&str){
        if let Some(col) = self.column.as_mut() {
            if verb == "newtag" {
                let row = format!("preview-tag:{key}");
                col.styles.clear_focus();
                col.focused_row = None;
                col.highlighted = None;
                col.filter.clear();
                col.reveal = Some(row.clone());
                col.renaming = Some((row, key.into()));
                col.focus_rename = true;
            } else {
                col.styles.applied(verb, key);
            }
        }
    }
    pub fn set_export_focus(&mut self, node: Option<String>, placements: Vec<Vec<String>>) {
        if let Some(col) = self.column.as_mut() {
            col.export_focus = (node, placements.into_iter().map(|path| serde_json::json!(path).to_string()).collect());
        }
    }
    pub fn step_issue(&mut self, step: i32) {
        let Some(col) = self.column.as_mut() else { return };
        let checks = &self.panels.data["readiness"]["checks"]["*"];
        let Some(rules) = checks.as_object() else { return };
        let mut paths: Vec<_> = rules.values().filter_map(|r| r[1].as_array()).flatten()
            .filter(|el| el["path"].is_array()).map(|el| el["path"].to_string()).collect();
        paths.sort();
        paths.dedup();
        if paths.is_empty() { return; }
        let at = col.issue_at.map_or(if step > 0 { 0 } else {paths.len()-1}, |i|
            (i as i64 + i64::from(step)).rem_euclid(paths.len() as i64) as usize);
        col.issue_at = Some(at);
        self.actions.push(format!("tree:item\u{1f}exportitem:{}",paths[at]));
    }
    pub fn set_host_context(&mut self, name: String, tags: Vec<crate::host_context::Tag>) {
        if let Some(col) = self.column.as_mut() { col.host = (name, tags); }
    }
    pub fn set_edit_path(&mut self, path: Vec<(String, String)>) {
        if let Some(col) = self.column.as_mut() { col.path = path; }
    }
    pub fn set_proposal_available(&mut self, available: bool) {
        if let Some(col) = self.column.as_mut() { col.proposal_available = available; }
    }
    pub fn set_tree_sort(&mut self, index: usize) {
        if let Some(col) = self.column.as_mut() {
            let sort = tree_order::Sort::OPTIONS.get(index).map(|&(sort, _)| sort).unwrap_or_default();
            if col.sort != sort {
                col.sort = sort;
                col.ordered = sort.order(&col.nodes);
            }
        }
    }
    pub fn set_lens(&mut self, lens: u8) {
        if let Some(col) = self.column.as_mut() { col.lens = Lens::from_index(lens); }
    }
    pub fn set_kinds(&mut self, kinds: Vec<String>) {
        if let Some(col) = self.column.as_mut() { col.kinds = kinds; }
    }
    pub fn set_prompt(&mut self, title: String, body: String, buttons: Vec<(String, String)>) {
        if let Some(col) = self.column.as_mut() {
            col.prompt = (!title.is_empty()).then_some((title, body, buttons));
        }
    }
    pub fn set_identity(&mut self, rows: Vec<(String, String, String, Option<bool>)>) {
        if let Some(col) = self.column.as_mut() { col.identity = rows; }
    }
    pub fn show_inspector(&mut self) { self.set_review(false); }
    pub fn set_inspector(&mut self, title: String, sections: Vec<(String, Vec<(String, String)>)>) {
        if let Some(col) = self.column.as_mut() {
            if col.inspector_title != title || col.inspector != sections {
                col.edits.clear();
            }
            col.inspector_title = title;
            col.inspector = sections;
            col.identity.clear();
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
        self.start_commands(String::new(), String::new());
    }
    pub fn set_command_results(&mut self, results: Vec<(String, String, String, bool)>) {
        self.cmd_results = results;
        self.cmd_at = self.cmd_results.iter().position(|row| row.3).unwrap_or(0);
        self.cmd_scroll = true;
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
        self.menu = None;
        self.cmd_open = false;
        if let Some(col) = self.column.as_mut() {
            col.row_menu = None; col.kind_menu = None; col.head_menu = None;
        }
        self.ctx.memory_mut(|m| m.stop_text_input());
        self.sheet = Some(Sheet { title, body, tone, buttons });
    }
    pub fn extension_menu(&mut self, rows: Vec<crate::extension_menu::Row>) { self.plugins.commands = rows; }
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
    pub fn set_tabs(&mut self, tabs: Vec<Vec<Item>>) {
        if let Some(scenes)=tabs.get(1){self.scene_browser.retain(scenes);}
        self.tabs=tabs;
    }
    pub fn scene_photo_mode(&mut self,py:pyo3::Python<'_>)->pyo3::PyResult<()>{
        if self.scene_browser.thumbnails.is_none(){self.scene_browser.thumbnails=Some(crate::scene_photo::thumbnail_mode(py)?);}
        Ok(())
    }
    pub fn finish_scene_photo(&mut self,key:&str,pixels:Option<crate::scene_photo::Pixels>){self.scene_browser.finish(&self.ctx,key,pixels);}
    pub fn set_delivery(&mut self, active: String, unfiled: u32, problems: u32) {
        self.delivery = Some((active, unfiled, problems));
    }
    pub fn set_options(&mut self, options: Vec<(String, String, String, u8)>) { self.options = options; }
    pub fn set_cluster(&mut self, cluster: Vec<Vec<Item>>, view_menu: Vec<(String, Vec<Item>)>) {
        self.cluster = cluster;
        self.view_menu = view_menu;
    }

    /// Review on or off. Python hears `review:1` / `review:0` and frames the
    /// camera into the area the panels leave (B8, owner decision 2026-11-23).
    pub fn release_model_texture(&mut self, target: usize) {
        if let Some(id) = self.model_textures.remove(&target) {
            self.free_after_frame.push(id);
        }
    }

    pub fn add_model_pane(&mut self, device: &wgpu::Device, target: usize, image: &wgpu::Texture,
                          bounds: [f32; 4], active: bool, label: String) {
        let id = *self.model_textures.entry(target).or_insert_with(|| {
            let view = image.create_view(&Default::default());
            self.painter.register_native_texture(device, &view, wgpu::FilterMode::Linear)
        });
        let [x, y, width, height] = bounds;
        self.model_panes.push((id, Rect::from_min_size(pos2(x,y), vec2(width,height)), active, label));
    }

    pub fn project_hidden(&self) -> bool {
        self.review || self.sheets.shown || self.gallery.is_some()
    }
    pub fn toggle_review(&mut self) { self.set_review(!self.review); }

    pub fn set_review(&mut self, on: bool) {
        if self.review != on {
            self.review = on;
            self.actions.push(format!("review:{}", u8::from(on)));
        }
    }

    /// The part of the window the model is seen in, in device coordinates
    /// (x0, y0, x1, y1, y up): the window minus the rail, the dock and the
    /// bars that are showing — the target state, not the retract animation.
    /// None before the first frame, or with a large sheet or Sheets up.
    pub fn free_ndc(&self) -> Option<[f64; 4]> {
        let s = self.screen;
        if !s.is_positive() || self.lsheet.open || self.sheets.shown || self.gallery.is_some() {
            return None;
        }
        let top = TOP_H;
        let (left, right, bottom) = if self.review {
            (0.0, s.max.x, s.max.y)
        } else {
            let left = if self.groups.is_empty() { 0.0 } else { GUT + self.rail_w };
            let dock = self.dock_w.min((s.width() * 0.45).max(DOCK_MIN_W));
            let right = if self.column.is_some() { s.max.x - GUT - dock } else { s.max.x };
            (left, right, s.max.y - BOT_H)
        };
        let x = |v: f32| f64::from(2.0 * v / s.width() - 1.0);
        let y = |v: f32| f64::from(1.0 - 2.0 * v / s.height());
        Some([x(left), y(bottom), x(right), y(top)])
    }

    /// Called only after the transient chrome and focused field declined Esc.
    pub fn model_escape(chrome: Option<&mut Self>, busy: bool, array_pending: bool, selected: bool, editing: bool) -> &'static str {
        if busy { return "tool"; }
        if array_pending { return "array"; }
        if selected { return "selection"; }
        if editing { return "edit"; }
        let Some(ui) = chrome else { return "" };
        if let Some(col) = ui.column.as_mut() {
            if !col.mode.is_empty() {
                col.mode.clear();
                ui.actions.push("tree:mode\u{1f}".into());
                return "dock";
            }
            if col.lens == Lens::Export {
                col.lens = Lens::Filing;
                ui.actions.push("tree:lens\u{1f}0".into());
                return "dock";
            }
        }
        ui.set_review(false);
        ""
    }

    fn scene_move_key(&self, name: &str, mods: u8) -> Option<bool> {
        if mods != 1 || self.cmd_open || self.ctx.wants_keyboard_input()
            || !self.menu.as_ref().is_some_and(|(kind, _)| kind == "scenes") {
            return None;
        }
        match name {
            "ArrowUp" => Some(true),
            "ArrowDown" => Some(false),
            _ => None,
        }
    }

    fn can_move_scene(&self, earlier: bool) -> bool {
        let Some(rows) = self.tabs.get(1) else { return false };
        let Some(i) = rows.iter().position(|row| row.4 == 1) else { return false };
        let peers = if earlier { &rows[..i] } else { &rows[i+1..] };
        peers.iter().any(|row| row.3 == rows[i].3)
    }

    /// Keyboard ownership: the chrome takes a key only while one of its
    /// fields has focus, or Escape while one of its popups is open.
    pub fn claims_key(&self, name: &str, mods: u8) -> bool {
        if self.gallery.is_some() || self.font_picker.is_some() { return true; }
        if name == "Escape" && (self.ctx_menu.is_some() || self.strip.menus_open() || self.egui_popup_open() || self.panels.data["terrain"].is_object()) { return true; }
        // The large sheet owns every key while it is up (Python keeps the
        // window's ⌘ shortcuts, sheet_runtime.rs).
        if self.lsheet.open { return true; }
        if self.sheets.shown { return self.sheets_claims_key(name); }
        if self.scene_move_key(name, mods).is_some() { return true; }
        let filing = self.column.as_ref().is_some_and(|c| c.filing.is_some() || c.row_menu.is_some()
            || c.kind_menu.is_some() || c.head_menu.is_some() || c.prompt.is_some() || c.edits.contains_key("\u{1d}pick"));
        self.column.as_ref().is_some_and(|c| !self.review && c.claims_row_key(name, mods)) || self.sheet.is_some() || self.dialog.is_some() || self.cmd_open || (name == "Escape" && (self.view_open || self.menu.is_some() || self.form.is_some() || filing
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
        if name == "Escape" && self.egui_popup_open() {
            if pressed { self.ctx.memory_mut(|m| m.close_popup()); }
            return true;
        }
        if self.dialog.is_some() { return self.dialog_key(name, pressed, mods); }
        if self.cmd_open && pressed {
            match name {
                "Escape" => { self.cmd_open = false; self.ctx.memory_mut(|m| m.stop_text_input()); return true; }
                "Backspace" if self.cmd_query.is_empty() && !self.cmd_scope.is_empty() => {
                    self.cmd_scope.clear();
                    self.actions.extend(["cmdscope:".into(), "cmdq:".into()]);
                    return true;
                }
                "ArrowDown" | "ArrowUp" | "PageDown" | "PageUp" => {
                    let forward = matches!(name, "ArrowDown" | "PageDown");
                    let count = if name.starts_with("Page") { 10 } else { 1 };
                    let last = self.cmd_results.len().saturating_sub(1);
                    let start = if forward { self.cmd_at.saturating_add(count).min(last) }
                        else { self.cmd_at.saturating_sub(count) };
                    let enabled = |&i: &usize| self.cmd_results[i].3;
                    let hit = if forward { (start..self.cmd_results.len()).find(enabled) }
                        else { (0..=start).rev().find(|&i| self.cmd_results.get(i).is_some_and(|r| r.3)) };
                    if let Some(i) = hit { self.cmd_at = i; self.cmd_scroll = true; }
                    return true;
                }
                "Enter" => {
                    if self.cmd_results.get(self.cmd_at).is_some_and(|r| r.3) {
                        self.actions.push(format!("cmdrun:{}", self.cmd_at));
                        self.cmd_open = false;
                        self.ctx.memory_mut(|m| m.stop_text_input());
                    }
                    return true;
                }
                _ => {}
            }
        }
        if name == "Escape" && (self.ctx_menu.is_some() || self.strip.menus_open()) {
            if pressed && self.ctx_menu.take().is_none() { self.strip.close_menus(); }
            return true;
        }
        if name == "Escape" && self.form.as_ref().is_some_and(|f| f.choice_open.is_some()) {
            if pressed { self.form.as_mut().unwrap().choice_open = None; }
            return true;
        }
        if self.lsheet.open && !self.cmd_open { return self.lsheet_key(name, pressed, mods); }
        if self.sheets.shown { return self.sheets_key(name, pressed, mods); }
        if let Some(earlier) = self.scene_move_key(name, mods) {
            if pressed && self.can_move_scene(earlier) {
                self.actions.push(if earlier { "tab:scene:up" } else { "tab:scene:down" }.into());
            }
            return true;
        }
        if name == "Escape" && self.panels.data["terrain"].is_object() && !self.ctx.wants_keyboard_input() {
            if pressed { self.actions.push(r#"panel:["terrain.close",null]"#.into()); }
            return true;
        }
        if name == "Escape" && (self.view_open || self.menu.is_some() || self.form.is_some()) {
            if pressed {
                if self.view_open {
                    self.view_open = false;
                } else if self.menu.take().is_none() && self.form.take().is_some() {
                    self.actions.push("form:close".into());
                }
            }
            return true;
        }
        if name == "Escape" {
            if let Some(col) = self.column.as_mut() {
                if col.edits.contains_key("\u{1d}pick") {
                    if pressed { col.edits.remove("\u{1d}pick"); }
                    return true;
                }
                if col.kind_menu.is_some()
                    || col.head_menu.is_some() || col.prompt.is_some() || col.row_menu.is_some() {
                    if pressed {
                        let _ = col.kind_menu.take().is_some() || col.head_menu.take().is_some()
                            || col.prompt.take().is_some() || col.row_menu.take().is_some();
                    }
                    return true;
                }
                if self.ctx.memory(|m| m.has_focus(egui::Id::new("dock_filter"))) {
                    if pressed {
                        if col.filter.is_empty() {
                            self.ctx.memory_mut(|m| m.stop_text_input());
                        } else {
                            col.filter.clear();
                            self.actions.push("tree:filter\u{1f}".into());
                        }
                    }
                    return true;
                }
                if let Some(focus) = self.ctx.memory(|m| m.focused()) {
                    if let Some((_, buf)) = col.renaming.as_mut() {
                        if pressed {
                            if buf.is_empty() {
                                col.renaming = None;
                                self.ctx.memory_mut(|m| m.surrender_focus(focus));
                            } else {
                                buf.clear();
                            }
                        }
                        return true;
                    }
                    if let Some((_, buf)) = col.edits.iter_mut().find(|(id, _)| egui::Id::new(("inspector_edit", id)) == focus) {
                        if pressed {
                            if buf.is_empty() { self.ctx.memory_mut(|m| m.surrender_focus(focus)); }
                            else { buf.clear(); }
                        }
                        return true;
                    }
                }
                if col.filing.is_some() {
                    if pressed { col.filing = None; }
                    return true;
                }
            }
        }
        if self.form.is_none() && self.menu.is_none() && !self.review {
            if let Some(col) = self.column.as_mut().filter(|c| c.claims_row_key(name, mods)) {
                if pressed {
                    if let Some(action) = col.row_key(name, mods) { self.actions.push(action); }
                }
                return true;
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

    pub fn set_active(&mut self, id: String) {
        // Another tool is the next action: the committed value's readback goes.
        if id != self.active { self.cursor.end_readback(); }
        self.active = id;
    }
    pub fn set_status(&mut self, status: String, hint: String) {
        self.status = status;
        self.hint = hint;
    }
    /// The value field's facts for this frame (ui/value_field.rs): `start`
    /// is the tool's start point on screen, `revision` that of what the value
    /// acts on (the document; the lens for Zoom).
    pub fn set_cursor_value(&mut self, caption: String, value: String, typing: bool, x: Option<f32>, y: Option<f32>,
                            start: Option<(f32, f32)>, revision: u64) {
        let at = x.zip(y).map(|(x, y)| pos2(x, y)).unwrap_or_else(|| {
            let pointer = self.ctx.input(|i| i.pointer.hover_pos()).unwrap_or(self.screen.center());
            self.cursor.retype_anchor(revision).unwrap_or(pointer - egui::vec2(0., 22.))
        });
        self.cursor.set(caption, value, typing, at, start.map(|(x, y)| at), revision);
    }
    pub fn set_selection(&mut self, summary: String, actions: Vec<(String, String, String, bool, bool)>) {
        self.selection = (!summary.is_empty()).then_some((summary, actions));
    }
    /// Where an area was last laid out, in points (tests and harnesses).
    pub fn area_rect(&self, name: &str) -> Option<(f32, f32, f32, f32)> {
        self.ctx.memory(|m| m.area_rect(egui::Id::new(name))).map(|r| (r.min.x, r.min.y, r.width(), r.height()))
    }
    pub fn take_actions(&mut self) -> Vec<String> { std::mem::take(&mut self.actions) }
    pub fn repaint_after_ms(&self) -> Option<f64> { self.repaint_ms }

    /// Only the interactive chrome takes the pointer; labels pass it through.
    fn egui_popup_open(&self) -> bool {
        self.ctx.memory(|m| m.any_popup_open())
    }

    pub fn input_state(&self) -> std::collections::HashMap<&'static str, bool> {
        let visible = |name| self.ctx.memory(|m| m.areas().visible_last_frame(
            &egui::LayerId::new(egui::Order::Foreground, egui::Id::new(name))));
        [
            ("recent_preview", self.glass.showing_preview()),
            ("tool_rail", visible("palette") || visible("sheets_rail")), ("dock", visible("column") || visible("sheets_dock")), ("bottom_bar", visible("bottom")),
            ("view_menu", self.view_open),
            ("drawing_line", self.column.as_ref().is_some_and(|c| c.lens==Lens::Drawing && c.styles.catalog.editing_line())),
            ("review", self.review),
            ("dialog", self.dialog.is_some()),
            ("choice", self.dialog.as_ref().is_some_and(|d| d.choice_open.is_some()) || self.lsheet.choice_open()),
            ("filter_focus", self.ctx.memory(|m| m.has_focus(egui::Id::new("dock_filter")))),
        ].into()
    }

    fn dismissible_menu_open(&self) -> bool {
        self.view_open || self.menu.is_some() || self.cmd_open
            || self.form.as_ref().is_some_and(|f| f.choice_open.is_some())
            || self.ctx_menu.is_some() || self.strip.menus_open() || (self.sheets.shown && self.sheets.dismissible_menu_open())
            || self.column.as_ref().is_some_and(|c| c.row_menu.is_some() || c.head_menu.is_some()
                || c.kind_menu.is_some()
                || c.edits.contains_key("\u{1d}pick"))
    }

    fn over_chrome(&self, pos: Pos2) -> bool {
        // A choice or colour picker can extend beyond its parent panel.
        // Its next outside click dismisses it before the model takes input.
        self.font_picker.is_some() || self.egui_popup_open() || self.hit.iter().any(|r| r.contains(pos))
            || self.ctx.layer_id_at(pos).is_some_and(|layer| self.panels.popup_layers.contains(&layer.id))
    }
    /// Is (x, y) on the interactive chrome (a right-click there is the
    /// chrome's, not the model's)?
    pub fn over_chrome_at(&self, x: f32, y: f32) -> bool { self.captured || self.over_chrome(pos2(x, y)) }

    /// kind: "move" | "press" | "release" | "leave". Coordinates in logical
    /// points. Returns whether the chrome takes the event (the view must
    /// then neither hover, pick, nor navigate).
    pub fn pointer(&mut self, kind: &str, x: f32, y: f32, button: u8, mods: u8) -> bool {
        let pos = pos2(x, y);
        // A press is the next action: the committed value's readback goes.
        if kind == "press" {
            self.cursor.end_readback();
            if let Some(c) = self.column.as_mut() { c.focused_row = None; }
        }
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
                if !self.captured {
                    self.captured = self.dismissible_menu_open();
                    if let Some(form) = self.form.as_mut() { form.choice_open = None; }
                    self.view_open = false;
                    self.menu = None;
                    self.cmd_open = false;
                    self.sheets_close_popups();
                    self.ctx_menu = None;
                    self.strip.close_menus();
                    if let Some(col) = self.column.as_mut() {
                        if !self.captured && button == egui::PointerButton::Primary && col.lens == Lens::Drawing {
                            col.styles.clear_focus();
                            col.highlighted = None;
                        }
                        col.row_menu = None;
                        col.kind_menu = None;
                        col.head_menu = None;


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

    fn palette(&mut self, ctx: &egui::Context, screen: Rect, retract: f32) {
        // Two columns, then three, then a scroll (ui/rail.rs, council B10).
        let (band_top, band_bottom) = self.band(ctx, screen);
        let (clicked, width) = rail::show(ctx, "palette", &self.groups, &self.active, band_top, band_bottom - band_top, retract);
        self.rail_w = width;
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
        self.cluster_buttons.clear();
        let sheet_up = self.lsheet.open;
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
                                let max_w = if sheet_up { screen.width() - lights - 68.0 } else { (screen.width() - lights - 8.0 - self.top_right_w - 300.0 - 60.0).max(160.0) };
                                let acts = self.strip.show(ui, max_w);
                                self.actions.extend(acts);
                                if !sheet_up { bar_divider(ui); }
                            } else if let Some((_, docs)) = &self.projects {
                                let cur = docs.iter().find(|d| d.1).or(docs.first());
                                let label = cur.map_or("Untitled".to_owned(), |d| if d.2 { format!("{} •", d.0) } else { d.0.clone() });
                                let more = if docs.len() > 1 { format!("  {}", docs.len()) } else { String::new() };
                                if let Some(r) = menu_button(ui, &format!("{label}{more}"), self.menu.as_ref().is_some_and(|m| m.0 == "docs")) {
                                    open = Some(("docs", r));
                                }
                                bar_divider(ui);
                            }
                            if sheet_up { return; }
                            if self.review {
                                let exit = text_button_response(ui, "Exit review", "Show the panels again", true);
                                self.cluster_buttons.push(("exit_review".into(), exit.rect.center().into()));
                                if exit.clicked() { clicked = Some("review".into()); }
                                bar_divider(ui);
                            }
                            let work = self.tabs.first().and_then(|g| g.iter().find(|i| i.4 == 1)).map_or("Model".to_owned(), |i| i.1.clone());
                            if let Some(r) = menu_button(ui, &work, self.menu.as_ref().is_some_and(|m| m.0 == "work")) {
                                open = Some(("work", r));
                            }
                            if let Some(scenes) = self.tabs.get(1).filter(|_| self.tabs[0].iter().any(|row| row.0 == "model" && row.4 == 1)) {
                                let cur = scenes.iter().position(|i| i.4 == 1);
                                let prev = (String::new(), "‹".to_owned(), "Previous scene (⇞)".to_owned(), String::new(), -1i8, true);
                                if !scenes.is_empty() && item_button(ui, &prev, None) { clicked = Some("scene:prev".into()); }
                                let name = cur.map_or("Scenes".to_owned(), |i| scenes[i].1.clone());
                                if let Some(r) = menu_button(ui, &name, self.menu.as_ref().is_some_and(|m| m.0 == "scenes")) {
                                    open = Some(("scenes", r));
                                }
                                let next = (String::new(), "›".to_owned(), "Next scene (⇟)".to_owned(), String::new(), -1i8, true);
                                if !scenes.is_empty() && item_button(ui, &next, None) { clicked = Some("scene:next".into()); }
                            }
                        });
                        groups.push(left.response.rect);
                        if sheet_up { return; }
                        // The right group, laid out left to right and pushed to
                        // the right edge by its width from the last frame.
                        ui.add_space((ui.available_width() - self.top_right_w).max(0.0));
                        {
                            let right = ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing = vec2(SPACE_XS, 0.0);
                                for (gi, group) in self.cluster.iter().enumerate() {
                                    if gi > 0 { bar_divider(ui); }
                                    for item in group {
                                        let response = item_response(ui, item, self.icons.get(&item.3));
                                        self.cluster_buttons.push((item.0.clone(), response.rect.center().into()));
                                        if response.clicked() {
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
            Some("review") => self.toggle_review(),
            Some(a) => self.actions.push(if a.starts_with("act:") { a.to_owned() } else { format!("tab:{a}") }),
            None => {}
        }
    }

    /// Is ``pos`` on the top bar but on none of its controls (it then moves
    /// the window, as a title bar does)?
    pub fn top_empty(&self, x: f32, y: f32) -> bool {
        let p = pos2(x, y);
        !self.dismissible_menu_open() && self.gallery.is_none() && y <= TOP_H && !self.top_groups.iter().any(|r| r.contains(p))
            && !((!self.lsheet.open || !self.jobs.is_empty()) && self.ctx.memory(|m| m.area_rect(egui::Id::new("jobs"))).is_some_and(|r| (!self.jobs.is_empty() || !self.history.is_empty()) && r.contains(p)))
            && !(self.projects.is_none() && x < 86.0)
    }

    /// The bottom bar: units | tool, its options, hint or message | where
    /// new geometry goes.
    fn bottom_bar(&mut self, ctx: &egui::Context, screen: Rect, retract: f32) {
        let mut clicked: Option<String> = None;
        egui::Area::new(egui::Id::new("bottom"))
            .constrain(false)
            .interactable(retract == 0.0)
            .fixed_pos(pos2(0.0, screen.max.y - BOT_H * (1.0 - retract)))
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

    /// Whether the Paint strip shows: always while Paint is active, even on
    /// a model with no materials, so Browse all… is always one click away
    /// (council B6).
    fn paint_strip_shown(&self) -> bool {
        self.active == "paint" || !self.palette_items.is_empty()
    }

    /// With Paint active: the model's materials as a strip above the bottom
    /// bar, the current one ringed, and Browse all… into the Library sheet.
    fn paint_strip(&mut self, ctx: &egui::Context, screen: Rect) {
        if !self.paint_strip_shown() { return; }
        let (_, band_bottom) = self.band(ctx, screen);
        let centre = (GUT + self.rail_w + self.free_right(ctx, screen)) / 2.0;
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
                        if self.palette_items.is_empty() {
                            ui.add_space(SPACE_XS);
                            ui.label(egui::RichText::new("No materials in this model yet").font(body()).color(DIM));
                            ui.add_space(SPACE_XS);
                        } else if self.palette_items.last().is_some_and(|i| i.0 != "-" && !i.0.is_empty()) { bar_divider(ui); }
                        if text_button(ui, "Browse all…", "Every material: the Library", true) {
                            clicked = Some("libsheet:materials".into());
                        }
                    });
                });
            });
        if let Some(a) = clicked { self.actions.push(a); }
    }

    /// The one open menu: documents, workspace or scenes (View has its own).
    fn bar_menu(&mut self, ctx: &egui::Context) {
        let Some((kind, anchor)) = self.menu.clone() else { return };
        let mut clicked: Option<String> = None;
        let mut submenu = None;
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
                            if kind=="scenes" && self.tabs.get(3).is_some_and(|rows|!rows.is_empty()) {
                                let scenes=&self.tabs[1];
                                current=scenes.iter().find(|r|r.4==1).map(|r|r.1.clone());
                                self.scene_browser.show(ui,scenes,&self.tabs[3],&mut clicked,&mut self.actions);
                            } else if let Some(items) = self.tabs.get(group) {
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
                                    rows.push(("scene:editcamera", format!("Edit camera of “{c}”…")));
                                    rows.push(("scene:update", format!("Update “{c}” from this view")));
                                    rows.push(("scene:duplicate", format!("Duplicate “{c}”")));
                                    rows.push(("scene:rename", format!("Rename “{c}”…")));
                                    rows.push(("scene:up", "Move earlier".into()));
                                    rows.push(("scene:down", "Move later".into()));
                                    rows.push(("scene:delete", format!("Delete “{c}”")));
                                }
                                for (id, label) in rows {
                                    let (hint, enabled) = match id {
                                        "scene:up" => ("Alt+↑", self.can_move_scene(true)),
                                        "scene:down" => ("Alt+↓", self.can_move_scene(false)),
                                        _ => ("", true),
                                    };
                                    if menu_row(ui, &(id.to_owned(), label, hint.into(), "check".into(), -1, enabled)) {
                                        clicked = Some(if id == "scene:add" {
                                            self.scene_browser.add_scene_action()
                                        } else { format!("tab:{id}") });
                                    }
                                }
                                ui.add_space(SPACE_XS);
                                caption_at(ui, "Visibility sets", 12.0);
                                if let Some(sets) = self.tabs.get(2) {
                                    for set in sets {
                                        if menu_row(ui, set) { submenu = Some(set.0.clone()); }
                                    }
                                }
                                if menu_row(ui, &("".into(), "Save current visibility…".into(), "".into(), "".into(), -1, true)) {
                                    clicked = Some("tab:visibility:save".into());
                                }
                            }
                        }
                        set if set.starts_with("visibility:apply:") => {
                            if let Some(row) = self.tabs.get(2).and_then(|rows| rows.iter().find(|r| r.0 == set)) {
                                caption_at(ui, &row.1, 12.0);
                                let id = &set["visibility:apply:".len()..];
                                for (verb, label) in [("apply", "Apply"), ("update", "Update from current visibility"),
                                                       ("rename", "Rename…"), ("delete", "Delete…")] {
                                    if menu_row(ui, &("".into(), label.into(), "".into(), "".into(), -1, true)) {
                                        clicked = Some(format!("tab:visibility:{verb}:{id}"));
                                    }
                                }
                            }
                            if menu_row(ui, &("".into(), "Back to scenes".into(), "".into(), "".into(), -1, true)) {
                                submenu = Some("scenes".to_owned());
                            }
                        }
                        _ => {}
                    }
                });
            });
        if let Some(next) = submenu { self.menu = Some((next, anchor)); }
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
                    egui::ScrollArea::vertical().id_salt("view command groups")
                        .max_height((self.screen.bottom() - a.bottom() - 90.).max(ROW_H * 3.))
                        .show(ui, |ui| {
                            for (si, (title, items)) in self.view_menu.iter().enumerate() {
                                if si > 0 { ui.add_space(SPACE_XS); }
                                caption_at(ui, title, 12.0);
                                let icons_only = items.iter().all(|i| self.icons.contains_key(&i.3));
                                if icons_only {
                                    ui.horizontal_wrapped(|ui| {
                                        for item in items {
                                            if item_button(ui, item, self.icons.get(&item.3)) { clicked = Some(item.0.clone()); close = true; }
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
            });
        if close { self.view_open = false; }
        if toggle_review {
            self.toggle_review();
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

    fn right_column(&mut self, ctx: &egui::Context, screen: Rect, retract: f32) {
        let delivery = self.delivery.clone();
        let visibility_sets = self.tabs.get(2).cloned().unwrap_or_default();
        let selection = self.selection.clone();
        let mut cmds: Vec<String> = Vec::new();
        let (top, bottom) = self.band(ctx, screen);
        let self_icons = self.icons.clone();
        let Some(col) = self.column.as_mut() else { return };
        #[allow(non_snake_case)]
        let W: f32 = self.dock_w.min((screen.width() * 0.45).max(DOCK_MIN_W));
        let height = (bottom - top).max(200.0);
        let mut actions: Vec<String> = Vec::new();
        egui::Area::new(egui::Id::new("column"))
            .constrain(false)
            .interactable(retract == 0.0)
            .fixed_pos(pos2(screen.max.x - GUT - W + retract * (W + GUT), top))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Panel).inner_margin(Margin::same(10)).show(ui, |ui| {
                    ui.set_width(W - 20.0);
                    ui.set_max_width(W - 20.0);
                    ui.set_height(height - 20.0);
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    if self.panels.data["camera"].is_object() {
                        egui::ScrollArea::vertical().id_salt("camera inspector").show(ui, |ui| self.panels.camera(ui,&mut cmds));
                        return;
                    }
                    ui.horizontal(|ui| {
                        ui.set_min_height(28.0);
                        let labels = [if col.proposal_available { "Tags" } else { "Filing" }, "Drawing", "Export"];
                        let width = (ui.available_width()-84.).clamp(144.,220.);
                        if let Some(i) = widgets::segmented(ui,&labels,col.lens as usize,width) {
                            self.plugins.showing = false;
                            col.lens = Lens::from_index(i as u8);
                            actions.push(format!("lens\u{1f}{i}"));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let dots = (String::new(), "⋯".to_owned(), "Structure: new node, propose from tags…".to_owned(), String::new(), -1i8, true);
                            let before = ui.cursor();
                            if item_button(ui, &dots, None) {
                                col.head_menu = Some(pos2(before.max.x, before.min.y + 30.0));
                            }
                            let total = delivery.as_ref().map_or(0, |d| d.2) + col.unfiled;
                            if total > 0 && !col.proposal_available && kit::quiet(ui,&format!("⚠ {total}")).on_hover_text("Open export checks").clicked() {
                                actions.push("lens\u{1f}2".into());
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
                    {
                    if col.lens == Lens::Export {
                        export::strip(ui,&self.panels.data["readiness"],&mut cmds);
                    } else if col.lens == Lens::Drawing {
                        ui.horizontal(|ui| {
                            if kit::quiet(ui, "Tag folders").clicked() { col.styles.pick_folder("", false); }
                            egui::ComboBox::from_id_salt("drawing_visibility").selected_text("Visibility set").show_ui(ui, |ui| {
                                for set in &visibility_sets {
                                    if ui.selectable_label(false, &set.1).clicked() { cmds.push(format!("tab:{}",set.0)); }
                                }
                                if kit::quiet(ui,"Save current visibility…").clicked() { cmds.push("tab:visibility:save".into()); }
                            });
                        });
                    } else if col.lens == Lens::Filing && !col.proposal_available {
                        if let Some((target,_,_)) = &delivery {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = SPACE_XS;
                                kit::caption_plain(ui,"New items go to");
                                ui.label(if target.is_empty() { "No filing target" } else { target });
                            });
                        }
                    }
                    kit::path_bar(ui, &col.path, &mut actions);
                    kit::host_visibility(ui, &col.host.0, &col.host.1, &mut actions);
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
                    let edit = kit::field(&mut col.filter).id(egui::Id::new("dock_filter"))
                        .hint_text(egui::RichText::new(if col.filing.is_some() { "Node name or kind" }
                            else { "Filter nodes" }).color(FAINT))
                        .margin(Margin::symmetric(8, 5))
                        .desired_width((W - 20.0 - 16.0 - 4.0).max(40.0))
                        .font(body());
                    let resp = ui.add(edit).on_hover_text("Filter by name or wildcard. Use is:hidden, is:visible, is:locked, is:selected, is:component, is:image or is:section. Prefix the query with ! to invert it.");
                    if col.focus_filter {
                        resp.request_focus();
                        col.focus_filter = false;
                    }
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if resp.changed() {
                        actions.push(format!("filter\u{1f}{}", col.filter));
                    }
                    ui.add_space(SPACE_XS);
                    // Enter files the selection at the first matching destination.
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
                    let available = ui.available_height();
                    let room = if !col.mode.is_empty() && col.inspector.is_empty() {
                        available - 44.0
                    } else if col.lens == Lens::Drawing {
                        available * 0.5
                    } else if col.lens == Lens::Filing && !col.identity.is_empty() {
                        // Keep the identity fields and their actions together in short windows.
                        (available * 0.6).min(available - ROW_H * 12.)
                    } else {
                        available * 0.6
                    };
                    if col.proposal_available && !col.proposal_dismissed && col.lens == Lens::Filing {
                        ui.horizontal(|ui| {
                            if kit::quiet(ui, "Propose kinds…").on_hover_text("Preview a filing structure from these tags").clicked() {
                                actions.push("propose\u{1f}".into());
                            }
                            if kit::quiet(ui, "×").on_hover_text("Dismiss the suggestion").clicked() {
                                col.proposal_dismissed = true;
                            }
                        });
                    } else if col.nodes.is_empty() {
                        empty_state(ui, "No matching rows");
                    }
                    let nodes_len = col.nodes.len();
                    // The header and virtualized rows share the lens layout.
                    let row_left = ui.cursor().min.x;
                    let cols = lens::RowLayout::plan(col.lens, row_left, row_left + ui.available_width());
                    let row_room = if col.mode.is_empty() && nodes_len > 0 {
                        let header = cols.header(ui, row_left, &mut actions);
                        if col.lens == Lens::Drawing {
                            drawing::drawing_drop(ui, &header, &col.nodes, None, col.styles.owner, &mut actions);
                        }
                        drawing::drop_on(ui, &header, "", &mut actions);
                        room - ROW_H
                    } else { room };
                    let visible_rows = (row_room / ROW_H).floor().max(3.0);
                    let tree_h = (nodes_len.max(3) as f32).min(visible_rows) * ROW_H;
                    let mut scroll = egui::ScrollArea::vertical().id_salt("tree").max_height(tree_h).auto_shrink([false, false]);
                    if let Some(key) = col.reveal.take() {
                        let pitch = ROW_H + ui.spacing().item_spacing.y;
                        if let Some(offset) = tree_order::reveal_offset(&col.nodes, &col.ordered, &key, pitch, tree_h) {
                            scroll = scroll.vertical_scroll_offset(offset);
                        }
                    }
                    scroll.show_rows(ui, ROW_H, nodes_len, |ui, range| {
                            for i in range {
                                let index = if col.mode.is_empty() { col.ordered[i] } else { i };
                                let n = &col.nodes[index];
                                let editable = n.row == 0 && n.key.starts_with("n:") && n.key.len() > 2 && !n.key.starts_with("n:link:");
                                let drawing_row = col.lens == Lens::Drawing && (n.key.starts_with("preview-folder:") || n.key.starts_with("preview-tag:"));
                                let sense = if editable || n.local_placement || drawing_row { Sense::click_and_drag() } else { Sense::click() };
                                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), sense);
                                // Items in the selection are lit like a picked row; nodes count them.
                                let lit = (col.lens == Lens::Drawing && col.styles.selected(n)) || n.mark == 1 || (selection.is_none() && col.highlighted.as_deref() == Some(n.key.as_str()));
                                let file_state = if n.row == 0 { lens::file_state(&n.icon) } else { None };
                                let cols = lens::RowLayout::plan(col.lens, rect.min.x, rect.max.x);
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
                                let x0 = rect.min.x + 22.0 + n.depth as f32 * 14.0;
                                let tri = Rect::from_min_size(pos2(x0, rect.min.y), vec2(14.0, ROW_H));
                                if n.expandable {
                                    ui.painter().text(tri.center(), egui::Align2::CENTER_CENTER, if n.expanded { "▾" } else { "▸" },
                                        meta(), if n.row == 0 { FAINT } else { OFF });
                                }
                                // Right columns: problems, count, then eye and lock —
                                // or the Drawing / Export cells.

                                let mut cells_left = f32::MAX;
                                let warn_col = rect.max.x - 6.0;
                                if col.lens != Lens::Filing {
                                    // Imported tag previews contain drawing attributes only.
                                    let cells = if n.row == 6 && col.lens == Lens::Export { &[] } else { n.cells.as_slice() };
                                    let cell = |i: usize| cells.get(i).cloned().unwrap_or_default();
                                    let inherited = n.row != 0;
                                    let ink_c = if inherited { OFF } else { DIM };
                                    if col.lens == Lens::Drawing && !cells.is_empty() {
                                        for (slot, index) in [(cols.weight, 2), (cols.line, 1), (cols.hatch, 4)] {
                                            if let Some((a, b)) = slot {
                                                let sample = Rect::from_min_max(pos2(a,cy-6.),pos2(b,cy+6.));
                                                if index == 4 {
                                                    kit::hatch_sample(ui, sample.expand2(vec2(0., 2.)), &cell(4), n.hatch.as_deref());
                                                } else {
                                                    kit::line_sample(ui, sample, &cell(2), if index == 1 { cells[1].as_str() } else { "continuous" }, ink_c);
                                                }
                                            }
                                        }
                                        if let Some((_, end)) = cols.print {
                                            ui.painter().text(pos2(end, cy), egui::Align2::RIGHT_CENTER, if cell(3) == "1" { "✓" } else { "–" }, meta(), ink_c);
                                        }
                                        if let Some((a,b)) = cols.swatch {
                                            let sw = Rect::from_center_size(pos2((a+b)*0.5,cy),vec2(12.0,12.0));
                                            kit::tag_swatch(ui, sw, parse_hex(cells.get(20).or(cells.first()).map_or("", String::as_str)));
                                        }
                                    } else if col.lens == Lens::Export {
                                        let right = rect.right() - SPACE_S;
                                        cells_left = right - 78.;
                                        if n.row == 0 || n.row == 6 {
                                            let (label,ink) = if n.problems>0 {(format!("{} to fix", n.problems),WARN)} else { (String::new(),DIM) };
                                            ui.painter().text(pos2(right,cy), egui::Align2::RIGHT_CENTER,label,meta(),ink);
                                        }

                                    }
                                }

                                if n.problems > 0 && col.lens == Lens::Filing && col.issues {
                                    let g = ui.painter().layout_no_wrap(n.problems.to_string(), meta(), WARN);
                                    let w = g.size().x;
                                    ui.painter().galley(pos2(warn_col - w, cy - g.size().y / 2.0), g, WARN);
                                    ui.painter().circle_filled(pos2(warn_col - w - 6.0, cy), 2.5, WARN);
                                }
                                let count_col = warn_col - 40.0;
                                let locate = file_state == Some("missing") && col.lens == Lens::Filing;
                                if col.lens == Lens::Filing && !locate && (n.count > 0 || n.row == 0) {
                                    let g = ui.painter().layout_no_wrap(n.count.to_string(), meta(), DIM);
                                    ui.painter().galley(pos2(count_col - g.size().x, cy - g.size().y / 2.0), g, DIM);
                                }
                                let show_ctl = resp.hovered() || n.eye == 0 || n.lock >= 1;
                                let control_edge = cols.flags.map_or(count_col, |(_, b)| b + 36.0);
                                let (_, lock_r) = tree_slots(ui, control_edge, cy, -1, n.lock, show_ctl || col.lens == Lens::Drawing);
                                let eye_r = Rect::from_center_size(pos2(rect.min.x + 10., cy), vec2(16., ROW_H));
                                if n.eye >= 0 { kit::paint_row_eye(ui, eye_r, n.eye); }
                                if resp.hovered() {
                                    for (slot, tip) in [(eye_r, &n.control_tips[0]), (lock_r, &n.control_tips[1])] {
                                        if !tip.is_empty() && ui.rect_contains_pointer(slot) {
                                            resp.clone().on_hover_text(tip);
                                        }
                                    }
                                }
                                // A row with no eye or lock (problems, library) keeps its width for the name.
                                let bare = n.eye < 0 && n.lock < 0 && n.problems == 0;
                                let right = if col.lens == Lens::Drawing { cols.name_right } else if col.lens == Lens::Export { cells_left.min(cols.name_right) - 8.0 }
                                    else if locate { warn_col - 50.0 }
                                    else if (bare || !show_ctl) && (n.count > 0 || n.row == 0) { count_col - 34.0 }
                                    else if bare { rect.max.x - 8.0 } else { lock_r.min.x - 6.0 };
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
                                    lens::paint_file(ui.painter(), pos2(tx + 6.0, cy), if lit { BRIGHT } else { DIM });
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
                                        if col.lens == Lens::Drawing && key.starts_with("preview-tag:") &&
                                            ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                            col.styles.pick(n, false);
                                            col.focused_row = Some(key.clone());
                                        }
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
                                let ink = if n.dim && !lit { OFF } else { ink };
                                // The selection is marked once (tree A): lit where it
                                // is held, a quiet dot on the rows above (after the name).
                                let marks_w = if file_state.is_some() { 10.0 } else { 0.0 } + if n.mark == 2 { 10.0 } else { 0.0 };
                                let right = right - marks_w;
                                if n.active && n.row != 5 {
                                    let icon = Rect::from_center_size(pos2(tx + 6., cy), vec2(12., 12.));
                                    if let Some(line) = self_icons.get("line") {
                                        egui::Image::new(line).tint(DIM).paint_at(ui, icon);
                                    } else {
                                        ui.painter().line_segment([icon.left_bottom(), icon.right_top()], Stroke::new(1.5_f32, DIM));
                                    }
                                    tx += icon.width() + SPACE_XS;
                                }
                                let name = n.name.clone();
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
                                    dot(ui.painter(), c, lens::state_color(state));
                                    let tip = match state { "resolved" => "Up to date", "modified" => "Changed on disk: reload to see the changes",
                                        "missing" => "Not found: relink it", _ => "Packed in this file" };
                                    ui.interact(Rect::from_center_size(c, vec2(10.0, 10.0)), ui.id().with(("file_dot", i)), Sense::hover())
                                        .on_hover_text(tip);
                                    kx += 10.0;
                                    let label = match state { "resolved" => "found", "modified" => "changed", "missing" => "missing", _ => "packed" };
                                    let galley = ui.painter().layout_job(one_line(label.into(), meta(), if state == "missing" { ACCENT } else { FAINT }, right - kx));
                                    let text_width = galley.size().x;
                                    ui.painter().galley(pos2(kx, cy - galley.size().y/2.), galley, DIM);
                                    if locate {
                                        let bounds = ui.painter().text(pos2(warn_col, cy), egui::Align2::RIGHT_CENTER, "Locate…", meta(), ACCENT);
                                        if ui.interact(bounds.expand(3.), ui.id().with(("locate", &n.key)), Sense::click()).clicked() {
                                            actions.push(format!("linkrelink\u{1f}{}", n.key));
                                        }
                                    }
                                    kx += text_width + SPACE_XS;
                                }
                                if n.mark == 2 {
                                    lens::paint_partial(ui.painter(), pos2(kx + 2.0, cy));
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
                                let on_control = |p: Pos2| (n.expandable && tri.contains(p)) ||
                                    (n.eye >= 0 && eye_r.expand(2.0).contains(p)) ||
                                    (n.lock >= 0 && lock_r.expand(2.0).contains(p));
                                if drawing_row {
                                    for (lock, state, slot, verb) in [(false, n.eye, eye_r, "eye"), (true, n.lock, lock_r, "lock")] {
                                        if state < 0 { continue; }
                                        let hit = ui.interact(slot, resp.id.with(verb), Sense::click_and_drag());
                                        if hit.contains_pointer() && ui.input(|i| i.pointer.primary_pressed()) {
                                            actions.push(col.styles.state_action(n, lock).unwrap_or_else(|| format!("{verb}\u{1f}{}", n.key)));
                                        }
                                    }
                                }
                                let drag_started = resp.drag_started() &&
                                    !ui.input(|i| i.pointer.press_origin().is_some_and(on_control));
                                if drawing_row {
                                    if drag_started {
                                        if let Some(payload) = drawing::DrawingDrag::begin(&col.styles, n) {
                                            resp.dnd_set_drag_payload(payload);
                                        }
                                    }
                                    drawing::drawing_drop(ui, &resp, &col.nodes, Some(index), col.styles.owner, &mut actions);
                                } else if editable {
                                    if drag_started { resp.dnd_set_drag_payload(drawing::DraggedRow::Node(n.key.clone())); }
                                    drawing::drop_on(ui, &resp, n.key.strip_prefix("n:").unwrap_or(""), &mut actions);
                                } else if n.local_placement {
                                    if drag_started { resp.dnd_set_drag_payload(drawing::DraggedRow::Placement { key: n.key.clone(), selected: n.selected > 0 }); }
                                    drawing::drop_on_occurrence(ui, &resp, &n.key, &mut actions);
                                }
                                if resp.clicked() {
                                    let p = resp.interact_pointer_pos().unwrap_or(rect.center());
                                    if col.lens==Lens::Drawing && n.cells.len()<7 && !n.key.starts_with("preview-folder:") {col.styles.clear_focus();}
                                    if n.expandable && tri.contains(p) {
                                        actions.push(format!("toggle\u{1f}{}", n.key));
                                    } else if drawing_row && on_control(p) {
                                        // Drawing controls own their gesture from the press onward.
                                    } else if n.eye >= 0 && eye_r.expand(2.0).contains(p) {
                                        let edit = (col.lens == Lens::Drawing).then(|| col.styles.state_action(n, false)).flatten();
                                        actions.push(edit.unwrap_or_else(|| format!("{}\u{1f}{}", if n.local_placement { "roweye" } else { "eye" }, n.key)));
                                    } else if n.lock >= 0 && lock_r.expand(2.0).contains(p) {
                                        let edit = (col.lens == Lens::Drawing).then(|| col.styles.state_action(n, true)).flatten();
                                        actions.push(edit.unwrap_or_else(|| format!("{}\u{1f}{}", if n.local_placement { "rowlock" } else { "lock" }, n.key)));
                                    } else if col.lens == Lens::Drawing && n.cells.len() >= 7 && !linked && col.filing.is_none() {
                                        col.focused_row = Some(n.key.clone());
                                        resp.surrender_focus();
                                        col.styles.pick(n, ui.input(|i| i.modifiers.command || i.modifiers.shift));
                                    } else if col.lens==Lens::Drawing && n.key.starts_with("preview-folder:") {
                                        col.focused_row = None;
                                        col.styles.pick_folder(&n.key["preview-folder:".len()..], ui.input(|i| i.modifiers.command || i.modifiers.shift));
                                        col.highlighted=None;
                                    } else if n.row == 9 {
                                        actions.push(format!("toggle\u{1f}{}",n.key));
                                    } else if n.row == 6 {
                                        if n.key.starts_with("preview-folder:") || col.lens==Lens::Export && n.expandable {
                                            actions.push(format!("toggle\u{1f}{}",n.key));
                                        } else { col.highlighted=Some(n.key.clone()); }
                                        continue;
                                    } else if col.filing.is_some() {
                                        if n.row == 0 && !linked {
                                            actions.push(format!("file\u{1f}{}", n.key));
                                            col.filing = None;
                                            col.filter.clear();
                                            actions.push("filter\u{1f}".into());
                                        }
                                    } else {
                                        col.highlighted = Some(n.key.clone());
                                        if col.lens == Lens::Filing && (n.local_placement || n.key.starts_with("section:")) {
                                            col.focused_row = Some(n.key.clone());
                                            resp.surrender_focus();
                                        }
                                        let verb = if n.local_placement && ui.input(|i| i.modifiers.shift || i.modifiers.command) {
                                            "toggleitem"
                                        } else if n.local_placement { "pickrow" } else { match n.row { 0 => "node", 3 => "reveal", 4 => "loose", 5 => "mat", _ => "item" } };
                                        actions.push(format!("{verb}\u{1f}{}", n.key));
                                    }
                                }
                                if resp.double_clicked() && !resp.interact_pointer_pos().is_some_and(on_control) && (n.row < 6 || n.row == 8) && col.filing.is_none() {
                                    let verb = if n.row == 0 { "isolate" } else { "open" };
                                    actions.push(format!("{verb}\u{1f}{}", n.key));
                                }
                                if resp.secondary_clicked() && (drawing_row || n.key.starts_with("section:") || n.row < 6 && n.row != 4 && (n.row != 5 || n.key.starts_with("cm:"))) {
                                    col.group_rows.clear();
                                    if n.key.starts_with("g:") || n.key.starts_with("i:") || n.key.starts_with("c:") || n.key.starts_with("d:") {
                                        actions.push(format!("grouprows\u{1f}{}", n.key));
                                    }
                                    col.row_menu = resp.interact_pointer_pos().map(|p| (n.key.clone(), n.row, p));
                                }
                            }
                        });
                    }
                    let selection = selection.as_ref();
                    // Inspector.
                    ui.add_space(SPACE_S);
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0_f32, EDGE));
                    ui.add_space(SPACE_S);
                    if self.active == "fillet" && self.panels.data["bevel"].is_object() {
                        egui::ScrollArea::vertical().id_salt("bevel inspector").show(ui, |ui| {
                            self.panels.bevel_inspector(ui, &mut cmds);
                        });
                        return;
                    }
                    if col.mode.is_empty() && col.lens == Lens::Drawing && (col.styles.active() || (col.inspector_title.is_empty() && selection.is_none())) && col.styles.draw(ui, &mut actions) { return; }
                    egui::ScrollArea::vertical().id_salt("inspector").auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing = vec2(0.0, SPACE_XS);
                        if col.lens == Lens::Export {
                            if !col.inspector_title.is_empty() { ui.strong(&col.inspector_title); }
                            kit::identity_card(ui, &col.identity, &mut actions);
                            export::checklist(ui,&self.panels.data["readiness"],col.export_focus.0.as_deref(),&col.export_focus.1,&mut actions);
                            self.panels.assembly_problems(ui, &mut cmds);
                            if self.panels.has_records() {
                                ui.collapsing("Export details", |ui| self.panels.export(ui,&mut cmds));
                            }
                            // Retain the solid provider's fault-specific repairs.
                            for (title,rows) in &col.inspector {
                                if title=="Actions" {
                                    for (id,label) in rows.iter().filter(|(id,_)|id.starts_with("solidfault:") || id=="solidrepair") {
                                        if kit::quiet(ui,label).clicked() { actions.push(format!("insp:{id}")); }
                                    }
                                }
                            }
                            return;
                        }
                        if col.lens==Lens::Filing && col.proposal_available && selection.is_none() {
                            if let Some(tag)=col.nodes.iter().find(|n|n.key.starts_with("preview-tag:") && col.highlighted.as_ref()==Some(&n.key)) {
                                if let Some(at)=kit::tag_inspector(ui,&tag.key,&tag.name,tag.count,&tag.cells,&mut actions){col.kind_menu=Some((tag.key.clone(),at));}
                                return;
                            }
                        }
                        if col.inspector_title.is_empty() {
                            if col.lens==Lens::Drawing { empty_state(ui,"Select tags to edit their drawing style"); return; }
                            ui.strong(if col.overview.file.is_empty(){"Untitled"}else{&col.overview.file});
                            if col.proposal_available {
                                kit::prop_row(ui,"Tags",&format!("{} tags · {} folders",col.overview.tags,col.overview.folders));
                            } else {
                                kit::prop_row(ui,"Filing",&format!("{} nodes · {} items",col.overview.nodes,col.overview.placements));
                                kit::prop_row(ui, "Not filed", &col.unfiled.to_string());
                            }
                            if col.overview.links>0 {kit::prop_row(ui,"Linked files",&col.overview.links.to_string());}
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
                        kit::identity_card(ui, &col.identity, &mut actions);
                        if col.mode.is_empty() {
                            self.panels.inspector(ui, &mut cmds);
                        }
                        let curated: Vec<_> = col.inspector.iter().filter(|(title,_)|title=="Actions").flat_map(|(_,rows)| rows).collect();
                        ui.horizontal_wrapped(|ui| {
                            if let Some((_,acts)) = selection {
                                for (id,label,_,enabled,_) in acts.iter().filter(|a| matches!(a.0.as_str(),"file"|"file_to"|"hide"|"edit"|"linkopen")) {
                                    if *enabled && kit::quiet(ui,label).clicked() { cmds.push(format!("cmd:{id}")); }
                                }
                            }
                            if selection.is_some() || !curated.is_empty() {
                                egui::ComboBox::from_id_salt("inspector actions").selected_text("Actions").show_ui(ui, |ui| {
                                    for (id,label) in &curated {
                                        if ui.selectable_label(false,label.as_str()).clicked() {
                                            if let Some(key) = id.strip_prefix("kindmenu:") { col.kind_menu = Some((key.into(),ui.cursor().min)); }
                                            else { actions.push(format!("insp:{id}")); }
                                        }
                                    }
                                    if let Some((_,acts)) = selection {
                                        for (id,label,_,enabled,_) in acts {
                                            if ui.add_enabled(*enabled,egui::Button::new(label.as_str()).frame(false)).clicked() {
                                                cmds.push(format!("cmd:{id}"));
                                            }
                                        }
                                    }
                                });
                            }
                        });
                        ui.add_space(SPACE_XS);
                        let native_export = self.panels.has_records() && col.mode.is_empty();
                        if native_export {
                            ui.collapsing("Export details", |ui| self.panels.export(ui, &mut cmds));
                        }
                        let sections = col.inspector.clone();
                        for (title, rows) in &sections {
                            if title == "Actions" { continue; }
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
                ("-", ""), ("hideitem", "Hide"), ("lockitem", "Lock / unlock"), ("zoom", "Zoom to"), ("-", ""), ("fileto", "File to…")];
            let comp_menu: &[(&str, &str)] = &[("cplace", "Place a copy"), ("cselect", "Select instances"),
                ("creplace", "Replace selection with this"), ("-", ""), ("ckeep", "Keep in file"), ("crename", "Rename…")];
            // A linked file's rows are read-only here: no filing, kind or rename.
            let link_menu: &[(&str, &str)] = &[("node", "Select"), ("contents", "Select contents"),
                ("isolate", "Isolate"), ("zoom", "Zoom to"), ("-", ""), ("eye", "Show / hide"), ("lock", "Lock / unlock"),
                ("-", ""), ("linkopen", "Edit source"), ("linkfinder", "Show in Finder"),
                ("linkreload", "Reload"), ("linkrelink", "Locate…"), ("linkunlink", "Unlink")];
            let section_menu: &[(&str, &str)] = &[("sectioncut", "Toggle cut"), ("eye", "Show / hide"), ("deleterows", "Delete")];
            let tag_menu: &[(&str, &str)] = &[("newtag", "New tag"), ("rename", "Rename"), ("tagactive", "Make active"), ("tagassign", "Move selection here"), ("tagselect", "Select items"), ("tagisolate", "Toggle tag isolation"), ("mergetag", "Merge into…"), ("scenetaghide", "Hide in all saved scenes"), ("scenetagshow", "Show in all saved scenes"), ("deletetag", "Delete"), ("purgetags", "Purge unused tags…")];
            let menu = if key.starts_with("preview-tag:") { tag_menu } else if key.starts_with("preview-folder:") { &[("newtag", "New tag in folder"), ("tagisolate", "Toggle tag isolation")] } else if key.starts_with("section:") { section_menu } else if row == 0 && key.starts_with("n:link") { link_menu } else if row == 0 { node_menu }
                else if row == 5 { comp_menu } else { item_menu };
            let resp = egui::Area::new(egui::Id::new("row_menu"))
                .fixed_pos(at + vec2(4.0, 4.0))
                .order(egui::Order::Tooltip)
                .show(ctx, |ui| {
                    glass(Surface::Raised).inner_margin(Margin::same(6)).show(ui, |ui| {
                        ui.set_width(200.0);
                        ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                        let row = col.nodes.iter().find(|n| n.key == key);
                        let editable_file = row.is_some_and(|n| n.icon.starts_with("file:") && !n.badge.starts_with("via "));
                        for (id, label) in menu {
                            if matches!(*id, "rename" | "deletetag" | "mergetag" | "tagisolate") && key == "preview-tag:Layer 0" { continue; }
                            if row.is_some_and(|n| n.icon == "file:missing") &&
                                matches!(*id, "contents" | "isolate" | "zoom" | "linkopen" | "linkfinder" | "linkreload") { continue; }
                            if matches!(*id, "linkreload" | "linkrelink" | "linkunlink") &&
                                !editable_file { continue; }
                            if *id == "hideitem" {
                                for (verb, label, why) in &col.group_rows {
                                    let item = (verb.clone(), label.clone(), String::new(), String::new(), -1, why.is_empty());
                                    let response = menu_row_resp(ui, &item).0;
                                    if why.is_empty() {
                                        if response.clicked() {
                                            actions.push(format!("{verb}\u{1f}{key}"));
                                            close = true;
                                        }
                                    } else {
                                        response.on_hover_text(why);
                                    }
                                }
                            }
                            if *id == "-" {
                                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 7.0), Sense::hover());
                                ui.painter().hline(r.x_range().shrink(6.0), r.center().y, Stroke::new(1.0_f32, EDGE));
                                continue;
                            }
                            let multi_rename = *id == "rename" && row.is_some_and(|n| n.local_placement && n.selected > 0)
                                && col.browser_picks > 1;
                            let label = if multi_rename { "Rename selected…" } else { *label };
                            let item = (id.to_string(), label.to_string(), String::new(), String::new(), -1i8, true);
                            let (r, rect) = menu_row_resp(ui, &item);
                            if r.clicked() {
                                match *id {
                                    "rename" if multi_rename => actions.push(col.rename_request(true)),
                                    "rename" => {
                                        let name = col.nodes.iter().find(|n| n.key == key).map(|n| n.name.clone()).unwrap_or_default();
                                        col.renaming = Some((key.clone(), name));
                                        col.focus_rename = true;
                                    }
                                    "tagisolate" if row.is_some() => actions.push(col.styles.isolate_action(row.unwrap())),
                                    "lockitem" if row.is_some_and(|n| n.local_placement) => actions.push(format!("rowlock\u{1f}{key}")),
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
        if let Some(at) = col.head_menu {
            let mut close = false;
            egui::Area::new(egui::Id::new("head_menu"))
                .pivot(egui::Align2::RIGHT_TOP)
                .fixed_pos(at)
                .order(egui::Order::Tooltip)
                .show(ctx, |ui| {
                    glass(Surface::Raised).inner_margin(Margin::same(6)).show(ui, |ui| {
                        ui.set_width(220.0);
                        egui::ScrollArea::vertical().id_salt("project_header_actions")
                            .max_height((screen.max.y - at.y - GUT * 2.).max(80.0)).show(ui, |ui| {
                            ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                            if col.mode.is_empty() {
                                if (col.lens == Lens::Filing || col.lens == Lens::Drawing && col.proposal_available) &&
                                    tree_filter::controls(ui, &mut col.filter, col.lens == Lens::Drawing) {
                                    actions.push(format!("filter\u{1f}{}", col.filter));
                                }
                                if col.lens == Lens::Drawing && col.proposal_available {
                                    if menu_row(ui, &("newtag".into(), "New tag".into(), String::new(), String::new(), -1, true)) {
                                        actions.push(format!("newtag\u{1f}{}", col.styles.creation_row()));
                                        close = true;
                                    }
                                    let hidden = col.styles.catalog.new_hidden;
                                    if menu_row(ui, &("newtaghidden".into(), "New tags hidden in saved scenes".into(), String::new(), String::new(), i8::from(hidden), true)) {
                                        col.styles.catalog.new_hidden = !hidden;
                                        actions.push(format!("newtaghidden\u{1f}{}", u8::from(!hidden)));
                                        close = true;
                                    }
                                }
                                if col.lens == Lens::Drawing {
                                    for (id, label, check) in [
                                        ("tagcolours", "Colour by tag", Some(col.styles.catalog.color_by_layer)),
                                        ("modellines", "Line types in the model", Some(col.styles.catalog.model_linetypes)),
                                        ("exporttags", "Export layer template…", None),
                                        ("importtags", "Import layer template…", None),
                                    ] {
                                        if menu_row(ui, &(id.into(), label.into(), String::new(), String::new(), check.map_or(-1, i8::from), true)) {
                                            let value = check.map_or("", |on| if on { "0" } else { "1" });
                                            actions.push(format!("{id}\u{1f}{value}"));
                                            close = true;
                                        }
                                    }
                                }
                                if col.styles.catalog.tag_isolation && menu_row(ui, &("endtagisolate".into(), "End tag isolation".into(), String::new(), String::new(), -1, true)) {
                                    actions.push("isolatetags\u{1f}[]".into());
                                    close = true;
                                }
                                if col.lens == Lens::Drawing && menu_row(ui, &("purgetags".into(), "Purge unused tags…".into(), String::new(), String::new(), -1, true)) {
                                    actions.push("purgetags\u{1f}".into());
                                    close = true;
                                }
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
                            if col.mode.is_empty() && col.lens == Lens::Filing {
                                let picked = col.browser_picks > 0;
                                let enabled = picked || col.unfiled > 0 || col.nodes.iter().any(|n| n.local_placement || n.key.starts_with("n:") && n.count > 0);
                                let label = if picked { "Rename selected…" } else if col.filter.is_empty() { "Rename model rows…" } else { "Rename matching rows…" };
                                if menu_row(ui, &("batchrename".into(), label.into(), String::new(), String::new(), -1, enabled)) {
                                    actions.push(col.rename_request(picked));
                                    close = true;
                                }
                            }
                            if col.mode.is_empty() {
                                for (id, label) in [("linkselection", "Link selection from open file…"), ("linkregistry", "Move registry to project…")] {
                                    let item = (id.to_owned(), label.to_owned(), String::new(), String::new(), -1, true);
                                    if menu_row(ui, &item) {
                                        actions.push(format!("{id}\u{1f}"));
                                        close = true;
                                    }
                                }
                                let shortcut = if cfg!(target_os = "macos") { "⇧⌘J" } else { "Ctrl+Shift+J" };
                                let item = ("revealselection".into(), "Reveal selection".into(), shortcut.into(), String::new(), -1, selection.is_some());
                                if menu_row(ui, &item) {
                                    actions.push("revealselection\u{1f}".into());
                                    close = true;
                                }
                            }
                            if self.plugins.available() && menu_row(ui,&("plugins".into(),"Plugins".into(),String::new(),String::new(),-1,true)) {
                                self.plugins.showing = !self.plugins.showing; close = true;
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
                .constrain(false)
                .interactable(retract == 0.0)
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

    /// The value field on the geometry it measures (Maxence 2026-11-22),
    /// and the committed value read back on the result after Enter until
    /// the next action (B1). Both stay inside the free area.
    fn cursor_value(&self, ctx: &egui::Context, screen: Rect) {
        let left = if self.review { GUT } else { GUT + self.rail_w + GUT };
        let right = if self.review { screen.max.x - GUT } else { self.free_right(ctx, screen) - GUT };
        let bottom = if self.review { screen.max.y - GUT } else { screen.max.y - BOT_H - GUT };
        let free = Rect::from_min_max(pos2(left, TOP_H + GUT), pos2(right, bottom));
        let (back, live) = self.cursor.place(free, vec2(170.0, 30.0));
        let pill = |id: &str, r: Rect, caption: &str, value: &str, typing: bool, committed: bool| {
            egui::Area::new(egui::Id::new(id))
                .fixed_pos(r.min)
                .order(egui::Order::Tooltip)
                .interactable(false)
                .show(ctx, |ui| value_pill(ui, caption, value, typing, committed));
        };
        if let (Some(r), Some(b)) = (back, &self.cursor.readback) {
            pill("value_readback", r, &b.caption, &b.value, false, true);
        }
        if let (Some(r), Some(l)) = (live, &self.cursor.live) {
            if screen.contains(l.at) {
                pill("cursor_value", r, &l.caption, &l.value, l.typing, false);
            }
        }
    }

    /// Lay out, tessellate and draw the chrome over ``target`` (already
    /// holding the frame). Called last in the frame, after the overlay.
    pub fn paint(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, e: &mut wgpu::CommandEncoder,
                 frame: &wgpu::Texture, target: &wgpu::TextureView, size: [u32; 2], scale: f32) {
        // The glass behind the chrome: blur the finished model image first.
        let sheet_up = self.lsheet.open;
        let begin = Instant::now();
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(size[0] as f32 / scale, size[1] as f32 / scale));
        self.screen = screen;
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
            self.panels.popup_layers.clear();
            if !sheet_up && !self.sheets.shown {
                let painter = ctx.layer_painter(egui::LayerId::background());
                let uv = Rect::from_min_max(pos2(0.,0.),pos2(1.,1.));
                for &(id, rect, active, ref label) in &self.model_panes {
                    painter.image(id, rect, uv, BRIGHT);
                    let label = painter.layout_no_wrap(label.clone(), caption_font(), INK);
                    let at = rect.min + vec2(8., 6.);
                    let tag = Rect::from_min_size(at, label.size() + vec2(10., 4.));
                    let clipped = painter.with_clip_rect(rect);
                    clipped.rect_filled(tag, R_INSET, SHEET);
                    clipped.galley(at + vec2(5., 2.), label, INK);
                    painter.rect_stroke(rect, 0., Stroke::new(1., if active { ACCENT } else { EDGE }),
                                        egui::StrokeKind::Inside);
                }
                for &[x0,y0,x1,y1] in &self.model_guides {
                    painter.line_segment([pos2(x0,y0),pos2(x1,y1)], Stroke::new(2., ACCENT));
                }
            }
            self.top_bar(ctx, screen);
            let retract = ctx.animate_bool_with_time(egui::Id::new("retract_model_chrome"),
                                                      self.review || sheet_up, large_sheet::ANIM_S);
            if self.sheets.shown {
                let free = self.page_free(ctx, screen);
                if self.page.tool != self.sheets.active {
                    self.page.set_tool(self.sheets.active.clone());
                }
                self.page.paint(ctx, screen, free);
                self.free_after_frame.append(&mut self.page.freed);
                self.sheets_frame(ctx, screen, retract);
            } else {
                if retract < 1.0 {
                    self.palette(ctx, screen, retract);
                    self.right_column(ctx, screen, retract);
                    self.bottom_bar(ctx, screen, retract);
                }
                if !self.review && !sheet_up {
                    self.plugins.utility(ctx, screen, self.dock_w, &mut self.actions);
                    self.paint_strip(ctx, screen);
                    if self.panels.data["terrain"].is_object() {
                        let bounds = Rect::from_min_max(pos2(12.,0.),pos2(self.free_right(ctx,screen)-8.,screen.bottom()-BOT_H-4.));
                        self.panels.terrain.draw(ctx,bounds,&self.panels.data["terrain"],&self.panels.data["projected"],&mut self.actions);
                    }
                    self.panels.frame_guide(ctx, screen);
                }
                if !sheet_up { self.cursor_value(ctx, screen); }
            }
            self.bar_menu(ctx);
            self.view_popup(ctx);
            self.form_popup(ctx);
            let acts = self.strip.popups(device, &mut self.painter, ctx, &self.icons);
            self.actions.extend(acts);
            if !sheet_up { self.context_menu(ctx, screen); }
            self.large_sheet(ctx, screen);
            self.command_palette(ctx, screen);
            self.dialog_popup(ctx, screen);
            if !sheet_up || !self.jobs.is_empty() { self.jobs_chip(ctx, screen); }
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
        let work = !self.review && !sheet_up;
        // The model's own areas are not drawn in the Sheets workspace: their
        // last rects must not take the pointer there.
        let model = !self.sheets.shown && !sheet_up;
        let shown = [("top", true), ("palette", work && model && !self.groups.is_empty()),
                     ("view_menu", self.view_open), ("menu", self.menu.is_some()), ("form", self.form.is_some()),
                     ("plugin_utility", work && model && self.plugins.utility_open()), ("column", work && model && self.column.is_some()), ("bottom", work),
                     ("paint_strip", work && model && self.paint_strip_shown()),
                     ("commands", self.cmd_open),
                     ("row_menu", model && self.column.as_ref().is_some_and(|c| c.row_menu.is_some())),
                     ("kind_menu", model && self.column.as_ref().is_some_and(|c| c.kind_menu.is_some())),
                     ("head_menu", model && self.column.as_ref().is_some_and(|c| c.head_menu.is_some())),
                     ("prompt", model && self.column.as_ref().is_some_and(|c| c.prompt.is_some())),
                     ("terrain_profile", work && model && self.panels.data["terrain"].is_object()),
                     ("jobs", !self.jobs.is_empty() || (!sheet_up && !self.history.is_empty())),
                     ("dialog", self.dialog.is_some()),
                     ("tab_menu", self.strip.menu_open()),
                     ("tab_overflow", self.strip.overflow_open()),
                     ("ctx_menu", self.ctx_menu.is_some()),
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
        let preview = self.lsheet.spec.as_ref().filter(|s| s.id == "start" && !s.closable)
            .and_then(|spec| self.icons.get(&spec.backdrop_icon))
            .and_then(|handle| self.painter.texture(&handle.id())?.texture.clone().map(|tex| (handle.id(), tex)));
        if let Some(res) = self.painter.callback_resources.get_mut::<crate::glass::Resources>() {
            self.strip.painted_doc = if preview.is_none() && !self.sheets.shown {
                self.strip.tabs.iter().find(|t| t.2).map(|t| t.0.clone())
            } else { None };
            self.glass.prepare(device, queue, e, frame, res, if self.lsheet.spec.is_some() { SHEET_BLUR } else { 1.0 }, preview);
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
        self.model_panes.clear();
        self.model_guides.clear();
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
    item_response(ui, item, icon).clicked()
}

fn item_response(ui: &mut egui::Ui, item: &Item, icon: Option<&egui::TextureHandle>) -> egui::Response {
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
    resp.on_hover_text(tip)
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
        if let Some(data) = value.strip_prefix("\u{1d}tagpicker\u{1d}") {
            if let Ok((mut at, options)) = serde_json::from_str::<(usize, Vec<(String, String)>)>(data) {
                let before = at;
                kit::property_control(ui, label, |ui| {
                    let labels: Vec<_> = options.iter().map(|(_, text)| text.as_str()).collect();
                    kit::choice(ui, "selection tag", &mut at, &labels);
                });
                if at != before && !options[at].0.is_empty() {
                    actions.push(format!("tagpick\u{1f}{}", serde_json::json!(&options[at].0)));
                }
            }
            continue;
        }
        if let Some((id, data)) = value.strip_prefix("\u{1d}picker\u{1d}").and_then(|rest| rest.split_once('\u{1d}')) {
            if let Ok((original, options)) = serde_json::from_str::<(usize, Vec<(String, String)>)>(data) {
                let mut at = original;
                let labels: Vec<_> = options.iter().map(|(_, text)| text.as_str()).collect();
                kit::property_control(ui, label, |ui| kit::choice(ui, id, &mut at, &labels));
                if at != original {
                    if let Some((key, _)) = options.get(at) {
                        actions.push(format!("edit\u{1f}{id}\u{1f}{key}"));
                    }
                }
            }
            continue;
        }
        if let Some(hex) = value.strip_prefix("\u{1d}swatch\u{1d}") {
            if let Some(color) = parse_hex(hex) { kit::material_readout(ui, label, color, hex); }
            continue;
        }
        if let Some(rest) = value.strip_prefix("\u{1d}slider\u{1d}") {
            let mut parts = rest.split('\u{1d}');
            let id = parts.next().unwrap_or_default();
            let committed = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0.);
            let max = parts.next().and_then(|s| s.parse().ok()).unwrap_or(100.);
            ui.horizontal(|ui| {
                ui.add_space(SPACE_S);
                let (r, _) = ui.allocate_exact_size(vec2(88., CONTROL_H), Sense::hover());
                ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, label, meta(), DIM);
                ui.spacing_mut().slider_width = (ui.available_width() - 60.).max(30.);
                if let Some(v) = kit::percent_slider(ui, ui.make_persistent_id(id), committed, max) {
                    actions.push(format!("edit\u{1f}{id}\u{1f}{v}"));
                }
            });
            continue;
        }
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
                let r = ui.add(kit::field(buf).id(egui::Id::new(("inspector_edit", id))).font(body()).margin(Margin::symmetric(6, 3))
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
    let (r, _) = ui.allocate_exact_size(vec2(SPACE_M, BUTTON_H), Sense::hover());
    ui.painter().vline(r.center().x, r.y_range().shrink(5.), Stroke::new(1.0_f32, OFF));
}

/// A text command in a bar, 28 high, 8 pt padding.
fn text_button(ui: &mut egui::Ui, label: &str, key: &str, enabled: bool) -> bool {
    text_button_response(ui, label, key, enabled).clicked()
}

fn text_button_response(ui: &mut egui::Ui, label: &str, key: &str, enabled: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), body(), INK);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 24.0, 28.0), sense);
    let st = state_of(&resp, false, enabled);
    paint_state(ui, rect, st, R_CONTROL);
    let ink = if enabled { INK } else { OFF };
    ui.painter().galley(pos2(rect.min.x + 12.0, rect.center().y - galley.size().y / 2.0), galley, ink);
    if key.is_empty() { resp } else { resp.on_hover_text(key) }
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

#[cfg(test)]
fn test_chrome() -> Option<Ui> {
    let instance = crate::backend::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    let (device, _) = pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
    Some(Ui::new(&device, wgpu::TextureFormat::Bgra8UnormSrgb))
}

#[test]
fn scene_chooser_owns_modified_arrows_and_moves_within_one_folder() {
    let Some(mut ui) = test_chrome() else { return };
    let row = |name: &str, folder: &str, marked: i8| {
        (name.into(), name.into(), String::new(), folder.into(), marked, true)
    };
    ui.tabs = vec![vec![], vec![row("A","I",1), row("B","I",0), row("C","II",0)]];
    assert!(!ui.claims_key("ArrowUp", 1));
    ui.menu = Some(("scenes".into(), Rect::ZERO));
    assert!(ui.claims_key("ArrowUp", 1));
    assert!(!ui.claims_key("ArrowUp", 0));
    assert!(ui.key("ArrowUp", true, 1));
    assert!(ui.take_actions().is_empty());
    assert!(ui.key("ArrowDown", true, 1));
    assert!(ui.key("ArrowDown", false, 1));
    assert_eq!(ui.take_actions(), ["tab:scene:down"]);
    ui.tabs[1][0].4 = 0;
    ui.tabs[1][1].4 = 1;
    assert!(ui.key("ArrowDown", true, 1));
    assert!(ui.take_actions().is_empty());
    assert!(ui.key("ArrowUp", true, 1));
    assert_eq!(ui.take_actions(), ["tab:scene:up"]);
}

#[test]
fn native_input_owners_unwind_in_order() {
    let Some(mut ui) = test_chrome() else { return };
    ui.set_tree(vec![], 0);
    ui.set_lens(2);
    ui.review = true;
    ui.view_open = true;
    assert!(ui.key("Escape", true, 0));
    assert!(!ui.view_open);
    let filter = egui::Id::new("dock_filter");
    ui.ctx.memory_mut(|m| m.request_focus(filter));
    ui.column.as_mut().unwrap().filter = "Box".into();
    assert!(ui.key("Escape", true, 0));
    assert_eq!(ui.take_actions(), ["tree:filter\u{1f}"]);
    assert!(ui.ctx.memory(|m| m.has_focus(filter)));
    assert!(ui.key("Escape", true, 0));
    assert!(!ui.ctx.memory(|m| m.has_focus(filter)));
    assert_eq!(Ui::model_escape(Some(&mut ui), true, false, true, true), "tool");
    assert_eq!(Ui::model_escape(Some(&mut ui), false, false, true, true), "selection");
    assert_eq!(Ui::model_escape(Some(&mut ui), false, false, false, true), "edit");
    assert_eq!(Ui::model_escape(Some(&mut ui), false, false, false, false), "dock");
    assert_eq!(ui.take_actions(), ["tree:lens\u{1f}0"]);
    assert!(ui.review);
    Ui::model_escape(Some(&mut ui), false, false, false, false);
    assert!(!ui.review);

}

#[test]
fn paint_always_offers_browse_all_even_without_materials() {
    let Some(mut ui) = test_chrome() else { return };
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280., 800.));
    let strip = |ui: &mut Ui, events: Vec<egui::Event>| {
        let ctx = ui.ctx.clone();
        let raw = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let _ = ctx.run(raw, |c| ui.paint_strip(c, screen));
        ctx.memory(|m| m.area_rect(egui::Id::new("paint_strip")))
    };
    assert!(ui.palette_items.is_empty());
    ui.set_active("select".into());
    assert!(!ui.paint_strip_shown());
    ui.set_active("paint".into());
    for _ in 0..3 { let _ = strip(&mut ui, vec![]); }
    let r = strip(&mut ui, vec![]).expect("the strip shows with Paint active and no materials");
    assert!(r.max.y < 800. - BOT_H, "{r:?}");
    // Clicking Browse all…, the strip's last control, opens the Library's
    // materials. (Tests have no fonts: the button is its 24 pt padding.)
    let at = pos2(r.max.x - 16., r.center().y);
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let _ = strip(&mut ui, vec![egui::Event::PointerMoved(at)]);
    let _ = strip(&mut ui, vec![button(true)]);
    let _ = strip(&mut ui, vec![button(false)]);
    assert!(ui.take_actions().contains(&"libsheet:materials".to_owned()));
}

#[test]
fn the_free_area_leaves_out_the_panels_and_review_frees_them() {
    let Some(mut ui) = test_chrome() else { return };
    assert_eq!(ui.free_ndc(), None);                          // before a frame
    ui.screen = Rect::from_min_size(Pos2::ZERO, vec2(1280., 800.));
    ui.groups = vec![vec![Tool { enabled: true, id: "select".into(), label: String::new(), key: String::new(), icon: None }]];
    ui.rail_w = rail::width(3);
    ui.set_tree(vec![], 0);
    ui.set_dock_width(320.);
    let [x0, y0, x1, y1] = ui.free_ndc().unwrap();
    let px = |v: f64| (v + 1.) / 2. * 1280.;
    let py = |v: f64| (1. - v) / 2. * 800.;
    assert!((px(x0) - f64::from(GUT + rail::width(3))).abs() < 0.01);
    assert!((px(x1) - f64::from(1280. - GUT - 320.)).abs() < 0.01);
    assert!((py(y1) - f64::from(TOP_H)).abs() < 0.01 && (py(y0) - f64::from(800. - BOT_H)).abs() < 0.01);
    // Review retracts the rail, dock and bottom bar: the free area grows at
    // once (not with the animation) and Python hears it to reframe.
    ui.set_review(true);
    assert_eq!(ui.take_actions(), ["review:1"]);
    let [x0, y0, x1, _] = ui.free_ndc().unwrap();
    assert_eq!((x0, y0, x1), (-1., -1., 1.));
    assert_eq!(Ui::model_escape(Some(&mut ui), false, false, false, false), "");
    assert_eq!(ui.take_actions(), ["review:0"]);
}

#[test]
fn dismissal_consumes_press_drag_and_release() {
    let Some(mut ui) = test_chrome() else { return };
    ui.view_open = true;
    assert!(ui.pointer("press", 300., 400., 1, 0));
    assert!(!ui.view_open);
    assert!(ui.pointer("move", 320., 420., 1, 0));
    assert!(ui.pointer("release", 320., 420., 1, 0));
    assert!(!ui.pointer("press", 300., 400., 1, 0));
    assert!(!ui.pointer("release", 300., 400., 1, 0));
}

#[cfg(test)]
fn tree_frame(ui: &mut Ui, events: Vec<egui::Event>) -> std::collections::HashMap<String, Pos2> {
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1024., 700.));
    let ctx = ui.ctx.clone();
    let raw = egui::RawInput {
        screen_rect: Some(screen),
        modifiers: events.iter().find_map(|ev| match ev {
            egui::Event::PointerButton { modifiers, .. } => Some(*modifiers),
            _ => None,
        }).unwrap_or_default(),
        events,
        ..Default::default()
    };
    let frame = ctx.run(raw, |c| ui.right_column(c, screen, 0.));
    frame
        .shapes
        .into_iter()
        .filter_map(|shape| {
            if let egui::Shape::Text(text) = shape.shape {
                let at = text.visual_bounding_rect().center();
                if shape.clip_rect.contains(at) {
                    return Some((text.galley.text().to_owned(), at));
                }
            }
            None
        })
        .collect::<std::collections::HashMap<_, _>>()
}

#[cfg(test)]
fn tree_click(ui: &mut Ui, pos: Pos2, button: egui::PointerButton) {
    tree_frame(ui, vec![egui::Event::PointerMoved(pos)]);
    for pressed in [true, false] {
        tree_frame(
            ui,
            vec![egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: Default::default(),
            }],
        );
    }
}

#[test]
fn occurrence_menu_runs_the_document_action_and_closes() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    let key = "i:box";
    ui.set_tree(
        vec![(vec![key.into(), "Box".into()], vec![0, 0, 0, 0, 1, -1, -1])],
        0,
    );
    for _ in 0..3 {
        tree_frame(&mut ui, vec![]);
    }
    let at = tree_frame(&mut ui, vec![])["Box"];
    tree_click(&mut ui, at, egui::PointerButton::Secondary);
    assert!(ui
        .take_actions()
        .contains(&format!("tree:grouprows\u{1f}{key}")));
    ui.set_tree_group_rows(
        key.into(),
        vec![("explode".into(), "Explode Group".into(), String::new())],
    );
    for _ in 0..3 {
        tree_frame(&mut ui, vec![]);
    }
    let at = tree_frame(&mut ui, vec![])["Explode Group"];
    tree_click(&mut ui, at, egui::PointerButton::Primary);
    assert!(ui
        .take_actions()
        .contains(&format!("tree:explode\u{1f}{key}")));
    assert!(ui.column.unwrap().row_menu.is_none());
}

#[test]
fn only_eye_and_lock_headers_dispatch_and_each_targets_its_lens() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    ui.set_tree(vec![(vec!["i:sample".into(), "Sample".into()], vec![0, 0, 0, 0, 1, 1, 0])], 0);
    for (lens, scope) in [(Lens::Filing, "placements"), (Lens::Drawing, "tags")] {
        ui.column.as_mut().unwrap().lens = lens;
        for _ in 0..3 { tree_frame(&mut ui, vec![]); }
        let at = tree_frame(&mut ui, vec![]);
        tree_click(&mut ui, at["Name"], egui::PointerButton::Primary);
        assert!(ui.take_actions().is_empty());
        for field in ["eye", "lock"] {
            let cell = ui.ctx.read_response(egui::Id::new(("project header", field))).unwrap();
            tree_click(&mut ui, cell.rect.center(), egui::PointerButton::Primary);
            assert_eq!(ui.take_actions(), [format!("tree:header{field}\x1f{scope}")]);
        }
    }
}

#[cfg(test)]
fn tree_drag(ui: &mut Ui, from: Pos2, to: Pos2) -> Vec<String> {
    let button = |pos, pressed| egui::Event::PointerButton {
        pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default()
    };
    tree_frame(ui, vec![egui::Event::PointerMoved(from), button(from, true)]);
    tree_frame(ui, vec![egui::Event::PointerMoved(from + vec2(8., 0.))]);
    tree_frame(ui, vec![egui::Event::PointerMoved(to)]);
    tree_frame(ui, vec![button(to, false)]);
    ui.take_actions()
}

#[test]
fn dragging_sorted_drawing_rows_files_tags_and_folders_by_key() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    let row = |key: &str, name: &str, depth: i64, folder: bool| {
        let mut text = vec![String::new(); if folder { 5 } else { 12 }];
        text[0] = key.into();
        text[1] = name.into();
        if folder { text[4] = "folder".into(); }
        else { text[10] = key.strip_prefix("preview-tag:").unwrap().into(); }
        (text, vec![depth, 0, 0, 0, 6, 1, 0, i64::from(folder), 1])
    };
    ui.set_tree(vec![row("preview-folder:f","Plans",0,true),
                     row("preview-tag:inside","Section",1,false),
                     row("preview-folder:g","Elevations",0,true),
                     row("preview-tag:z","Brick",0,false),
                     row("preview-tag:a","Steel",0,false)],0);
    ui.set_tree_sort(1);
    ui.column.as_mut().unwrap().lens = Lens::Drawing;
    for _ in 0..3 { tree_frame(&mut ui, vec![]); }
    let at = tree_frame(&mut ui, vec![]);
    for (source, dest, tags, folders, parent) in [
        ("Brick", "Section", vec!["z"], vec![], "f"),
        ("Plans", "Elevations", vec![], vec!["f"], "g"),
        ("Section", "Name", vec!["inside"], vec![], ""),
    ] {
        let actions = tree_drag(&mut ui, at[source], at[dest]);
        let expected = serde_json::json!({"owner":0,"tags":tags,"folders":folders,"parent":parent});
        assert!(actions.contains(&format!("tree:tagfolder\u{1f}batch\u{1f}{expected}")), "{source}: {actions:?}");
    }
    tree_click(&mut ui, at["Brick"], egui::PointerButton::Primary);
    for name in ["Steel", "Plans", "Steel", "Steel"] {
        let pos = at[name];
        tree_frame(&mut ui, vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            tree_frame(&mut ui, vec![egui::Event::PointerButton {
                pos, pressed, button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::SHIFT,
            }]);
        }
    }
    let actions = tree_drag(&mut ui, at["Brick"], at["Elevations"]);
    let expected = serde_json::json!({"owner":0,"tags":["a","z"],"folders":["f"],"parent":"g"});
    assert!(actions.contains(&format!("tree:tagfolder\u{1f}batch\u{1f}{expected}")), "{actions:?}");
    tree_click(&mut ui, at["Brick"], egui::PointerButton::Primary);
    assert!(ui.key("Delete", true, 0));
    assert_eq!(ui.take_actions(), ["tree:deletetag\x1fpreview-tag:z"]);
    tree_click(&mut ui, at["Brick"], egui::PointerButton::Secondary);
    let menu = tree_frame(&mut ui, vec![]);
    tree_click(&mut ui, menu["Delete"], egui::PointerButton::Primary);
    assert_eq!(ui.take_actions(), ["tree:deletetag\x1fpreview-tag:z"]);


}

#[test]
fn new_drawing_tag_keeps_its_editor_through_refresh_and_commits_only_its_own_name() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    let row = || {
        let mut text = vec![String::new(); 12];
        text[0] = "preview-tag:Layer 1".into();
        text[1] = "Layer 1".into();
        text[10] = "Layer 1".into();
        (text, vec![0, 0, 0, 0, 6, 1, 0])
    };
    ui.set_tree(vec![row()], 0);
    ui.set_lens(1);
    for _ in 0..3 { tree_frame(&mut ui, vec![]); }
    for enter in [true, false] {
        ui.drawing_edit_applied("newtag", "Layer 1");
        tree_frame(&mut ui, vec![]);
        ui.set_tree(vec![row()], 0);
        let tool = ui.active.clone();
        assert!(ui.claims_key("L", 0));
        assert!(ui.key("L", true, 0) && ui.text("l".into()) && ui.key("L", false, 0));
        let events = std::mem::take(&mut ui.events);
        tree_frame(&mut ui, events);
        let col = ui.column.as_ref().unwrap();
        assert!(ui.ctx.wants_keyboard_input() && ui.active == tool);
        assert!(!col.styles.selected(&col.nodes[0]));
        assert!(col.renaming.as_ref().unwrap().1.ends_with('l'));
        if enter {
            tree_frame(&mut ui, vec![egui::Event::Key {
                key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false,
                modifiers: Default::default(),
            }]);
        } else {
            tree_click(&mut ui, pos2(100., 400.), egui::PointerButton::Primary);
        }
        let col = ui.column.as_ref().unwrap();
        assert!(col.renaming.is_none());
        assert_eq!(col.styles.selected(&col.nodes[0]), enter);
        assert!(ui.take_actions().iter().any(|a| a.starts_with("tree:rename\x1fpreview-tag:Layer 1\x1f")));
    }
}

#[test]
fn a_tree_refresh_keeps_the_selected_row_at_its_scroll_position() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    let rows = |count: usize| (0..count).map(|i| {
        let mut text = vec![String::new(); 12];
        text[0] = format!("preview-tag:{i}");
        text[1] = format!("Tag {i}");
        text[10] = i.to_string();
        (text, vec![0, 0, 0, 0, 6, 1, 0])
    }).collect();
    ui.set_tree(rows(100), 0);
    ui.column.as_mut().unwrap().lens = Lens::Drawing;
    for _ in 0..3 { tree_frame(&mut ui, vec![]); }
    ui.reveal_row("preview-tag:50".into());
    let at = tree_frame(&mut ui, vec![])["Tag 50"];
    tree_click(&mut ui, at, egui::PointerButton::Primary);
    let before = tree_frame(&mut ui, vec![])["Tag 50"];
    ui.set_tree(rows(101), 0);
    let after = tree_frame(&mut ui, vec![])["Tag 50"];
    let col = ui.column.as_ref().unwrap();
    assert!(col.styles.selected(&col.nodes[50]));
    assert!((before.y - after.y).abs() < 1., "{before:?} -> {after:?}");
}

#[test]
fn name_sort_uses_the_new_label_and_unknown_preferences_restore_project_order() {
    let Some(mut ui) = test_chrome() else { return };
    let groups = |a: &str| vec![
        (vec!["i:a".into(), a.into()], vec![0, 0, 0, 0, 1]),
        (vec!["i:b".into(), "Muro 2".into()], vec![0, 0, 0, 0, 1]),
    ];
    ui.set_tree(groups("Muro 1"), 0);
    ui.set_tree_sort(1);
    assert_eq!(ui.column.as_ref().unwrap().ordered, [0, 1]);
    ui.set_tree(groups("Muro 10"), 0);
    assert_eq!(ui.column.as_ref().unwrap().ordered, [1, 0]);
    ui.set_tree_sort(usize::MAX);
    assert_eq!(ui.column.as_ref().unwrap().ordered, [0, 1]);
}

#[test]
fn the_drop_band_files_beside_or_into_an_occurrence() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    ui.set_tree(["House", "Shed"].into_iter().map(|name|
        (vec![format!("i:{name}"), name.into()], vec![0, 0, 0, 0, 1, -1, -1])
    ).collect(), 0);
    for _ in 0..3 { tree_frame(&mut ui, vec![]); }
    let at = tree_frame(&mut ui, vec![]);
    for (offset, verb) in [(0.35, "into"), (-0.43, "beside")] {
        let to = at["House"] + vec2(0., ROW_H * offset);
        assert_eq!(tree_drag(&mut ui, at["Shed"], to), [format!("tree:{verb}\x1fi:Shed\x1fi:House")]);
    }
    assert_eq!(tree_drag(&mut ui, at["Shed"], at["Name"]), ["tree:root\x1fi:Shed"]);
}

#[test]
fn an_eye_gesture_never_opens_or_drags_its_occurrence() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    ui.set_tree(["House", "Shed"].into_iter().map(|name|
        (vec![format!("i:{name}"), name.into(), "".into(), "".into(), "group".into(), "Hidden by Building".into(), "Locked by Building".into()], vec![0, 0, 0, 0, 1, 2, 2])
    ).collect(), 0);
    let house = &ui.column.as_ref().unwrap().nodes[0];
    assert!(house.dim && house.cells.is_empty());
    assert_eq!(house.control_tips, ["Hidden by Building", "Locked by Building"]);
    for _ in 0..3 { tree_frame(&mut ui, vec![]); }
    let at = tree_frame(&mut ui, vec![]);
    let eye = pos2(at["Name"].x - 26., at["House"].y);
    for _ in 0..2 {
        tree_click(&mut ui, eye, egui::PointerButton::Primary);
        assert_eq!(ui.take_actions(), ["tree:roweye\x1fi:House"]);
    }
    let button = |pos, pressed| egui::Event::PointerButton {
        pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default(),
    };
    tree_frame(&mut ui, vec![button(eye, true)]);
    tree_frame(&mut ui, vec![egui::Event::PointerMoved(eye + vec2(10., 0.))]);
    tree_frame(&mut ui, vec![egui::Event::PointerMoved(at["Shed"])]);
    tree_frame(&mut ui, vec![button(at["Shed"], false)]);
    assert!(ui.take_actions().is_empty());
}

#[test]
fn focused_tree_keys_win_over_viewport_shortcuts() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    let group = |name: &str| {
        (
            vec![format!("i:{name}"), name.to_owned()],
            vec![0, 0, 0, 0, 1, -1, -1],
        )
    };
    ui.set_tree(vec![group("A"), group("B")], 0);
    for _ in 0..3 {
        tree_frame(&mut ui, vec![]);
    }
    let pos = tree_frame(&mut ui, vec![])["A"];
    tree_click(&mut ui, pos, egui::PointerButton::Primary);
    assert_eq!(ui.take_actions(), ["tree:pickrow\x1fi:A"]);
    assert!(!ui.claims_key("ArrowRight", 0)); // axis lock belongs to the model
    assert!(ui.claims_key("ArrowDown", 0));
    ui.set_review(true);
    assert!(ui.project_hidden() && !ui.claims_key("ArrowDown", 0));
    ui.set_review(false);
    assert!(ui.claims_key("ArrowDown", 0));
    ui.take_actions();
    assert!(!ui.claims_key("Z", 1));
    assert!(!ui.claims_key("Space", 4));
    for (key, mods, action) in [
        ("ArrowDown", 0, "pickrow\x1fi:B"),
        ("Tab", 0, "into\x1fi:B\x1fi:A"),
        ("Tab", 4, "outdent\x1fi:B"),
        ("Space", 0, "roweye\x1fi:B"),
        ("Delete", 0, "deleterows\x1fi:B"),
        ("Backspace", 0, "deleterows\x1fi:B"),
        ("Enter", 0, "open\x1fi:B"),
        ("Escape", 0, "stepout\x1f"),
    ] {
        assert!(ui.claims_key(key, mods), "{key}");
        assert!(ui.key(key, true, mods));
        assert_eq!(ui.take_actions(), [format!("tree:{action}")]);
    }
    assert!(ui.key("F2", true, 0));
    assert_eq!(
        ui.column.as_ref().unwrap().renaming.as_ref().unwrap().0,
        "i:B"
    );
    ui.pointer("press", 300., 400., 1, 0);
    assert!(ui.column.as_ref().unwrap().focused_row.is_none());
}

#[test]
fn drawing_lock_and_visibility_address_the_selected_tags() {
    let Some(mut ui) = test_chrome() else { return };
    let tag = |key: &str| {
        let mut text = vec![String::new(); 12];
        text[0] = format!("preview-tag:{key}");
        text[10] = key.into();
        (text, vec![0, 0, 0, 0, 0, 1, 0])
    };
    ui.set_tree(vec![tag("a"), tag("b"), tag("other")], 0);
    let col = ui.column.as_mut().unwrap();
    col.styles.pick(&col.nodes[0], false);
    col.styles.pick(&col.nodes[1], true);
    for (lock, field, expected) in [(true, "locked", true), (false, "visible", false)] {
        let action = col.styles.state_action(&col.nodes[1], lock).unwrap();
        let patch: serde_json::Value =
            serde_json::from_str(action.strip_prefix("tagstate\x1f").unwrap()).unwrap();
        assert_eq!(patch["tags"], serde_json::json!(["a", "b"]));
        assert_eq!(patch[field], expected);
        assert!(col.styles.state_action(&col.nodes[2], lock).is_none());
    }
}

#[test]
fn drawing_controls_own_press_drag_and_double_click_and_release_focus_to_model() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    let rows = |visible: bool| ["Walls", "Doors", "Other"].map(|name| {
        let mut text = vec![String::new(); 22];
        text[0] = format!("preview-tag:{name}");
        text[1] = name.into();
        text[10] = name.into();
        (text, vec![0, 0, 0, 0, 6, i64::from(visible), 0])
    }).to_vec();
    ui.set_tree(rows(true), 0);
    ui.set_lens(1);
    for _ in 0..3 { tree_frame(&mut ui, vec![]); }
    let col = ui.column.as_mut().unwrap();
    col.styles.pick(&col.nodes[0], false);
    col.styles.pick(&col.nodes[1], true);
    let at = tree_frame(&mut ui, vec![]);
    let eye = pos2(at["Name"].x - 26., at["Walls"].y);
    let set_visible = |value| format!("tree:tagstate\x1f{}", serde_json::json!({"tags":["Doors","Walls"], "visible":value}));
    assert_eq!(tree_drag(&mut ui, eye, at["Other"]), [set_visible(false)]);
    for visible in [false, true] {
        ui.set_tree(rows(visible), 0);
        tree_frame(&mut ui, vec![]);
        tree_click(&mut ui, eye, egui::PointerButton::Primary);
        assert_eq!(ui.take_actions(), [set_visible(!visible)]);
        let col = ui.column.as_ref().unwrap();
        assert!(col.styles.selected(&col.nodes[0]) && col.styles.selected(&col.nodes[1]));
        assert!(!col.styles.selected(&col.nodes[2]) && col.renaming.is_none());
    }
    ui.hit.push(Rect::from_center_size(eye, vec2(24., 24.)));
    assert!(ui.pointer("press", eye.x, eye.y, 1, 0));
    ui.pointer("release", eye.x, eye.y, 1, 0);
    assert!(ui.column.as_ref().unwrap().styles.active());
    assert!(!ui.pointer("press", 300., 400., 1, 0));
    assert!(!ui.column.as_ref().unwrap().styles.active());
}

#[test]
fn browser_rename_uses_focused_row_selection_and_header_scope() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    let rows = ["Box", "Door"].map(|name| (
        vec![format!("i:{name}"), name.into()],
        vec![0, 0, 0, 0, 1, -1, -1, 0, 0, 1],
    ));
    ui.set_tree(rows.to_vec(), 0);
    ui.set_browser_picks(2);
    let col = ui.column.as_mut().unwrap();
    col.focused_row = Some("i:Box".into());
    let selected = format!("tree:{}", col.rename_request(true));
    assert!(ui.key("F2", true, 0));
    assert_eq!(ui.take_actions(), [selected.clone()]);
    for _ in 0..3 { tree_frame(&mut ui, vec![]); }
    let at = tree_frame(&mut ui, vec![])["Box"];
    tree_click(&mut ui, at, egui::PointerButton::Secondary);
    ui.take_actions();
    for _ in 0..3 { tree_frame(&mut ui, vec![]); }
    let rename = tree_frame(&mut ui, vec![])["Rename selected…"];
    tree_click(&mut ui, rename, egui::PointerButton::Primary);
    assert_eq!(ui.take_actions(), [selected]);
    assert!(ui.rename_browser_row("Door"));
    assert_eq!(ui.column.as_mut().unwrap().renaming.take().unwrap().0, "i:Door");
    assert!(!ui.rename_browser_row("removed"));
    for (empty, filter, label) in [
        (false, "", "Rename model rows…"), (false, "Door", "Rename matching rows…"),
        (true, "", "Rename model rows…")
    ] {
        if empty { ui.set_tree(vec![], 0); }
        ui.set_browser_picks(0);
        let col = ui.column.as_mut().unwrap();
        col.filter = filter.into();
        col.head_menu = Some(pos2(680., 80.));
        let action = format!("tree:{}", col.rename_request(false));
        for _ in 0..3 { tree_frame(&mut ui, vec![]); }
        let p = tree_frame(&mut ui, vec![])[label];
        tree_click(&mut ui, p, egui::PointerButton::Primary);
        if empty { assert!(ui.take_actions().is_empty()); }
        else { assert_eq!(ui.take_actions(), [action]); }
    }
}
