// Compare the installed AppKit tree on the main thread, including key equivalents
// and selectors for standard roles. No windows or system actions are invoked.
#[cfg(target_os = "macos")]
#[path = "../src/menu.rs"]
mod menu;

#[cfg(target_os = "macos")]
fn main() {
    use objc2::{MainThreadMarker, Message};
    use objc2_app_kit::{NSApplication, NSMenu};
    fn walk(tree: &NSMenu, path: &str, rows: &mut Vec<String>) {
        for item in tree.itemArray() {
            let title = item.title().to_string();
            let id = menu::command_id(path, &title).unwrap_or_default();
            let selector = item
                .action()
                .map(|s| s.name().to_string_lossy().into_owned())
                .unwrap_or_default();
            let title = if selector == "hide:" {
                "Hide <application>".to_owned()
            } else {
                title
            };
            let id = if title == menu::EMPTY_RECENT {
                String::new()
            } else {
                id
            };
            rows.push(format!(
                "{path}\t{title}\t{id}\t{:?}\t{}\t{}\t[{selector}]",
                item.keyEquivalent().to_string(),
                item.keyEquivalentModifierMask().bits(),
                item.isEnabled()
            ));
            if let Some(sub) = item.submenu() {
                let next = if path.is_empty() {
                    title
                } else {
                    format!("{path} ▸ {title}")
                };
                walk(&sub, &next, rows);
            }
        }
    }
    let marker = MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(marker);
    let _menus = menu::build().unwrap();
    let mut rows = Vec::new();
    walk(&app.mainMenu().unwrap(), "", &mut rows);
    one_home_and_one_label(&rows);
    let actual = rows.join("\n") + "\n";
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/mac_menu_tree.txt");
    // MAC_MENU_TREE_BLESS=1 rewrites the fixture after an intended change.
    if std::env::var_os("MAC_MENU_TREE_BLESS").is_some() {
        std::fs::write(fixture, &actual).unwrap();
    }
    assert_eq!(actual, std::fs::read_to_string(fixture).unwrap());
}
/// docs/menus.md, rule 1, on the installed tree: a command id has one
/// home across the menu bar (unless allow-listed), and within one
/// top-level menu a visible label names one row (Import ▸ and Export ▸
/// name the same formats on purpose).
#[cfg(target_os = "macos")]
fn one_home_and_one_label(rows: &[String]) {
    use std::collections::HashMap;
    let mut homes: HashMap<&str, &str> = HashMap::new();
    let mut labels: HashMap<(String, &str), &str> = HashMap::new();
    let mut places: HashMap<(&str, &str), ()> = HashMap::new();
    for row in rows {
        let cols: Vec<&str> = row.split('\t').collect();
        let (path, title, id) = (cols[0], cols[1], cols[2]);
        if title.is_empty() {
            continue; // separators
        }
        assert!(places.insert((path, title), ()).is_none(), "{path} ▸ {title} twice");
        if id.is_empty() {
            continue; // submenus and system rows
        }
        let top = path.split(" ▸ ").next().unwrap();
        if let Some(first) = homes.insert(id, path) {
            assert!(
                built_render::desktop_menu_model::SHARED_COMMANDS.contains(&id)
                    && first.split(" ▸ ").next() != Some(top),
                "{id} is in {first} and in {path}"
            );
        }
        let menu = if matches!(path, "File ▸ Import" | "File ▸ Export") { path.to_owned() } else { top.to_owned() };
        if let Some(other) = labels.insert((menu.clone(), title), id) {
            panic!("“{title}” names {other} and {id} in {menu}");
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {}
