//! A protocol fixture for events that seatless Weston cannot generate.
use std::{
    os::unix::net::UnixStream,
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};
use wayland_protocols::wp::pointer_gestures::zv1::server::{
    zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1, zwp_pointer_gestures_v1,
};
use wayland_server::{
    protocol::{wl_compositor, wl_pointer, wl_seat, wl_surface},
    Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, New, Resource,
};

#[derive(Default)]
struct Compositor {
    surface: Option<wl_surface::WlSurface>,
}

macro_rules! global {
    ($resource:ty, $item:ident, $on_bind:block) => {
        impl GlobalDispatch<$resource, ()> for Compositor {
            fn bind(
                _: &mut Self,
                _: &DisplayHandle,
                _: &Client,
                new: New<$resource>,
                _: &(),
                init: &mut DataInit<'_, Self>,
            ) {
                let $item = init.init(new, ());
                $on_bind
            }
        }
    };
}
global!(wl_compositor::WlCompositor, _compositor, {});
global!(zwp_pointer_gestures_v1::ZwpPointerGesturesV1, _manager, {});
global!(wl_seat::WlSeat, seat, {
    seat.capabilities(wl_seat::Capability::Pointer);
});

macro_rules! requests {
    ($resource:ty, $state:ident, $request:ident, $init:ident, $body:block) => {
        impl Dispatch<$resource,()> for Compositor {
            fn request($state:&mut Self,_:&Client,_:&$resource,$request:<$resource as Resource>::Request,_:&(),_:&DisplayHandle,$init:&mut DataInit<'_,Self>) $body
        }
    };
}
requests!(wl_compositor::WlCompositor, s, request, init, {
    if let wl_compositor::Request::CreateSurface { id } = request {
        s.surface = Some(init.init(id, ()));
    }
});
requests!(wl_seat::WlSeat, _s, request, init, {
    if let wl_seat::Request::GetPointer { id } = request {
        init.init(id, ());
    }
});
requests!(wl_pointer::WlPointer, _s, _r, _i, {});
requests!(wl_surface::WlSurface, _s, _r, _i, {});
requests!(ZwpPointerGesturePinchV1, _s, _r, _i, {});
requests!(
    zwp_pointer_gestures_v1::ZwpPointerGesturesV1,
    s,
    request,
    init,
    {
        if let zwp_pointer_gestures_v1::Request::GetPinchGesture { id, pointer } = request {
            let gesture = init.init(id, ());
            let surface = s.surface.as_ref().unwrap();
            pointer.enter(1, surface, 30., 40.);
            pointer.button(2, 90, 0x110, wl_pointer::ButtonState::Pressed);
            pointer.button(3, 92, 0x110, wl_pointer::ButtonState::Released);
            gesture.begin(4, 100, surface, 2);
            gesture.update(110, 0., 0., 1.25, 0.);
            gesture.update(120, 0., 0., 1.5, 0.);
            gesture.end(5, 130, 0);
            for time in [140, 156] {
                pointer.axis_source(wl_pointer::AxisSource::Finger);
                pointer.axis(time, wl_pointer::Axis::VerticalScroll, 8.);
                pointer.frame();
            }
            pointer.axis_source(wl_pointer::AxisSource::Finger);
            pointer.axis_stop(160, wl_pointer::Axis::VerticalScroll);
            pointer.frame();
        }
    }
);

struct Window;
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol as client, Connection, Proxy, QueueHandle,
};
wayland_client::delegate_noop!(Window: ignore client::wl_compositor::WlCompositor);
wayland_client::delegate_noop!(Window: ignore client::wl_surface::WlSurface);
impl wayland_client::Dispatch<client::wl_registry::WlRegistry, GlobalListContents> for Window {
    fn event(
        _: &mut Self,
        _: &client::wl_registry::WlRegistry,
        _: client::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

#[test]
fn compositor_delivers_pointer_pinch_and_finger_inertia_to_the_window() {
    let (peer, socket) = UnixStream::pair().unwrap();
    let (stop, done) = mpsc::channel::<()>();
    let server = std::thread::spawn(move || {
        let mut display = Display::<Compositor>::new().unwrap();
        let mut handle = display.handle();
        handle.create_global::<Compositor, wl_compositor::WlCompositor, _>(4, ());
        handle.create_global::<Compositor, wl_seat::WlSeat, _>(7, ());
        handle.create_global::<Compositor, zwp_pointer_gestures_v1::ZwpPointerGesturesV1, _>(3, ());
        let _client = handle.insert_client(peer, Arc::new(())).unwrap();
        let mut state = Compositor::default();
        while done.try_recv() == Err(mpsc::TryRecvError::Empty) {
            display.dispatch_clients(&mut state).unwrap();
            display.flush_clients().unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }
    });
    let client = Connection::from_socket(socket).unwrap();
    let (globals, queue) = registry_queue_init::<Window>(&client).unwrap();
    let compositor: client::wl_compositor::WlCompositor =
        globals.bind(&queue.handle(), 1..=4, ()).unwrap();
    let surface = compositor.create_surface(&queue.handle(), ());
    client.roundtrip().unwrap();
    let handle = surface.id().as_ptr() as usize;
    let input = super::linux_pointer::Pointer::attach(
        handle,
        client.backend().display_ptr() as usize,
        "wayland",
    )
    .unwrap();
    client.roundtrip().unwrap();
    let clicks = super::linux_pointer::progress(handle, 2., 0);
    assert_eq!(
        clicks
            .iter()
            .map(|e| (e.0.as_str(), e.1, e.2, e.3))
            .collect::<Vec<_>>(),
        [("press", 30., 40., 1), ("release", 30., 40., 1)]
    );
    let pinch = input.normal();
    assert_eq!(
        pinch.iter().map(|(phase, _)| *phase).collect::<Vec<_>>(),
        ["begin", "zoom", "zoom", "end"]
    );
    assert!((pinch[1].1 - 0.25).abs() < 0.001 && (pinch[2].1 - 0.2).abs() < 0.001);
    let (_, deadline) = input.momentum(Instant::now());
    assert!(input.momentum(deadline.unwrap()).0.unwrap().1 < 0.);
    drop(input);
    stop.send(()).unwrap();
    server.join().unwrap();
}
