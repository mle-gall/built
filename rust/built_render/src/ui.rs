//! Viewport chrome drawn by egui in the frame that draws the model (the
//! UI spike, `ui/egui-spike`). Python forwards pointer events and registers
//! the tools; clicks come back as action strings. Nothing here owns model
//! state: the palette shows what Python says is active.

use std::sync::Arc;
use std::time::Instant;

use egui::{pos2, vec2, Color32, CornerRadius, Margin, Pos2, Rect, Sense, Stroke, Vec2};

/// Small widgets shared by the dock and the forms (segmented, tabs).
/// The chrome's modal forms (rust/built_render/src/forms_host.rs).
pub mod dialog;
pub mod operation_form;
mod library_previews;
/// Sheets, a workspace of the window (sheet_workspace).
mod sheets;
mod sketch;
thread_local! {
    static LABELS: std::cell::RefCell<(String, std::collections::HashMap<String, String>)> = Default::default();
}

pub(crate) fn sync_language(py: pyo3::Python<'_>) -> pyo3::PyResult<()> {
    use pyo3::prelude::*;
    let module = py.import("built_core.i18n")?;
    let language: String = module.call_method0("current_language")?.extract()?;
    LABELS.with_borrow_mut(|(active, labels)| {
        if language != *active {
            *labels = module.call_method0("translations")?.extract()?;
            *active = language;
        }
        Ok(())
    })
}

pub(crate) fn tr(source: &str) -> String {
    LABELS.with_borrow(|(_, labels)| labels.get(source).cloned().unwrap_or_else(|| source.into()))
}

mod activity;
mod plugins;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DrawerMode { Fillet, Form, Pinned }

impl DrawerMode {
    fn resolve(form: bool, fillet: bool, pinned: bool) -> Option<Self> {
        if form { Some(Self::Form) } else if fillet { Some(Self::Fillet) }
        else if pinned { Some(Self::Pinned) } else { None }
    }
}

pub use activity::ActivityRow;
/// Project tabs in the top bar (views/wgpu_tabs.py).
mod tabs;
/// The viewport right-click menu (viewport_menu.rs, docs/context-menu.md).
mod context_menu;

mod lens;
use lens::{Col, Lens};
mod tree_order;
pub(crate) use tree_order::natural as natural_name_order;
mod tree_filter;
mod tree_keys;
mod tree_rows;
mod dock_menus;
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
mod view_overlay;
mod tasks;
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

