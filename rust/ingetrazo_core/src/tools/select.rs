//! Select: click, the click cycle, double and triple clicks, the box.
//!
//! The rules (from `tools/select.py`, issue numbers kept with them):
//! - Modifiers: Shift toggles, Ctrl adds, Shift+Ctrl removes, nothing
//!   replaces; the box reads the same modifiers.
//! - A plain click again within [`CYCLE_PX`] of the last, the pointer not
//!   having left that spot, the selection still what that click made and
//!   the same things under the cursor, takes the next candidate.
//! - A click on nothing clears the selection; inside an edit context with
//!   nothing selected, a click outside the context's box leaves one level.
//! - Double click: a group opens; a section plane toggles the active cut;
//!   a label or a dimension edits its text; a face takes its edges, an edge
//!   its faces. Triple click: everything connected.
//! - Hover marks what a click would take, never a face (#832).
//! - Delete erases the selection or, with none, the hover; Backspace the
//!   selection only.
//!
//! Until picking is native the host answers what lies under a pixel
//! ([`ToolHost::pick`], [`ToolHost::candidates`]) and applies the
//! selection; the machine owns the gesture and every decision above.

use super::gesture::Gesture;
use super::{Grow, Pick, PickKind, Pointer, Preview, Reply, SelectMode, Tool, ToolEvent, ToolHost, ToolInfo};

/// A repeated click this close to the last takes the next candidate.
pub const CYCLE_PX: f64 = 4.0;

static INFO: ToolInfo = ToolInfo {
    kind: "select",
    name: "Select",
    shortcut: "",
    description: "Pick edges, faces and objects. Shift+click adds or takes away, \
                  Ctrl+click adds, Shift+Ctrl+click takes away; the same with a box.",
    uses_snap: false,
    box_select: true,
    vcb_label: None,
    magnetic_axis_deg: None,
    screen_axis_px: None,
    axis_source: false,
    axis_source_empty_only: false,
};

#[derive(Clone, Debug, PartialEq)]
struct Cycle {
    at: [f64; 2],
    candidates: Vec<Pick>,
    index: usize,
    selected: u64,
}

#[derive(Default)]
pub struct SelectTool {
    gesture: Gesture,
    /// The box being dragged, as dragged (press corner first).
    dragged: Option<[f64; 4]>,
    cycle: Option<Cycle>,
}

fn far(a: [f64; 2], b: [f64; 2]) -> bool { (a[0] - b[0]).hypot(a[1] - b[1]) > CYCLE_PX }

impl SelectTool {
    fn note_pointer(&mut self, px: [f64; 2]) {
        if self.cycle.as_ref().is_some_and(|c| far(c.at, px)) {
            self.cycle = None;
        }
    }

    /// What a click takes, with the cycle. Returns the pick and, for a
    /// plain click, the cycle to remember once the selection is made.
    fn cycled_pick(&mut self, px: [f64; 2], mode: SelectMode, host: &mut dyn ToolHost)
        -> (Option<Pick>, Option<Cycle>) {
        let cycle = self.cycle.take();
        if mode != SelectMode::Replace {
            return (host.pick(px), None);
        }
        let candidates = host.candidates(px);
        if candidates.is_empty() {
            return (None, None);
        }
        let mut index = 0;
        if let Some(c) = cycle {
            if !far(c.at, px) && c.candidates == candidates && host.selection_signature() == c.selected {
                index = (c.index + 1) % candidates.len();
            }
        }
        let pick = candidates[index];
        if candidates.len() > 1 {
            host.cycle_status(index, candidates.len(), pick.id);
        }
        (Some(pick), Some(Cycle { at: px, candidates, index, selected: 0 }))
    }

    fn click(&mut self, p: &Pointer, host: &mut dyn ToolHost) -> Reply {
        let mode = SelectMode::from_mods(p.mods);
        // An extension's item (a render light) is drawn over the model:
        // a click on it outranks the geometry behind (#205).
        if let Some(item) = host.extension_at(p.px) {
            self.cycle = None;
            host.clear_selection();
            host.set_extension_pick(Some(item));
            return Reply::redraw();
        }
        if host.has_extension_pick() {
            host.set_extension_pick(None);
        }
        let (pick, cycle) = self.cycled_pick(p.px, mode, host);
        match pick {
            None => {
                // The edit box, not the occupied faces, is the context's
                // boundary; beyond it, step out one level.
                if host.editing() && mode == SelectMode::Replace && host.selection_empty()
                    && !host.edit_box_contains(p.px) {
                    host.close_placement();
                    return Reply::redraw();
                }
                if mode == SelectMode::Replace {
                    host.clear_selection();
                }
            }
            Some(pick) => {
                host.select(&[pick.id], mode, Grow::Natural);
                self.cycle = cycle.map(|c| Cycle { selected: host.selection_signature(), ..c });
            }
        }
        Reply::redraw()
    }

