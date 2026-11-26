// SPDX-License-Identifier: GPL-3.0-or-later
//! Reconcile the rows supplied by the document with the visible Browser nodes.
use super::*;
use std::collections::HashMap;

type WireRow = (Vec<String>, Vec<i64>);

fn update_text(slot: &mut String, value: &str) {
    if slot != value {
        value.clone_into(slot);
    }
}

pub(super) fn reconcile(mut previous: Vec<Node>, rows: Vec<WireRow>) -> Vec<Node> {
    // A context row and its Filing counterpart can have the same key. Match
    // their row roles separately, in display order within each role.
    let mut heads = HashMap::with_capacity(previous.len());
    let mut next = vec![None; previous.len()];
    for (i, node) in previous.iter_mut().enumerate().rev() {
        let key = std::mem::take(&mut node.key);
        next[i] = heads.insert((key, node.row), i);
    }
    let mut ancestors = Vec::new();
    rows.into_iter()
        .map(|(mut text, numbers)| {
            let n = |i: usize| numbers.get(i).copied().unwrap_or_default();
            let depth = n(0) as usize;
            let role = n(4) as u8;
            let key = text.first_mut().map(std::mem::take).unwrap_or_default();
            let identity = (key, role);
            let mut node = if let Some(index) = heads.remove(&identity) {
                if let Some(successor) = next[index] {
                    heads.insert(identity.clone(), successor);
                }
                std::mem::take(&mut previous[index])
            } else {
                Node::default()
            };
            node.key = identity.0;
            let t = |i: usize| text.get(i).map(String::as_str).unwrap_or_default();
            let (kind, tip) = t(2).split_once('\u{1e}').unwrap_or((t(2), ""));
            update_text(&mut node.name, t(1));
            update_text(&mut node.kind, kind);
            update_text(&mut node.tip, tip);
            update_text(&mut node.badge, t(3));
            update_text(&mut node.icon, t(4));
            let control_tips = (n(5) == 2 || n(6) == 2) && text.len() >= 7;
            let end = if control_tips {
                text.len() - 2
            } else {
                text.len()
            };
            for (i, slot) in node.control_tips.iter_mut().enumerate() {
                update_text(slot, if control_tips { t(end + i) } else { "" });
            }
            let cells = text.get(5..end).unwrap_or_default();
            if node.cells != cells {
                node.hatch = kit::hatch_override(cells).map(Box::new);
                node.cells = cells.to_vec();
            }
            ancestors.truncate(depth);
            let linked = ancestors.last().copied().unwrap_or(false)
                || node.key.starts_with("n:link:")
                || node.icon == "link";
            ancestors.push(linked);
            node.depth = depth;
            node.row = role;
            node.local_placement = !linked
                && matches!(role, 1 | 2)
                && (node.key.starts_with("i:") || node.key.starts_with("c:"));
            node.count = n(1) as u32;
            node.problems = n(2) as u32;
            node.active = n(3) != 0;
            node.eye = n(5) as i8;
            node.lock = n(6) as i8;
            node.expandable = n(7) != 0;
            node.expanded = n(8) != 0;
            node.selected = n(9) as u32;
            node.mark = n(10) as u8;
            node.dim = n(11) != 0 || n(5) == 2;
            node
        })
        .collect()
}

