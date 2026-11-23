// SPDX-License-Identifier: GPL-3.0-or-later
//! A legacy geometry ID can describe several assemblies. Match the ordered
//! child definitions and their local placements before sharing a layout.
use super::{lock, Data, Doc, FVal, FastMap, FastSet, Field, Key, ListId, Trash};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use std::sync::{Arc, Mutex};

// Slot IDs remain stable when a member is moved to a different parent.
// Only the member's definition and placement belong to the assembly.
// UIDs, records, tags and visibility live on its projected occurrence.
#[derive(Clone)]
struct Member {
    slot: Key,
    source: Key,
    children: Vec<Key>,
    geometry: Option<u64>,
    fields: [(Field, FVal); 3],
}

#[derive(Default)]
pub struct Assembly {
    root: Key,
    geometry: u64,
    children: Vec<Key>,
    members: Vec<Member>,
    occurrences: FastMap<Key, FastMap<Key, Key>>,
}

fn topology(doc: &Doc, owner: Key, reverse: &FastMap<Key, Key>) -> (Vec<Key>, Vec<Member>) {
    let slot = |k: Key| *reverse.get(&k).unwrap_or(&k);
    let children = &doc.node(owner).unwrap().children;
    let mut stack = children.clone();
    let mut members = Vec::new();
    let mut seen = FastSet::default();
    while let Some(key) = stack.pop() {
        if !seen.insert(key) {
            continue;
        }
        let n = doc.node(key).expect("attached member");
        let fields = [Field::Xform, Field::Axes, Field::Component]
            .map(|field| (field, n.get(field).expect("placement field")));
        let Data::Placement(p) = &n.data else {
            continue;
        };
        members.push(Member {
            geometry: p.definition.as_ref().map(|def| def.0.get().key),
            slot: slot(key),
            source: key,
            children: n.children.iter().copied().map(slot).collect(),
            fields,
        });
        stack.extend(&n.children);
    }
    (children.iter().copied().map(slot).collect(), members)
}

fn map_tree(
    doc: &Doc,
    original: Key,
    occurrence: Key,
    reverse: &FastMap<Key, Key>,
    projection: &mut FastMap<Key, Key>,
) {
    let mut stack = vec![(original, occurrence)];
    let mut seen = FastSet::default();
    while let Some((src, dst)) = stack.pop() {
        if !seen.insert(src) {
            continue;
        }
        projection
            .entry(*reverse.get(&src).unwrap_or(&src))
            .or_insert(dst);
        stack.extend(
            doc.node(src)
                .unwrap()
                .children
                .iter()
                .copied()
                .zip(doc.node(dst).unwrap().children.iter().copied()),
        );
    }
}

fn linked(doc: &Doc, key: Key, ext: u16) -> bool {
    matches!(doc.node(key).and_then(|n| n.attr(ext)), Some(super::Value::Dict(pairs))
        if pairs.iter().any(|(k, _)| matches!(k, super::Value::Str(name) if name == "link")))
}

pub(super) fn root_of(doc: &Doc, mut key: Key, exclude_links: bool) -> Option<Key> {
    let ext = super::attr_id("ext").ok()?;
    for _ in 0..doc.slots.len() {
        if exclude_links && linked(doc, key, ext) {
            return None;
        }
        let parent = *doc.node(key)?.owners.first()?;
        let n = doc.node(parent)?;
        if n.kind == super::Kind::Root {
            return (!n.kept.contains(&key)).then_some(parent);
        }
        key = parent;
    }
    None
}

fn equivalent(doc: &Doc, a: Key, b: Key, ext: u16) -> bool {
    let mut stack = vec![(a, b)];
    let mut seen = FastSet::default();
    while let Some((left, right)) = stack.pop() {
        if !seen.insert((left, right)) {
            continue;
        }
        let (Some(l), Some(r)) = (doc.node(left), doc.node(right)) else {
            return false;
        };
        let (Data::Placement(x), Data::Placement(y)) = (&l.data, &r.data) else {
            return false;
        };
        if (left != right && (linked(doc, left, ext) || linked(doc, right, ext)))
            || x.definition != y.definition
            || l.children.len() != r.children.len()
            || x.component != y.component
            || (left != a && (x.xform != y.xform || x.axes != y.axes))
        {
            return false;
        }
        stack.extend(l.children.iter().copied().zip(r.children.iter().copied()));
    }
    true
}

