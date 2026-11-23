// SPDX-License-Identifier: GPL-3.0-or-later
//! Browse saved views in their document folders without changing scene order.
use super::{kit, tokens::*, Item};

const PHOTO_ATTEMPTS: usize = 4;

#[derive(Default)]
struct Photo {
    revision: String,
    texture: Option<egui::TextureHandle>,
    attempts: usize,
    next: f64,
}

#[derive(Default)]
pub(super) struct Browser {
    owner: String,
    pub thumbnails: Option<bool>,
    photos: std::collections::BTreeMap<String, Photo>,
    pending: Option<String>,
    folder: String,
    destination: String,
    name: String,
    query: String,
}

impl Browser {
    pub fn retain(&mut self, scenes: &[Item]) {
        let names: std::collections::HashSet<_> = scenes.iter().map(|s| s.1.as_str()).collect();
        self.photos.retain(|name, _| names.contains(name.as_str()));
    }

    pub fn finish(
        &mut self,
        ctx: &egui::Context,
        request: &str,
        pixels: Option<crate::scene_photo::Pixels>,
    ) {
        if self.pending.as_deref() != Some(request) {
            return;
        }
        self.pending = None;
        if let (Ok((owner, revision, name)), Some((w, h, pixels))) = (
            serde_json::from_str::<(String, String, String)>(request),
            pixels,
        ) {
            if owner == self.owner
                && (1..=192).contains(&w)
                && (1..=128).contains(&h)
                && pixels.len() == w * h * 4
            {
                if let Some(photo) = self
                    .photos
                    .get_mut(&name)
                    .filter(|p| p.revision == revision)
                {
                    let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &pixels);
                    photo.texture =
                        Some(ctx.load_texture(name, image, egui::TextureOptions::LINEAR));
                    photo.attempts = PHOTO_ATTEMPTS;
                }
            }
        }
        ctx.request_repaint();
    }

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
        background: &mut Vec<String>,
    ) {
        let Some(root) = folders.first() else {
            return;
        };
        if self.owner != root.3 {
            let thumbnails = self.thumbnails;
            *self = Self::default();
            self.thumbnails = thumbnails;
            self.owner = root.3.clone();
        }
        let thumbnails = self.thumbnails.unwrap_or(true);
        if let Some(choice) = kit::segmented(
            ui,
            &["List", "Thumbnails"],
            usize::from(thumbnails),
            ui.available_width(),
        ) {
            self.thumbnails = Some(choice == 1);
            background.push(format!("scenephotomode:{choice}"));
        }
        let now = ui.input(|i| i.time);
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
                    let activated = if thumbnails {
                        let cached = self.photos.get(&row.1).and_then(|p| p.texture.as_ref());
                        let response = kit::scene_preview_row(ui, &row.1, row.4 == 1, cached);
                        if ui.is_rect_visible(response.rect) && self.pending.is_none() {
                            let p = self.photos.entry(row.1.clone()).or_default();
                            if p.revision != root.2 {
                                p.revision = root.2.clone();
                                p.attempts = 0;
                                p.next = 0.;
                            }
                            if p.attempts < PHOTO_ATTEMPTS {
                                if now >= p.next {
                                    let request =
                                        serde_json::json!([root.3, root.2, row.1]).to_string();
                                    p.attempts += 1;
                                    p.next = now + 0.4;
                                    self.pending = Some(request.clone());
                                    background.push(format!("scenephoto:{request}"));
                                } else {
                                    ui.ctx().request_repaint_after(
                                        std::time::Duration::from_millis(400),
                                    );
                                }
                            }
                        }
                        response.clicked()
                    } else {
                        let target = (
                            row.0.clone(),
                            row.1.clone(),
                            String::new(),
                            "check".into(),
                            row.4,
                            true,
                        );
                        kit::menu_row(ui, &target)
                    };
                    if activated {
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

#[cfg(test)]
mod tests {
    use super::*;
    fn row(key: &str, name: &str, rev: &str, parent: &str) -> Item {
        (key.into(), name.into(), rev.into(), parent.into(), 0, true)
    }

    fn frame(
        ctx: &egui::Context,
        browser: &mut Browser,
        rows: &[Item],
        folders: &[Item],
        time: f64,
    ) -> Vec<String> {
        let mut jobs = vec![];
        let _ = ctx.run(
            egui::RawInput {
                time: Some(time),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    browser.show(ui, rows, folders, &mut None, &mut jobs)
                });
            },
        );
        jobs
    }

    #[test]
    fn photos_use_visible_rows_and_retry_a_miss_without_discarding_a_previous_image() {
        let ctx = crate::ui::test_context();
        let mut browser = Browser::default();
        let scenes = [
            row("scene:0", "A", "", ""),
            row("scene:1", "B", "", "hidden"),
        ];
        let mut folders = [
            row("", "All scenes", "1", "doc"),
            row("hidden", "Folder", "", "doc"),
        ];
        let first = frame(&ctx, &mut browser, &scenes, &folders, 0.);
        assert_eq!(first.len(), 1);
        let req = first[0].strip_prefix("scenephoto:").unwrap();
        assert!(req.contains("\"A\""));
        assert!(frame(&ctx, &mut browser, &scenes, &folders, 1.).is_empty());
        browser.finish(&ctx, req, Some((2, 2, vec![255; 16])));
        assert!(frame(&ctx, &mut browser, &scenes, &folders, 2.).is_empty());
        folders[0].2 = "2".into();
        for i in 0..PHOTO_ATTEMPTS {
            let next = frame(&ctx, &mut browser, &scenes, &folders, 3. + i as f64);
            assert_eq!(next.len(), 1);
            browser.finish(&ctx, next[0].strip_prefix("scenephoto:").unwrap(), None);
        }
        assert!(frame(&ctx, &mut browser, &scenes, &folders, 7.).is_empty());
        assert!(browser.photos["A"].texture.is_some());
        assert!(!browser.photos.contains_key("B"));
        browser.open("hidden");
        assert!(frame(&ctx, &mut browser, &scenes, &folders, 8.)[0].contains("\"B\""));
    }

    #[test]
    fn list_mode_stays_picture_free_and_late_replies_cannot_cross_documents() {
        let ctx = crate::ui::test_context();
        let mut browser = Browser::default();
        let scenes = [row("scene:0", "A", "", "")];
        let mut folders = [row("", "All scenes", "1", "doc")];
        let old = frame(&ctx, &mut browser, &scenes, &folders, 0.)
            .pop()
            .unwrap();
        folders[0].3 = "other".into();
        browser.thumbnails = Some(false);
        assert!(frame(&ctx, &mut browser, &scenes, &folders, 1.).is_empty());
        browser.finish(
            &ctx,
            old.strip_prefix("scenephoto:").unwrap(),
            Some((1, 1, vec![255; 4])),
        );
        assert!(browser.photos.is_empty());
    }
}
