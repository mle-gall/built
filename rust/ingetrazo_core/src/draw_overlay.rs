// SPDX-License-Identifier: GPL-3.0-or-later
//! A drawing plan overlays edits on the scan at its starting revision. Original
//! elements keep their scan row; replacements occupy that row and appends follow.
use crate::{
    drawing_queries::{identity, item_box, meets, window, Bounds},
    math::{xyz, Vec3},
};
use pyo3::{
    exceptions::{PyIndexError, PyValueError},
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::{BTreeMap, HashMap, HashSet};
type Obj<'py> = Bound<'py, PyAny>;
type Position = (&'static str, usize);
type Segment<'py> = (Obj<'py>, Obj<'py>);
type Hit = [[f64; 3]; 2];

struct Piece {
    item: Py<PyAny>,
    bounds: Option<Bounds>,
}
impl Piece {
    fn new(item: Obj<'_>) -> Self {
        Self {
            item: item.unbind(),
            bounds: None,
        }
    }
    fn meets(&mut self, py: Python<'_>, w: Bounds) -> PyResult<bool> {
        if self.bounds.is_none() {
            self.bounds = Some(item_box(self.item.bind(py))?);
        }
        Ok(meets(self.bounds.unwrap(), w))
    }
}

#[pyclass(dict)]
pub(crate) struct ScanOverlay {
    scan: Py<PyAny>,
    edges: bool,
    got: HashMap<usize, Py<PyAny>>,
    at: HashMap<i128, usize>,
    dead: HashSet<usize>,
    replaced: BTreeMap<usize, Piece>,
    extra: Vec<Piece>,
}
impl ScanOverlay {
    fn elements<'py>(&mut self, py: Python<'py>, rows: &[usize]) -> PyResult<Vec<Obj<'py>>> {
        let missing: Vec<_> = rows
            .iter()
            .copied()
            .filter(|i| !self.got.contains_key(i))
            .collect();
        if !missing.is_empty() {
            let kind = if self.edges { "edges" } else { "faces" };
            let items = self.scan.bind(py).call_method1(kind, (&missing,))?;
            for (i, item) in missing.into_iter().zip(items.try_iter()?) {
                let item = item?;
                self.at.insert(identity(&item)?, i);
                self.got.insert(i, item.unbind());
            }
        }
        rows.iter()
            .map(|i| {
                self.got
                    .get(i)
                    .map(|e| e.bind(py).clone())
                    .ok_or_else(|| PyIndexError::new_err(*i))
            })
            .collect()
    }
    fn native<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, crate::scans::MeshScan>> {
        Ok(self.scan.bind(py).getattr("native")?.cast_into()?)
    }
    fn rows(&self, py: Python<'_>, w: Bounds) -> PyResult<Vec<usize>> {
        let native = self.native(py)?;
        let mut scan = native.try_borrow_mut()?;
        let (lo, hi) = ((w[0], w[1], w[2]), (w[3], w[4], w[5]));
        let hits = if self.edges {
            scan.edges_meeting(py, lo, hi)?
        } else {
            scan.faces_meeting(py, lo, hi)?
        };
        Ok(hits
            .into_iter()
            .map(|i| i as usize)
            .filter(|i| !self.dead.contains(i) && !self.replaced.contains_key(i))
            .collect())
    }
    fn extras<'py>(&mut self, py: Python<'py>, w: Bounds) -> PyResult<Vec<Obj<'py>>> {
        let mut result = Vec::new();
        for p in &mut self.extra {
            if p.meets(py, w)? {
                result.push(p.item.bind(py).clone());
            }
        }
        Ok(result)
    }
    fn position(&self, item: &Obj<'_>) -> PyResult<Option<Position>> {
        let py = item.py();
        if let Some(&i) = self.at.get(&identity(item)?) {
            if !self.dead.contains(&i)
                && !self.replaced.contains_key(&i)
                && self.got[&i].bind(py).eq(item)?
            {
                return Ok(Some(("base", i)));
            }
        }
        for (&i, p) in &self.replaced {
            if !self.dead.contains(&i) && p.item.bind(py).eq(item)? {
                return Ok(Some(("base", i)));
            }
        }
        for (i, p) in self.extra.iter().enumerate() {
            if p.item.bind(py).eq(item)? {
                return Ok(Some(("extra", i)));
            }
        }
        Ok(None)
    }
    fn ends(&self, py: Python<'_>) -> PyResult<Vec<Hit>> {
        self.extra
            .iter()
            .map(|p| endpoints(p.item.bind(py)))
            .collect()
    }
}
fn endpoints(e: &Obj<'_>) -> PyResult<Hit> {
    Ok([
        xyz(&e.getattr("a")?)?.to_array(),
        xyz(&e.getattr("b")?)?.to_array(),
    ])
}

