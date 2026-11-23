// SPDX-License-Identifier: GPL-3.0-or-later
//! A legacy geometry ID can describe several assemblies. Match the ordered
//! child definitions and their local placements before sharing a layout.
use super::{lock, Data, Doc, FastMap, FastSet, Field, Key, ListId, Trash};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct Assembly {
    // A slot ID survives reordering. Occurrences map it to their own
    // placement keys (and thus their persisted UIDs, tags and records).
    root: Key,
    slots: Vec<Key>,
    occurrences: FastMap<Key, FastMap<Key, Key>>,
}

fn linked(doc: &Doc, key: Key, ext: u16) -> bool {
    matches!(doc.node(key).and_then(|n| n.attr(ext)), Some(super::Value::Dict(pairs))
        if pairs.iter().any(|(k, _)| matches!(k, super::Value::Str(name) if name == "link")))
}

fn root_of(doc: &Doc, mut key: Key) -> Option<Key> {
    let mut seen = FastSet::default();
    while seen.insert(key) {
        let n = doc.node(key)?;
        if n.kind == super::Kind::Root {
            return Some(key);
        }
        key = *n.owners.first()?;
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
    let existing = node.assembly.clone().filter(|layout| layout.lock().unwrap().root == root);
    let reverse: FastMap<Key, Key> = existing.as_ref().and_then(|a| {
        let layout = a.lock().unwrap();
        Some(layout.occurrences.get(&owner)?.iter().map(|(&slot, &child)| (child, slot)).collect())
    }).unwrap_or_default();
    let slots: Vec<Key> = node.children.iter().map(|k| *reverse.get(k).unwrap_or(k)).collect();
    let mut occurrences: FastMap<Key, FastMap<Key, Key>> = FastMap::default();
    let mut stack = doc.node(root)?.children.clone();
    let mut seen = FastSet::default();
    while let Some(k) = stack.pop() {
        if !seen.insert(k) {
            continue;
        }
        let n = doc.node(k)?;
        if linked(doc, k, ext) {
            continue;
        }
        if matches!(&n.data, Data::Placement(p) if p.xform.is_some() && p.definition == data.definition)
            && equivalent(doc, owner, k, ext)
        {
            occurrences.insert(
                k,
                slots
                    .iter()
                    .copied()
                    .zip(n.children.iter().copied())
                    .collect(),
            );
        }
        stack.extend_from_slice(&n.children);
    }
    if occurrences.len() < 2 || !occurrences.contains_key(&owner) {
        return None;
    }
    let parents: Vec<_> = occurrences.keys().copied().collect();
    let assembly = existing.unwrap_or_default();
    {
        let mut layout = assembly.lock().unwrap();
        layout.root = root;
        layout.slots = slots;
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
    let (assembly, children, slots, peers) = {
        let doc = lock();
        let Some(n) = doc.node(owner) else {
            return Ok(());
        };
        let Some(shared) = n.assembly.as_ref() else {
            return Ok(());
        };
        let layout = shared.lock().unwrap();
        if root_of(&doc, owner) != Some(layout.root) {
            return Ok(());
        }
        let Some(projection) = layout.occurrences.get(&owner) else {
            return Ok(());
        };
        let reverse: FastMap<_, _> = projection.iter().map(|(&s, &k)| (k, s)).collect();
        let slots: Vec<_> = n
            .children
            .iter()
            .map(|k| *reverse.get(k).unwrap_or(k))
            .collect();
        let children: Vec<_> = n
            .children
            .iter()
            .map(|&k| {
                doc.handle_ref(py, k)
                    .ok_or_else(|| PyValueError::new_err("detached assembly child"))
            })
            .collect::<PyResult<_>>()?;
        let mut peers: Vec<_> = layout
            .occurrences
            .iter()
            .filter(|(k, _)| **k != owner)
            .filter_map(|(&k, map)| {
                let peer = doc.node(k)?;
                // An unlinked, made-unique, deleted or scoped instance can
                // outlive this session in a History step.
                if root_of(&doc, k) != Some(layout.root)
                    || !Arc::ptr_eq(peer.assembly.as_ref()?, shared)
                    || peer.get(Field::Definition).ok()? != n.get(Field::Definition).ok()?
                {
                    return None;
                }
                Some((k, map.clone()))
            })
            .collect();
        peers.sort_unstable_by_key(|(key, _)| *key);
        (shared.clone(), children, slots, peers)
    };
    let mut bindings = Vec::new();
    for (parent, mut map) in peers {
        let mut created = Vec::new();
        let mut keys = Vec::new();
        for (&slot, child) in slots.iter().zip(&children) {
            if !map
                .get(&slot)
                .is_some_and(|&key| lock().node(key).is_some())
            {
                let g = py.import("core.group")?.getattr("copy_group")?.call1((
                    child,
                    py.None(),
                    true,
                ))?;
                g.setattr("mesh", child.bind(py).getattr("mesh")?)?;
                let k = g.getattr("_doc_key")?.extract()?;
                map.insert(slot, k);
                created.push(g);
            }
            keys.push(map[&slot]);
        }
        {
            let mut doc = lock();
            for g in &created {
                let k = g.cast::<super::py::PyPlacement>()?.get().key;
                doc.set_handle(k, g.as_ptr());
            }
            for (src, &dst) in children.iter().zip(&keys) {
                let src = src.bind(py).cast::<super::py::PyPlacement>()?.get().key;
                for field in [
                    Field::Definition,
                    Field::Xform,
                    Field::Axes,
                    Field::Component,
                ] {
                    let value = doc
                        .need(src)
                        .and_then(|n| n.get(field))
                        .map_err(PyValueError::new_err)?;
                    if doc
                        .need(dst)
                        .and_then(|n| n.get(field))
                        .map_err(PyValueError::new_err)?
                        != value
                    {
                        doc.write(py, dst, field, value, &mut trash)
                            .map_err(PyValueError::new_err)?;
                    }
                }
            }
            if doc.need(parent).map_err(PyValueError::new_err)?.children != keys {
                doc.list_replace(py, parent, ListId::Children, keys, &mut trash)
                    .map_err(PyValueError::new_err)?;
            }
        }
        bindings.push((parent, map));
    }
    let mut layout = assembly.lock().unwrap();
    layout.slots.clone_from(&slots);
    let projection = layout.occurrences.get_mut(&owner).unwrap();
    for (slot, child) in slots.into_iter().zip(children.iter()) {
        projection.insert(
            slot,
            child.bind(py).cast::<super::py::PyPlacement>()?.get().key,
        );
    }
    layout.occurrences.extend(bindings);
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
        let slots = doc.node(parents[0]).unwrap().children.clone();
        let occurrences = parents
            .iter()
            .map(|&k| {
                (
                    k,
                    slots
                        .iter()
                        .copied()
                        .zip(doc.node(k).unwrap().children.iter().copied())
                        .collect(),
                )
            })
            .collect();
        let shared = Arc::new(Mutex::new(Assembly {
            root,
            slots,
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
    if doc.replaying || doc.projecting || doc.dirty_assemblies.contains(&owner) {
        return;
    }
    if !doc.node(owner).is_some_and(|n| !n.owners.is_empty()
        && matches!(&n.data, Data::Placement(p) if p.component && p.xform.is_some())) {
        return;
    }
    if let Some(root) = root_of(doc, owner) {
        bind(doc, root, owner);
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

pub fn finish(py: Python<'_>) -> PyResult<()> {
    let idle = { lock().depth() == 0 };
    if idle {
        drain(py)?;
    }
    Ok(())
}
