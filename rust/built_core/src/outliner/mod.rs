// SPDX-License-Identifier: GPL-3.0-or-later
//! Read the document's placement hierarchy for Browser actions and extensions.
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
use std::collections::{HashMap, HashSet};
type Obj<'p> = Bound<'p, PyAny>;
type Row<'p> = (Obj<'p>, Obj<'p>, usize);
mod sheets;
fn id(o: &Obj) -> usize {
    o.as_ptr() as usize
}
fn none(py: Python) -> Obj {
    py.None().into_bound(py)
}
fn value<'p>(o: &Obj<'p>, k: &str) -> PyResult<Obj<'p>> {
    Ok(o.getattr_opt(k)?.unwrap_or_else(|| none(o.py())))
}
fn text(o: &Obj) -> PyResult<String> {
    if o.is_none() {
        Ok(String::new())
    } else {
        Ok(o.str()?.to_string())
    }
}
fn flag(o: &Obj, k: &str) -> PyResult<bool> {
    value(o, k)?.is_truthy()
}
fn elements<'p>(o: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    if o.is_none() {
        Ok(vec![])
    } else {
        o.try_iter()?.collect()
    }
}
fn ids(o: &Obj) -> PyResult<HashSet<usize>> {
    o.try_iter()?.map(|v| Ok(id(&v?))).collect()
}

