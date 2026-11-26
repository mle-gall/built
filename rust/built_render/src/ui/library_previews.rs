// SPDX-License-Identifier: GPL-3.0-or-later
//! View-local thumbnail textures. The worker only holds owned drawing inputs.
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, SyncSender};

type Pixels = Option<egui::ColorImage>;
struct Preview {
    pixels: Pixels,
    texture: Option<egui::TextureHandle>,
}

type Job = (String, Box<dyn FnOnce() -> Pixels + Send>);

#[derive(Default)]
pub(super) struct Previews {
    worker: Option<(SyncSender<Job>, Receiver<(String, Pixels)>)>,
    pending: HashSet<String>,
    entries: HashMap<String, (u64, Preview)>,
    clock: u64,
}

impl Previews {
    pub fn needed(&self, key: &str) -> bool {
        self.pending.len() < 8 && !self.pending.contains(key) && !self.entries.contains_key(key)
    }

    pub fn request(&mut self, key: &str, job: impl FnOnce() -> Pixels + Send + 'static) {
        if !self.needed(key) {
            return;
        }
        let (tx, _) = self.worker.get_or_insert_with(|| {
            let (tx, work) = mpsc::sync_channel::<Job>(8);
            let (done, rx) = mpsc::channel();
            std::thread::spawn(move || {
                for (key, job) in work {
                    if done.send((key, job())).is_err() {
                        break;
                    }
                }
            });
            (tx, rx)
        });
        if tx.try_send((key.to_owned(), Box::new(job))).is_ok() {
            self.pending.insert(key.to_owned());
        }
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some((_, rx)) = &self.worker else { return };
        for (key, pixels) in rx.try_iter() {
            ctx.request_repaint();
            self.pending.remove(&key);
            if self.entries.len() >= 128 {
                if let Some(oldest) = self
                    .entries
                    .iter()
                    .min_by_key(|(_, e)| e.0)
                    .map(|(k, _)| k.clone())
                {
                    self.entries.remove(&oldest);
                }
            }
            self.clock += 1;
            let texture = Preview {
                pixels,
                texture: None,
            };
            self.entries.insert(key, (self.clock, texture));
        }
        if !self.pending.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(12));
        }
    }

    pub fn prefetch_file(&mut self, key: &str) {
        if let Some(path) = key.strip_prefix("lib:").filter(|_| self.needed(key)) {
            let path = std::path::PathBuf::from(path);
            self.request(key, move || {
                let image = crate::image_assets::decode_file(&path)?;
                let side = image.width().min(image.height());
                let crop = image::imageops::crop_imm(
                    &image,
                    (image.width() - side) / 2,
                    (image.height() - side) / 2,
                    side,
                    side,
                );
                let pixels = crate::image_assets::resize(crop.to_image(), 192, 192);
                Some(egui::ColorImage::from_rgba_unmultiplied(
                    [192, 192],
                    pixels.as_raw(),
                ))
            });
        }
    }

    pub fn texture(&mut self, ctx: &egui::Context, key: &str) -> Option<&egui::TextureHandle> {
        self.prefetch_file(key);
        let (used, texture) = self.entries.get_mut(key)?;
        self.clock += 1;
        *used = self.clock;
        if let Some(pixels) = texture.pixels.take() {
            texture.texture = Some(ctx.load_texture(key, pixels, egui::TextureOptions::LINEAR));
        }
        texture.texture.as_ref()
    }
}
