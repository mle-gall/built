//! winit keys and modifiers in Qt's numbering, which the Python tools read.
//!
//! Qt on macOS calls ⌘ "Control" and the Control key "Meta"; the tools test
//! `ControlModifier` for ⌘ shortcuts (Cmd+click, Cmd+Z), so ⌘ maps to
//! `CONTROL` here and the Control key to `META`.

use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};

pub const SHIFT: u32 = 0x0200_0000;
pub const CONTROL: u32 = 0x0400_0000;
pub const ALT: u32 = 0x0800_0000;
pub const META: u32 = 0x1000_0000;

#[cfg(target_os = "macos")]
const SYSTEM_MODIFIERS: (u32, u32) = (CONTROL, META);
#[cfg(not(target_os = "macos"))]
const SYSTEM_MODIFIERS: (u32, u32) = (META, CONTROL);

/// Qt's modifier mask for winit's modifier state.
pub fn qt_modifiers(m: ModifiersState) -> u32 {
    let mut out = 0;
    if m.shift_key() {
        out |= SHIFT;
    }
    if m.super_key() {
        out |= SYSTEM_MODIFIERS.0;
    }
    if m.alt_key() {
        out |= ALT;
    }
    if m.control_key() {
        out |= SYSTEM_MODIFIERS.1;
    }
    out
}

/// Qt's `Qt::Key` for a named key, 0 for one Qt has no code for.
pub fn qt_named(key: NamedKey) -> u32 {
    use NamedKey::*;
    match key {
        Escape => 0x0100_0000,
        Tab => 0x0100_0001,
        Backspace => 0x0100_0003,
        Enter => 0x0100_0004,
        Insert => 0x0100_0006,
        Delete => 0x0100_0007,
        Pause => 0x0100_0008,
        PrintScreen => 0x0100_0009,
        Clear => 0x0100_000b,
        Home => 0x0100_0010,
        End => 0x0100_0011,
        ArrowLeft => 0x0100_0012,
        ArrowUp => 0x0100_0013,
        ArrowRight => 0x0100_0014,
        ArrowDown => 0x0100_0015,
        PageUp => 0x0100_0016,
        PageDown => 0x0100_0017,
        Shift => 0x0100_0020,
        // ⌘ is Qt's Key_Control on the Mac, Control its Key_Meta.
        Super | Meta => if cfg!(target_os = "macos") { 0x0100_0021 } else { 0x0100_0022 },
        Control => if cfg!(target_os = "macos") { 0x0100_0022 } else { 0x0100_0021 },
        Alt => 0x0100_0023,
        CapsLock => 0x0100_0024,
        NumLock => 0x0100_0025,
        ScrollLock => 0x0100_0026,
        F1 => 0x0100_0030,
        F2 => 0x0100_0031,
        F3 => 0x0100_0032,
        F4 => 0x0100_0033,
        F5 => 0x0100_0034,
        F6 => 0x0100_0035,
        F7 => 0x0100_0036,
        F8 => 0x0100_0037,
        F9 => 0x0100_0038,
        F10 => 0x0100_0039,
        F11 => 0x0100_003a,
        F12 => 0x0100_003b,
        ContextMenu => 0x0100_0055,
        Help => 0x0100_0058,
        Space => 0x20,
        _ => 0,
    }
}

/// Qt's key code for a printable character: letters as their capital
/// (Qt::Key_A is 'A'), everything else as its own code point.
pub fn qt_char(c: char) -> u32 {
    if c.is_alphabetic() {
        c.to_uppercase().next().unwrap_or(c) as u32
    } else {
        c as u32
    }
}

/// `(qt key, text)` for a winit logical key. `text` is what Qt's
/// `QKeyEvent::text()` would hold (the typed character, or "\r", "\t",
/// "\x1b", "\x08" for those keys), empty when a modifier other than Shift
/// is down, as on the Mac.
pub fn qt_key(key: &Key, text: Option<&str>, mods: u32) -> (u32, String) {
    let typed = text.unwrap_or("").to_string();
    let plain = mods & (CONTROL | META) == 0;
    match key {
        Key::Named(named) => {
            let code = qt_named(*named);
            let text = match named {
                NamedKey::Space => " ".into(),
                NamedKey::Enter => "\r".into(),
                NamedKey::Tab => "\t".into(),
                NamedKey::Escape => "\u{1b}".into(),
                NamedKey::Backspace => "\u{8}".into(),
                NamedKey::Delete => "\u{7f}".into(),
                _ => String::new(),
            };
            (code, if plain { text } else { String::new() })
        }
        Key::Character(s) => {
            let c = s.chars().next().unwrap_or('\0');
            let shown = if typed.is_empty() { s.to_string() } else { typed };
            (qt_char(c), if plain { shown } else { String::new() })
        }
        _ => (0, if plain { typed } else { String::new() }),
    }
}

/// Shift changes the glyph on the digit row, but the binding is still Shift+digit.
pub fn window_key(physical: PhysicalKey, logical: &Key, text: Option<&str>, mods: u32) -> (u32, String) {
    let (code, typed) = qt_key(logical, text, mods);
    if mods & SHIFT == 0 || !matches!(logical, Key::Character(_)) {
        return (code, typed);
    }
    use KeyCode::*;
    let digit = match physical {
        PhysicalKey::Code(Digit0) => '0',
        PhysicalKey::Code(Digit1) => '1',
        PhysicalKey::Code(Digit2) => '2',
        PhysicalKey::Code(Digit3) => '3',
        PhysicalKey::Code(Digit4) => '4',
        PhysicalKey::Code(Digit5) => '5',
        PhysicalKey::Code(Digit6) => '6',
        PhysicalKey::Code(Digit7) => '7',
        PhysicalKey::Code(Digit8) => '8',
        PhysicalKey::Code(Digit9) => '9',
        _ => return (code, typed),
    };
    (digit as u32, typed)
}