#[derive(Clone, Debug, PartialEq, Eq)]
enum Destination {
    Project,
    Tasks,
    Deliveries,
    Plugin(String),
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
    coordinate: String,
    coordinate_tooltip: String,
    status: String,
    hint: String,
    /// The value field on the geometry and its readback (ui/value_field.rs).
    cursor: value_field::ValueField,
    /// The live value is the tool's refusal (Fillet: "Radius too large").
    pub(crate) section_inset: Option<Vec<Vec<[f32;2]>>>,
    pub(crate) cursor_error: bool,
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
    /// The junction review list (title, count, rows, ticked count).
    junction_review: Option<(String, String, Vec<crate::junction_review::Row>, usize)>,
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
    scene_controls_w: f32,
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
    activity_secondary: std::collections::HashMap<String, String>,
    hovered_toast: Option<String>,
    /// The keystroke overlay's pill (keycast.rs), shown with View ▸ Show Keystrokes.
    keycast: crate::keycast::Keycast,
    link_banner: String,
    /// The dock's width (points): dragged on its left edge, kept by Python.
    dock_w: f32,
    destination: Destination,
    /// The Tasks destination's composer, answers and pins (ui/tasks.rs).
    tasks: tasks::TaskPane,
    fix_rows: bool,
    fix_rule: Option<String>,
    /// The pointer shape the chrome asks for ("" or "resize_h").
    cursor_shape: &'static str,
    /// The Sheets workspace: while on, the page replaces the model (ui/sheets.rs).
    sheets: sheets::Sheets,
    /// The Sketch workspace: while on, a flat drawing space replaces the
    /// model (ui/sketch.rs).
    sketch: sketch::SketchWs,
    /// The page of the Sheets workspace (ui/page.rs).
    pub page: crate::sheet_page::Page,
    /// The project tabs (ui/tabs.rs): drawn instead of the document ▾
    /// once Python sends them.
    menu_hits: Vec<Rect>,
    strip: tabs::Strip,
    /// The right-click menu over the model (ui/context_menu.rs).
    ctx_menu: Option<context_menu::Menu>,
    /// The large sheet (ui/large_sheet.rs): over everything under the top bar.
    lsheet: large_sheet::LargeSheet,
    /// The axis gizmo and view label (ui/view_overlay.rs).
    gizmo: view_overlay::Overlay,
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
    tag_index: Option<std::collections::HashMap<String, usize>>,
    marked_rows: Vec<usize>,
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
    filter: String,
    /// "File to…": the next row clicked (or Enter) receives the selection.
    filing: Option<String>,
    focus_filter: bool,
    group_rows: Vec<(String, String, String)>,
    /// The row menu: row key, row type, where.
    row_menu: Option<(String, u8, Pos2)>,
    choosing_folder: bool,
    /// Inline rename: row key and the text being typed.
    renaming: Option<(String, String)>,
    focus_rename: bool,
    /// The open header menu (Columns, filter, delivery, more) and its anchor.
    head_menu: Option<(dock_menus::HeadMenu, Rect)>,
    kind_menu: Option<(String, Pos2)>,
    kinds: Vec<String>,
    /// A confirmation over the dock: title, body, (id, label) buttons.
    prompt: Option<(String, String, Vec<(String, String)>)>,
    /// The tree's columns: a preset (Filing, Drawing, Export) or the
    /// user's own set. They change the trailing cells and nothing else.
    columns: lens::Columns,
    /// How far the trailing cells are scrolled under the pinned name.
    cells_scroll: f32,
    /// Whether the document is asked for the export readiness (the
    /// Readiness column shows, or the Fix checklist is open), as last sent.
    readiness_sent: Option<bool>,
    /// The tag in focus came with a node row: its Drawing style is a
    /// section under the node's inspector, not the inspector itself.
    tag_from_node: bool,
    /// Legacy Library overlay state.
    mode: String,
    /// Issues shown (dots in the tree, counts in the header) or kept out of the way.
    issues: bool,
    /// The inspector's editable fields: id → text being typed.
    edits: std::collections::HashMap<String, String>,
    identity: Vec<(String, String, String, Option<bool>)>,
    inspector_title: String,
    inspector_subtitle: Option<String>,
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
            coordinate: String::new(),
            coordinate_tooltip: String::new(),
            status: String::new(),
            hint: String::new(),
            cursor: Default::default(),
            section_inset: None,
            cursor_error: false,
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
            junction_review: None,
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
            gizmo: Default::default(),
            top_groups: Vec::new(),
            cluster_buttons: Vec::new(),
            scene_controls_w: 250.,
            top_right_w: 0.0,
            units: "mm".into(),
            sheet: None,
            dialog: None,
            jobs: Vec::new(),
            history: Vec::new(),
            activity_open: false,
            activity_secondary: Default::default(),
            hovered_toast: None,
            keycast: Default::default(),
            link_banner: String::new(),
            dock_w: DOCK_W,
            destination: Destination::Project,
            tasks: Default::default(),
            fix_rows: false,
            fix_rule: None,
            plugins: Default::default(),
            plugin_document: String::new(),
            background_plugins: Default::default(),
            cursor_shape: "",
            sheets: Default::default(),
            sketch: Default::default(),
            page: Default::default(),
            menu_hits: Vec::new(), strip: Default::default(),
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
        let database = crate::image_assets::fonts();
        use resvg::usvg::fontdb::{Family, Query};
        let mono = database.query(&Query {
            families: &[Family::Name("Menlo"), Family::Name("Consolas"), Family::Name("DejaVu Sans Mono"), Family::Monospace],
            ..Default::default()
        });
        if let Some(face) = mono.and_then(|id| database.with_face_data(id, |bytes, index| {
            let mut face = egui::FontData::from_owned(bytes.to_vec());
            face.index = index;
            face
        })) {
            fonts.font_data.insert("transcript".into(), Arc::new(face));
            fonts.families.entry(egui::FontFamily::Monospace).or_default().insert(0, "transcript".into());
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
                            let image = if key.starts_with("component:") { egui::ColorImage::from_rgba_premultiplied([w,h],&rgba) } else { egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba) };
                            self.ctx.load_texture(&id, image, egui::TextureOptions::LINEAR)
                        });
                        Tool { enabled: !disabled.contains(&id), id, label, key, icon }
                    })
                    .collect()
            })
            .collect();
    }

    pub fn set_icons(&mut self, icons: Vec<(String, usize, usize, Vec<u8>)>) {
        if let Some(prefix) = icons.iter().filter(|(k,..)|k.starts_with("component:")).find_map(|(k,..)|k.rsplit_once(':').map(|(p,_)|p)) {
            self.icons.retain(|k,_| !k.starts_with("component:") || k.rsplit_once(':').is_some_and(|(p,_)|p==prefix));
        }
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
            tag_index: None, marked_rows: Vec::new(), nodes: Vec::new(), path: Vec::new(), host: Default::default(), overview: Default::default(), styles: Default::default(), reveal: None, proposal_available: false, proposal_dismissed: false, sort: Default::default(), ordered: Vec::new(), unfiled: 0, browser_picks: 0, issue_at: None, filter: String::new(),
            filing: None, focus_filter: false, group_rows: Vec::new(), row_menu: None, choosing_folder: false, renaming: None, focus_rename: false, head_menu: None,
            kind_menu: None, kinds: Vec::new(), prompt: None, columns: Default::default(), cells_scroll: 0., readiness_sent: Some(false), tag_from_node: false, edits: Default::default(), mode: String::new(), issues: false,
            identity: Vec::new(),
            inspector_title: String::new(),
            inspector_subtitle: None,
            inspector: Vec::new(), highlighted: None, focused_row: None,
        });
        col.nodes = tree_rows::reconcile(std::mem::take(&mut col.nodes), rows);
        // Folded, tags-only documents need only a few row mutations on a viewport pick.
        col.tag_index = col.nodes.iter().all(|n| matches!(n.key.split_once(':').map(|(kind, _)| kind),
            Some("preview-tag" | "preview-folder" | "geometry" | "sections" | "section" |
                 "sheet-page" | "sheet-pages" | "sheet-frame" | "saved-scene" | "saved-scenes")))
            .then(|| col.nodes.iter().enumerate().filter(|(_, n)| n.row == 6)
                .map(|(i, n)| (n.key.clone(), i)).collect());
        col.marked_rows = if col.tag_index.is_some() {
            col.nodes.iter().enumerate().filter(|(_, n)| n.mark != 0 || n.selected != 0).map(|(i, _)| i).collect()
        } else { Vec::new() };
        if col.highlighted.as_ref().is_some_and(|k| !k.starts_with("n:") && !k.starts_with("preview-tag:") &&
            !col.nodes.iter().any(|n| &n.key == k && n.selected > 0)) {
            col.highlighted = None;
        }
        col.styles.refresh(&col.nodes);
        col.ordered = col.sort.order(&col.nodes);
        col.unfiled = unfiled;
    }
    pub fn mark_tag_paths(&mut self, paths: Vec<Vec<String>>, picked: usize) -> bool {
        let Some(col) = self.column.as_mut() else { return false };
        let Some(index) = &col.tag_index else { return false };
        for i in col.marked_rows.drain(..) {
            col.nodes[i].selected = 0;
            col.nodes[i].mark = 0;
        }
        for path in paths {
            let mut mark = 1;
            for key in path {
                if let Some(&i) = index.get(&key) {
                    let row = &mut col.nodes[i];
                    if row.mark == 0 { col.marked_rows.push(i); }
                    row.mark = if row.mark == 1 { 1 } else { mark };
                    row.selected += 1;
                    mark = 2;
                }
            }
        }
        col.browser_picks = picked;
        true
    }
    pub fn set_mode(&mut self, mode: String) {
        if let Some(col) = self.column.as_mut() {
            col.mode = mode;
        }
    }
    /// The dock's width as last kept (clamped to what the window allows).
    pub fn set_dock_width(&mut self, w: f32) { self.dock_w = w.clamp(DOCK_MIN_W, DOCK_MAX_W); }
    /// The dock's width in this window: the user's, within its bounds.
    fn dock_width(&self, screen: Rect) -> f32 { dock_width(self.dock_w + DESTINATION_STRIP_W, screen.width()) }
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
        self.destination = Destination::Project;
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
            if col.styles.owner != owner { col.styles = Default::default(); col.styles.owner=owner; col.row_menu=None; col.renaming=None; col.proposal_dismissed=false; col.highlighted=None; col.focused_row=None; col.issue_at=None; }
            col.styles.lines=lines;
            col.styles.set_patterns(patterns);
            if let Some(scene)=scene { col.styles.sync_catalog(scene)?; col.styles.hatches.sync(scene)?; }
        }
        Ok(())
    }
    pub fn open_line_type(&mut self,name:&str,tags:Vec<String>)->bool {
        self.destination = Destination::Project;
        let Some(col)=self.column.as_mut() else{return false;};
        self.plugins.showing=false;
        col.mode.clear();
        col.styles.catalog.open_line(name,tags)
    }
    pub fn hatch_preference(&mut self,py:pyo3::Python<'_>,verb:&str,value:&str)->pyo3::PyResult<()> {
        if let Some(col)=self.column.as_mut(){col.styles.hatches.preference(py,verb,value)?;}
        Ok(())
    }
    pub fn tick_tag_counts(&mut self, history: &pyo3::Bound<'_, pyo3::PyAny>) -> pyo3::PyResult<bool> {
        if let Some(col) = self.column.as_mut() { return col.styles.tick_counts(history); }
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
    /// A column preset by index (0 Filing, 1 Drawing, 2 Export).
    pub fn set_lens(&mut self, lens: u8) { self.set_columns(lens, None); }
    /// The preset and, when the user changed its columns, the columns shown.
    pub fn set_columns(&mut self, lens: u8, mask: Option<u16>) {
        if let Some(col) = self.column.as_mut() { col.columns = lens::Columns::stored(Lens::from_index(lens), mask); }
    }
    pub fn set_kinds(&mut self, kinds: Vec<String>) {
        if let Some(col) = self.column.as_mut() { col.kinds = kinds; }
    }
    pub fn set_prompt(&mut self, title: String, body: String, buttons: Vec<(String, String)>) {
        if body.starts_with("\u{1d}edit\u{1d}") {
            self.set_review(false);
            self.destination = Destination::Project;
            self.plugins.showing = false;
        }
        if let Some(col) = self.column.as_mut() {
            col.edits.retain(|key, _| !key.starts_with("prompt:"));
            col.prompt = (!title.is_empty()).then_some((title, body, buttons));
        }
    }
    pub fn set_identity(&mut self, rows: Vec<(String, String, String, Option<bool>)>) {
        if let Some(col) = self.column.as_mut() { col.identity = rows; }
    }
    pub fn show_inspector(&mut self) { self.set_review(false); }
    pub fn set_inspector_subtitle(&mut self, subtitle: Option<String>) {
        if let Some(col) = self.column.as_mut() { col.inspector_subtitle = subtitle; }
    }
    pub fn set_inspector(&mut self, title: String, sections: Vec<(String, Vec<(String, String)>)>) {
        if let Some(col) = self.column.as_mut() {
            if col.inspector_title != title || col.inspector != sections {
                col.edits.clear();
            }
            col.inspector_subtitle = None;
            col.inspector_title = title;
            col.inspector = sections;
            col.identity.clear();
        }
    }
    /// "File to…": the tree takes the next pick; the filter gets the keys.
    pub fn start_filing(&mut self, what: String) {
        self.destination = Destination::Project;
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
    pub fn set_junction_review(&mut self, panel: Option<(String, String, Vec<crate::junction_review::Row>, usize)>) {
        self.junction_review = panel;
    }

    /// The junction review list beside the rail (docs/junctions.md §11): the
    /// meetings that fit, near misses with Widen, skipped ones with Un-skip.
    /// Apply (Enter) applies the ticked; nothing is applied otherwise.
    fn junction_review_panel(&mut self, ctx: &egui::Context, screen: Rect) {
        let Some((title, count, rows, ticked)) = self.junction_review.clone() else { return };
        let (band_top, band_bottom) = self.band(ctx, screen);
        let at = pos2(GUT + self.rail_w + GUT, band_top);
        let mut out = Vec::new();
        egui::Area::new(egui::Id::new("junction_review"))
            .fixed_pos(at)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                popover().inner_margin(Margin::same(8)).show(ui, |ui| {
                    ui.set_width(316.0);
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
                    ui.painter().text(hr.left_center() + vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, &title, title_font(), INK);
                    ui.painter().text(hr.right_center() - vec2(6.0, 0.0), egui::Align2::RIGHT_CENTER, &count, meta(), DIM);
                    let tone = |t: &str| match t { "ok" => OK, "warn" => WARN, "off" => OFF, _ => DIM };
                    let mut section = "";
                    egui::ScrollArea::vertical().max_height((band_bottom - band_top - 120.0).max(120.0)).show(ui, |ui| {
                        for (index, label, why, status, t, on, tickable, actions) in &rows {
                            let group = match t.as_str() { "warn" => "near", "off" => "skipped", _ => "" };
                            if group != section {
                                section = group;
                                ui.add_space(SPACE_XS);
                                caption_at(ui, if group == "near" { tr("Near") } else { tr("Skipped") }.as_str(), 6.0);
                            }
                            let names: Vec<String> = actions.iter().map(|(_, n)| n.clone()).collect();
                            let (tick, act) = check_row(ui, label, why, status, tone(t), *on, *tickable, &names);
                            if tick {
                                out.push(format!("jreview:tick:{index}"));
                            }
                            if let Some(a) = act {
                                out.push(format!("jreview:{}:{index}", actions[a].0));
                            }
                        }
                    });
                    ui.add_space(SPACE_S);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(SPACE_S, 0.0);
                        let apply = tr("Apply {n}").replace("{n}", &ticked.to_string());
                        if sheet_button(ui, &apply, "↩", 1) {
                            out.push("jreview:apply".into());
                        }
                        if sheet_button(ui, &tr("Skip all"), "", 0) {
                            out.push("jreview:skip_all".into());
                        }
                        if sheet_button(ui, &tr("Close"), "esc", 3) {
                            out.push("jreview:close".into());
                        }
                    });
                });
            });
        self.actions.extend(out);
    }

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
    pub fn place_form(&mut self, placement: &str) {
        // A child prompt above a sheet must stay on top of that sheet.
        if placement == "drawer" && (self.lsheet.open || self.sheets.shown) {
            return;
        }
        self.set_review(false);
        if placement == "inspector" {
            self.destination = Destination::Project;
            self.plugins.showing = false;
        }
        if let Some(d) = &mut self.dialog {
            if placement=="cursor" && d.cursor_anchor.is_none(){d.cursor_anchor=self.ctx.pointer_latest_pos();}
            d.placement = placement.into();
        }
    }
    pub fn organise_scenes(&mut self) {
        self.menu = None;
        self.lsheet.set(Some(large_sheet::Spec { id: "scene-organiser".into(),
            title: tr("Organise scenes"), width: 760., closable: true, ..Default::default() }));
    }

    pub fn extension_menu(&mut self, rows: Vec<crate::extension_menu::Row>) { self.plugins.commands = rows; }
    pub fn plugin_panel(&mut self, raw: &str) -> Result<(), String> { self.plugins.set(raw) }
    pub fn show_destination(&mut self, name: &str) {
        self.set_review(false);
        self.destination = match name {
            "deliveries" => Destination::Deliveries,
            "tasks" => Destination::Tasks,
            "project" => Destination::Project,
            _ => return,
        };
        if let Some(col) = &mut self.column { col.head_menu = None; col.kind_menu = None; }
    }
    pub fn plugin_open(&mut self, id: &str) {
        if self.plugins.destinations().any(|(key, _)| key == id) {
            self.destination = Destination::Plugin(id.into());
        } else {
            self.plugins.open(id);
            if self.plugins.dock_visible(id) { self.destination = Destination::Project; }
        }
    }
    pub fn plugin_visible(&self, instance: &str) -> bool {
        self.gallery.is_none() && !self.review && !self.lsheet.open && !self.sheets.shown
            && (self.destination == Destination::Plugin(instance.into()) || (self.drawer_mode() == Some(DrawerMode::Pinned) && self.plugins.pinned_visible(instance)) || (self.destination == Destination::Project && self.plugins.dock_visible(instance)))
    }
    pub fn plugin_document(&mut self, document: String, live: &[String]) {
        if self.plugin_document != document {
            let next = self.background_plugins.remove(&document).unwrap_or_default();
            let previous = std::mem::replace(&mut self.plugins, next);
            if matches!(&self.destination, Destination::Plugin(id) if !self.plugins.destinations().any(|(live, _)| live == id)) {
                self.destination = Destination::Project;
            }
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
            let dock = self.dock_width(s);
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
            // A tag (or tag folder) in focus is the dock's own selection.
            if col.styles.active() {
                col.styles.clear_focus();
                col.tag_from_node = false;
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
        if name == "Escape" && (self.activity_open || self.menu_hits.len() > 1 || self.ctx_menu.is_some() || self.strip.menus_open() || self.egui_popup_open() || self.panels.data["terrain"].is_object() || self.panels.data["camera"].is_object()) { return true; }
        // The large sheet owns every key while it is up (Python keeps the
        // window's ⌘ shortcuts, sheet_runtime.rs).
        if self.lsheet.open { return true; }
        if self.sheets.shown { return self.sheets_claims_key(name); }
        if self.sketch_claims_key() { return true; }
        if self.scene_move_key(name, mods).is_some() { return true; }
        if self.fillet_key(name) { return true; }
        let filing = self.column.as_ref().is_some_and(|c| c.filing.is_some() || c.row_menu.is_some()
            || c.kind_menu.is_some() || c.head_menu.is_some() || c.prompt.is_some() || c.edits.contains_key("\u{1d}pick"));
        self.column.as_ref().is_some_and(|c| !self.review && c.claims_row_key(name, mods)) || self.sheet.is_some() || self.dialog.is_some() || self.cmd_open || (name == "Escape" && (self.view_open || self.menu.is_some() || self.form.is_some() || filing
            || self.ctx_menu.is_some() || self.strip.menus_open()))
            || self.ctx.wants_keyboard_input()
    }
    fn fillet_key(&self, name: &str) -> bool {
        self.drawer_mode() == Some(DrawerMode::Fillet) && !self.cmd_open && !self.review &&
            (name == "Escape" || (name == "Enter" && !self.cursor.live.as_ref().is_some_and(|v| v.typing)))
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
        if name == "Escape" && self.activity_open && !self.cmd_open && self.dialog.is_none() {
            if pressed { self.activity_open = false; }
            return true;
        }
        if name == "Escape" && self.menu_hits.len() > 1 {
            self.events.push(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: mods_from(mods) });
            return true;
        }
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
        if self.sketch_claims_key() { return self.sketch_key(name, pressed, mods); }
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
        if self.fillet_key(name) {
            if name == "Enter" && self.ctx.wants_keyboard_input() {
                // The form's field ends its edit on Enter inside egui's pass,
                // so it sees the focus go and sends the typed size. Clearing
                // the focus here, between passes, hid that: the size never
                // reached the tool and every later Apply refused the old one.
                if let Some(key) = egui::Key::from_name(name) {
                    self.events.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: mods_from(mods) });
                }
            } else if pressed {
                if name == "Escape" {
                    self.actions.push("tool:select".into());
                } else {
                    self.actions.push(r#"panel:["bevel.finish",null]"#.into());
                }
            }
            return true;
        }
        if name == "Escape" && self.panels.data["camera"].is_object() && !self.ctx.wants_keyboard_input() {
            if pressed { self.actions.push(r#"panel:["camera.close",null]"#.into()); }
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
        if self.sketch_claims_key() && !self.ctx.wants_keyboard_input() {
            if kind == "commit" { self.sketch.texts.push(text); }
            return true;
        }
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
        if self.sketch_claims_key() && self.font_picker.is_none() {
            return self.sketch_text(text);
        }
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

    /// The keystroke overlay (keycast.rs): a key or chord pressed, the tool
    /// it armed, the value field's text after a key, a mouse button.
    pub fn keycast_key(&mut self, keys: String) {
        let now = self.start.elapsed().as_secs_f64();
        self.keycast.key(keys, now);
    }
    pub fn keycast_tool(&mut self, keys: String, tool: String) {
        let now = self.start.elapsed().as_secs_f64();
        self.keycast.tool(keys, tool, now);
    }
    pub fn keycast_typed(&mut self, value: String) {
        let now = self.start.elapsed().as_secs_f64();
        self.keycast.typed(value, now);
    }
    pub fn keycast_button(&mut self, button: u32, mods: i64) {
        let now = self.start.elapsed().as_secs_f64();
        self.keycast.button(button, mods, now);
    }
    /// What the pill shows now (keys, tool, typed), empty once it faded.
    pub fn keycast_chips(&self) -> Vec<(String, String, bool)> {
        let now = self.start.elapsed().as_secs_f64();
        if !crate::keycast::shown() || self.keycast.opacity(now).is_none() { return Vec::new(); }
        self.keycast.chips.iter().map(|c| (c.keys.clone(), c.tool.clone(), c.typed)).collect()
    }
    pub fn set_active(&mut self, id: String) {
        // Another tool is the next action: the committed value's readback goes.
        if id != self.active { self.cursor.end_readback(); }
        self.active = id;
    }
    pub fn set_status(&mut self, status: String, hint: String, coordinate: &str, tooltip: &str) {
        coordinate.clone_into(&mut self.coordinate);
        tooltip.clone_into(&mut self.coordinate_tooltip);
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
        let confirming_fillet = self.bevel_active() && self.cursor.live.as_ref().is_some_and(|v| v.typing);
        self.cursor.set(caption, value, typing, at, start.map(|(x, y)| pos2(x, y)), revision);
        if confirming_fillet && !typing && self.cursor.readback.is_some() {
            self.actions.push("tool:select".into());
        }
    }
    pub fn set_selection(&mut self, summary: String, actions: Vec<(String, String, String, bool, bool)>) {
        self.selection = (!summary.is_empty()).then_some((summary, actions));
    }
    /// Where an area was last laid out, in points (tests and harnesses).
    pub fn area_rect(&self, name: &str) -> Option<(f32, f32, f32, f32)> {
        self.ctx.memory(|m| m.area_rect(egui::Id::new(name))).map(|r| (r.min.x, r.min.y, r.width(), r.height()))
    }
    pub fn take_actions(&mut self) -> Vec<String> { std::mem::take(&mut self.actions) }
    /// The Sketch workspace (ui/sketch.rs): enter with a JSON spec, ask, run
    /// a window command in it, leave, show a message, reopen what Cancel
    /// discarded.
    pub fn sketch_open(&mut self, spec: &str) -> Result<(), String> { self.sketch_enter(spec) }
    pub fn sketch_shown(&self) -> bool { self.sketch.shown }
    pub fn sketch_run(&mut self, cmd: &str) -> bool { self.sketch_command(cmd) }
    pub fn sketch_close(&mut self) { self.sketch.shown = false; }
    pub fn sketch_message(&mut self, text: &str) { self.sketch.say(text.to_owned(), None); }
    pub fn sketch_discarded(&mut self) -> Option<String> { self.sketch_reopen_discarded() }
    pub fn sketch_row(&self) -> String { self.sketch.row_json().to_string() }
    pub fn repaint_after_ms(&self) -> Option<f64> { self.repaint_ms }

    /// Only the interactive chrome takes the pointer; labels pass it through.
    fn egui_popup_open(&self) -> bool {
        self.ctx.memory(|m| m.any_popup_open())
    }

    pub fn input_state(&self) -> std::collections::HashMap<&'static str, bool> {
        let visible = |name| self.ctx.memory(|m| m.areas().visible_last_frame(
            &egui::LayerId::new(egui::Order::Foreground, egui::Id::new(name))));
        [
            ("drawer_pinned", visible("drawer") && self.drawer_mode() == Some(DrawerMode::Pinned)),
            ("drawer_operation", visible("drawer") && matches!(self.drawer_mode(), Some(DrawerMode::Fillet | DrawerMode::Form))),
            ("recent_preview", self.glass.showing_preview()),
            ("tool_rail", visible("palette") || visible("sheets_rail")), ("dock", visible("column") || visible("sheets_dock")), ("bottom_bar", visible("bottom")),
            ("view_menu", self.view_open),
            ("drawing_line", self.column.as_ref().is_some_and(|c| c.styles.catalog.editing_line())),
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
        self.dialog.is_some() || self.gallery.is_some() || self.menu_hits.len() > 1 || self.menu_hits.iter().any(|r| r.contains(pos)) || self.font_picker.is_some() || self.egui_popup_open() || self.hit.iter().any(|r| r.contains(pos))
            || self.gizmo.takes(pos)
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
        // Keep the confirmation at rest. Moving on the model starts a new reading.
        if kind == "press" || (kind == "move" && self.cursor.readback.is_some() && !self.over_chrome(pos)) {
            self.cursor.end_readback();
        }
        if kind == "press" {
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
                        // A press on the model leaves the tag in focus.
                        if !self.captured && button == egui::PointerButton::Primary && col.styles.active() {
                            col.styles.clear_focus();
                            col.tag_from_node = false;
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
            self.cursor.end_readback();
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
        let lights = self.projects.as_ref().map_or(tokens::LIGHTS_ROOM, |p| p.0);
        #[cfg(any(target_os="linux", windows))]
        egui::Area::new("desktop_menu".into()).fixed_pos(Pos2::ZERO).order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::NONE.fill(ctx.style().visuals.panel_fill).show(ui, |ui| {
                    ui.set_min_size(vec2(screen.width(), MENU_H));
                    self.menu_hits = crate::desktop_menu::show(ui, &self.plugins.commands,
                        self.sheet.is_none() && self.dialog.is_none() && self.form.is_none() && self.jobs.is_empty());
                    self.cluster_buttons.extend(crate::desktop_menu::take_buttons());
                    self.menu_hits.push(ui.min_rect());
                });
            });
        egui::Area::new(egui::Id::new("top"))
            .fixed_pos(pos2(0.0, MENU_H))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Bar).inner_margin(Margin { left: lights as i8, right: 8, top: 6, bottom: 6 }).show(ui, |ui| {
                    ui.set_width(screen.width() - lights - 8.0);
                    ui.set_height(TOP_H - MENU_H - 12.0);
                    ui.spacing_mut().interact_size.y = 28.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(SPACE_XS, 0.0);
                        let left = ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = vec2(SPACE_XS, 0.0);
                            if !self.strip.tabs.is_empty() {
                                // The project tabs (option B): room is left for
                                // the scene control and at least 60 pt to drag.
                                let max_w = if sheet_up { screen.width() - lights - 68.0 } else { (screen.width() - lights - 8.0 - self.top_right_w - self.scene_controls_w - 60.0).max(160.0) };
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
                            if self.sketch.shown {
                                self.sketch_crumbs(ui);
                                return;
                            }
                            let controls_left = ui.cursor().left();
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
                            self.scene_controls_w = ui.cursor().left() - controls_left;
                        });
                        groups.push(left.response.rect);
                        if sheet_up { return; }
                        if self.sketch.shown {
                            ui.add_space((ui.available_width() - self.top_right_w).max(0.0));
                            let right = ui.horizontal(|ui| self.sketch_bar_right(ui));
                            groups.push(right.response.rect);
                            return;
                        }
                        // The right group, laid out left to right and pushed to
                        // the right edge by its width from the last frame.
                        ui.add_space((ui.available_width() - self.top_right_w).max(0.0));
                        {
                            let right = ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing = vec2(SPACE_XS, 0.0);
                                if self.plugins.registered("render_blender") && !self.sheets.shown {
                                    let render = text_button_response(ui, "Render", "Open render drawer", true);
                                    self.cluster_buttons.push(("render".into(), render.rect.center().into()));
                                    if render.clicked() { self.actions.push("drawer:render_blender".into()); }
                                }
                                let library = text_button_response(ui, "▤  Library", "Open Library (⇧⌘L)", true);
                                self.cluster_buttons.push(("library".into(), library.rect.center().into()));
                                if library.clicked() { clicked = Some("libsheet:".into()); }
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
                self.view_open = !self.view_open || self.gizmo.views_only;
                self.gizmo.views_only = false;
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
            Some(a) => self.actions.push(if a.starts_with("act:") || a == "libsheet:" { a.to_owned() } else { format!("tab:{a}") }),
            None => {}
        }
    }

    /// Is ``pos`` on the top bar but on none of its controls (it then moves
    /// the window, as a title bar does)?
    pub fn top_empty(&self, x: f32, y: f32) -> bool {
        let p = pos2(x, y);
        !self.menu_hits.iter().any(|r| r.contains(p)) && !self.dismissible_menu_open() && self.gallery.is_none() && y <= TOP_H && !self.top_groups.iter().any(|r| r.contains(p))
            && !(self.ctx.memory(|m| m.area_rect(egui::Id::new("jobs"))).is_some_and(|r| (!self.jobs.is_empty() || self.activity_open) && r.contains(p)))
            && !(self.projects.is_none() && x < tokens::LIGHTS_ROOM)
    }

    /// The active tool, its options and its gesture hint.
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
                        ui.allocate_ui_with_layout(vec2(68., CONTROL_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            let choices = ["m", "cm", "mm", "ft-in"];
                            if let (Some(i), _) = kit::select(ui, "document-units", &self.units, &choices) {
                                clicked = Some(format!("start:units{}{}", crate::start_host::SEP, i));
                            }
                        });
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
                        for (id, label, key, state) in self.options.iter().filter(|_| !self.bevel_active()) {
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
        let mut close_menu = false;
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
                                if menu_row(ui, &(String::new(), "New model".into(), "⌘N".into(), "check".into(), -1, true)) {
                                    clicked = Some("proj:new".into());
                                }
                                if let Some(i) = docs.iter().position(|d| d.1) {
                                    if menu_row(ui, &(String::new(), "Close model".into(), "⌘W".into(), "check".into(), -1, true)) {
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
                                self.scene_browser.show(ui,scenes,&self.tabs[3],&mut clicked,&mut self.actions,false);
                            } else if let Some(items) = self.tabs.get(group) {
                                for item in items {
                                    if item.4 == 1 { current = Some(item.1.clone()); }
                                    let row = (item.0.clone(), item.1.clone(), String::new(), "check".into(), check(item.4 == 1), true);
                                    if menu_row(ui, &row) { clicked = Some(format!("tab:{}", item.0)); }
                                }
                            }
                            // What acts on the whole model lives here, not in the tree's menu.
                            if kind == "work" && !self.sheets.shown {
                                if let Some(col) = self.column.as_mut() {
                                    let (sent, plugins) = dock_menus::model_rows(col, ui, self.plugins.available());
                                    if plugins {
                                        self.plugins.showing = self.destination != Destination::Project || !self.plugins.showing;
                                        self.destination = Destination::Project;
                                    }
                                    close_menu = plugins || !sent.is_empty();
                                    self.actions.extend(sent.into_iter().map(|action| format!("tree:{action}")));
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
        if close_menu { self.menu = None; }
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
                    // From the view label: the views, projection and style only.
                    let views_only = self.gizmo.views_only;
                    if !views_only {
                        let review = (String::new(), "Review — hide the panels".to_owned(), String::new(), "check".to_owned(),
                                      if self.review { 1 } else { 0 }, true);
                        if menu_row(ui, &review) { toggle_review = true; }
                        ui.add_space(SPACE_XS);
                    }
                    // Down to the bottom bar (M05). The area remembers last
                    // frame's size, so give the scroll its room explicitly or
                    // it stays at the first frame's default height.
                    let room = view_menu_room(self.screen, ui.cursor().top());
                    let at = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), room));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(at), |ui| egui::ScrollArea::vertical().id_salt("view command groups")
                        .max_height(room)
                        .show(ui, |ui| {
                            let groups = self.view_menu.iter().filter(|(_, items)| !views_only || view_overlay::views_section(items));
                            for (si, (title, items)) in groups.enumerate() {
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
                        }));
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
                                let r = switch_row(ui, &label, f.4 > 0.5, true);
                                let r = if text.is_empty() { r } else { r.on_hover_text(&text) };
                                if r.clicked() {
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

    fn bevel_active(&self) -> bool { matches!(self.active.as_str(), "fillet" | "chamfer") }

    fn drawer_mode(&self) -> Option<DrawerMode> {
        DrawerMode::resolve(self.dialog.as_ref().is_some_and(|d| d.in_drawer()),
                            self.bevel_active() && self.panels.data["bevel"].is_object(),
                            self.plugins.has_pinned())
    }

    fn draw_drawer(&mut self, ctx: &egui::Context, screen: Rect) {
        let Some(mode) = self.drawer_mode() else { return };
        let bounds = self.drawer_bounds(ctx, screen);
        if mode == DrawerMode::Form {
            self.draw_operation_preview(ctx, bounds);
        }
        kit::drawer(ctx, bounds, |ui| {
            let width = ui.available_width();
            match mode {
                DrawerMode::Pinned => {
                    egui::Resize::default().id_salt("drawer-height")
                        .resizable([false, true]).default_width(width)
                        .min_width(width).max_width(width).default_height(230.)
                        .max_height((bounds.height() * 0.60).max(120.))
                        .show(ui, |ui| self.plugins.pinned(ui, &mut self.actions));
                }
                DrawerMode::Fillet => {
                    kit::caption_plain(ui, if self.active == "chamfer" { "Chamfer" } else { "Fillet" });
                    ui.push_id(&self.active, |ui| self.panels.bevel(ui, &mut self.actions));
                }
                DrawerMode::Form => self.drawer_form(ui),
            }
        });
    }

    fn drawer_bounds(&self, ctx: &egui::Context, screen: Rect) -> Rect {
        let (top, bottom) = self.band(ctx, screen);
        let left = if self.groups.is_empty() { GUT } else { GUT + self.rail_w + GUT };
        let right = self.free_right(ctx, screen) - GUT;
        Rect::from_min_max(pos2(left, top), pos2(right.max(left + 120.), bottom))
    }

    fn form_anchor(&self, ctx: &egui::Context, screen: Rect) -> Pos2 {
        let free = self.drawer_bounds(ctx, screen);
        pos2(free.center().x, free.top())
    }

    /// The right edge of the free viewport (left of the column).
    fn free_right(&self, ctx: &egui::Context, screen: Rect) -> f32 {
        if self.column.is_none() { return screen.max.x; }
        ctx.memory(|m| m.area_rect(egui::Id::new("column"))).map_or(screen.max.x, |r| r.min.x)
    }

    fn right_column(&mut self, ctx: &egui::Context, screen: Rect, retract: f32) {
        let delivery = self.delivery.clone();
        let selection = self.selection.clone();
        let mut cmds: Vec<String> = Vec::new();
        let (top, bottom) = self.band(ctx, screen);
        let self_icons = self.icons.clone();
        let anchor = self.form_anchor(ctx, screen);
        // One width, the user's, whatever the tree's columns.
        let pane_width = self.dock_width(screen);
        #[allow(non_snake_case)]
        let W = pane_width - DESTINATION_STRIP_W;
        let Some(col) = self.column.as_mut() else { return };
        let height = (bottom - top).max(200.0);
        let mut actions: Vec<String> = Vec::new();
        egui::Area::new(egui::Id::new("column"))
            .constrain(false)
            .interactable(retract == 0.0)
            .fixed_pos(pos2(screen.max.x - GUT - pane_width + retract * (pane_width + GUT), top))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Panel).inner_margin(Margin::same(10)).show(ui, |ui| {
                    ui.set_min_width(pane_width - 20.);
                    let outer = ui.cursor().min + vec2(pane_width - 20., 0.) + vec2(10., -10.);
                    let strip = Rect::from_min_size(outer - vec2(DESTINATION_STRIP_W, 0.), vec2(DESTINATION_STRIP_W, height));
                    let report = &self.panels.data["readiness"];
                    let blocked = export::blocked(report);
                    let tasks = self.panels.data["tasks"]["unresolved"].as_u64().filter(|n| *n > 0).map(|n| n.to_string()).unwrap_or_default();
                    let mut next = None;
                    kit::destination_strip(ui, strip, |ui| {
                        for (i, (dest, label, glyph)) in [
                            (Destination::Project, "Project", "folder"),
                            (Destination::Tasks, "Tasks", "tasks"),
                            (Destination::Deliveries, "Deliveries", "component"),
                        ].into_iter().enumerate() {
                            let shortcut = if cfg!(target_os="macos") { format!("⌥⌘{}", i+1) } else { format!("Ctrl+Alt+{}", i+1) };
                            let badge = match dest { Destination::Deliveries if blocked > 0 => "!", Destination::Tasks => &tasks, _ => "" };
                            if kit::destination_button(ui, label, glyph, &shortcut, badge, self.destination == dest) { next = Some(dest); }
                        }
                        for (i, (id, name)) in self.plugins.destinations().enumerate() {
                            if i == 0 { ui.separator(); }
                            let dest = Destination::Plugin(id.to_owned());
                            if kit::destination_button(ui, name, "sheet", "", "", self.destination == dest) { next = Some(dest); }
                        }
                    });
                    if let Some(dest) = next {
                        self.destination = dest;
                        col.head_menu = None;
                        col.kind_menu = None;
                    }
                    ui.set_min_height(height - 20.);
                    let content = Rect::from_min_size(ui.cursor().min, vec2(W - 20., height - 20.));
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(content));
                    let ui = &mut child;
                    ui.set_width(W - 20.0);
                    ui.set_max_width(W - 20.0);
                    ui.set_height(height - 20.0);
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    match &self.destination {
                        Destination::Tasks => {
                            Self::tasks_destination(ui, &self.panels.data["tasks"], &mut self.tasks, &mut cmds);
                            return;
                        }
                        Destination::Deliveries => {
                            if export::destination(ui, report, &self.jobs, &mut cmds, &mut actions, &mut self.fix_rule) {
                                self.fix_rows = true;
                                self.destination = Destination::Project;
                                actions.push("fixrows\u{1f}1".into());
                            }
                            return;
                        }
                        Destination::Plugin(id) => {
                            self.plugins.draw(ui, id, &mut cmds);
                            return;
                        }
                        Destination::Project => {}
                    }
                    if self.fix_rows {
                        let label = self.fix_rule.as_deref().unwrap_or("Delivery checks");
                        kit::caption_plain(ui, label);
                        export::status_fixes(ui, report, label, &mut actions);
                    }
                    if self.fix_rows && kit::quiet(ui, "← Back to Deliveries").clicked() {
                        self.fix_rows = false;
                        self.destination = Destination::Deliveries;
                        actions.push("fixrows\u{1f}0".into());
                        return;
                    }
                    if self.panels.data["camera"].is_object() {
                        egui::ScrollArea::vertical().id_salt("camera inspector").show(ui, |ui| self.panels.camera(ui,&mut cmds));
                        return;
                    }
                    // The header: the tree's columns, then its own short menu.
                    ui.horizontal(|ui| {
                        ui.set_min_height(28.0);
                        let open = matches!(col.head_menu, Some((dock_menus::HeadMenu::Columns, _)));
                        let control = kit::dropdown(ui, "Columns", if self.fix_rows { "Export" } else { col.columns.name() }, open, (ui.available_width() - 84.).max(120.))
                            .on_hover_text("The tree's columns: a preset, or column by column");
                        if control.clicked() {
                            self.plugins.showing = false;
                            col.head_menu = (!open).then_some((dock_menus::HeadMenu::Columns, control.rect));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let dots = (String::new(), "⋯".to_owned(), "Tree: new node, expand, collapse…".to_owned(), String::new(), -1i8, true);
                            let open = matches!(col.head_menu, Some((dock_menus::HeadMenu::More, _)));
                            let more = item_response(ui, &dots, None);
                            if more.clicked() {
                                col.head_menu = (!open).then_some((dock_menus::HeadMenu::More, more.rect));
                            }
                            let total = delivery.as_ref().map_or(0, |d| d.2) + col.unfiled;
                            if total > 0 && !col.proposal_available && kit::quiet(ui,&format!("⚠ {total}")).on_hover_text("Show the Export columns and what to fix").clicked() {
                                col.columns = lens::Columns::preset(Lens::Export);
                                col.cells_scroll = 0.;
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
                    let columns = if self.fix_rows { lens::Columns::preset(Lens::Export) } else { col.columns };
                    let readiness_shown = columns.has(Col::Readiness);
                    // One line of context over the filter, the same height whatever it
                    // says, so a preset does not move the rows: the delivery the
                    // Readiness column answers for; otherwise the suggestion to give a
                    // tagged model its filing, or where new items go.
                    if readiness_shown {
                        if export::strip(ui, report) {
                            self.destination = Destination::Deliveries;
                        }
                        ui.add_space(SPACE_XS);
                    } else if col.path.len() <= 1 && col.proposal_available && !col.proposal_dismissed {
                        ui.horizontal(|ui| {
                            ui.set_height(BUTTON_H);
                            let overview = &col.overview;
                            let has_source = overview.tags > 1 || overview.folders > 0 || overview.links > 0;
                            if !has_source {
                                if kit::sheet_button(ui, "New node", "", 0) { actions.push("new\u{1f}".into()); }
                                return;
                            }
                            if kit::panel_button(ui, "Propose structure from tags…", "", 0, None).on_hover_text("Preview a filing structure from these tags").clicked() {
                                actions.push("propose\u{1f}".into());
                            }
                            if kit::icon_button(ui, "×", "Dismiss the suggestion").clicked() {
                                col.proposal_dismissed = true;
                            }
                        });
                        ui.add_space(SPACE_XS);
                    } else if col.path.len() <= 1 && !col.proposal_available {
                        if let Some((target,_,_)) = &delivery {
                            ui.horizontal(|ui| {
                                ui.set_height(BUTTON_H);
                                ui.spacing_mut().item_spacing.x = SPACE_XS;
                                ui.add_space(SPACE_S);
                                kit::caption_plain(ui,"New items go to");
                                ui.label(if target.is_empty() { "No filing target" } else { target });
                            });
                            ui.add_space(SPACE_XS);
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
                    // The filter; its quick choices and the sort open from the button at its end.
                    let narrowed = tree_filter::active(&col.filter) || col.sort != tree_order::Sort::default();
                    let mut field = None;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = SPACE_XS;
                        let edit = kit::field(&mut col.filter).id(egui::Id::new("dock_filter"))
                            .hint_text(egui::RichText::new(if col.filing.is_some() { "Node name or kind" }
                                else { "Filter" }).color(FAINT))
                            .margin(Margin::symmetric(8, 5))
                            .desired_width((W - 20.0 - 16.0 - 4.0 - CONTROL_H - SPACE_XS).max(40.0))
                            .font(body());
                        field = Some(ui.add_enabled(!self.fix_rows, edit).on_hover_text("Filter by name or wildcard. Use is:hidden, is:visible, is:locked, is:selected, is:component, is:image or is:section. Prefix the query with ! to invert it."));
                        let open = matches!(col.head_menu, Some((dock_menus::HeadMenu::Filter, _)));
                        let button = kit::filter_button(ui, narrowed, open);
                        if button.clicked() {
                            col.head_menu = (!open).then_some((dock_menus::HeadMenu::Filter, button.rect));
                        }
                    });
                    let Some(resp) = field else { return };
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
                    // The split follows what is in focus, never the columns.
                    let available = ui.available_height();
                    let tag_focus = col.mode.is_empty() && col.styles.active() && !col.tag_from_node;
                    let room = if !col.mode.is_empty() && col.inspector.is_empty() {
                        available - 44.0
                    } else if tag_focus {
                        available * 0.5
                    } else if !col.identity.is_empty() {
                        // Keep the identity fields and their actions together in short windows.
                        (available * 0.6).min(available - ROW_H * 12.)
                    } else {
                        available * 0.6
                    };
                    let filtered;
                    let order = if self.fix_rows {
                        let keys = self.fix_rule.as_ref().map(|rule| export::rule_rows(report, rule));
                        filtered = col.ordered.iter().copied().filter(|&i| {
                            let key = &col.nodes[i].key;
                            keys.as_ref().map_or_else(|| matches!(export::row_state(report, key, None), export::Ready::ToFix(_)), |keys| keys.contains(key))
                        }).collect::<Vec<_>>();
                        &filtered
                    } else { &col.ordered };
                    let nodes_len = if self.fix_rows { order.len() } else { col.nodes.len() };
                    if nodes_len > 0 && !col.filter.trim().is_empty() {
                        kit::caption_plain(ui, &format!("{nodes_len} {} shown", if nodes_len == 1 { "row" } else { "rows" }));
                    }
                    if nodes_len == 0 {
                        empty_state(ui, if self.fix_rows { "No rows to fix" } else { "No matching rows" });
                    }
                    // The header and virtualized rows share one layout of the columns.
                    let row_left = ui.cursor().min.x;
                    let cols = lens::RowLayout::plan(columns, row_left, row_left + ui.available_width(), col.cells_scroll);
                    col.cells_scroll = col.cells_scroll.clamp(0., cols.max_scroll());
                    let tree_top = ui.cursor().min.y;
                    let row_room = if col.mode.is_empty() && nodes_len > 0 {
                        // A tree of tags: the header's eye and lock act on the tags.
                        let header = cols.header(ui, row_left, col.proposal_available, &mut actions);
                        drawing::drawing_drop(ui, &header, &col.nodes, None, col.styles.owner, &mut actions);
                        drawing::drop_on(ui, &header, "", &mut actions);
                        if cols.max_scroll() > 0. {
                            // More cells than fit: where the window is over them.
                            let area = cols.area_rect(header.rect);
                            let w = area.width();
                            let thumb = Rect::from_min_size(
                                pos2(area.min.x + col.cells_scroll / cols.content * w, area.max.y - 3.),
                                vec2(w * w / cols.content, 2.));
                            ui.painter().rect_filled(thumb, CornerRadius::same(1), HOVER_EDGE);
                        }
                        room - ROW_H
                    } else { room };
                    let visible_rows = (row_room / ROW_H).floor().max(3.0);
                    let tree_h = (nodes_len.max(3) as f32).min(visible_rows) * ROW_H;
                    let mut scroll = egui::ScrollArea::vertical().id_salt(("tree", self.fix_rows)).max_height(tree_h).auto_shrink([false, false]);
                    if let Some(key) = col.reveal.take() {
                        let pitch = ROW_H + ui.spacing().item_spacing.y;
                        if let Some(offset) = tree_order::reveal_offset(&col.nodes, &col.ordered, &key, pitch, tree_h) {
                            scroll = scroll.vertical_scroll_offset(offset);
                        }
                    }
                    let by_order = self.fix_rows || col.mode.is_empty();
                    scroll.show_rows(ui, ROW_H, nodes_len, |ui, range| {
                            // A placement's readiness follows the rows above it: those of
                            // the first row drawn are looked up once.
                            let report = &self.panels.data["readiness"];
                            let mut above: Vec<(usize, export::Ready)> = Vec::new();
                            if readiness_shown && range.start < nodes_len {
                                let at = |i: usize| if by_order { order[i] } else { i };
                                let mut depth = col.nodes[at(range.start)].depth;
                                let mut chain = Vec::new();
                                for j in (0..range.start).rev() {
                                    if depth == 0 { break; }
                                    let m = &col.nodes[at(j)];
                                    if m.depth < depth { chain.push(at(j)); depth = m.depth; }
                                }
                                for index in chain.into_iter().rev() {
                                    let m = &col.nodes[index];
                                    let state = export::row_state(report, &m.key, above.last().map(|a| a.1));
                                    above.push((m.depth, state));
                                }
                            }
                            for i in range {
                                let index = if by_order { order[i] } else { i };
                                let n = &col.nodes[index];
                                let editable = n.row == 0 && n.key.starts_with("n:") && n.key.len() > 2 && !n.key.starts_with("n:link:");
                                let tag_row = n.key.starts_with("preview-folder:") || n.key.starts_with("preview-tag:");
                                let sense = if editable || n.local_placement || tag_row { Sense::click_and_drag() } else { Sense::click() };
                                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), sense);
                                let cols = lens::RowLayout::plan(columns, rect.min.x, rect.max.x, col.cells_scroll);
                                // The model's content, then a divider, then its scenes and sheets.
                                if n.depth == 0 && tree_order::document_row(n) && i > 0
                                    && !tree_order::document_row(&col.nodes[if by_order { order[i - 1] } else { i - 1 }]) {
                                    ui.painter().hline(rect.x_range(), rect.min.y, Stroke::new(1.0_f32, EDGE));
                                }
                                // Items in the selection are lit like a picked row; nodes count them.
                                let lit = (tag_row && col.styles.selected(n)) || n.mark == 1 || (selection.is_none() && col.highlighted.as_deref() == Some(n.key.as_str()));
                                let file_state = if n.row == 0 { lens::file_state(&n.icon) } else { None };
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
                                let x0 = cols.lead + 22.0 + n.depth as f32 * 14.0;
                                let tri = Rect::from_min_size(pos2(x0, rect.min.y), vec2(14.0, ROW_H));
                                if n.expandable {
                                    // The one disclosure mark, as on the inspector's sections.
                                    kit::chevron(ui.painter(), tri.center(), n.expanded, if n.row == 0 { DIM } else { FAINT });
                                }
                                // The trailing cells, in their window: they scroll under the pinned name.
                                let area = cols.area_rect(rect);
                                let clip = ui.clip_rect();
                                ui.set_clip_rect(clip.intersect(area));
                                let cell = |i: usize| n.cells.get(i).map_or("", String::as_str);
                                let ink_c = if n.row != 0 { OFF } else { DIM };
                                // Rows that carry a tag's drawing style: tags, and nodes filed by a tag.
                                if n.cells.len() >= 7 {
                                    for (column, index) in [(Col::Weight, 2), (Col::Line, 1), (Col::Hatch, 4)] {
                                        if let Some((a, b)) = cols.cell(column) {
                                            let sample = Rect::from_min_max(pos2(a,cy-6.),pos2(b,cy+6.));
                                            if index == 4 {
                                                kit::hatch_sample(ui, sample.expand2(vec2(0., 2.)), cell(4), n.hatch.as_deref());
                                            } else {
                                                kit::line_sample(ui, sample, cell(2), if index == 1 { cell(1) } else { "continuous" }, ink_c);
                                            }
                                        }
                                    }
                                    if let Some((a, b)) = cols.cell(Col::Print) {
                                        ui.painter().text(pos2((a + b) * 0.5, cy), egui::Align2::CENTER_CENTER, if cell(3) == "1" { "✓" } else { "–" }, meta(), ink_c);
                                    }
                                    if let Some((a,b)) = cols.cell(Col::Colour) {
                                        let sw = Rect::from_center_size(pos2((a+b)*0.5,cy),vec2(12.0,12.0));
                                        kit::tag_swatch(ui, sw, parse_hex(n.cells.get(20).or(n.cells.first()).map_or("", String::as_str)));
                                    }
                                }
                                let locate = file_state == Some("missing");
                                if let Some((_, b)) = cols.cell(Col::Items) {
                                    if !locate && (n.count > 0 || n.row == 0) {
                                        ui.painter().text(pos2(b, cy), egui::Align2::RIGHT_CENTER, n.count.to_string(), meta(), DIM);
                                    }
                                }
                                if let Some((a, b)) = cols.cell(Col::Readiness) {
                                    // Every row says where it stands for the delivery.
                                    while above.last().is_some_and(|row| row.0 >= n.depth) { above.pop(); }
                                    let state = export::row_state(report, &n.key, above.last().map(|row| row.1));
                                    above.push((n.depth, state));
                                    let (label, ink) = state.label(cols.compact);
                                    if cols.compact {
                                        ui.painter().text(pos2((a + b) * 0.5, cy), egui::Align2::CENTER_CENTER, label, meta(), ink);
                                    } else {
                                        ui.painter().text(pos2(b, cy), egui::Align2::RIGHT_CENTER, label, meta(), ink);
                                    }
                                    let cell = Rect::from_min_max(pos2(a, rect.min.y), pos2(b, rect.max.y)).intersect(area);
                                    if resp.hovered() && ui.rect_contains_pointer(cell) {
                                        resp.clone().on_hover_text(state.tip());
                                    }
                                }
                                // The lock keeps the trailing edge; every row that can be locked shows its state.
                                let lock_r = match cols.cell(Col::Lock) {
                                    Some((a, b)) if area.contains(pos2((a + b) * 0.5, cy)) => tree_slots(ui, (a + b) * 0.5 + 44., cy, -1, n.lock, true).1,
                                    _ => Rect::NOTHING,
                                };
                                ui.set_clip_rect(clip);
                                let eye_r = if cols.columns.has(Col::Eye) { Rect::from_center_size(pos2(rect.min.x + 10., cy), vec2(16., ROW_H)) } else { Rect::NOTHING };
                                if n.eye >= 0 && eye_r.is_positive() { kit::paint_row_eye(ui, eye_r, n.eye); }
                                if resp.hovered() {
                                    for (slot, tip) in [(eye_r, &n.control_tips[0]), (lock_r, &n.control_tips[1])] {
                                        if !tip.is_empty() && ui.rect_contains_pointer(slot) {
                                            resp.clone().on_hover_text(tip);
                                        }
                                    }
                                }
                                // The name takes everything up to the cells.
                                let mut right = cols.name_right;
                                if n.problems > 0 && col.issues && !readiness_shown {
                                    let g = ui.painter().layout_no_wrap(n.problems.to_string(), meta(), WARN);
                                    let w = g.size().x;
                                    ui.painter().galley(pos2(right - w, cy - g.size().y / 2.0), g, WARN);
                                    ui.painter().circle_filled(pos2(right - w - 6.0, cy), 2.5, WARN);
                                    right -= w + 14.0;
                                }
                                let locate_right = right;
                                if locate { right -= 50.0; }
                                // Item icon (a material row shows its swatch).
                                let mut tx = x0 + 16.0;
                                if n.row == 5 {
                                    let sw = Rect::from_center_size(pos2(tx + 8.0, cy), vec2(16.0, 16.0));
                                    if let Some(tex) = self_icons.get(&n.icon).filter(|_| !n.icon.is_empty()) {
                                        egui::Image::new(tex).corner_radius(CornerRadius::same(3)).paint_at(ui, sw);
                                    } else if let Some(c) = parse_hex(&n.badge) {
                                        ui.painter().rect_filled(sw, CornerRadius::same(3), c);
                                    } else {
                                        kit::row_glyph(ui.painter(), sw.center(), "component", DIM);
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
                                    // Each kind of row has its own glyph.
                                    kit::row_glyph(ui.painter(), pos2(tx + 6.0, cy), tree_order::row_kind(n),
                                        if n.row == 3 { OFF } else if lit { INK_2 } else { FAINT });
                                    tx += 18.0;
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
                                        if key.starts_with("preview-tag:") && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                            col.styles.pick(n, false);
                                            col.tag_from_node = false;
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
                                let reveal_dot = n.mark == 2 || (n.mark == 1 && (n.row == 6 || n.expandable && !n.local_placement));
                                let marks_w = if file_state.is_some() { 10.0 } else { 0.0 } + if reveal_dot { 10.0 } else { 0.0 };
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
                                        let bounds = ui.painter().text(pos2(locate_right, cy), egui::Align2::RIGHT_CENTER, "Locate…", meta(), ACCENT);
                                        if ui.interact(bounds.expand(3.), ui.id().with(("locate", &n.key)), Sense::click()).clicked() {
                                            actions.push(format!("linkrelink\u{1f}{}", n.key));
                                        }
                                    }
                                    kx += text_width + SPACE_XS;
                                }
                                let dot = Rect::from_center_size(pos2(kx + 2.0, cy), vec2(14.0, ROW_H));
                                if reveal_dot {
                                    lens::paint_partial(ui.painter(), dot.center());
                                    if ui.rect_contains_pointer(dot) {
                                        resp.clone().on_hover_text("Reveal selection");
                                    }
                                    kx += 10.0;
                                }
                                let right = right + marks_w;
                                if !n.badge.is_empty() && n.row != 5 && kx + 20.0 < right {
                                    let col_b = if n.row == 3 { ACCENT } else { FAINT };
                                    let g = ui.painter().layout_job(one_line(n.badge.clone(), meta(), col_b, right - kx));
                                    let w = g.size().x;
                                    // A note cut at the cells shows whole on hover.
                                    if g.elided && resp.hovered() && ui.rect_contains_pointer(Rect::from_min_max(pos2(kx, rect.min.y), pos2(right, rect.max.y))) {
                                        resp.clone().on_hover_text(n.badge.clone());
                                    }
                                    ui.painter().galley(pos2(kx, cy - g.size().y / 2.0), g, col_b);
                                    kx += w + 8.0;
                                }
                                if cols.columns.has(Col::Kind) && !n.kind.is_empty() && kx + 30.0 < right {
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
                                if tag_row {
                                    for (lock, state, slot, verb) in [(false, n.eye, eye_r, "eye"), (true, n.lock, lock_r, "lock")] {
                                        if state < 0 || !slot.is_positive() { continue; }
                                        let hit = ui.interact(slot, resp.id.with(verb), Sense::click_and_drag());
                                        if hit.contains_pointer() && ui.input(|i| i.pointer.primary_pressed()) {
                                            actions.push(col.styles.state_action(n, lock).unwrap_or_else(|| format!("{verb}\u{1f}{}", n.key)));
                                        }
                                    }
                                }
                                let drag_started = resp.drag_started() &&
                                    !ui.input(|i| i.pointer.press_origin().is_some_and(on_control));
                                if tag_row {
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
                                    let extend = ui.input(|i| i.modifiers.command || i.modifiers.shift);
                                    if reveal_dot && dot.contains(p) {
                                        actions.push("revealselection\u{1f}".into());
                                    } else if n.expandable && tri.contains(p) {
                                        actions.push(format!("toggle\u{1f}{}", n.key));
                                    } else if tag_row && on_control(p) {
                                        // A tag's eye and lock own their gesture from the press onward.
                                    } else if n.eye >= 0 && eye_r.expand(2.0).contains(p) {
                                        actions.push(format!("{}\u{1f}{}", if n.local_placement { "roweye" } else { "eye" }, n.key));
                                    } else if n.lock >= 0 && lock_r.expand(2.0).contains(p) {
                                        actions.push(format!("{}\u{1f}{}", if n.local_placement { "rowlock" } else { "lock" }, n.key));
                                    } else if col.filing.is_some() {
                                        if n.row == 0 && !linked {
                                            actions.push(format!("file\u{1f}{}", n.key));
                                            col.filing = None;
                                            col.filter.clear();
                                            actions.push("filter\u{1f}".into());
                                        }
                                    } else if n.key.starts_with("preview-tag:") && n.cells.len() >= 7 {
                                        // A tag: it takes the inspector, with its drawing style.
                                        col.focused_row = Some(n.key.clone());
                                        resp.surrender_focus();
                                        col.styles.pick(n, extend);
                                        col.tag_from_node = false;
                                        col.highlighted = None;
                                    } else if let Some(folder) = n.key.strip_prefix("preview-folder:") {
                                        col.focused_row = None;
                                        col.styles.pick_folder(folder, extend);
                                        col.tag_from_node = false;
                                        col.highlighted = None;
                                    } else if n.row == 9 {
                                        actions.push(format!("toggle\u{1f}{}",n.key));
                                    } else if n.row == 6 {
                                        col.styles.clear_focus();
                                        col.tag_from_node = false;
                                        col.highlighted = Some(n.key.clone());
                                        continue;
                                    } else {
                                        col.styles.clear_focus();
                                        // A node filed by a tag brings that tag's drawing style along.
                                        col.tag_from_node = n.row == 0 && n.cells.len() >= 7 && !linked;
                                        if col.tag_from_node { col.styles.pick(n, false); }
                                        col.highlighted = Some(n.key.clone());
                                        if n.local_placement || n.key.starts_with("section:") {
                                            col.focused_row = Some(n.key.clone());
                                            resp.surrender_focus();
                                        }
                                        let verb = if n.local_placement && extend {
                                            "toggleitem"
                                        } else if n.local_placement { "pickrow" } else { match n.row { 0 => "node", 3 => "reveal", 4 => "loose", 5 => "mat", _ => "item" } };
                                        actions.push(format!("{verb}\u{1f}{}", n.key));
                                    }
                                }
                                if resp.double_clicked() && !resp.interact_pointer_pos().is_some_and(on_control) && (n.row < 6 || n.row == 8) && col.filing.is_none() {
                                    let verb = if n.row == 0 { "isolate" } else { "open" };
                                    actions.push(format!("{verb}\u{1f}{}", n.key));
                                }
                                if resp.secondary_clicked() && (col.mode.is_empty() || n.row != 5 || n.key.starts_with("cm:")) {
                                    col.choosing_folder = false;
                                    col.group_rows.clear();
                                    if n.local_placement || n.key.starts_with("g:") || n.key.starts_with("d:") {
                                        actions.push(format!("grouprows\u{1f}{}", n.key));
                                    }
                                    col.row_menu = resp.interact_pointer_pos().map(|p| (n.key.clone(), n.row, p));
                                }
                            }
                        });
                    // A horizontal scroll over the tree moves the cells under the pinned name.
                    if cols.max_scroll() > 0. {
                        let tree = Rect::from_min_max(pos2(row_left, tree_top), pos2(row_left + ui.available_width(), ui.cursor().min.y));
                        let dx = ui.input(|i| i.smooth_scroll_delta.x);
                        if dx != 0. && ui.rect_contains_pointer(tree) {
                            col.cells_scroll = (col.cells_scroll - dx).clamp(0., cols.max_scroll());
                        }
                    }
                    }
                    let selection = selection.as_ref();
                    // The inspector: what is in focus decides it, never the columns.
                    ui.add_space(SPACE_S);
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0_f32, EDGE));
                    ui.add_space(SPACE_S);
                    ui.spacing_mut().scroll = egui::style::ScrollStyle {
                        foreground_color: true,
                        floating_width: SPACE_XS,
                        ..egui::style::ScrollStyle::thin()
                    };
                    if let Some((title, wire, _)) = &col.prompt {
                        if let Some((id, value)) = wire.strip_prefix("\u{1d}edit\u{1d}").and_then(|v| v.split_once('\u{1d}')) {
                            kit::caption_plain(ui, title);
                            let key = format!("prompt:{id}");
                            let first = !col.edits.contains_key(&key);
                            let text = col.edits.entry(key).or_insert_with(|| value.to_owned());
                            let output = kit::field(text).desired_width(ui.available_width()-12.).show(ui);
                            if first {
                                output.response.request_focus();
                                let mut state = output.state;
                                state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()))));
                                state.store(ui.ctx(), output.response.id);
                            }
                            if output.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                actions.push(format!("prompt\u{1f}yes\u{1f}{text}"));
                                col.prompt = None;
                                col.edits.retain(|key, _| !key.starts_with("prompt:"));
                            }
                            return;
                        }
                    }
                    if let Some(d) = self.dialog.as_mut().filter(|d| d.placement == "inspector") {
                        kit::caption_plain(ui, &d.title);
                        dialog::fields(ui, d, &mut self.actions);
                        return;
                    }
                    // A line type being edited takes the inspector until it is saved or closed.
                    if col.mode.is_empty() && col.styles.catalog.editing_line() {
                        egui::ScrollArea::vertical().id_salt("line type editor").auto_shrink([false, false]).show(ui, |ui| {
                            col.styles.line_editor(ui, &mut actions);
                        });
                        return;
                    }
                    let tag_focus = col.mode.is_empty() && col.styles.active() && !col.tag_from_node;
                    // The subject and what it is on one line, pinned over the details.
                    if tag_focus {
                        let names = col.styles.names();
                        let (title, subtitle) = match names.len() {
                            0 => (col.styles.catalog.folder_label(), "Tag folder".to_owned()),
                            1 => {
                                let key = format!("preview-tag:{}", col.styles.tags()[0]);
                                let items = col.nodes.iter().find(|n| n.key == key).map_or(0, |n| n.count as usize);
                                (names[0].to_owned(), format!("Tag · {}", lens::count(items, "item", "items")))
                            }
                            count => (lens::count(count, "tag", "tags"), names.join(", ")),
                        };
                        kit::inspector_title(ui, &title, &subtitle);
                        ui.add_space(SPACE_XS);
                    } else if !col.inspector_title.is_empty() {
                        let subtitle = selection.map(|(summary, _)| {
                            let measure = match col.inspector_title.as_str() {
                                "Edge" => Some("Length"),
                                "Face" => Some("Area"),
                                _ => None,
                            };
                            col.inspector.iter()
                                .filter(|(section, _)| section == "Geometry")
                                .flat_map(|(_, rows)| rows)
                                .find(|(label, _)| Some(label.as_str()) == measure)
                                .map_or(summary, |(_, value)| value).as_str()
                        });
                        kit::inspector_title(ui, &col.inspector_title, col.inspector_subtitle.as_deref().or(subtitle).unwrap_or(""));
                        ui.add_space(SPACE_XS);
                    } else if col.path.len() > 1 {
                        return;
                    } else {
                        kit::inspector_title(ui, if col.overview.file.is_empty() { "Untitled" } else { &col.overview.file }, "Model");
                        ui.add_space(SPACE_XS);
                    }
                    if !tag_focus && !col.inspector_title.is_empty() {
                        let inline = |id: &str| matches!(id, "fileto" | "isolate" | "hide" | "open" | "linkopen");
                        let material_inline = |id: &str| id.starts_with("lib:select:") || id == "lib:browse:materials";
                        let curated: Vec<_> = col.inspector.iter().filter(|(title,_)|title=="Actions").flat_map(|(_,rows)| rows).collect();
                        if let Some((_, verbs)) = col.inspector.iter().find(|(title, _)| title == "Component actions") {
                            ui.horizontal_wrapped(|ui| {
                                for (id, label) in verbs {
                                    if kit::quiet(ui, label).clicked() { actions.push(format!("insp:{id}")); }
                                }
                            });
                        }
                        ui.horizontal_wrapped(|ui| {
                            if let Some((_,acts)) = selection {
                                for (id,label,_,enabled,_) in acts.iter().filter(|a| inline(&a.0)) {
                                    if *enabled && kit::quiet(ui,label).clicked() { cmds.push(format!("cmd:{id}")); }
                                }
                            }
                            for (id, label) in curated.iter().copied().filter(|(id, _)| material_inline(id)) {
                                if kit::quiet(ui, label).clicked() { actions.push(format!("insp:{id}")); }
                            }
                            if selection.is_some() || curated.iter().any(|(id, _)| !material_inline(id)) {
                                egui::ComboBox::from_id_salt("inspector actions").selected_text("Actions").show_ui(ui, |ui| {
                                    for (id,label) in curated.iter().copied().filter(|(id, _)| !material_inline(id)) {
                                        if ui.selectable_label(false,label.as_str()).clicked() {
                                            if let Some(key) = id.strip_prefix("kindmenu:") { col.kind_menu = Some((key.into(),ui.cursor().min)); }
                                            else { actions.push(format!("insp:{id}")); }
                                        }
                                    }
                                    if let Some((_,acts)) = selection {
                                        for (id,label,_,enabled,_) in acts.iter().filter(|a| !inline(&a.0)) {
                                            if ui.add_enabled(*enabled,egui::Button::new(label.as_str()).frame(false)).clicked() {
                                                cmds.push(format!("cmd:{id}"));
                                            }
                                        }
                                    }
                                });
                            }
                        });
                        ui.add_space(SPACE_XS);
                    }
                    ui.spacing_mut().scroll.floating = false;
                    egui::ScrollArea::vertical().id_salt("inspector").auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing = vec2(0.0, SPACE_XS);
                        if tag_focus {
                            // A tag (or several): its verbs and its drawing style, under every preset.
                            let tags = col.styles.tags();
                            if let [tag] = tags.as_slice() {
                                let key = format!("preview-tag:{tag}");
                                if let Some(at) = kit::tag_inspector(ui, &key, col.proposal_available, &mut actions) {
                                    col.kind_menu = Some((key, at));
                                }
                            } else if tags.len() > 1 && kit::quiet(ui, "Isolate").clicked() {
                                let keys: Vec<_> = tags.iter().map(|tag| format!("preview-tag:{tag}")).collect();
                                actions.push(format!("isolatetags\u{1f}{}", serde_json::json!(keys)));
                            }
                            if col.styles.folder_focus() {
                                if !tags.is_empty() { caption(ui, "Tag folder"); }
                                col.styles.folder_editor(ui, &mut actions);
                            }
                            col.styles.style(ui, &mut actions);
                            return;
                        }
                        if col.inspector_title.is_empty() {
                            // Nothing selected: the document. Its counts, what to fix
                            // before a delivery, and what belongs to the whole model.
                            if col.proposal_available {
                                kit::prop_row(ui,"Tags",&format!("{} · {}", lens::count(col.overview.tags, "tag", "tags"), lens::count(col.overview.folders, "folder", "folders")));
                            } else {
                                kit::prop_row(ui,"Filing",&format!("{} · {}", lens::count(col.overview.nodes, "node", "nodes"), lens::count(col.overview.placements, "item", "items")));
                                kit::prop_row(ui, "Not filed", &col.unfiled.to_string());
                            }
                            if col.overview.links>0 {kit::prop_row(ui,"Linked files",&col.overview.links.to_string());}
                            ui.add_space(SPACE_XS);
                            if col.mode.is_empty() {
                                col.styles.document(ui, &mut actions);
                            }
                            return;
                        }
                        kit::identity_card(ui, &col.identity, &mut actions);
                        if col.mode.is_empty() {
                            self.panels.inspector(ui, &mut cmds);
                        }
                        let native_export = self.panels.has_records() && col.mode.is_empty();
                        if native_export {
                            let measured = col.inspector.iter().find(|(title,_)|title=="Quantities").map_or(&[][..],|(_,rows)|rows.as_slice());
                            kit::disclosure(ui, "export details", "Export details", false, |ui| self.panels.export(ui, &mut cmds, measured));
                        }
                        let sections = col.inspector.clone();
                        for (title, rows) in &sections {
                            if matches!(title.as_str(), "Actions" | "Component actions") || (title == "Solid" && selection.is_some() && self.plugins.registered("solid_inspector")) { continue; }
                            if matches!(title.as_str(),"Quantities" | "Export details") && native_export { continue; }
                            if title.starts_with("Export") || matches!(title.as_str(), "What it needs" | "Solid") {
                                ui.add_space(SPACE_XS);
                                kit::disclosure(ui, title, title, false,
                                    |ui| inspector_rows(ui, rows, &mut col.edits, &mut actions));
                            } else {
                                ui.add_space(SPACE_S);
                                caption(ui, title);
                                inspector_rows(ui, rows, &mut col.edits, &mut actions);
                            }
                        }
                        if let Some((summary, _)) = selection.filter(|_| self.plugins.registered("solid_inspector")) {
                            ui.add_space(SPACE_S);
                            caption(ui, "Health");
                            kit::prop_row(ui, "Selection", summary);
                            if let Some(count) = self.panels.data["selection"]["health_faces"].as_u64() {
                                kit::prop_row(ui, "Faces", &count.to_string());
                            }
                            for (_, counts) in sections.iter().filter(|(title, _)| title == "Solid") {
                                inspector_rows(ui, counts, &mut col.edits, &mut actions);
                            }
                            if kit::row_link(ui, "Show in drawer").clicked() {
                                cmds.push("drawer:selection_health".into());
                            }
                        }
                        // A node filed by a tag: that tag's drawing style, as a section.
                        if col.mode.is_empty() && col.tag_from_node && col.styles.active() && sections.iter().any(|(title, _)| title == "Node") {
                            ui.add_space(SPACE_XS);
                            kit::disclosure(ui, "node drawing style", "Drawing style of its tag", false, |ui| col.styles.style(ui, &mut actions));
                        }
                    });
                });
            });
        tree_rows::show_menu(col, ctx, height, &mut actions);
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
        dock_menus::show(col, ctx, screen, selection.is_some(), &mut actions);
        // The document is asked for the export checks only while something shows them.
        let wanted = self.destination == Destination::Deliveries || (self.destination == Destination::Project && (self.fix_rows || col.columns.has(Col::Readiness)));
        if self.fix_rows && (self.destination == Destination::Deliveries ||
            actions.iter().any(|action| action.starts_with("lens\u{1f}") || action.starts_with("columns\u{1f}"))) {
            self.fix_rows = false;
            actions.push("fixrows\u{1f}0".into());
        }
        if col.readiness_sent != Some(wanted) {
            col.readiness_sent = Some(wanted);
            actions.push(format!("readiness\u{1f}{}", u8::from(wanted)));
        }
        if let Some((title, body_text, buttons)) = col.prompt.clone().filter(|(_, body, _)| !body.starts_with("\u{1d}edit\u{1d}")) {
            let mut answered = false;
            egui::Area::new(egui::Id::new("prompt"))
                .pivot(egui::Align2::CENTER_TOP)
                .fixed_pos(anchor)
                .order(egui::Order::Tooltip)
                .show(ctx, |ui| {
                    glass(Surface::Raised).inner_margin(Margin::same(14)).show(ui, |ui| {
                        ui.set_width(W - 40.0);
                        ui.label(egui::RichText::new(&title).font(title_font()).color(INK));
                        ui.add_space(SPACE_S);
                        ui.label(egui::RichText::new(&body_text).font(body()).color(DIM));
                        ui.add_space(SPACE_M);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing = vec2(SPACE_S, 0.0);
                            for (i, (id, label)) in buttons.iter().enumerate() {
                                let hint = if i == 0 { "↩" } else if matches!(id.as_str(), "cancel" | "no") { "esc" } else { "" };
                                if sheet_button(ui, label, hint, u8::from(i == 0)) {
                                    actions.push(format!("prompt\u{1f}{id}"));
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
                    let max_w = dock_width(DOCK_MAX_W, screen.width()) - DESTINATION_STRIP_W;
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
                                "error" | "warning" | "info" | "success" => notice_mark(&sheet.tone),
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
                        for (id, label, detail, role) in sheet.buttons.iter().filter(|b| matches!(b.3, 6 | 7)) {
                            if kit::disclosure_toggle(ui, label, *role == 7) { clicked = Some(id.clone()); }
                            if *role == 7 {
                                egui::ScrollArea::vertical().max_height(screen.height() * 0.35).show(ui, |ui| {
                                    ui.add(egui::Label::new(egui::RichText::new(detail).font(body()).color(DIM)).wrap());
                                });
                            }
                        }
                        if sheet.buttons.iter().any(|b| matches!(b.3, 4 | 5)) {
                            egui::ScrollArea::vertical().max_height(screen.height() * 0.45).show(ui, |ui| {
                                for (id, label, _, role) in sheet.buttons.iter().filter(|b| matches!(b.3, 4 | 5)) {
                                    let mut checked = *role == 5;
                                    if ui.push_id(id, |ui| kit::toggle(ui, &mut checked, label)).inner.changed() {
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
        let bottom = if !self.review && self.drawer_mode().is_some() {
            ctx.memory(|m| m.area_rect(egui::Id::new("drawer"))).map_or(bottom, |r| bottom.min(r.top() - GUT))
        } else { bottom };
        let free = Rect::from_min_max(pos2(left, TOP_H + GUT), pos2(right, bottom));
        let (back, live) = self.cursor.place(free, vec2(170.0, 30.0));
        let pill = |id: &str, r: Rect, caption: &str, value: &str, typing: bool, committed: bool, error: bool| {
            egui::Area::new(egui::Id::new(id))
                .fixed_pos(r.min)
                .order(egui::Order::Tooltip)
                .interactable(false)
                .show(ctx, |ui| kit::value_pill_tone(ui, caption, value, typing, committed, error));
        };
        if let (Some(r), Some(b)) = (back, &self.cursor.readback) {
            pill("value_readback", r, &b.caption, &b.value, false, true, false);
        }
        if let (Some(r), Some(l)) = (live, &self.cursor.live) {
            if screen.contains(l.at) {
                if let Some(rings)=&self.section_inset{
                    let center=pos2((l.at.x+45.).clamp(free.left(),free.right().max(free.left())),(l.at.y-50.).max(free.top()+32.));
                    let painter=ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip,egui::Id::new("section inset")));
                    painter.rect_filled(Rect::from_center_size(center,vec2(64.,64.)),6.,tokens::POPOVER);
                    for ring in rings {
                        painter.add(egui::Shape::closed_line(ring.iter().map(|p|center+vec2(p[0],p[1])).collect(),egui::Stroke::new(1.6_f32,tokens::INK)));
                    }
                }
                pill("cursor_value", r, &l.caption, &l.value, l.typing, false, self.cursor_error && !l.typing);
            }
        }
    }

    /// The keystroke pill at the bottom centre of the free area, above the
    /// bottom bar and the Paint strip, moved up over the value field when
    /// they would meet (it never covers the status bar or the field).
    fn keystrokes(&self, ctx: &egui::Context, screen: Rect) {
        if !crate::keycast::shown() { return; }
        let now = self.start.elapsed().as_secs_f64();
        let Some(opacity) = self.keycast.opacity(now) else { return };
        if let Some(ms) = self.keycast.repaint_ms(now) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(ms / 1000.));
        }
        let left = if self.review { GUT } else { GUT + self.rail_w + GUT };
        let right = if self.review { screen.max.x - GUT } else { self.free_right(ctx, screen) - GUT };
        let (_, mut bottom) = self.band(ctx, screen);
        let area = |name: &str| ctx.memory(|m| m.area_rect(egui::Id::new(name)));
        if self.paint_strip_shown() && !self.review {
            if let Some(r) = area("paint_strip") { bottom = bottom.min(r.top() - GUT); }
        }
        let size = area("keystrokes").map_or(vec2(160., 36.), |r| r.size());
        let centre = (left + right) / 2.0;
        let mut pill = Rect::from_center_size(pos2(centre, bottom - size.y / 2.0), size);
        for name in ["cursor_value", "value_readback"] {
            if let Some(field) = area(name).filter(|r| r.expand(GUT / 2.0).intersects(pill)) {
                pill = pill.translate(vec2(0., field.top() - GUT - pill.bottom()));
            }
        }
        egui::Area::new(egui::Id::new("keystrokes"))
            .pivot(egui::Align2::CENTER_BOTTOM)
            .fixed_pos(pos2(centre, pill.bottom()))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| kit::keystroke_pill(ui, &self.keycast.chips, opacity));
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
                    painter.rect_stroke(rect, 0., Stroke::new(1_f32, if active { ACCENT } else { EDGE }),
                                        egui::StrokeKind::Inside);
                }
                for &[x0,y0,x1,y1] in &self.model_guides {
                    painter.line_segment([pos2(x0,y0),pos2(x1,y1)], Stroke::new(2_f32, ACCENT));
                }
            }
            self.top_bar(ctx, screen);
            let retract = ctx.animate_bool_with_time(egui::Id::new("retract_model_chrome"),
                                                      self.review || sheet_up, large_sheet::ANIM_S);
            if self.sketch.shown && retract < 1.0 {
                self.sketch_frame(ctx, screen);
            } else if self.sheets.shown {
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
                    self.draw_drawer(ctx, screen);
                    if self.drawer_mode() == Some(DrawerMode::Pinned) {
                        if let Some(at) = ctx.data(|d| d.get_temp::<Pos2>("drawer-close".into())) {
                            self.cluster_buttons.push(("drawer-close".into(), at.into()));
                        }
                    }
                    self.paint_strip(ctx, screen);
                    if self.panels.data["terrain"].is_object() {
                        let bounds = Rect::from_min_max(pos2(12.,0.),pos2(self.free_right(ctx,screen)-8.,screen.bottom()-BOT_H-4.));
                        self.panels.terrain.draw(ctx,bounds,&self.panels.data["terrain"],&self.panels.data["projected"],&mut self.actions);
                    }
                    self.panels.frame_guide(ctx, screen);
                }
                if !self.review && !sheet_up { self.junction_review_panel(ctx, screen); }
                if !sheet_up { self.cursor_value(ctx, screen); }
                if !sheet_up { self.keystrokes(ctx, screen); }
                if !self.review && !sheet_up { self.task_pins(ctx, screen); }
                self.view_overlay(ctx);
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
            self.jobs_chip(ctx, screen);
            self.notices(ctx, screen);
            self.sheet_popup(ctx, screen);
            let anchor = self.form_anchor(ctx, screen);
            if let Some(picker) = &mut self.font_picker {
                if let Some(apply) = picker.popup(ctx, anchor) {
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
        let model = !self.sheets.shown && !self.sketch.shown && !sheet_up;
        let shown = [("top", true), ("palette", work && model && !self.groups.is_empty()),
                     ("view_menu", self.view_open), ("menu", self.menu.is_some()), ("form", self.form.is_some()),
                     ("drawer", work && model && self.drawer_mode().is_some()), ("column", work && model && self.column.is_some()), ("bottom", work),
                     ("paint_strip", work && model && self.paint_strip_shown()),
                     ("commands", self.cmd_open),
                     ("row_menu", model && self.column.as_ref().is_some_and(|c| c.row_menu.is_some())),
                     ("kind_menu", model && self.column.as_ref().is_some_and(|c| c.kind_menu.is_some())),
                     ("head_menu", model && self.column.as_ref().is_some_and(|c| c.head_menu.is_some())),
                     ("prompt", model && self.column.as_ref().is_some_and(|c| c.prompt.is_some())),
                     ("terrain_profile", work && model && self.panels.data["terrain"].is_object()),
                     ("jobs", !self.jobs.is_empty() || self.activity_open),
                     ("notices", self.history.iter().any(|r| r.5 > 0.)),
                     ("junction_review", work && model && self.junction_review.is_some()),
                     ("dialog", self.dialog.as_ref().is_some_and(|d| d.placement.is_empty() && !d.in_drawer())),
                     ("tab_menu", self.strip.menu_open()),
                     ("tab_overflow", self.strip.overflow_open()),
                     ("ctx_menu", self.ctx_menu.is_some()),
                     ("sheet", self.sheet.is_some()),
                     ("dock_grip", work && model && self.column.is_some()),
                     ("insp_color", work && model && self.column.as_ref().is_some_and(|c| c.edits.contains_key("\u{1d}pick")))];
        let sheet_areas = self.sheets_areas();
        let sketch_areas = self.sketch_areas();
        let large = self.lsheet_areas();
        self.hit = shown.iter().chain(sheet_areas.iter()).chain(sketch_areas.iter()).chain(large.iter())
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
    if st == State::On { paint_lit(ui, rect, R_CONTROL) } else { paint_state(ui, rect, st, R_CONTROL) }
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

/// Inspector rows: label left, value right; a value written
/// "\u{1d}edit\u{1d}<id>\u{1d}<text>" is an editable field (Enter applies).
fn inspector_rows(ui: &mut egui::Ui, rows: &[(String, String)], edits: &mut std::collections::HashMap<String, String>,
                  actions: &mut Vec<String>) {
    for (label, value) in rows {
        if let Some((id, name)) = value.strip_prefix("\u{1d}imagefile\u{1d}").and_then(|rest| rest.split_once('\u{1d}')) {
            if kit::image_file_row(ui, label, name) { actions.push(format!("insp:{id}")); }
            continue;
        }
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
                if let Some(i) = kit::segmented(ui, &labels, at, w) {
                    if i != at { actions.push(format!("edit\u{1f}{id}\u{1f}{}", labels[i])); }
                }
            });
            continue;
        }
        // "\u{1d}color\u{1d}<id>\u{1d}#rrggbb": a swatch; a click opens the picker.
        if let Some(rest) = value.strip_prefix("\u{1d}color\u{1d}") {
            let (id, hex) = rest.split_once('\u{1d}').unwrap_or((rest, ""));
            let c = parse_hex(hex).unwrap_or(SWATCH_FALLBACK);
            let mut rgb = [c.r(), c.g(), c.b()];
            kit::property_control(ui, label, |ui| {
                if kit::colour_well(ui, &mut rgb).changed() {
                    actions.push(format!("edit\u{1f}{id}\u{1f}#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]));
                }
            });
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

/// The View menu's scrolling height from `top` (its groups' first row)
/// down to the bottom bar, less a gutter and the menu's own margin.
fn view_menu_room(screen: Rect, top: f32) -> f32 {
    (screen.bottom() - BOT_H - GUT - 6.0 - top).max(ROW_H * 3.)
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
fn tool_button(ui: &mut egui::Ui, tool: &Tool, active: bool, mark: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), if tool.enabled { Sense::click() } else { Sense::hover() });
    let st = state_of(&resp, active, tool.enabled);
    if st == State::On { paint_lit(ui, rect, R_CONTROL) } else { paint_state(ui, rect, st, R_CONTROL) }
    let tint = ink_of(st);
    if !kit::solid_glyph(ui.painter(), rect.shrink(3.), &tool.id, tint) {
    match &tool.icon {
        Some(tex) => {
            egui::Image::new(tex).tint(tint).paint_at(ui, rect.shrink(3.0));
        }
        None => {
            // No art: the rail's unique, language-stable mark (rail::fallback_mark).
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, mark, meta(), tint);
        }
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
fn organiser_uses_a_dismissible_sheet() {
    let Some(mut ui) = test_chrome() else { return };
    ui.menu = Some(("scenes".into(), Rect::ZERO));
    ui.organise_scenes();
    assert!(ui.menu.is_none());
    assert_eq!(ui.large_sheet_state().0, "scene-organiser");
    assert!(ui.lsheet_key("Escape", true, 0));
    assert!(!ui.large_sheet_shown());
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
fn the_view_menu_reaches_down_to_the_bottom_bar() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    let (cluster, menu) = crate::commands::command_chrome(Default::default(), Default::default(), Default::default());
    ui.set_cluster(cluster, menu);
    for (w, h) in [(1600., 1000.), (1280., 800.)] {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(w, h));
        ui.screen = screen;
        ui.view_open = true;
        ui.view_anchor = Rect::from_min_size(pos2(w - 80., 6.), vec2(70., 28.));
        let ctx = ui.ctx.clone();
        for _ in 0..3 {
            let raw = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            let _ = ctx.run(raw, |c| ui.view_popup(c));
        }
        let r = ctx.memory(|m| m.area_rect(egui::Id::new("view_menu"))).unwrap();
        // Every row fits at 1000 high; either way the menu stops at the bottom bar.
        assert!(r.max.y <= h - BOT_H - GUT + 0.5, "{h}: ends at {}", r.max.y);
        assert!(r.height() > 450., "{h}: only {} high", r.height());
    }
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
    // A tag in focus in the dock is left first.
    let tag = Node { key: "preview-tag:walls".into(), name: "Walls".into(), cells: vec!["walls".into(); 7], ..Default::default() };
    ui.column.as_mut().unwrap().styles.pick(&tag, false);
    assert_eq!(Ui::model_escape(Some(&mut ui), false, false, false, false), "dock");
    assert!(!ui.column.as_ref().unwrap().styles.active() && ui.review);
    // The Export columns are a preset, not a mode: Escape does not leave them.
    assert_eq!(Ui::model_escape(Some(&mut ui), false, false, false, false), "");
    assert!(!ui.take_actions().iter().any(|action| action.starts_with("tree:lens")));
    assert_eq!(ui.column.as_ref().unwrap().columns.name(), "Export");
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
    assert!((px(x1) - f64::from(1280. - GUT - 320. - DESTINATION_STRIP_W)).abs() < 0.01);
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
    let frame = ctx.run(raw, |c| {
        ui.right_column(c, screen, 0.);
        ui.bar_menu(c);
    });
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
    let at = tree_frame(&mut ui, vec![])["Explode"];
    tree_click(&mut ui, at, egui::PointerButton::Primary);
    assert!(ui
        .take_actions()
        .contains(&format!("tree:explode\u{1f}{key}")));
    assert!(ui.column.unwrap().row_menu.is_none());
}

#[test]
fn only_eye_and_lock_headers_dispatch_and_target_the_trees_rows_under_every_preset() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    ui.set_tree(vec![(vec!["i:sample".into(), "Sample".into()], vec![0, 0, 0, 0, 1, 1, 0])], 0);
    // A tree of tags: the header acts on the tags. Otherwise on the top-level
    // groups. The columns shown do not change what the header means.
    for (lens, tags, scope) in [(Lens::Filing, false, "placements"), (Lens::Drawing, false, "placements"),
                                (Lens::Filing, true, "tags"), (Lens::Drawing, true, "tags")] {
        ui.column.as_mut().unwrap().columns = lens::Columns::preset(lens);
        ui.set_proposal_available(tags);
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
    ui.column.as_mut().unwrap().columns = lens::Columns::preset(Lens::Drawing);
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
    ui.column.as_mut().unwrap().columns = lens::Columns::preset(Lens::Drawing);
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
        let action = format!("tree:{}", col.rename_request(false));
        // A verb on the whole model: in the Model menu, not the tree's.
        ui.menu = Some(("work".into(), Rect::from_min_size(pos2(300., 8.), vec2(60., 24.))));
        for _ in 0..3 { tree_frame(&mut ui, vec![]); }
        let p = tree_frame(&mut ui, vec![])[label];
        tree_click(&mut ui, p, egui::PointerButton::Primary);
        if empty { assert!(ui.take_actions().is_empty()); }
        else { assert_eq!(ui.take_actions(), [action]); }
    }
}

#[test]
fn tag_proxy_reveals_selection_and_its_filing_menu_targets_the_tag() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    ui.set_tree(vec![(vec!["preview-tag:oak".into(), "Oak".into()], vec![0, 1, 0, 0, 6, 1, 0])], 1);
    assert!(ui.mark_tag_paths(vec![vec!["preview-tag:oak".into()]], 1));
    for _ in 0..3 { tree_frame(&mut ui, vec![]); }
    let name = tree_frame(&mut ui, vec![])["Oak"];
    let half_width = ui.ctx.fonts(|fonts| fonts.layout_no_wrap("Oak".into(), body(), INK).size().x / 2.);
    tree_click(&mut ui, name + vec2(half_width + 10., 0.), egui::PointerButton::Primary);
    assert_eq!(ui.take_actions(), ["tree:revealselection\x1f"]);
    tree_click(&mut ui, name, egui::PointerButton::Secondary);
    for _ in 0..3 { tree_frame(&mut ui, vec![]); }
    let menu = tree_frame(&mut ui, vec![]);
    for label in ["New tag", "Rename…", "Hide", "Move to folder…", "Merge into…", "Delete"] {
        assert!(menu.contains_key(label), "{label}");
    }
    tree_click(&mut ui, menu["Select items"], egui::PointerButton::Primary);
    assert_eq!(ui.take_actions(), ["tree:tagselect\x1fpreview-tag:oak"]);
    assert!(ui.mark_tag_paths(vec![], 0));
    assert_eq!(ui.column.unwrap().nodes[0].mark, 0);
}

#[test]
fn scrolling_inspector_details_keeps_the_title_visible() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    ui.set_tree(Vec::new(), 0);
    ui.set_inspector("Selected group".into(), vec![("Item".into(),
        (0..70).map(|i| (format!("Field {i}"), format!("Value {i}"))).collect())]);
    for _ in 0..3 { tree_frame(&mut ui, vec![]); }
    let before = tree_frame(&mut ui, vec![]);
    let at = before["Value 0"];
    tree_frame(&mut ui, vec![egui::Event::PointerMoved(at), egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point, delta: vec2(0., -450.), modifiers: Default::default()
    }]);
    for _ in 0..6 { tree_frame(&mut ui, vec![]); }
    let after = tree_frame(&mut ui, vec![]);
    assert_eq!(before["Selected group"], after["Selected group"]);
    assert!(!after.contains_key("Value 0"));
}

#[test]
fn each_project_row_menu_offers_actions_for_its_target() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    for (key, kind, verb) in [
        ("preview-folder:wood", 6, "New tag in folder"),
        ("section:cut", 7, "Name and symbol…"),
        ("saved-scene:view", 8, "Show scene"),
        ("sheet-page:sheet", 8, "Open sheet"),
        ("geometry:loose", 7, "Select items"),
    ] {
        ui.set_tree(vec![(vec![key.into(), "Target".into()], vec![0, 0, 0, 0, kind, 1, 0])], 0);
        for _ in 0..3 { tree_frame(&mut ui, vec![]); }
        let at = tree_frame(&mut ui, vec![])["Target"];
        tree_click(&mut ui, at, egui::PointerButton::Secondary);
        for _ in 0..3 { tree_frame(&mut ui, vec![]); }
        assert!(tree_frame(&mut ui, vec![]).contains_key(verb), "{key}");
        ui.column.as_mut().unwrap().row_menu = None;
    }
}

/// Tag rows as the document sends them: the key in cell 5, drawing cells behind.
#[cfg(test)]
fn tag_rows(names: &[&str]) -> Vec<(Vec<String>, Vec<i64>)> {
    names.iter().map(|name| {
        let mut text = vec![String::new(); 26];
        text[0] = format!("preview-tag:{name}");
        text[1] = (*name).into();
        text[8] = "1".into();
        text[10] = (*name).into();
        (text, vec![0, 3, 0, 0, 6, 1, 0])
    }).collect()
}

/// The texts the inspector shows: everything under the tree's last row.
#[cfg(test)]
fn inspector_texts(frame: &std::collections::HashMap<String, Pos2>, under: f32) -> Vec<String> {
    let mut texts: Vec<_> = frame.iter().filter(|(_, at)| at.y > under).map(|(text, _)| text.clone()).collect();
    texts.sort();
    texts
}

#[test]
fn a_plain_name_filter_reports_the_visible_row_count() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    ui.set_tree(tag_rows(&["Walls"]), 0);
    ui.column.as_mut().unwrap().filter = "wall".into();
    for _ in 0..3 { tree_frame(&mut ui, vec![]); }
    assert!(tree_frame(&mut ui, vec![]).contains_key("1 row shown"));
}

#[test]
fn a_column_preset_keeps_the_width_the_rows_the_focus_and_the_inspector() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    ui.set_tree(tag_rows(&["Walls", "Doors"]), 0);
    ui.set_proposal_available(true);
    ui.set_dock_width(340.);
    for _ in 0..3 { tree_frame(&mut ui, vec![]); }
    let at = tree_frame(&mut ui, vec![])["Walls"];
    tree_click(&mut ui, at, egui::PointerButton::Primary);
    ui.take_actions();
    let mut seen = Vec::new();
    for lens in [0, 1, 2, 0] {
        ui.set_lens(lens);
        for _ in 0..3 { tree_frame(&mut ui, vec![]); }
        let frame = tree_frame(&mut ui, vec![]);
        let dock = ui.ctx.memory(|m| m.area_rect(egui::Id::new("column"))).unwrap();
        // The tag stays in focus and its drawing style is in the inspector under every preset.
        assert!(ui.column.as_ref().unwrap().styles.active(), "preset {lens}");
        for text in ["Drawing style", "Line type", "Print", "Select items"] {
            assert!(frame.contains_key(text), "{text} under preset {lens}");
        }
        // What belongs to the document is not in one tag's editor, and nothing waits for an Apply.
        for text in ["Tag folders", "Line types", "Import hatch patterns…", "Change a field to apply."] {
            assert!(!frame.contains_key(text), "{text} under preset {lens}");
        }
        seen.push((dock.width(), frame["Walls"], frame["Doors"], inspector_texts(&frame, frame["Doors"].y + ROW_H)));
    }
    // One width, the rows where they were, the same inspector.
    assert!(seen.iter().all(|state| state == &seen[0]), "{seen:?}");
    assert_eq!(seen[0].0, 340. + DESTINATION_STRIP_W);
}

#[test]
fn a_selection_has_one_inspector_under_every_preset() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    ui.set_tree(tag_rows(&["Walls"]), 0);
    ui.set_selection("1 group".into(), vec![("hide".into(), "Hide".into(), String::new(), true, false)]);
    ui.set_identity(vec![("Kind".into(), "Group".into(), String::new(), None)]);
    ui.set_inspector("Bench".into(), vec![
        ("Item".into(), vec![("Name".into(), "Bench".into())]),
        ("Actions".into(), vec![("cutlist".into(), "Cut list…".into())]),
        ("Solid".into(), vec![("Watertight".into(), "0.061 m³".into())]),
    ]);
    let mut seen = Vec::new();
    for lens in [0, 1, 2] {
        ui.set_lens(lens);
        for _ in 0..3 { tree_frame(&mut ui, vec![]); }
        let frame = tree_frame(&mut ui, vec![]);
        // Export used to drop the name, the count, Hide and Actions.
        for text in ["Bench", "1 group", "Name", "Hide", "Actions", "Solid"] {
            assert!(frame.contains_key(text), "{text} under preset {lens}");
        }
        seen.push(inspector_texts(&frame, frame["Walls"].y + ROW_H));
    }
    assert!(seen[1] == seen[0] && seen[2] == seen[0], "{seen:?}");
}

#[test]
fn destination_round_trip_preserves_project_state_and_width() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    ui.set_tree(tag_rows(&["Walls", "Ceiling"]), 0);
    ui.set_dock_width(345.);
    let frame = |ui: &mut Ui| {
        for _ in 0..3 { tree_frame(ui, vec![]); }
        tree_frame(ui, vec![])
    };
    let project = frame(&mut ui);
    assert!(project.contains_key("Walls") && !project.contains_key("Fix checklist"));
    let column_rect = |ui: &Ui| ui.ctx.memory(|m| m.area_rect(egui::Id::new("column")).unwrap());
    let width = column_rect(&ui).width();
    ui.show_destination("tasks");
    assert!(frame(&mut ui).contains_key("No tasks yet"));
    assert_eq!(column_rect(&ui).width(), width);
    ui.show_destination("deliveries");
    frame(&mut ui);
    assert_eq!(ui.take_actions(), ["tree:readiness\u{1f}1"]);
    ui.panels.set("readiness", serde_json::json!({
        "active": "d", "choices": [["d", "Handover"]], "ready": 0, "total": 3,
        "rows": {"preview-tag:Walls": [3, 3]}, "items": {}, "notes": ["Coverage detail"],
        "checks": {
            "*": {"unfiled/": ["Not filed", [{"path": ["a"]}, {"path": ["b"]}, {"path": ["c"]}]]},
            "preview-tag:Walls": {"unfiled/": ["Not filed", [{"path": ["a"]}]]}
        },
    }));
    let delivery = frame(&mut ui);
    for label in ["READY TO DELIVER", "⚠ 1 issue", "3 elements not filed", "What is checked", "KEPT", "Handover", "+ New", "!"] {
        assert!(delivery.contains_key(label), "{label}");
    }
    assert!(!delivery.contains_key("Delete delivery"));
    tree_click(&mut ui, delivery["⋯"], egui::PointerButton::Primary);
    let menu = frame(&mut ui);
    tree_click(&mut ui, menu["Delete delivery"], egui::PointerButton::Primary);
    assert!(ui.take_actions().iter().any(|a| a == "deliver:delete:d"));
    assert!(!delivery.contains_key("Coverage detail"));
    tree_click(&mut ui, delivery["What is checked"], egui::PointerButton::Primary);
    assert!(frame(&mut ui).contains_key("Coverage detail"));
    tree_click(&mut ui, delivery["Show ›"], egui::PointerButton::Primary);
    let narrowed = frame(&mut ui);
    assert!(narrowed.contains_key("Walls") && !narrowed.contains_key("Ceiling"));
    tree_click(&mut ui, narrowed["← Back to Deliveries"], egui::PointerButton::Primary);
    assert!(frame(&mut ui).contains_key("Handover"));
    ui.panels.set("readiness", serde_json::json!({"ready": 0, "total": 0}));
    let ready = frame(&mut ui);
    assert!(ready.contains_key("✓ Ready") && ready.contains_key("No deliveries yet"));
    assert!(!ready.contains_key("Show ›"));
    ui.take_actions();
    tree_click(&mut ui, ready["+ New"], egui::PointerButton::Primary);
    assert!(ui.take_actions().iter().any(|action| action.contains("deliver:new")));
    ui.show_destination("project");
    assert!(frame(&mut ui).contains_key("Ceiling"));
    assert_eq!(column_rect(&ui).width(), width);
}

