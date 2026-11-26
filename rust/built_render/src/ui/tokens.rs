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
    // Selected: accent at 14 % (was 24 % plus an outline; design pass 2026-11-23).
    ACTIVE = Color32::from_rgba_premultiplied(15, 23, 36, 36);
    ACCENT_LINE = Color32::from_rgba_premultiplied(58, 91, 140, 140);
    // Lit: an armed tool or a bar toggle that is on: accent at 28 % under an
    // ACCENT_LINE edge (UX pass 1, M02: ACTIVE alone read as idle on the rail).
    LIT = Color32::from_rgba_premultiplied(30, 47, 72, 72);
    HOVER = Color32::from_rgba_premultiplied(18, 18, 18, 18);
    // The rest fill of buttons and chips (a neutral wash, no outline).
    CONTROL = Color32::from_rgba_premultiplied(18, 18, 18, 18);
    // A visible track on glass: progress bars, switches.
    TRACK = Color32::from_rgba_premultiplied(28, 28, 28, 28);
    // The raised segment of a segmented control.
    THUMB = Color32::from_rgba_premultiplied(41, 41, 41, 41);
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
    // The Sheets desk around the paper: a graphite well, never slate or blue
    // (council C8, 2026-11-23).
    DESK = WELL;
    GLASS_BAR = SIDE;
    POPOVER = Color32::from_gray(33);
    SHEET_SCRIM = Color32::from_black_alpha(112);
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
    ON_PRIMARY = SHEET;
    ON_PRIMARY_HINT = ON_PRIMARY;
    PRIMARY_PRESS = Color32::from_rgb(0x55, 0x8C, 0xE6);
    PRIMARY_HOVER = Color32::from_rgb(0x7A, 0xB0, 0xFF);
    WARNING_PRESS = Color32::from_rgba_premultiplied(54, 40, 16, 56);
    WARNING_HOVER = Color32::from_rgba_premultiplied(44, 33, 13, 46);
    IMAGE_BACK = Color32::from_rgba_premultiplied(0, 0, 0, 120);
    BRIGHT = Color32::WHITE;
    SWATCH_FALLBACK = Color32::GRAY;
}

/// The sheet shows the blurred scene with neutral colour and no panel rim.
pub(crate) const SHEET_BLUR: f32 = 1.75;
pub(crate) const SHEET_VIGNETTE: f32 = 0.42;
pub(crate) fn sheet_glass() -> crate::glass::Look {
    let tint = egui::Rgba::from(SHEET);
    crate::glass::Look {
        tint: [tint.r(), tint.g(), tint.b(), 0.55], peak: 0.085,
        saturation: 0.42, rim: [0.06, 0.015], glow: 0.0,
        vignette: SHEET_VIGNETTE, ..Surface::Panel.look()
    }
}
pub(crate) const SHEET_BACKDROP: crate::glass::Look = crate::glass::Look {
    tint: [0.0; 4], saturation: 0.30, peak: 0.65,
    vignette: SHEET_VIGNETTE, rim: [0.0; 2], rim_width: 1.0, glow: 0.0, inner_shadow: 0.0,
};

