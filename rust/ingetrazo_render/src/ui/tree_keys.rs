//! Keys for the focused occurrence or section row in the Project tree.
use super::{Column, Lens};

impl Column {
    pub(super) fn claims_row_key(&self, name: &str, mods: u8) -> bool {
        let Some(key) = self.focused_row.as_ref() else {
            return false;
        };
        (name == "Escape" || self.nodes.iter().any(|n| &n.key == key))
            && (self.lens == Lens::Filing
                || (self.lens == Lens::Drawing
                    && key.starts_with("preview-tag:")
                    && matches!(name, "Delete" | "Backspace")))
            && self.mode.is_empty()
            && self.renaming.is_none()
            && self.row_menu.is_none()
            && self.prompt.is_none()
            && (mods == 0 || (name == "Tab" && mods == 4))
            && matches!(
                name,
                "ArrowUp"
                    | "ArrowDown"
                    | "Space"
                    | "Escape"
                    | "Enter"
                    | "F2"
                    | "Tab"
                    | "Delete"
                    | "Backspace"
            )
    }

    pub(super) fn row_key(&mut self, name: &str, mods: u8) -> Option<String> {
        let key = self.focused_row.as_ref()?;
        if name == "Escape" {
            return Some("tree:stepout\u{1f}".into());
        }
        let pos = self
            .ordered
            .iter()
            .position(|&i| &self.nodes[i].key == key)?;
        let node = &self.nodes[self.ordered[pos]];
        let plane = key.starts_with("section:");
        match name {
            "Delete" | "Backspace" if self.lens == Lens::Drawing => {
                Some(format!("tree:deletetag\u{1f}{key}"))
            }
            "Delete" | "Backspace" if plane || node.local_placement => {
                Some(format!("tree:deleterows\u{1f}{key}"))
            }
            "Enter" | "Space" => {
                let verb = match (name, plane) {
                    ("Enter", true) => "sectioncut",
                    ("Enter", false) => "open",
                    (_, true) => "eye",
                    (_, false) => "roweye",
                };
                Some(format!("tree:{verb}\u{1f}{key}"))
            }
            "Tab" if node.local_placement => {
                if mods == 4 {
                    return Some(format!("tree:outdent\u{1f}{key}"));
                }
                let prev = self.ordered[..pos]
                    .iter()
                    .rev()
                    .map(|&i| &self.nodes[i])
                    .find(|n| n.depth <= node.depth)?;
                (prev.depth == node.depth && prev.local_placement)
                    .then(|| format!("tree:into\u{1f}{key}\u{1f}{}", prev.key))
            }
            "F2" => {
                self.renaming = Some((key.clone(), node.name.clone()));
                self.focus_rename = true;
                None
            }
            "ArrowUp" | "ArrowDown" => {
                let eligible = |&i: &&usize| {
                    self.nodes[*i].local_placement || self.nodes[*i].key.starts_with("section:")
                };
                let next = if name == "ArrowUp" {
                    self.ordered[..pos].iter().rev().find(eligible)
                } else {
                    self.ordered[pos + 1..].iter().find(eligible)
                }?;
                let next = &self.nodes[*next];
                self.reveal = Some(next.key.clone());
                self.focused_row = Some(next.key.clone());
                self.highlighted = Some(next.key.clone());
                let verb = if next.local_placement {
                    "pickrow"
                } else {
                    "item"
                };
                Some(format!("tree:{verb}\u{1f}{}", next.key))
            }
            _ => None,
        }
    }
}
