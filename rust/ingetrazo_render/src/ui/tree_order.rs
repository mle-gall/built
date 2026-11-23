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
}

impl Sort {
    pub(super) const OPTIONS: [(Self, &'static str); 4] = [
        (Self::Project, "Project order"),
        (Self::Name, "Name"),
        (Self::Visible, "Visible first"),
        (Self::Locked, "Locked first"),
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
                    Self::Visible => y.eye.cmp(&x.eye),
                    Self::Locked => y.lock.cmp(&x.lock),
                    _ => Ordering::Equal,
                };
                state.then_with(|| natural(&names[a], &names[b]))
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

fn category(row: &Node) -> u8 {
    if row.icon.starts_with("file:") {
        3
    } else if row.key == "n:" || row.row == 4 {
        4
    } else if row.icon == "folder" || row.key.starts_with("n:folder:") {
        0
    } else if matches!(row.row, 0 | 6) {
        1
    } else {
        2
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