pub fn bind(doc: &mut Doc, root: Key, owner: Key) -> Option<Arc<Mutex<Assembly>>> {
    let node = doc.node(owner)?;
    let Data::Placement(data) = &node.data else {
        return None;
    };
    if data.xform.is_none() || !data.component {
        return None;
    }
    let ext = super::attr_id("ext").ok()?;
    let geometry = data.definition.as_ref()?.0.get().key;
    let existing = node.assembly.clone().filter(|shared| {
        let layout = shared.lock().unwrap();
        layout.root == root && layout.geometry == geometry
    });
    let reverse: FastMap<Key, Key> = existing
        .as_ref()
        .and_then(|a| {
            let layout = a.lock().unwrap();
            Some(
                layout
                    .occurrences
                    .get(&owner)?
                    .iter()
                    .map(|(&slot, &child)| (child, slot))
                    .collect(),
            )
        })
        .unwrap_or_default();

    let mut occurrences: FastMap<Key, FastMap<Key, Key>> = FastMap::default();
    let placed = data.definition.as_ref()?.0.get().placements.lock().unwrap();
    for &k in placed.iter() {
        let n = doc.node(k)?;
        if root_of(doc, k, true) != Some(root) {
            continue;
        }
        if matches!(&n.data, Data::Placement(p) if p.xform.is_some() && p.definition == data.definition)
            && equivalent(doc, owner, k, ext)
        {
            let mut projection = FastMap::default();
            for (&src, &dst) in node.children.iter().zip(&n.children) {
                map_tree(doc, src, dst, &reverse, &mut projection);
            }
            occurrences.insert(k, projection);
        }
    }
    drop(placed);
    if occurrences.len() < 2 || !occurrences.contains_key(&owner) {
        return None;
    }
    let parents: Vec<_> = occurrences.keys().copied().collect();
    let assembly = existing.unwrap_or_default();
    {
        let mut layout = assembly.lock().unwrap();
        layout.root = root;
        layout.geometry = geometry;
        (layout.children, layout.members) = topology(doc, owner, &reverse);
        for (parent, map) in occurrences {
            layout.occurrences.entry(parent).or_default().extend(map);
        }
    }
    for parent in parents {
        doc.node_mut(parent)?.assembly = Some(assembly.clone());
    }
    Some(assembly)
}

