//! Input while the shell's command handler is waiting for a modal answer.
use crate::native::Input;
use std::cell::RefCell;
use windows::Win32::{
    Foundation::HWND,
    UI::{
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT},
        WindowsAndMessaging::*,
    },
};

thread_local! {
    static SCRIPT_KEYS: RefCell<Vec<(usize, Input)>> = const { RefCell::new(Vec::new()) };
}

pub fn post_key(view: usize, code: u16, chars: &str, command: bool) -> bool {
    let key = match code {
        36 => 0x01000004,
        53 => 0x01000000,
        _ => chars
            .chars()
            .next()
            .map_or(0, |c| c.to_ascii_uppercase() as u32),
    };
    SCRIPT_KEYS.with_borrow_mut(|keys| {
        keys.push((
            view,
            (
                "key".into(),
                0.,
                0.,
                0,
                if command { 0x04000000 } else { 0 },
                key,
                chars.into(),
            ),
        ))
    });
    true
}

pub fn pump(view: usize, timeout_ms: f64) -> Vec<Input> {
    let mut out = Vec::new();
    SCRIPT_KEYS.with_borrow_mut(|keys| {
        keys.retain(|(target, key)| {
            if *target == view {
                out.push(key.clone());
                false
            } else {
                true
            }
        })
    });
    let hwnd = HWND(view as _);
    if hwnd.0.is_null() {
        return out;
    }
    unsafe {
        if out.is_empty() {
            MsgWaitForMultipleObjectsEx(
                None,
                timeout_ms.clamp(0., 16.) as u32,
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            );
        }
        let scale = (GetDpiForWindow(hwnd) as f64 / 96.).max(1.);
        let mut msg = MSG::default();
        for _ in 0..64 {
            if !PeekMessageW(&mut msg, hwnd, 0, 0, PM_REMOVE).as_bool() {
                break;
            }
            let mut mods = 0;
            for (key, mask) in [
                (VK_SHIFT, 0x02000000),
                (VK_CONTROL, 0x04000000),
                (VK_MENU, 0x08000000),
            ] {
                if GetKeyState(key.0 as i32) < 0 {
                    mods |= mask;
                }
            }
            let mouse = match msg.message {
                WM_LBUTTONDOWN => Some(("press", 1)),
                WM_LBUTTONUP => Some(("release", 1)),
                WM_RBUTTONDOWN => Some(("press", 2)),
                WM_RBUTTONUP => Some(("release", 2)),
                WM_MBUTTONDOWN => Some(("press", 4)),
                WM_MBUTTONUP => Some(("release", 4)),
                WM_MOUSEMOVE => Some(("move", 0)),
                _ => None,
            };
            if let Some((kind, button)) = mouse {
                let x = msg.lParam.0 as i16 as f64 / scale;
                let y = (msg.lParam.0 >> 16) as i16 as f64 / scale;
                out.push((kind.into(), x, y, button, mods, 0, String::new()));
                continue;
            }
            match msg.message {
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    let vk = msg.wParam.0 as u32;
                    let key = match vk {
                        13 => 0x01000004,
                        27 => 0x01000000,
                        9 => 0x01000001,
                        37..=40 => 0x01000012 + vk - 37,
                        _ => vk,
                    };
                    let text = char::from_u32(vk)
                        .filter(char::is_ascii_alphanumeric)
                        .map_or_else(String::new, |c| c.to_string());
                    out.push(("key".into(), 0., 0., 0, mods, key, text));
                }
                WM_KEYUP | WM_SYSKEYUP | WM_MOUSEWHEEL | WM_MOUSEHWHEEL => (),
                _ => {
                    DispatchMessageW(&msg);
                }
            }
        }
    }
    out
}