/// A depth-first iterator. Children are consulted when the caller resumes the walk.
#[pyclass]
struct Walk {
    stack: Vec<(Py<PyAny>, Py<PyAny>)>,
    pending: Option<Py<PyAny>>,
    depth: usize,
}
#[pymethods]
impl Walk {
    fn __traverse__(&self, visit: pyo3::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        for (children, parent) in &self.stack {
            visit.call(children)?;
            visit.call(parent)?;
        }
        if let Some(g) = &self.pending {
            visit.call(g)?;
        }
        Ok(())
    }
    fn __clear__(&mut self) {
        self.stack.clear();
        self.pending = None;
    }
    fn __iter__(slf: PyRef<Self>) -> PyRef<Self> {
        slf
    }
    fn __next__<'p>(&mut self, py: Python<'p>) -> PyResult<Option<Row<'p>>> {
        if let Some(last) = self.pending.take() {
            let children = value(last.bind(py), "children")?;
            if !children.is_none() {
                self.stack
                    .push((children.try_iter()?.into_any().unbind(), last));
            }
        }
        while let Some((iterator, parent)) = self.stack.last() {
            if let Some(child) = iterator
                .bind(py)
                .cast::<pyo3::types::PyIterator>()?
                .clone()
                .next()
            {
                let child = child?;
                self.pending = Some(child.clone().unbind());
                return Ok(Some((
                    child,
                    parent.bind(py).clone(),
                    self.depth + self.stack.len() - 1,
                )));
            }
            self.stack.pop();
        }
        Ok(None)
    }
}
#[pyfunction(signature=(groups,parent=None,depth=0))]
fn walk(groups: &Obj, parent: Option<&Obj>, depth: usize) -> PyResult<Walk> {
    Ok(Walk {
        stack: vec![(
            groups.try_iter()?.into_any().unbind(),
            parent.map_or_else(|| groups.py().None(), |p| p.clone().unbind()),
        )],
        pending: None,
        depth,
    })
}
fn visit<'p>(groups: &Obj<'p>, mut f: impl FnMut(Row<'p>) -> PyResult<bool>) -> PyResult<()> {
    let mut it = walk(groups, None, 0)?;
    while let Some(row) = it.__next__(groups.py())? {
        if !f(row)? {
            break;
        }
    }
    Ok(())
}
#[pyfunction]
fn parents<'p>(groups: &Obj<'p>) -> PyResult<HashMap<usize, Obj<'p>>> {
    let mut up = HashMap::new();
    visit(groups, |(g, p, _)| {
        up.insert(id(&g), p);
        Ok(true)
    })?;
    Ok(up)
}
#[pyfunction]
fn path_to<'p>(groups: &Obj<'p>, target: &Obj) -> PyResult<Option<Vec<Obj<'p>>>> {
    let mut ancestors = vec![];
    let mut found = false;
    visit(groups, |(g, _, depth)| {
        ancestors.truncate(depth);
        found = g.is(target);
        if !found {
            ancestors.push(g);
        }
        Ok(!found)
    })?;
    Ok(found.then_some(ancestors))
}
#[pyfunction]
fn structure<'p>(groups: &Obj<'p>) -> PyResult<Bound<'p, PyTuple>> {
    let mut shape = vec![];
    visit(groups, |(g, p, _)| {
        shape.push((id(&g), if p.is_none() { 0 } else { id(&p) }));
        Ok(true)
    })?;
    PyTuple::new(groups.py(), shape)
}
#[pyfunction]
fn open_path<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let root = value(scene, "_root")?;
    if !root.is_none() {
        return root.getattr("_edit_path");
    }
    let stack = elements(&value(scene, "_edit_stack")?)?;
    Ok(PyList::new(
        scene.py(),
        stack
            .iter()
            .map(|l| l.get_item("group"))
            .collect::<PyResult<Vec<_>>>()?,
    )?
    .into_any())
}
#[pyfunction]
fn kind(group: &Obj) -> PyResult<&'static str> {
    Ok(if flag(group, "billboard")? {
        "image"
    } else if flag(group, "text3d")? {
        "text"
    } else if group.call_method0("is_component")?.is_truthy()? {
        "component"
    } else {
        "group"
    })
}
#[pyfunction]
fn ifc_class(group: &Obj) -> PyResult<String> {
    let tag = value(group, "ifc")?;
    match tag.cast::<PyDict>() {
        Ok(d) => text(&d.get_item("class")?.unwrap_or_else(|| none(group.py()))),
        Err(_) => Ok(String::new()),
    }
}
#[pyfunction]
fn instance_counts(groups: &Obj) -> PyResult<HashMap<usize, usize>> {
    let mut counts = HashMap::new();
    visit(groups, |(g, _, _)| {
        if g.call_method0("is_component")?.is_truthy()? {
            *counts.entry(id(&g.getattr("mesh")?)).or_default() += 1;
        }
        Ok(true)
    })?;
    Ok(counts)
}
fn match_group(g: &Obj, q: &str, provider: Option<&Obj>) -> PyResult<bool> {
    if q.is_empty() {
        return Ok(true);
    }
    let py = g.py();
    let wildcard = q.contains(['*', '?']);
    let labels = [
        g.getattr("name")?,
        provider
            .map(|cb| cb.call1((g,)))
            .transpose()?
            .unwrap_or_else(|| none(py)),
        ifc_class(g)?.into_pyobject(py)?.into_any(),
        py.import("core.layers")?.call_method1("layer_of", (g,))?,
    ];
    for label in labels {
        if label.is_none() {
            continue;
        }
        let folded = label.str()?.call_method0("casefold")?;
        if if wildcard {
            py.import("fnmatch")?
                .call_method1("fnmatchcase", (&folded, q))?
                .is_truthy()?
        } else {
            folded.contains(q)?
        } {
            return Ok(true);
        }
    }
    Ok(false)
}
#[pyfunction(signature=(groups,query,type_of=None,state_of=None,invert=false))]
fn filter_keep(
    groups: &Obj,
    query: &str,
    type_of: Option<&Obj>,
    state_of: Option<&Obj>,
    invert: bool,
) -> PyResult<(HashSet<usize>, HashSet<usize>)> {
    let mut keep = HashSet::new();
    let mut hit = HashSet::new();
    let mut path = vec![];
    visit(groups, |(g, _, d)| {
        path.truncate(d);
        path.push(id(&g));
        if (match_group(&g, query, type_of)?
            && state_of
                .map(|f| f.call1((&g,))?.is_truthy())
                .transpose()?
                .unwrap_or(true))
            != invert
        {
            hit.insert(id(&g));
            keep.extend(&path);
        }
        Ok(true)
    })?;
    Ok((keep, hit))
}
#[pyfunction]
fn isolate_hides<'p>(groups: &Obj<'p>, keep: &Obj) -> PyResult<Vec<Obj<'p>>> {
    let chosen = ids(keep)?;
    let mut holders = chosen.clone();
    let mut rows = vec![];
    let mut path = vec![];
    visit(groups, |(g, p, d)| {
        path.truncate(d);
        path.push(id(&g));
        if chosen.contains(&id(&g)) {
            holders.extend(&path);
        }
        rows.push((g, p));
        Ok(true)
    })?;
    let mut hidden = vec![];
    for (g, p) in rows {
        if !holders.contains(&id(&g))
            && !flag(&g, "hidden")?
            && (p.is_none() || holders.contains(&id(&p)) && !chosen.contains(&id(&p)))
        {
            hidden.push(g);
        }
    }
    Ok(hidden)
}
#[pyfunction]
fn topmost<'p>(groups: &Obj<'p>, chosen: &Obj) -> PyResult<Vec<Obj<'p>>> {
    let chosen = ids(chosen)?;
    let mut out = vec![];
    let mut inside = None;
    visit(groups, |(g, _, d)| {
        if inside.is_some_and(|level| d > level) {
            return Ok(true);
        }
        inside = None;
        if chosen.contains(&id(&g)) {
            inside = Some(d);
            out.push(g);
        }
        Ok(true)
    })?;
    Ok(out)
}
fn belongs(up: &HashMap<usize, Obj>, group: &Obj) -> PyResult<bool> {
    let mut p = group.clone();
    while !p.is_none() {
        if p.call_method0("is_component")?.is_truthy()? {
            return Ok(true);
        }
        let Some(next) = up.get(&id(&p)) else { break };
        p = next.clone();
    }
    Ok(false)
}
#[pyfunction]
fn in_component(groups: &Obj, group: &Obj) -> PyResult<bool> {
    belongs(&parents(groups)?, group)
}
#[pyfunction]
fn editing_inside_component(scene: &Obj) -> PyResult<bool> {
    for l in elements(&value(scene, "_edit_stack")?)? {
        let frame = l.cast::<PyDict>()?;
        for key in ["share", "definition"] {
            if frame.get_item(key)?.is_some_and(|v| !v.is_none()) {
                return Ok(true);
            }
        }
    }
    let g = value(scene, "edit_group")?;
    Ok(!g.is_none() && in_component(&scene.getattr("groups")?, &g)?)
}
#[pyfunction]
fn reparent_problem(groups: &Obj, chosen: &Obj, target: &Obj) -> PyResult<Option<&'static str>> {
    let up = parents(groups)?;
    let moving = topmost(groups, chosen)?;
    let mut p = target.clone();
    while !p.is_none() {
        if moving.iter().any(|g| g.is(&p)) {
            return Ok(Some("cycle"));
        }
        let Some(next) = up.get(&id(&p)) else { break };
        p = next.clone();
    }
    if belongs(&up, target)? {
        return Ok(Some("component"));
    }
    for g in moving {
        if let Some(old) = up.get(&id(&g)) {
            if belongs(&up, old)? {
                return Ok(Some("component"));
            }
        }
        if flag(&g, "locked")? {
            return Ok(Some("locked"));
        }
    }
    Ok((!target.is_none() && flag(target, "locked")?).then_some("locked"))
}
#[pyfunction]
fn compose<'p>(a: &Obj<'p>, b: &Obj<'p>) -> PyResult<Obj<'p>> {
    if a.is_none() {
        Ok(b.clone())
    } else if b.is_none() {
        Ok(a.clone())
    } else {
        a.mul(b)
    }
}
#[pyfunction]
fn world_matrix<'p>(groups: &Obj<'p>, group: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = groups.py();
    let Some(mut path) = path_to(groups, group)? else {
        return Ok(none(py));
    };
    path.push(group.clone());
    let mut matrix = none(py);
    for g in path {
        matrix = compose(&matrix, &value(&g, "xform")?)?;
    }
    Ok(matrix)
}
#[pyfunction]
fn custom_kind<'p>(group: &Obj<'p>, providers: &Obj<'p>) -> PyResult<Option<Bound<'p, PyDict>>> {
    let py = group.py();
    for f in elements(providers)? {
        let out = match f.call1((group,)) {
            Ok(v) => v,
            Err(err) => {
                let kw = PyDict::new(py);
                kw.set_item("exc_info", err.value(py))?;
                py.import("logging")?
                    .call_method1("getLogger", ("built.plugins",))?
                    .call_method("error", ("outliner kind provider failed",), Some(&kw))?;
                continue;
            }
        };
        let Ok(dict) = out.cast::<PyDict>() else {
            continue;
        };
        let Some(ty) = dict.get_item("type")? else {
            continue;
        };
        if !ty.is_truthy()? {
            continue;
        }
        let mut props = vec![];
        if let Some(p) = dict.get_item("props")? {
            for pair in elements(&p)? {
                if let Ok(pair) = elements(&pair) {
                    if let [k, v] = &pair[..] {
                        props.push((k.str()?.to_string(), v.str()?.to_string()));
                    }
                }
            }
        }
        let d = PyDict::new(py);
        d.set_item("type", ty.str()?)?;
        d.set_item("props", props)?;
        return Ok(Some(d));
    }
    Ok(None)
}
pub(crate) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    for f in [
        wrap_pyfunction!(walk, m)?,
        wrap_pyfunction!(parents, m)?,
        wrap_pyfunction!(path_to, m)?,
        wrap_pyfunction!(structure, m)?,
        wrap_pyfunction!(open_path, m)?,
        wrap_pyfunction!(kind, m)?,
        wrap_pyfunction!(ifc_class, m)?,
        wrap_pyfunction!(instance_counts, m)?,
        wrap_pyfunction!(filter_keep, m)?,
        wrap_pyfunction!(isolate_hides, m)?,
        wrap_pyfunction!(topmost, m)?,
        wrap_pyfunction!(in_component, m)?,
        wrap_pyfunction!(editing_inside_component, m)?,
        wrap_pyfunction!(reparent_problem, m)?,
        wrap_pyfunction!(compose, m)?,
        wrap_pyfunction!(world_matrix, m)?,
        wrap_pyfunction!(custom_kind, m)?,
    ] {
        m.add_function(f)?
    }
    sheets::install(m)
}