// ---- Design tokens (council 2026-11-22; one place, every component
// below draws from these). Text on graphite glass, three levels plus off.
// Geometry (Maxence 2026-11-22: "padding issues, rectangular buttons"):
// round controls, capsule bars, concentric radii. A bar is 28-pt controls
// in 4 pt of padding = 36 high, radius 18; menus pad 6 (radius 14, rows 8);
// the column pads 10 (radius 18, rows 8). Everything sits 12 from the
// window's edges and 8 from its neighbours.
// Rounded squares, not circles (Maxence 2026-11-22: "buttons can remain
// rounded squares … reduce the radii, it's a bit too much").
// Radii follow height (design pass 2026-11-23): 24-pt controls and rows 6,
// 28-pt buttons, tool buttons and tiles 8, marks inside a control 4.
pub const R_CONTROL: u8 = 8; // 28-pt buttons, tool buttons, tiles
pub const R_ROW: u8 = 6; // rows and every 24-pt control: fields, chips, tracks
pub const R_INSET: u8 = 4; // a thumb, swatch or mark inside a control
// Heights: every inline control is CONTROL_H so a row of them shares one
// baseline; sheet buttons are BUTTON_H; tree, list and menu rows ROW_H.
pub const CONTROL_H: f32 = 24.0;
pub const BUTTON_H: f32 = 28.0;
pub const ROW_H: f32 = 24.0;
// A label-and-value row (inspector properties): denser than a control row.
pub const PROP_H: f32 = 20.0;
// Focus: a 1-pt ring 1 pt outside the control, never a fill.
pub const FOCUS_RING_W: f32 = 1.0;
pub const FOCUS_GAP: f32 = 1.0;
// The selected row's leading marker.
pub const MARKER_W: f32 = 2.0;
pub const BAR_H: f32 = 36.0;
pub const EDGE_GAP: f32 = 12.0;
// The frame: bars docked to the window's edges, side panes floating 8 in.
pub const MENU_H: f32 = if cfg!(any(target_os = "linux", windows)) { 26.0 } else { 0.0 };
pub const TOP_H: f32 = 40.0 + MENU_H;
pub const BOT_H: f32 = 36.0;
pub const GUT: f32 = 8.0;
// The dock has one width, the user's: dragged on its inner edge between
// DOCK_MIN_W and DOCK_MAX_SHARE of the window, the same under every column
// preset of the tree. DOCK_MAX_W only bounds a stored value.
pub const DOCK_W: f32 = 300.0;
pub const DOCK_MIN_W: f32 = 280.0;
pub const DOCK_MAX_SHARE: f32 = 0.45;
pub const DOCK_MAX_W: f32 = 1200.0;
/// The dock's width in a window `window` wide, from the width the user set.
pub fn dock_width(wanted: f32, window: f32) -> f32 {
    wanted.clamp(DOCK_MIN_W, (window * DOCK_MAX_SHARE).max(DOCK_MIN_W))
}
// Type scale (design pass 2026-11-23): four sizes. Meta 11 for labels,
// captions, counts and shortcuts; body 12 for values and controls; title 13
// semibold for a panel's subject; heading 16 semibold for a sheet. Weight
// and ink, not more sizes, make the rest of the hierarchy.
pub const TYPE_META: f32 = 11.0;
pub const TYPE_BODY: f32 = 12.0;
pub const TYPE_TITLE: f32 = 13.0;
pub const TYPE_HEADING: f32 = 16.0;
// Type roles (Inter 400 / 500 / 600 when bundled, else the system font).
pub fn body() -> egui::FontId {
    egui::FontId::proportional(TYPE_BODY)
}
pub fn meta() -> egui::FontId {
    egui::FontId::proportional(TYPE_META)
}
pub fn strong(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("medium".into()))
}
/// A section label: meta size, medium weight, sentence case, DIM ink.
pub fn caption_font() -> egui::FontId {
    strong(TYPE_META)
}
pub fn title() -> egui::FontId {
    egui::FontId::new(TYPE_TITLE, egui::FontFamily::Name("semibold".into()))
}
pub fn title_font() -> egui::FontId {
    title()
}

pub fn document_rgb([r,g,b]: [u8; 3]) -> Color32 { Color32::from_rgb(r,g,b) }

pub fn parse_hex(s: &str) -> Option<Color32> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(document_rgb([(v >> 16) as u8, (v >> 8) as u8, v as u8]))
}

/// Glass surfaces, lowest to highest (design pass 2026-11-23: each level
/// has its own tint, opacity and shadow; higher is lighter and casts more):
/// Bar for docked bars (opaque, flat), Panel for data (tree, inspector),
/// Pill for compact floating chrome (palette, capsule), Raised for menus,
/// popovers and toasts.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Surface {
    Panel,
    Raised,
    Pill,
    Bar,
}
impl Surface {
    pub const ALL: [Surface; 4] = [Surface::Bar, Surface::Panel, Surface::Pill, Surface::Raised];
    pub fn name(self) -> &'static str {
        match self {
            Surface::Bar => "Bar",
            Surface::Panel => "Panel",
            Surface::Pill => "Pill",
            Surface::Raised => "Raised",
        }
    }
    /// The graphite each level settles to, as an sRGB grey.
    pub fn grey(self) -> u8 {
        match self {
            Surface::Bar => 31,
            Surface::Panel => 33,
            Surface::Pill => 36,
            Surface::Raised => 44,
        }
    }
    pub(crate) fn look(self) -> crate::glass::Look {
        // (tint mix, saturation, peak, lit rim, quiet rim, top, inner shadow)
        let (mix, sat, peak, lit, quiet, top, inner) = match self {
            // Docked bars: nearly opaque, no rim or glow (their outer edges are the window's).
            Surface::Bar => (0.90, 0.30, 0.060, 0.05, 0.03, 0.0, 0.0),
            Surface::Panel => (0.80, 0.30, 0.075, 0.18, 0.045, 0.020, 0.12),
            Surface::Pill => (0.70, 0.40, 0.085, 0.22, 0.05, 0.024, 0.10),
            Surface::Raised => (0.90, 0.16, 0.065, 0.28, 0.07, 0.036, 0.14),
        };
        let g = Color32::from_gray(self.grey());
        let lin = egui::Rgba::from(g);
        crate::glass::Look {
            tint: [lin.r(), lin.g(), lin.b(), mix],
            saturation: sat,
            rim: [lit, quiet],
            rim_width: 1.25,
            glow: top,
            peak,
            inner_shadow: inner, vignette: 0.0,
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
            Surface::Bar => (0, 0, 0),
            Surface::Panel => (6, 20, 90),
            Surface::Pill => (4, 14, 100),
            Surface::Raised => (12, 32, 150),
        };
        Shadow {
            offset: [0, y],
            blur,
            spread: 0,
            color: Color32::from_black_alpha(alpha),
        }
    }
}

