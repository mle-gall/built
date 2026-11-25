use super::{label, send};
use crate::ui::tokens;
use egui::{pos2, vec2, Rect, Stroke};
use serde_json::Value;

#[derive(Default)]
pub struct Terrain {
    pub plot: Option<Rect>,
    samples: Vec<(f32, Option<f32>)>,
    low: f32,
    high: f32,
}

impl Terrain {
    pub fn load(&mut self, data: &Value) {
        self.samples = data["samples"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| Some((s[0].as_f64()? as f32, s[1].as_f64().map(|v| v as f32))))
            .collect();
        self.low = self
            .samples
            .iter()
            .filter_map(|s| s.1)
            .fold(f32::INFINITY, f32::min);
        self.high = self
            .samples
            .iter()
            .filter_map(|s| s.1)
            .fold(f32::NEG_INFINITY, f32::max);
    }
    pub fn draw(
        &mut self,
        ctx: &egui::Context,
        bounds: Rect,
        data: &Value,
        projected: &Value,
        out: &mut Vec<String>,
    ) {
        let length = self.samples.last().map_or(0., |s| s.0);
        let positions = projected.as_array().map(Vec::as_slice).unwrap_or_default();
        let pixel = |i: usize| {
            positions
                .get(i)
                .and_then(|p| Some(pos2(p[0].as_f64()? as f32, p[1].as_f64()? as f32)))
        };
        egui::Area::new("terrain_profile".into())
            .pivot(egui::Align2::LEFT_BOTTOM)
            .fixed_pos(bounds.left_bottom())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width((bounds.width() - 16.).max(170.));
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(format!("Terrain profile · {}", label(&data["name"])));
                        ui.weak(label(&data["source"]));
                        for (key, name) in [
                            ("csv", "Export CSV…"),
                            ("image", "Export image…"),
                            ("open", "Recompute"),
                            ("close", "Close"),
                        ] {
                            if ui.button(name).clicked() {
                                send(out, &format!("terrain.{key}"), Value::Null);
                            }
                        }
                    });
                    let (r, resp) = ui
                        .allocate_exact_size(vec2(ui.available_width(), 96.), egui::Sense::hover());
                    self.plot = Some(r);
                    let chart = Rect::from_min_max(r.min + vec2(51., 10.), r.max - vec2(20., 18.));
                    let painter = ui.painter();
                    let ink = tokens::DIM;
                    let grid = Stroke::new(1.0_f32, tokens::DIVIDER);
                    let font = tokens::meta();
                    for i in 0..=4 {
                        let t = i as f32 / 4.;
                        let x = egui::lerp(chart.x_range(), t);
                        painter.vline(x, chart.y_range(), grid);
                        painter.text(
                            pos2(x, chart.bottom() + 2.),
                            egui::Align2::CENTER_TOP,
                            format!("{:.1} m", length * t),
                            font.clone(),
                            ink,
                        );
                    }
                    if self.low.is_finite() {
                        for i in 0..=2 {
                            let t = i as f32 / 2.;
                            let y = egui::lerp(chart.y_range(), 1. - t);
                            painter.hline(chart.x_range(), y, grid);
                            painter.text(
                                pos2(chart.left() - 5., y),
                                egui::Align2::RIGHT_CENTER,
                                format!("{:.2} m", self.low + t * (self.high - self.low)),
                                font.clone(),
                                ink,
                            );
                        }
                    }
                    let xy = |(s, e): (f32, f32)| {
                        pos2(
                            chart.left() + chart.width() * s / length.max(0.001),
                            chart.bottom()
                                - chart.height() * (e - self.low)
                                    / (self.high - self.low).max(0.001),
                        )
                    };
                    for w in self.samples.windows(2) {
                        if let (Some(a), Some(b)) = (w[0].1, w[1].1) {
                            ui.painter().line_segment(
                                [xy((w[0].0, a)), xy((w[1].0, b))],
                                Stroke::new(2.0_f32, tokens::INK_2),
                            );
                        }
                    }
                    let pointer = ctx.pointer_hover_pos();
                    let index = if resp.hovered() {
                        pointer.and_then(|p| {
                            let s = (p.x - chart.left()) / chart.width() * length;
                            self.samples
                                .iter()
                                .enumerate()
                                .min_by(|(_, a), (_, b)| {
                                    (a.0 - s).abs().total_cmp(&(b.0 - s).abs())
                                })
                                .map(|p| p.0)
                        })
                    } else {
                        pointer.and_then(|p| {
                            (0..self.samples.len())
                                .filter_map(|i| Some((i, p.distance(pixel(i)?))))
                                .filter(|p| p.1 < 20.)
                                .min_by(|a, b| a.1.total_cmp(&b.1))
                                .map(|p| p.0)
                        })
                    };
                    if let Some(i) = index {
                        let (s, e) = self.samples[i];
                        let x = chart.left() + chart.width() * s / length.max(0.001);
                        ui.painter().vline(
                            x,
                            chart.y_range(),
                            Stroke::new(1.0_f32, tokens::INK),
                        );
                        if let Some(p) = pixel(i) {
                            ctx.debug_painter().circle_stroke(
                                p,
                                6.,
                                Stroke::new(2.0_f32, tokens::FOCUS_RING),
                            );
                        }
                        ui.label(format!(
                            "Station {s:.1} m · Elevation {}",
                            e.map(|n| format!("{n:.2} m"))
                                .unwrap_or("unavailable".into())
                        ));
                    } else if self.low.is_finite() {
                        ui.label(format!(
                            "Distance {length:.1} m · Elevation {:.2} – {:.2} m",
                            self.low, self.high
                        ));
                    } else {
                        ui.label("Waiting for elevation samples");
                    }
                });
            });
    }
}

/// Encode the plot from the resident window capture at its physical resolution.
pub fn plot_png(
    frame: &[u8],
    width: usize,
    height: usize,
    scale: f32,
    rect: Rect,
    title: &str,
) -> Result<Vec<u8>, String> {
    let px = |n: f32| (n * scale).round().max(0.) as usize;
    let (left, right) = (px(rect.left()).min(width), px(rect.right()).min(width));
    let (top, bottom) = (px(rect.top()).min(height), px(rect.bottom()).min(height));
    if right <= left || bottom <= top || frame.len() != width * height * 4 {
        return Err("The profile plot is outside the frame".into());
    }
    let mut cropped = Vec::with_capacity((bottom - top) * (right - left) * 4);
    for row in top..bottom {
        cropped.extend_from_slice(&frame[4 * (row * width + left)..4 * (row * width + right)]);
    }
    let mut bytes = Vec::new();
    {
        let mut encoder =
            png::Encoder::new(&mut bytes, (right - left) as u32, (bottom - top) as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .add_itxt_chunk("Description".into(), title.into())
            .map_err(|e| e.to_string())?;
        encoder
            .write_header()
            .map_err(|e| e.to_string())?
            .write_image_data(&cropped)
            .map_err(|e| e.to_string())?;
    }
    Ok(bytes)
}
