//! The program's own toolbar, menu and panel icons (formerly drawn with
//! QPainter in `views/icons.py`), as theme-independent weight atlases.
//!
//! A source-over drawing in three theme colours is, per pixel, affine in
//! those colours. `icon_weights.png` holds each 48 px cell's ink, accent and
//! base weights in R, G and B with coverage in A; `icon_fixed.png` holds the
//! premultiplied colour that does not follow the theme (two layer icons
//! keep their own blue and green). Recolouring reproduces the painted
//! pixmap exactly, up to rounding.
use image::RgbaImage;
use pyo3::{prelude::*, types::PyBytes};
use std::sync::OnceLock;

const SIDE: u32 = 48;

type Rgb = (u8, u8, u8);

struct Atlas {
    side: u32,
    columns: u32,
    keys: &'static str,
    weights: RgbaImage,
    fixed: RgbaImage,
}

fn load(bytes: &[u8]) -> RgbaImage {
    image::load_from_memory(bytes).expect("invalid embedded icon atlas").into_rgba8()
}

/// The 48 px tool, view, panel and sheet icons and panel initials.
fn icons() -> &'static Atlas {
    static ATLAS: OnceLock<Atlas> = OnceLock::new();
    ATLAS.get_or_init(|| Atlas {
        side: SIDE,
        columns: 16,
        keys: include_str!("../../assets/icon_keys.txt"),
        weights: load(include_bytes!("../../assets/icon_weights.png")),
        fixed: load(include_bytes!("../../assets/icon_fixed.png")),
    })
}

/// The 18 px marks of the Classic tree rows (formerly views/tree_icons.py):
/// ink is the palette's text, base the palette's base.
fn rows() -> &'static Atlas {
    static ATLAS: OnceLock<Atlas> = OnceLock::new();
    ATLAS.get_or_init(|| Atlas {
        side: 18,
        columns: 8,
        keys: include_str!("../../assets/row_icon_keys.txt"),
        weights: load(include_bytes!("../../assets/row_icon_weights.png")),
        fixed: load(include_bytes!("../../assets/row_icon_fixed.png")),
    })
}

impl Atlas {
    fn cell(&self, key: &str) -> Option<u32> {
        self.keys.lines().position(|k| k == key).map(|i| i as u32)
    }

    fn paint(&self, key: &str, ink: Rgb, accent: Rgb, base: Rgb) -> Option<Vec<u8>> {
        let index = self.cell(key)?;
        let side = self.side;
        let (x0, y0) = (index % self.columns * side, index / self.columns * side);
        let mut out = Vec::with_capacity((side * side * 4) as usize);
        let colours = [[ink.0, ink.1, ink.2], [accent.0, accent.1, accent.2], [base.0, base.1, base.2]];
        for y in y0..y0 + side {
            for x in x0..x0 + side {
                let w = self.weights.get_pixel(x, y).0;
                let f = self.fixed.get_pixel(x, y).0;
                for c in 0..3 {
                    let sum: u32 = (0..3).map(|layer| u32::from(w[layer]) * u32::from(colours[layer][c])).sum();
                    let value = u32::from(f[c]) + (sum + 127) / 255;
                    out.push(value.min(u32::from(w[3])) as u8);
                }
                out.push(w[3]);
            }
        }
        Some(out)
    }
}

/// Every key the atlas holds, in atlas order. Letters are `letter:A`.
pub fn keys() -> impl Iterator<Item = &'static str> {
    icons().keys.lines().filter(|k| !k.is_empty())
}

fn cell(key: &str) -> Option<u32> {
    icons().cell(key)
}

fn initial(name: &str) -> char {
    name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().next().unwrap_or(c)).unwrap_or('?')
}

/// The atlas cell for a panel without its own icon: its initial in a
/// rounded frame. Latin capitals, the accented ones of the shipped
/// languages and digits are baked; other initials have no cell.
pub fn letter_key(name: &str) -> Option<String> {
    Some(format!("letter:{}", initial(name))).filter(|k| cell(k).is_some())
}

/// An initial outside the atlas, drawn with the system fonts in the same
/// frame: the frame at the thinned icon stroke, the letter 20 px.
fn letter_drawn(c: char, ink: Rgb) -> Option<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let colour = format!("#{:02x}{:02x}{:02x}", ink.0, ink.1, ink.2);
    let text: String = match c {
        '&' => "&amp;".into(),
        '<' => "&lt;".into(),
        c => c.into(),
    };
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='48' height='48'>\
         <rect x='9' y='9' width='30' height='30' rx='6' fill='none' stroke='{colour}' stroke-width='1.95'/>\
         <text x='24' y='31' font-size='20' text-anchor='middle' fill='{colour}'>{text}</text></svg>");
    let options = usvg::Options { fontdb: crate::image_assets::fonts().clone(), ..Default::default() };
    let tree = usvg::Tree::from_str(&svg, &options).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(SIDE, SIDE)?;
    resvg::render(&tree, tiny_skia::Transform::identity(), &mut pixmap.as_mut());
    Some(pixmap.take())
}

