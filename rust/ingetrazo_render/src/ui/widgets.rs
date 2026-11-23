//! What the dock and the forms share beyond the kit: the kit's segmented
//! switch and tabs, and a mode caption with its way back. (The Library's
//! dock browser that lived in ui/library.rs is the large sheet now,
//! ui/large_sheet.rs.)

use super::*;

pub use super::kit::{segmented, underline_tabs};

/// Under Project, its mode (Problems) with a way back. Used by
/// `Ui::right_column`.
pub fn caption_mode(ui: &mut egui::Ui, mode: &str) -> bool {
    ui.add_space(8.0);
    caption_plain(ui, mode);
    let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
    paint_state(ui, r, state_of(&resp, false, true), R_ROW);
    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "×", body(), if resp.hovered() { INK } else { FAINT });
    resp.on_hover_text("Back to the tree (Esc)").clicked()
}