#[test]
fn tasks_destination_groups_tasks_by_what_they_need_and_queues_their_verbs() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    ui.set_tree(tag_rows(&["Walls"]), 0);
    let frame = |ui: &mut Ui| {
        for _ in 0..3 { tree_frame(ui, vec![]); }
        tree_frame(ui, vec![])
    };
    ui.show_destination("tasks");
    let empty = frame(&mut ui);
    assert!(empty.contains_key("No tasks yet") && empty.contains_key("New task"), "{:?}", empty.keys());
    let row = |guid: &str, title: &str, label: &str, tone: &str, count: u32| serde_json::json!({
        "guid": guid, "title": title, "agent": "Codex", "label": label, "tone": tone, "count": count});
    ui.panels.set("tasks", serde_json::json!({
        "unresolved": 2, "selected": "t1", "empty": false, "access": true, "selection": 0,
        "agents": ["Built-in assistant", "Codex"],
        "groups": [
            {"name": "Needs attention", "rows": [row("t1", "Align the backrest slats", "To review", "warn", 3), row("t2", "Check clearance", "Question", "warn", 0)]},
            {"name": "In progress", "rows": [row("t3", "Check sheet callouts", "Working", "dim", 0)]},
            {"name": "Settled", "rows": [row("t4", "Name the groups", "Done", "ok", 1)]},
        ],
        "detail": {"guid": "t1", "title": "Align the backrest slats", "description": "Into plane with the frame", "agent": "Codex",
                   "label": "To review", "tone": "warn", "summary": "Moved three slats.", "question": "",
                   "changes": [{"key": "u1", "label": "Backrest slat 01", "change": "Changed"}], "comments": [],
                   "can_accept": true, "can_request": true, "can_revert": true, "can_reply": false},
    }));
    let f = frame(&mut ui);
    for text in ["2 need attention", "Needs attention", "In progress", "Settled · 1", "To review", "Question", "Working",
                 "Codex · 3 changes", "Backrest slat 01", "Accept changes", "Request changes…", "Revert task", "Show in Project ›", "+ New task"] {
        assert!(f.contains_key(text), "{text} missing: {:?}", f.keys());
    }
    assert!(!f.contains_key("Name the groups"), "settled tasks fold away");
    ui.take_actions();
    tree_click(&mut ui, f["Accept changes"], egui::PointerButton::Primary);
    assert_eq!(ui.take_actions(), ["task:accept:t1"]);
    tree_click(&mut ui, f["Check clearance"], egui::PointerButton::Primary);
    assert_eq!(ui.take_actions(), ["task:select:t2"]);
    tree_click(&mut ui, f["Backrest slat 01"], egui::PointerButton::Primary);
    assert_eq!(ui.take_actions(), ["task:element:t1\u{1f}u1"]);
    // Request changes… opens a reply on the same task.
    tree_click(&mut ui, f["Request changes…"], egui::PointerButton::Primary);
    assert!(frame(&mut ui).contains_key("Cancel"));
    // A question takes a reply instead of review buttons.
    let mut data = ui.panels.data["tasks"].clone();
    data["detail"] = serde_json::json!({"guid": "t2", "title": "Check clearance", "agent": "Claude Code", "label": "Question",
        "tone": "warn", "question": "From the backrest or the rear posts?", "changes": [], "comments": [], "can_reply": true});
    ui.panels.set("tasks", data);
    let q = frame(&mut ui);
    assert!(q.contains_key("From the backrest or the rear posts?") && q.contains_key("Send") && !q.contains_key("Accept changes"));
}

