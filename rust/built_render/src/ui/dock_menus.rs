// SPDX-License-Identifier: GPL-3.0-or-later
//! The dock header's menus: the tree's columns, the filter popover, the
//! delivery picker and the tree's own short "⋯" menu. One is open at a
//! time, under its control and inside the window. What acts on the whole
//! model (links, quantities, drawing standards, plugins) is not here: the
//! Model menu of the top bar lists it (`model_rows`).
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HeadMenu {
    Columns,
    Filter,
    More,
}

fn row(ui: &mut egui::Ui, label: &str, hint: &str, checked: i8, enabled: bool) -> bool {
    // Every row keeps the check lane, so one menu has one text indent.
    menu_row(ui, &(String::new(), label.to_owned(), hint.to_owned(), "check".to_owned(), checked, enabled))
}

/// The presets, then each column. A preset closes the menu; a column
/// toggle leaves it open for the next one.
fn columns(col: &mut Column, ui: &mut egui::Ui, actions: &mut Vec<String>) -> bool {
    let mut close = false;
    caption_at(ui, "Presets", 12.0);
    for preset in Lens::ALL {
        if row(ui, preset.label(), "", i8::from(col.columns.matching() == Some(preset)), true) {
            col.columns = lens::Columns::preset(preset);
            col.cells_scroll = 0.;
            actions.push(format!("lens\u{1f}{}", preset as u8));
            close = true;
        }
    }
    menu_separator(ui);
    caption_at(ui, "Columns", 12.0);
    for column in Col::ALL {
        if row(ui, column.label(), "", i8::from(col.columns.has(column)), true) {
            col.columns.toggle(column);
            actions.push(format!("columns\u{1f}{}", col.columns.mask()));
        }
    }
    close
}

/// The filter's quick choices and the order of the rows.
fn filter(col: &mut Column, ui: &mut egui::Ui, actions: &mut Vec<String>) -> bool {
    if tree_filter::controls(ui, &mut col.filter, col.proposal_available) {
        actions.push(format!("filter\u{1f}{}", col.filter));
    }
    menu_separator(ui);
    let options = tree_order::Sort::OPTIONS;
    let old = options.iter().position(|(sort, _)| *sort == col.sort).unwrap_or(0);
    let mut index = old;
    kit::property_control(ui, "Sort by", |ui| {
        kit::choice(ui, "project_sort", &mut index, &options.map(|(_, label)| label));
    });
    if index != old {
        col.sort = options[index].0;
        col.ordered = col.sort.order(&col.nodes);
        actions.push(format!("sort\u{1f}{index}"));
    }
    false
}

/// The tree's own verbs: what it holds, how it is unfolded, what it shows.
fn more(col: &mut Column, ui: &mut egui::Ui, selected: bool, actions: &mut Vec<String>) -> bool {
    let mut close = false;
    let mut send = |ui: &mut egui::Ui, label: &str, hint: &str, enabled: bool, action: String| {
        if row(ui, label, hint, -1, enabled) {
            actions.push(action);
            close = true;
        }
    };
    send(ui, "New node", "", true, "new\u{1f}".into());
    if col.proposal_available {
        send(ui, "New tag", "", true, format!("newtag\u{1f}{}", col.styles.creation_row()));
    }
    send(ui, "Propose structure from tags…", "", true, "propose\u{1f}".into());
    menu_separator(ui);
    send(ui, "Expand all", "", true, "expandall\u{1f}".into());
    send(ui, "Collapse all", "", true, "collapseall\u{1f}".into());
    menu_separator(ui);
    let shortcut = if cfg!(target_os = "macos") { "⇧⌘J" } else { "Ctrl+Shift+J" };
    send(ui, "Reveal selection", shortcut, selected, "revealselection\u{1f}".into());
    send(ui, "Show all", "", true, "showall\u{1f}".into());
    if col.styles.catalog.tag_isolation {
        send(ui, "End tag isolation", "", true, "isolatetags\u{1f}[]".into());
    }
    close
}

