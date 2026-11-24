//! A second event subscription, never a second winit event loop. A synchronous
//! job can dispatch this queue without re-entering its window or model.
use crate::native::Input;
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
    time::{Duration, Instant},
};
use x11rb::{
    connection::Connection as _,
    protocol::{
        xinput::{ConnectionExt, EventMask, XIEventMask},
        Event,
    },
    rust_connection::RustConnection,
};
#[path = "kinetic.rs"]
pub mod kinetic;
#[path = "wayland_pointer.rs"]
mod wayland;
pub use wayland::compositor_has_seat;

thread_local! { static WINDOWS: RefCell<HashMap<usize, Weak<RefCell<Subscription>>>> = RefCell::default(); }

pub struct Pointer {
    handle: usize,
    source: Rc<RefCell<Subscription>>,
}
struct Subscription {
    source: Source,
    consumed: Vec<(String, f64, f64, u32)>,
    last_progress: Option<Instant>,
}
enum Source {
    Wayland(wayland::Seat),
    X11(RustConnection),
}
impl Pointer {
    pub fn attach(handle: usize, display: usize, platform: &str) -> Option<Self> {
        let source = match platform {
            "wayland" => Source::Wayland(wayland::Seat::new(handle, display)?),
            "xlib" | "xcb" => {
                let (conn, _) = x11rb::connect(None).ok()?;
                conn.xinput_xi_query_version(2, 0).ok()?.reply().ok()?;
                let mask =
                    XIEventMask::BUTTON_PRESS | XIEventMask::BUTTON_RELEASE | XIEventMask::MOTION;
                conn.xinput_xi_select_events(
                    handle as u32,
                    &[EventMask {
                        deviceid: 1,
                        mask: vec![mask],
                    }],
                )
                .ok()?
                .check()
                .ok()?;
                Source::X11(conn)
            }
            _ => return None,
        };
        let source = Rc::new(RefCell::new(Subscription {
            source,
            consumed: Vec::new(),
            last_progress: None,
        }));
        WINDOWS.with_borrow_mut(|map| map.insert(handle, Rc::downgrade(&source)));
        Some(Self { handle, source })
    }
    // Winit will receive its own copy of the button events after the
    // synchronous command returns. Keep those clicks out of the model.
    pub fn consumed(&self, kind: &str, pos: (f64, f64), button: u32) -> bool {
        let mut state = self.source.borrow_mut();
        if state
            .last_progress
            .is_some_and(|t| t.elapsed() > Duration::from_secs(1))
        {
            state.consumed.clear();
        }
        if let Some(i) = state.consumed.iter().position(|(k, x, y, b)| {
            k == kind && *b == button && (pos.0 - x).abs() < 1. && (pos.1 - y).abs() < 1.
        }) {
            state.consumed.remove(i);
            true
        } else {
            false
        }
    }
    pub fn momentum(
        &self,
        now: std::time::Instant,
    ) -> (Option<(f64, f64)>, Option<std::time::Instant>) {
        let mut source = self.source.borrow_mut();
        let Source::Wayland(seat) = &mut source.source else {
            return (None, None);
        };
        (seat.state.inertia.advance(now), seat.state.inertia.next())
    }
    // Discard the duplicate pointer stream while winit is dispatching normally.
    pub fn normal(&self) -> Vec<(&'static str, f64)> {
        let mut source = self.source.borrow_mut();
        source.source.poll(1., 0);
        if let Source::Wayland(seat) = &mut source.source {
            std::mem::take(&mut seat.state.pinch)
        } else {
            Vec::new()
        }
    }
}
impl Drop for Pointer {
    fn drop(&mut self) {
        WINDOWS.with_borrow_mut(|map| map.remove(&self.handle));
    }
}
impl Source {
    fn poll(&mut self, scale: f64, mods: u32) -> Vec<Input> {
        match self {
            Self::Wayland(seat) => {
                seat.poll();
                std::mem::take(&mut seat.state.pointer)
                    .into_iter()
                    .map(|(kind, x, y, button)| (kind.into(), x, y, button, mods, 0, String::new()))
                    .collect()
            }
            Self::X11(conn) => {
                let mut inputs = Vec::new();
                while let Ok(Some(event)) = conn.poll_for_event() {
                    let (kind, ev) = match event {
                        Event::XinputButtonPress(e) => ("press", e),
                        Event::XinputButtonRelease(e) => ("release", e),
                        Event::XinputMotion(e) => ("move", e),
                        _ => continue,
                    };
                    let button = if kind == "move" {
                        0
                    } else {
                        match ev.detail {
                            1 => 1,
                            2 => 4,
                            3 => 2,
                            _ => continue,
                        }
                    };
                    let divisor = 65536. * scale;
                    inputs.push((
                        kind.into(),
                        ev.event_x as f64 / divisor,
                        ev.event_y as f64 / divisor,
                        button,
                        mods,
                        0,
                        String::new(),
                    ));
                }
                inputs
            }
        }
    }
}
pub fn progress(handle: usize, scale: f64, mods: u32) -> Vec<Input> {
    WINDOWS
        .with_borrow(|map| map.get(&handle).and_then(Weak::upgrade))
        .map_or_else(Vec::new, |src| {
            let mut state = src.borrow_mut();
            let inputs = state.source.poll(scale, mods);
            state.consumed.extend(
                inputs
                    .iter()
                    .filter(|i| i.0 != "move")
                    .map(|i| (i.0.clone(), i.1, i.2, i.3)),
            );
            state.last_progress = Some(Instant::now());
            inputs
        })
}