#[test]
fn the_columns_control_offers_presets_and_columns_and_the_tree_menu_is_short() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    ui.set_tree(tag_rows(&["Walls"]), 0);
    for _ in 0..3 { tree_frame(&mut ui, vec![]); }
    let frame = tree_frame(&mut ui, vec![]);
    // The header names the preset; there are no lens tabs.
    assert!(frame.contains_key("Columns") && frame.contains_key("Filing"));
    assert!(!frame.contains_key("Tags") && !frame.contains_key("Export"));
    tree_click(&mut ui, frame["Columns"], egui::PointerButton::Primary);
    for _ in 0..2 { tree_frame(&mut ui, vec![]); }
    let menu = tree_frame(&mut ui, vec![]);
    for text in ["Presets", "Drawing", "Export", "Visibility", "Lock", "Items", "Kind", "Colour", "Line type", "Hatch", "Readiness"] {
        assert!(menu.contains_key(text), "{text}");
    }
    // A column toggle keeps the menu open and reports the user's set.
    tree_click(&mut ui, menu["Readiness"], egui::PointerButton::Primary);
    let custom = ui.column.as_ref().unwrap().columns;
    assert!(custom.has(Col::Readiness) && custom.name() == "Custom");
    assert_eq!(ui.take_actions(), [format!("tree:columns\u{1f}{}", custom.mask()), "tree:readiness\u{1f}1".to_owned()]);
    assert!(ui.column.as_ref().unwrap().head_menu.is_some());
    // A preset closes it.
    let menu = tree_frame(&mut ui, vec![]);
    tree_click(&mut ui, menu["Drawing"], egui::PointerButton::Primary);
    // The checklist that was open with the Readiness column folds with it: the checks stop.
    tree_frame(&mut ui, vec![]);
    assert_eq!(ui.take_actions(), ["tree:lens\u{1f}1", "tree:readiness\u{1f}0"]);
    let col = ui.column.as_ref().unwrap();
    assert!(col.head_menu.is_none() && col.columns == lens::Columns::preset(Lens::Drawing));
    // The tree's own menu: its verbs in three groups, nothing about the file.
    let dots = tree_frame(&mut ui, vec![])["⋯"];
    tree_click(&mut ui, dots, egui::PointerButton::Primary);
    for _ in 0..2 { tree_frame(&mut ui, vec![]); }
    let more = tree_frame(&mut ui, vec![]);
    for text in ["New node", "Expand all", "Collapse all", "Reveal selection", "Show all"] {
        assert!(more.contains_key(text), "{text}");
    }
    for text in ["Plugins", "Link selection from open file…", "Move registry to project…", "Measure quantities",
                 "Rename model rows…", "Sort by", "Invert filter", "Export layer template…"] {
        assert!(!more.contains_key(text), "{text}");
    }
    // A press elsewhere in the dock closes it.
    let name = more["Name"];
    tree_frame(&mut ui, vec![egui::Event::PointerMoved(name), egui::Event::PointerButton {
        pos: name, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }]);
    assert!(ui.column.as_ref().unwrap().head_menu.is_none());
    tree_frame(&mut ui, vec![egui::Event::PointerButton {
        pos: name, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }]);
    // The file's verbs and Plugins are in the Model menu.
    ui.menu = Some(("work".into(), Rect::from_min_size(pos2(300., 8.), vec2(60., 24.))));
    for _ in 0..2 { tree_frame(&mut ui, vec![]); }
    let model = tree_frame(&mut ui, vec![]);
    for text in ["Link selection from open file…", "Move registry to project…", "Measure quantities",
                 "Export quantities CSV…", "Import hatch patterns…", "Export hatch patterns…", "Purge unused tags…"] {
        assert!(model.contains_key(text), "{text}");
    }
    tree_click(&mut ui, model["Measure quantities"], egui::PointerButton::Primary);
    assert_eq!(ui.take_actions(), ["tree:measure\u{1f}"]);
    assert!(ui.menu.is_none());
}

