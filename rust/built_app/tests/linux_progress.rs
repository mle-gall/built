//! Verify the second input subscription on a real X server while the main
//! event loop is paused. The smoke image supplies its own Xvfb display.
#![cfg(target_os = "linux")]

mod native {
    pub type Input = (String, f64, f64, u32, u32, u32, String);
}
#[path = "../src/linux_pointer.rs"]
mod linux_pointer;

#[test]
fn progress_receives_press_and_release_without_reentering_winit() {
    if std::env::var_os("BUILT_XVFB_TEST").is_none() {
        return;
    }
    use x11rb::{
        connection::Connection,
        protocol::{xproto::*, xtest::ConnectionExt as _},
    };
    let (conn, screen) = x11rb::connect(None).unwrap();
    let root = conn.setup().roots[screen].root;
    let window = conn.generate_id().unwrap();
    conn.create_window(
        0,
        window,
        root,
        0,
        0,
        100,
        100,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::default(),
    )
    .unwrap()
    .check()
    .unwrap();
    conn.map_window(window).unwrap().check().unwrap();
    let handle = window as usize;
    let pointer = linux_pointer::Pointer::attach(handle, 0, "xlib").unwrap();
    conn.warp_pointer(0u32, window, 0, 0, 0, 0, 30, 40)
        .unwrap()
        .check()
        .unwrap();
    for kind in [BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT] {
        conn.xtest_fake_input(kind, 1, 0, root, 0, 0, 0)
            .unwrap()
            .check()
            .unwrap();
    }
    let events = linux_pointer::progress(handle, 2., 0);
    let buttons: Vec<_> = events
        .iter()
        .filter(|e| e.3 == 1)
        .map(|e| (e.0.as_str(), e.1, e.2))
        .collect();
    assert_eq!(buttons, [("press", 15., 20.), ("release", 15., 20.)]);
    assert!(pointer.consumed("press", (15., 20.), 1));
    assert!(pointer.consumed("release", (15., 20.), 1));
    assert!(!pointer.consumed("release", (15., 20.), 1));
    assert!(linux_pointer::progress(handle, 2., 0).is_empty());
}
