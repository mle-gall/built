//! Bind the installed AppKit menu tree to command search. Read the live tree
//! at opening so imports, system services and recent files have the same
//! actions, shortcuts and availability as their menu items.
use objc2::{rc::Retained, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSEventModifierFlags as Flags, NSMenu, NSMenuItem};
use objc2_foundation::NSString;
use pyo3::prelude::*;
use std::collections::HashMap;

#[derive(Clone)]
struct FactoryKey {
    text: String,
    modifiers: Flags,
    portable: String,
}

#[pyclass(unsendable)]
pub struct NativeMenuCommand {
    menu: Retained<NSMenu>,
    item: Retained<NSMenuItem>,
    factory: FactoryKey,
    #[pyo3(get)]
    name: String,
    #[pyo3(get)]
    path: String,
    #[pyo3(get)]
    keys: String,
    #[pyo3(get)]
    id: String,
}

thread_local! {
    static DEFAULTS: std::cell::RefCell<HashMap<String, FactoryKey>> = Default::default();
}

fn portable_key(key: &str, flags: Flags) -> String {
    if key.is_empty() {
        return String::new();
    }
    let mut text = String::new();
    for (flag, prefix) in [
        (Flags::Control, "Meta+"),
        (Flags::Option, "Alt+"),
        (Flags::Shift, "Shift+"),
        (Flags::Command, "Ctrl+"),
    ] {
        if flags.contains(flag) {
            text.push_str(prefix);
        }
    }
    text.push_str(&match key {
        "\t" => "Tab".into(),
        "\r" => "Return".into(),
        " " => "Space".into(),
        "\u{8}" => "Backspace".into(),
        "\u{f700}" => "Up".into(),
        "\u{f701}" => "Down".into(),
        "\u{f702}" => "Left".into(),
        "\u{f703}" => "Right".into(),
        k if k.chars().count() == 1
            && (0xf704..=0xf726).contains(&(k.chars().next().unwrap() as u32)) =>
        {
            format!("F{}", k.chars().next().unwrap() as u32 - 0xf703)
        }
        _ => key.to_uppercase(),
    });
    text
}

impl FactoryKey {
    fn read(menu: &NSMenu, item: &NSMenuItem) -> Self {
        let text = item.keyEquivalent().to_string();
        let modifiers = item.keyEquivalentModifierMask();
        // AppKit shows Globe+F for Full Screen and also installs the regular
        // Control+Command+F equivalent. Qt cannot encode the Globe modifier.
        let portable = if modifiers.contains(Flags::Function) {
            menu.itemArray()
                .iter()
                .find(|other| {
                    other.isHidden()
                        && other.action() == item.action()
                        && other.title() == item.title()
                        && !other.keyEquivalentModifierMask().contains(Flags::Function)
                        && !other.keyEquivalent().is_empty()
                })
                .map(|other| {
                    portable_key(
                        &other.keyEquivalent().to_string(),
                        other.keyEquivalentModifierMask(),
                    )
                })
                .unwrap_or_default()
        } else {
            portable_key(&text, modifiers)
        };
        Self {
            text,
            modifiers,
            portable,
        }
    }
}

#[pymethods]
impl NativeMenuCommand {
    fn visible(&self) -> bool {
        (0..self.menu.numberOfItems()).contains(&self.menu.indexOfItem(&self.item))
            && !self.item.isHiddenOrHasHiddenAncestor()
    }
    #[pyo3(name = "isEnabled")]
    fn enabled(&self) -> bool {
        self.item.isEnabled()
    }
    fn set_shortcut(&self, portable: &str, factory: bool) {
        if factory {
            self.apply_equivalent(&self.factory.text, self.factory.modifiers);
            return;
        }
        let mut rest = portable;
        let mut flags = Flags::empty();
        while let Some((prefix, tail)) = rest.split_once('+') {
            let flag = match prefix {
                "Ctrl" => Flags::Command,
                "Alt" => Flags::Option,
                "Shift" => Flags::Shift,
                "Meta" => Flags::Control,
                _ => break,
            };
            flags |= flag;
            rest = tail;
        }
        // A bare letter or number must first reach the focused field or tool.
        if !flags.intersects(Flags::Command | Flags::Option | Flags::Control) {
            rest = "";
        }
        let key = match rest {
            "⇥" | "Tab" => "\t".to_owned(),
            "↩" | "Return" => "\r".to_owned(),
            "Space" => " ".to_owned(),
            "⌫" | "Backspace" => "\u{8}".to_owned(),
            "←" | "Left" => "\u{f702}".to_owned(),
            "→" | "Right" => "\u{f703}".to_owned(),
            "↑" | "Up" => "\u{f700}".to_owned(),
            "↓" | "Down" => "\u{f701}".to_owned(),
            key if key.starts_with('F') && key.len() > 1 => key[1..]
                .parse::<u32>()
                .ok()
                .filter(|n| (1..=35).contains(n))
                .and_then(|n| char::from_u32(0xf703 + n))
                .map(String::from)
                .unwrap_or_default(),
            key if key.chars().count() == 1 => key.to_lowercase(),
            _ => String::new(),
        };
        self.apply_equivalent(&key, flags);
    }

