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
    let actual = rows.join("\n") + "\n";
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/mac_menu_tree.txt");
    assert_eq!(actual, std::fs::read_to_string(fixture).unwrap());
}
#[cfg(not(target_os = "macos"))]
fn main() {}
