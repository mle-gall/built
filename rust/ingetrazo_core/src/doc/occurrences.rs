// SPDX-License-Identifier: GPL-3.0-or-later
//! Isolating occurrences from the placement tree, without the Python facet
//! index (`core/facets.py build`): the tree's own walk gives each occurrence
//! its key and ancestors, which is all `core.agent_facets.set_display`
//! needs for an "occurrence in [...]" filter.
//!
//! The keys are `core.facets.build`'s: a placement's UID, or, for a UID that
//! repeats in the tree or starts with `@[`, the address `link_structure.row_id`
//! gives it ("@" and the JSON list of UIDs from the root). The walk is the same
//! pre-order over every path, so a shared definition is walked under each of
//! its placements.
use std::collections::HashSet;

/// What isolating a set of occurrence keys shows.
#[derive(Debug, Default, PartialEq)]
pub struct Closure<K> {
    /// The wanted keys present in the tree, in walk order.
    pub matched: Vec<String>,
    /// Shown: the wanted occurrences and (unless exact) everything inside them.
    pub allowed: HashSet<String>,
    /// Drawn through: ancestors of a shown occurrence that are not shown
    /// themselves (a linked file's placement above an isolated part).
    pub through: HashSet<String>,
    /// The placements whose key is an address, not their UID.
    pub qualified: Vec<(K, String)>,
}

/// `json.dumps(list_of_str, separators=(",", ":"))` with Python's default
/// `ensure_ascii`, so the address matches `link_structure.row_id`.
pub fn row_id(uids: &[&str]) -> String {
    let mut out = String::from("@[");
    for (i, uid) in uids.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('"');
        for c in uid.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\u{8}' => out.push_str("\\b"),
                '\u{c}' => out.push_str("\\f"),
                c if (c as u32) < 0x20 || (c as u32) > 0x7f => {
                    let mut units = [0u16; 2];
                    for u in c.encode_utf16(&mut units) {
                        out.push_str(&format!("\\u{u:04x}"));
                    }
                }
                c => out.push(c),
            }
        }
        out.push('"');
    }
    out.push(']');
    out
}

/// Walk `roots` (pre-order, every path). `node(k)` gives a placement's UID
/// and its child placements, or None for a node that is not a placement.
pub fn closure<K: Copy>(
    roots: &[K],
    node: &impl Fn(K) -> Option<(String, Vec<K>)>,
    repeated: &HashSet<String>,
    wanted: &HashSet<String>,
    exact: bool,
) -> Closure<K> {
    let mut out = Closure { matched: Vec::new(), allowed: HashSet::new(), through: HashSet::new(), qualified: Vec::new() };
    let mut seen = HashSet::new();
    let mut near = HashSet::new();
    let mut uids: Vec<String> = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    walk(roots, node, repeated, wanted, exact, false, &mut uids, &mut keys, &mut seen, &mut near, &mut out);
    out.through = near.difference(&out.allowed).cloned().collect();
    out
}

#[allow(clippy::too_many_arguments)]
fn walk<K: Copy>(
    level: &[K],
    node: &impl Fn(K) -> Option<(String, Vec<K>)>,
    repeated: &HashSet<String>,
    wanted: &HashSet<String>,
    exact: bool,
    inside: bool,
    uids: &mut Vec<String>,
    keys: &mut Vec<String>,
    seen: &mut HashSet<String>,
    near: &mut HashSet<String>,
    out: &mut Closure<K>,
) {
    for &k in level {
        let Some((uid, children)) = node(k) else { continue };
        let key = if repeated.contains(&uid) || uid.starts_with("@[") {
            let path: Vec<&str> = uids.iter().map(String::as_str).chain([uid.as_str()]).collect();
            let key = row_id(&path);
            out.qualified.push((k, key.clone()));
            key
        } else {
            uid.clone()
        };
        let own = wanted.contains(&key);
        if own && seen.insert(key.clone()) {
            out.matched.push(key.clone());
        }
        let shown = own || (!exact && inside);
        if shown {
            out.allowed.insert(key.clone());
            near.extend(keys.iter().cloned());
        }
        uids.push(uid);
        keys.push(key);
        walk(&children, node, repeated, wanted, exact, inside || own, uids, keys, seen, near, out);
        uids.pop();
        keys.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// host ─ A (link) ─ rail, tie ; B ; a repeated "x" under A and B.
    fn tree(k: usize) -> Option<(String, Vec<usize>)> {
        let (uid, kids): (&str, &[usize]) = match k {
            0 => ("A", &[1, 2, 5]),
            1 => ("rail", &[]),
            2 => ("tie", &[]),
            3 => ("B", &[4]),
            4 => ("x", &[]),
            5 => ("x", &[]),
            _ => return None,
        };
        Some((uid.into(), kids.to_vec()))
    }

    fn set(keys: &[&str]) -> HashSet<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    #[test]
    fn a_link_shows_its_parts_and_a_part_draws_through_its_link() {
        let repeated = set(&["x"]);
        let c = closure(&[0, 3], &tree, &repeated, &set(&["A"]), false);
        assert_eq!(c.matched, ["A"]);
        assert_eq!(c.allowed, set(&["A", "rail", "tie", r#"@["A","x"]"#]));
        assert!(c.through.is_empty());
        let c = closure(&[0, 3], &tree, &repeated, &set(&["rail"]), false);
        assert_eq!((c.allowed, c.through), (set(&["rail"]), set(&["A"])));
        // Exact: the occurrence alone.
        let c = closure(&[0, 3], &tree, &repeated, &set(&["A"]), true);
        assert_eq!(c.allowed, set(&["A"]));
        assert_eq!(c.qualified.iter().map(|(k, _)| *k).collect::<Vec<_>>(), [5, 4]);
    }

    #[test]
    fn addresses_match_pythons_json() {
        assert_eq!(row_id(&["A", "x"]), r#"@["A","x"]"#);
        assert_eq!(row_id(&["L\u{e1}mina \"1\"", "\u{1f600}"]), "@[\"L\\u00e1mina \\\"1\\\"\",\"\\ud83d\\ude00\"]");
    }
}
