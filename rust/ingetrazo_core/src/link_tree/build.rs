// SPDX-License-Identifier: GPL-3.0-or-later
//! Build ordered display rows and assign each occurrence to its mounted file.
use super::{link_placements, Obj};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::HashMap;
fn stems(chain: &Obj<'_>) -> PyResult<Vec<String>> {
    chain
        .try_iter()?
        .map(|p| p?.get_item(0)?.extract())
        .collect()
}
pub(super) fn mount(stems: &[String]) -> String {
    format!("link:{}", stems.join("/link:"))
}
fn resolved<'py>(aliases: &Obj<'py>, mut key: Obj<'py>) -> PyResult<Obj<'py>> {
    for _ in 0..64 {
        let Some(v) = aliases.cast::<PyDict>()?.get_item(&key)? else {
            break;
        };
        key = v;
    }
    Ok(key)
}
#[pyfunction]
pub(super) fn build_project_tree<'py>(
    tree: &Obj<'py>,
    scene: &Obj<'py>,
    registry: &Obj<'py>,
) -> PyResult<()> {
    let py = scene.py();
    tree.setattr("scene", scene)?;
    let default = crate::model::module(py, "collections")?.getattr("defaultdict")?;
    for name in ["nodes", "member", "tables", "alias"] {
        tree.setattr(name, PyDict::new(py))?;
    }
    for name in ["direct", "links"] {
        tree.setattr(name, default.call1((py.get_type::<PyList>(),))?)?;
    }
    tree.setattr("unfiled", PyList::empty(py))?;
    super::mount::host(tree, registry)?;
    let rows = link_placements(scene)?;
    tree.setattr("_qualified", scene.call_method0("repeated_uids")?)?;
    let mut files = Vec::new();
    let mut rank = HashMap::new();
    for row in rows.iter() {
        let chain = row.get_item(1)?;
        let inner = row.get_item(2)?;
        if !inner.eq(chain)? {
            let s = stems(&inner)?;
            if !rank.contains_key(&s) {
                let path = inner.get_item(inner.len()? - 1)?.get_item(1)?;
                rank.insert(s.clone(), files.len());
                files.push((s, path));
            }
        }
    }
    files.sort_by_key(|(chain, _)| chain.len());
    let mut shown: HashMap<String, Vec<String>> = HashMap::new();
    for (s, path) in files {
        let p = path.extract::<String>()?;
        if let Some(first) = shown.get(&p) {
            super::mount::same(tree, &s, first)?;
        } else {
            super::mount::file(tree, &s, &path)?;
            shown.insert(p, s);
        }
    }
    let nodes = tree.getattr("nodes")?.cast_into::<PyDict>()?;
    let mut files = vec![];
    let recipes = PyDict::new(py);
    for (k, n) in nodes.iter() {
        recipes.set_item(&k, n.get_item("recipe")?)?;
        if n.get_item("kind")?.eq("link")? {
            let s = n.get_item("stems")?.extract::<Vec<String>>()?;
            files.push((*rank.get(&s).unwrap_or(&0), n));
        }
    }
    files.sort_by_key(|v| v.0);
    let host = tree.getattr("_host_roots")?;
    for (i, (_, n)) in files.into_iter().enumerate() {
        n.set_item("order", host.add(i + 1)?)?;
    }
    let structure = crate::model::module(py, "ingetrazo_core.structure")?;
    let member = tree.getattr("member")?;
    let direct = tree.getattr("direct")?;
    let links = tree.getattr("links")?;
    let unsorted = tree.getattr("unfiled")?;
    let alias = tree.getattr("alias")?;
    let cache = PyDict::new(py);
    for row in rows.iter() {
        let g = row.get_item(0)?;
        let chain = row.get_item(1)?;
        let inner = row.get_item(2)?;
        let root = row.get_item(3)?;
        let is_link = !inner.eq(&chain)?;
        let addr = g.as_ptr() as usize;
        let mounted = |chain: &Obj<'py>| -> PyResult<Obj<'py>> {
            if let Some(k) = cache.get_item(chain)? {
                return Ok(k);
            }
            let k = resolved(&alias, mount(&stems(chain)?).into_pyobject(py)?.into_any())?;
            cache.set_item(chain, &k)?;
            Ok(k)
        };
        if is_link {
            let k = mounted(&inner)?;
            links.get_item(&k)?.call_method1("append", (&g,))?;
            member.set_item(addr, k)?;
            continue;
        }
        let own = structure.call_method1("node_of", (&g,))?;
        let key = if chain.is_empty()? {
            let k = if nodes.contains(&own)? {
                own
            } else {
                py.None().into_bound(py)
            };
            if structure
                .call_method1("_is_unsorted", (&g, py.None(), &root, &k, &recipes))?
                .is_truthy()?
            {
                unsorted.call_method1("append", (&g,))?;
            }
            k
        } else {
            let mk = mounted(&chain)?;
            let mk = mk.extract::<String>()?;
            let mut key = None;
            if !own.is_none() {
                let k = resolved(
                    &alias,
                    format!("{mk}/{}", own.extract::<String>()?)
                        .into_pyobject(py)?
                        .into_any(),
                )?;
                if nodes.contains(&k)? {
                    key = Some(k)
                }
            }
            if key.is_none() && root.is_truthy()? {
                if let Some(layer) = g
                    .getattr_opt("layer")?
                    .filter(|v| v.is_truthy().unwrap_or(false))
                {
                    let k = format!("{mk}/tag:{}", layer.extract::<String>()?);
                    if nodes.contains(&k)? {
                        key = Some(k.into_pyobject(py)?.into_any());
                    }
                }
                if key.is_none() {
                    key = Some(mk.into_pyobject(py)?.into_any())
                }
            }
            key.unwrap_or_else(|| py.None().into_bound(py))
        };
        if !key.is_none() {
            member.set_item(addr, &key)?;
            direct.get_item(key)?.call_method1("append", (g,))?;
        }
    }
    let children = PyDict::new(py);
    children.set_item(py.None(), PyList::empty(py))?;
    for (k, _) in nodes.iter() {
        children.set_item(k, PyList::empty(py))?;
    }
    let mut groups: Vec<(Obj, Bound<PyList>)> = vec![];
    let indices = PyDict::new(py);
    for (k, n) in nodes.iter() {
        let parent = n.get_item("parent")?;
        let idx = match indices.get_item(&parent)? {
            Some(i) => i.extract::<usize>()?,
            None => {
                let i = groups.len();
                indices.set_item(&parent, i)?;
                groups.push((parent, PyList::empty(py)));
                i
            }
        };
        // Stable ties follow the original node insertion order.
        groups[idx].1.append((
            n.get_item("order")?,
            n.get_item("name")?.call_method0("casefold")?,
            groups[idx].1.len(),
            k,
        ))?;
    }
    for (parent, items) in groups {
        items.sort()?;
        let out = PyList::empty(py);
        for it in items.iter() {
            out.append(it.get_item(3)?)?;
        }
        children.set_item(parent, out)?;
    }
    tree.setattr("children", children)
}
