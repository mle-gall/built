// SPDX-License-Identifier: GPL-3.0-or-later
//! The keystroke overlay for screen recordings (View ▸ Show Keystrokes,
//! Settings › Display › Recording; off by default): a pill at the bottom
//! centre of the model showing the keys and chords just pressed, a value
//! as it is typed into the value field, and the tool a key armed. It
//! fades a moment after the last key (`HOLD`).
use std::sync::atomic::{AtomicBool, Ordering};

/// The settings key; "1" shows the overlay.
pub const SETTING: &str = "view/show_keystrokes";
/// Seconds the pill stays after the last key, then it fades.
pub const HOLD: f64 = 1.2;
/// Seconds the fade takes (the kit's MOTION_FADE).
pub const FADE: f64 = 0.2;
/// The pill keeps the last few things pressed.
const KEEP: usize = 4;

static SHOWN: AtomicBool = AtomicBool::new(false);

pub fn shown() -> bool {
    SHOWN.load(Ordering::Relaxed)
}
pub fn set_shown(on: bool) {
    SHOWN.store(on, Ordering::Relaxed);
}

const SHIFT: i64 = 0x0200_0000;
const CONTROL: i64 = 0x0400_0000;
const ALT: i64 = 0x0800_0000;
const META: i64 = 0x1000_0000;
pub const RETURN: i64 = 0x0100_0004;
pub const ENTER: i64 = 0x0100_0005;

/// The modifier glyphs of a Qt modifier mask, in the Mac's order (⌃⌥⇧⌘).
/// Qt calls ⌘ "Control" on the Mac and the Control key "Meta".
pub fn modifiers(mods: i64) -> String {
    let mac = cfg!(target_os = "macos");
    let (command, control) = if mac { (CONTROL, META) } else { (META, CONTROL) };
    let mut out = String::new();
    for (bit, glyph, word) in [
        (control, "⌃", "Ctrl+"),
        (ALT, "⌥", "Alt+"),
        (SHIFT, "⇧", "Shift+"),
        (command, "⌘", "Super+"),
    ] {
        if mods & bit != 0 {
            out.push_str(if mac { glyph } else { word });
        }
    }
    out
}

/// The label of a key press: its modifiers, then the key. None for a
/// modifier on its own (it shows with the key it goes with).
pub fn chord(key: i64, mods: i64) -> Option<String> {
    let name = match key {
        0x0100_0020..=0x0100_0023 | 0x0100_0024 | 0x0100_0025 => return None,
        0x0100_0000 => "esc".to_owned(),
        0x0100_0001 | 0x0100_0002 => "⇥".to_owned(),
        0x0100_0003 => "⌫".to_owned(),
        RETURN | ENTER => "↩".to_owned(),
        0x0100_0007 => "⌦".to_owned(),
        0x0100_0010 => "↖".to_owned(),
        0x0100_0011 => "↘".to_owned(),
        0x0100_0012 => "←".to_owned(),
        0x0100_0013 => "↑".to_owned(),
        0x0100_0014 => "→".to_owned(),
        0x0100_0015 => "↓".to_owned(),
        0x0100_0016 => "⇞".to_owned(),
        0x0100_0017 => "⇟".to_owned(),
        0x0100_0030..=0x0100_0052 => format!("F{}", key - 0x0100_0030 + 1),
        0x20 => "Space".to_owned(),
        k if (0x21..0x0100_0000).contains(&k) => {
            char::from_u32(k as u32)?.to_uppercase().collect()
        }
        _ => return None,
    };
    Some(modifiers(mods) + &name)
}

/// One thing the pill shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Chip {
    /// The key or chord ("⌘Z", "R"), or the value typed ("2.5m").
    pub keys: String,
    /// The tool the key armed, after the key.
    pub tool: String,
    /// A typed value (drawn as a field, not a key).
    pub typed: bool,
}

/// What the pill holds, and when it last changed (seconds on the UI clock).
#[derive(Default)]
pub struct Keycast {
    pub chips: Vec<Chip>,
    pub at: f64,
}