#[test]
fn the_filter_button_opens_the_quick_choices_and_the_sort() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    ui.set_tree(tag_rows(&["Walls"]), 0);
    for _ in 0..3 { tree_frame(&mut ui, vec![]); }
    let frame = tree_frame(&mut ui, vec![]);
    let dock = ui.ctx.memory(|m| m.area_rect(egui::Id::new("column"))).unwrap();
    // The button ends the filter field's row.
    let button = pos2(dock.max.x - DESTINATION_STRIP_W - 10. - CONTROL_H / 2., frame["Filter"].y);
    tree_click(&mut ui, button, egui::PointerButton::Primary);
    for _ in 0..2 { tree_frame(&mut ui, vec![]); }
    let popover = tree_frame(&mut ui, vec![]);
    for text in ["Show", "All rows", "Invert the filter", "Sort by", "Project order"] {
        assert!(popover.contains_key(text), "{text}: {popover:?}");
    }
    tree_click(&mut ui, popover["Invert the filter"], egui::PointerButton::Primary);
    assert_eq!(ui.take_actions(), ["tree:filter\u{1f}!"]);
    // The popover stays inside the window.
    let menu = ui.ctx.memory(|m| m.area_rect(egui::Id::new("head_menu"))).unwrap();
    assert!(menu.max.x <= 1024. - GUT && menu.min.x >= GUT && menu.max.y <= 700. - GUT);
}

