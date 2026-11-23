// SPDX-License-Identifier: GPL-3.0-or-later
//! Browse saved views in their document folders without changing scene order.
use super::{kit, tokens::*, Item};

#[derive(Default)]
pub(super) struct Browser {
    owner: String,
    folder: String,
    destination: String,
    name: String,
    query: String,
}

impl Browser {
    fn open(&mut self, key: &str) {
        self.folder = key.into();
        self.destination = key.into();
        self.name = key.into();
        self.query.clear();
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        scenes: &[Item],
        folders: &[Item],
        clicked: &mut Option<String>,
    ) {
        let Some(root) = folders.first() else {
            return;
        };
        if self.owner != root.3 {
            *self = Self::default();
            self.owner = root.3.clone();
        }
        if !folders.iter().any(|row| row.0 == self.folder) {
            self.open("");
        }
        let current = scenes.iter().find(|v| v.4 == 1);
        if let Some(row) = folders
            .iter()
            .find(|f| f.0 == self.folder && !f.0.is_empty())
        {
            let label = if row.2.is_empty() {
                "All scenes"
            } else {
                &row.2
            };
            if kit::quiet(ui, &format!("‹ {label}")).clicked() {
                self.open(&row.2);
            }
            ui.strong(&self.folder);
        }
        if scenes.len() + folders.len() > 4 || !self.query.is_empty() {
            ui.add(kit::search(&mut self.query, "Find scene or folder"));
        }
        let query = self.query.trim().to_lowercase();
        egui::ScrollArea::vertical()
            .id_salt("saved views")
            .max_height(ui.ctx().screen_rect().height() * 0.31)
            .show(ui, |ui| {
                for row in folders.iter().skip(1) {
                    if if query.is_empty() {
                        row.2 != self.folder
                    } else {
                        !row.1.to_lowercase().contains(&query)
                    } {
                        continue;
                    }
                    if kit::quiet(ui, &format!("{}  ›", row.1)).clicked() {
                        self.open(&row.0);
                    }
                }
                for row in scenes {
                    if if query.is_empty() {
                        row.3 != self.folder
                    } else {
                        !row.1.to_lowercase().contains(&query)
                    } {
                        continue;
                    }
                    let target = (
                        row.0.clone(),
                        row.1.clone(),
                        String::new(),
                        "check".into(),
                        row.4,
                        true,
                    );
                    if kit::menu_row(ui, &target) {
                        *clicked = Some(format!("tab:{}", row.0));
                    }
                }
            });
        kit::disclosure(ui, "manage scene folders", "Organise scenes", false, |ui| {
            ui.add(
                kit::field(&mut self.name)
                    .hint_text("Folder name")
                    .desired_width(ui.available_width()),
            );
            kit::property_control(ui, "Destination", |ui| {
                egui::ComboBox::from_id_salt("scene destination")
                    .selected_text(if self.destination.is_empty() {
                        "All scenes"
                    } else {
                        &self.destination
                    })
                    .show_ui(ui, |ui| {
                        for f in folders {
                            ui.selectable_value(&mut self.destination, f.0.clone(), &f.1);
                        }
                    });
            });
            let request = |verb: &str, label: &str| {
                format!(
                    "tab:scenefolders:{}",
                    serde_json::json!([verb, self.owner, self.folder, self.destination, label])
                )
            };
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_S;
                if !self.name.trim().is_empty() && kit::quiet(ui, "New folder").clicked() {
                    *clicked = Some(request("add", &self.name));
                }
                if !self.folder.is_empty() {
                    if !self.name.trim().is_empty()
                        && self.name != self.folder
                        && kit::quiet(ui, "Rename").clicked()
                    {
                        *clicked = Some(request("rename", &self.name));
                    }
                    if self.destination != self.folder && kit::quiet(ui, "Move folder").clicked() {
                        *clicked = Some(request("move", ""));
                    }
                    if kit::quiet(ui, "Remove folder")
                        .on_hover_text("Keep its scenes and child folders in the parent")
                        .clicked()
                    {
                        *clicked = Some(request("remove", ""));
                    }
                }
                if let Some(scene) = current {
                    if scene.3 != self.destination
                        && kit::quiet(ui, &format!("Move “{}” here", scene.1)).clicked()
                    {
                        *clicked = Some(request("file", &scene.1));
                    }
                }
            });
        });
    }
}
