//! Platform clipboard formats used by the shell. Keep the document MIME
//! interoperable with older Built instances and text with other applications.
use std::sync::Mutex;
/// The model type, by its name here and on the platform. Callers asking by
/// the Qt-era name (what the Qt window wrote on macOS) get the same data.
const MIME: &str = "application/x-ingetrazo-clipboard";
const QT_ERA: &str = "com.trolltech.anymime.application--x-ingetrazo-clipboard";
const TEXT: &str = "public.utf8-plain-text";
type Items = Vec<(String, Vec<u8>)>;

// Scripted launches ask for a private board to avoid touching the desktop.
static PRIVATE: Mutex<(Items, isize)> = Mutex::new((Vec::new(), 0));
fn private() -> bool {
    std::env::var_os("BUILT_PASTEBOARD").is_some_and(|s| !s.is_empty())
}
pub fn write(items: Items) -> bool {
    if private() {
        let mut board = PRIVATE.lock().unwrap();
        board.0 = items;
        board.1 = board.1.wrapping_add(1);
        true
    } else {
        platform::write(items)
    }
}
pub fn read(kind: &str) -> Option<Vec<u8>> {
    if private() {
        PRIVATE
            .lock()
            .unwrap()
            .0
            .iter()
            .find(|(k, _)| k == kind)
            .map(|(_, v)| v.clone())
    } else {
        platform::read(kind)
    }
}
pub fn types() -> (Vec<String>, isize) {
    if private() {
        let board = PRIVATE.lock().unwrap();
        (board.0.iter().map(|(k, _)| k.clone()).collect(), board.1)
    } else {
        platform::types()
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows::{
        core::w,
        Win32::{
            Foundation::{GlobalFree, HANDLE, HGLOBAL},
            System::{DataExchange::*, Memory::*},
            UI::Input::KeyboardAndMouse::GetActiveWindow,
        },
    };

    fn format(kind: &str) -> Option<u32> {
        match kind {
            TEXT => Some(13), // CF_UNICODETEXT
            MIME | QT_ERA => {
                Some(unsafe { RegisterClipboardFormatW(w!("application/x-ingetrazo-clipboard")) })
                    .filter(|f| *f != 0)
            }
            _ => None,
        }
    }
    struct Board;
    impl Board {
        fn open() -> Option<Self> {
            unsafe {
                OpenClipboard(GetActiveWindow()).ok()?;
            }
            Some(Self)
        }
    }
    impl Drop for Board {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseClipboard();
            }
        }
    }
    struct Block(HGLOBAL);
    impl Drop for Block {
        fn drop(&mut self) {
            unsafe {
                let _ = GlobalFree(self.0);
            }
        }
    }
    fn allocate(bytes: &[u8]) -> Option<Block> {
        let block = Block(unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)).ok()? });
        unsafe {
            let p = GlobalLock(block.0).cast::<u8>();
            if p.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
            let _ = GlobalUnlock(block.0);
        }
        Some(block)
    }
    pub fn write(items: Items) -> bool {
        let Some(prepared) = items
            .into_iter()
            .map(|(kind, bytes)| {
                let fmt = format(&kind)?;
                let data = if kind == TEXT {
                    String::from_utf8_lossy(&bytes)
                        .encode_utf16()
                        .chain([0])
                        .flat_map(u16::to_le_bytes)
                        .collect()
                } else {
                    bytes
                };
                Some((fmt, allocate(&data)?))
            })
            .collect::<Option<Vec<_>>>()
        else {
            return false;
        };
        let Some(_board) = Board::open() else {
            return false;
        };
        unsafe {
            if EmptyClipboard().is_err() {
                return false;
            }
            for (fmt, block) in prepared {
                if SetClipboardData(fmt, HANDLE(block.0 .0)).is_err() {
                    return false;
                }
                std::mem::forget(block); // Windows owns a published block.
            }
        }
        true
    }
    pub fn read(kind: &str) -> Option<Vec<u8>> {
        let fmt = format(kind)?;
        let _board = Board::open()?;
        unsafe {
            let block = HGLOBAL(GetClipboardData(fmt).ok()?.0);
            let len = GlobalSize(block);
            let p = GlobalLock(block).cast::<u8>();
            if p.is_null() {
                return None;
            }
            let bytes = std::slice::from_raw_parts(p, len);
            let data = if kind == TEXT {
                let words: Vec<_> = bytes
                    .chunks_exact(2)
                    .map(|v| u16::from_le_bytes([v[0], v[1]]))
                    .take_while(|c| *c != 0)
                    .collect();
                String::from_utf16_lossy(&words).into_bytes()
            } else {
                bytes.to_vec()
            };
            let _ = GlobalUnlock(block);
            Some(data)
        }
    }
    pub fn types() -> (Vec<String>, isize) {
        let types = [MIME, TEXT]
            .into_iter()
            .filter(|k| {
                format(k).is_some_and(|fmt| unsafe { IsClipboardFormatAvailable(fmt).is_ok() })
            })
            .map(str::to_owned)
            .collect();
        (types, unsafe { GetClipboardSequenceNumber() } as isize)
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    use gtk::{
        gdk, glib, glib::translate::ToGlibPtr, prelude::*, Clipboard, TargetEntry, TargetFlags,
    };
    use std::{
        cell::Cell,
        rc::Rc,
        sync::{mpsc, OnceLock},
    };
    type Task = Box<dyn FnOnce(&Clipboard, isize) + Send>;
    // GDK services both X11 and Wayland selections. Its owning thread must
    // keep processing requests even when all winit windows are asleep.
    fn worker() -> Option<&'static Mutex<glib::Sender<Task>>> {
        static SENDER: OnceLock<Option<Mutex<glib::Sender<Task>>>> = OnceLock::new();
        SENDER
            .get_or_init(|| {
                let (started, ready) = mpsc::sync_channel(1);
                std::thread::Builder::new()
                    .name("clipboard".to_owned())
                    .spawn(move || {
                        #[cfg(target_os = "linux")]
                        if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_some()
                            && crate::linux_pointer::compositor_has_seat() == Some(false) {
                            return;
                        }
                        if gtk::init().is_err() {
                            return;
                        }
                        // A headless compositor can advertise no input seat. GDK's
                        // Wayland clipboard requires one for keyboard selection.
                        if gdk::Display::default().and_then(|d| d.default_seat()).is_none() {
                            return;
                        }
                        let board = Clipboard::get(&gdk::SELECTION_CLIPBOARD);
                        let changes = Rc::new(Cell::new(1isize));
                        let serial = changes.clone();
                        board.connect_local("owner-change", false, move |_| {
                            serial.set(serial.get().wrapping_add(1));
                            None
                        });
                        let (tx, rx) = glib::MainContext::channel::<Task>(glib::Priority::DEFAULT);
                        rx.attach(None, move |task| {
                            task(&board, changes.get());
                            glib::ControlFlow::Continue
                        });
                        if started.send(tx).is_ok() {
                            gtk::main();
                        }
                    })
                    .ok()?;
                ready.recv().ok().map(Mutex::new)
            })
            .as_ref()
    }
    fn request<T: Send + 'static>(
        work: impl FnOnce(&Clipboard, isize) -> T + Send + 'static,
    ) -> Option<T> {
        let (send, recv) = mpsc::sync_channel(1);
        worker()?
            .lock()
            .ok()?
            .send(Box::new(move |b, n| {
                let _ = send.send(work(b, n));
            }))
            .ok()?;
        recv.recv().ok()
    }
    pub fn on_gtk_thread<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        request(move |_, _| work())
    }
    pub fn write(items: Items) -> bool {
        request(move |board, _| {
            let mut targets = Vec::new();
            for (i, (kind, _)) in items.iter().enumerate() {
                let formats: &[&str] = match kind.as_str() {
                    MIME | QT_ERA => &[MIME],
                    TEXT => &["UTF8_STRING", "text/plain;charset=utf-8", "text/plain"],
                    _ => return false,
                };
                for name in formats {
                    targets.push(TargetEntry::new(name, TargetFlags::empty(), i as u32));
                }
            }
            let ok = board.set_with_data(&targets, move |_, sel, i| {
                sel.set(&sel.target(), 8, &items[i as usize].1)
            });
            if ok {
                // Offer all targets to a clipboard manager for persistence.
                unsafe {
                    gtk::ffi::gtk_clipboard_set_can_store(
                        board.to_glib_none().0,
                        std::ptr::null_mut(),
                        0,
                    );
                }
                board.store();
            }
            ok
        })
        .unwrap_or(false)
    }
    pub fn read(kind: &str) -> Option<Vec<u8>> {
        let text = match kind {
            TEXT => true,
            MIME | QT_ERA => false,
            _ => return None,
        };
        request(move |board, _| {
            if text {
                return board.wait_for_text().map(|s| s.as_bytes().to_vec());
            }
            let data = board.wait_for_contents(&gdk::Atom::intern(MIME))?;
            (data.length() >= 0).then(|| data.data())
        })
        .flatten()
    }
    pub fn types() -> (Vec<String>, isize) {
        request(|board, n| {
            let mut types = Vec::new();
            for atom in board.wait_for_targets().unwrap_or_default() {
                let name = atom.name();
                let key = match name.as_str() {
                    MIME => MIME,
                    "UTF8_STRING" | "text/plain;charset=utf-8" | "text/plain" => TEXT,
                    _ => continue,
                };
                if !types.iter().any(|t| t == key) {
                    types.push(key.to_owned());
                }
            }
            (types, n)
        })
        .unwrap_or_default()
    }
}

#[cfg(not(windows))]
pub use platform::on_gtk_thread;