#[test]
fn a_divider_separates_the_model_from_its_scenes_and_sheets() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    let row = |key: &str, name: &str, kind: i64| (vec![key.to_owned(), name.to_owned()], vec![0, 0, 0, 0, kind, 1, 0]);
    ui.set_tree(vec![row("preview-tag:walls", "Walls", 6), row("geometry:loose", "Loose geometry", 7),
                     row("sections:all", "Section planes", 6), row("saved-scenes:all", "Saved scenes", 6),
                     row("sheet-pages:all", "Sheets", 6)], 0);
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1024., 700.));
    let ctx = ui.ctx.clone();
    for _ in 0..3 { tree_frame(&mut ui, vec![]); }
    let mut texts = std::collections::HashMap::new();
    let mut lines = Vec::new();
    let frame = ctx.run(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |c| ui.right_column(c, screen, 0.));
    for shape in frame.shapes {
        match shape.shape {
            egui::Shape::Text(text) => { texts.insert(text.galley.text().to_owned(), text.visual_bounding_rect().center()); }
            // A hairline across the whole row.
            egui::Shape::LineSegment { points, .. } if points[0].y == points[1].y && points[1].x - points[0].x > 200. => lines.push(points[0].y),
            _ => {}
        }
    }
    let between = |a: &str, b: &str| lines.iter().any(|y| *y > texts[a].y && *y < texts[b].y);
    assert!(between("Section planes", "Saved scenes"));
    assert!(!between("Walls", "Loose geometry") && !between("Loose geometry", "Section planes") && !between("Saved scenes", "Sheets"));
}

