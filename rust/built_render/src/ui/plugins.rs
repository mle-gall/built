//! Plugin panel protocol v1. Widget memory is scoped to a document instance.
use super::*;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Deserialize)]
pub(super) struct Panel {
    plugin: String,
    panel: String,
    instance: String,
    document: String,
    revision: u64,
    title: String,
    slot: Slot,
    sections: Vec<Section>,
}
#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Slot {
    Dock,
    Destination,
    Utility,
}
#[derive(Deserialize)]
struct Section {
    id: String,
    title: String,
    items: Vec<Item>,
}
#[derive(Deserialize)]
struct Item {
    id: String,
    #[serde(default)]
    label: String,
    #[serde(default = "enabled")]
    enabled: bool,
    #[serde(default)]
    primary: bool,
    #[serde(flatten)]
    widget: Widget,
}
fn enabled() -> bool {
    true
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Widget {
    Field {
        #[serde(flatten)]
        field: Field,
    },
    Button,
    Log {
        lines: Vec<String>,
    },
    Console {
        history: Vec<String>,
    },
    List {
        rows: Vec<Row>,
        columns: Vec<String>,
        selection: Option<String>,
    },
    Table {
        rows: Vec<Row>,
        columns: Vec<String>,
        selection: Option<String>,
    },
    Progress {
        fraction: Option<f32>,
    },
    Image {
        path: String,
    },
}
#[derive(Deserialize)]
struct Row {
    key: String,
    cells: Vec<String>,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Field {
    Text {
        value: String,
    },
    Path {
        value: String,
    },
    Toggle {
        value: bool,
    },
    Choice {
        value: String,
        options: Vec<String>,
    },
    Colour {
        value: [u8; 3],
    },
    Number {
        value: f64,
        min: f64,
        max: f64,
        #[serde(default)]
        unit: String,
    },
}
impl Panel {
    fn parse(raw: &str) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct Version {
            api: u32,
        }
        let Version { api } = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        if api != 1 {
            return Err(format!("Unsupported plugin panel API {api}"));
        }
        let p: Self = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        if [&p.plugin, &p.panel, &p.instance, &p.document, &p.title]
            .iter()
            .any(|key| key.trim().is_empty())
        {
            return Err("Panel identity and title are required".into());
        }
        let mut ids = HashSet::new();
        let mut sections = HashSet::new();
        for s in &p.sections {
            if s.id.is_empty() || !sections.insert(&s.id) {
                return Err("Duplicate or empty section ID".into());
            }
            for i in &s.items {
                if i.id.is_empty() || !ids.insert(&i.id) {
                    return Err("Duplicate or empty control ID".into());
                }
                let ok = match &i.widget {
                    Widget::Field {
                        field:
                            Field::Number {
                                value, min, max, ..
                            },
                    } => min <= value && value <= max,
                    Widget::Field {
                        field: Field::Choice { value, options },
                    } => options.contains(value),
                    Widget::Progress { fraction: Some(f) } => (0.0..=1.0).contains(f),
                    Widget::List { rows, columns, .. } | Widget::Table { rows, columns, .. } => {
                        let mut keys = HashSet::new();
                        rows.iter()
                            .all(|r| keys.insert(&r.key) && r.cells.len() == columns.len())
                    }
                    _ => true,
                };
                if !ok {
                    return Err(format!("Invalid value for {}", i.id));
                }
            }
        }
        Ok(p)
    }
}
#[derive(Default)]
pub(super) struct Panels {
    pub(super) commands: Vec<crate::extension_menu::Row>,
    models: Vec<Panel>,
    dock: Option<String>,
    utility: Option<String>,
    focus_prompt: Option<String>,
    pub(super) showing: bool,
    drafts: HashMap<(String, String), String>,
    images: HashMap<String, Result<egui::TextureHandle, String>>,
    sequence: u64,
}
impl Panels {
    pub(super) fn set(&mut self, raw: &str) -> Result<(), String> {
        let panel = Panel::parse(raw)?;
        if let Some(old) = self
            .models
            .iter_mut()
            .find(|p| p.instance == panel.instance)
        {
            *old = panel
        } else {
            self.models.push(panel)
        }
        self.images.retain(|path, _| {
            self.models.iter().any(|p| {
                p.sections
                    .iter()
                    .flat_map(|section| &section.items)
                    .any(|item| matches!(&item.widget, Widget::Image {path: live} if live == path))
            })
        });
        Ok(())
    }
    pub(super) fn open(&mut self, instance: &str) {
        if let Some(p) = self.models.iter().find(|p| p.instance == instance) {
            match p.slot {
                Slot::Destination => {}
                Slot::Dock => {
                    self.showing = true;
                    self.dock = Some(instance.into())
                }
                Slot::Utility => {
                    self.utility = Some(instance.into());
                    self.focus_prompt = Some(instance.into());
                }
            }
        }
    }
    pub(super) fn destinations(&self) -> impl Iterator<Item=(&str, &str)> {
        self.models.iter().filter(|p| p.slot == Slot::Destination).map(|p| (p.instance.as_str(), p.title.as_str()))
    }
    pub(super) fn dock_visible(&self, id: &str) -> bool { self.showing && self.dock.as_deref() == Some(id) }
    pub(super) fn utility_visible(&self, id: &str) -> bool { self.utility.as_deref() == Some(id) }
    pub(super) fn available(&self) -> bool {
        !self.models.is_empty() || !self.commands.is_empty()
    }
    pub(super) fn utility_open(&self) -> bool {
        self.utility.is_some()
    }
    pub(super) fn dock(&mut self, ui: &mut egui::Ui, actions: &mut Vec<String>) {
        let mut last_path: Option<&[String]> = None;
        for (id, path, label, key, mark, enabled) in &self.commands {
            if last_path != Some(path) && !path.is_empty() {
                super::kit::caption_plain(ui, &path.join(" ▸ "));
            }
            last_path = Some(path);
            if label.is_empty() {
                ui.separator();
                continue;
            }
            let row = (
                id.clone(),
                label.clone(),
                key.clone(),
                String::new(),
                *mark,
                *enabled,
            );
            if super::kit::menu_row(ui, &row) {
                actions.push(id.clone());
            }
        }
        let mut open = None;
        ui.horizontal_wrapped(|ui| {
            for p in self.models.iter().filter(|p| p.slot != Slot::Destination) {
                if ui
                    .selectable_label(self.dock.as_ref() == Some(&p.instance), &p.title)
                    .clicked()
                {
                    open = Some(p.instance.clone())
                }
            }
        });
        if let Some(id) = open {
            self.open(&id)
        }
        if self.dock.is_none() {
            self.dock = self
                .models
                .iter()
                .find(|p| p.slot == Slot::Dock)
                .map(|p| p.instance.clone())
        }
        if let Some(id) = self.dock.clone() {
            self.draw(ui, &id, actions)
        }
    }
    /// The utility pane (Python console, Model Info…) in `bay`: the free
    /// band between the rail and the dock, its bottom on the bottom bar's
    /// gutter. The caller leaves the 8-pt gutters; the pane takes the dock's
    /// glass, radius and margin (P3: it used to sit over the rail).
    pub(super) fn utility(&mut self, ctx: &egui::Context, bay: Rect, actions: &mut Vec<String>) {
        let Some(id) = self.utility.clone() else {
            return;
        };
        egui::Area::new("plugin_utility".into())
            .constrain(false)
            .pivot(egui::Align2::LEFT_BOTTOM)
            .fixed_pos(bay.left_bottom())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Panel)
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        let width = (bay.width() - 20.).max(120.);
                        ui.set_width(width);
                        egui::Resize::default()
                            .id_salt("utility_height")
                            .resizable([false, true])
                            .default_width(width)
                            .min_width(width)
                            .max_width(width)
                            .default_height(165.)
                            .max_height((bay.height() * 0.65).max(120.))
                            .show(ui, |ui| self.draw(ui, &id, actions));
                    });
            });
    }
    pub(super) fn draw(&mut self, ui: &mut egui::Ui, id: &str, actions: &mut Vec<String>) {
        let Some(p) = self.models.iter().find(|p| p.instance == id) else {
            return;
        };
        ui.spacing_mut().item_spacing = vec2(SPACE_S, 1.);
        let console = p
            .sections
            .iter()
            .flat_map(|s| &s.items)
            .any(|i| matches!(i.widget, Widget::Console { .. }));
        let transcript_height =
            console.then(|| (ui.available_height() - 2. * BUTTON_H - SPACE_XS).max(44.));
        let mut buttons = p
            .sections
            .iter()
            .flat_map(|s| &s.items)
            .filter(|i| matches!(i.widget, Widget::Button));
        let primary_id = buttons
            .clone()
            .find(|i| i.primary)
            .or_else(|| buttons.next())
            .map(|i| &i.id);
        let mut control = |ui: &mut egui::Ui, item: &Item, primary: bool| {
            let response = ui
                .push_id((id, &item.id), |ui| {
                    ui.add_enabled_ui(item.enabled, |ui| {
                        let focus = matches!(item.widget, Widget::Console { .. })
                            && self.focus_prompt.as_deref() == Some(id)
                            && ui.is_visible()
                            && !ui.is_sizing_pass();
                        if focus {
                            self.focus_prompt = None;
                        }
                        draw_item(
                            ui,
                            item,
                            &mut self.drafts,
                            (id.into(), item.id.clone()),
                            &mut self.images,
                            focus,
                            primary,
                            transcript_height,
                        )
                    })
                    .inner
                })
                .inner;
            if let Some((event, value)) = response {
                self.sequence += 1;
                let msg = json!({
                    "api":1,"plugin":p.plugin,"panel":p.panel,"instance":id,"document":p.document,"revision":p.revision,
                    "event_id":self.sequence,"control":item.id,"event":event,"value":value
                });
                actions.push(format!("plugin:{msg}"));
            }
        };
        let mut close = false;
        if p.slot == Slot::Utility {
            ui.horizontal(|ui| {
                kit::caption_plain(ui, &p.title);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if kit::quiet(ui, "Close").clicked() {
                        close = true;
                    }
                    if console {
                        for item in p
                            .sections
                            .iter()
                            .flat_map(|s| &s.items)
                            .rev()
                            .filter(|i| matches!(i.widget, Widget::Button))
                        {
                            control(ui, item, false);
                        }
                    }
                });
            });
        }
        egui::ScrollArea::vertical().id_salt(id).show(ui, |ui| {
            for section in &p.sections {
                if !section.title.is_empty() && section.title != p.title {
                    kit::caption_plain(ui, &section.title);
                }
                for group in section.items.chunk_by(|a, b| {
                    matches!(a.widget, Widget::Button) && matches!(b.widget, Widget::Button)
                }) {
                    if console
                        && p.slot == Slot::Utility
                        && matches!(group[0].widget, Widget::Button)
                    {
                        continue;
                    }
                    if group.len() > 1 {
                        ui.horizontal_wrapped(|ui| {
                            for item in group {
                                control(ui, item, !console && primary_id == Some(&item.id));
                            }
                        });
                    } else {
                        control(ui, &group[0], !console && primary_id == Some(&group[0].id));
                    }
                }
            }
        });
        if close {
            self.utility = None;
            self.sequence += 1;
            let event = json!({"api":1,"plugin":p.plugin,"panel":p.panel,"instance":id,
                "document":p.document,"revision":p.revision,"event_id":self.sequence,
                "control":"_panel","event":"closed","value":null});
            actions.push(format!("plugin:{event}"));
        }
    }
}
type Event = Option<(&'static str, Value)>;
fn draw_item(
    ui: &mut egui::Ui,
    i: &Item,
    drafts: &mut HashMap<(String, String), String>,
    key: (String, String),
    images: &mut HashMap<String, Result<egui::TextureHandle, String>>,
    focus: bool,
    primary: bool,
    transcript_height: Option<f32>,
) -> Event {
    let mut event = None;
    match &i.widget {
        Widget::Button => {
            if kit::sheet_button(ui, &i.label, "", u8::from(primary)) {
                event = Some(("invoke", Value::Null))
            }
        }
        Widget::Field { field } => {
            kit::property_control(ui, &i.label, |ui| match field {
                Field::Toggle { value } => {
                    if kit::switch_row(ui, "", *value, ui.is_enabled()).clicked() {
                        event = Some(("commit", json!(!value)))
                    }
                }
                Field::Number {
                    value,
                    min,
                    max,
                    unit,
                } => {
                    let mut v = *value;
                    if ui
                        .add(super::kit::number(&mut v, unit).range(*min..=*max))
                        .changed()
                    {
                        event = Some(("commit", json!(v)))
                    }
                }
                Field::Choice { value, options } => {
                    let labels: Vec<_> = options.iter().map(String::as_str).collect();
                    let mut index = options.iter().position(|v| v == value).unwrap_or(0);
                    kit::choice(ui, "choice", &mut index, &labels);
                    if options.get(index) != Some(value) {
                        event = Some(("commit", json!(options[index])));
                    }
                }
                Field::Colour { value } => {
                    let mut rgb = *value;
                    if kit::colour_well(ui, &mut rgb).changed() {
                        event = Some(("commit", json!(rgb)))
                    }
                }
                Field::Text { value } | Field::Path { value } => {
                    let path = matches!(field, Field::Path { .. });
                    let draft = drafts.entry(key.clone()).or_insert_with(|| value.clone());
                    let r = if path {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if kit::quiet(ui, "Choose…").clicked() {
                                event = Some(("browse", Value::Null));
                            }
                            kit::path_field(ui, draft)
                        })
                        .inner
                    } else {
                        ui.add(kit::field(draft).desired_width(ui.available_width()))
                    };
                    if r.lost_focus() && draft != value {
                        event = Some(("commit", json!(draft)))
                    }
                    if !r.has_focus() {
                        drafts.remove(&key);
                    }
                }
            });
        }
        Widget::Console { history } => {
            let draft = drafts.entry(key).or_default();
            let input_id = ui.make_persistent_id("prompt");

            let focused = ui.memory(|m| m.has_focus(input_id));
            let run = focused
                && ui.input_mut(|i| {
                    i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)
                        || i.consume_key(egui::Modifiers::CTRL, egui::Key::Enter)
                });
            let idx_id = input_id.with("history");
            if focused {
                let cursor = egui::TextEdit::load_state(ui.ctx(), input_id)
                    .and_then(|s| s.cursor.char_range())
                    .map(|r| r.primary.index)
                    .unwrap_or(0);
                let offset = draft
                    .char_indices()
                    .nth(cursor)
                    .map_or(draft.len(), |(i, _)| i);
                let mut idx = ui
                    .data(|d| d.get_temp::<usize>(idx_id))
                    .unwrap_or(history.len())
                    .min(history.len());
                for (key, delta, boundary) in [
                    (egui::Key::ArrowUp, -1, !draft[..offset].contains('\n')),
                    (egui::Key::ArrowDown, 1, !draft[offset..].contains('\n')),
                ] {
                    if boundary && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key)) {
                        if idx == history.len() {
                            ui.data_mut(|d| d.insert_temp(idx_id.with("draft"), draft.clone()));
                        }
                        idx = idx.saturating_add_signed(delta).min(history.len());
                        *draft = history
                            .get(idx)
                            .cloned()
                            .or_else(|| ui.data(|d| d.get_temp(idx_id.with("draft"))))
                            .unwrap_or_default();
                        ui.data_mut(|d| d.insert_temp(idx_id, idx));
                    }
                }
            }
            let (response, clicked) = ui
                .horizontal(|ui| {
                    let width = (ui.available_width() - 66.).max(40.);
                    let response = ui.add(
                        egui::TextEdit::multiline(draft)
                            .id(input_id)
                            .desired_rows(1)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(width)
                            .hint_text(">>>"),
                    );
                    (response, kit::sheet_button(ui, "Run", "", 1))
                })
                .inner;
            if focus {
                response.request_focus();
            }
            if (clicked || run) && !draft.trim().is_empty() {
                event = Some(("console_submit", json!(std::mem::take(draft).trim_end())));
                ui.data_mut(|d| d.remove::<usize>(idx_id));
                response.request_focus();
            }
        }
        Widget::Log { lines } => {
            egui::ScrollArea::vertical()
                .id_salt("log")
                .max_height(transcript_height.unwrap_or(100.))
                .auto_shrink([false, transcript_height.is_none()])
                .stick_to_bottom(true)
                .show_rows(ui, 16., lines.len(), |ui, range| {
                    for n in range {
                        ui.monospace(&lines[n]);
                    }
                });
        }
        Widget::Progress { fraction } => {
            kit::property_control(ui, &i.label, |ui| {
                if let Some(value) = fraction {
                    kit::progress_bar(ui, *value, 4.);
                } else {
                    kit::ring(ui);
                }
            });
        }
        Widget::Image { path } => {
            match images
                .entry(path.clone())
                .or_insert_with(|| load_png(ui.ctx(), path))
            {
                Ok(tex) => {
                    let width = ui.available_width();
                    let height = (width * tex.size_vec2().y / tex.size_vec2().x).clamp(60., 300.);
                    super::kit::render_preview(ui, Some(tex), vec2(width, height));
                }
                Err(e) => {
                    ui.label(e.as_str());
                }
            }
        }
        Widget::List {
            columns,
            rows,
            selection,
        }
        | Widget::Table {
            columns,
            rows,
            selection,
        } => {
            kit::table_row(ui, columns, false, true);
            for row in rows {
                if kit::table_row(ui, &row.cells, selection.as_ref() == Some(&row.key), false)
                    .clicked()
                {
                    event = Some(("selection", json!(row.key)));
                }
            }
        }
    }
    event
}
fn load_png(ctx: &egui::Context, path: &str) -> Result<egui::TextureHandle, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let size = reader
        .output_buffer_size()
        .filter(|n| *n <= 128 * 1024 * 1024)
        .ok_or("Preview too large")?;
    let mut data = vec![0; size];
    let info = reader.next_frame(&mut data).map_err(|e| e.to_string())?;
    let dims = [info.width as usize, info.height as usize];
    let image = match info.color_type {
        png::ColorType::Rgb => egui::ColorImage::from_rgb(dims, &data[..info.buffer_size()]),
        png::ColorType::Rgba => {
            egui::ColorImage::from_rgba_unmultiplied(dims, &data[..info.buffer_size()])
        }
        _ => return Err("Preview needs an RGB or RGBA PNG".into()),
    };
    Ok(ctx.load_texture(path, image, Default::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(item: Value) -> Value {
        json!({"api":1,"plugin":"demo","panel":"options","instance":"doc1-demo","document":"doc1",
            "revision":7,"slot":"dock","title":"Demo",
            "sections":[{"id":"options","title":"","items":[item]}]})
    }

    #[test]
    fn the_utility_pane_fills_its_bay_and_never_reaches_the_rail() {
        let mut model = descriptor(
            json!({"kind":"field","id":"size","type":"number","value":1,"min":0,"max":10,"unit":"m"}),
        );
        model["slot"] = json!("utility");
        let mut panels = Panels::default();
        panels.set(&model.to_string()).unwrap();
        panels.open("doc1-demo");
        for (w, h) in [(1600.0, 1000.0), (1280.0, 800.0)] {
            // Rail 72 wide at x 8; dock 300 wide with an 8 gutter.
            let bay = Rect::from_min_max(
                pos2(8. + 72. + 8., 56.),
                pos2(w - 8. - 300. - 8., h - 36. - 8.),
            );
            let ctx = super::super::test_context();
            for _ in 0..3 {
                let raw = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(w, h))),
                    ..Default::default()
                };
                let _ = ctx.run(raw, |ctx| panels.utility(ctx, bay, &mut Vec::new()));
            }
            let r = ctx
                .memory(|m| m.area_rect(egui::Id::new("plugin_utility")))
                .unwrap();
            assert!(
                (r.min.x - bay.min.x).abs() < 0.5,
                "{w}: starts at {}",
                r.min.x
            );
            assert!(
                (r.max.x - bay.max.x).abs() < 1.0,
                "{w}: ends at {}",
                r.max.x
            );
            assert!((r.max.y - bay.max.y).abs() < 0.5, "{w}: bottom {}", r.max.y);
        }
    }

    #[test]
    fn schema_and_version() {
        let model = descriptor(json!({"kind":"field","id":"size","type":"number",
            "value":9,"min":0,"max":10,"unit":"m"}));
        let mut panels = Panels::default();
        assert!(panels.set(&model.to_string()).is_ok());
        let mut bad = model.clone();
        bad["api"] = json!(2);
        assert!(panels
            .set(&bad.to_string())
            .unwrap_err()
            .contains("Unsupported"));
        bad = model.clone();
        bad["sections"][0]["items"][0]["value"] = json!(11);
        assert!(panels.set(&bad.to_string()).is_err());
        let mut duplicate = model;
        let copy = duplicate["sections"][0]["items"][0].clone();
        duplicate["sections"][0]["items"]
            .as_array_mut()
            .unwrap()
            .push(copy);
        assert!(panels.set(&duplicate.to_string()).is_err());
        assert_eq!(panels.models.len(), 1);
    }

    #[test]
    fn console_actions_share_the_header_and_run_aligns_with_the_prompt() {
        let ctx = super::super::test_context();
        let mut panel = descriptor(json!({"kind":"log","id":"log","lines":["output"]}));
        panel["slot"] = json!("utility");
        let items = panel["sections"][0]["items"].as_array_mut().unwrap();
        items.push(json!({"kind":"console","id":"prompt","history":[]}));
        items.push(json!({"kind":"button","id":"help","label":"Help"}));
        let mut panels = Panels::default();
        panels.set(&panel.to_string()).unwrap();
        let mut positions = HashMap::new();
        for _ in 0..3 {
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(640., 200.))), ..Default::default() };
            let frame = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| panels.draw(ui, "doc1-demo", &mut Vec::new()));
            });
            for clipped in frame.shapes {
                if let egui::Shape::Text(text) = clipped.shape {
                    positions.insert(text.galley.text().to_owned(), text.pos + text.galley.size() / 2.);
                }
            }
        }
        assert!((positions["Help"].y - positions["Close"].y).abs() < 4.);
        assert!((positions[">>>"].y - positions["Run"].y).abs() < BUTTON_H / 2., "{positions:?}");
        assert!(positions["Run"].x > positions[">>>"].x);
        assert!(positions["Run"].y - positions["output"].y > 80.);
    }

    #[test]
    fn table_hit_area_spans_the_last_column_and_returns_the_row_key() {
        let ctx = super::super::test_context();
        let mut rows = Panels::default();
        rows.set(
            &descriptor(
                json!({"kind":"table", "id":"objects","columns":["Type","Count"],
            "rows":[{"key":"faces","cells":["Faces","5"]}]}),
            )
            .to_string(),
        )
        .unwrap();
        let mut events = Vec::new();
        for press in [None, Some(true), Some(false)] {
            let mut raw = egui::RawInput::default();
            raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(300., 150.)));
            let point = pos2(270., 45.);
            if let Some(pressed) = press {
                raw.events = vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ];
            }
            let _ = ctx.run(raw, |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| {
                        rows.draw(ui, "doc1-demo", &mut events);
                    })
                    .inner
            });
        }
        assert_eq!(events.len(), 1);
        let event: Value =
            serde_json::from_str(events[0].strip_prefix("plugin:").unwrap()).unwrap();
        assert_eq!(event["event"], "selection");
        assert_eq!(event["value"], "faces");
    }

    #[test]
    fn destination_registration_uses_document_instance_and_updates_in_place() {
        let mut panel = descriptor(json!({"kind":"log","id":"events","lines":[]}));
        panel["slot"] = json!("destination");
        let mut panels = Panels::default();
        panels.set(&panel.to_string()).unwrap();
        panel["title"] = json!("Coordination");
        panels.set(&panel.to_string()).unwrap();
        assert_eq!(panels.destinations().collect::<Vec<_>>(), [("doc1-demo", "Coordination")]);
    }

    #[test]
    fn field_types() {
        for (ty, value, wrong) in [
            ("text", json!("name"), json!(2)),
            ("path", json!("/model.png"), json!(null)),
            ("number", json!(3.5), json!("3.5")),
            ("toggle", json!(true), json!(1)),
            ("choice", json!("a"), json!("unknown")),
            ("colour", json!([0, 12, 255]), json!([1, 2, 300])),
        ] {
            let mut model = descriptor(json!({"kind":"field","id":"input","type":ty,"value":value,
                "min":0,"max":10,"options":["a","b"]}));
            assert!(Panel::parse(&model.to_string()).is_ok(), "{ty}");
            model["sections"][0]["items"][0]["value"] = wrong;
            assert!(Panel::parse(&model.to_string()).is_err(), "{ty}");
        }
    }
}
