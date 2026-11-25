// SPDX-License-Identifier: GPL-3.0-or-later
//! Assemble the resident edge columns, named points and lazy face rules for compute_snap.
use super::{
    acquisition,
    context::{self, Obj},
    crossing,
    edges::ResidentEdges,
    picking,
};
use crate::math::{xyz, Vec3};
use glam::DVec3;
use numpy::{IntoPyArray, PyReadonlyArray1, PyReadonlyArray2, PyUntypedArrayMethods};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet, PyTuple},
};
use std::collections::{HashMap, HashSet};

fn attr<'p>(o: &Obj<'p>, name: &str) -> PyResult<Obj<'p>> {
    Ok(o.getattr_opt(name)?
        .unwrap_or_else(|| o.py().None().into_bound(o.py())))
}
fn truth(o: &Obj<'_>, name: &str) -> PyResult<bool> {
    attr(o, name)?.is_truthy()
}
fn named<'p>(features: &Obj<'p>, p: Obj<'p>, tag: &str, word: Option<&str>) -> PyResult<()> {
    let kw = PyDict::new(p.py());
    kw.set_item("a", &p)?;
    kw.set_item("b", &p)?;
    kw.set_item(tag, true)?;
    kw.set_item("context", word)?;
    features.call_method1("append", (acquisition::record(p.py(), &kw)?,))?;
    Ok(())
}

#[pyclass]
struct SnapRules {
    index: Py<PyAny>,
    view: Py<PyAny>,
    row: Py<PyAny>,
    context: Py<PyAny>,
    point: Py<PyAny>,
    normal: Py<PyAny>,
    matrix: Py<PyAny>,
    width: f64,
    height: f64,
    x: f64,
    y: f64,
    #[pyo3(get)]
    snap_projection: Py<PyAny>,
    hidden: std::sync::Mutex<HashMap<[u64; 3], bool>>,
}
fn key(p: DVec3) -> [u64; 3] {
    p.to_array().map(|x| if x == 0. { 0 } else { x.to_bits() })
}
#[pymethods]
impl SnapRules {
    fn __traverse__(&self, visit: pyo3::PyVisit) -> Result<(), pyo3::PyTraverseError> {
        for value in [
            &self.index,
            &self.view,
            &self.row,
            &self.context,
            &self.point,
            &self.normal,
            &self.matrix,
            &self.snap_projection,
        ] {
            visit.call(value)?;
        }
        Ok(())
    }

    fn __call__<'p>(&self, py: Python<'p>, point: &Obj<'p>) -> PyResult<Obj<'p>> {
        self.matrix
            .bind(py)
            .call_method1("world_pixel", (point, self.width, self.height))
    }
    fn are_occluded(&self, py: Python<'_>, points: &Obj<'_>) -> PyResult<Vec<bool>> {
        let mut keys = Vec::new();
        let mut pending = Vec::new();
        let mut unique = HashSet::new();
        let missing = PyList::empty(py);
        {
            let memo = self.hidden.lock().unwrap();
            for p in points.try_iter()? {
                let p = p?;
                let k = key(xyz(&p)?);
                keys.push(k);
                if !memo.contains_key(&k) && unique.insert(k) {
                    pending.push(k);
                    missing.append(p)?;
                }
            }
        }
        if !pending.is_empty() {
            let values = context::occluded_many(
                self.index.bind(py),
                self.view.bind(py),
                missing.as_any(),
                None,
            )?;
            self.hidden
                .lock()
                .unwrap()
                .extend(pending.into_iter().zip(values));
        }
        let memo = self.hidden.lock().unwrap();
        Ok(keys.iter().map(|k| memo[k]).collect())
    }
    fn is_occluded(&self, py: Python<'_>, point: &Obj<'_>) -> PyResult<bool> {
        let k = key(xyz(point)?);
        if let Some(&v) = self.hidden.lock().unwrap().get(&k) {
            return Ok(v);
        }
        Ok(self.are_occluded(py, PyList::new(py, [point])?.as_any())?[0])
    }
    fn contains(slf: &Bound<'_, Self>, q: &Obj<'_>) -> PyResult<bool> {
        let t = slf.borrow();
        let py = slf.py();
        context::on_surface(
            t.index.bind(py),
            t.view.bind(py),
            t.row.bind(py),
            q,
            Some(slf.as_any()),
        )
    }
    fn outline<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        let t = slf.borrow();
        let py = slf.py();
        py.import("core.snap")?.call_method1(
            "SurfacePatch",
            (
                t.point.bind(py),
                t.normal.bind(py),
                slf.getattr("contains")?,
                context::placement_context(t.view.bind(py), t.row.bind(py), t.context.bind(py))?,
            ),
        )
    }
    fn crossing<'p>(slf: &Bound<'p, Self>) -> PyResult<Option<(Obj<'p>, Option<&'static str>)>> {
        let t = slf.borrow();
        let py = slf.py();
        crossing::face_crossing(
            t.index.bind(py),
            t.view.bind(py),
            t.x,
            t.y,
            t.row.bind(py),
            t.point.bind(py),
            t.normal.bind(py),
            None,
            slf.as_any(),
            t.context.bind(py),
        )
    }
}

