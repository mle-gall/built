// SPDX-License-Identifier: GPL-3.0-or-later
//! A cycle graph may start on the store's scan and materialise its adjacency on demand.
use super::{store_api, Obj};
use crate::{
    draw_plan,
    math::{xyz, Vec3},
    object_outline,
    store::commands::cell,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PySet, PyTuple},
};
use std::collections::HashMap;

#[pyfunction]
pub(super) fn _key<'py>(p: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    if p.is_instance_of::<Vec3>() || p.hasattr("toTuple")? {
        let coords = xyz(p)?.to_array();
        let rounded = cell(coords);
        return PyTuple::new(
            p.py(),
            (0..3).map(|i| f64::from_bits(rounded[i]).copysign(coords[i])),
        );
    }
    let items = p
        .try_iter()?
        .map(|x| x?.call_method1("__round__", (4,)))
        .collect::<PyResult<Vec<_>>>()?;
    PyTuple::new(p.py(), items)
}
#[pyclass(module = "core.topology")]
struct KeyMemo {
    seen: HashMap<usize, (Py<PyAny>, Py<PyTuple>)>,
}
#[pymethods]
impl KeyMemo {
    #[new]
    fn new() -> Self {
        Self {
            seen: HashMap::new(),
        }
    }
    fn __call__<'py>(&mut self, p: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let id = p.as_ptr() as usize;
        if let Some((_, k)) = self.seen.get(&id) {
            return Ok(k.bind(p.py()).clone());
        }
        let key = _key(p)?;
        self.seen
            .insert(id, (p.clone().unbind(), key.clone().unbind()));
        Ok(key)
    }
}
#[pyfunction]
fn same_position(p: &Obj<'_>, q: &Obj<'_>) -> PyResult<bool> {
    _key(p)?.eq(_key(q)?)
}

