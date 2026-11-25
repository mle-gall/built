// SPDX-License-Identifier: GPL-3.0-or-later
//! Display-only mounted file rows, derived from each linked file's current structure.
use super::Obj;
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};
use std::collections::{HashMap, HashSet};
fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
fn get<'py>(v: &Obj<'py>, field: &str) -> PyResult<Obj<'py>> {
    v.call_method1("get", (field,))
}
fn fallback<'py>(v: Obj<'py>, other: Obj<'py>) -> PyResult<Obj<'py>> {
    Ok(if v.is_truthy()? { v } else { other })
}
fn own<'py>(table: &Obj<'py>, linked: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(table.py());
    for (k, n) in table.cast::<PyDict>()?.iter() {
        if n.is_instance_of::<PyDict>()
            && !n.contains("source")?
            && !k.str()?.to_string_lossy().starts_with("link:")
        {
            out.set_item(k, n)?;
        }
    }
    let mut parents = HashSet::new();
    for (_, n) in out.iter() {
        if let Ok(p) = get(&n, "parent")?.extract::<String>() {
            parents.insert(p);
        }
    }
    for (k, n) in out.copy()?.iter() {
        if get(&n, "name")?.eq(linked)? && !parents.contains(&k.extract::<String>()?) {
            out.del_item(k)?;
        }
    }
    Ok(out)
}
fn subtable<'py>(table: &Obj<'py>, stem: &str) -> PyResult<Bound<'py, PyDict>> {
    let py = table.py();
    let pre = format!("link:{stem}/");
    let out = PyDict::new(py);
    for (k, n) in table.cast::<PyDict>()?.iter() {
        let Ok(k) = k.extract::<String>() else {
            continue;
        };
        let Some(rel) = k.strip_prefix(&pre) else {
            continue;
        };
        let Ok(n) = n.cast::<PyDict>() else { continue };
        let v = n.copy()?;
        for key in ["source", "source_node", "mount"] {
            if v.contains(key)? {
                v.del_item(key)?;
            }
        }
        let parent = get(n.as_any(), "parent")?.extract::<String>().ok();
        v.set_item(
            "parent",
            parent.as_deref().and_then(|p| p.strip_prefix(&pre)),
        )?;
        if !v.contains("recipe")? {
            if let Some(r) = n.get_item("source_recipe")? {
                v.set_item("recipe", r)?;
            }
        }
        out.set_item(rel, v)?;
    }
    Ok(out)
}
fn node<'py>(
    name: &Obj<'py>,
    parent: &Obj<'py>,
    order: &Obj<'py>,
    kind: &str,
    path: &Obj<'py>,
    stems: &Obj<'py>,
    recipe: &Obj<'py>,
    tag: &Obj<'py>,
    folder: &Obj<'py>,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(name.py());
    d.set_item("kind", kind)?;
    for (k, v) in [
        ("name", name),
        ("parent", parent),
        ("order", order),
        ("path", path),
        ("stems", stems),
        ("recipe", recipe),
        ("tag", tag),
        ("folder", folder),
    ] {
        d.set_item(k, v)?;
    }
    Ok(d)
}
pub(super) fn host(tree: &Obj<'_>, registry: &Obj<'_>) -> PyResult<()> {
    let py = tree.py();
    let structure = crate::model::module(py, "core.structure")?;
    let linked = crate::model::module(py, "core.links")?.getattr("LINKED_FOLDER")?;
    let own = own(&structure.call_method1("_nodes", (registry,))?, &linked)?;
    let nodes = tree.getattr("nodes")?;
    let nil = none(py);
    let stems = PyTuple::empty(py);
    let mut top: Option<Obj> = None;
    for (k, n) in own.iter() {
        let p = get(&n, "parent")?;
        let p = if own.contains(&p)? { p } else { nil.clone() };
        let order = n.call_method1("get", ("order", 0))?;
        if p.is_none()
            && match &top {
                None => true,
                Some(v) => order.gt(v)?,
            }
        {
            top = Some(order.clone())
        }
        nodes.set_item(
            &k,
            node(
                &n.get_item("name")?,
                &p,
                &order,
                "host",
                &nil,
                stems.as_any(),
                &structure.call_method1("resolve", (registry, &k))?,
                &nil,
                &nil,
            )?,
        )?;
    }
    tree.setattr(
        "_host_roots",
        top.unwrap_or((-1i64).into_pyobject(py)?.into_any()),
    )
}
pub(super) fn same(tree: &Obj<'_>, stems: &[String], first: &[String]) -> PyResult<()> {
    let py = tree.py();
    let mk = super::build::mount(stems);
    let canon = super::build::mount(first);
    let sep = crate::model::module(py, "core.links")?
        .getattr("SEP")?
        .extract::<String>()?;
    tree.getattr("alias")?.set_item(mk, &canon)?;
    let tables = tree.getattr("tables")?;
    tables.set_item(
        PyTuple::new(py, stems)?,
        tables.call_method1("get", (PyTuple::new(py, first)?, PyDict::new(py)))?,
    )?;
    let folders = tree
        .getattr("nodes")?
        .get_item(canon)?
        .get_item("folders")?;
    let base = stems.join(&sep);
    if !folders.contains(&base)? {
        folders.call_method1("append", (base,))?;
    }
    Ok(())
}
pub(super) fn file(tree: &Obj<'_>, s: &[String], path: &Obj<'_>) -> PyResult<()> {
    let py = tree.py();
    let scene = tree.getattr("scene")?;
    let links = crate::model::module(py, "core.links")?;
    let sep = links.getattr("SEP")?.extract::<String>()?;
    let linked = links.getattr("LINKED_FOLDER")?;
    let base = s.join(&sep);
    let key = super::build::mount(s);
    let nil = none(py);
    let nodes = tree.getattr("nodes")?;
    let aliases = tree.getattr("alias")?;
    let stems = PyTuple::new(py, s)?;
    let last = s.last().unwrap();
    let basename = base.as_str().into_pyobject(py)?.into_any();
    let row = node(
        last.into_pyobject(py)?.as_any(),
        &nil,
        nodes.len()?.into_pyobject(py)?.as_any(),
        "link",
        path,
        stems.as_any(),
        PyDict::new(py).as_any(),
        &nil,
        &basename,
    )?;
    row.set_item("via", s.len().checked_sub(2).map(|i| &s[i]))?;
    row.set_item("folders", vec![&base])?;
    row.set_item("merged", &nil)?;
    nodes.set_item(&key, &row)?;
    let tables = tree.getattr("tables")?;
    let table = if s.len() == 1 {
        let entry = scene
            .getattr("link_registry")?
            .call_method1("get", (path,))?;
        if entry.is_truthy()? {
            fallback(get(&entry, "structure")?, PyDict::new(py).into_any())?
        } else {
            PyDict::new(py).into_any()
        }
    } else {
        subtable(
            &tables.call_method1(
                "get",
                (PyTuple::new(py, &s[..s.len() - 1])?, PyDict::new(py)),
            )?,
            last,
        )?
        .into_any()
    };
    tables.set_item(&stems, &table)?;
    let own = own(&table, &linked)?;
    if own.is_empty() {
        return tags(
            tree,
            &stems,
            path,
            &key,
            &base,
            &sep,
            &linked.extract::<String>()?,
        );
    }
    let mut tops = vec![];
    for (k, n) in own.iter() {
        if !own.contains(get(&n, "parent")?)? {
            tops.push(k)
        }
    }
    let single = tops.first().filter(|_| tops.len() == 1);
    for (k, n) in own.iter() {
        let recipe = fallback(get(&n, "recipe")?, get(&n, "source_recipe")?)?;
        let recipe = if recipe.is_truthy()? {
            py.get_type::<PyDict>().call1((recipe,))?
        } else {
            PyDict::new(py).into_any()
        };
        let tag = get(&recipe, "tag")?;
        let tag = if tag.is_truthy()? {
            let tag = format!("{base}{sep}{}", tag.extract::<String>()?);
            if scene.call_method1("layer_by_key", (&tag,))?.is_none() {
                nil.clone()
            } else {
                tag.into_pyobject(py)?.into_any()
            }
        } else {
            nil.clone()
        };
        let name = fallback(get(&n, "name")?, k.clone())?;
        let display = format!("{key}/{}", k.extract::<String>()?);
        if single.is_some_and(|v| k.is(v)) {
            aliases.set_item(display, &key)?;
            row.set_item("merged", name)?;
            row.set_item("recipe", recipe)?;
            row.set_item("tag", tag)?;
            continue;
        }
        let parent = get(&n, "parent")?;
        let parent =
            if !own.contains(&parent)? || single.is_some_and(|k| k.eq(&parent).unwrap_or(false)) {
                key.clone()
            } else {
                format!("{key}/{}", parent.extract::<String>()?)
            };
        nodes.set_item(
            display,
            node(
                &name,
                parent.into_pyobject(py)?.as_any(),
                &n.call_method1("get", ("order", 0))?,
                "node",
                path,
                stems.as_any(),
                &recipe,
                &tag,
                &nil,
            )?,
        )?;
    }
    Ok(())
}
fn tags(
    tree: &Obj<'_>,
    stems: &Bound<'_, PyTuple>,
    path: &Obj<'_>,
    key: &str,
    base: &str,
    sep: &str,
    linked_name: &str,
) -> PyResult<()> {
    let py = tree.py();
    let scene = tree.getattr("scene")?;
    let prefix = format!("{base}{sep}");
    let nested = format!("{prefix}{linked_name}");
    let mut dirs = vec![];
    let mut found = HashSet::new();
    let parents = scene
        .getattr_opt("folder_parents")?
        .filter(|d| !d.is_none())
        .unwrap_or_else(|| PyDict::new(py).into_any());
    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    for (c, p) in parents.cast::<PyDict>()?.iter() {
        if let Ok(p) = p.extract::<String>() {
            children.entry(p).or_default().push(c.extract()?);
        }
    }
    let mut stack = vec![base.to_owned()];
    while let Some(folder) = stack.pop() {
        for c in children.get(&folder).into_iter().flatten() {
            if *c != nested && found.insert(c.clone()) {
                dirs.push((c.clone(), folder.clone()));
                stack.push(c.clone());
            }
        }
    }
    let tops = dirs
        .iter()
        .filter(|(_, p)| p == base)
        .map(|(c, _)| c)
        .collect::<Vec<_>>();
    let layers = scene.getattr("layers")?;
    let mut mine = vec![];
    let mut other = false;
    let untagged = format!("{prefix}Untagged");
    for (i, ly) in layers.try_iter()?.enumerate() {
        let ly = ly?;
        let k = ly.getattr("key")?.extract::<String>()?;
        if !k.starts_with(&prefix) {
            continue;
        }
        let Some(folder) = ly.getattr("folder")?.extract::<Option<String>>()? else {
            continue;
        };
        if folder == base || found.contains(&folder) {
            other |= folder == base && k != untagged;
            mine.push((i, ly, k, folder));
        }
    }
    let single = tops
        .first()
        .filter(|_| tops.len() == 1 && !other)
        .map(|v| v.as_str());
    let under = |p: &str| {
        if p == base || Some(p) == single {
            key.to_owned()
        } else {
            format!("{key}/tf:{p}")
        }
    };
    let nil = none(py);
    let nodes = tree.getattr("nodes")?;
    let tag_edits = crate::model::module(py, "ingetrazo_core")?.getattr("tag_edits")?;
    for (i, (f, p)) in dirs.iter().enumerate() {
        let label = tag_edits.call_method1("folder_label", (&scene, f))?;
        let label = if label.is_truthy()? {
            label.extract::<String>()?
        } else {
            f.clone()
        };
        let label = label.strip_prefix(&prefix).unwrap_or(&label);
        let display = format!("{key}/tf:{f}");
        if Some(f.as_str()) == single {
            tree.getattr("alias")?.set_item(display, key)?;
            nodes.get_item(key)?.set_item("merged", label)?;
            continue;
        }
        nodes.set_item(
            display,
            node(
                label.into_pyobject(py)?.as_any(),
                under(p).into_pyobject(py)?.as_any(),
                i.into_pyobject(py)?.as_any(),
                "tagfolder",
                path,
                stems.as_any(),
                PyDict::new(py).as_any(),
                &nil,
                f.into_pyobject(py)?.as_any(),
            )?,
        )?;
    }
    for (i, ly, k, f) in mine {
        let name = ly.getattr("name")?;
        let name = if name.is_truthy()? {
            name.extract::<String>()?
        } else {
            k.clone()
        };
        let name = name.strip_prefix(&prefix).unwrap_or(&name);
        let recipe = PyDict::new(py);
        recipe.set_item("tag", &k)?;
        nodes.set_item(
            format!("{key}/tag:{k}"),
            node(
                name.into_pyobject(py)?.as_any(),
                under(&f).into_pyobject(py)?.as_any(),
                (1000 + i).into_pyobject(py)?.as_any(),
                "tag",
                path,
                stems.as_any(),
                recipe.as_any(),
                k.into_pyobject(py)?.as_any(),
                &nil,
            )?,
        )?;
    }
    Ok(())
}
