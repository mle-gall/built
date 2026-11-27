//! Tool pointers from the original Built drawings. The atlas contains 96 px
//! masters in eight columns, in tool order, plain then guide/copy mode.
use image::{imageops, RgbaImage};
use pyo3::{prelude::*, types::PyBytes};
use std::sync::OnceLock;

// Hotspots use the artwork's 48-unit coordinate space.
const TOOLS: &[(&str, [f32; 2])] = &[
    ("arc", [6.0, 42.0]),
    ("arc3", [6.0, 42.0]),
    ("center_arc", [6.0, 42.0]),
    ("circle", [6.0, 42.0]),
    ("dimension", [12.0, 30.0]),
    ("eraser", [15.0, 31.0]),
    ("eyedropper", [9.5, 38.5]),
    ("fillet", [24.0, 24.0]),
    ("first_person", [24.0, 24.0]),
    ("flip", [24.0, 24.0]),
    ("followme", [24.0, 24.0]),
    ("freehand", [6.0, 42.0]),
    ("geopath", [6.0, 42.0]),
    ("line", [6.0, 42.0]),
    ("line_on_surface", [6.0, 42.0]),
    ("look_around", [24.0, 26.0]),
    ("move", [24.0, 24.0]),
    ("north", [24.0, 28.0]),
    ("offset", [3.0, 3.0]),
    ("orbit", [24.0, 24.0]),
    ("paint", [13.0, 35.0]),
    ("pan", [24.0, 24.0]),
    ("pie", [6.0, 42.0]),
    ("polygon", [6.0, 42.0]),
    ("position_camera", [23.0, 44.0]),
    ("protractor", [24.0, 24.0]),
    ("pushpull", [24.0, 24.0]),
    ("rectangle", [6.0, 42.0]),
    ("rectangle_center", [6.0, 42.0]),
    ("rotate", [24.0, 24.0]),
    ("rotated_rect", [6.0, 42.0]),
    ("scale", [24.0, 24.0]),
    ("section", [24.0, 27.0]),
    ("tape", [10.0, 28.0]),
    ("text", [24.0, 24.0]),
    ("text3d", [24.0, 24.0]),
    ("walk", [24.0, 24.0]),
    ("zoom", [21.0, 21.0]),
    ("zoom_window", [21.0, 22.0]),
    ("solid-no", [1.5, 1.5]),
    ("solid-1", [1.5, 1.5]),
    ("solid-2", [1.5, 1.5]),
];

fn lookup(key: &str) -> Option<(u32, [f32; 2])> {
    let key = if key == "chamfer" { "fillet" } else { key };
    TOOLS
        .iter()
        .enumerate()
        .find_map(|(index, &(name, hot))| (name == key).then_some((index as u32, hot)))
}

/// A stable key lets winit reuse an already-created platform cursor.
#[pyfunction]
pub fn tool_cursor_name(key: Option<&str>, plus: bool, style: &str) -> Option<String> {
    let key = key.filter(|s| !s.is_empty() && *s != "select")?;
    if style.eq_ignore_ascii_case("crosshair") {
        return Some("crosshair".into());
    }
    if style.eq_ignore_ascii_case("arrow") {
        return None;
    }
    lookup(key)?;
    Some(format!("tool:{key}{}", if plus { "+" } else { "" }))
}

pub(crate) fn pixels(name: &str, side: u32) -> Option<(RgbaImage, [f32; 2])> {
    let key = name.strip_suffix('+').unwrap_or(name);
    let (index, hot) = lookup(key)?;
    if !(1..=256).contains(&side) {
        return None;
    }
    const MASTER: u32 = 96;
    const COLUMNS: u32 = 8;
    static SHEET: OnceLock<RgbaImage> = OnceLock::new();
    let sheet = SHEET.get_or_init(|| {
        image::load_from_memory(include_bytes!("../assets/tool_cursors.png"))
            .expect("invalid embedded cursor artwork")
            .into_rgba8()
    });
    let cell = 2 * index + u32::from(key.len() != name.len());
    let mut art = imageops::crop_imm(
        sheet,
        cell % COLUMNS * MASTER,
        cell / COLUMNS * MASTER,
        MASTER,
        MASTER,
    )
    .to_image();
    if side != MASTER {
        crate::image_assets::premultiply(&mut art);
        art = imageops::resize(&art, side, side, imageops::FilterType::Triangle);
        crate::image_assets::unpremultiply(&mut art);
    }
    Some((art, hot))
}

pub fn winit_pixels(key: &str) -> Option<(u16, u16, u16, u16, Vec<u8>)> {
    let (art, hot) = pixels(key, 32)?;
    let [x, y] = hot.map(|v| (v * 32. / 48.).round() as u16);
    Some((32, 32, x, y, art.into_raw()))
}

/// Classic toolkit adapter. Its hotspot units vary between X11 and Wayland.
#[pyfunction]
pub fn tool_cursor_art<'py>(
    py: Python<'py>,
    key: &str,
    side: u32,
) -> Option<(Bound<'py, PyBytes>, [f32; 2])> {
    pixels(key, side).map(|(art, hot)| (PyBytes::new(py, art.as_raw()), hot))
}

