// SPDX-License-Identifier: GPL-3.0-or-later
//! Reconcile the rows supplied by the document with the visible Browser nodes.
use super::{kit, Node};
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
