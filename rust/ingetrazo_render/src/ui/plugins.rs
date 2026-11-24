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
            if label.is_empty() { ui.separator(); continue; }
            let row = (id.clone(), label.clone(), key.clone(), String::new(), *mark, *enabled);
            if super::kit::menu_row(ui, &row) { actions.push(id.clone()); }
        }
        let mut open = None;
        ui.horizontal_wrapped(|ui| {
            for p in &self.models {
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
    pub(super) fn utility(
        &mut self,
        ctx: &egui::Context,
        screen: Rect,
        dock: f32,
        actions: &mut Vec<String>,
    ) {
        let Some(id) = self.utility.clone() else {
            return;
        };
        egui::Area::new("plugin_utility".into())
            .anchor(egui::Align2::LEFT_BOTTOM, vec2(58., -44.))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Panel)
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width((screen.width() - dock - 96.).max(120.));
                        let width = ui.available_width();
                        egui::Resize::default()
                            .id_salt("utility_height")
                            .resizable([false, true])
                            .default_width(width)
                            .max_width(width)
                            .default_height(165.)
                            .max_height(screen.height() * 0.65)
                            .show(ui, |ui| self.draw(ui, &id, actions));
                    });
            });
    }
    fn draw(&mut self, ui: &mut egui::Ui, id: &str, actions: &mut Vec<String>) {
        let Some(p) = self.models.iter().find(|p| p.instance == id) else {
            return;
        };
        ui.spacing_mut().item_spacing = vec2(SPACE_S, 5.);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&p.title).font(heading()));
            if p.slot == Slot::Utility {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Close").clicked() {
                        self.utility = None;
                        self.sequence += 1;
                        let event = json!({"api":1,"plugin":p.plugin,"panel":p.panel,"instance":id,
                            "document":p.document,"revision":p.revision,"event_id":self.sequence,
                            "control":"_panel","event":"closed","value":null});
                        actions.push(format!("plugin:{event}"));
                    }
                });
            }
        });
        egui::ScrollArea::vertical().id_salt(id).show(ui, |ui| {
            for section in &p.sections {
                if !section.title.is_empty() {
                    ui.add_space(SPACE_XS);
                    ui.separator();
                    ui.strong(&section.title);
                }
                let mut control = |ui: &mut egui::Ui, item: &Item| {
                    let response = ui.push_id((id, &item.id), |ui| {
                        ui.add_enabled_ui(item.enabled, |ui| {
                            let focus = matches!(item.widget, Widget::Console { .. })
                                && self.focus_prompt.as_deref() == Some(id)
                                && ui.is_visible() && !ui.is_sizing_pass();
                            if focus { self.focus_prompt = None; }
                            draw_item(ui, item, &mut self.drafts, (id.into(), item.id.clone()), &mut self.images, focus)
                        }).inner
                    }).inner;
                    if let Some((event,value)) = response {
                        self.sequence += 1;
                        let msg = json!({
                            "api":1,"plugin":p.plugin,"panel":p.panel,"instance":id,"document":p.document,"revision":p.revision,
                            "event_id":self.sequence,"control":item.id,"event":event,"value":value
                        });
                        actions.push(format!("plugin:{msg}"));
                    }
                };
                for group in section.items.chunk_by(|a,b| matches!(a.widget,Widget::Button) && matches!(b.widget,Widget::Button)) {
                    if group.len()>1 {
                        ui.horizontal_wrapped(|ui| for item in group {control(ui,item);});
                    } else {
                        control(ui,&group[0]);
                    }
                }
            }
        });
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
) -> Event {
    let mut event = None;
    match &i.widget {
        Widget::Button => {
            if ui
                .add(egui::Button::new(&i.label).wrap_mode(egui::TextWrapMode::Extend))
                .clicked()
            {
                event = Some(("invoke", Value::Null))
            }
        }
        Widget::Field { field } => {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [82., 20.],
                    egui::Label::new(&i.label).halign(egui::Align::LEFT),
                );
                match field {
                    Field::Toggle { value } => {
                        let mut v = *value;
                        if ui.checkbox(&mut v, "").changed() {
                            event = Some(("commit", json!(v)))
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
                        egui::ComboBox::from_id_salt("choice")
                            .selected_text(value)
                            .show_ui(ui, |ui| {
                                for v in options {
                                    if ui.selectable_label(v == value, v).clicked() {
                                        event = Some(("commit", json!(v)))
                                    }
                                }
                            });
                    }
                    Field::Colour { value } => {
                        let mut rgb = *value;
                        if ui.color_edit_button_srgb(&mut rgb).changed() {
                            event = Some(("commit", json!(rgb)))
                        }
                    }
                    Field::Text { value } | Field::Path { value } => {
                        if matches!(field, Field::Path { .. }) && ui.small_button("…").clicked() {
                            event = Some(("browse", Value::Null))
                        }
                        let draft = drafts.entry(key.clone()).or_insert_with(|| value.clone());
                        let r = ui.add(
                            super::kit::field(draft).desired_width(ui.available_width()),
                        );
                        if r.lost_focus() && draft != value {
                            event = Some(("commit", json!(draft)))
                        }
                        if !r.has_focus() {
                            drafts.remove(&key);
                        }
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
            let response = ui.add(
                egui::TextEdit::multiline(draft)
                    .id(input_id)
                    .desired_rows(1)
                    .desired_width(f32::INFINITY)
                    .hint_text(">>>"),
            );
            if focus {
                response.request_focus();
            }
            if (ui.button("Run").clicked() || run) && !draft.trim().is_empty() {
                event = Some(("console_submit", json!(std::mem::take(draft).trim_end())));
                ui.data_mut(|d| d.remove::<usize>(idx_id));
                response.request_focus();
            }
        }
        Widget::Log { lines } => {
            egui::ScrollArea::vertical()
                .id_salt("log")
                .max_height(100.)
                .stick_to_bottom(true)
                .show_rows(ui, 16., lines.len(), |ui, range| {
                    for n in range {
                        ui.monospace(&lines[n]);
                    }
                });
        }
        Widget::Progress { fraction } => {
            ui.add(
                super::kit::progress(fraction.unwrap_or_default())
                    .animate(fraction.is_none())
                    .text(&i.label),
            );
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
            egui::Grid::new("rows").striped(true).show(ui, |ui| {
                for col in columns {
                    ui.strong(col);
                }
                ui.end_row();
                for row in rows {
                    for cell in &row.cells {
                        if ui
                            .selectable_label(selection.as_ref() == Some(&row.key), cell)
                            .clicked()
                        {
                            event = Some(("selection", json!(row.key)))
                        }
                    }
                    ui.end_row();
                }
            });
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