pub const SPACE: [f32; 5] = [SPACE_XS, SPACE_S, SPACE_M, SPACE_L, SPACE_XL];
/// Between a label and its control, or items that belong together.
pub const SPACE_XS: f32 = 4.;
/// Between items in a group (rows of controls, chips in a row).
pub const SPACE_S: f32 = 8.;
/// Between groups; a panel's inner padding.
pub const SPACE_M: f32 = 12.;
/// Between sections.
pub const SPACE_L: f32 = 16.;
/// Between a sheet's regions.
pub const SPACE_XL: f32 = 24.;
pub const MOTION_QUICK: f32 = 0.12;
pub const MOTION_FADE: f32 = 0.20;
/// A sheet heading: 16 semibold (was 18 before the design pass).
pub fn heading() -> egui::FontId {
    egui::FontId::new(TYPE_HEADING, egui::FontFamily::Name("semibold".into()))
}

/// Invisible input backdrop for a compact modal popover.
pub const CLEAR: Color32 = Color32::TRANSPARENT;

#[cfg(test)]
mod sheet_contrast {
    use super::*;

    fn luminance(ink: Color32) -> f32 {
        let c = egui::Rgba::from(ink);
        0.2126 * c.r() + 0.7152 * c.g() + 0.0722 * c.b()
    }

    #[test]
    fn sheet_text_meets_four_and_a_half_to_one() {
        let glass = sheet_glass();
        // White source, centre of the card, including the upper dither bound.
        let brightest = glass.peak * (1.0 - glass.tint[3]) + glass.tint[0] * glass.tint[3] + 0.5 / 255.0;
        assert!((luminance(DIM) + 0.05) / (brightest + 0.05) >= 4.5);
        for fill in [ACCENT, PRIMARY_HOVER, PRIMARY_PRESS] {
            assert!((luminance(fill) + 0.05) / (luminance(ON_PRIMARY) + 0.05) >= 4.5);
        }
    }
}

/// A colour role by name as unmultiplied RGBA, for the hosts' Qt pieces
/// that still paint (the Sheets page's desk): the palette stays here.
#[pyo3::pyfunction]
pub fn chrome_token(name: &str) -> Option<(u8, u8, u8, u8)> {
    SWATCHES.iter().find(|(n, _)| *n == name).map(|(_, c)| {
        let [r, g, b, a] = c.to_srgba_unmultiplied();
        (r, g, b, a)
    })
}

#[cfg(test)]
mod lit_tests {
    use super::*;
    /// A premultiplied fill over an opaque grey, per channel.
    fn over(fill: Color32, grey: u8) -> [f32; 3] {
        let a = fill.a() as f32 / 255.0;
        [fill.r(), fill.g(), fill.b()].map(|c| c as f32 + grey as f32 * (1.0 - a))
    }
    #[test]
    fn an_armed_tool_reads_as_accent_on_the_rail() {
        // M02: the armed tool must stand out from the rail's graphite far more
        // than the selection wash does, and lean blue (the accent), not grey.
        let grey = Surface::Panel.grey();
        let shift = |fill| over(fill, grey)[2] - grey as f32;
        assert!(shift(LIT) >= 2.0 * shift(ACTIVE), "{} vs {}", shift(LIT), shift(ACTIVE));
        let [r, _, b] = over(LIT, grey);
        assert!(b - r > 30.0);
        assert!(LIT.a() < 255 / 2, "a control is never filled with full accent");
    }
}

#[cfg(test)]
mod chrome_token_tests {
    #[test]
    fn the_desk_is_graphite() {
        let (r, g, b, a) = super::chrome_token("DESK").unwrap();
        assert_eq!((r, g, b, a), (19, 19, 19, 255));
        assert!(super::chrome_token("NOPE").is_none());
    }
}
