// SPDX-License-Identifier: GPL-3.0-or-later
//! View ordering for the Project tree. Keep each branch together and the
//! linked-file roots in their supplied order, including nested files "via X".
use super::Node;
use std::cmp::Ordering;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Sort {
    #[default]
    Project,
    Name,
    Visible,
    Locked,
    NameDescending,
}

impl Sort {
    pub(super) const OPTIONS: [(Self, &'static str); 5] = [
        (Self::Project, "Project order"),
        (Self::Name, "Name"),
        (Self::Visible, "Visible first"),
        (Self::Locked, "Locked first"),
        (Self::NameDescending, "Name (Z–A)"),
    ];

    pub(super) fn order(self, rows: &[Node]) -> Vec<usize> {
        if self == Self::Project {
            return (0..rows.len()).collect();
        }
        let root = rows.len();
        let mut children = vec![Vec::new(); root + 1];
        let mut ancestors = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            ancestors.truncate(row.depth);
            let parent = ancestors.last().copied().unwrap_or(root);
            children[parent].push(index);
            ancestors.push(index);
        }
        let names: Vec<_> = rows.iter().map(|row| row.name.to_lowercase()).collect();
        for siblings in &mut children {
            siblings.sort_by(|&a, &b| {
                let x = &rows[a];
                let y = &rows[b];
                let kind = category(x).cmp(&category(y));
                if kind != Ordering::Equal || x.icon.starts_with("file:") {
                    return kind;
                }
                let state = match self {
                    Self::Visible => (if y.eye == 2 { 0 } else { y.eye }).cmp(&(if x.eye == 2 { 0 } else { x.eye })),
                    Self::Locked => y.lock.min(1).cmp(&x.lock.min(1)),
                    _ => Ordering::Equal,
                };
                state.then_with(|| {
                    if self == Self::NameDescending {
                        natural(&names[b], &names[a])
                    } else {
                        natural(&names[a], &names[b])
                    }
                })
            });
        }
        let mut result = Vec::with_capacity(root);
        let mut stack = std::mem::take(&mut children[root]);
        stack.reverse();
        while let Some(index) = stack.pop() {
            result.push(index);
            stack.extend(children[index].iter().rev());
        }
        result
    }
}

/// ScrollArea virtualizes the rows. Target their sorted display index even
/// when the occurrence is too far away to have an egui response this frame.
pub(super) fn reveal_offset(
    rows: &[Node],
    order: &[usize],
    key: &str,
    pitch: f32,
    height: f32,
) -> Option<f32> {
    let index = order.iter().position(|&i| rows[i].key == key)?;
    Some((index as f32 * pitch - (height - pitch) * 0.5).max(0.0))
}

fn category(row: &Node) -> u8 {
    if row.key == "geometry:loose" {
        0
    } else if row.icon == "folder" || row.key.starts_with("n:folder:") {
        1
    } else if row.icon.starts_with("file:") {
        4
    } else if row.key == "n:" || row.row == 4 {
        5
    } else if matches!(row.row, 0 | 6) {
        2
    } else {
        3
    }
}

