//! Shared graphite palette, typography, spacing and elevation.
use egui::{Color32, Shadow};
macro_rules! palette {
    ($($role:ident = $value:expr;)+) => {
        $(pub const $role: Color32 = $value;)+
        pub const SWATCHES: &[(&str, Color32)] = &[$((stringify!($role), $role)),+];
    };
}
palette! {
    INK = Color32::from_rgb(0xEE, 0xF1, 0xF4);
    INK_2 = Color32::from_rgb(0xC9, 0xCF, 0xD6);
    DIM = Color32::from_rgb(0xA2, 0xA9, 0xB1);
    FAINT = Color32::from_rgb(0x85, 0x8C, 0x94);
    OFF = Color32::from_rgb(0x5C, 0x63, 0x6A);
    ACCENT = Color32::from_rgb(0x6A, 0xA5, 0xFF);
    ACTIVE = Color32::from_rgba_premultiplied(25, 40, 61, 61);
    ACCENT_LINE = Color32::from_rgba_premultiplied(58, 91, 140, 140);
    HOVER = Color32::from_rgba_premultiplied(18, 18, 18, 18);
    PRESS = Color32::from_rgba_premultiplied(31, 31, 31, 31);
    EDGE = Color32::from_rgba_premultiplied(20, 20, 20, 20);
    FIELD = Color32::from_rgba_premultiplied(0, 0, 0, 71);
    WARN = Color32::from_rgb(0xF5, 0xB8, 0x4A);
    WARN_FILL = Color32::from_rgba_premultiplied(34, 26, 10, 36);
    PANEL = Color32::from_rgba_premultiplied(31, 31, 31, 245);
    SHEET = Color32::from_gray(26);
    SIDE = Color32::from_gray(31);
    CARD = Color32::from_gray(36);
    CARD_HOVER = Color32::from_gray(43);
    WELL = Color32::from_gray(19);
    GLASS_BAR = SIDE;
    POPOVER = PANEL;
    SCRIM = Color32::from_black_alpha(72);
    DIVIDER = EDGE;
    FOCUS_RING = ACCENT;
    OK = Color32::from_rgb(92,196,122);
    ERROR = Color32::from_rgb(235,99,99);
    INFO = INK_2;
    FOUND = OK;
    CHANGED = WARN;
    MISSING = Color32::from_rgb(240,98,90);
    PACKED = DIM;
    WORKING = INFO;
    WAITING = DIM;
    QUESTION = WARN;
    DONE = OK;
    FAILED = ERROR;
    CONTACT = Color32::from_black_alpha(115);
    POP_SHADOW = Color32::from_black_alpha(90);
    IMAGE_HOVER = Color32::from_black_alpha(170);
    HOVER_EDGE = Color32::from_rgba_premultiplied(46, 46, 46, 46);
    ON_PRIMARY_HINT = Color32::from_rgba_premultiplied(150, 150, 150, 150);
    PRIMARY_PRESS = Color32::from_rgb(0x55, 0x8C, 0xE6);
    PRIMARY_HOVER = Color32::from_rgb(0x7A, 0xB0, 0xFF);
    WARNING_PRESS = Color32::from_rgba_premultiplied(54, 40, 16, 56);
    WARNING_HOVER = Color32::from_rgba_premultiplied(44, 33, 13, 46);
    IMAGE_BACK = Color32::from_rgba_premultiplied(0, 0, 0, 120);
    BRIGHT = Color32::WHITE;
    SWATCH_FALLBACK = Color32::GRAY;
}

// ---- Design tokens (council 2026-11-22; one place, every component
// below draws from these). Text on graphite glass, three levels plus off.
// Geometry (Maxence 2026-11-22: "padding issues, rectangular buttons"):
// round controls, capsule bars, concentric radii. A bar is 28-pt controls
// in 4 pt of padding = 36 high, radius 18; menus pad 6 (radius 14, rows 8);
// the column pads 10 (radius 18, rows 8). Everything sits 12 from the
// window's edges and 8 from its neighbours.
// Rounded squares, not circles (Maxence 2026-11-22: "buttons can remain
// rounded squares … reduce the radii, it's a bit too much").
pub const R_CONTROL: u8 = 8; // 28-pt controls, tabs, chips
pub const R_ROW: u8 = 6; // menu and tree rows, fields
pub const BAR_H: f32 = 36.0;
pub const EDGE_GAP: f32 = 12.0;
// The frame: bars docked to the window's edges, side panes floating 8 in.
pub const TOP_H: f32 = 40.0;
pub const BOT_H: f32 = 36.0;
pub const GUT: f32 = 8.0;
pub const DOCK_W: f32 = 300.0;
pub const DOCK_MIN_W: f32 = 260.0;
pub const DOCK_MAX_W: f32 = 560.0;
// Type roles (Inter 400 / 500 / 600 when bundled, else the system font).
pub fn body() -> egui::FontId {
    egui::FontId::proportional(12.0)
}
pub fn meta() -> egui::FontId {
    egui::FontId::proportional(11.0)
}
pub fn strong(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("medium".into()))
}
pub fn title() -> egui::FontId {
    egui::FontId::new(13.0, egui::FontFamily::Name("semibold".into()))
}
pub fn title_font() -> egui::FontId {
    title()
}

pub fn parse_hex(s: &str) -> Option<Color32> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// Glass surfaces (council 2026-11-22): Panel for data (tree, inspector),
/// Raised for menus, Pill for compact chrome (palette, rows, bars).
#[derive(Clone, Copy)]
pub enum Surface {
    Panel,
    Raised,
    Pill,
    Bar,
}
impl Surface {
    pub(crate) fn look(self) -> crate::glass::Look {
        // (tint mix, saturation, peak, lit rim, quiet rim, top, inner shadow)
        let (mix, sat, peak, lit, quiet, top, inner) = match self {
            Surface::Panel => (0.72, 0.30, 0.080, 0.20, 0.045, 0.022, 0.12),
            Surface::Raised => (0.82, 0.16, 0.065, 0.23, 0.055, 0.030, 0.14),
            Surface::Pill => (0.61, 0.45, 0.085, 0.20, 0.045, 0.022, 0.10),
            // Docked bars: quieter rim, no glow (their outer edges are the window's).
            Surface::Bar => (0.70, 0.35, 0.080, 0.06, 0.03, 0.0, 0.0),
        };
        crate::glass::Look {
            tint: [0.01764, 0.01764, 0.01764, mix],
            saturation: sat,
            rim: [lit, quiet],
            rim_width: 1.25,
            glow: top,
            peak,
            inner_shadow: inner,
        }
    }
    pub fn radius(self) -> u8 {
        match self {
            Surface::Panel => 14,
            Surface::Raised => 12,
            Surface::Pill => 12,
            Surface::Bar => 0,
        }
    }
    pub fn shadow(self) -> Shadow {
        let (y, blur, alpha) = match self {
            Surface::Panel => (8, 24, 100),
            Surface::Raised => (12, 32, 128),
            Surface::Pill => (4, 14, 90),
            Surface::Bar => (0, 0, 0),
        };
        Shadow {
            offset: [0, y],
            blur,
            spread: 0,
            color: Color32::from_black_alpha(alpha),
        }
    }
}

pub const SPACE: [f32; 5] = [4., 8., 12., 16., 24.];
pub const MOTION_QUICK: f32 = 0.12;
pub const MOTION_FADE: f32 = 0.20;
pub fn heading() -> egui::FontId {
    egui::FontId::new(18., egui::FontFamily::Name("semibold".into()))
}
