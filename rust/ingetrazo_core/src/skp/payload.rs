// SPDX-License-Identifier: GPL-3.0-or-later
//! Install script-provided geometry using the active mesh backend.
use super::apply::{apply_geometry, opt, rows, skp_apply_metadata};
use crate::file_model::{assign, dae_add_fused, new_mesh};
use crate::math::{xyz, Vec3};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::HashMap;
type Obj<'p> = Bound<'p, PyAny>;
fn point<'p>(p: &Obj<'p>) -> PyResult<Obj<'p>> {
    if p.hasattr("x")? {
        return Ok(p.clone());
    }
    Ok(Py::new(p.py(), Vec3::from(xyz(p)?))?
        .into_bound(p.py())
        .into_any())
}
fn ring<'p>(r: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let l = PyList::empty(r.py());
    for p in r.try_iter()? {
        l.append(point(&p?)?)?;
    }
    Ok(l)
}
#[pyclass]
struct Progress {
    clock: Py<PyAny>,
    report: Option<Py<PyAny>>,
    total: usize,
    done: usize,
    last: f64,
}
#[pymethods]
impl Progress {
    #[pyo3(signature=(step=0))]
    fn __call__(&mut self, py: Python<'_>, step: usize) -> PyResult<()> {
        self.done += step;
        if let Some(cb) = &self.report {
            let now = self.clock.call0(py)?.extract::<f64>(py)?;
            if now - self.last >= 0.15 {
                self.last = now;
                cb.call1(
                    py,
                    (
                        0.75 + 0.21 * (self.done as f64 / self.total as f64).min(1.),
                        "Building geometry…",
                    ),
                )?;
            }
        }
        Ok(())
    }
}
fn edge<'p>(mesh: &Obj<'p>, a: &Obj<'p>, b: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    let a = mesh.call_method1("vertex_at", (point(a)?,))?;
    let b = mesh.call_method1("vertex_at", (point(b)?,))?;
    if a.is_none() || b.is_none() || a.is(&b) {
        return Ok(None);
    }
    Ok(Some(mesh.call_method1("find_edge", (a, b))?).filter(|e| !e.is_none()))
}
#[pyfunction]
pub fn skp_soften_bulk(
    mesh: &Obj<'_>,
    vertices: &Obj<'_>,
    edge_map: &Obj<'_>,
    segments: &Obj<'_>,
    tick: &Obj<'_>,
) -> PyResult<()> {
    let mut ids = HashMap::new();
    let mut points = HashMap::new();
    let key = |p: &Obj<'_>| -> PyResult<[u64; 3]> {
        Ok(xyz(p)?
            .to_array()
            .map(|v| if v == 0. { 0 } else { v.to_bits() }))
    };
    for (i, v) in vertices.try_iter()?.enumerate() {
        let v = v?;
        points.insert(key(&v.getattr("position")?)?, Some(i));
        ids.insert(v.as_ptr(), i);
    }
    let mut lookup = |p: &Obj<'_>| -> PyResult<Option<usize>> {
        let k = key(p)?;
        if let Some(idx) = points.get(&k) {
            return Ok(*idx);
        }
        let v = mesh.call_method1("vertex_at", (point(p)?,))?;
        let idx = if v.is_none() {
            None
        } else {
            // Stores can return fresh Python proxies for the same vertex.
            ids.get(&v.as_ptr()).copied().or_else(|| {
                vertices
                    .call_method1("index", (&v,))
                    .ok()
                    .and_then(|n| n.extract().ok())
            })
        };
        points.insert(k, idx);
        Ok(idx)
    };
    let count = vertices.len()?;
    for (i, row) in segments.try_iter()?.enumerate() {
        if i % 4096 == 0 {
            tick.call0()?;
        }
        let row = row?;
        let (Some(a), Some(b)) = (lookup(&row.get_item(0)?)?, lookup(&row.get_item(1)?)?) else {
            continue;
        };
        if a == b {
            continue;
        }
        let e = if edge_map.is_none() {
            mesh.call_method1("find_edge", (vertices.get_item(a)?, vertices.get_item(b)?))?
        } else {
            edge_map.call_method1("get", (a.min(b) * count + a.max(b),))?
        };
        if !e.is_none() {
            e.setattr("soft", true)?;
        }
    }
    Ok(())
}
fn faces<'p>(mesh: &Obj<'p>, input: &Obj<'p>, soft: &Obj<'p>, tick: &Obj<'p>) -> PyResult<()> {
    let py = mesh.py();
    let raw = PyList::empty(py);
    let flat = PyList::empty(py);
    let mut data = vec![];
    let mut sizes = vec![];
    let mut counts = vec![];
    let attrs = PyList::empty(py);
    let mut corners = 0;
    for item in input.try_iter()? {
        let (outer, holes, attr) = item?.extract::<(Obj, Obj, Obj)>()?;
        let mut rings = vec![outer];
        if holes.is_truthy()? {
            for h in holes.try_iter()? {
                rings.push(h?);
            }
        }
        if !soft.is_none() && rings.len() > 2 {
            let hh = PyList::empty(py);
            for h in &rings[1..] {
                hh.append(ring(h)?)?;
            }
            let Ok(kept) = py
                .import("core.topology")?
                .call_method1("_maximal_holes", (hh,))
            else {
                continue;
            };
            rings.truncate(1);
            for h in kept.try_iter()? {
                let points = PyList::empty(py);
                for p in h?.try_iter()? {
                    points.append(p?.call_method0("toTuple")?)?;
                }
                rings.push(points.into_any());
            }
        }
        if soft.is_none() && rings.len() == 1 {
            raw.append((ring(&rings[0])?, &attr))?;
            continue;
        }
        counts.push(rings.len());
        for r in &rings {
            sizes.push(r.len()?);
            corners += r.len()?;
            flat.append(r)?;
        }
        attrs.append(&attr)?;
        data.push((rings, attr));
    }
    if soft.is_none() {
        let fused = py
            .import("formats.fuse")?
            .call_method1("fuse_coplanar_loops", (raw,))?;
        dae_add_fused(mesh, &fused)?;
    }
    if !soft.is_none() && corners >= 1024 {
        let coords = py
            .import("formats.skp_coordinates")?
            .call_method1("flatten_rings", (flat, corners))?;
        let (v, inv) = mesh
            .call_method1("bulk_weld", (coords,))?
            .extract::<(Obj, Obj)>()?;
        let map = if soft.is_truthy()?
            && mesh
                .get_type()
                .is(&py.import("core.mesh")?.getattr("Mesh")?)
        {
            PyDict::new(py).into_any()
        } else {
            py.None().into_bound(py)
        };
        let kw = PyDict::new(py);
        kw.set_item("edge_map", &map)?;
        mesh.call_method(
            "add_faces_welded",
            (&v, inv, sizes, counts, attrs),
            Some(&kw),
        )?;
        if soft.is_truthy()? {
            py.import("formats.skp")?
                .call_method1("_soften_bulk_edges", (mesh, v, map, soft, tick))?;
        }
    } else {
        for (rings, attr) in data {
            let holes = PyList::empty(py);
            for h in &rings[1..] {
                holes.append(ring(h)?)?;
            }
            if let Ok(f) = mesh.call_method1(
                "add_face",
                (ring(&rings[0])?, (!holes.is_empty()).then_some(holes)),
            ) {
                assign(&f, &attr)?;
            }
        }
        if soft.is_none() {
            py.import("formats.fuse")?
                .call_method1("soften_smooth_edges", (mesh,))?;
        } else {
            for (i, row) in soft.try_iter()?.enumerate() {
                if i % 4096 == 0 {
                    tick.call0()?;
                }
                let row = row?;
                if let Some(e) = edge(mesh, &row.get_item(0)?, &row.get_item(1)?)? {
                    e.setattr("soft", true)?;
                }
            }
        }
    }
    Ok(())
}
fn build<'p>(rec: &Obj<'p>, tick: &Obj<'p>) -> PyResult<Obj<'p>> {
    let mesh = new_mesh(rec.py())?;
    faces(
        &mesh,
        &rec.get_item("faces")?,
        &opt(rec, "soft_edges")?,
        tick,
    )?;
    for row in rows(rec, "loose_edges")?.try_iter()? {
        let row = row?;
        if let Err(e) = mesh.call_method1(
            "add_edge",
            (point(&row.get_item(0)?)?, point(&row.get_item(1)?)?),
        ) {
            if !e.is_instance_of::<PyValueError>(rec.py()) {
                return Err(e);
            }
        }
    }
    for stroke in rows(rec, "curves")?.try_iter()? {
        let id = mesh.call_method0("next_curve_id")?;
        for row in stroke?.try_iter()? {
            let row = row?;
            if let Some(e) = edge(&mesh, &row.get_item(0)?, &row.get_item(1)?)? {
                e.setattr("curve", &id)?;
            }
        }
    }
    Ok(mesh)
}

