//! Press, move and release become clicks, boxes, double and triple clicks,
//! by one set of rules for every tool and both hosts.

use super::{Pointer, ToolEvent};

/// A release this close to its press (Manhattan, px) is a click.
pub const CLICK_SLOP_PX: f64 = 3.0;
/// A box shows only past this drag (px): a click that jitters is no box.
pub const BOX_DRAG_THRESHOLD_PX: f64 = 4.0;
/// A press within this time and distance after a double click is a triple
/// (Qt's double-click interval on the Mac, the shell's `DOUBLE_CLICK`).
pub const TRIPLE_MS: f64 = 500.0;
pub const TRIPLE_PX: f64 = 8.0;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gesture {
    press: Option<Pointer>,
    last: Option<[f64; 2]>,
    double: Option<(f64, [f64; 2])>,
}

fn manhattan(a: [f64; 2], b: [f64; 2]) -> f64 { (a[0] - b[0]).abs() + (a[1] - b[1]).abs() }

impl Gesture {
    /// The left button went down. A triple click is answered at once.
    pub fn press(&mut self, p: &Pointer) -> Option<ToolEvent> {
        if let Some((t, at)) = self.double.take() {
            if p.t_ms - t <= TRIPLE_MS && manhattan(p.px, at) < TRIPLE_PX {
                self.press = None;
                self.last = None;
                return Some(ToolEvent::TripleClick(p.clone()));
            }
        }
        self.press = Some(p.clone());
        self.last = Some(p.px);
        None
    }

    /// The host's double click (Qt makes it, the native shell too: same
    /// button, < 500 ms, < 5 px). Remembered for the triple.
    pub fn double(&mut self, p: &Pointer) -> ToolEvent {
        self.press = None;
        self.last = None;
        self.double = Some((p.t_ms, p.px));
        ToolEvent::DoubleClick(p.clone())
    }

    /// The pointer moved with the button down: the box to show, if any.
    pub fn drag(&mut self, p: &Pointer) -> Option<[f64; 4]> {
        let press = self.press.as_ref()?;
        self.last = Some(p.px);
        let (dx, dy) = (p.px[0] - press.px[0], p.px[1] - press.px[1]);
        if dx.hypot(dy) >= BOX_DRAG_THRESHOLD_PX {
            Some([press.px[0], press.px[1], p.px[0], p.px[1]])
        } else {
            None
        }
    }

    pub fn pressed(&self) -> bool { self.press.is_some() }

    /// The button came up: a click, or a box (normalised rectangle;
    /// crossing when dragged right to left).
    pub fn release(&mut self, p: &Pointer) -> Option<ToolEvent> {
        let press = self.press.take()?;
        self.last = None;
        if manhattan(p.px, press.px) < CLICK_SLOP_PX {
            return Some(ToolEvent::Click(Pointer { mods: p.mods, ..p.clone() }));
        }
        let rect = [press.px[0].min(p.px[0]), press.px[1].min(p.px[1]),
                    press.px[0].max(p.px[0]), press.px[1].max(p.px[1])];
        Some(ToolEvent::Box { rect, crossing: p.px[0] < press.px[0], mods: p.mods })
    }

    pub fn reset(&mut self) { *self = Gesture::default(); }

}
