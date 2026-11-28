// SPDX-License-Identifier: GPL-3.0-or-later
//! Present the existing filing/linked-file membership index as one paged catalogue.
use crate::project_tree::{
    project_item_matches, project_name_matches, row, source_placement_rows, Row,
};
use pyo3::prelude::*;
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    path::Path,
};

type Obj<'py> = Bound<'py, PyAny>;
fn text(o: &Obj<'_>) -> PyResult<String> {
    Ok(o.extract::<Option<String>>()?.unwrap_or_default())
}
fn id(o: &Obj<'_>) -> usize {
    o.as_ptr() as usize
}
fn natural(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    while let (Some(x), Some(y)) = (a.first(), b.first()) {
        if x.is_ascii_digit() && y.is_ascii_digit() {
            let na = a.iter().take_while(|c| c.is_ascii_digit()).count();
            let nb = b.iter().take_while(|c| c.is_ascii_digit()).count();
            let za = a[..na].iter().take_while(|&&c| c == b'0').count();
            let zb = b[..nb].iter().take_while(|&&c| c == b'0').count();
            let order = (na - za).cmp(&(nb - zb)).then(a[za..na].cmp(&b[zb..nb]));
            if !order.is_eq() {
                return order;
            }
            a = &a[na..];
            b = &b[nb..];
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

// Count occurrences in this document only. A definition can also be held by
// undo, a component library, or a different open document.
#[derive(Default)]
struct PlacementIndex {
    repeated: HashSet<String>,
    copies: HashMap<usize, usize>,
}

impl PlacementIndex {
    fn read(scene: &Obj<'_>, reject_links: bool) -> PyResult<Option<Self>> {
        use crate::doc::{self, value::Value, Data};
        let root = doc::py::handle_key(&scene.getattr("_root")?)?;
        let ext = doc::attr_id("ext").map_err(pyo3::exceptions::PyValueError::new_err)?;
        let doc = doc::lock();
        let Some(root) = doc.node(root) else {
            return Ok(None);
        };
        let mut stack = root.children.clone();
        let mut index = Self::default();
        let mut seen = HashSet::new();
        let mut uids = HashSet::new();
        while let Some(key) = stack.pop() {
            if !seen.insert(key) {
                continue;
            }
            let Some(node) = doc.node(key) else {
                return Ok(None);
            };
            let Data::Placement(p) = &node.data else {
                return Ok(None);
            };
            if reject_links
                && matches!(node.attr(ext), Some(Value::Dict(fields)) if fields.iter().any(
                    |(k, _)| matches!(k, Value::Str(s) if s == "link")
                ))
            {
                return Ok(None);
            }
            if reject_links {
                if let Some(uid) = &p.uid {
                    if !uids.insert(uid.as_str()) {
                        index.repeated.insert(uid.clone());
                    }
                }
            }
            if p.component && p.xform.is_some() {
                if let Some(def) = &p.definition {
                    let mesh = def.0.get().geometry.as_ptr() as usize;
                    *index.copies.entry(mesh).or_default() += 1;
                }
            }
            stack.extend(&node.children);
        }
        Ok(Some(index))
    }
}

/// An unstructured document files its root placements in the unfiled branch.
/// An inserted linked descendant needs the full mounted-file projection.
#[pyfunction]
pub(crate) fn refresh_unfiled(scene: &Obj<'_>, tree: &Obj<'_>) -> PyResult<bool> {
    if tree.getattr("nodes")?.is_truthy()? {
        return Ok(false);
    }
    let Some(index) = PlacementIndex::read(scene, true)? else {
        return Ok(false);
    };
    let groups = scene.getattr("groups")?.extract::<Vec<Obj<'_>>>()?;
    tree.setattr("unfiled", groups)?;
    tree.setattr("_qualified", index.repeated)?;
    tree.setattr("_filing_copies", index.copies)?;
    tree.getattr("__dict__")?
        .call_method1("pop", ("_sidebar_scope", scene.py().None()))?;
    Ok(true)
}

struct Catalogue<'py> {
    junctions: HashMap<String, Vec<(String, String, String)>>,
    cuts: HashMap<String, Vec<(String, String, bool)>>,
    repeats: HashMap<String, String>,
    scene: Obj<'py>,
    tree: Obj<'py>,
    nodes: Obj<'py>,
    member: Obj<'py>,
    expanded: HashSet<String>,
    pages: HashMap<String, usize>,
    cap: usize,
    discover: bool,
    selected: HashSet<usize>,
    bad: HashSet<usize>,
    unfiled: HashSet<usize>,
    filter: String,
    matching_branches: HashSet<usize>,
    providers: Option<Obj<'py>>,
}
impl<'py> Catalogue<'py> {
    fn cap(&self, key: &str) -> usize {
        if self.discover {
            usize::MAX
        } else {
            self.pages.get(key).copied().unwrap_or(self.cap)
        }
    }
    fn kind(&self, g: &Obj<'py>) -> PyResult<&'static str> {
        Ok(
            if crate::model::links::is_linked(g)? {
                "link"
            } else if g.call_method0("is_component")?.is_truthy()? {
                "component"
            } else {
                "group"
            },
        )
    }
    fn extension_kind(&self, group: &Obj<'py>) -> PyResult<(String, String)> {
        let Some(providers) = &self.providers else { return Ok(Default::default()) };
        let description = group.py().import("built_core.outliner")?.call_method1("custom_kind", (group, providers))?;
        if description.is_none() { return Ok(Default::default()); }
        let kind = description.get_item("type")?.extract()?;
        let properties: Vec<(String, String)> = description.get_item("props")?.extract()?;
        Ok((kind, properties.into_iter().map(|(k,v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n")))
    }

    fn label_kind(&self, group: &Obj<'py>, kind: &str) -> PyResult<String> {
        let (label, tip) = self.extension_kind(group)?;
        Ok(if label.is_empty() {
            match kind { "component" => "Component", "link" => "Reference", _ => "Group" }.into()
        } else if tip.is_empty() { label } else { format!("{label}\u{1e}{tip}") })
    }

    /// Include the way to each match. A closed group is still searchable, and
    /// opening a search result never changes the user's saved disclosure state.
    fn filter_branches(&self) -> PyResult<HashSet<usize>> {
        let walk = self
            .scene
            .py()
            .import("built_core.outliner")?
            .call_method1("walk", (self.scene.getattr("groups")?,))?;
        let mut path: Vec<(usize, bool, bool)> = Vec::new();
        let query = self.filter.to_lowercase();
        let visibility = query.contains("is:hidden") || query.contains("is:visible");
        let locks = query.contains("is:locked");
        let mut rows = Vec::new();
        let mut hits = HashSet::new();
        for entry in walk.try_iter()? {
            let entry = entry?;
            let group = entry.get_item(0)?;
            let depth: usize = entry.get_item(2)?.extract()?;
            path.truncate(depth);
            let parent = path.last().copied();
            let hidden = parent.is_some_and(|p| p.1);
            let locked = parent.is_some_and(|p| p.2);
            let key = id(&group);
            if crate::project_tree::item_matches(
                &self.scene,
                &group,
                self.kind(&group)?,
                &self.extension_kind(&group)?.0.to_lowercase(),
                &self.filter,
                self.bad.contains(&key) || self.unfiled.contains(&key),
                hidden,
                locked,
            )? {
                hits.insert(key);
            }
            rows.push((key, parent.map(|p| p.0)));
            path.push((
                key,
                hidden
                    || (visibility
                        && !self
                            .scene
                            .call_method1("entity_visible", (&group,))?
                            .is_truthy()?),
                locked
                    || (locks
                        && (self
                            .scene
                            .call_method1("entity_locked", (&group,))?
                            .is_truthy()?
                            || self
                                .scene
                                .call_method1("_layer_state", (&group,))?
                                .get_item(1)?
                                .is_truthy()?)),
            ));
        }
        for (key, parent) in rows.into_iter().rev() {
            if hits.contains(&key) {
                if let Some(parent) = parent {
                    hits.insert(parent);
                }
            }
        }
        Ok(hits)
    }
    fn more(&self, rows: &mut Vec<Row>, key: &str, depth: usize, left: usize) {
        rows.push(row(
            format!("m:{key}"),
            format!("Show {} more ({left} not shown)", left.min(self.cap)),
            "",
            "",
            depth,
            4,
            0,
            0,
        ));
    }
    fn item(
        &self,
        rows: &mut Vec<Row>,
        g: &Obj<'py>,
        node: &str,
        d: usize,
        nested: bool,
    ) -> PyResult<()> {
        let key: String = self.tree.call_method1("row_id", (g,))?.extract()?;
        let name = text(&g.getattr("name")?)?;
        let name = if name.is_empty() {
            "Group".into()
        } else {
            name
        };
        let selected = usize::from(self.selected.contains(&id(g)));
        let kind = self.kind(g)?;
        if nested {
            if let Some(k) = self
                .member
                .call_method1("get", (id(g),))?
                .extract::<Option<String>>()?
            {
                if k != node && self.nodes.contains(&k)? {
                    let mut jump = row(format!("g:{key}"), name, "", kind, d, 3, 0, selected);
                    jump.1[5] = i64::from(!g.getattr("hidden")?.is_truthy()?);
                    jump.0[3] = format!("→ {}", text(&self.nodes.get_item(k)?.get_item("name")?)?);
                    rows.push(jump);
                    return Ok(());
                }
            }
        }
        let key = format!("{}:{key}", if nested { "c" } else { "i" });
        let kids = g.getattr("children")?;
        let open = !self.filter.is_empty() || self.expanded.contains(&key);
        let mut r = row(
            key.clone(),
            name,
            "",
            kind,
            d,
            if nested { 2 } else { 1 },
            0,
            selected,
        );
        r.0[2] = self.label_kind(g, kind)?;
        if kind == "link" {
            r.0[3] = "⛓".into();
        } else if kind == "component" {
            let copies = match self.tree.getattr_opt("_filing_copies")? {
                Some(cached) => cached,
                None => {
                    let counts = PlacementIndex::read(&self.scene, false)?
                        .unwrap_or_default()
                        .copies;
                    let cached = counts.into_pyobject(self.scene.py())?;
                    self.tree.setattr("_filing_copies", &cached)?;
                    cached.into_any()
                }
            };
            let count: usize = copies
                .call_method1("get", (id(&g.getattr("mesh")?), 0))?
                .extract()?;
            if count > 1 {
                r.0[3] = format!("×{count}");
            }
        }
        r.1[2] = self.bad.contains(&id(g)) as i64;
        crate::project_tree::placement_controls(&mut r, g)?;
        if !self.repeats.is_empty() {
            if let Some(label) = self.repeats.get(&g.getattr("uid")?.extract::<String>()?) {
                r.0[2] = label.clone();
                r.0[3] = format!("×{}", kids.len()?);
                rows.push(r);
                return Ok(());
            }
        }
        let cuts = if self.cuts.is_empty() {
            None
        } else {
            self.cuts.get(&g.getattr("uid")?.extract::<String>()?)
        };
        let junctions = if self.junctions.is_empty() {
            None
        } else {
            self.junctions.get(&g.getattr("uid")?.extract::<String>()?)
        };
        r.1[7] = (kids.is_truthy()? || cuts.is_some() || junctions.is_some()) as i64;
        r.1[8] = open as i64;
        rows.push(r);
        if open {
            for (path, name, state) in junctions.into_iter().flatten() {
                rows.push(row(
                    format!("g:{path}"),
                    name.clone(),
                    state,
                    "group",
                    d + 1,
                    3,
                    0,
                    0,
                ));
            }
            for (id, label, on) in cuts.into_iter().flatten() {
                rows.push(crate::project_tree::cut_row(id, label, *on, d + 1));
            }
            let max = self.cap(&key);
            if self.filter.is_empty() {
                for child in kids.try_iter()?.take(max) {
                    self.item(rows, &child?, node, d + 1, true)?;
                }
                if kids.len()? > max {
                    self.more(rows, &key, d + 1, kids.len()? - max);
                }
            } else {
                let mut count = 0;
                for child in kids.try_iter()? {
                    let child = child?;
                    if self.matching_branches.contains(&id(&child)) {
                        if count < max {
                            self.item(rows, &child, node, d + 1, true)?;
                        }
                        count += 1;
                    }
                }
                if count > max {
                    self.more(rows, &key, d + 1, count - max);
                }
            }
        }
        Ok(())
    }
    fn items(
        &self,
        rows: &mut Vec<Row>,
        groups: Vec<Obj<'py>>,
        node: &str,
        d: usize,
    ) -> PyResult<()> {
        let mut entries: Vec<(String, Vec<Obj<'py>>)> = Vec::new();
        let mut definitions = HashMap::<(String, usize), usize>::new();
        for g in groups {
            let name = text(&g.getattr("name")?)?;
            if g.call_method0("is_component")?.is_truthy()?
                && !crate::model::links::is_linked(&g)?
            {
                let identity = (name.clone(), id(&g.getattr("mesh")?));
                if let Some(&index) = definitions.get(&identity) {
                    entries[index].1.push(g);
                    continue;
                }
                definitions.insert(identity, entries.len());
            }
            entries.push((name, vec![g]));
        }
        let mut entries: Vec<_> = entries
            .into_iter()
            .map(|(name, groups)| (name.to_lowercase(), name, groups))
            .collect();
        entries.sort_by(|a, b| natural(&a.0, &b.0));
        let total = entries.len();
        let max = self.cap(node);
        for (_, name, mut gs) in entries.into_iter().take(max) {
            if gs.len() == 1 {
                self.item(rows, &gs.pop().unwrap(), node, d, false)?;
                continue;
            }
            let key = format!("d:{node}:{}", id(&gs[0].getattr("mesh")?));
            let selected = gs.iter().filter(|g| self.selected.contains(&id(g))).count();
            let mut r = row(
                key.clone(),
                if name.is_empty() {
                    "Component".into()
                } else {
                    name
                },
                "",
                "component",
                d,
                1,
                0,
                selected,
            );
            r.1[5] = i64::from(!gs[0].getattr("hidden")?.is_truthy()?);
            r.0[2] = self.label_kind(&gs[0], "component")?;
            r.0[3] = format!("×{}", gs.len());
            r.1[2] = gs.iter().filter(|g| self.bad.contains(&id(g))).count() as i64;
            let open = self.expanded.contains(&key);
            r.1[7] = 1;
            r.1[8] = open as i64;
            rows.push(r);
            if open || !self.filter.is_empty() {
                let max = self.cap(&key);
                for g in gs.iter().take(max) {
                    self.item(rows, g, node, d + 1, false)?;
                }
                if gs.len() > max {
                    self.more(rows, &key, d + 1, gs.len() - max);
                }
            }
        }
        if total > max {
            self.more(rows, node, d, total - max);
        }
        Ok(())
    }
}

fn layer<'py>(scene: &Obj<'py>, tag: &Obj<'py>) -> PyResult<Obj<'py>> {
    let Some(name) = tag.extract::<Option<String>>()? else {
        return Ok(scene.py().None().into_bound(scene.py()));
    };
    let mut ly = scene.call_method1("layer", (&name,))?;
    if ly.is_none() {
        if let Some((_, tail)) = name.rsplit_once('/') {
            ly = scene.call_method1("layer", (tail,))?;
        }
    }
    Ok(ly)
}
fn controls<'py>(
    c: &Catalogue<'py>,
    model: &Obj<'py>,
    key: &str,
    node: &Obj<'py>,
    drawing: bool,
) -> PyResult<(i64, i64, Vec<String>)> {
    let py = c.scene.py();
    let layers = py.import("built_core.layers")?;
    let kind: String = node.get_item("kind")?.extract()?;
    if kind == "link" {
        let folders = node.get_item("folders")?;
        let mut eye = true;
        for f in folders.try_iter()? {
            eye &= crate::model::module(layers.py(), "built_core")?.getattr("tag_edits")?
                .call_method1("folder_visible", (&c.scene, f?))?
                .is_truthy()?;
        }
        let tags = c.tree.call_method1("file_tags", (key,))?;
        let mut locked = tags.is_truthy()?;
        for tag in tags.try_iter()? {
            locked &= c
                .scene
                .call_method1("layer", (tag?,))?
                .getattr("locked")?
                .is_truthy()?;
        }
        return Ok((eye as i64, locked as i64, vec![]));
    }
    let main = if kind == "host" {
        layer(
            &c.scene,
            &node.get_item("recipe")?.call_method1("get", ("tag",))?,
        )?
    } else {
        layer(&c.scene, &node.get_item("tag")?)?
    };
    let styles = if drawing && !main.is_none() {
        main.call_method0("_drawing_cells")?.extract()?
    } else {
        Vec::new()
    };
    let mut ly = main;
    if ly.is_none() {
        let node_layers = model.get_item("node_layers")?;
        for k in c.tree.call_method1("subtree", (key,))?.try_iter()? {
            let k = k?;
            let tags = node_layers.get_item(&k)?;
            if let Some(t) = tags.try_iter()?.next() {
                ly = layer(&c.scene, &t?)?;
            }
            if ly.is_none() {
                let child = c.nodes.get_item(&k)?;
                ly = layer(
                    &c.scene,
                    &child.get_item("recipe")?.call_method1("get", ("tag",))?,
                )?;
            }
            if !ly.is_none() {
                break;
            }
        }
    }
    let lock = if ly.is_none() {
        -1
    } else {
        ly.getattr("locked")?.is_truthy()? as i64
    };
    let eye = if kind == "tagfolder" {
        crate::model::module(layers.py(), "built_core")?.getattr("tag_edits")?
            .call_method1("folder_visible", (&c.scene, node.get_item("folder")?))?
            .is_truthy()? as i64
    } else if ly.is_none() {
        -1
    } else {
        crate::model::module(layers.py(), "built_core")?.getattr("tag_edits")?
            .call_method1("effective_visible", (&c.scene, &ly))?
            .is_truthy()? as i64
    };
    Ok((eye, lock, styles))
}

#[pyfunction]
#[pyo3(signature = (scene, model, expanded, pages, filter, discover, cap, lens, selected, active, kinds, tips, states, providers=None))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn filing_rows<'py>(
    scene: Obj<'py>,
    model: Obj<'py>,
    expanded: HashSet<String>,
    pages: HashMap<String, usize>,
    filter: String,
    discover: bool,
    cap: usize,
    lens: u8,
    selected: HashSet<usize>,
    active: Option<String>,
    kinds: Obj<'py>,
    tips: HashMap<String, String>,
    states: HashMap<String, String>,
    providers: Option<Obj<'py>>,
) -> PyResult<Vec<Row>> {
    let tree = model.get_item("tree")?;
    let nodes = tree.getattr("nodes")?;
    let direct = tree.getattr("direct")?;
    let unfiled: Vec<Obj<'py>> = tree.getattr("unfiled")?.extract()?;
    let order: Vec<(String, usize)> = model.get_item("order")?.extract()?;
    let cls_name = |cls: &Obj<'py>| -> PyResult<String> {
        if let Some(s) = kinds
            .call_method1("get", (cls,))?
            .extract::<Option<String>>()?
        {
            return Ok(s);
        }
        Ok(text(cls)?.trim_start_matches("Ifc").to_string())
    };
    let mut c = Catalogue {
        junctions: crate::live::junction::labels(&scene)?,
        cuts: crate::live::cut_labels(&scene)?,
        repeats: crate::live::repeat_labels(&scene)?,
        scene,
        tree: tree.clone(),
        nodes: nodes.clone(),
        member: tree.getattr("member")?,
        expanded,
        pages,
        cap,
        discover,
        selected,
        unfiled: unfiled.iter().map(id).collect(),
        bad: model
            .get_item("to_fix")?
            .extract::<Vec<Obj<'py>>>()?
            .iter()
            .map(id)
            .collect(),
        filter: filter.trim().to_owned(),
        matching_branches: HashSet::new(),
        providers: providers.filter(|p| p.is_truthy().unwrap_or(false)),
    };
    let mut selected = HashMap::<String, usize>::new();
    for &g in &c.selected {
        if let Some(k) = c
            .member
            .call_method1("get", (g,))?
            .extract::<Option<String>>()?
        {
            *selected.entry(k).or_default() += 1;
        }
    }
    for (key, _) in order.iter().rev() {
        if let Some(&n) = selected.get(key) {
            if let Some(parent) = nodes
                .get_item(key)?
                .get_item("parent")?
                .extract::<Option<String>>()?
            {
                *selected.entry(parent).or_default() += n;
            }
        }
    }
    let mounted = tree.getattr("links")?;
    let copies = |key: &str| -> PyResult<Vec<Obj<'py>>> {
        Ok(mounted
            .call_method1("get", (key,))?
            .extract::<Option<Vec<Obj<'py>>>>()?
            .unwrap_or_default())
    };
    let filtering = !c.filter.trim().is_empty();
    let words: Vec<String> = c
        .filter
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let mut visible = HashSet::new();
    let mut members = HashMap::new();
    let mut matching_files = HashSet::new();
    if filtering {
        c.matching_branches = c.filter_branches()?;
        for (key, _) in &order {
            let node = nodes.get_item(key)?;
            let cls = node.get_item("recipe")?.call_method1("get", ("class",))?;
            let named = !words.iter().any(|w| w.contains(':'))
                && project_name_matches(
                    &text(&node.get_item("name")?)?,
                    &cls_name(&cls)?,
                    words.clone(),
                );
            let mut hits = Vec::new();
            for g in direct.get_item(key)?.try_iter()? {
                let g = g?;
                if c.matching_branches.contains(&id(&g)) {
                    hits.push(g);
                }
            }
            if text(&node.get_item("kind")?)? == "link" {
                let state = crate::model::links::link_state(&c.scene, &node.get_item("path")?)?;
                for g in copies(key)? {
                    if project_item_matches(
                        &c.scene,
                        &g,
                        "link",
                        &c.filter,
                        matches!(state.as_str(), "missing" | "modified"),
                    )? {
                        matching_files.insert(key.clone());
                        break;
                    }
                }
            }
            if named || !hits.is_empty() || matching_files.contains(key) {
                let mut k = Some(key.clone());
                while let Some(current) = k {
                    if !visible.insert(current.clone()) {
                        break;
                    }
                    k = nodes.get_item(&current)?.get_item("parent")?.extract()?;
                }
                members.insert(key.clone(), hits);
            }
        }
    }
    let mut rows = Vec::new();
    let mut folded = None;
    let counts = model.get_item("counts")?;
    let problems = model.get_item("problems")?;
    let children = tree.getattr("children")?;
    for (key, depth) in order {
        if filtering && !visible.contains(&key) || folded.is_some_and(|d| depth > d) {
            continue;
        }
        let node = nodes.get_item(&key)?;
        let kind = text(&node.get_item("kind")?)?;
        let kids = children.get_item(&key)?.is_truthy()?;
        let path = text(&node.get_item("path")?)?;
        let mut name = text(&node.get_item("name")?)?;
        let mut icon = String::new();
        let mut badge = String::new();
        let cls = node.get_item("recipe")?.call_method1("get", ("class",))?;
        let label = if kind == "link" {
            let state = crate::model::links::link_state(&c.scene, &pyo3::types::PyString::new(c.scene.py(), &path))?;
            let via = node
                .call_method1("get", ("via",))?
                .extract::<Option<String>>()?;
            if let Some(v) = via {
                badge = format!("via {v}");
            }
            name = Path::new(&path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&name)
                .to_owned();
            icon = format!("file:{state}");
            let mut tip = format!(
                "\x1e{}\n\n{}",
                tips["Reference"].replace("{path}", &path),
                states.get(&state).unwrap_or(&state)
            );
            if let Some(merged) = node
                .call_method1("get", ("merged",))?
                .extract::<Option<String>>()?
            {
                tip.push_str(&format!(
                    "\n\nIts one top node «{merged}» is shown as this row."
                ));
            }
            tip
        } else {
            let mut label = if kind == "tagfolder" || cls.is_none() && kids {
                String::new()
            } else {
                cls_name(&cls)?
            };
            let mut help = tips.get(&label).cloned().unwrap_or_default();
            if !path.is_empty() {
                if !help.is_empty() {
                    help.push_str("\n\n");
                }
                help.push_str(&format!(
                    "From {} — read-only here.",
                    Path::new(&path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                ));
            }
            if !help.is_empty() {
                label.push('\x1e');
                label.push_str(&help);
            }
            label
        };
        let rk = format!("n:{key}");
        let open = filtering || c.expanded.contains(&rk);
        let placed = copies(&key)?;
        let groups = direct.get_item(&key)?;
        let mut r = row(
            rk,
            name,
            &label,
            &icon,
            depth,
            0,
            counts.get_item(&key)?.extract()?,
            selected.get(&key).copied().unwrap_or(0),
        );
        let (eye, lock, style) = controls(&c, &model, &key, &node, lens == 1)?;
        r.0[3] = badge;
        r.0.extend(style);
        r.1[2] = problems.get_item(&key)?.extract()?;
        r.1[3] = (Some(&key) == active.as_ref()) as i64;
        r.1[5] = eye;
        r.1[6] = lock;
        r.1[7] = (kids || groups.is_truthy()? || placed.len() > 1) as i64;
        r.1[8] = open as i64;
        rows.push(r);
        folded = (!open).then_some(depth);
        if open {
            if kind == "link" && (!filtering || matching_files.contains(&key)) {
                rows.extend(source_placement_rows(&c.scene, placed, depth + 1)?);
            }
            let items = if filtering {
                members.remove(&key).unwrap_or_default()
            } else {
                groups.extract()?
            };
            c.items(&mut rows, items, &key, depth + 1)?;
        }
    }
    let mut loose = Vec::new();
    for g in unfiled {
        if !filtering || c.matching_branches.contains(&id(&g)) {
            loose.push(g);
        }
    }
    if !loose.is_empty() {
        let open = filtering || c.expanded.contains("n:");
        let mut r = row(
            "n:".into(),
            "Not filed".into(),
            "",
            "",
            0,
            0,
            loose.len(),
            loose.iter().filter(|g| c.selected.contains(&id(g))).count(),
        );
        r.1[5] = i64::from(!loose[0].getattr("hidden")?.is_truthy()?);
        r.1[2] = loose.len() as i64;
        r.1[7] = 1;
        r.1[8] = open as i64;
        rows.push(r);
        if open {
            c.items(&mut rows, loose, "", 1)?;
        }
    }
    // The project Junctions view (docs/junctions.md §11).
    if !filtering {
        for (key, name, label, kind, depth, count, expandable) in crate::live::junction::view_rows(&c.scene, &c.expanded)? {
            let mut r = row(key.clone(), name, kind, "", depth, 0, count, 0);
            r.0[2] = label;
            r.1[7] = i64::from(expandable);
            r.1[8] = i64::from(c.expanded.contains(&key));
            rows.push(r);
        }
    }
    Ok(rows)
}

/// Attribute selection to the deepest displayed occurrence or its paged
/// parent. An ancestor of a selected row gets the quiet partial mark.
#[pyfunction]
pub(crate) fn mark_filing_selection(mut rows: Vec<Row>) -> Vec<Row> {
    let mut below = HashMap::<i64, i64>::new();
    for (_, flags) in rows.iter_mut().rev() {
        if flags.len() < 11 {
            flags.resize(11, 0);
        }
        let depth = flags[0];
        let children = below.remove(&(depth + 1)).unwrap_or(0);
        below.retain(|&level, _| level <= depth);
        let count = flags[9].max(0);
        let placement = matches!(flags[4], 1..=3 | 7 | 10);
        flags[10] = i64::from(if placement {
            count > 0
        } else {
            count > children
        });
        *below.entry(depth).or_default() += if placement {
            count + children
        } else {
            count.max(children)
        };
    }
    let mut parents = Vec::<usize>::new();
    for i in 0..rows.len() {
        let depth = rows[i].1[0];
        while parents.last().is_some_and(|&j| rows[j].1[0] >= depth) {
            parents.pop();
        }
        if rows[i].1[10] == 1 {
            for &j in &parents {
                if rows[j].1[10] == 0 {
                    rows[j].1[10] = 2;
                }
            }
        }
        parents.push(i);
    }
    rows
}
