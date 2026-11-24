//! Font-picker coverage on Windows and Linux uses the capture renderer's shaper.
use crate::{annotation_canvas::AnnotationFont, image_assets::fonts};

pub fn families() -> Vec<String> {
    let mut names: Vec<_> = fonts()
        .faces()
        .filter_map(|face| face.families.first().map(|f| f.0.clone()))
        .collect();
    names.sort_by_cached_key(|s| s.to_lowercase());
    names.dedup();
    names
}

pub fn sample(
    name: &str,
    text: &str,
    bold: bool,
    italic: bool,
    size: f32,
    scale: f32,
    extent: egui::Vec2,
) -> Option<egui::FontImage> {
    AnnotationFont::sample(name, text, bold, italic, size, scale, extent)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shapes_a_family_sample_without_an_application_window() {
        // A minimal Linux runner can have no system font packages.
        let Some(name) = families().into_iter().find(|s| {
            ["Arial", "DejaVu Sans", "Liberation Sans", "Helvetica"].contains(&s.as_str())
        }) else {
            return;
        };
        let image = sample(&name, "A & B", true, false, 15., 2., egui::vec2(80., 24.)).unwrap();
        assert_eq!(image.size, [160, 48]);
        assert!(image.pixels.iter().any(|&a| a > 0.5));
        assert!(image.pixels.contains(&0.));
    }
}