pub(super) fn apply<'p>(
    scene: &Obj<'p>,
    payload: &Bound<'p, PyDict>,
    progress: Option<&Obj<'p>>,
    consume: bool,
) -> PyResult<String> {
    let py = scene.py();
    let mut uses = HashMap::new();
    let mut total = 0;
    for (key, weight) in [("groups", 1), ("protos", 2)] {
        let records = rows(payload, key)?;
        total += records.len()? * weight;
        if consume {
            for rec in records.try_iter()? {
                *uses.entry(rec?.as_ptr()).or_insert(0usize) += 1;
            }
        }
    }
    let tick = Py::new(
        py,
        Progress {
            clock: py.import("time")?.getattr("monotonic")?.unbind(),
            report: progress.cloned().map(Bound::unbind),
            total: total.max(1),
            done: 0,
            last: f64::NEG_INFINITY,
        },
    )?
    .into_bound(py)
    .into_any();
    skp_apply_metadata(scene, payload)?;
    apply_geometry(
        scene,
        payload,
        |rec| {
            let mesh = build(rec, &tick)?;
            if let Some(left) = uses.get_mut(&rec.as_ptr()) {
                *left -= 1;
                if *left == 0 {
                    rec.set_item("faces", PyList::empty(py))?;
                    for key in ["soft_edges", "loose_edges", "curves"] {
                        rec.call_method1("pop", (key, py.None()))?;
                    }
                }
            }
            Ok(mesh)
        },
        |step| {
            tick.call1((step,))?;
            Ok(())
        },
    )?;
    super::apply::finish(scene, payload, consume)
}
