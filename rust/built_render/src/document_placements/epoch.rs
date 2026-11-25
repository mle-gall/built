// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use std::collections::{HashMap, HashSet};
fn attr0<'py>(v: &Obj<'py>, s: &str) -> PyResult<Obj<'py>> {
    Ok(v.getattr_opt(s)?
        .unwrap_or(0i32.into_pyobject(v.py())?.into_any()))
}
fn part<'py>(g: &Obj<'py>, loose: &Obj<'py>, m: &Bound<'py, PyModule>) -> PyResult<Obj<'py>> {
    let mesh = g.getattr("mesh")?;
    let xf = g.getattr("xform")?;
    let serial = if mesh.is_none() || mesh.is(loose) {
        none(g.py())
    } else {
        opt(&mesh, "_mut_serial")?
    };
    let matrix = if xf.is_none() {
        xf
    } else {
        tuple(&xf.call_method0("data")?)?
    };
    let empty = !mesh.is_none()
        && !mesh.getattr("__dict__")?.contains("_deferred")?
        && !mesh.getattr("edges")?.is_truthy()?
        && !mesh.getattr("faces")?.is_truthy()?;
    Ok((
        id(g),
        id(&mesh),
        serial,
        matrix,
        g.getattr("layer")?,
        truth(g, "billboard")?,
        truth(g, "hidden")?,
        truth(g, "locked")?,
        m.call_method1("_material_sig", (opt(g, "material")?,))?,
        empty,
    )
        .into_pyobject(g.py())?
        .into_any())
}
#[pyfunction]
fn placements_epoch<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let m = host(py)?;
    let tick = attr0(v, "_tick")?;
    let version = m.call_method1("_cache_ver", (v,))?;
    let stamp = (&tick, &version).into_pyobject(py)?;
    let memo = opt(v, "_epoch_memo")?;
    if !memo.is_none() && memo.get_item(0)?.eq(&stamp)? {
        return memo.get_item(1);
    }
    let sc = v.getattr("scene")?;
    let mesh = sc.getattr("mesh")?;
    let group = sc.getattr("edit_group")?;
    let tops = sc.getattr("groups")?;
    let revision = attr0(v, "_navigator_revision")?;
    let geom = sc
        .getattr_opt("geometry_version")?
        .or(sc.getattr_opt("content_version")?)
        .unwrap_or(sc.getattr("version")?);
    let preview = attr0(v, "_preview_epoch")?;
    let pv = opt(v, "_preview_groups")?;
    let moving = pv.is_truthy()?;
    let mode = opt(v, "_edit_rest_mode")?;
    let objs = truth(&sc, "show_hidden_objects")?;
    let geo = truth(&sc, "show_hidden_geometry")?;
    let mut layers = Vec::new();
    let visible = crate::host_context::core_module(py)?.getattr("tag_edits")?.getattr("effective_visible")?;
    for ly in sc.getattr("layers")?.try_iter()? {
        let ly = ly?;
        layers.push((
            ly.getattr("key")?,
            visible.call1((&sc, &ly))?,
            ly.getattr("locked")?,
        ))
    }
    let layers = PyTuple::new(py, layers)?;
    let look = m.call_method1("_layer_look_sig", (&sc, v.call_method0("_view_style")?))?;
    let cheap = (
        &revision,
        geom,
        id(&group),
        id(&mesh),
        &preview,
        moving,
        &mode,
        objs,
        geo,
        &layers,
        &look,
        tops.len()?,
    )
        .into_pyobject(py)?;
    let live = opt(v, "_frozen_cache_version")?.is_none();
    let same = opt(v, "_epoch_same")?;
    if live && !same.is_none() && same.get_item(0)?.eq(&cheap)? {
        let epoch = same.get_item(1)?;
        v.setattr("_epoch_memo", (stamp, &epoch))?;
        return Ok(epoch);
    }
    let def = mesh
        .getattr_opt("_instance_definition")?
        .unwrap_or_else(|| mesh.clone());
    let head = (
        id(&group),
        id(&def),
        preview,
        moving,
        mode,
        objs,
        geo,
        truth(&sc, "hide_similar_components")?,
        layers,
        look,
        revision,
    )
        .into_pyobject(py)?;
    let parts = head.to_list();
    let head_size = parts.len();
    let base = opt(v, "_epoch_frozen_base")?;
    if !live
        && moving
        && !base.is_none()
        && base.get_item(0)?.eq(&cheap)?
        && base.get_item(1)?.eq(&parts)?
    {
        let whole = base.get_item(2)?;
        for mover in base.get_item(3)?.try_iter()? {
            let (i, g): (usize, Obj) = mover?.extract()?;
            whole.set_item(i, part(&g, &mesh, &m)?)?
        }
        let epoch = tuple(&whole)?.hash()?;
        v.call_method1(
            "_epoch_remember",
            (
                epoch,
                head_size,
                whole.call_method0("copy")?,
                base.get_item(4)?,
            ),
        )?;
        v.setattr("_epoch_memo", (stamp, epoch))?;
        v.setattr("_epoch_same", py.None())?;
        return Ok(epoch.into_pyobject(py)?.into_any());
    }
    let mut movers = Vec::new();
    let mut ranges = Vec::new();
    for g in tops.try_iter()? {
        let root = g?;
        let begin = parts.len();
        let mut stack = vec![root.clone()];
        while let Some(g) = stack.pop() {
            if moving && pv.contains(id(&g))? {
                movers.push((parts.len(), g.clone()))
            }
            parts.append(part(&g, &mesh, &m)?)?;
            let children = g.getattr("children")?;
            if children.is_truthy()? {
                let kids = children.try_iter()?.collect::<PyResult<Vec<_>>>()?;
                stack.extend(kids.into_iter().rev())
            }
        }
        ranges.push((id(&root), begin, parts.len()));
    }
    let epoch = tuple(parts.as_any())?.hash()?;
    let ranges = PyList::new(py, ranges)?;
    v.call_method1("_epoch_remember", (epoch, head_size, &parts, &ranges))?;
    if !live && moving {
        v.setattr(
            "_epoch_frozen_base",
            (
                &cheap,
                head.to_list(),
                parts.call_method0("copy")?,
                movers,
                ranges,
            ),
        )?
    } else {
        v.setattr("_epoch_frozen_base", py.None())?
    }
    v.setattr("_epoch_memo", (stamp, epoch))?;
    if live {
        v.setattr("_epoch_same", (cheap, epoch))?
    } else {
        v.setattr("_epoch_same", py.None())?
    }
    Ok(epoch.into_pyobject(py)?.into_any())
}
#[pyfunction]
fn epoch_remember(v: &Obj, epoch: &Obj, head: &Obj, parts: &Obj, tops: &Obj) -> PyResult<()> {
    let mut hist = opt(v, "_epoch_hist")?;
    if hist.is_none() {
        hist = v.py().import("collections")?.call_method0("OrderedDict")?;
        v.setattr("_epoch_hist", &hist)?
    }
    hist.set_item(epoch, (head, parts, tops))?;
    hist.call_method1("move_to_end", (epoch,))?;
    let cap = v.getattr("_EPOCH_HISTORY")?.extract::<usize>()?;
    while hist.len()? > cap {
        let kw = PyDict::new(v.py());
        kw.set_item("last", false)?;
        hist.call_method("popitem", (), Some(&kw))?;
    }
    Ok(())
}
fn epochs<'py>(
    v: &Obj<'py>,
    old: &Obj<'py>,
    new: &Obj<'py>,
) -> PyResult<Option<(Obj<'py>, Obj<'py>)>> {
    let hist = opt(v, "_epoch_hist")?;
    if hist.is_none() {
        return Ok(None);
    }
    let a = hist.call_method1("get", (old,))?;
    let b = hist.call_method1("get", (new,))?;
    Ok((!a.is_none() && !b.is_none() && a.get_item(0)?.eq(b.get_item(0)?)?).then_some((a, b)))
}
fn header(a: &Obj, b: &Obj, head: usize, ctx: bool) -> PyResult<bool> {
    for i in 0..head {
        if i == 2 || i == 3 || ctx && i < 2 {
            continue;
        }
        if !a.get_item(i)?.eq(b.get_item(i)?)? {
            return Ok(false);
        }
    }
    Ok(true)
}
fn same_fields(a: &Obj, b: &Obj, omit: &[usize]) -> PyResult<bool> {
    if a.len()? != b.len()? {
        return Ok(false);
    }
    for i in 0..a.len()? {
        if !omit.contains(&i) && !a.get_item(i)?.eq(b.get_item(i)?)? {
            return Ok(false);
        }
    }
    Ok(true)
}
#[pyfunction]
fn epoch_only_matrices(v: &Obj, old: &Obj, new: &Obj) -> PyResult<bool> {
    let new = if new.is_none() {
        v.call_method0("_placements_epoch")?
    } else {
        new.clone()
    };
    if old.eq(&new)? {
        return Ok(true);
    }
    let Some((a, b)) = epochs(v, old, &new)? else {
        return Ok(false);
    };
    let head = a.get_item(0)?.extract::<usize>()?;
    let (pa, pb) = (a.get_item(1)?, b.get_item(1)?);
    if pa.len()? != pb.len()? || !header(&pa, &pb, head, false)? {
        return Ok(false);
    }
    let mut edited = HashSet::new();
    let mut placed = HashSet::new();
    for i in head..pa.len()? {
        let (x, y) = (pa.get_item(i)?, pb.get_item(i)?);
        let has = !x.get_item(3)?.is_none();
        let mesh = y.get_item(1)?.extract::<usize>()?;
        if has {
            placed.insert(mesh);
        }
        if x.is(&y) || x.eq(&y)? {
            continue;
        }
        if !same_fields(&x, &y, &[2, 3])? || has == y.get_item(3)?.is_none() {
            return Ok(false);
        }
        if !x.get_item(2)?.eq(y.get_item(2)?)? {
            if has {
                return Ok(false);
            }
            edited.insert(mesh);
        }
    }
    Ok(edited.is_disjoint(&placed))
}
#[pyfunction]
fn epoch_kept_tops<'py>(
    v: &Obj<'py>,
    old: &Obj<'py>,
    new: &Obj<'py>,
    ctx: bool,
) -> PyResult<Obj<'py>> {
    let py = v.py();
    let new = if new.is_none() {
        v.call_method0("_placements_epoch")?
    } else {
        new.clone()
    };
    let key = (old, &new, ctx).into_pyobject(py)?;
    let memo = opt(v, "_kept_tops_memo")?;
    if !memo.is_none() && memo.get_item(0)?.eq(&key)? {
        return memo.get_item(1);
    }
    let compute = || -> PyResult<Obj<'py>> {
        let Some((a, b)) = epochs(v, old, &new)? else {
            return Ok(none(py));
        };
        if a.len()? < 3 || b.len()? < 3 {
            return Ok(none(py));
        }
        let head = a.get_item(0)?.extract::<usize>()?;
        let (pa, pb) = (a.get_item(1)?, b.get_item(1)?);
        if !header(&pa, &pb, head, ctx)? {
            return Ok(none(py));
        }
        let moved = head > 1
            && (!pa.get_item(0)?.eq(pb.get_item(0)?)? || !pa.get_item(1)?.eq(pb.get_item(1)?)?);
        if moved && (head <= 7 || pa.get_item(7)?.is_truthy()? || pb.get_item(7)?.is_truthy()?) {
            return Ok(none(py));
        }
        let mut roots = HashMap::new();
        for (t, s, e) in a.get_item(2)?.extract::<Vec<(usize, usize, usize)>>()? {
            roots.insert(t, (s, e));
        }
        let mut kept = Vec::new();
        'root: for (t, s, e) in b.get_item(2)?.extract::<Vec<(usize, usize, usize)>>()? {
            let Some(&(os, oe)) = roots.get(&t) else {
                continue;
            };
            if oe - os != e - s {
                continue;
            }
            for k in s..e {
                let (x, y) = (pa.get_item(os + k - s)?, pb.get_item(k)?);
                let empty = x.len()? > 9
                    && y.len()? > 9
                    && x.get_item(9)?.is_truthy()?
                    && y.get_item(9)?.is_truthy()?;
                if !x.is(&y) && !x.eq(&y)? && (!empty || !same_fields(&x, &y, &[2])?) {
                    continue 'root;
                }
                if moved
                    && !empty
                    && (y.get_item(1)?.eq(pa.get_item(1)?)? || y.get_item(1)?.eq(pb.get_item(1)?)?)
                {
                    continue 'root;
                }
            }
            kept.push(t)
        }
        Ok(PyFrozenSet::new(py, kept)?.into_any())
    };
    let out = compute()?;
    v.setattr("_kept_tops_memo", (key, &out))?;
    Ok(out)
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(placements_epoch, m)?)?;
    m.add_function(wrap_pyfunction!(epoch_remember, m)?)?;
    m.add_function(wrap_pyfunction!(epoch_only_matrices, m)?)?;
    m.add_function(wrap_pyfunction!(epoch_kept_tops, m)?)?;
    Ok(())
}