/// Rail art drawn as SVG on the same 48-unit grid, 3-unit round strokes and
/// 45 % fills as the atlas (the Sheets tools that had none, P10).
const RAIL_SVG: &[(&str, &str)] = &[
    ("line_on_surface", include_str!("../assets/line_on_surface.svg")),
    ("comp_vista", include_str!("../assets/sheet_icons/comp_vista.svg")),
    ("comp_etiqueta", include_str!("../assets/sheet_icons/comp_etiqueta.svg")),
    ("comp_nivel", include_str!("../assets/sheet_icons/comp_nivel.svg")),
    ("comp_llamada", include_str!("../assets/sheet_icons/comp_llamada.svg")),
    ("image", include_str!("../assets/sheet_icons/image.svg")),
    ("comp_cajetin", include_str!("../assets/sheet_icons/comp_cajetin.svg")),
    ("comp_escala", include_str!("../assets/sheet_icons/comp_escala.svg")),
    ("comp_norte", include_str!("../assets/sheet_icons/comp_norte.svg")),
    ("comp_leyenda", include_str!("../assets/sheet_icons/comp_leyenda.svg")),
    ("comp_perfil", include_str!("../assets/sheet_icons/comp_perfil.svg")),
    ("comp_flecha", include_str!("../assets/sheet_icons/comp_flecha.svg")),
    ("comp_terreno", include_str!("../assets/sheet_icons/comp_terreno.svg")),
    ("dimension_chain", include_str!("../assets/sheet_icons/dimension_chain.svg")),
    ("dimension_baseline", include_str!("../assets/sheet_icons/dimension_baseline.svg")),
    ("dimension_angular", include_str!("../assets/sheet_icons/dimension_angular.svg")),
    ("dimension_radius", include_str!("../assets/sheet_icons/dimension_radius.svg")),
];

const RAIL_PX: u32 = 48;

fn svg_pixels(svg: &str) -> Option<RgbaImage> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).ok()?;
    let mut map = tiny_skia::Pixmap::new(RAIL_PX, RAIL_PX)?;
    let scale = RAIL_PX as f32 / tree.size().width();
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut map.as_mut());
    // White with the art's coverage as alpha, like the atlas, so egui can tint it.
    let rgba = map.data().chunks_exact(4).flat_map(|p| [255, 255, 255, p[3]]).collect();
    RgbaImage::from_raw(RAIL_PX, RAIL_PX, rgba)
}

/// A rail icon as 48-px white-alpha pixels: the atlas, then the SVG art.
pub fn rail_pixels(key: &str) -> Option<RgbaImage> {
    static ATLAS: OnceLock<RgbaImage> = OnceLock::new();
    if let Some(index) = include_str!("../assets/rail_icons.txt").lines().position(|name| name == key) {
        let atlas = ATLAS.get_or_init(|| image::load_from_memory(include_bytes!("../assets/rail_icons.png"))
            .expect("invalid embedded rail artwork").into_rgba8());
        let i = index as u32;
        return Some(imageops::crop_imm(atlas, i % 8 * RAIL_PX, i / 8 * RAIL_PX, RAIL_PX, RAIL_PX).to_image());
    }
    RAIL_SVG.iter().find(|(name, _)| *name == key).and_then(|(_, svg)| svg_pixels(svg))
}

/// 48-pixel white-alpha masters for the native rail and view-command cluster.
#[pyfunction]
pub fn rail_icon<'py>(py: Python<'py>, key: &str) -> (u32, u32, Bound<'py, PyBytes>) {
    match rail_pixels(key) {
        Some(image) => (RAIL_PX, RAIL_PX, PyBytes::new(py, image.as_raw())),
        None => (0, 0, PyBytes::new(py, &[])),
    }
}

#[cfg(test)]
mod rail_art_tests {
    use super::*;

    #[test]
    fn every_svg_icon_draws_inside_its_cell() {
        for (key, _) in RAIL_SVG {
            let image = rail_pixels(key).unwrap_or_else(|| panic!("{key} does not render"));
            let ink = image.pixels().filter(|p| p[3] > 128).count();
            assert!(ink > 120, "{key}: only {ink} inked pixels");
            // The art keeps the atlas's margin: nothing on the outer pixel ring.
            for (x, y, p) in image.enumerate_pixels() {
                if x == 0 || y == 0 || x == RAIL_PX - 1 || y == RAIL_PX - 1 {
                    assert!(p[3] < 40, "{key} touches the cell edge at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn every_sheets_rail_tool_has_art() {
        for (key, label, icon) in crate::sheet_workspace::RAIL.iter().flat_map(|g| g.iter()) {
            assert!(rail_pixels(icon).is_some(), "{key} ({label}) has no icon {icon}");
        }
    }

    #[test]
    fn svg_art_does_not_shadow_the_atlas() {
        let atlas: Vec<&str> = include_str!("../assets/rail_icons.txt").lines().collect();
        for (key, _) in RAIL_SVG {
            assert!(!atlas.contains(key), "{key} is in both");
        }
    }
}