// Compare digit runs by magnitude without parsing them into a bounded integer.
fn natural(left: &str, right: &str) -> Ordering {
    let (mut a, mut b) = (left.as_bytes(), right.as_bytes());
    while let (Some(x), Some(y)) = (a.first(), b.first()) {
        if x.is_ascii_digit() && y.is_ascii_digit() {
            let an = a.iter().take_while(|c| c.is_ascii_digit()).count();
            let bn = b.iter().take_while(|c| c.is_ascii_digit()).count();
            let a0 = a[..an].iter().take_while(|&&c| c == b'0').count();
            let b0 = b[..bn].iter().take_while(|&&c| c == b'0').count();
            let order = (an - a0)
                .cmp(&(bn - b0))
                .then_with(|| a[a0..an].cmp(&b[b0..bn]));
            if order != Ordering::Equal {
                return order;
            }
            a = &a[an..];
            b = &b[bn..];
        } else {
            if x != y {
                return x.cmp(y);
            }
            a = &a[1..];
            b = &b[1..];
        }
    }
    a.len().cmp(&b.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(key: &str, name: &str, depth: usize, eye: i8, lock: i8) -> Node {
        Node {
            key: key.into(),
            name: name.into(),
            depth,
            eye,
            lock,
            ..Default::default()
        }
    }

    #[test]
    fn ten_thousand_rows_sort_naturally_without_reordering_the_model() {
        let rows: Vec<_> = (0..10_000).rev().map(|i|
            node(&format!("i:{i}"), &format!("Item {i}"), 0, 1, 0)).collect();
        let started = std::time::Instant::now();
        let order = Sort::Name.order(&rows);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        assert!(order.into_iter().eq((0..10_000).rev()));
        assert_eq!(rows[0].key, "i:9999");
    }

    #[test]
    fn equal_names_keep_their_document_order_within_each_parent() {
        let rows = [
            node("i:b", "Pieza", 0, 1, 0),
            node("i:c", "Casa", 0, 1, 0),
            node("c:d", "Hijo 10", 1, 1, 0),
            node("c:e", "Hijo 2", 1, 1, 0),
            node("i:a", "Pieza", 0, 1, 0),
        ];
        assert_eq!(Sort::Name.order(&rows), [1, 3, 2, 0, 4]);
    }

    #[test]
    fn sorting_keeps_the_default_geometry_first() {
        let rows = [
            node("preview-tag:Z", "zeta", 0, 1, 0),
            node("preview-tag:A", "alfa", 0, 1, 0),
            node("geometry:loose", "Loose geometry", 0, 1, 0),
        ];
        assert_eq!(Sort::Name.order(&rows), [2, 1, 0]);
        assert_eq!(Sort::NameDescending.order(&rows), [2, 0, 1]);
    }

    #[test]
    fn names_sort_with_their_numbers_read_as_numbers() {
        let rows =
            ["Capa 10", "capa 2", "Capa 1", "Alzado"].map(|label| node(label, label, 0, 1, 0));
        assert_eq!(Sort::Name.order(&rows), [3, 2, 1, 0]);
    }

    #[test]
    fn sort_by_order_created_keeps_new_siblings_at_the_end() {
        let mut rows =
            Vec::from(["Zeta", "Alfa", "Media"].map(|label| node(label, label, 0, 1, 0)));
        assert_eq!(Sort::Name.order(&rows), [1, 2, 0]);
        rows.push(node("new", "Layer 1", 0, 1, 0));
        assert_eq!(Sort::Project.order(&rows), [0, 1, 2, 3]);
    }

    #[test]
    fn sort_by_visible_or_locked_uses_natural_names_for_ties() {
        let rows = [
            node("w", "Capa 10", 0, 1, 0),
            node("x", "Capa 2", 0, 0, 1),
            node("y", "Capa 1", 0, 1, 0),
            node("z", "Alzado", 0, 0, 1),
        ];
        assert_eq!(Sort::Visible.order(&rows), [2, 0, 3, 1]);
        assert_eq!(Sort::Locked.order(&rows), [3, 1, 2, 0]);
    }

    #[test]
    fn sort_by_visible_moves_a_row_when_its_eye_changes() {
        let mut rows = ["A", "B", "C"].map(|label| node(label, label, 0, 1, 0));
        rows[0].eye = 0;
        assert_eq!(Sort::Visible.order(&rows), [1, 2, 0]);
        rows[0].eye = 1;
        assert_eq!(Sort::Visible.order(&rows), [0, 1, 2]);
        rows[2].lock = 1;
        assert_eq!(Sort::Locked.order(&rows), [2, 0, 1]);
    }

    #[test]
    fn sort_by_visible_keeps_folders_first_and_sorts_inside_them() {
        let mut rows = [
            node("preview-folder:H", "Apagada", 0, 0, -1),
            node("preview-folder:S", "Zona", 0, 1, -1),
            node("preview-tag:L", "Losa", 1, 0, 0),
            node("preview-tag:P", "Pilar", 1, 1, 0),
            node("preview-tag:A", "Suelta", 0, 0, 0),
            node("preview-tag:V", "Viga", 0, 1, 0),
        ];
        rows[0].icon = "folder".into();
        rows[1].icon = "folder".into();
        assert_eq!(Sort::Visible.order(&rows), [1, 3, 2, 0, 5, 4]);
        assert_eq!(Sort::Name.order(&rows), [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn natural_names_and_state_reorder_siblings_within_their_folders() {
        let mut rows = vec![
            node("n:folder:y", "Yard", 0, 1, 0),
            node("n:a", "Level 10", 1, 1, 0),
            node("n:b", "level 2", 1, 1, 1),
            node("n:c", "Floor", 1, 0, 0),
            node("n:folder:x", "Annex", 0, 0, 0),
        ];
        assert_eq!(Sort::Project.order(&rows), [0, 1, 2, 3, 4]);
        assert_eq!(Sort::Name.order(&rows), [4, 0, 3, 2, 1]);
        assert_eq!(Sort::Visible.order(&rows), [0, 2, 1, 3, 4]);
        assert_eq!(Sort::Locked.order(&rows), [4, 0, 2, 3, 1]);
        rows[2].eye = 0;
        assert_eq!(Sort::Visible.order(&rows), [0, 1, 3, 2, 4]);
    }

    #[test]
    fn reveal_scrolls_to_the_sorted_occurrence_outside_the_viewport() {
        let rows: Vec<_> = (0..100)
            .rev()
            .map(|n| node(&format!("i:{n}"), &format!("Door {n}"), 0, 1, 0))
            .collect();
        let order = Sort::Name.order(&rows);
        assert_eq!(
            reveal_offset(&rows, &order, "i:90", 24.0, 216.0),
            Some(2064.0)
        );
        assert_eq!(reveal_offset(&rows, &order, "i:0", 24.0, 216.0), Some(0.0));
        assert_eq!(reveal_offset(&rows, &order, "i:removed", 24.0, 216.0), None);
    }

    #[test]
    fn each_file_stays_next_to_its_nested_file_and_the_pager_stays_last() {
        let mut rows = vec![
            node("n:a", "Zeta", 0, 1, 0),
            node("m:a", "Show more", 1, -1, -1),
            node("i:t", "Tree", 1, 1, 0),
            node("n:link:z", "Z.igz", 0, 1, 0),
            node("n:link:z/link:a", "A.igz", 0, 1, 0),
        ];
        rows[1].row = 4;
        rows[2].row = 1;
        rows[3].icon = "file:resolved".into();
        rows[4].icon = "file:missing".into();
        assert_eq!(Sort::Name.order(&rows), [0, 2, 1, 3, 4]);
    }
}
