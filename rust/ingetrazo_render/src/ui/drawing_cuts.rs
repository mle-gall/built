// SPDX-License-Identifier: GPL-3.0-or-later
//! Paper geometry for the selected hatch's plan and section. Request the
//! same marks the sheet exporter draws, only when a draft changes.
use super::{kit, tokens::*};
use egui::{Color32, Pos2, Rect, Stroke};
use pyo3::prelude::*;

#[derive(Clone, PartialEq)]
pub(super) struct CutSpec {
    pub hatch: String,
    pub kind: String,
    pub size: Vec<f64>,
    pub offset: Vec<f64>,
    pub joints: String,
    pub mortar: bool,
    pub cross: bool,
    pub fitted: bool,
    pub scale: f64,
}

#[derive(Default)]
struct Band {
    width: f32,
    height: f32,
    lines: Vec<Vec<[f32; 2]>>,
    bricks: Vec<Vec<[f32; 2]>>,
}

pub(super) struct Cuts {
    plan: Band,
    section: Band,
}

impl CutSpec {
    fn read(&self, py: Python<'_>, vertical: bool) -> PyResult<Band> {
        // The native module itself: `core.hatches` is the name the `core`
        // package gives it, and only exists once that package is imported.
        let h = py.import("ingetrazo_core")?.getattr("hatches")?;
        let brick = h.call_method1("clean_brick", (&self.size,))?;
        let dims: [f64; 5] = h
            .call_method1("brick_dims", (&brick, &self.kind))?
            .extract()?;
        let [l, w, height, _, head] = dims;
        let wall = match self.hatch.as_str() {
            "brick" | "hollow_brick" | "brick_honeycomb" | "block" | "aac_block"
            | "perforated_block" | "concrete_brick" => w,
            "brick_on_edge" => height,
            "brick_one_half" => l + w + head,
            "ashlar" => 400.,
            "insulation" => 80.,
            "insulation_zigzag" | "wood_cross" => 50.,
            _ if self.cross => 50.,
            _ => l,
        };
        let thick = wall / 10.;
        let length = thick * if vertical { 2.5 } else { 5. };
        let (bw, bh) = if vertical {
            (thick, length)
        } else {
            (length, thick)
        };
        let mut out = Band {
            width: bw as f32,
            height: bh as f32,
            ..Default::default()
        };
        let along = if vertical { (0., 1.) } else { (1., 0.) };
        let across = if vertical { (1., 0.) } else { (0., 1.) };
        let origin = (0., 0.);
        if self.cross && !vertical {
            // The cut along the grain uses its face pattern.
            return Ok(out);
        }
        if self.fitted || self.cross {
            let key = if self.cross {
                "wood_cross"
            } else {
                &self.hatch
            };
            out.lines = h
                .call_method1(
                    "fitted_marks",
                    (key, origin, along, across, length, thick, self.scale),
                )?
                .extract()?;
        } else if self.joints != "none" {
            let cut = if vertical { "vertical" } else { "plan" };
            let band = (origin, along, across, length, thick);
            let factor = h
                .call_method1("real_scale", (&self.hatch, 10., &brick))?
                .extract::<Option<f64>>()?
                .unwrap_or(1.);
            let offset = (self.offset.len() == 2).then_some(&self.offset);
            let marks = h.call_method1(
                "bond_marks",
                (
                    &self.hatch,
                    band,
                    cut,
                    factor * self.scale,
                    10.,
                    &brick,
                    &self.joints,
                    offset,
                ),
            )?;
            if !marks.is_none() {
                out.lines = marks.get_item("polys")?.extract()?;
                let lines: Vec<[f32; 4]> = marks.get_item("lines")?.extract()?;
                out.lines
                    .extend(lines.into_iter().map(|[x, y, u, v]| vec![[x, y], [u, v]]));
                out.bricks = marks.get_item("bricks")?.extract()?;
            }
        }
        Ok(out)
    }
    pub fn build(&self) -> PyResult<Cuts> {
        Python::attach(|py| {
            Ok(Cuts {
                plan: self.read(py, false)?,
                section: self.read(py, true)?,
            })
        })
    }
}

impl Band {
    fn draw(
        &self,
        ui: &mut egui::Ui,
        vertical: bool,
        look: &CutSpec,
        ground: Color32,
        ink: Color32,
    ) {
        let (r, _) = ui.allocate_exact_size(
            egui::vec2(if vertical { 40. } else { 92. }, 44.),
            egui::Sense::hover(),
        );
        let factor = (r.width() / self.width).min(r.height() / self.height);
        let rect = Rect::from_center_size(r.center(), egui::vec2(self.width, self.height) * factor);
        let p = ui.painter_at(rect);
        let mortar = look.mortar && look.joints == "double";
        p.rect_filled(rect, 0, if mortar { INK_2 } else { ground });
        if look.cross && !vertical {
            kit::hatch_swatch(ui, rect, &look.hatch, None, ground, ink);
        }
        let at = |[x, y]: [f32; 2]| rect.min + egui::vec2(x, y) * factor;
        let pen = Stroke::new(0.8_f32, ink);
        for ring in &self.bricks {
            let mut pts: Vec<_> = ring.iter().copied().map(at).collect();
            if pts.first() == pts.last() {
                pts.pop();
            }
            if pts.len() >= 3 {
                p.add(egui::Shape::convex_polygon(pts, ground, pen));
            }
        }
        for line in &self.lines {
            p.add(egui::Shape::line(
                line.iter().copied().map(at).collect(),
                pen,
            ));
        }
        p.rect_stroke(rect, 0, pen, egui::StrokeKind::Inside);
    }
}
impl Cuts {
    pub fn show(&self, ui: &mut egui::Ui, look: &CutSpec, ground: Color32, ink: Color32) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                kit::caption_plain(ui, "Plan");
                self.plan.draw(ui, false, look, ground, ink);
            });
            ui.vertical(|ui| {
                kit::caption_plain(ui, "Section");
                self.section.draw(ui, true, look, ground, ink);
            });
        });
    }
}

