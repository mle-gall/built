//! Native gesture dispatch and raw touchscreen contacts, in logical pixels.
use super::{phase_name, python, App, TouchPhase, WindowId};

/// A slide's delta in whole logical pixels, the rest carried to the next
/// event. The viewport reads integer deltas (Qt's `pixelDelta` and
/// `angleDelta`), and a trackpad's slow slide and inertia tail come as
/// fractions of a pixel. Rounding each event alone dropped every delta under
/// a quarter pixel and zoomed the 0.25-0.5 px ones backwards: their pixel
/// delta rounded to nothing, which reads as a mouse wheel, and a mouse wheel
/// skips the Natural Scrolling flip. Carried, the whole pixels handed over
/// add up to the finger's travel, in its direction.
pub(super) fn carry(carry: &mut (f64, f64), delta: (f64, f64)) -> (f64, f64) {
    let total = (carry.0 + delta.0, carry.1 + delta.1);
    let whole = (total.0.round(), total.1.round());
    *carry = (total.0 - whole.0, total.1 - whole.1);
    whole
}

impl App {
    pub(super) fn slide(
        &mut self,
        id: WindowId,
        pos: (f64, f64),
        delta: (f64, f64),
        phase: &str,
        touch: bool,
        inverted: bool,
    ) {
        let mods = self.mods();
        if let Some(w) = self.win_mut(id) {
            if phase == "start" {
                w.slide_carry = (0., 0.);
            }
            let (dx, dy) = carry(&mut w.slide_carry, delta);
            // Under a pixel so far: nothing to turn, pan or zoom yet. The
            // phases that open and close a gesture always go through.
            if dx == 0. && dy == 0. && matches!(phase, "move" | "momentum") {
                return;
            }
            let (x, y) = pos;
            w.host.call(
                "wheel",
                (
                    x,
                    y,
                    dx,
                    dy,
                    dx * 2.,
                    dy * 2.,
                    phase,
                    mods,
                    w.held,
                    inverted,
                    touch,
                ),
            );
        }
    }

    pub(super) fn touch(&mut self, id: WindowId, finger: u64, phase: TouchPhase, pos: (f64, f64)) {
        let mods = self.mods();
        let Some(w) = self.win_mut(id) else {
            return;
        };
        // Some backends only deliver raw contacts. Use their midpoint for
        // sliding and relative separation for pinching through the same lock.
        if let Some((primary, a, selecting)) = w.contact {
            if phase == TouchPhase::Started && finger != primary && w.second_contact.is_none() {
                w.second_contact = Some((finger, pos));
                w.contact = Some((primary, a, false));
                if selecting {
                    self.pointer(id, "release", a.0, a.1, 1, mods);
                }
                self.slide(id, a, (0., 0.), "start", false, true);
                return;
            }
            if let Some((secondary, b)) = w.second_contact {
                if finger != primary && finger != secondary {
                    return;
                }
                if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                    w.second_contact = None;
                    w.contact = Some(if finger == primary {
                        (secondary, b, false)
                    } else {
                        (primary, a, false)
                    });
                    w.host.call("gesture", ("end", 0., pos.0, pos.1));
                    return;
                }
                let (next_a, next_b) = if finger == primary {
                    (pos, b)
                } else {
                    (a, pos)
                };
                w.contact = Some((primary, next_a, false));
                w.second_contact = Some((secondary, next_b));
                let old_span = (b.0 - a.0).hypot(b.1 - a.1);
                let new_span = (next_b.0 - next_a.0).hypot(next_b.1 - next_a.1);
                let center = ((next_a.0 + next_b.0) * 0.5, (next_a.1 + next_b.1) * 0.5);
                if old_span > 0. && new_span > 0. {
                    w.host.call(
                        "gesture",
                        ("zoom", new_span / old_span - 1., center.0, center.1),
                    );
                }
                self.slide(
                    id,
                    center,
                    (center.0 - (a.0 + b.0) * 0.5, center.1 - (a.1 + b.1) * 0.5),
                    "move",
                    false,
                    true,
                );
                return;
            }
        }
        if phase == TouchPhase::Started && w.contact.is_none() {
            let select = python::touch_select();
            w.contact = Some((finger, pos, select));
            if select {
                w.host.call("menu", ("tool:select",));
                self.pointer(id, "press", pos.0, pos.1, 1, mods);
            } else {
                self.slide(id, pos, (0., 0.), "start", true, true);
            }
            return;
        }
        let Some((active, last, select)) = w.contact else {
            return;
        };
        if finger != active {
            return;
        }
        let end = matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled);
        w.contact = (!end).then_some((finger, pos, select));
        if select {
            self.pointer(
                id,
                if end { "release" } else { "move" },
                pos.0,
                pos.1,
                if end { 1 } else { 0 },
                mods,
            );
        } else {
            self.slide(
                id,
                last,
                (pos.0 - last.0, pos.1 - last.1),
                phase_name(phase),
                true,
                true,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::carry;

    /// A slow slide and an inertia tail, in fractions of a pixel: what is
    /// handed over adds up to the travel, never goes the other way, and is
    /// never more than half a pixel behind.
    #[test]
    fn fractional_slides_add_up_in_their_direction() {
        for step in [0.05, 0.2, 0.3, 0.4, 0.6, 1.4, 3.7] {
            let mut c = (0., 0.);
            let (mut sent, mut travel) = (0., 0.);
            for _ in 0..240 {
                let (dx, dy) = carry(&mut c, (step * 0.5, -step));
                assert!(dx >= 0. && dy <= 0., "{step}: handed over against the slide");
                assert!(dx.fract() == 0. && dy.fract() == 0.);
                sent += dy;
                travel -= step;
                assert!((sent - travel).abs() <= 0.5 + 1e-9, "{step}: {sent} behind {travel}");
            }
        }
        let mut c = (0., 0.);
        let mut tail = 0.;
        let mut v = 5.;
        while v > 0.05 {
            v *= 0.94;
            tail += carry(&mut c, (0., v)).1;
        }
        assert_eq!(tail, 78.);
    }
}
