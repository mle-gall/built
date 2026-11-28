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

/// What a click at `px` takes (`tools/select.py`'s cascade before R2.5
/// slice 2): a label's text block; the visible line under the cursor (a
/// loose edge, or an object of lines only, nearest first, the edge on a
/// tie); a group, unless an edge over it is in plain sight; the edge; then
/// [`pick_rest`]; last, the smallest object of lines whose box holds the
/// cursor. `lines` false: past the lines (a repeated click's candidates).
pub fn pick(host: &mut dyn ToolHost, px: [f64; 2], lines: bool) -> Option<Pick> {
    let first = |host: &mut dyn ToolHost, what: &str| host.query(what, px).first().map(|h| h.0);
    if let Some(label) = first(host, "label_block") {
        return Some(label);
    }
    let has_linework = host.has("linework");
    if lines && has_linework {
        if let Some((hit, _)) = linework(host, px).first() {
            return Some(*hit);
        }
    }
    let group = first(host, "group");
    // With the linework picks, those answered for the visible edges.
    let edge = if has_linework { None } else { first(host, "visible_edge") };
    if let (Some(_), Some(e)) = (group, edge) {
        if host.edge_clear(e.id, px) {
            return Some(e);
        }
    }
    group.or(edge).or_else(|| pick_rest(host, px)).or_else(|| first(host, "groups_around"))
}

/// The visible lines within reach, nearest first: the loose edge, then the
/// objects of lines only (a stable sort keeps the edge first on a tie).
pub fn linework(host: &mut dyn ToolHost, px: [f64; 2]) -> Vec<(Pick, f64)> {
    let mut found = host.query("linework_edge", px);
    found.extend(host.query("linework_groups", px));
    found.sort_by(|a, b| a.1.total_cmp(&b.1));
    found
}

/// Past the lines and the groups: a dimension, a label, a geo path, a
/// section plane (by its frame), a guide, the front face unless a picture
/// lies in front of it (#426), last a reference image (an underlay).
pub fn pick_rest(host: &mut dyn ToolHost, px: [f64; 2]) -> Option<Pick> {
    for what in ["dimension", "text_label", "geopath", "section_plane", "guide"] {
        if let Some(hit) = host.query(what, px).first() {
            return Some(hit.0);
        }
    }
    if let Some((face, _)) = host.query("face", px).first().copied() {
        return Some(host.image_in_front(face.id, px).unwrap_or(face));
    }
    host.query("image_plane", px).first().map(|h| h.0)
}

/// Everything a click may select, in the order repeated clicks go through
/// them, each once.
pub fn candidates(host: &mut dyn ToolHost, px: [f64; 2]) -> Vec<Pick> {
    let mut found: Vec<Pick> = Vec::new();
    let add = |p: Option<Pick>, found: &mut Vec<Pick>| {
        if let Some(p) = p {
            if !found.iter().any(|q| q.id == p.id) {
                found.push(p);
            }
        }
    };
    add(pick(host, px, true), &mut found);
    for (hit, _) in linework(host, px) {
        add(Some(hit), &mut found);
    }
    add(pick(host, px, false), &mut found);
    let group = host.query("group", px).first().map(|h| h.0);
    add(group, &mut found);
    add(pick_rest(host, px), &mut found);
    for (hit, _) in host.query("groups_around", px) {
        add(Some(hit), &mut found);
    }
    found
}

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
            return (pick(host, px, true), None);
        }
        let candidates = candidates(host, px);
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
        // A junction's marker (drawn for the selection's junctions) is over
        // the members it joins: a click on it selects the junction.
        if mode == SelectMode::Replace && host.call("marker_junction", serde_json::json!({"px": p.px, "open": false})) == serde_json::json!(true) {
            self.cycle = None;
            return Reply::redraw();
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
        // Double-clicking a junction's marker opens the junction to edit
        // inside it (docs/junctions.md §6.3).
        if host.call("marker_junction", serde_json::json!({"px": p.px, "open": true})) == serde_json::json!(true) {
            return Reply::redraw();
        }
        let Some(pick) = pick(host, p.px, true) else { return self.click(p, host) };
        match pick.kind {
            // A swept member's section face opens its profile in the
            // Sketch workspace; elsewhere a group opens for editing.
            PickKind::Group if host.call("sweep_cap_profile", serde_json::json!({"px": p.px})) == serde_json::json!(true) => {}
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
        match pick(host, p.px, true) {
            Some(pick) if matches!(pick.kind, PickKind::Face | PickKind::Edge) => {
                host.select(&[pick.id], SelectMode::from_mods(p.mods), Grow::Connected);
                Reply::redraw()
            }
            _ => self.click(p, host),
        }
    }

    fn hover(&mut self, p: &Pointer, host: &mut dyn ToolHost) -> Reply {
        self.note_pointer(p.px);
        let pick = pick(host, p.px, true);
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