/// A key by name for scripted runs: "escape", "enter", "shift", "up", a
/// single character, or "f3".
pub fn qt_key_by_name(name: &str) -> Option<(u32, String)> {
    use NamedKey::*;
    let lower = name.to_ascii_lowercase();
    let named = match lower.as_str() {
        "escape" | "esc" => Some(Escape),
        "enter" | "return" => Some(Enter),
        "tab" => Some(Tab),
        "backspace" => Some(Backspace),
        "delete" | "del" => Some(Delete),
        "space" => Some(Space),
        "shift" => Some(Shift),
        "cmd" | "command" | "super" => Some(Super),
        "ctrl" | "control" => Some(Control),
        "alt" | "option" => Some(Alt),
        "left" => Some(ArrowLeft),
        "right" => Some(ArrowRight),
        "up" => Some(ArrowUp),
        "down" => Some(ArrowDown),
        "home" => Some(Home),
        "end" => Some(End),
        _ => None,
    };
    if let Some(k) = named {
        return Some(qt_key(&Key::Named(k), None, 0));
    }
    if let Some(n) = lower.strip_prefix('f').and_then(|d| d.parse::<u32>().ok()) {
        if (1..=12).contains(&n) {
            return Some((0x0100_0030 + n - 1, String::new()));
        }
    }
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some((qt_char(c), c.to_string())),
        _ => None,
    }
}

/// Modifier names ("shift+cmd") → Qt mask, for scripted runs.
pub fn qt_modifiers_by_name(names: &str) -> u32 {
    names
        .split('+')
        .map(|n| match n.trim().to_ascii_lowercase().as_str() {
            "shift" => SHIFT,
            "cmd" | "command" | "super" => SYSTEM_MODIFIERS.0,
            "alt" | "option" => ALT,
            "ctrl" | "control" => SYSTEM_MODIFIERS.1,
            _ => 0,
        })
        .fold(0, |a, b| a | b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::SmolStr;

    #[test]
    fn platform_modifiers_match_qt() {
        assert_eq!(qt_modifiers(ModifiersState::SUPER), if cfg!(target_os = "macos") { CONTROL } else { META });
        assert_eq!(qt_modifiers(ModifiersState::CONTROL), if cfg!(target_os = "macos") { META } else { CONTROL });
        assert_eq!(qt_modifiers(ModifiersState::SHIFT | ModifiersState::ALT), SHIFT | ALT);
    }

    #[test]
    fn letters_are_capitals_with_their_typed_text() {
        let (code, text) = qt_key(&Key::Character(SmolStr::new("l")), Some("l"), 0);
        assert_eq!((code, text.as_str()), ('L' as u32, "l"));
        let (code, text) = qt_key(&Key::Character(SmolStr::new("z")), None, CONTROL);
        assert_eq!((code, text.as_str()), ('Z' as u32, ""));
    }

    #[test]
    fn named_keys_carry_qt_codes_and_control_text() {
        assert_eq!(qt_key(&Key::Named(NamedKey::Escape), None, 0), (0x0100_0000, "\u{1b}".into()));
        assert_eq!(qt_key(&Key::Named(NamedKey::Enter), None, 0), (0x0100_0004, "\r".into()));
        assert_eq!(qt_key(&Key::Named(NamedKey::ArrowUp), None, 0).0, 0x0100_0013);
        assert_eq!(qt_key(&Key::Named(NamedKey::Super), None, 0).0, if cfg!(target_os = "macos") { 0x0100_0021 } else { 0x0100_0022 });
    }

    fn press(key: KeyCode, glyph: &str, mods: u32) -> (u32, String) {
        window_key(key.into(), &Key::Character(glyph.into()), Some(glyph), mods)
    }

    #[test]
    fn shifted_digit_shortcuts_keep_the_digit_and_type_the_glyph() {
        for (key, glyph, digit) in [(KeyCode::Digit5, "%", '5'), (KeyCode::Digit1, "!", '1'), (KeyCode::Digit0, ")", '0')] {
            assert_eq!(press(key, glyph, SHIFT), (digit as u32, glyph.into()));
        }
    }

    #[test]
    fn control_shift_digit_keeps_its_shortcut_without_inserting_text() {
        assert_eq!(press(KeyCode::Digit5, "%", CONTROL | SHIFT), ('5' as u32, String::new()));
    }

    #[test]
    fn other_shifted_keys_and_unshifted_digits_keep_the_layout_key() {
        assert_eq!(press(KeyCode::Slash, "?", SHIFT).0, '?' as u32);
        assert_eq!(press(KeyCode::KeyA, "A", SHIFT).0, 'A' as u32);
        assert_eq!(press(KeyCode::Digit5, "5", 0).0, '5' as u32);
    }

    #[test]
    fn scripted_names() {
        assert_eq!(qt_key_by_name("esc").unwrap().0, 0x0100_0000);
        assert_eq!(qt_key_by_name("p").unwrap(), ('P' as u32, "p".into()));
        assert_eq!(qt_key_by_name("F3").unwrap().0, 0x0100_0032);
        assert_eq!(qt_key_by_name("2").unwrap(), ('2' as u32, "2".into()));
        assert_eq!(qt_modifiers_by_name("shift+cmd"), SHIFT | CONTROL);
    }
}
