// SPDX-License-Identifier: GPL-3.0-or-later
//! Stable face-hit order and deferred hit resolution for a viewport ray.
use super::*;
use crate::pick::{ray_triangle, P};
use numpy::{PyReadonlyArray1, PyReadonlyArray2};
use pyo3::exceptions::{PyIndexError, PyValueError};
use std::collections::HashMap;

struct LazyFace<'py> {
    face: usize,
    faces: Obj<'py>,
    placement: Obj<'py>,
    t: f64,
    area: f64,
}

fn lazy_hits<'py>(
    view: &Obj<'py>,
    nat: &Obj<'py>,
    origin: P,
    direction: P,
    which: &str,
    reach: Option<f64>,
) -> PyResult<Vec<LazyFace<'py>>> {
    let rows = match which {
        "sel" => super::prototypes::pick_lazy_selectable(view, nat)?,
        "infer" => nat.get_item(1)?,
        _ => nat.get_item(2)?,
    };
    if rows.len()? == 0 {
        return Ok(Vec::new());
    }
    let query = nat.get_item(0)?.call_method1(
        "ray_cands",
        (
            rows,
            triple(origin),
            triple(direction),
            reach.unwrap_or(f64::INFINITY),
        ),
    )?;
    let (pl, tr, a, b, c): (
        PyReadonlyArray1<i64>,
        PyReadonlyArray1<i64>,
        PyReadonlyArray2<f64>,
        PyReadonlyArray2<f64>,
        PyReadonlyArray2<f64>,
    ) = query.extract()?;
    let (pl, tr, a, b, c) = (
        pl.as_array(),
        tr.as_array(),
        a.as_array(),
        b.as_array(),
        c.as_array(),
    );
    let n = pl.len();
    if tr.len() != n || [a.dim(), b.dim(), c.dim()].iter().any(|d| *d != (n, 3)) {
        return Err(PyValueError::new_err("inconsistent ray candidate columns"));
    }
    if n == 0 {
        return Ok(Vec::new());
    }
    let py = view.py();
    let host = py.import("views.document_interaction")?;
    let groups = view.call_method0("_pick_lazy_table")?.get_item(0)?;
    let positions = view.getattr("_pick_cands_memo")?.get_item(3)?;
    let mut placements = HashMap::new();
    let mut best = HashMap::<(usize, usize), f64>::new();
    for k in 0..n {
        let pt = |col: &numpy::ndarray::ArrayView2<f64>| [col[[k, 0]], col[[k, 1]], col[[k, 2]]];
        let Some(t) = ray_triangle(origin, direction, pt(&a), pt(&b), pt(&c)) else {
            continue;
        };
        let i = pl[k] as usize;
        if let std::collections::hash_map::Entry::Vacant(entry) = placements.entry(i) {
            let g = groups.get_item(i)?;
            let base = view.call_method1(
                "_proto_base_chunk",
                (
                    g.getattr("mesh")?,
                    host.call_method1("_paint_of", (view, &g))?,
                ),
            )?;
            let order = positions
                .get_item(g.as_ptr() as usize)?
                .get_item(0)?
                .extract::<usize>()?;
            let linear = host
                .call_method1("_placed_maps", (g.getattr("xform")?,))?
                .get_item(0)?;
            let det: f64 = py
                .import("numpy.linalg")?
                .call_method1("det", (linear,))?
                .extract()?;
            entry.insert((g, base, order, det.abs().powf(2. / 3.)));
        }
        let entity = placements[&i]
            .1
            .get_item("tri_ent")?
            .get_item(tr[k])?
            .extract::<usize>()?;
        best.entry((i, entity))
            .and_modify(|v| *v = v.min(t))
            .or_insert(t);
    }
    let mut keys = best.into_iter().collect::<Vec<_>>();
    keys.sort_unstable_by_key(|((i, e), _)| (placements[i].2, *e));
    keys.into_iter()
        .map(|((i, face), t)| {
            let (g, base, _, scale) = &placements[&i];
            Ok(LazyFace {
                face,
                faces: base.get_item("faces")?,
                placement: g.clone(),
                t,
                area: base.get_item("areas")?.get_item(face)?.extract::<f64>()? * scale,
            })
        })
        .collect()
}

#[pyclass]
struct FaceHits {
    fixed: Vec<usize>,
    idx: Py<PyAny>,
    // (deferred face collection, local face, placement)
    lazy: Vec<(Py<PyAny>, usize, Py<PyAny>)>,
    owner_of: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    t: Py<PyAny>,
    #[pyo3(get)]
    area: Py<PyAny>,
    #[pyo3(get)]
    loose: Py<PyAny>,
}

