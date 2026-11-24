//! Outline the shaped line in font-sized units. Keep HarfBuzz's source clusters
//! so ligatures and combining marks stay together in editable letter groups.
use crate::{annotation_canvas::AnnotationFont, image_assets::fonts};
use kurbo::{BezPath, PathEl, Point};
use pyo3::prelude::*;
use resvg::usvg::fontdb::ID;
use rustybuzz::{
    ttf_parser::{Face, OutlineBuilder},
    Direction, UnicodeBuffer,
};
use std::collections::{BTreeMap, HashMap};
use unicode_bidi::BidiInfo;
use unicode_script::{Script, UnicodeScript};

type Rings = Vec<Vec<(f64, f64)>>;
#[derive(Default)]
struct Outline(BezPath);
impl OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((x as f64, y as f64));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((x as f64, y as f64));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to((x1 as f64, y1 as f64), (x as f64, y as f64));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.curve_to(
            (x1 as f64, y1 as f64),
            (x2 as f64, y2 as f64),
            (x as f64, y as f64),
        );
    }
    fn close(&mut self) {
        self.0.close_path();
    }
}

fn has_char(font: ID, c: char) -> bool {
    fonts()
        .with_face_data(font, |bytes, index| {
            Face::parse(bytes, index)
                .ok()
                .is_some_and(|f| f.glyph_index(c).is_some())
        })
        .unwrap_or(false)
}

fn shape(text: &str, family: &str, bold: bool, italic: bool) -> BTreeMap<usize, Rings> {
    let mut letters = BTreeMap::<usize, Rings>::new();
    let Some(primary) = AnnotationFont::outline_face(family, bold, italic) else {
        return letters;
    };
    let offsets: HashMap<usize, usize> = text
        .char_indices()
        .enumerate()
        .map(|(i, (offset, _))| (offset, i))
        .collect();
    let mut fallback = HashMap::new();
    let mut pen = 0.;
    let bidi = BidiInfo::new(text, None);
    for paragraph in &bidi.paragraphs {
        let (levels, runs) = bidi.visual_runs(paragraph, paragraph.range.clone());
        for run in runs {
            let rtl = levels[run.start].is_rtl();
            let mut segments: Vec<(usize, usize, ID, Script)> = Vec::new();
            let mut script = Script::Common;
            for (local, c) in text[run.clone()].char_indices() {
                let at = run.start + local;
                let face = *fallback.entry(c).or_insert_with(|| {
                    if c.is_control() || has_char(primary, c) {
                        primary
                    } else {
                        fonts()
                            .faces()
                            .find(|f| has_char(f.id, c))
                            .map_or(primary, |f| f.id)
                    }
                });
                let next_script = c.script();
                if !matches!(next_script, Script::Common | Script::Inherited) {
                    script = next_script;
                }
                if let Some(last) = segments.last_mut() {
                    if last.2 == face && (last.3 == script || last.3 == Script::Common) {
                        last.1 = at + c.len_utf8();
                        last.3 = script;
                        continue;
                    }
                }
                segments.push((at, at + c.len_utf8(), face, script));
            }
            if rtl {
                segments.reverse();
            }
            for (start, end, id, _) in segments {
                fonts().with_face_data(id, |data, index| {
                    let Some(face) = rustybuzz::Face::from_slice(data, index) else {
                        return;
                    };
                    let scale = 100. / face.units_per_em() as f64;
                    let mut buf = UnicodeBuffer::new();
                    buf.push_str(&text[start..end]);
                    buf.set_direction(if rtl {
                        Direction::RightToLeft
                    } else {
                        Direction::LeftToRight
                    });
                    buf.guess_segment_properties();
                    let output = rustybuzz::shape(&face, &[], buf);
                    for (glyph, pos) in output.glyph_infos().iter().zip(output.glyph_positions()) {
                        let left = pen + f64::from(pos.x_offset) * scale;
                        let bottom = f64::from(pos.y_offset) * scale;
                        pen += f64::from(pos.x_advance) * scale;
                        let mut path = Outline::default();
                        if face
                            .outline_glyph(
                                rustybuzz::ttf_parser::GlyphId(glyph.glyph_id as u16),
                                &mut path,
                            )
                            .is_none()
                        {
                            continue;
                        }
                        let Some(owner) = offsets.get(&(start + glyph.cluster as usize)) else {
                            continue;
                        };
                        let rings = letters.entry(*owner).or_default();
                        let mut ring = Vec::new();
                        let transform = |p: Point| (left + p.x * scale, bottom + p.y * scale);
                        kurbo::flatten(path.0.elements().iter().copied(), 0.22 / scale, |el| {
                            match el {
                                PathEl::MoveTo(p) => {
                                    ring.clear();
                                    ring.push(transform(p));
                                }
                                PathEl::LineTo(p) => ring.push(transform(p)),
                                PathEl::ClosePath => {
                                    if ring.first() == ring.last() {
                                        ring.pop();
                                    }
                                    if ring.len() >= 3 {
                                        rings.push(std::mem::take(&mut ring));
                                    }
                                }
                                _ => unreachable!(),
                            }
                        });
                    }
                });
            }
        }
    }
    letters
}

#[pyfunction]
pub fn text_outlines(
    py: Python<'_>,
    text: &str,
    family: &str,
    bold: bool,
    italic: bool,
) -> BTreeMap<usize, Rings> {
    py.detach(|| shape(text, family, bold, italic))
}