    fn double(&mut self, p: &Pointer, host: &mut dyn ToolHost) -> Reply {
        self.cycle = None;
        let Some(pick) = host.pick(p.px) else { return self.click(p, host) };
        match pick.kind {
            PickKind::Group => host.open_placement(pick.id),
            PickKind::SectionPlane => host.host_verb("toggle_section", pick.id),
            PickKind::TextLabel => host.host_verb("edit_text", pick.id),
            PickKind::Dimension => host.host_verb("edit_dimension", pick.id),
            PickKind::Face | PickKind::Edge =>
                host.select(&[pick.id], SelectMode::from_mods(p.mods), Grow::Bounded),
            PickKind::Other => return self.click(p, host),
        }
        Reply::redraw()
    }

    fn triple(&mut self, p: &Pointer, host: &mut dyn ToolHost) -> Reply {
        self.cycle = None;
        match host.pick(p.px) {
            Some(pick) if matches!(pick.kind, PickKind::Face | PickKind::Edge) => {
                host.select(&[pick.id], SelectMode::from_mods(p.mods), Grow::Connected);
                Reply::redraw()
            }
            _ => self.click(p, host),
        }
    }

    fn hover(&mut self, p: &Pointer, host: &mut dyn ToolHost) -> Reply {
        self.note_pointer(p.px);
        let pick = host.pick(p.px);
        host.set_hover(pick.filter(|k| k.kind != PickKind::Face).map(|k| k.id));
        Reply::redraw()
    }
}

impl Tool for SelectTool {
    fn info(&self) -> &'static ToolInfo { &INFO }

    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Press(p) => {
                if let Some(ToolEvent::TripleClick(t)) = self.gesture.press(p) {
                    return self.triple(&t, host);
                }
                Reply { consumed: true, ..Reply::default() }
            }
            ToolEvent::Drag(p) => {
                if !self.gesture.pressed() {
                    return Reply::ignored();
                }
                self.dragged = self.gesture.drag(p);
                Reply::redraw()
            }
            ToolEvent::Release(p) => {
                self.dragged = None;
                match self.gesture.release(p) {
                    Some(ToolEvent::Click(c)) => self.click(&c, host),
                    Some(ToolEvent::Box { rect, crossing, mods }) => {
                        self.box_select(rect, crossing, SelectMode::from_mods(mods), host)
                    }
                    _ => Reply::ignored(),
                }
            }
            ToolEvent::Click(p) => self.click(p, host),
            ToolEvent::DoubleClick(p) => {
                let p = match self.gesture.double(p) { ToolEvent::DoubleClick(p) => p, _ => unreachable!() };
                self.double(&p, host)
            }
            ToolEvent::TripleClick(p) => self.triple(p, host),
            ToolEvent::Box { rect, crossing, mods } =>
                self.box_select(*rect, *crossing, SelectMode::from_mods(*mods), host),
            ToolEvent::Hover(p) => self.hover(p, host),
            ToolEvent::Key { key, .. } => match key.as_str() {
                "Delete" => { host.erase(false); Reply::committed() }
                "Backspace" => { host.erase(true); Reply::committed() }
                _ => Reply::ignored(),
            },
            ToolEvent::Cancel => {
                let busy = self.dragged.is_some();
                self.gesture.reset();
                self.dragged = None;
                Reply { consumed: busy, redraw: busy, committed: false }
            }
            ToolEvent::Deactivate => {
                self.gesture.reset();
                self.dragged = None;
                self.cycle = None;
                host.set_hover(None);
                Reply::redraw()
            }
            _ => Reply::ignored(),
        }
    }

    fn preview(&self) -> Preview {
        Preview {
            box_rect: self.dragged.map(|r| (r, r[2] < r[0])),
            ..Preview::default()
        }
    }

    fn busy(&self) -> bool { self.dragged.is_some() }

    fn state(&self) -> serde_json::Value {
        serde_json::json!({
            "pressed": self.gesture.pressed(),
            "cycle": self.cycle.as_ref().map(|c| serde_json::json!({
                "at": c.at, "index": c.index, "count": c.candidates.len()})),
        })
    }
}

impl SelectTool {
    fn box_select(&mut self, rect: [f64; 4], crossing: bool, mode: SelectMode, host: &mut dyn ToolHost) -> Reply {
        self.cycle = None;
        if mode == SelectMode::Replace && host.has_extension_pick() {
            host.set_extension_pick(None);
        }
        host.box_select(rect, crossing, mode);
        Reply::redraw()
    }
}
