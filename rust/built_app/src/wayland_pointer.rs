//! Listen on a private queue of the window's existing wl_display. Pointer
//! gestures are seat-scoped; the begin event identifies the target wl_surface.
use crate::linux_pointer::kinetic::Kinetic;
use std::time::Instant;
use wayland_client::{
    backend::Backend,
    delegate_noop,
    protocol::{wl_pointer, wl_registry, wl_seat},
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
};
use wayland_protocols::wp::pointer_gestures::zv1::client::{
    zwp_pointer_gesture_pinch_v1 as pinch, zwp_pointer_gestures_v1 as gestures,
};

pub struct Seat {
    queue: EventQueue<State>,
    pub state: State,
}
#[derive(Default)]
pub struct State {
    handle: usize,
    pub inertia: Kinetic,
    axis: (f64, f64),
    axis_time: u32,
    axis_stop: bool,
    finger: bool,
    manager: Option<gestures::ZwpPointerGesturesV1>,
    seats: Vec<(u32, wl_seat::WlSeat)>,
    pointers: Vec<(
        wl_seat::WlSeat,
        wl_pointer::WlPointer,
        Option<pinch::ZwpPointerGesturePinchV1>,
    )>,
    focus: Option<wl_pointer::WlPointer>,
    pos: (f64, f64),
    gesture: Option<(pinch::ZwpPointerGesturePinchV1, f64)>,
    pub pointer: Vec<(&'static str, f64, f64, u32)>,
    pub pinch: Vec<(&'static str, f64)>,
}
impl Seat {
    pub fn new(handle: usize, display: usize) -> Option<Self> {
        // The owner drops this subscription before dropping its winit window.
        let backend = unsafe { Backend::from_foreign_display(display as *mut _) };
        let conn = Connection::from_backend(backend);
        let mut queue = conn.new_event_queue();
        let mut state = State {
            handle,
            ..Default::default()
        };
        conn.display().get_registry(&queue.handle(), ());
        queue.roundtrip(&mut state).ok()?;
        queue.roundtrip(&mut state).ok()?;
        conn.flush().ok()?;
        Some(Self { queue, state })
    }
    pub fn poll(&mut self) {
        if let Some(read) = self.queue.prepare_read() {
            // libwayland's socket is nonblocking. Unread winit events stay on
            // winit's queue, including when the shell is inside a progress callback.
            let _ = read.read();
        }
        let _ = self.queue.dispatch_pending(&mut self.state);
        let _ = self.queue.flush();
    }
}
impl Drop for Seat {
    fn drop(&mut self) {
        for (_, pointer, gesture) in self.state.pointers.drain(..) {
            if let Some(gesture) = gesture {
                gesture.destroy();
            }
            if pointer.version() >= 3 {
                pointer.release();
            }
        }
        for (_, seat) in self.state.seats.drain(..) {
            if seat.version() >= 5 {
                seat.release();
            }
        }
        if let Some(manager) = self.state.manager.take() {
            if manager.version() >= 2 {
                manager.release();
            }
        }
        let _ = self.queue.flush();
    }
}
impl State {
    fn bind_gestures(&mut self, q: &QueueHandle<Self>) {
        if let Some(manager) = &self.manager {
            for (_, pointer, pinch) in &mut self.pointers {
                if pinch.is_none() {
                    *pinch = Some(manager.get_pinch_gesture(pointer, q, ()));
                }
            }
        }
    }
}
impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        s: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        q: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_seat" => {
                    let seat = registry.bind(name, version.min(7), q, ());
                    s.seats.push((name, seat));
                }
                "zwp_pointer_gestures_v1" => {
                    s.manager = Some(registry.bind(name, version.min(3), q, ()));
                    s.bind_gestures(q);
                }
                _ => {}
            }
        }
    }
}
delegate_noop!(State: ignore gestures::ZwpPointerGesturesV1);
impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        s: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        q: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            let prev = s.pointers.iter().position(|(owner, _, _)| owner == seat);
            match (caps.contains(wl_seat::Capability::Pointer), prev) {
                (true, None) => {
                    s.pointers
                        .push((seat.clone(), seat.get_pointer(q, ()), None));
                    s.bind_gestures(q);
                }
                (false, Some(i)) => {
                    let (_, p, g) = s.pointers.remove(i);
                    if let Some(g) = g {
                        g.destroy();
                    }
                    if p.version() >= 3 {
                        p.release();
                    }
                }
                _ => {}
            }
        }
    }
}
impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        s: &mut Self,
        p: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wl_pointer::Event::*;
        match event {
            Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } if surface.id().as_ptr() as usize == s.handle => {
                s.focus = Some(p.clone());
                s.pos = (surface_x, surface_y);
            }
            Leave { .. } if s.focus.as_ref() == Some(p) => {
                s.focus = None;
                s.inertia = Kinetic::default();
            }
            AxisSource { axis_source, .. } if s.focus.as_ref() == Some(p) => {
                s.finger = axis_source == WEnum::Value(wl_pointer::AxisSource::Finger);
                if !s.finger {
                    s.inertia = Kinetic::default();
                }
            }
            Axis {
                axis: WEnum::Value(axis),
                time,
                value,
            } if s.focus.as_ref() == Some(p) => {
                s.axis_time = time;
                if axis == wl_pointer::Axis::VerticalScroll {
                    s.axis.1 -= value;
                } else {
                    s.axis.0 -= value;
                }
            }
            AxisStop { time, .. } if s.focus.as_ref() == Some(p) => {
                s.axis_time = time;
                s.axis_stop = true;
            }
            Frame if s.focus.as_ref() == Some(p) => {
                if s.finger {
                    s.inertia
                        .frame(s.axis_time, s.axis, s.axis_stop, Instant::now());
                }
                s.axis = (0., 0.);
                s.axis_stop = false;
            }
            Motion {
                surface_x,
                surface_y,
                ..
            } if s.focus.as_ref() == Some(p) => {
                s.pos = (surface_x, surface_y);
                s.pointer.push(("move", surface_x, surface_y, 0));
            }
            Button {
                state: WEnum::Value(state),
                button,
                ..
            } if s.focus.as_ref() == Some(p) => {
                s.inertia = Kinetic::default();
                let bit = match button {
                    0x110 => 1,
                    0x111 => 2,
                    0x112 => 4,
                    _ => return,
                };
                let kind = if state == wl_pointer::ButtonState::Pressed {
                    "press"
                } else {
                    "release"
                };
                s.pointer.push((kind, s.pos.0, s.pos.1, bit));
            }
            _ => {}
        }
    }
}
impl Dispatch<pinch::ZwpPointerGesturePinchV1, ()> for State {
    fn event(
        s: &mut Self,
        g: &pinch::ZwpPointerGesturePinchV1,
        event: pinch::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            pinch::Event::Begin { surface, .. } if surface.id().as_ptr() as usize == s.handle => {
                s.inertia = Kinetic::default();
                s.gesture = Some((g.clone(), 1.));
                s.pinch.push(("begin", 0.));
            }
            pinch::Event::Update { scale, .. } if scale.is_finite() && scale > 0. => {
                if let Some((owner, prev)) = &mut s.gesture {
                    if owner == g {
                        s.pinch.push(("zoom", scale / *prev - 1.));
                        *prev = scale;
                    }
                }
            }
            pinch::Event::End { .. } if s.gesture.as_ref().is_some_and(|(owner, _)| owner == g) => {
                s.gesture = None;
                s.pinch.push(("end", 0.));
            }
            _ => {}
        }
    }
}

/// GTK 3 initializes the keyboard map in gtk_init_check itself. Avoid that
/// call on a compositor with no wl_seat; it cannot provide a clipboard.
pub fn compositor_has_seat() -> Option<bool> {
    use wayland_client::globals::{registry_queue_init, GlobalListContents};
    struct Probe;
    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Probe {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }
    let conn = Connection::connect_to_env().ok()?;
    let (globals, _) = registry_queue_init::<Probe>(&conn).ok()?;
    Some(
        globals
            .contents()
            .with_list(|list| list.iter().any(|g| g.interface == "wl_seat")),
    )
}