/// Reconcile the authored child's definition and local placement through
/// each occurrence's existing handle. The receiving occurrence keeps all
/// of its own attributes. List changes use the ordinary document journal.
pub fn sync(py: Python<'_>, owner: Key) -> PyResult<()> {
    let mut trash = Trash::default();
    let (assembly, children, members, peers) = {
        let doc = lock();
        let Some(n) = doc.node(owner) else {
            return Ok(());
        };
        let Some(shared) = n.assembly.as_ref() else {
            return Ok(());
        };
        let mut layout = shared.lock().unwrap();
        if root_of(&doc, owner, true) != Some(layout.root) {
            return Ok(());
        }
        let Some(projection) = layout.occurrences.get(&owner) else {
            return Ok(());
        };
        let reverse: FastMap<_, _> = projection.iter().map(|(&s, &k)| (k, s)).collect();
        (layout.children, layout.members) = topology(&doc, owner, &reverse);
        let mut peers: Vec<_> = layout
            .occurrences
            .iter()
            .filter(|(k, _)| **k != owner)
            .filter_map(|(&k, map)| {
                let peer = doc.node(k)?;
                // An unlinked, made-unique, deleted or scoped instance can
                // outlive this session in a History step.
                if root_of(&doc, k, true) != Some(layout.root)
                    || !Arc::ptr_eq(peer.assembly.as_ref()?, shared)
                    || peer.get(Field::Definition).ok()? != n.get(Field::Definition).ok()?
                {
                    return None;
                }
                Some((k, map.clone()))
            })
            .collect();
        peers.sort_unstable_by_key(|(key, _)| *key);
        (
            shared.clone(),
            layout.children.clone(),
            layout.members.clone(),
            peers,
        )
    };
    let reverse = members
        .iter()
        .map(|member| (member.source, member.slot))
        .collect();
    let mut bindings = Vec::new();
    for (parent, mut map) in peers {
        // Pin new branches until their parent lists own them.
        let mut created = Vec::new();
        for member in &members {
            if map
                .get(&member.slot)
                .is_some_and(|&k| lock().node(k).is_some())
            {
                continue;
            }
            let child = lock()
                .handle_ref(py, member.source)
                .ok_or_else(|| PyValueError::new_err("detached assembly member"))?;
            let g = py.import("core.group")?.getattr("copy_group")?.call1((
                &child,
                py.None(),
                true,
                py.None(),
                true,
            ))?;
            let key = g.cast::<super::py::PyPlacement>()?.get().key;
            {
                let mut doc = lock();
                doc.set_handle(key, g.as_ptr());
                map_tree(&doc, member.source, key, &reverse, &mut map);
            }
            created.push(g);
        }
        let mut doc = lock();
        for member in &members {
            let dst = map[&member.slot];
            let Data::Placement(p) = &doc.need(dst).map_err(PyValueError::new_err)?.data else {
                continue;
            };
            if p.definition.as_ref().map(|def| def.0.get().key) != member.geometry {
                let value = doc
                    .need(member.source)
                    .and_then(|n| n.get(Field::Definition))
                    .map_err(PyValueError::new_err)?;
                doc.write(py, dst, Field::Definition, value, &mut trash)
                    .map_err(PyValueError::new_err)?;
            }
            for (field, value) in &member.fields {
                if doc
                    .need(dst)
                    .and_then(|n| n.get(*field))
                    .map_err(PyValueError::new_err)?
                    != *value
                {
                    doc.write(py, dst, *field, value.clone(), &mut trash)
                        .map_err(PyValueError::new_err)?;
                }
            }
            replace_children(py, &mut doc, dst, &member.children, &map, &mut trash)?;
        }
        replace_children(py, &mut doc, parent, &children, &map, &mut trash)?;
        bindings.push((parent, map));
    }
    let mut layout = assembly.lock().unwrap();
    layout
        .occurrences
        .get_mut(&owner)
        .unwrap()
        .extend(members.iter().map(|m| (m.slot, m.source)));
    layout.occurrences.extend(bindings);
    Ok(())
}

fn replace_children(
    py: Python<'_>,
    doc: &mut Doc,
    parent: Key,
    slots: &[Key],
    map: &FastMap<Key, Key>,
    trash: &mut Trash,
) -> PyResult<()> {
    let keys: Vec<_> = slots.iter().map(|k| map[k]).collect();
    if doc.need(parent).map_err(PyValueError::new_err)?.children != keys {
        for &k in &keys {
            if doc.node(k).unwrap().owners.iter().any(|&p| p != parent) {
                doc.detach(py, k, trash).map_err(PyValueError::new_err)?;
            }
        }
        doc.list_replace(py, parent, ListId::Children, keys, trash)
            .map_err(PyValueError::new_err)?;
    }
    Ok(())
}

