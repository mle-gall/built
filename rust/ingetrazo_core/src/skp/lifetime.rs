// SPDX-License-Identifier: GPL-3.0-or-later
//! The decoder transfers its private model to the worker. Drop source tables
//! after the final material variant that reads them, retaining image supports.
use super::tree::{attr, either, flagged, image, values, Obj};
use pyo3::{prelude::*, types::PyDict};
use std::collections::{HashMap, HashSet};
fn oid(d: &Obj<'_>) -> usize {
    d.as_ptr() as usize
}
fn children<'p>(d: &Obj<'p>, by_id: &Bound<'p, PyDict>) -> PyResult<Vec<(Obj<'p>, Obj<'p>)>> {
    let mut out = Vec::new();
    for ins in values(d, "instances")? {
        if let Some(child) = by_id.get_item(attr(&ins, "ref_idx")?)? {
            out.push((ins, child));
        }
    }
    Ok(out)
}
fn release(d: &Obj<'_>) -> PyResult<()> {
    let py = d.py();
    for k in ["faces", "edges", "vertices"] {
        d.setattr(k, PyDict::new(py))?;
    }
    let cache = py
        .import("ingetrazo_core.formats.skp_reader")?
        .getattr("_geometry_cache")?
        .call_method0("get")?;
    for k in ["_geom", "_soft_arr", "_loose_arr", "_face_boxes"] {
        if !cache.is_none() {
            cache.call_method1("forget", (d, k))?;
        }
        if d.hasattr(k)? {
            d.delattr(k)?;
        }
    }
    Ok(())
}
#[pyfunction]
pub fn image_stage_geometry(by_id: &Bound<'_, PyDict>) -> PyResult<HashSet<usize>> {
    let mut keep = HashSet::new();
    let mut pending = Vec::new();
    for (_, d) in by_id {
        if flagged(&d, "is_image")? {
            keep.insert(oid(&d));
        } else if flagged(&d, "always_faces_camera")? {
            pending.push(d);
        }
    }
    while let Some(d) = pending.pop() {
        if keep.insert(oid(&d)) {
            pending.extend(children(&d, by_id)?.into_iter().map(|(_, c)| c));
        }
    }
    Ok(keep)
}
fn protected(roots: &Obj<'_>, keep: Option<HashSet<usize>>) -> PyResult<HashSet<usize>> {
    let mut set = keep.unwrap_or_default();
    for r in roots.try_iter()? {
        set.insert(oid(&r?));
    }
    Ok(set)
}
#[pyclass(module = "ingetrazo_core")]
pub struct FlatGeometry {
    #[pyo3(get)]
    proved: bool,
    remaining: HashMap<usize, usize>,
}
fn height<'p>(
    d: &Obj<'p>,
    by_id: &Bound<'p, PyDict>,
    max_depth: usize,
    path: &mut HashSet<usize>,
    heights: &mut HashMap<usize, usize>,
) -> PyResult<Option<usize>> {
    let id = oid(d);
    if path.contains(&id) {
        return Ok(None);
    }
    if let Some(&n) = heights.get(&id) {
        return Ok(Some(n));
    }
    if path.len() > max_depth {
        return Ok(None);
    }
    path.insert(id);
    let mut h = 0;
    for (_, child) in children(d, by_id)? {
        let Some(n) = height(&child, by_id, max_depth, path, heights)? else {
            return Ok(None);
        };
        h = h.max(n + 1);
    }
    path.remove(&id);
    heights.insert(id, h);
    Ok(Some(h))
}
#[pymethods]
impl FlatGeometry {
    #[new]
    #[pyo3(signature=(by_id,proto_ids,uses,roots,max_depth,keep=None))]
    fn new(
        by_id: &Bound<'_, PyDict>,
        proto_ids: &Obj<'_>,
        uses: &Bound<'_, PyDict>,
        roots: &Obj<'_>,
        max_depth: usize,
        keep: Option<HashSet<usize>>,
    ) -> PyResult<Self> {
        let mut out = Self {
            proved: false,
            remaining: HashMap::new(),
        };
        let mut heights = HashMap::new();
        for root in roots.try_iter()? {
            if height(&root?, by_id, max_depth, &mut HashSet::new(), &mut heights)?
                .is_none_or(|n| n > max_depth)
            {
                return Ok(out);
            }
        }
        let mut skip = protected(roots, keep)?;
        let mut pending = Vec::new();
        for key in proto_ids.try_iter()? {
            if let Some(d) = by_id.get_item(key?)? {
                pending.push(d);
            }
        }
        let mut seen = HashSet::new();
        while let Some(d) = pending.pop() {
            if seen.insert(oid(&d)) {
                skip.insert(oid(&d));
                pending.extend(children(&d, by_id)?.into_iter().map(|(_, c)| c));
            }
        }
        for (key, d) in by_id {
            let id = oid(&d);
            if heights.contains_key(&id) && !skip.contains(&id) {
                if let Some(cnt) = uses.get_item(key)? {
                    let n = cnt.extract()?;
                    if n > 0 {
                        out.remaining.insert(id, n);
                    }
                }
            }
        }
        out.proved = true;
        Ok(out)
    }
    fn release(&mut self, definition: &Obj<'_>) -> PyResult<()> {
        let id = oid(definition);
        if let Some(n) = self.remaining.get_mut(&id) {
            *n -= 1;
            if *n == 0 {
                self.remaining.remove(&id);
                release(definition)?;
            }
        }
        Ok(())
    }
}
struct Reads<'p, 'a> {
    by_id: &'a Bound<'p, PyDict>,
    ids: &'a Obj<'p>,
    max_depth: usize,
    pending: Vec<Obj<'p>>,
}
impl<'p> Reads<'p, '_> {
    fn visit(
        &mut self,
        d: &Obj<'p>,
        paint: &Obj<'p>,
        path: &mut HashSet<usize>,
        reads: &mut HashSet<usize>,
    ) -> PyResult<()> {
        let id = oid(d);
        if path.len() > self.max_depth || !path.insert(id) {
            return Ok(());
        }
        reads.insert(id);
        for (ins, c) in children(d, self.by_id)? {
            if image(&c)? {
                continue;
            }
            let p = either(attr(&ins, "material_id")?, paint)?;
            let k = attr(&c, "id")?;
            if self.ids.contains(&k)? {
                self.pending.push((k, p).into_pyobject(d.py())?.into_any());
            } else {
                self.visit(&c, &p, path, reads)?;
            }
        }
        path.remove(&id);
        Ok(())
    }
}
#[pyclass(module = "ingetrazo_core")]
pub struct PrototypeGeometry {
    definitions: HashMap<usize, Py<PyAny>>,
    reads: Py<PyDict>,
    remaining: HashMap<usize, usize>,
    protected: HashSet<usize>,
}
#[pymethods]
impl PrototypeGeometry {
    #[new]
    #[pyo3(signature=(by_id,proto_ids,proto_uses,roots,max_depth,keep=None))]
    fn new<'p>(
        by_id: &Bound<'p, PyDict>,
        proto_ids: &Obj<'p>,
        proto_uses: &Obj<'p>,
        roots: &Obj<'p>,
        max_depth: usize,
        keep: Option<HashSet<usize>>,
    ) -> PyResult<Self> {
        let py = by_id.py();
        let reads = PyDict::new(py);
        let mut remaining = HashMap::new();
        let mut walker = Reads {
            by_id,
            ids: proto_ids,
            max_depth,
            pending: proto_uses.try_iter()?.collect::<PyResult<_>>()?,
        };
        while let Some(key) = walker.pending.pop() {
            if reads.contains(&key)? {
                continue;
            }
            let mut group = HashSet::new();
            if let Some(d) = by_id.get_item(key.get_item(0)?)? {
                walker.visit(&d, &key.get_item(1)?, &mut HashSet::new(), &mut group)?;
            }
            for &id in &group {
                *remaining.entry(id).or_insert(0) += 1;
            }
            reads.set_item(key, group)?;
        }
        let protected = protected(roots, keep)?;
        let mut definitions = HashMap::new();
        for (_, d) in by_id {
            let id = oid(&d);
            if !protected.contains(&id) && !remaining.contains_key(&id) {
                release(&d)?;
            } else {
                definitions.insert(id, d.unbind());
            }
        }
        Ok(Self {
            reads: reads.unbind(),
            remaining,
            definitions,
            protected,
        })
    }
    #[getter]
    fn remaining(&self) -> HashMap<usize, usize> {
        self.remaining.clone()
    }
    fn release(&mut self, key: &Obj<'_>) -> PyResult<()> {
        let set = self
            .reads
            .bind(key.py())
            .call_method1("pop", (key,))?
            .extract::<HashSet<usize>>()?;
        for id in set {
            let n = self
                .remaining
                .get_mut(&id)
                .expect("each dependency counted once");
            *n -= 1;
            if *n == 0 {
                self.remaining.remove(&id);
                if let Some(d) = self.definitions.remove(&id) {
                    if !self.protected.contains(&id) {
                        release(d.bind(key.py()))?;
                    }
                }
            }
        }
        Ok(())
    }
}