#[test]
fn the_proposal_shortcut_accepts_empty_named_tags_folders_and_links() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    ui.set_tree(tag_rows(&["Untagged"]), 0);
    ui.set_proposal_available(true);
    for (tags, folders, links, offers_proposal) in [
        (1, 0, 0, false), (2, 0, 0, true), (1, 1, 0, true), (1, 0, 1, true),
    ] {
        let overview = &mut ui.column.as_mut().unwrap().overview;
        (overview.tags, overview.folders, overview.links) = (tags, folders, links);
        for _ in 0..3 { tree_frame(&mut ui, vec![]); }
        let frame = tree_frame(&mut ui, vec![]);
        assert_eq!(frame.contains_key("Propose structure from tags…"), offers_proposal);
        assert_eq!(frame.contains_key("New node"), !offers_proposal);
    }
}

#[test]
fn a_size_typed_in_the_fillet_form_reaches_the_tool_on_enter() {
    // Typed 5 in the Size field after a refused 100 mm, then Enter: the
    // field sends the size (the tool replans) and stays usable; a second
    // Enter, with no field in focus, applies. Apply with a size still in
    // the field sends that size first.
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    ui.set_active("fillet".into());
    ui.panels.data["bevel"] = serde_json::json!({
        "affect": "edges", "measure": "radius", "amount": "100.0 mm", "segments": 8,
        "shape": null, "numeric_profile": false, "profile": 0.5, "outline": [],
        "corners": "round", "points": [],
    });
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280., 800.));
    ui.screen = screen;
    let frame = |ui: &mut Ui| {
        let ctx = ui.ctx.clone();
        let raw = egui::RawInput { screen_rect: Some(screen), events: std::mem::take(&mut ui.events), ..Default::default() };
        let _ = ctx.run(raw, |c| ui.draw_drawer(c, screen));
    };
    let field = crate::inspector::bevel_size_field();
    let typed = |ui: &mut Ui, text: &str| {
        ui.ctx.memory_mut(|m| m.request_focus(field));
        frame(ui);
        ui.events.push(egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND });
        ui.events.push(egui::Event::Text(text.into()));
        frame(ui);
        assert!(ui.ctx.wants_keyboard_input());
    };
    for _ in 0..2 { frame(&mut ui); }
    let _ = ui.take_actions();
    for size in ["5", "2"] {
        typed(&mut ui, size);
        assert!(ui.claims_key("Enter", 0));
        assert!(ui.key("Enter", true, 0) && ui.key("Enter", false, 0));
        frame(&mut ui);
        frame(&mut ui);
        assert_eq!(ui.take_actions(), [format!(r#"panel:["bevel.amount","{size}"]"#)]);
        assert!(!ui.ctx.memory(|m| m.has_focus(field)));
    }
    assert!(ui.key("Enter", true, 0));
    assert_eq!(ui.take_actions(), [r#"panel:["bevel.finish",null]"#]);
}