#[pymethods]
impl ScanOverlay {
    #[new]
    fn new(scan: Py<PyAny>, edges: bool) -> Self {
        Self {
            scan,
            edges,
            got: HashMap::new(),
            at: HashMap::new(),
            dead: HashSet::new(),
            replaced: BTreeMap::new(),
            extra: Vec::new(),
        }
    }
    fn pick<'py>(&mut self, pts: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
        let py = pts.py();
        let w = window(pts)?;
        let rows = self.rows(py, w)?;
        let items = self.elements(py, &rows)?;
        let mut sorted: Vec<_> = rows.into_iter().zip(items).collect();
        for (&i, p) in &mut self.replaced {
            if !self.dead.contains(&i) && p.meets(py, w)? {
                sorted.push((i, p.item.bind(py).clone()));
            }
        }
        if !self.replaced.is_empty() {
            sorted.sort_by_key(|(i, _)| *i);
        }
        let mut out: Vec<_> = sorted.into_iter().map(|(_, e)| e).collect();
        out.extend(self.extras(py, w)?);
        Ok(out)
    }
    fn __contains__(&self, item: &Obj<'_>) -> PyResult<bool> {
        Ok(self.position(item)?.is_some())
    }
    fn index(&self, item: &Obj<'_>) -> PyResult<Position> {
        self.position(item)?
            .ok_or_else(|| PyValueError::new_err("element is not in the drawing plan"))
    }
    fn remove(&mut self, item: &Obj<'_>) -> PyResult<()> {
        let (kind, i) = self.index(item)?;
        if kind == "base" {
            self.dead.insert(i);
        } else {
            self.extra.remove(i);
        }
        Ok(())
    }
    fn append(&mut self, item: Obj<'_>) {
        self.extra.push(Piece::new(item));
    }
    fn __setitem__(&mut self, pos: (&str, usize), item: Obj<'_>) -> PyResult<()> {
        let piece = Piece::new(item);
        if pos.0 == "base" {
            self.replaced.insert(pos.1, piece);
        } else {
            *self
                .extra
                .get_mut(pos.1)
                .ok_or_else(|| PyIndexError::new_err(pos.1))? = piece;
        }
        Ok(())
    }
    fn all_elements<'py>(&mut self, py: Python<'py>) -> PyResult<Vec<Obj<'py>>> {
        let (_, edges, faces) = self.native(py)?.try_borrow()?.counts(py);
        let n = if self.edges { edges } else { faces };
        let indices: Vec<_> = (0..n).filter(|i| !self.dead.contains(i)).collect();
        let mut items = self.elements(py, &indices)?;
        for (row, item) in indices.into_iter().zip(&mut items) {
            if let Some(p) = self.replaced.get(&row) {
                *item = p.item.bind(py).clone();
            }
        }
        items.extend(self.extra.iter().map(|p| p.item.bind(py).clone()));
        Ok(items)
    }
    fn cycle_adjacency<'py>(&mut self, ka: &Obj<'py>, kb: &Obj<'py>) -> PyResult<Obj<'py>> {
        let py = ka.py();
        py.import("core.topology")?
            .call_method1("_cycle_adjacency", (self.all_elements(py)?, ka, kb))
    }
    #[pyo3(signature=(a,b,*,whole=false))]
    fn split_plan<'py>(
        &mut self,
        a: &Obj<'py>,
        b: &Obj<'py>,
        whole: bool,
    ) -> PyResult<(Vec<Segment<'py>>, Bound<'py, PyDict>)> {
        let py = a.py();
        let w = window(PyList::new(py, [a, b])?.as_any())?;
        let native = self.native(py)?;
        let rows = if whole {
            (0..native.try_borrow()?.counts(py).1).collect()
        } else {
            self.rows(py, w)?
        };
        let extra = self.extras(py, w)?;
        let ends = extra.iter().map(endpoints).collect::<PyResult<Vec<_>>>()?;
        let hypot = py
            .import("core.store_mesh")?
            .call_method0("_hypot")?
            .extract::<Option<String>>()?
            .unwrap_or_else(|| "libstdc++".into());
        let (hits, cuts) = native.try_borrow_mut()?.split_plan(
            py,
            xyz(a)?.to_array(),
            xyz(b)?.to_array(),
            rows.clone(),
            ends,
            &hypot,
            py.version_info().minor >= 12,
        )?;
        let mut chain = vec![a.clone()];
        for p in hits {
            chain.push(
                Vec3::from(glam::DVec3::from_array(p))
                    .into_pyobject(py)?
                    .into_any(),
            );
        }
        chain.push(b.clone());
        let result = PyDict::new(py);
        for (i, p) in cuts {
            let e = if let Some(&row) = rows.get(i) {
                self.elements(py, &[row])?.remove(0)
            } else {
                extra
                    .get(i - rows.len())
                    .ok_or_else(|| PyIndexError::new_err(i))?
                    .clone()
            };
            result.set_item(e, Vec3::from(glam::DVec3::from_array(p)))?;
        }
        Ok((
            chain
                .windows(2)
                .map(|p| (p[0].clone(), p[1].clone()))
                .collect(),
            result,
        ))
    }
    fn drawing_cycles(
        &self,
        a: &Obj<'_>,
        b: &Obj<'_>,
        limit: isize,
        both: bool,
    ) -> PyResult<Vec<Vec<Vec3>>> {
        let py = a.py();
        let cycles = self.native(py)?.try_borrow_mut()?.drawing_cycles(
            py,
            xyz(a)?.to_array(),
            xyz(b)?.to_array(),
            self.dead.iter().copied().collect(),
            self.ends(py)?,
            limit.max(0) as usize,
            both,
        )?;
        Ok(cycles
            .into_iter()
            .map(|r| {
                r.into_iter()
                    .map(|p| Vec3::from(glam::DVec3::from_array(p)))
                    .collect()
            })
            .collect())
    }
}