pub(super) fn piece_view(ui: &mut egui::Ui, kind: &str, size: &[f64]) {
    if size.len() < 4 {
        return;
    }
    let [l, w, h] = [size[0], size[1], size[2]].map(|v| v as f32);
    if !(l > 0. && w > 0. && h > 0.) {
        return;
    }
    let (r, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().min(160.), 84.),
        egui::Sense::hover(),
    );
    let factor =
        ((r.width() - 16.) / ((l + w) * 0.866)).min((r.height() - 12.) / ((l + w) * 0.5 + h));
    let origin = r.center() - egui::vec2((l - w) * 0.433, (l + w) * 0.25 - h * 0.5) * factor;
    let at = |x, y, z| -> Pos2 { origin + egui::vec2((x - y) * 0.866, (x + y) * 0.5 - z) * factor };
    let quad = |pts: [Pos2; 4], fill| {
        ui.painter().add(egui::Shape::convex_polygon(
            pts.to_vec(),
            fill,
            Stroke::new(1_f32, INK_2),
        ));
    };
    quad(
        [at(0., 0., h), at(l, 0., h), at(l, w, h), at(0., w, h)],
        CONTROL,
    );
    quad(
        [at(0., w, 0.), at(l, w, 0.), at(l, w, h), at(0., w, h)],
        TRACK,
    );
    quad(
        [at(l, 0., 0.), at(l, w, 0.), at(l, w, h), at(l, 0., h)],
        FIELD,
    );
    let (nx, ny) = match kind {
        "perforated" => (
            (l / 45.).round().max(3.) as usize,
            (w / 45.).round().max(2.) as usize,
        ),
        "cored" => ((l / 80.).round().clamp(2., 4.) as usize, 1),
        "block" => (2, 1),
        "hollow" => ((w / 40.).round().max(2.) as usize, 2),
        _ => (0, 0),
    };
    for x in 0..nx.min(14) {
        for y in 0..ny.min(10) {
            let (a, b) = ((x as f32 + 0.2) / nx as f32, (x as f32 + 0.8) / nx as f32);
            let (c, d) = ((y as f32 + 0.2) / ny as f32, (y as f32 + 0.8) / ny as f32);
            let hole = |i, j| {
                if kind == "hollow" {
                    at(l, i * w, j * h)
                } else {
                    at(i * l, j * w, h)
                }
            };
            quad([hole(a, c), hole(b, c), hole(b, d), hole(a, d)], WELL);
        }
    }
    if kind == "aac" {
        for i in 0..26 {
            let a = ((i * 19) % 31) as f32 / 31.;
            let b = ((i * 23) % 29) as f32 / 29.;
            ui.painter().circle_filled(
                match i % 3 {
                    0 => at(a * l, w, b * h),
                    1 => at(l, a * w, b * h),
                    _ => at(a * l, b * w, h),
                },
                0.7,
                INK_2,
            );
        }
    }
    kit::caption_plain(ui, &format!("{l} × {w} × {h} mm"));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cut_previews_use_the_document_bond_and_insulation_geometry() {
        Python::initialize();
        Python::attach(|py| {
            // The native core is an extension module of the project's
            // environment; the test interpreter starts without it.
            let lib = concat!(env!("CARGO_MANIFEST_DIR"), "/../../.venv/lib");
            let site = py.import("site").unwrap();
            for python in std::fs::read_dir(lib).unwrap() {
                let packages = python.unwrap().path().join("site-packages");
                site.call_method1("addsitedir", (packages.to_str().unwrap(),)).unwrap();
            }
            let mut spec = CutSpec {
                hatch: "brick_bond".into(),
                kind: "brick".into(),
                size: vec![240., 120., 60., 10.],
                offset: vec![],
                joints: "single".into(),
                mortar: false,
                cross: false,
                fitted: false,
                scale: 1.,
            };
            let cuts = spec.build().unwrap();
            assert!(!cuts.plan.lines.is_empty() && !cuts.section.lines.is_empty());
            assert!(cuts.plan.width > cuts.plan.height && cuts.section.height > cuts.section.width);
            spec.joints = "double".into();
            spec.mortar = true;
            assert!(!spec.build().unwrap().plan.bricks.is_empty());
            spec.joints = "none".into();
            assert!(spec.build().unwrap().plan.lines.is_empty());
            spec.hatch = "insulation_zigzag".into();
            spec.fitted = true;
            assert!(!spec.build().unwrap().section.lines.is_empty());
        });
    }
}