/// The open header menu, under its control.
pub(super) fn show(
    col: &mut Column,
    ctx: &egui::Context,
    screen: Rect,
    selected: bool,
    actions: &mut Vec<String>,
) {
    let Some((menu, anchor)) = col.head_menu else { return };
    let width = match menu {
        HeadMenu::Columns => 190.,
        HeadMenu::Filter => 250.,
        _ => 230.,
    };
    let id = egui::Id::new("head_menu");
    // The size it had last frame keeps it inside the window.
    let size = ctx.memory(|m| m.area_rect(id)).map_or(vec2(width, ROW_H * 8.), |r| vec2(width, r.height()));
    let trailing = matches!(menu, HeadMenu::Filter | HeadMenu::More);
    let area = egui::Area::new(id)
        .fixed_pos(kit::popover_pos(anchor, size, screen, trailing))
        .order(egui::Order::Tooltip)
        .show(ctx, |ui| {
            kit::menu(ui, width, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("head menu", menu as u8))
                    .max_height((screen.height() - GUT * 4.).max(ROW_H * 3.))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                        match menu {
                            HeadMenu::Columns => columns(col, ui, actions),
                            HeadMenu::Filter => filter(col, ui, actions),
                            HeadMenu::More => more(col, ui, selected, actions),
                        }
                    })
                    .inner
            })
            .inner
        });
    // A press outside the menu and its control closes it (a choice's own
    // list opens outside the menu: it is not "outside").
    let outside = !ctx.memory(|m| m.any_popup_open())
        && ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p) && !anchor.contains(p))
        });
    if area.inner || outside {
        col.head_menu = None;
    }
}

/// The Model menu's rows for the open model: what acts on the whole file
/// rather than on the tree. Returns the tree actions to send and whether
/// "Plugins" was chosen.
pub(super) fn model_rows(col: &mut Column, ui: &mut egui::Ui, plugins: bool) -> (Vec<String>, bool) {
    let mut actions = Vec::new();
    let mut toggle_plugins = false;
    if !col.mode.is_empty() {
        return (actions, false);
    }
    menu_separator(ui);
    let picked = col.browser_picks > 0;
    let can_rename = picked || col.unfiled > 0 || col.nodes.iter().any(|n| n.local_placement || n.key.starts_with("n:") && n.count > 0);
    let label = if picked { "Rename selected…" } else if col.filter.is_empty() { "Rename model rows…" } else { "Rename matching rows…" };
    if row(ui, label, "", -1, can_rename) {
        actions.push(col.rename_request(picked));
    }
    for (id, label) in [
        ("linkselection", "Link selection from open file…"),
        ("linkregistry", "Move registry to project…"),
        ("measure", "Measure quantities"),
        ("exportcsv", "Export quantities CSV…"),
    ] {
        if row(ui, label, "", -1, true) {
            actions.push(format!("{id}\u{1f}"));
        }
    }
    menu_separator(ui);
    let catalog = &mut col.styles.catalog;
    for (id, label, on) in [
        ("tagcolours", "Colour by tag", catalog.color_by_layer),
        ("modellines", "Line types in the model", catalog.model_linetypes),
        ("newtaghidden", "New tags hidden in saved scenes", catalog.new_hidden),
    ] {
        if row(ui, label, "", i8::from(on), true) {
            if id == "newtaghidden" {
                catalog.new_hidden = !on;
            }
            actions.push(format!("{id}\u{1f}{}", u8::from(!on)));
        }
    }
    for (id, label) in [
        ("importtags", "Import layer template…"),
        ("exporttags", "Export layer template…"),
        ("importpat", "Import hatch patterns…"),
        ("exportpat", "Export hatch patterns…"),
        ("purgetags", "Purge unused tags…"),
    ] {
        if row(ui, label, "", -1, true) {
            actions.push(format!("{id}\u{1f}"));
        }
    }
    if plugins {
        menu_separator(ui);
        toggle_plugins = row(ui, "Plugins", "", -1, true);
    }
    (actions, toggle_plugins)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menus_stay_inside_the_window() {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280., 800.));
        let size = vec2(230., 400.);
        // Under its control when there is room, lined up with it.
        let anchor = Rect::from_min_size(pos2(1000., 60.), vec2(100., 24.));
        assert_eq!(kit::popover_pos(anchor, size, screen, false), pos2(1000., 88.));
        // The "⋯" at the window's edge: the menu ends where the button ends.
        let dots = Rect::from_min_size(pos2(1240., 60.), vec2(24., 24.));
        assert_eq!(kit::popover_pos(dots, size, screen, true).x, 1264. - 230.);
        // Never past the right edge or the bottom: moved in, or over the control.
        let low = Rect::from_min_size(pos2(1200., 700.), vec2(60., 24.));
        let at = kit::popover_pos(low, size, screen, false);
        assert!(at.x + size.x <= 1280. - GUT && at.y + size.y <= 800. - GUT && at.y >= GUT);
        let tall = kit::popover_pos(anchor, vec2(230., 2000.), screen, false);
        assert_eq!(tall.y, GUT);
    }
}
