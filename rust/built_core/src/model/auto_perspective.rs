// SPDX-License-Identifier: GPL-3.0-or-later
//! Auto perspective: aligning the view to a standard direction (Top,
//! Front, a face) shows it in parallel projection; the first orbit away
//! from that view returns to the projection the user had before.
//!
//! The camera keeps a [`Stash`] while it shows such a view. A stash only
//! counts while the camera still looks the way it was aligned and is still
//! parallel: a scene, an undo or a typed pose that changes either leaves
//! the user's projection alone.
use std::f64::consts::{PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projection {
    Parallel,
    Perspective,
    TwoPoint,
}

impl Projection {
    pub fn name(self) -> &'static str {
        match self {
            Self::Parallel => "parallel",
            Self::Perspective => "perspective",
            Self::TwoPoint => "two_point",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "parallel" => Some(Self::Parallel),
            "perspective" => Some(Self::Perspective),
            "two_point" => Some(Self::TwoPoint),
            _ => None,
        }
    }
}

/// What an aligned view remembers: the projection to go back to and the
/// direction it was aligned to (radians).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stash {
    pub before: Projection,
    pub yaw: f64,
    pub pitch: f64,
}

const SAME: f64 = 1e-6;

fn same_angle(a: f64, b: f64) -> bool {
    let d = (a - b).rem_euclid(TAU);
    d.min(TAU - d) <= SAME
}

/// The stash, if the camera still shows the view it was made for.
pub fn held(stash: Option<Stash>, now: Projection, yaw: f64, pitch: f64) -> Option<Stash> {
    stash.filter(|s| {
        now == Projection::Parallel
            && (s.pitch - pitch).abs() <= SAME
            // Straight down or up, the yaw only turns the picture.
            && (same_angle(s.yaw, yaw) || (pitch.abs() - PI / 2.).abs() <= SAME)
    })
}

/// Aligning a camera that shows `now` at (`yaw0`, `pitch0`) to (`yaw`,
/// `pitch`): the projection to show and the stash to keep.
pub fn align(
    now: Projection,
    stash: Option<Stash>,
    auto: bool,
    (yaw0, pitch0): (f64, f64),
    (yaw, pitch): (f64, f64),
) -> (Projection, Option<Stash>) {
    if !auto {
        return (now, None);
    }
    match held(stash, now, yaw0, pitch0) {
        // From one aligned view to another: still back to the same one.
        Some(s) => (Projection::Parallel, Some(Stash { yaw, pitch, ..s })),
        // Already parallel by choice: nothing to go back to.
        None if now == Projection::Parallel => (now, None),
        None => (Projection::Parallel, Some(Stash { before: now, yaw, pitch })),
    }
}

/// Leaving an aligned view (an orbit, Isometric): the projection to restore,
/// if any. The stash goes either way.
pub fn leave(stash: Option<Stash>, now: Projection, yaw: f64, pitch: f64) -> Option<Projection> {
    held(stash, now, yaw, pitch).map(|s| s.before).filter(|p| *p != now)
}

#[cfg(test)]
mod tests {
    use super::Projection::*;
    use super::*;
    const FRONT: (f64, f64) = (-PI / 2., 0.);
    const TOP: (f64, f64) = (-PI / 2., PI / 2.);
    const FREE: (f64, f64) = (-PI / 4., PI / 6.);

    #[test]
    fn aligning_shows_parallel_and_an_orbit_returns_to_perspective() {
        let (shown, stash) = align(Perspective, None, true, FREE, FRONT);
        assert_eq!(shown, Parallel);
        assert_eq!(stash, Some(Stash { before: Perspective, yaw: FRONT.0, pitch: FRONT.1 }));
        assert_eq!(leave(stash, shown, FRONT.0, FRONT.1), Some(Perspective));
    }

    #[test]
    fn view_to_view_keeps_the_first_projection() {
        let (shown, stash) = align(TwoPoint, None, true, FREE, FRONT);
        let (shown, stash) = align(shown, stash, true, FRONT, TOP);
        assert_eq!((shown, stash.unwrap().before), (Parallel, TwoPoint));
        assert_eq!(leave(stash, shown, TOP.0, TOP.1), Some(TwoPoint));
    }

    #[test]
    fn already_parallel_stays_parallel_after_orbiting() {
        let (shown, stash) = align(Parallel, None, true, FREE, FRONT);
        assert_eq!((shown, stash), (Parallel, None));
        assert_eq!(leave(stash, shown, FRONT.0, FRONT.1), None);
    }

    #[test]
    fn the_setting_off_leaves_the_projection_unchanged() {
        assert_eq!(align(Perspective, None, false, FREE, FRONT), (Perspective, None));
        assert_eq!(align(TwoPoint, None, false, FREE, TOP), (TwoPoint, None));
    }

    #[test]
    fn a_stash_lapses_once_the_view_or_projection_changed_elsewhere() {
        let (_, stash) = align(Perspective, None, true, FREE, FRONT);
        // A scene put the camera back in perspective.
        assert_eq!(leave(stash, Perspective, FRONT.0, FRONT.1), None);
        // A parallel scene from another direction.
        assert_eq!(leave(stash, Parallel, FREE.0, FREE.1), None);
        // Aligning from that scene remembers parallel, the user's choice.
        assert_eq!(align(Parallel, stash, true, FREE, TOP), (Parallel, None));
    }

    #[test]
    fn top_and_bottom_hold_whatever_the_yaw() {
        let stash = Some(Stash { before: Perspective, yaw: TOP.0, pitch: TOP.1 });
        assert_eq!(leave(stash, Parallel, 1.0, TOP.1), Some(Perspective));
        assert_eq!(leave(stash, Parallel, FRONT.0 + TAU, 0.0), None);
        let front = Some(Stash { before: Perspective, yaw: FRONT.0, pitch: 0.0 });
        assert_eq!(leave(front, Parallel, FRONT.0 + TAU, 0.0), Some(Perspective));
    }
}
