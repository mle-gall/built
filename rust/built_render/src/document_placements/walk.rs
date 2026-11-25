// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[pyfunction]
fn placement_top<'py>(v: &Obj<'py>, g: &Obj<'py>) -> PyResult<usize> {
    let tops = opt(v, "_placement_tops")?;
    if tops.is_truthy()? {
        tops.call_method1("get", (id(g), id(g)))?.extract()
    } else {
        Ok(id(g))
    }
}
#[pyfunction]
fn refilter<'py>(
    v: &Obj<'py>,
    epoch: &Obj<'py>,
    old_out: &Obj<'py>,
    source: &Obj<'py>,
    test: &Obj<'py>,
    old_source: &Obj<'py>,
    ctx: bool,
) -> PyResult<Obj<'py>> {
    let py = v.py();
    let kw = PyDict::new(py);
    kw.set_item("ctx", ctx)?;
    let kept = v.call_method("_epoch_kept_tops", (epoch,), Some(&kw))?;
    if kept.is_none() {
        return Ok(kept);
    }
    let previous: std::collections::HashSet<usize> = old_out
        .try_iter()?
        .map(|g| g.map(|x| id(&x)))
        .collect::<PyResult<_>>()?;
    let old_src = if old_source.is_none() {
        None
    } else {
        Some(py.import("builtins")?.call_method1("set", (old_source,))?)
    };
    let top = v.getattr("_placement_top")?;
    let out = PyList::empty(py);
    for g in source.try_iter()? {
        let g = g?;
        let stable = kept.contains(top.call1((&g,))?)?
            && match &old_src {
                Some(s) => s.contains(id(&g))?,
                None => true,
            };
        if if stable {
            previous.contains(&id(&g))
        } else {
            test.call1((&g,))?.is_truthy()?
        } {
            out.append(g)?
        }
    }
    Ok(out.into_any())
}
fn tree<'py>(py: Python<'py>, groups: &Obj<'py>) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let data = py
        .import("built_core.doc_model")?
        .getattr("NATIVE")?
        .call_method1("doc_tree", (groups,))?;
    Ok((data.get_item(0)?, data.get_item(1)?.call_method0("tolist")?))
}
fn filter_allowed<'py>(
    scene: &Obj<'py>,
    groups: Obj<'py>,
    allowed: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    if allowed.is_none() {
        return Ok(groups);
    }
    let out = PyList::empty(scene.py());
    let key = scene.getattr("navigator_key")?;
    for g in groups.try_iter()? {
        let g = g?;
        if allowed.contains(key.call1((&g,))?)? {
            out.append(g)?
        }
    }
    Ok(out.into_any())
}
#[pyfunction]
fn placements<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let scene = v.getattr("scene")?;
    let groups = scene.getattr("groups")?;
    let ctx = scene.getattr("edit_group")?;
    let epoch = opt(v, "_placements_epoch")?;
    let key = if epoch.is_truthy()? {
        (epoch.call0()?, id(&groups), groups.len()?)
            .into_pyobject(py)?
            .into_any()
    } else {
        none(py)
    };
    let memo = opt(v, "_placements_memo")?;
    if !key.is_none() && !memo.is_none() && memo.get_item(0)?.eq(&key)? {
        return memo.get_item(1);
    }
    let allowed = opt(&scene, "_navigator_allowed")?;
    let mut nested = truth(&ctx, "children")?;
    if !nested {
        for g in groups.try_iter()? {
            if truth(&g?, "children")? {
                nested = true;
                break;
            }
        }
    }
    let out = if nested {
        let proxies = cache(v, "_placement_proxies")?;
        let out = PyList::empty(py);
        let seen = PySet::empty(py)?;
        let (handles, parents) = tree(py, &groups)?;
        class(py)?.call_method1("_bind_placement_rows", (v, handles, parents, &out, &seen))?;
        if proxies.len()? > seen.len() && !truth(v, "_preview_groups")? {
            let tops = opt(v, "_placement_tops")?;
            let keys = proxies
                .call_method0("keys")?
                .try_iter()?
                .collect::<PyResult<Vec<_>>>()?;
            for k in keys {
                if !seen.contains(&k)? {
                    let gone = proxies.call_method1("pop", (k, py.None()))?;
                    if !gone.is_none() && !tops.is_none() {
                        tops.call_method1("pop", (id(&gone), py.None()))?;
                    }
                }
            }
        }
        out.into_any()
    } else {
        groups
    };
    let out = filter_allowed(&scene, out, &allowed)?;
    v.setattr("_placements_memo", (key, &out))?;
    Ok(out)
}
#[pyfunction]
fn expand_placements<'py>(
    v: &Obj<'py>,
    group: &Obj<'py>,
    out: &Obj<'py>,
    seen: &Obj<'py>,
    above: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = v.py();
    let out = if out.is_none() {
        PyList::empty(py).into_any()
    } else {
        out.clone()
    };
    if !truth(group, "children")? {
        out.call_method1("append", (group,))?;
        return Ok(out);
    }
    let (handles, parents) = tree(py, PyList::new(py, [group])?.as_any())?;
    class(py)?.call_method1(
        "_bind_placement_rows",
        (v, handles, parents, out, seen, above),
    )
}
struct Ancestor<'py> {
    world: Obj<'py>,
    inside: Obj<'py>,
    hidden: bool,
    forced: Obj<'py>,
    layer: Obj<'py>,
    paint: Obj<'py>,
}
fn first_truthy<'py>(items: &[&Obj<'py>]) -> PyResult<Obj<'py>> {
    for item in items.iter().take(items.len() - 1) {
        if item.is_truthy()? {
            return Ok((*item).clone());
        }
    }
    Ok((*items[items.len() - 1]).clone())
}
fn forced_layer<'py>(scene: &Obj<'py>, g: &Obj<'py>) -> PyResult<Obj<'py>> {
    let (vis, locked): (bool, bool) = scene.call_method1("_layer_state", (g,))?.extract()?;
    if !vis || locked {
        g.getattr("layer")
    } else {
        Ok(none(scene.py()))
    }
}
#[pyfunction]
fn bind_placement_rows<'py>(
    v: &Obj<'py>,
    handles: &Obj<'py>,
    parents: Vec<isize>,
    out: &Obj<'py>,
    seen: &Obj<'py>,
    above: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = v.py();
    let proxies = cache(v, "_placement_proxies")?;
    let tops = cache(v, "_placement_tops")?;
    let scene = v.getattr("scene")?;
    let ctx = scene.getattr("edit_group")?;
    let module = host(py)?;
    let mut ancestors: std::collections::HashMap<isize, Ancestor> = Default::default();
    let mut root = none(py);
    for (i, (child, &parent)) in handles.try_iter()?.zip(&parents).enumerate() {
        let child = child?;
        let branches = parents.get(i + 1) == Some(&(i as isize));
        if parent < 0 {
            root = child;
            out.call_method1("append", (&root,))?;
            if branches {
                ancestors.insert(
                    i as isize,
                    Ancestor {
                        world: root.getattr("xform")?,
                        inside: none(py),
                        hidden: truth(&root, "hidden")?,
                        forced: forced_layer(&scene, &root)?,
                        layer: root.getattr("layer")?,
                        paint: first_truthy(&[&root.getattr("material")?, above])?,
                    },
                );
            }
            continue;
        }
        let a = ancestors.get(&parent).ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err("placement row has no parent")
        })?;
        let local = child.getattr("xform")?;
        let world = if a.world.is_none() {
            local
        } else if local.is_none() {
            a.world.clone()
        } else {
            a.world.mul(local)?
        };
        let mesh = child.getattr("mesh")?;
        let mut proxy = proxies.call_method1("get", (id(&child),))?;
        if proxy.is_none() {
            let kw = PyDict::new(py);
            kw.set_item("name", child.getattr("name")?)?;
            kw.set_item("uid", child.getattr("uid")?)?;
            proxy = module
                .getattr("PlacementProxy")?
                .call((&mesh,), Some(&kw))?;
            proxies.set_item(id(&child), &proxy)?
        }
        if !seen.is_none() {
            seen.call_method1("add", (id(&child),))?;
        }
        tops.set_item(id(&proxy), id(&root))?;
        proxy.setattr("mesh", mesh)?;
        proxy.setattr("source_occurrence", &child)?;
        proxy.setattr("uid", scene.call_method1("navigator_key", (&child,))?)?;
        let layer = child.getattr("layer")?;
        let own = if crate::host_context::core_module(py)?.getattr("tag_styles")?
            .call_method1("is_link_untagged", (&layer,))?
            .is_truthy()?
        {
            none(py)
        } else {
            layer.clone()
        };
        let parent_group = handles.get_item(parent)?;
        let layer = first_truthy(&[
            &a.forced,
            &own,
            &a.layer,
            &parent_group.getattr("layer")?,
            &layer,
        ])?;
        proxy.setattr("layer", &layer)?;
        proxy.setattr("billboard", child.getattr("billboard")?)?;
        let hidden = a.hidden || truth(&child, "hidden")?;
        proxy.setattr("hidden", hidden)?;
        let paint = first_truthy(&[&child.getattr("material")?, &a.paint])?;
        proxy.setattr("material", &paint)?;
        let inside = if !ctx.is_none() && parent_group.is(&ctx) {
            child.clone()
        } else {
            a.inside.clone()
        };
        let (owner, context) = if child.is(&ctx) {
            (&child, &ctx)
        } else if !inside.is_none() {
            (&inside, &ctx)
        } else {
            (&root, &none(py))
        };
        proxy.setattr("owner", owner)?;
        proxy.setattr("context", context)?;
        proxy.setattr("xform", &world)?;
        out.call_method1("append", (&proxy,))?;
        if branches {
            let forced = if a.forced.is_none() && layer.is_truthy()? {
                forced_layer(&scene, &proxy)?
            } else {
                a.forced.clone()
            };
            ancestors.insert(
                i as isize,
                Ancestor {
                    world,
                    inside,
                    hidden,
                    forced,
                    layer,
                    paint,
                },
            );
        }
    }
    Ok(out.clone())
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(refilter, m)?)?;
    m.add_function(wrap_pyfunction!(placement_top, m)?)?;
    m.add_function(wrap_pyfunction!(placements, m)?)?;
    m.add_function(wrap_pyfunction!(expand_placements, m)?)?;
    m.add_function(wrap_pyfunction!(bind_placement_rows, m)?)?;
    Ok(())
}
