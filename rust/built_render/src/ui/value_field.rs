//! The value field on the geometry (Maxence 2026-11-22) and its readback
//! (review council B1, 2026-11-23).
//!
//! Python passes facts each frame: the tool's caption, its live reading or
//! what is typed, where that sits on the geometry, the tool's start point and
//! the revision of what the value acts on (the document; the camera's lens
//! for Zoom). The decisions are here:
//!
//! - **Commit.** Typing stopped and that revision moved: Enter
//!   applied the value. (Esc or Backspace to empty stop typing without a new
//!   revision.)
//! - **Readback.** The committed value stays on the result: the segment from
//!   the old start to the new one when the tool chains on (Line), else where
//!   the typed value stood.
//! - **The next action ends it:** a pointer move or press on the model, wheel zoom, typing again,
//!   another tool, or any other change to the document (undo included).
use egui::{pos2, Pos2, Rect};

/// The live field: caption, value, typing, anchor.
#[derive(Clone, Debug, PartialEq)]
pub struct Live {
    pub caption: String,
    pub value: String,
    pub typing: bool,
    pub at: Pos2,
    pub start: Option<Pos2>,
}

/// A committed value, read back on the result.
#[derive(Clone, Debug, PartialEq)]
pub struct Readback {
    pub caption: String,
    pub value: String,
    pub at: Pos2,
    revision: u64,
}

#[derive(Default)]
pub struct ValueField {
    pub live: Option<Live>,
    pub readback: Option<Readback>,
    revision: Option<u64>,
}

/// How far above the result the pill sits (points).
const LIFT: f32 = 22.0;

impl ValueField {
    /// An idle retype stays at the result as subsequent digits arrive.
    pub(super) fn retype_anchor(&self, revision: u64) -> Option<Pos2> {
        if self.revision != Some(revision) {
            return None;
        }
        self.readback.as_ref().map(|r| r.at)
            .or_else(|| self.live.as_ref().filter(|l| l.typing).map(|l| l.at))
    }

    /// One frame's facts. `caption` empty and not typing: no live field.
    pub fn set(&mut self, caption: String, value: String, typing: bool, at: Pos2,
               start: Option<Pos2>, revision: u64) {
        let moved = self.revision.is_some_and(|r| r != revision);
        if let Some(prev) = self.live.as_ref().filter(|p| p.typing && !p.value.is_empty()) {
            if !typing && moved {
                let at = match (prev.start, start) {
                    // The tool chains on from the new end: the result is the
                    // segment between the two starts.
                    (Some(a), Some(b)) if a.distance(b) > 4.0 => pos2((a.x + b.x) / 2.0, (a.y + b.y) / 2.0 - LIFT),
                    _ => prev.at,
                };
                self.readback = Some(Readback { caption: prev.caption.clone(), value: prev.value.clone(), at, revision });
            }
        }
        if typing || self.readback.as_ref().is_some_and(|r| r.revision != revision) {
            self.readback = None;
        }
        self.revision = Some(revision);
        self.live = (!caption.is_empty() || typing).then(|| Live { caption, value, typing, at, start });
    }

    /// The next action (a press on the model, another tool): the readback goes.
    pub fn end_readback(&mut self) {
        self.readback = None;
    }

    /// Where the pills go inside `free` (points), `size` each: the readback
    /// first; the live one is dropped when it is not being typed and would
    /// cover the readback.
    pub fn place(&self, free: Rect, size: egui::Vec2) -> (Option<Rect>, Option<Rect>) {
        let put = |at: Pos2| {
            let min = pos2((at.x - size.x / 2.0).clamp(free.min.x, (free.max.x - size.x).max(free.min.x)),
                           (at.y - size.y / 2.0).clamp(free.min.y, (free.max.y - size.y).max(free.min.y)));
            Rect::from_min_size(min, size)
        };
        let back = self.readback.as_ref().map(|r| put(r.at));
        let live = self.live.as_ref()
            .filter(|l| l.typing || !l.value.is_empty())
            .map(|l| put(l.at))
            .filter(|r| back.is_none_or(|b| self.live.as_ref().is_some_and(|l| l.typing) || !b.expand(2.0).intersects(*r)));
        (back, live)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    fn typed(f: &mut ValueField, v: &str, rev: u64, start: Option<Pos2>) {
        f.set("Length".into(), v.into(), true, pos2(400., 300.), start, rev);
    }

    #[test]
    fn retyping_without_an_anchor_reuses_the_committed_result() {
        let mut f = ValueField::default();
        let at = pos2(350., 240.);
        f.set("Distance".into(), "2 m".into(), true, at, None, 7);
        f.set(String::new(), String::new(), false, Pos2::ZERO, None, 8);
        assert_eq!(f.retype_anchor(8), Some(at));
        for text in ["3 m", "30 m"] {
            f.set("Distance".into(), text.into(), true, f.retype_anchor(8).unwrap(), None, 8);
        }
        f.set(String::new(), String::new(), false, Pos2::ZERO, None, 9);
        let result = f.readback.as_ref().unwrap();
        assert_eq!((&*result.value, result.at), ("30 m", at));
        assert_eq!(f.retype_anchor(10), None);
    }

    #[test]
    fn enter_keeps_the_committed_value_on_the_result() {
        let mut f = ValueField::default();
        let a = pos2(300., 650.);
        f.set("Length".into(), "1.10 m".into(), false, pos2(360., 640.), Some(a), 7);
        typed(&mut f, "2.35 m", 7, Some(a));
        // Enter: the line chains on from the new end; the live reading is 0.
        let b = pos2(500., 700.);
        f.set("Length".into(), "0.00 m".into(), false, b, Some(b), 8);
        let r = f.readback.clone().expect("readback after Enter");
        assert_eq!(r.value, "2.35 m");
        assert_eq!(r.at, pos2(400., 675. - LIFT));
        // Moving the pointer keeps it (no action yet).
        f.set("Length".into(), "0.40 m".into(), false, pos2(520., 690.), Some(b), 8);
        assert!(f.readback.is_some());
        // The next segment (a new revision) ends it.
        f.set("Length".into(), "0.00 m".into(), false, b, Some(b), 9);
        assert!(f.readback.is_none());
    }

    #[test]
    fn escape_and_typing_again_leave_no_readback() {
        let mut f = ValueField::default();
        typed(&mut f, "2", 3, None);
        f.set(String::new(), String::new(), false, pos2(0., 0.), None, 3); // Esc: same revision
        assert!(f.readback.is_none());
        typed(&mut f, "2", 3, None);
        f.set(String::new(), String::new(), false, pos2(0., 0.), None, 4); // Enter, tool done
        assert_eq!(f.readback.as_ref().map(|r| r.at), Some(pos2(400., 300.)));
        typed(&mut f, "3", 4, None);
        assert!(f.readback.is_none());
    }

    #[test]
    fn a_stale_live_reading_does_not_cover_the_readback() {
        let mut f = ValueField::default();
        typed(&mut f, "2.35 m", 1, None);
        f.set("Length".into(), "0.00 m".into(), false, pos2(410., 305.), None, 2);
        let free = Rect::from_min_max(pos2(0., 0.), pos2(1280., 800.));
        let (back, live) = f.place(free, vec2(170., 30.));
        assert!(back.is_some() && live.is_none());
        f.set("Length".into(), "1.00 m".into(), false, pos2(700., 500.), None, 2);
        let (back, live) = f.place(free, vec2(170., 30.));
        assert!(back.is_some() && live.is_some());
        f.end_readback();
        assert_eq!(f.place(free, vec2(170., 30.)).0, None);
    }
}