    fn trigger(&self) -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        let Some(root) = NSApplication::sharedApplication(mtm).mainMenu() else {
            return false;
        };
        let mut parent = Some(self.menu.clone());
        while let Some(menu) = parent {
            if std::ptr::eq(&*menu, &*root) {
                self.menu.update();
                let index = self.menu.indexOfItem(&self.item);
                if index >= 0 && self.visible() && self.enabled() {
                    self.menu.performActionForItemAtIndex(index);
                    return true;
                }
                break;
            }
            // AppKit retains the installed menu hierarchy on the main thread.
            parent = unsafe { menu.supermenu() };
        }
        false
    }
}

impl NativeMenuCommand {
    fn apply_equivalent(&self, text: &str, flags: Flags) {
        // Even an identical setter can make AppKit regenerate hidden aliases.
        if self.item.keyEquivalent().to_string() != text {
            self.item.setKeyEquivalent(&NSString::from_str(text));
        }
        if self.item.keyEquivalentModifierMask() != flags {
            self.item.setKeyEquivalentModifierMask(flags);
        }
    }
}

fn collect(menu: &NSMenu, trail: &mut Vec<String>, out: &mut Vec<NativeMenuCommand>) {
    menu.update();
    for item in menu.itemArray().iter() {
        if item.isSeparatorItem() || item.isHidden() {
            continue;
        }
        let name = item.title().to_string();
        if let Some(sub) = item.submenu() {
            let title = if name.is_empty() {
                sub.title().to_string()
            } else {
                name
            };
            trail.push(title);
            collect(&sub, trail, out);
            trail.pop();
        } else if !name.is_empty() && item.action().is_some() {
            use objc2::Message;
            let path = trail.join(" ▸ ");
            if name == crate::menu::EMPTY_RECENT && trail.last().is_some_and(|p| p == "Open Recent")
            {
                continue;
            }
            let selector = item.action().unwrap().name().to_string_lossy();
            let id = crate::menu::command_id(&path, &name).unwrap_or_else(|| {
                if trail.last().is_some_and(|p| p == "Open Recent") {
                    format!("recent-file:{name}")
                } else if matches!(
                    selector.as_ref(),
                    "hide:"
                        | "hideOtherApplications:"
                        | "unhideAllApplications:"
                        | "toggleFullScreen:"
                        | "performMiniaturize:"
                        | "performZoom:"
                        | "orderFrontCharacterPalette:"
                        | "startDictation:"
                ) {
                    format!("platform:{selector}")
                } else {
                    // Window and Services menus can share a selector across distinct items.
                    format!("platform:{selector}:{path}:{name}")
                }
            });
            let factory = DEFAULTS.with(|d| {
                d.borrow_mut()
                    .entry(id.clone())
                    .or_insert_with(|| FactoryKey::read(menu, &item))
                    .clone()
            });
            out.push(NativeMenuCommand {
                id,
                name,
                path,
                keys: factory.portable.clone(),
                factory,
                menu: menu.retain(),
                item,
            });
        }
    }
}

#[pyfunction]
pub fn menu_commands() -> Vec<NativeMenuCommand> {
    let mut rows = Vec::new();
    if let Some(menu) =
        MainThreadMarker::new().and_then(|mtm| NSApplication::sharedApplication(mtm).mainMenu())
    {
        collect(&menu, &mut Vec::new(), &mut rows);
    }
    DEFAULTS.with(|defaults| {
        let live: std::collections::HashSet<_> = rows.iter().map(|row| row.id.as_str()).collect();
        defaults
            .borrow_mut()
            .retain(|id, _| live.contains(id.as_str()));
    });
    rows
}