pub(super) fn show_menu(
    col: &mut Column,
    ctx: &egui::Context,
    height: f32,
    actions: &mut Vec<String>,
) {
    let Some((key, row, at)) = col.row_menu.clone() else {
        return;
    };
    let node_menu: &[(&str, &str)] = &[
        ("active", "Draw into this node"),
        ("filehere", "File selection here"),
        ("node", "Select its items"),
        ("isolate", "Isolate"),
        ("hidemembers", "Hide its items"),
        ("-", ""),
        ("rename", "Rename…"),
        ("newchild", "New child node"),
        ("kind", "Kind  ›"),
        ("-", ""),
        ("delete", "Delete node"),
    ];
    let item_menu: &[(&str, &str)] = &[
        ("item", "Select"),
        ("open", "Open for editing"),
        ("rename", "Rename…"),
        ("-", ""),
        ("hideitem", "Hide"),
        ("lockitem", "Lock / unlock"),
        ("zoom", "Zoom to"),
        ("groupplan", "Plan of this group…"),
        ("-", ""),
        ("fileto", "File to…"),
    ];
    let comp_menu: &[(&str, &str)] = &[
        ("cplace", "Place a copy"),
        ("cselect", "Select instances"),
        ("creplace", "Replace selection with this"),
        ("-", ""),
        ("ckeep", "Keep in file"),
        ("crename", "Rename…"),
    ];
    // A linked file's rows are read-only here: no filing, kind or rename.
    let link_menu: &[(&str, &str)] = &[
        ("node", "Select"),
        ("contents", "Select contents"),
        ("isolate", "Isolate"),
        ("zoom", "Zoom to"),
        ("-", ""),
        ("eye", "Show / hide"),
        ("lock", "Lock / unlock"),
        ("-", ""),
        ("linkopen", "Edit source"),
        ("linkfinder", "Show in Finder"),
        ("linkreload", "Reload"),
        ("linkrelink", "Locate…"),
        ("linkunlink", "Unlink"),
    ];
    let section_menu: &[(&str, &str)] = &[
        ("secname", "Name and symbol…"),
        ("cutview", "View of the cut…"),
        ("sectioncut", "Active cut"),
        ("eye", "Show / hide"),
        ("deleterows", "Delete"),
    ];
    let tag_menu: &[(&str, &str)] = &[
        ("newtag", "New tag"),
        ("rename", "Rename…"),
        ("tagactive", "Make active"),
        ("tagassign", "Move selection here"),
        ("tagselect", "Select items"),
        ("eye", "Hide"),
        ("movefolder", "Move to folder…"),
        ("tagisolate", "Isolate"),
        ("mergetag", "Merge into…"),
        ("scenetaghide", "Hide in all saved scenes"),
        ("scenetagshow", "Show in all saved scenes"),
        ("deletetag", "Delete"),
        ("purgetags", "Purge unused tags…"),
    ];
    let Some(n) = col.nodes.iter().find(|n| n.key == key && n.row == row) else {
        col.row_menu = None;
        return;
    };
    let menu: &[(&str, &str)] = match key.split_once(':').map(|(kind, _)| kind) {
        Some("preview-tag") => tag_menu,
        Some("preview-folder") => &[
            ("newtag", "New tag in folder"),
            ("tagselect", "Select items"),
            ("rename", "Rename…"),
            ("eye", "Hide"),
            ("tagisolate", "Isolate"),
            ("movefolder", "Move to folder…"),
            ("deletefolder", "Remove folder"),
        ],
        Some("section") => section_menu,
        Some("project-file") if n.icon == "file:missing" => &[("linkrelink", "Locate…")],
        Some("project-file") => &[("open", "Open"), ("edit", "Edit source")],
        Some("exportitem") => &[("item", "Select"), ("zoom", "Zoom to")],
        Some("d") => &[
            ("contents", "Select instances"),
            ("hideitem", "Hide"),
            ("zoom", "Zoom to"),
            ("fileto", "File to…"),
        ],
        Some("sheet-page") => &[
            ("item", "Inspect"),
            ("open", "Open sheet"),
            ("renamesheet", "Rename…"),
            ("deletesheet", "Delete…"),
        ],
        Some("sheet-frame") => &[("item", "Inspect"), ("open", "Open sheet")],
        Some("saved-scene") => &[
            ("item", "Inspect"),
            ("open", "Show scene"),
            ("rename", "Rename…"),
            ("deletescene", "Delete"),
        ],
        Some("sections") => &[("toggle", "Expand"), ("eye", "Hide")],
        Some("geometry") => &[("item", "Select items"), ("eye", "Hide"), ("lock", "Lock")],
        Some("m") => &[("loose", "Show more")],
        Some("n") if key.starts_with("n:link") => link_menu,
        Some("n") if key == "n:" => &[
            ("node", "Select items"),
            ("isolate", "Isolate"),
            ("hidemembers", "Hide"),
        ],
        Some("n") => node_menu,
        Some("cm") => comp_menu,
        Some("i" | "c") if !n.local_placement => &[("item", "Select"), ("zoom", "Zoom to")],
        _ if n.local_placement || matches!(row, 1..=3) => item_menu,
        _ if n.expandable => &[("toggle", "Expand")],
        _ => &[("item", "Inspect")],
    };
    let mut entries = Vec::new();
    if col.choosing_folder {
        entries.push(("destination:".into(), "Root".into(), true));
        for (folder, path) in &col.styles.catalog.folders {
            if col.styles.catalog.can_move_to(&key, folder) {
                entries.push((format!("destination:{folder}"), path.clone(), true));
            }
        }
    } else {
        let editable_file = n.icon.starts_with("file:") && !n.badge.starts_with("via ");
        for &(id, label) in menu {
            if key == "preview-tag:Layer 0"
                && matches!(
                    id,
                    "rename" | "deletetag" | "mergetag" | "tagisolate" | "movefolder"
                )
            {
                continue;
            }
            if n.icon == "file:missing"
                && matches!(
                    id,
                    "contents" | "isolate" | "zoom" | "linkopen" | "linkfinder" | "linkreload"
                )
            {
                continue;
            }
            if key.starts_with("n:link")
                && !editable_file
                && matches!(id, "linkreload" | "linkrelink" | "linkunlink")
            {
                continue;
            }
            if id == "groupplan" && !n.local_placement {
                continue;
            }
            if id == "hideitem" {
                entries.extend(
                    col.group_rows
                        .iter()
                        .map(|(id, label, why)| (id.clone(), label.clone(), why.is_empty())),
                );
            }
            let label = match id {
                "rename" if n.local_placement && n.selected > 0 && col.browser_picks > 1 => {
                    "Rename selected…"
                }
                "eye" if n.eye == 0 => "Show",
                "lock" | "lockitem" if n.lock == 1 => "Unlock",
                "lockitem" => "Lock",
                "toggle" if n.expanded => "Collapse",
                _ => label,
            };
            let enabled = id != "deletesheet"
                || col
                    .nodes
                    .iter()
                    .any(|n| n.key == "sheet-pages:all" && n.count > 1);
            entries.push((id.into(), label.into(), enabled));
        }
    }
    let rows = crate::verbs::tree_menu_rows(entries);
    let heading = if col.choosing_folder {
        "Move to folder…"
    } else {
        &n.name
    };
    let response = egui::Area::new(egui::Id::new("row_menu"))
        .fixed_pos(at + vec2(SPACE_XS, SPACE_XS))
        .order(egui::Order::Tooltip)
        .show(ctx, |ui| {
            glass(Surface::Raised)
                .inner_margin(Margin::same(6))
                .show(ui, |ui| {
                    ui.set_width(220.);
                    ui.spacing_mut().item_spacing = vec2(0., 1.);
                    egui::ScrollArea::vertical()
                        .max_height(height - SPACE_M)
                        .show(ui, |ui| {
                            context_menu::menu_contents(
                                ui,
                                ui.available_width(),
                                heading,
                                "",
                                &rows,
                                &col.group_rows,
                            )
                        })
                        .inner
                })
                .inner
        });
    if let Some(id) = response.inner {
        match id.as_str() {
            "rename" if n.local_placement && n.selected > 0 && col.browser_picks > 1 => {
                actions.push(col.rename_request(true))
            }
            "rename" => {
                col.renaming = Some((key.clone(), n.name.clone()));
                col.focus_rename = true;
            }
            "movefolder" => col.choosing_folder = true,
            "deletefolder" => {
                let draft = serde_json::json!({"folder":key.trim_start_matches("preview-folder:")});
                actions.push(format!("tagfolder\x1fdelete\x1f{draft}"));
            }
            "tagisolate" => actions.push(col.styles.isolate_action(n)),
            "lockitem" if n.local_placement => actions.push(format!("rowlock\x1f{key}")),
            "kind" => col.kind_menu = Some((key.clone(), at)),
            _ if id.starts_with("destination:") => {
                if let Some(payload) = drawing::DrawingDrag::begin(&col.styles, n) {
                    actions.push(payload.action(&id["destination:".len()..]));
                }
            }
            _ => actions.push(format!("{id}\x1f{key}")),
        }
        if id != "movefolder" {
            col.row_menu = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &str, name: &str, role: i64, depth: i64, dim: i64) -> WireRow {
        (
            [
                key,
                name,
                "Group\u{1e}Properties",
                "",
                "group",
                "drawing cell",
            ]
            .map(str::to_owned)
            .to_vec(),
            vec![depth, 0, 0, 0, role, 1, 0, 0, 0, 0, 0, dim],
        )
    }

    #[test]
    fn retained_rows_follow_their_occurrences_through_insert_remove_and_edit_context() {
        let a = row("i:a", "Wall", 1, 0, 0);
        let b = row("i:b", "Wall", 1, 0, 0);
        let previous = reconcile(vec![], vec![a.clone(), b.clone()]);
        let retained_name = previous[1].name.as_ptr();
        let retained_cell = previous[1].cells[0].as_ptr();
        let context = row("i:b", "Wall", 7, 1, 0);
        let mut dimmed = b.clone();
        dimmed.1[11] = 1;
        let next = reconcile(previous, vec![context, dimmed]);
        assert_eq!(next[1].name.as_ptr(), retained_name);
        assert_eq!(next[1].cells[0].as_ptr(), retained_cell);
        assert!(next[1].dim && !next[0].dim);
        let restored = reconcile(next, vec![a, b]);
        assert_eq!(restored[1].cells[0].as_ptr(), retained_cell);
        assert!(!restored[1].dim);
    }
}