impl Keycast {
    fn push(&mut self, chip: Chip, now: f64) {
        // A pause long enough for the pill to have gone starts afresh.
        if now - self.at > HOLD + FADE {
            self.chips.clear();
        }
        self.chips.push(chip);
        if self.chips.len() > KEEP {
            self.chips.remove(0);
        }
        self.at = now;
    }
    /// A key press that did nothing else visible.
    pub fn key(&mut self, keys: String, now: f64) {
        self.push(Chip { keys, tool: String::new(), typed: false }, now);
    }
    /// A key that armed a tool.
    pub fn tool(&mut self, keys: String, tool: String, now: f64) {
        self.push(Chip { keys, tool, typed: false }, now);
    }
    /// The value field holds `value` after a key: the chip of the value
    /// being typed grows, or a new one starts.
    pub fn typed(&mut self, value: String, now: f64) {
        let fresh = now - self.at <= HOLD + FADE;
        match self.chips.last_mut() {
            Some(last) if last.typed && fresh => {
                last.keys = value;
                self.at = now;
            }
            _ => self.push(Chip { keys: value, tool: String::new(), typed: true }, now),
        }
    }
    /// A mouse button (L, R, M) pressed in the model. A plain left click
    /// is what every tool does all the time and shows nothing; a right or
    /// middle click, or a click with a modifier, shows as a badge.
    pub fn button(&mut self, button: u32, mods: i64, now: f64) {
        if button == 1 && mods & (SHIFT | CONTROL | ALT | META) == 0 {
            return;
        }
        let name = match button {
            1 => "L",
            2 => "R",
            4 => "M",
            _ => return,
        };
        self.key(format!("{}{name} click", modifiers(mods)), now);
    }
    /// How opaque the pill is at `now`: 1 while held, then fading; None
    /// when gone (or never shown).
    pub fn opacity(&self, now: f64) -> Option<f32> {
        if self.chips.is_empty() {
            return None;
        }
        let age = now - self.at;
        if age < HOLD {
            Some(1.)
        } else if age < HOLD + FADE {
            Some((1. - (age - HOLD) / FADE) as f32)
        } else {
            None
        }
    }
    /// Milliseconds until the pill next changes (a fade step or going).
    pub fn repaint_ms(&self, now: f64) -> Option<f64> {
        self.opacity(now)?;
        let age = now - self.at;
        Some(if age < HOLD { (HOLD - age) * 1000. } else { 16. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_read_as_the_keys_pressed() {
        let cmd = if cfg!(target_os = "macos") { CONTROL } else { META };
        assert_eq!(chord('Z' as i64, 0).as_deref(), Some("Z"));
        assert_eq!(chord(RETURN, 0).as_deref(), Some("↩"));
        assert_eq!(chord(0x0100_0000, 0).as_deref(), Some("esc"));
        assert_eq!(chord(0x0100_0020, SHIFT), None, "a modifier alone");
        if cfg!(target_os = "macos") {
            assert_eq!(chord('Z' as i64, cmd | SHIFT).as_deref(), Some("⇧⌘Z"));
            assert_eq!(chord('1' as i64, META | ALT).as_deref(), Some("⌃⌥1"));
        }
        assert_eq!(chord(0x0100_0031, 0).as_deref(), Some("F2"));
    }

    #[test]
    fn a_typed_value_grows_in_one_chip_and_the_pill_fades() {
        let mut k = Keycast::default();
        assert_eq!(k.opacity(0.), None);
        k.tool("R".into(), "Rectangle".into(), 1.0);
        k.typed("2".into(), 1.2);
        k.typed("2.5".into(), 1.4);
        k.key("↩".into(), 1.5);
        assert_eq!(k.chips.len(), 3);
        assert_eq!(k.chips[1], Chip { keys: "2.5".into(), tool: String::new(), typed: true });
        assert_eq!(k.opacity(1.5 + HOLD - 0.01), Some(1.));
        let half = k.opacity(1.5 + HOLD + FADE / 2.).unwrap();
        assert!((half - 0.5).abs() < 1e-3);
        assert_eq!(k.opacity(1.5 + HOLD + FADE + 0.01), None);
        // After the pill went, the next key starts a fresh pill.
        k.key("P".into(), 10.);
        assert_eq!(k.chips.len(), 1);
        for i in 0..10 {
            k.key(format!("{i}"), 10. + i as f64 * 0.1);
        }
        assert_eq!(k.chips.len(), KEEP);
        let mut k = Keycast::default();
        k.button(1, 0, 0.);
        assert!(k.chips.is_empty(), "a plain click shows nothing");
        k.button(4, 0, 0.);
        k.button(1, SHIFT, 0.1);
        assert_eq!(k.chips.iter().map(|c| c.keys.as_str()).collect::<Vec<_>>(),
                   ["M click", if cfg!(target_os = "macos") { "⇧L click" } else { "Shift+L click" }]);
    }
}