struct Cursor<'p> {
    index: &'p Obj<'p>,
    view: &'p Obj<'p>,
    packed: Obj<'p>,
    camera: Obj<'p>,
    mvp: Obj<'p>,
    matrix_data: Obj<'p>,
    width: f64,
    height: f64,
    x: f64,
    y: f64,
    groups: Obj<'p>,
    context: Obj<'p>,
}
impl<'p> Cursor<'p> {
    fn gather(&self, rows: &Obj<'p>, reach: f64, cap: usize) -> PyResult<Obj<'p>> {
        self.index.call_method1(
            "edges_near",
            (
                rows,
                &self.matrix_data,
                self.width,
                self.height,
                self.x,
                self.y,
                reach,
                cap,
            ),
        )
    }
    fn columns(&self, picked: Option<i64>) -> PyResult<Obj<'p>> {
        let py = self.view.py();
        let rows = context::near_rows(self.index, self.view, &self.mvp, self.x, self.y, 64., None)?;
        let mut loose = Vec::new();
        let mut groups = Vec::new();
        for &r in rows.extract::<PyReadonlyArray1<i64>>()?.as_array() {
            if !self.groups.is_none() && self.groups.get_item(r)?.is(&self.context) {
                loose.push(r);
            } else {
                groups.push(r);
            }
        }
        let mut batches = Vec::new();
        if !loose.is_empty() {
            batches.push(self.gather(loose.into_pyarray(py).as_any(), 64., 400)?);
        }
        let multiple = groups.len() > 1;
        let near = context::near_rows(
            self.index,
            self.view,
            &self.mvp,
            self.x,
            self.y,
            48.,
            Some(groups.into_pyarray(py).as_any()),
        )?;
        let group = self.gather(&near, 48., 48)?;
        let extra = if let Some(r) = picked.filter(|_| multiple) {
            if near
                .extract::<PyReadonlyArray1<i64>>()?
                .as_array()
                .iter()
                .any(|&i| i == r)
            {
                let other = self.gather(vec![r].into_pyarray(py).as_any(), 48., 48)?;
                let ids = |v: &Obj<'p>, i| -> PyResult<Vec<i64>> {
                    Ok(v.get_item(i)?
                        .extract::<PyReadonlyArray1<i64>>()?
                        .as_array()
                        .to_vec())
                };
                let seen: HashSet<_> = ids(&group, 0)?.into_iter().zip(ids(&group, 1)?).collect();
                let keep: Vec<_> = ids(&other, 0)?
                    .into_iter()
                    .zip(ids(&other, 1)?)
                    .map(|k| !seen.contains(&k))
                    .collect();
                if keep.iter().any(|&b| b) {
                    let mask = keep.into_pyarray(py);
                    Some(
                        PyTuple::new(
                            py,
                            other
                                .try_iter()?
                                .map(|c| c?.get_item(&mask))
                                .collect::<PyResult<Vec<_>>>()?,
                        )?
                        .into_any(),
                    )
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        batches.push(group);
        if let Some(b) = extra {
            batches.push(b);
        }
        if batches.len() == 1 {
            return Ok(batches.pop().unwrap());
        }
        let np = py.import("numpy")?;
        let mut cols = Vec::new();
        for i in 0..5 {
            let pieces = batches
                .iter()
                .map(|b| b.get_item(i))
                .collect::<PyResult<Vec<_>>>()?;
            cols.push(np.call_method1("concatenate", (pieces,))?);
        }
        Ok(PyTuple::new(py, cols)?.into_any())
    }
    fn filter(&self, edges: Obj<'p>, tool: &Obj<'p>, moving: &Obj<'p>) -> PyResult<Obj<'p>> {
        let py = tool.py();
        let rows = edges.getattr("rows")?.extract::<PyReadonlyArray1<i64>>()?;
        let rows = rows.as_array();
        let mut keep = vec![true; rows.len()];
        if moving.is_truthy()? {
            let topo = py.import("core.topology")?;
            let key = topo.getattr("_key")?;
            let excluded = PySet::empty(py)?;
            for v in moving.try_iter()? {
                excluded.add(key.call1((v?.getattr("position")?,))?)?;
            }
            let round = py.import("builtins")?.getattr("round")?;
            let places = topo.getattr("_KEY_DECIMALS")?;
            let a = edges.getattr("a")?.extract::<PyReadonlyArray2<f64>>()?;
            let b = edges.getattr("b")?.extract::<PyReadonlyArray2<f64>>()?;
            let in_group = edges
                .call_method0("in_group")?
                .extract::<PyReadonlyArray1<bool>>()?;
            let in_group = in_group.as_array();
            for (i, k) in keep.iter_mut().enumerate() {
                if !in_group[i] {
                    for v in [a.as_array(), b.as_array()] {
                        let p = PyTuple::new(
                            py,
                            (0..3)
                                .map(|j| round.call1((v[[i, j]], &places)))
                                .collect::<PyResult<Vec<_>>>()?,
                        )?;
                        if excluded.contains(p)? {
                            *k = false;
                            break;
                        }
                    }
                }
            }
        }
        if let Some(f) = tool.getattr_opt("snap_excluded")? {
            let exclude = f.call0()?;
            if !exclude.is_none() {
                let ids = exclude.get_item(1)?;
                if ids.is_truthy()? {
                    let mut flags = HashMap::new();
                    for (i, &r) in rows.iter().enumerate() {
                        if keep[i] {
                            if let std::collections::hash_map::Entry::Vacant(entry) = flags.entry(r)
                            {
                                let g = if self.groups.is_none() {
                                    self.groups.clone()
                                } else {
                                    self.groups.get_item(r)?
                                };
                                entry.insert(!ids.contains(g.as_ptr() as usize)?);
                            }
                            keep[i] = flags[&r];
                        }
                    }
                }
            }
        }
        if keep.iter().all(|&k| k) {
            Ok(edges)
        } else {
            edges.call_method1("keep", (keep.into_pyarray(py),))
        }
    }
    fn features(&self, edges: &Obj<'p>, cols: &Obj<'p>, picked: Option<i64>) -> PyResult<bool> {
        let py = self.view.py();
        let rows = edges.getattr("rows")?.extract::<PyReadonlyArray1<i64>>()?;
        let src = cols.get_item(0)?.extract::<PyReadonlyArray1<i64>>()?;
        let ds = cols.get_item(4)?.extract::<PyReadonlyArray1<f64>>()?;
        let threshold = self
            .view
            .getattr_opt("pick_threshold_px")?
            .map(|o| o.extract())
            .transpose()?
            .unwrap_or(8.);
        let mut rim = false;
        let mut seen = HashSet::new();
        let features = edges.getattr("features")?;
        let overrides = self.index.getattr("rows")?;
        let instances = self.index.getattr("packed")?.getattr("instances")?;
        let fits = self.index.getattr("features")?;
        for r in rows.as_array().iter().copied().chain(picked) {
            if !seen.insert(r) {
                continue;
            }
            let g = if self.groups.is_none() {
                self.groups.clone()
            } else {
                self.groups.get_item(r)?
            };
            let instance = !g.is_none() && g.call_method0("is_instance")?.is_truthy()?;
            let value = match overrides.cast::<PyDict>()?.get_item(r)? {
                Some(value) => value,
                None => instances.get_item(r)?,
            };
            let f = fits.get_item(value.get_item("info")?.get_item(0)?)?;
            let centers = f.get_item(0)?;
            let mids = f.get_item(2)?;
            if !instance && centers.is_empty()? && mids.is_empty()? {
                continue;
            }
            let word =
                context::snap_context(self.view, r.into_pyobject(py)?.as_any(), &self.context)?;
            if !centers.is_empty()? || !mids.is_empty()? {
                let world = value
                    .get_item("world")?
                    .extract::<PyReadonlyArray2<f32>>()?;
                let world = world.as_array();
                let name = if g.is(&self.context) {
                    None
                } else if instance {
                    Some("component")
                } else {
                    Some("group")
                };
                for (seq, tag, kind) in [(&centers, "center", name), (&mids, "arc_midpoint", word)]
                {
                    for p in seq.try_iter()? {
                        let p = p?.extract::<[f64; 3]>()?;
                        let pos = DVec3::from_array(std::array::from_fn(|j| {
                            world[[0, j]] as f64 * p[0]
                                + world[[1, j]] as f64 * p[1]
                                + world[[2, j]] as f64 * p[2]
                                + world[[3, j]] as f64
                        }));
                        named(
                            &features,
                            Vec3::from(pos).into_pyobject(py)?.into_any(),
                            tag,
                            kind,
                        )?;
                    }
                }
                if !centers.is_empty()? {
                    rim |= src
                        .as_array()
                        .iter()
                        .zip(ds.as_array())
                        .any(|(&k, &d)| k == r && d <= threshold);
                }
            }
            if instance {
                let loc = self
                    .index
                    .call_method1("committed", (r,))?
                    .get_item(1)?
                    .extract::<PyReadonlyArray2<f64>>()?;
                let loc = loc.as_array();
                named(
                    &features,
                    Vec3::from(DVec3::new(loc[[3, 0]], loc[[3, 1]], loc[[3, 2]]))
                        .into_pyobject(py)?
                        .into_any(),
                    "component_origin",
                    word,
                )?;
            }
        }
        Ok(rim)
    }
}

#[pyfunction(signature=(index,row,view,x,y,**options))]
fn resident_snap<'p>(
    index: &'p Obj<'p>,
    row: &Obj<'p>,
    view: &'p Obj<'p>,
    x: f64,
    y: f64,
    options: Option<&Bound<'p, PyDict>>,
) -> PyResult<Obj<'p>> {
    let py = view.py();
    let none = py.None().into_bound(py);
    if index.is_none() {
        return Ok(none);
    }
    let opts = match options {
        Some(o) => o.copy()?,
        None => PyDict::new(py),
    };
    let pop = |name: &str, d: &Obj<'p>| opts.call_method1("pop", (name, d));
    let mvp = context::matrix(view)?;
    let packed = view.getattr("packed")?;
    let scene = view.getattr("scene")?;
    let context = scene.getattr("edit_group")?;
    let c = Cursor {
        index,
        view,
        matrix_data: mvp.call_method0("data")?,
        mvp,
        width: view.call_method0("width")?.extract()?,
        height: view.call_method0("height")?.extract()?,
        x,
        y,
        groups: attr(&packed, "groups")?,
        context,
        packed,
        camera: view.getattr("camera")?,
    };
    let info = c
        .packed
        .getattr("instances")?
        .get_item("info")?
        .extract::<PyReadonlyArray2<u32>>()?;
    // An empty scene's info table may come from an empty buffer whose data
    // pointer is not aligned for u32; never view it as an array.
    let picked = if info.len() == 0 {
        None
    } else {
        let info = info.as_array();
        row.extract::<i64>()
            .ok()
            .filter(|&r| r >= 0 && (r as usize) < info.nrows() && info[[r as usize, 2]] & 129 == 0)
    };
    let ray = view.call_method1("_pixel_to_ray", (x, y))?;
    let (o, d) = (ray.get_item(0)?, ray.get_item(1)?);
    if o.is_none() || d.is_none() {
        return Ok(none);
    }
    let engine = pop("inference", &none)?;
    let modifiers = pop("modifiers", &none)?;
    let hit = if picked.is_some() {
        picking::surface_hit(index, row, &o, &d, f64::INFINITY)?
    } else {
        None
    };
    let tools = attr(view, "tools")?;
    let tool = attr(&tools, "tool")?;
    let work = pop("work_point", py.Ellipsis().bind(py))?;
    let (mut point, normal, mut on_face) = match hit {
        Some((p, n)) => (p, n.into_pyobject(py)?.into_any(), true),
        None => {
            let mut p = if work.is(py.Ellipsis().bind(py)) {
                if tool.is_none() {
                    none.clone()
                } else if let Some(f) = tools.getattr_opt("work_point")? {
                    f.call1((x, y))?
                } else {
                    none.clone()
                }
            } else {
                work
            };
            if p.is_none() {
                let n = match c.camera.getattr_opt("forward")? {
                    Some(f) => xyz(&f.call0()?)?,
                    None => xyz(&d)?,
                };
                let origin = xyz(&o)?;
                let dir = xyz(&d)?;
                let target = xyz(&c.camera.getattr("target")?)?;
                let denom = dir.dot(n);
                p = Vec3::from(if denom.abs() > 1e-9 {
                    origin + dir * ((target - origin).dot(n) / denom)
                } else {
                    target
                })
                .into_pyobject(py)?
                .into_any();
            }
            (p, none.clone(), false)
        }
    };
    let cols = c.columns(picked)?;
    let edges = py
        .get_type::<ResidentEdges>()
        .call1((&cols, &c.groups, &c.context))?;
    let moving = if !attr(&tool, "grab")?.is_none() && !truth(&tool, "_copy")? {
        attr(&tool, "_verts")?
    } else {
        none.clone()
    };
    let edges = c.filter(edges, &tool, &moving)?;
    if moving.is_truthy()? {
        let p = tools.call_method1("work_point", (x, y))?;
        if p.is_truthy()? {
            point = p;
        }
        on_face = false;
    }
    let rim_candidate = c.features(&edges, &cols, picked)?;
    edges.setattr("prefix", context::scene_edges(view)?)?;
    let threshold = pop(
        "threshold_px",
        &view
            .getattr_opt("snap_threshold_px")?
            .unwrap_or(12f64.into_pyobject(py)?.into_any()),
    )?;
    opts.call_method1(
        "setdefault",
        ("edge_threshold_px", attr(view, "edge_snap_threshold_px")?),
    )?;
    let rules = Bound::new(
        py,
        SnapRules {
            index: index.clone().unbind(),
            view: view.clone().unbind(),
            row: row.clone().unbind(),
            context: c.context.clone().unbind(),
            point: point.clone().unbind(),
            normal: normal.clone().unbind(),
            matrix: c.mvp.clone().unbind(),
            width: c.width,
            height: c.height,
            x,
            y,
            snap_projection: (&c.matrix_data, (c.width, c.height))
                .into_pyobject(py)?
                .into_any()
                .unbind(),
            hidden: Default::default(),
        },
    )?;
    for (k, method) in [
        ("is_occluded", "is_occluded"),
        ("are_occluded", "are_occluded"),
    ] {
        if !opts.contains(k)? {
            opts.set_item(k, rules.getattr(method)?)?;
        }
    }
    opts.call_method1("setdefault", ("face_under_cursor", on_face))?;
    let source = attr(view, "source_viewport")?;
    let acquire = truth(&tool, "uses_snap")? && source.hasattr("_valid_center_ref")?;
    let rim = if acquire && rim_candidate {
        acquisition::acquire_rim(view, index, x, y)?
    } else {
        None
    };
    if !opts.contains("face_centers")? {
        opts.set_item(
            "face_centers",
            acquisition::face_centers_near(
                view,
                x,
                y,
                on_face,
                rules.as_any(),
                acquire && rim.is_none(),
            )?,
        )?;
    }
    if acquire {
        if let Some(r) = rim {
            source.setattr("_center_ref", r)?;
        }
        let r = source.call_method0("_valid_center_ref")?;
        if !r.is_none() {
            let p = r.get_item(0)?;
            let pixel = rules.call1((&p,))?;
            if !pixel.is_none()
                && source
                    .call_method1("_center_hint_px", (&r, pixel))?
                    .extract::<f64>()?
                    > 0.
            {
                named(&edges.getattr("features")?, p, "center", None)?;
            }
        }
    }
    if on_face && !normal.is_none() {
        opts.call_method1(
            "setdefault",
            (
                "face_context",
                context::snap_context(view, row, &c.context)?,
            ),
        )?;
        for (k, method) in [("face_outline", "outline"), ("face_crossing", "crossing")] {
            if !opts.contains(k)? {
                opts.set_item(k, rules.getattr(method)?)?;
            }
        }
    }
    let plane = attr(&tools, "_last_work_plane")?;
    if !plane.is_none() {
        opts.call_method1("setdefault", ("work_plane_normal", plane.get_item(1)?))?;
    } else if on_face && !normal.is_none() {
        opts.call_method1("setdefault", ("work_plane_normal", &normal))?;
    }
    if let Some(start) = opts.get_item("start_point")?.filter(|v| !v.is_none()) {
        if !opts.contains("start_edges")? {
            opts.set_item(
                "start_edges",
                acquisition::start_edges(index, view, &start, None)?,
            )?;
        }
    }
    if !engine.is_none() {
        acquisition::acquire(
            &engine,
            view,
            &edges,
            on_face,
            &point,
            &normal,
            &opts,
            x,
            y,
            if modifiers.is_none() {
                None
            } else {
                Some(&modifiers)
            },
        )?;
    }
    let fields = PyDict::new(py);
    fields.set_item("edges", edges)?;
    py.import("core.snap")?.getattr("compute_snap")?.call(
        (
            point,
            (x, y),
            acquisition::record(py, &fields)?,
            rules,
            threshold,
        ),
        Some(&opts),
    )
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(resident_snap, m)?)
}
