//! The inline site picker. A paint can read a cached tile, but only emits
//! requests for missing tiles. The host delivers those after the frame.
use super::{kit, tokens::*};
use egui::{pos2, vec2, Pos2, Rect, Sense, Vec2};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::Path,
};

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct MapSpec {
    pub lat: f64,
    pub lon: f64,
    pub zoom: u8,
    pub max_zoom: u8,
    pub source: String,
    pub cache: String,
    pub drawing: bool,
    pub corners: Option<[f64; 4]>,
    pub limited: bool,
    pub origin: String,
    pub attribution: String,
    pub revision: u64,
}

const TILE: f64 = 256.;
fn world(lat: f64, lon: f64, z: u8) -> (f64, f64) {
    let scale = TILE * f64::from(1u32 << z);
    (
        (lon + 180.) / 360. * scale,
        (1. - lat.clamp(-85., 85.).to_radians().tan().asinh() / std::f64::consts::PI) * 0.5 * scale,
    )
}
fn location(x: f64, y: f64, z: u8) -> (f64, f64) {
    let scale = TILE * f64::from(1u32 << z);
    let lat = (std::f64::consts::PI * (1. - 2. * y / scale))
        .sinh()
        .atan()
        .to_degrees()
        .clamp(-85., 85.);
    (lat, (x / scale * 360.).rem_euclid(360.) - 180.)
}
fn point(center: (f64, f64), rect: Rect, pos: Pos2, z: u8) -> (f64, f64) {
    let d = pos - rect.center();
    location(center.0 + f64::from(d.x), center.1 + f64::from(d.y), z)
}
fn project(lat: f64, lon: f64, center: (f64, f64), rect: Rect, z: u8) -> Pos2 {
    let (x, y) = world(lat, lon, z);
    let scale = TILE * f64::from(1u32 << z);
    let dx = (x - center.0 + scale / 2.).rem_euclid(scale) - scale / 2.;
    rect.center() + vec2(dx as f32, (y - center.1) as f32)
}

type Tile = (i32, i32, u8);
#[derive(Default)]
pub struct MapState {
    source: String,
    cache: String,
    textures: HashMap<Tile, egui::TextureHandle>,
    requested: HashSet<Tile>,
    checked: HashSet<Tile>,
    revision: u64,
    anchor: Option<Pos2>,
}
impl MapState {
    fn tile(&mut self, ctx: &egui::Context, k: Tile) -> Option<&egui::TextureHandle> {
        if !self.textures.contains_key(&k) && self.checked.insert(k) {
            let path = Path::new(&self.cache)
                .join(&self.source)
                .join(k.2.to_string())
                .join(k.0.to_string())
                .join(k.1.to_string());
            let mut data = Vec::new();
            if let Ok(file) = std::fs::File::open(path) {
                if file
                    .take(8 * 1024 * 1024 + 1)
                    .read_to_end(&mut data)
                    .is_ok()
                    && data.len() <= 8 * 1024 * 1024
                {
                    let image = image::ImageReader::new(std::io::Cursor::new(&data))
                        .with_guessed_format()
                        .ok()
                        .and_then(|mut reader| {
                            let mut limits = image::Limits::default();
                            limits.max_image_width = Some(1024);
                            limits.max_image_height = Some(1024);
                            reader.limits(limits);
                            reader.decode().ok()
                        });
                    if let Some(image) = image {
                        let rgba = image.into_rgba8();
                        let pixels = egui::ColorImage::from_rgba_unmultiplied(
                            [rgba.width() as usize, rgba.height() as usize],
                            &rgba,
                        );
                        self.textures.insert(
                            k,
                            ctx.load_texture(
                                format!("map-{:?}", k),
                                pixels,
                                egui::TextureOptions::LINEAR,
                            ),
                        );
                    }
                }
            }
        }
        self.textures.get(&k)
    }