#[pyfunction]
fn _store_edge_plan<'py>(edges: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let py = edges.py();
    let Some(mesh) = edges.getattr_opt("owner")? else {
        return Ok(None);
    };
    let api = store_api(py)?;
    if !edges.is_instance(&py.import("built_core")?.getattr("MeshElements")?)?
        || !edges.getattr("_kind")?.eq("edge")?
        || !mesh.get_type().is(&api.getattr("StoreMesh")?)
    {
        return Ok(None);
    }
    let scan = py
        .import("built_core.scan_index")?
        .call_method1("scan", (mesh,))?;
    if scan.is_none() {
        return Ok(None);
    }
    Ok(Some(
        py.import("core.edits")?
            .call_method1("_ScanEdges", (scan,))?,
    ))
}
fn cycles<'py>(
    edges: &Obj<'py>,
    a: &Obj<'py>,
    b: &Obj<'py>,
    limit: isize,
    both: bool,
) -> PyResult<Option<Obj<'py>>> {
    let plan = _store_edge_plan(edges)?;
    match plan
        .as_ref()
        .unwrap_or(edges)
        .getattr_opt("drawing_cycles")?
    {
        Some(query) if !query.is_none() => Ok(Some(query.call1((a, b, limit, both))?)),
        _ => Ok(None),
    }
}
#[pyfunction]
fn _drawing_cycles<'py>(
    edges: &Obj<'py>,
    a: &Obj<'py>,
    b: &Obj<'py>,
    limit: isize,
    both: bool,
) -> PyResult<Obj<'py>> {
    Ok(cycles(edges, a, b, limit, both)?
        .unwrap_or_else(|| edges.py().NotImplemented().into_bound(edges.py())))
}
fn key_fn(py: Python<'_>) -> PyResult<Obj<'_>> {
    py.import("core.topology")?.getattr("_key")
}
#[pyfunction]
#[pyo3(signature=(edges,ka,kb,key=None))]
fn _cycle_adjacency<'py>(
    edges: &Obj<'py>,
    ka: &Obj<'py>,
    kb: &Obj<'py>,
    key: Option<&Obj<'py>>,
) -> PyResult<Bound<'py, PyDict>> {
    let default = key_fn(edges.py())?;
    draw_plan::graph_adjacency(edges, ka, kb, key.unwrap_or(&default))
}
#[pyclass(module = "core.topology", subclass)]
struct CycleGraph {
    adjacency: Py<PyDict>,
    key: Py<PyAny>,
    scan: Option<Py<PyAny>>,
}
#[pymethods]
impl CycleGraph {
    #[new]
    #[pyo3(signature=(edges,key=None))]
    fn new(edges: &Obj<'_>, key: Option<&Obj<'_>>) -> PyResult<Self> {
        let py = edges.py();
        let default = key_fn(py)?;
        let key = key.unwrap_or(&default);
        let scan = if key.is(&default) || key.is_instance_of::<KeyMemo>() {
            _store_edge_plan(edges)?
        } else {
            None
        };
        let adjacency = PyDict::new(py);
        if scan.is_none() {
            for e in edges.try_iter()? {
                draw_plan::graph_insert(&adjacency, key, &e?)?
            }
        }
        Ok(Self {
            adjacency: adjacency.unbind(),
            key: key.clone().unbind(),
            scan: scan.map(Bound::unbind),
        })
    }
    #[getter]
    fn _scan<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyAny>> {
        self.scan.as_ref().map(|s| s.bind(py).clone())
    }
    #[getter]
    fn adj<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let adj = self.adjacency.bind(py);
        if let Some(scan) = self.scan.take() {
            for e in scan.bind(py).call_method0("all_elements")?.try_iter()? {
                draw_plan::graph_insert(adj, self.key.bind(py), &e?)?
            }
        }
        Ok(adj.clone())
    }
    fn add(&self, edge: &Obj<'_>) -> PyResult<()> {
        let py = edge.py();
        if let Some(s) = &self.scan {
            s.bind(py).call_method1("append", (edge,))?;
            Ok(())
        } else {
            draw_plan::graph_insert(self.adjacency.bind(py), self.key.bind(py), edge)
        }
    }
    fn remove(&self, edge: &Obj<'_>) -> PyResult<bool> {
        let py = edge.py();
        if let Some(s) = &self.scan {
            let s = s.bind(py);
            for c in s
                .call_method1("pick", ((edge.getattr("a")?, edge.getattr("b")?),))?
                .try_iter()?
            {
                let c = c?;
                if c.eq(edge)? {
                    s.call_method1("remove", (c,))?;
                    return Ok(true);
                }
            }
            Ok(false)
        } else {
            draw_plan::graph_remove(self.adjacency.bind(py), self.key.bind(py), edge)
        }
    }
}
#[pyfunction]
#[pyo3(signature=(edges,a,b,max_len=32))]
fn find_smallest_cycle_through<'py>(
    edges: &Obj<'py>,
    a: &Obj<'py>,
    b: &Obj<'py>,
    max_len: isize,
) -> PyResult<Option<Obj<'py>>> {
    let ka = _key(a)?;
    let kb = _key(b)?;
    if ka.eq(&kb)? {
        return Ok(None);
    }
    if let Some(c) = cycles(edges, a, b, max_len, false)? {
        return if c.is_truthy()? {
            Ok(Some(c.get_item(0)?))
        } else {
            Ok(None)
        };
    }
    let adj = _cycle_adjacency(edges, &ka, &kb, None)?;
    Ok(
        draw_plan::smallest_cycle(&adj, a, b, &ka, &kb, max_len, None, false)?
            .map(|r| r.into_pyobject(edges.py()))
            .transpose()?
            .map(|r| r.into_any()),
    )
}
#[pyfunction]
#[pyo3(signature=(edges,a,b,max_results=2,key=None,graph=None))]
fn find_cycles_through<'py>(
    edges: &Obj<'py>,
    a: &Obj<'py>,
    b: &Obj<'py>,
    max_results: isize,
    key: Option<&Obj<'py>>,
    graph: Option<&Obj<'py>>,
) -> PyResult<Obj<'py>> {
    let py = edges.py();
    let out = pyo3::types::PyList::empty(py);
    let ka = _key(a)?;
    let kb = _key(b)?;
    if ka.eq(&kb)? {
        return Ok(out.into_any());
    }
    let both = max_results >= 2;
    let adj = if let Some(g) = graph {
        let s = g.getattr("_scan")?;
        if !s.is_none() {
            return s.call_method1("drawing_cycles", (a, b, 32, both));
        }
        g.getattr("adj")?
    } else {
        if let Some(c) = cycles(edges, a, b, 32, both)? {
            return Ok(c);
        }
        if let Some(lazy) = edges
            .getattr_opt("cycle_adjacency")?
            .filter(|v| !v.is_none())
        {
            lazy.call1((&ka, &kb))?
        } else {
            _cycle_adjacency(edges, &ka, &kb, key)?.into_any()
        }
    };
    if let Some(first) = draw_plan::smallest_cycle(&adj, a, b, &ka, &kb, 32, None, graph.is_some())?
    {
        let first = pyo3::types::PyList::new(py, first)?;
        out.append(&first)?;
        if both {
            let interior = PySet::empty(py)?;
            for v in first.iter() {
                let k = _key(&v)?;
                if !k.eq(&ka)? && !k.eq(&kb)? {
                    interior.add(k)?
                }
            }
            if !interior.is_empty() {
                if let Some(r) = draw_plan::smallest_cycle(
                    &adj,
                    a,
                    b,
                    &ka,
                    &kb,
                    32,
                    Some(&interior),
                    graph.is_some(),
                )? {
                    let r = pyo3::types::PyList::new(py, r)?;
                    if crate::object_drawing::outline_key(&r)?
                        != crate::object_drawing::outline_key(&first)?
                    {
                        out.append(r)?
                    }
                }
            }
        }
    }
    Ok(out.into_any())
}
#[pyfunction]
fn plan_edge_split<'py>(edges: &Obj<'py>, a: &Obj<'py>, b: &Obj<'py>) -> PyResult<Obj<'py>> {
    if let Some(plan) = _store_edge_plan(edges)? {
        let kw = PyDict::new(edges.py());
        kw.set_item("whole", true)?;
        return plan.getattr("split_plan")?.call((a, b), Some(&kw));
    }
    Ok(object_outline::split_edges(edges, a, b)?
        .into_pyobject(edges.py())?
        .into_any())
}

pub(super) fn register(s: &Bound<'_, PyModule>) -> PyResult<()> {
    s.add_class::<CycleGraph>()?;
    s.add_class::<KeyMemo>()?;
    s.add_function(wrap_pyfunction!(_key, s)?)?;
    s.add_function(wrap_pyfunction!(same_position, s)?)?;
    s.add_function(wrap_pyfunction!(_store_edge_plan, s)?)?;
    s.add_function(wrap_pyfunction!(_drawing_cycles, s)?)?;
    s.add_function(wrap_pyfunction!(_cycle_adjacency, s)?)?;
    s.add_function(wrap_pyfunction!(find_smallest_cycle_through, s)?)?;
    s.add_function(wrap_pyfunction!(find_cycles_through, s)?)?;
    s.add_function(wrap_pyfunction!(plan_edge_split, s)?)?;
    Ok(())
}