/// A panel's letter icon, premultiplied.
pub fn letter(name: &str, ink: Rgb, accent: Rgb, base: Rgb) -> Option<Vec<u8>> {
    match letter_key(name) {
        Some(key) => premultiplied(&key, ink, accent, base),
        None => letter_drawn(initial(name), ink),
    }
}

/// A 48 × 48 premultiplied RGBA icon in the given colours.
pub fn premultiplied(key: &str, ink: Rgb, accent: Rgb, base: Rgb) -> Option<Vec<u8>> {
    icons().paint(key, ink, accent, base)
}

/// `(48, 48, premultiplied RGBA)` for a Classic toolbar, menu or tab icon,
/// or None for a key the atlas does not hold.
#[pyfunction]
pub fn icon_art<'py>(py: Python<'py>, key: &str, ink: Rgb, accent: Rgb, base: Rgb) -> Option<(u32, u32, Bound<'py, PyBytes>)> {
    premultiplied(key, ink, accent, base).map(|px| (SIDE, SIDE, PyBytes::new(py, &px)))
}

/// `(18, 18, premultiplied RGBA)` for a Classic tree row mark: `scene`,
/// `lock`, `lock_open`, `eye`, `eye_off`, `isolate_on` or `isolate_off`.
#[pyfunction]
pub fn row_icon_art<'py>(py: Python<'py>, key: &str, text: Rgb, fill: Rgb, base: Rgb) -> Option<(u32, u32, Bound<'py, PyBytes>)> {
    let side = rows().side;
    rows().paint(key, text, fill, base).map(|px| (side, side, PyBytes::new(py, &px)))
}

#[pyfunction]
pub fn icon_keys() -> Vec<&'static str> {
    keys().filter(|k| !k.starts_with("letter:")).collect()
}

/// `(48, 48, premultiplied RGBA)`: the initial of ``name`` in a frame.
#[pyfunction]
pub fn letter_art<'py>(py: Python<'py>, name: &str, ink: Rgb, accent: Rgb, base: Rgb) -> Option<(u32, u32, Bound<'py, PyBytes>)> {
    letter(name, ink, accent, base).map(|px| (SIDE, SIDE, PyBytes::new(py, &px)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_has_a_drawn_cell() {
        let n = keys().count() as u32;
        for atlas in [icons(), rows()] {
            let n = atlas.keys.lines().count() as u32;
            assert_eq!(atlas.weights.dimensions(), atlas.fixed.dimensions());
            assert!(atlas.weights.width() == atlas.columns * atlas.side
                && atlas.weights.height() >= n.div_ceil(atlas.columns) * atlas.side);
            for key in atlas.keys.lines() {
                let px = atlas.paint(key, (255, 255, 255), (255, 255, 255), (255, 255, 255)).unwrap();
                assert!(px.chunks(4).any(|p| p[3] > 0), "{key} is blank");
            }
        }
        assert!(n >= 180);
        for key in keys() {
            let px = premultiplied(key, (255, 255, 255), (255, 255, 255), (255, 255, 255)).unwrap();
            assert!(px.chunks(4).any(|p| p[3] > 0), "{key} is blank");
        }
    }

    #[test]
    fn colours_follow_the_theme_and_fixed_ones_stay() {
        let a = premultiplied("line", (255, 0, 0), (0, 255, 0), (0, 0, 0)).unwrap();
        let b = premultiplied("line", (0, 0, 255), (0, 255, 0), (0, 0, 0)).unwrap();
        assert_ne!(a, b);
        let black = premultiplied("layers_colour", (0, 0, 0), (0, 0, 0), (0, 0, 0)).unwrap();
        assert!(black.chunks(4).any(|p| p[2] > 100), "the layer blue does not follow the theme");
        assert!(premultiplied("no such icon", (0, 0, 0), (0, 0, 0), (0, 0, 0)).is_none());
    }

    #[test]
    fn letters() {
        assert_eq!(letter_key("zeta").as_deref(), Some("letter:Z"));
        assert_eq!(letter_key("  3D").as_deref(), Some("letter:3"));
        assert_eq!(letter_key("été").as_deref(), Some("letter:É"));
        assert_eq!(letter_key("").as_deref(), Some("letter:?"));
        assert_eq!(letter_key("Ωmega"), None);
        let drawn = letter("Ωmega", (255, 255, 255), (0, 0, 0), (0, 0, 0)).unwrap();
        let frame = letter("", (255, 255, 255), (0, 0, 0), (0, 0, 0)).unwrap();
        assert_eq!(drawn.len(), frame.len());
        // The frame's left side lies at x = 9 in both.
        let at = |px: &[u8], x: usize, y: usize| px[(y * 48 + x) * 4 + 3];
        assert!(at(&drawn, 9, 24) > 100 && at(&frame, 9, 24) > 100);
    }
}