impl FaceHits {
    fn local(&self, k: usize) -> PyResult<&(Py<PyAny>, usize, Py<PyAny>)> {
        self.lazy
            .get(k - self.fixed.len())
            .ok_or_else(|| PyIndexError::new_err("face hit index"))
    }
}

#[pymethods]
impl FaceHits {
    fn __traverse__(&self, visit: pyo3::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        for v in [&self.idx, &self.t, &self.area, &self.loose] {
            visit.call(v)?;
        }
        visit.call(&self.owner_of)?;
        for (faces, _, g) in &self.lazy {
            visit.call(faces)?;
            visit.call(g)?;
        }
        Ok(())
    }
    fn ent<'py>(&self, py: Python<'py>, k: usize) -> PyResult<Obj<'py>> {
        if let Some(i) = self.fixed.get(k) {
            return self.idx.bind(py).getattr("entities")?.get_item(*i);
        }
        let (faces, face, _) = self.local(k)?;
        Ok((faces.bind(py).get_item(*face)?, self.owner(py, k)?)
            .into_pyobject(py)?
            .into_any())
    }
    fn owner<'py>(&self, py: Python<'py>, k: usize) -> PyResult<Obj<'py>> {
        if let Some(i) = self.fixed.get(k) {
            let entities = self.idx.bind(py).getattr("entities")?;
            return match entities.getattr_opt("owner_at")? {
                Some(f) if !f.is_none() => f.call1((*i,)),
                _ => entities.get_item(*i)?.get_item(1),
            };
        }
        let (_, _, g) = self.local(k)?;
        self.owner_of
            .as_ref()
            .ok_or_else(|| PyValueError::new_err("missing placement owner"))?
            .bind(py)
            .call1((g.bind(py),))
    }
    fn place<'py>(&self, py: Python<'py>, k: usize) -> PyResult<Obj<'py>> {
        if let Some(i) = self.fixed.get(k) {
            let idx = self.idx.bind(py);
            let mapping = attr(idx, "ent_place_idx")?;
            if !mapping.is_none() && *i < mapping.len()? {
                let j = mapping.get_item(*i)?.extract::<i64>()?;
                if j >= 0 {
                    return idx.getattr("ent_placements")?.get_item(j);
                }
            }
            return self.owner(py, k);
        }
        Ok(self.local(k)?.2.bind(py).clone())
    }
}

fn native<'py>(view: &Obj<'py>) -> PyResult<Obj<'py>> {
    match view.getattr_opt("_lazy_native")? {
        Some(f) => f.call0(),
        None => Ok(none(view.py())),
    }
}

#[pyfunction]
pub fn face_hits<'py>(
    view: &Obj<'py>,
    idx: &Obj<'py>,
    origin: &Obj<'py>,
    direction: &Obj<'py>,
    which: &str,
) -> PyResult<Obj<'py>> {
    let py = view.py();
    let nat = native(view)?;
    let mask = match which {
        "sel" => idx.getattr("ent_sel")?,
        "infer" => match idx.getattr_opt("ent_infer")? {
            Some(a) => a,
            None => idx.getattr("ent_sel")?,
        },
        _ => idx.getattr("ent_vis")?,
    };
    let fixed_t = if idx.getattr("entities")?.len()? > 0 {
        let kw = PyDict::new(py);
        kw.set_item("own", nat.is_none())?;
        view.call_method("_ray_hits", (idx, origin, direction, mask), Some(&kw))?
    } else {
        none(py)
    };
    let lazy = if nat.is_none() {
        Vec::new()
    } else {
        lazy_hits(
            view,
            &nat,
            xyz(origin)?.to_array(),
            xyz(direction)?.to_array(),
            which,
            None,
        )?
    };
    if fixed_t.is_none() && lazy.is_empty() {
        return Ok(none(py));
    }
    let mut fixed = Vec::new();
    let mut times = Vec::new();
    let mut areas = Vec::new();
    let mut loose = Vec::new();
    if !fixed_t.is_none() {
        let arr: PyReadonlyArray1<f64> = fixed_t.extract()?;
        let area = idx.getattr("ent_area")?;
        let flags = idx.getattr("ent_loose")?;
        for (i, t) in arr.as_array().iter().copied().enumerate() {
            if t.is_finite() {
                fixed.push(i);
                times.push(t);
                areas.push(area.get_item(i)?.extract::<f64>()?);
                loose.push(flags.get_item(i)?.is_truthy()?);
            }
        }
    }
    let owner_of = if lazy.is_empty() {
        None
    } else {
        Some(view.getattr("_owner_of")?.unbind())
    };
    let mut records = Vec::new();
    for row in lazy {
        times.push(row.t);
        areas.push(row.area);
        loose.push(false);
        records.push((row.faces.unbind(), row.face, row.placement.unbind()));
    }
    Ok(Bound::new(
        py,
        FaceHits {
            fixed,
            idx: idx.clone().unbind(),
            lazy: records,
            owner_of,
            t: times.into_pyarray(py).into_any().unbind(),
            area: areas.into_pyarray(py).into_any().unbind(),
            loose: loose.into_pyarray(py).into_any().unbind(),
        },
    )?
    .into_any())
}

