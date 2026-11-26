use super::{label, send};
use serde_json::{json, Value};

#[derive(Default)]
pub struct Rule {
    original: Value,
    draft: String,
}
impl Rule {
    pub fn set(&mut self, rule: &Value) {
        if self.original != *rule {
            self.draft = serde_json::to_string_pretty(&rule["recipe"]).unwrap_or_default();
            self.original = rule.clone();
        }
    }

    pub fn draw(&mut self, ui: &mut egui::Ui, preview: &Value, out: &mut Vec<String>) {
        if self.original.is_null() {
            return;
        }
        crate::ui::kit::disclosure(
            ui,
            ("filing defaults", label(&self.original["node"])),
            &format!("Edit filing rule · {}", label(&self.original["name"])),
            self.original["focused"] == true,
            |ui| {
                ui.label("Local overrides");
                ui.add(
                    egui::TextEdit::multiline(&mut self.draft)
                        .code_editor()
                        .desired_width(ui.available_width())
                        .desired_rows(5),
                );
                match serde_json::from_str::<Value>(&self.draft) {
                    Ok(recipe) if recipe.is_object() => {
                        let req = json!([self.original["node"], self.original["recipe"], recipe]);
                        if ui.button("Preview changes").clicked() {
                            send(out, "rule.preview", req.clone());
                        }
                        if preview["request"] == req {
                            if preview["error"].is_string() {
                                ui.label(label(&preview["error"]));
                            } else {
                                ui.label(format!(
                                    "{} member records will change",
                                    preview["changed"]
                                ));
                                if ui.button("Apply filing rule").clicked() {
                                    send(out, "rule.apply", req);
                                }
                            }
                        }
                    }
                    _ => {
                        ui.label("Enter a JSON object to preview the filing rule.");
                    }
                }
            },
        );
    }
}