    pub fn show(&mut self, ui: &mut egui::Ui, s: &MapSpec, out: &mut Vec<String>) {
        if s.source.is_empty()
            || !s
                .source
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return;
        }
        if self.source != s.source || self.cache != s.cache {
            *self = Self {
                source: s.source.clone(),
                cache: s.cache.clone(),
                ..Default::default()
            };
        }
        if self.revision != s.revision {
            self.checked.clear();
            self.revision = s.revision;
        }
        let z = s.zoom.clamp(1, 22);
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 240.), Sense::drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, R_ROW, WELL);
        let origin = world(s.lat, s.lon, z);
        let mouse = response.interact_pointer_pos();
        if response.drag_started() {
            self.anchor = ui.input(|i| i.pointer.press_origin());
        }
        let delta = mouse.zip(self.anchor).map_or(Vec2::ZERO, |(p, a)| p - a);
        let center = if s.drawing {
            origin
        } else {
            (origin.0 - f64::from(delta.x), origin.1 - f64::from(delta.y))
        };
        let lo = (
            center.0 - f64::from(rect.width()) / 2.,
            center.1 - f64::from(rect.height()) / 2.,
        );
        let n = 1i32 << z;
        let mut visible = HashSet::new();
        let mut missing = Vec::new();
        for y in
            (lo.1 / TILE).floor() as i32..=((lo.1 + f64::from(rect.height())) / TILE).floor() as i32
        {
            if !(0..n).contains(&y) {
                continue;
            }
            for x in (lo.0 / TILE).floor() as i32
                ..=((lo.0 + f64::from(rect.width())) / TILE).floor() as i32
            {
                let k = (x.rem_euclid(n), y, z);
                visible.insert(k);
                let min = rect.min
                    + vec2(
                        (f64::from(x) * TILE - lo.0) as f32,
                        (f64::from(y) * TILE - lo.1) as f32,
                    );
                let r = Rect::from_min_size(min, Vec2::splat(TILE as f32));
                if let Some(tex) = self.tile(ui.ctx(), k) {
                    painter.image(
                        tex.id(),
                        r,
                        Rect::from_min_max(Pos2::ZERO, pos2(1., 1.)),
                        BRIGHT,
                    );
                } else if self.requested.insert(k) {
                    missing.push(format!("{},{},{}", k.0, k.1, k.2));
                }
            }
        }
        self.textures.retain(|k, _| visible.contains(k));
        self.requested.retain(|k| visible.contains(k));
        self.checked.retain(|k| visible.contains(k));
        if !missing.is_empty() {
            out.push(format!("site_tiles\u{1f}{}", missing.join("\u{1f}")));
        }
        let dragging = s.drawing && self.anchor.is_some() && mouse.is_some();
        let band = if dragging {
            Some(Rect::from_two_pos(self.anchor.unwrap(), mouse.unwrap()))
        } else {
            s.corners.map(|[a, b, c, d]| {
                Rect::from_two_pos(
                    project(a, b, center, rect, z),
                    project(c, d, center, rect, z),
                )
            })
        };
        if let Some(r) = band {
            let edge = if s.limited { WARN } else { ACCENT };
            painter.rect_filled(r, 0, edge.gamma_multiply(0.15));
            painter.rect_stroke(
                r,
                0,
                egui::Stroke::new(MARKER_W, edge),
                egui::StrokeKind::Inside,
            );
        }
        kit::site_pin(&painter, rect.center(), &s.origin);
        if response.drag_stopped() {
            if dragging && delta.length() > 2. {
                let (a, b) = point(origin, rect, self.anchor.unwrap(), z);
                let (c, d) = point(origin, rect, mouse.unwrap(), z);
                out.push(format!("site_rect\u{1f}{a}\u{1f}{b}\u{1f}{c}\u{1f}{d}"));
            } else if !s.drawing {
                let (lat, lon) = location(center.0, center.1, z);
                out.push(format!("site_center\u{1f}{lat}\u{1f}{lon}\u{1f}{z}"));
            }
            self.anchor = None;
        }
        if response.hovered() {
            let dy = ui.input(|i| i.raw_scroll_delta.y);
            if dy.abs() > 0. {
                let nz = (i32::from(z) + if dy > 0. { 1 } else { -1 })
                    .clamp(1, i32::from(s.max_zoom.clamp(1, 22))) as u8;
                ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
                if nz != z {
                    let d = response
                        .hover_pos()
                        .map_or(Vec2::ZERO, |p| p - rect.center());
                    let ratio = 2f64.powi(i32::from(nz) - i32::from(z));
                    let (lat, lon) = location(
                        (center.0 + f64::from(d.x)) * ratio - f64::from(d.x),
                        (center.1 + f64::from(d.y)) * ratio - f64::from(d.y),
                        nz,
                    );
                    out.push(format!("site_center\u{1f}{lat}\u{1f}{lon}\u{1f}{nz}"));
                }
            }
        }
        ui.label(egui::RichText::new(&s.attribution).font(meta()).color(DIM));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn map_paints_cached_tiles_and_emits_only_missing_requests_after_layout() {
        let root = std::env::temp_dir().join(format!("built-map-test-{}", std::process::id()));
        let folder = root.join("osm/3/4");
        std::fs::create_dir_all(&folder).unwrap();
        image::RgbImage::from_pixel(2, 2, image::Rgb([20, 100, 40]))
            .save_with_format(folder.join("4"), image::ImageFormat::Png)
            .unwrap();
        let spec = MapSpec {
            lat: 0.,
            lon: 0.,
            zoom: 3,
            max_zoom: 19,
            source: "osm".into(),
            cache: root.to_string_lossy().into_owned(),
            drawing: false,
            corners: None,
            limited: false,
            origin: String::new(),
            attribution: String::new(),
            revision: 0,
        };
        let ctx = egui::Context::default();
        let mut state = MapState::default();
        let mut actions = Vec::new();
        for frame in 0..2 {
            actions.clear();
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(480., 340.))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .show(ctx, |ui| state.show(ui, &spec, &mut actions));
                },
            );
            assert!(state.textures.contains_key(&(4, 4, 3)));
            if frame == 0 {
                assert_eq!(actions.len(), 1);
                assert!(actions[0].starts_with("site_tiles\u{1f}"));
                assert!(!actions[0].split('\u{1f}').any(|tile| tile == "4,4,3"));
            } else {
                assert!(actions.is_empty());
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn slippy_map_wraps_tiles_and_the_cursor_stays_on_its_site_during_zoom() {
        let at = world(-15.8402, 179.999, 12);
        let rect = Rect::from_min_size(pos2(9., 11.), vec2(480., 340.));
        let cursor = rect.center() + vec2(120., -46.);
        let old = point(at, rect, cursor, 12);
        let next = ((at.0 + 120.) * 2. - 120., (at.1 - 46.) * 2. + 46.);
        let after = point(next, rect, cursor, 13);
        assert!((old.0 - after.0).abs() < 1e-9 && (old.1 - after.1).abs() < 1e-9);
        assert!(old.1 < 0.);
    }
}