#[pyfunction]
pub fn cached_face_hits<'py>(
    view: &Obj<'py>,
    idx: &Obj<'py>,
    origin: &Obj<'py>,
    direction: &Obj<'py>,
    infer: bool,
) -> PyResult<Obj<'py>> {
    let py = view.py();
    let scene = view.getattr("scene")?;
    let [ox, oy, oz] = xyz(origin)?.to_array();
    let [dx, dy, dz] = xyz(direction)?.to_array();
    let key = (
        scene.getattr("version")?,
        scene.getattr("mesh")?.as_ptr() as usize,
        ox,
        oy,
        oz,
        dx,
        dy,
        dz,
    )
        .into_pyobject(py)?;
    let nat = native(view)?;
    let slot = if infer {
        "_infer_hits_cache"
    } else {
        "_hover_hits_cache"
    };
    let cache = attr(view, slot)?;
    if !cache.is_none()
        && cache.get_item(0)?.eq(&key)?
        && cache.get_item(2)?.is(idx)
        && cache.get_item(3)?.is(&nat)
    {
        return cache.get_item(1);
    }
    let hits = face_hits(
        view,
        idx,
        origin,
        direction,
        if infer { "infer" } else { "sel" },
    )?;
    drop_past_cut(view, &hits, origin, direction)?;
    view.setattr(slot, (key, &hits, idx, nat))?;
    Ok(hits)
}

#[pyfunction]
pub fn index_ray_distance(
    view: &Obj,
    origin: &Obj,
    direction: &Obj,
    reach: Option<f64>,
) -> PyResult<Option<f64>> {
    let py = view.py();
    let nat = native(view)?;
    let kw = PyDict::new(py);
    if !nat.is_none() {
        kw.set_item("near", "skip")?;
    } else if let Some(r) = reach {
        kw.set_item("near", ("ray", origin, direction, r))?;
    } else {
        kw.set_item("near", ("ray", origin, direction))?;
    }
    let idx = view.call_method("_pick_index", (), Some(&kw))?;
    if idx.is_none()
        || (attr(&idx, "tri_v0")?.is_none()
            && nat.is_none()
            && !attr(&idx, "own_spans")?.is_truthy()?)
    {
        return Ok(None);
    }
    let args = PyDict::new(py);
    args.set_item("reduce_global", true)?;
    args.set_item("reach", reach)?;
    if !nat.is_none() {
        args.set_item("own", false)?;
    }
    let fixed = view
        .call_method(
            "_ray_hits",
            (&idx, origin, direction, idx.getattr("ent_vis")?),
            Some(&args),
        )?
        .extract::<Option<f64>>()?;
    let mut best = fixed.unwrap_or(f64::INFINITY);
    if !nat.is_none() {
        for hit in lazy_hits(
            view,
            &nat,
            xyz(origin)?.to_array(),
            xyz(direction)?.to_array(),
            "vis",
            reach,
        )? {
            best = best.min(hit.t);
        }
    }
    Ok((best.is_finite() && reach.is_none_or(|r| best <= r)).then_some(best))
}

#[pyfunction]
pub fn drop_past_cut(view: &Obj, hits: &Obj, origin: &Obj, direction: &Obj) -> PyResult<()> {
    if !hits.is_none() {
        let cut = view
            .py()
            .import("views.document_interaction")?
            .call_method1("_active_cut", (view.getattr("scene")?,))?;
        if !cut.is_none() {
            let n = xyz(&cut.getattr("normal")?)?;
            let c = n.dot(xyz(&cut.getattr("point")?)?);
            let o = xyz(origin)?;
            let d = xyz(direction)?;
            let t = hits.getattr("t")?.extract::<PyReadonlyArray1<f64>>()?;
            let clipped = t
                .as_array()
                .iter()
                .map(|&t| {
                    if (o + d * t).dot(n) - c > 1e-6 {
                        f64::INFINITY
                    } else {
                        t
                    }
                })
                .collect::<Vec<_>>();
            hits.setattr("t", clipped.into_pyarray(view.py()))?;
        }
    }
    Ok(())
}