/// Index persisted trees after loading, once, bottom-up. No occurrence
/// handles or Python wrappers are created by indexing.
pub fn index(doc: &mut Doc, root: Key) {
    let Some(n) = doc.node(root) else { return };
    let ext = super::attr_id("ext").expect("document attributes");
    let mut stack = n.children.clone();
    let mut order = Vec::new();
    let mut seen = FastSet::default();
    while let Some(k) = stack.pop() {
        if !seen.insert(k) || linked(doc, k, ext) {
            continue;
        }
        let Some(n) = doc.node(k) else { continue };
        stack.extend_from_slice(&n.children);
        order.push(k);
    }
    let mut shapes = FastMap::default();
    let mut shape_ids = FastMap::default();
    let mut groups: FastMap<usize, Vec<Key>> = FastMap::default();
    for k in order.into_iter().rev() {
        let n = doc.node(k).unwrap();
        let Data::Placement(p) = &n.data else {
            continue;
        };
        let mut parts = Vec::with_capacity(n.children.len());
        for &c in &n.children {
            let Some(child) = doc.node(c) else { continue };
            let Data::Placement(part) = &child.data else {
                continue;
            };
            let matrix = |x: &super::Xform| x.doubles().map(f64::to_bits);
            // A linked subtree has independent source identity.
            parts.push((
                shape_ids.get(&c).copied(),
                if linked(doc, c, ext) { c } else { 0 },
                part.xform.as_ref().map(matrix),
                part.axes.as_ref().map(matrix),
            ));
        }
        let next = shapes.len();
        let geometry = p.definition.as_ref().map(|d| d.0.as_ptr() as usize);
        let shape = *shapes.entry((geometry, p.component, parts)).or_insert(next);
        shape_ids.insert(k, shape);
        if p.component && p.xform.is_some() && !n.children.is_empty() {
            groups.entry(shape).or_default().push(k);
        }
    }
    for parents in groups.into_values().filter(|parents| parents.len() > 1) {
        let first = parents[0];
        let reverse = FastMap::default();
        let (children, members) = topology(doc, first, &reverse);
        let source = doc.node(first).unwrap();
        let Data::Placement(data) = &source.data else {
            continue;
        };
        let Some(geometry) = &data.definition else {
            continue;
        };
        let occurrences = parents
            .iter()
            .map(|&k| {
                let mut map = FastMap::default();
                for (&src, &dst) in source.children.iter().zip(&doc.node(k).unwrap().children) {
                    map_tree(doc, src, dst, &reverse, &mut map);
                }
                (k, map)
            })
            .collect();
        let shared = Arc::new(Mutex::new(Assembly {
            root,
            geometry: geometry.0.get().key,
            children,
            members,
            occurrences,
        }));
        for k in parents {
            doc.node_mut(k).unwrap().assembly = Some(shared.clone());
        }
    }
}

/// Bind before the first structural mutation. Newly created and pasted
/// occurrences also participate before they have been opened for editing.
pub fn prepare(doc: &mut Doc, owner: Key) {
    if doc.replaying || doc.projecting {
        return;
    }
    let mut current = owner;
    let mut has_layout = false;
    for _ in 0..doc.slots.len() {
        let Some(n) = doc.node(current) else { break };
        let Some(&parent) = n.owners.first() else {
            break;
        };
        if matches!(&n.data, Data::Placement(p) if p.component && p.xform.is_some())
            && !doc.dirty_assemblies.contains(&current)
        {
            if let Some(root) = root_of(doc, current, true) {
                has_layout |= bind(doc, root, current).is_some();
            }
        }
        current = parent;
    }
    if has_layout && doc.open.is_none() {
        let document = root_of(doc, owner, true);
        doc.implicit_assembly = doc.begin(document).is_some();
    }
}

pub fn drain(py: Python<'_>) -> PyResult<()> {
    let owners = {
        let mut doc = lock();
        if doc.projecting || doc.dirty_assemblies.is_empty() {
            return Ok(());
        }
        doc.projecting = true;
        std::mem::take(&mut doc.dirty_assemblies)
    };
    let result = owners.into_iter().try_for_each(|k| sync(py, k));
    lock().projecting = false;
    result
}

/// A raw list or placement setter has the same all-or-nothing semantics as a
/// command. Copying a new occurrence may call the Python IFC copy hook.
pub fn finish_result<T>(py: Python<'_>, result: PyResult<T>) -> PyResult<T> {
    let (implicit, idle) = {
        let doc = lock();
        if doc.projecting {
            return result;
        }
        (
            doc.implicit_assembly && doc.depth() == 1,
            doc.open.is_none(),
        )
    };
    if !implicit {
        if idle && result.is_ok() {
            drain(py)?;
        }
        return result;
    }
    let result = result.and_then(|value| drain(py).map(|_| value));
    let mut trash = Trash::default();
    let journal = {
        let mut doc = lock();
        doc.implicit_assembly = false;
        if result.is_ok() {
            doc.commit()
        } else {
            doc.rollback(py, &mut trash)
                .map_err(PyValueError::new_err)?;
            None
        }
    };
    drop(journal);
    drop(trash);
    result
}
