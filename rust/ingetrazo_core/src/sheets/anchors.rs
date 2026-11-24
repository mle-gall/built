// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! Sheet items bound to the model follow it (the composer's
//! `_reproject_anchored_cotas`, `_resnap_anchor`, `_frame_world_to_page`):
//! each bound point first asks its hold (the model's own attachment, which
//! follows its vertex or edge however far it moved); failing that, a point
//! still inside its frame re-attaches to the nearest of the frame's current
//! snap points within 2.5 paper mm (0.5 m at most), or along an edge;
//! then the item is reprojected through its frame's camera, so moving,
//! rescaling or re-viewing the frame keeps it true. Derived state, like the
//! renders: never a step of the history.

use super::camera::{frame_camera, Cam, ViewCam};
use crate::doc;
use glam::DVec3;
use numpy::{PyReadonlyArray2, PyReadonlyArray3};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

/// `_ON_EDGE_TOL_M`: a point this close to an edge sits on it.
const ON_EDGE_TOL_M: f64 = 1e-4;

/// A frame's projection: world metres → page mm (`frame_page_projector`
/// after `_to_cam`).
#[derive(Clone)]
pub struct Projector {
    cam: Cam,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    scale_n: f64,
    perspective: bool,
    projection: super::projection::PageProjection,
}

impl Projector {
    pub fn page(&self, p: [f64; 3]) -> [f64; 2] {
        let v = self.cam.view_matrix().transform_point3(DVec3::from_array(p));
        let [x,y] = self.projection.local(v.x, v.y, -v.z);
        [self.x+x, self.y+y]
    }
    /// The projector of ``frame`` from the frame camera's inputs ``d``
    /// (``{view, base, bounds}``, see ``sheet_frame_camera``): onto the
    /// page, or onto the frame itself (``local``: frame-local mm).
    pub fn of(frame: &Bound<'_, PyAny>, d: &Bound<'_, PyDict>, local: bool)
        -> PyResult<(super::model::Fields, Projector)> {
        let f = super::model::read_fields(doc::py::handle_key(frame)?)?;
        let view = match d.get_item("view")?.filter(|v| !v.is_none()) {
            Some(v) => Some(ViewCam::from_fields(&super::model::read_fields(doc::py::handle_key(&v)?)?)),
            None => None,
        };
        let base = super::camera::base_cam_of(d.get_item("base")?.map(|b| b.cast_into::<PyDict>()).transpose()?.as_ref())?;
        let bounds: Option<((f64, f64, f64), (f64, f64, f64))> = match d.get_item("bounds")? {
            Some(b) if !b.is_none() => Some(b.extract()?),
            _ => None,
        };
        let bounds = bounds.map(|(a, b)| (DVec3::new(a.0, a.1, a.2), DVec3::new(b.0, b.1, b.2)));
        let cam = frame_camera(base, &f, view.as_ref(), bounds);
        let (x, y) = if local { (0.0, 0.0) } else { (f.num("x_mm", 0.0), f.num("y_mm", 0.0)) };
        let projection = super::projection::PageProjection::for_frame(
            f.num("w_mm",100.), f.num("h_mm",100.), f.num("scale_n",100.),
            cam.perspective.then_some(cam.fov_deg));
        let proj = Projector { projection, perspective: cam.perspective, cam, x, y, w: f.num("w_mm", 100.0),
                               h: f.num("h_mm", 100.0), scale_n: f.num("scale_n", 100.0) };
        Ok((f, proj))
    }
    /// The direction the frame's camera looks along.
    pub fn forward(&self) -> DVec3 {
        self.cam.forward()
    }
    pub fn perspective(&self) -> bool {
        self.perspective
    }
    fn inside(&self, p: [f64; 2]) -> bool {
        self.x - 0.5 <= p[0] && p[0] <= self.x + self.w + 0.5 && self.y - 0.5 <= p[1] && p[1] <= self.y + self.h + 0.5
    }
    fn tol(&self) -> f64 {
        (2.5 * self.scale_n / 1000.0).min(0.5)
    }
}

/// `_resnap_anchor`: where a bound world point goes when the model may
/// have moved under it. A point on a corner goes with the nearest point
/// within `tol`; one placed along an edge stays while that edge passes
/// through it (`held`: whether it was first seen on an edge); failing
/// both, the nearest spot along an edge within `tol`. Returns the point
/// and whether it is now held as on an edge.
pub fn resnap(w: [f64; 3], wpts: &[[f64; 3]], edges: &[[[f64; 3]; 2]], tol: f64, held: Option<bool>) -> ([f64; 3], Option<bool>) {
    let d2 = |a: [f64; 3], b: [f64; 3]| (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>();
    let tol_on = ON_EDGE_TOL_M * ON_EDGE_TOL_M;
    let mut point = None;
    if let Some((i, best)) = wpts.iter().map(|p| d2(*p, w)).enumerate().min_by(|a, b| a.1.total_cmp(&b.1)) {
        if best <= tol_on {
            return (wpts[i], Some(false));
        }
        if best <= tol * tol {
            point = Some(wpts[i]);
        }
    }
    let mut foot = None;
    let mut on_edge = false;
    let mut best = f64::INFINITY;
    let mut best_foot = [0.0; 3];
    for [a, b] in edges {
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let l2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
        let t = if l2 > 1e-18 {
            (((w[0] - a[0]) * ab[0] + (w[1] - a[1]) * ab[1] + (w[2] - a[2]) * ab[2]) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let f = [a[0] + t * ab[0], a[1] + t * ab[1], a[2] + t * ab[2]];
        let dist = d2(f, w);
        if dist < best {
            best = dist;
            best_foot = f;
        }
    }
    if best.is_finite() {
        on_edge = best <= tol_on;
        if best <= tol * tol {
            foot = Some(best_foot);
        }
    }
    let placed_on_edge = held.unwrap_or(on_edge);
    if on_edge && placed_on_edge {
        return (w, Some(true));
    }
    match point.or(foot) {
        None => (w, held),
        Some(now) => (now, Some(point.is_none())),
    }
}

fn v3(v: &Bound<'_, PyAny>) -> PyResult<Option<[f64; 3]>> {
    if v.is_none() || !v.is_truthy()? {
        return Ok(None);
    }
    let c: Vec<f64> = v.extract()?;
    Ok((c.len() >= 3).then(|| [c[0], c[1], c[2]]))
}

/// The snap points a frame offers, asked for once and only when an end
/// needs them.
struct Snaps<'py> {
    get: Bound<'py, PyAny>,
    frame: Bound<'py, PyAny>,
    got: Option<(Vec<[f64; 3]>, Vec<[[f64; 3]; 2]>)>,
}

impl Snaps<'_> {
    fn load(&mut self) -> PyResult<&(Vec<[f64; 3]>, Vec<[[f64; 3]; 2]>)> {
        if self.got.is_none() {
            let r = self.get.call1((&self.frame,))?;
            let (wpts, edges) = if r.is_none() {
                (Vec::new(), Vec::new())
            } else {
                let w: PyReadonlyArray2<'_, f64> = r.get_item(0)?.extract()?;
                let e: PyReadonlyArray3<'_, f64> = r.get_item(1)?.extract()?;
                let (w, e) = (w.as_array(), e.as_array());
                ((0..w.nrows()).map(|i| [w[[i, 0]], w[[i, 1]], w[[i, 2]]]).collect(),
                 (0..e.shape()[0]).map(|i| [[e[[i, 0, 0]], e[[i, 0, 1]], e[[i, 0, 2]]], [e[[i, 1, 0]], e[[i, 1, 1]], e[[i, 1, 2]]]]).collect())
            };
            self.got = Some((wpts, edges));
        }
        Ok(self.got.as_ref().unwrap())
    }
}

/// One bound point: follow its hold, else re-snap it when it lies inside
/// the frame. Writes the item's world point; returns it.
#[allow(clippy::too_many_arguments)]
fn follow_point<'py>(py: Python<'py>, item: &Bound<'py, PyAny>, attr: &str, proj: &Projector, snaps: &mut Snaps<'py>,
                     follow: &Bound<'py, PyAny>, memo: &Bound<'py, PyDict>) -> PyResult<Option<[f64; 3]>> {
    let Some(w) = v3(&item.getattr(attr)?)? else { return Ok(None) };
    if follow.call1((item, attr))?.is_truthy()? {
        return v3(&item.getattr(attr)?);
    }
    if !proj.inside(proj.page(w)) {
        return Ok(Some(w));
    }
    let key = PyTuple::new(py, [item.getattr("_doc_key")?, attr.into_pyobject(py)?.into_any()])?;
    let held = match memo.get_item(&key)? {
        Some(m) => {
            let (pt, edge): (Vec<f64>, bool) = m.extract()?;
            (pt == w.to_vec()).then_some(edge)
        }
        None => None,
    };
    let (wpts, edges) = snaps.load()?;
    let (now, edge) = resnap(w, wpts, edges, proj.tol(), held);
    if let Some(e) = edge {
        memo.set_item(key, (now.to_vec(), e))?;
    }
    if now != w {
        item.setattr(attr, now.to_vec())?;
    }
    Ok(Some(now))
}

/// Make every item of sheet ``comp`` bound to a frame follow the model:
/// ``frames`` ``[{frame, view, base, bounds}]`` (the frame camera's inputs,
/// see ``sheet_frame_camera``), ``snaps(frame)`` → ``(world points (M,3),
/// edges (E,2,3))`` or None, ``follow(item, attr)`` → whether the model's
/// hold answered (it moved the point itself), ``memo`` what each point was
/// first seen on. Returns how many items it moved.
#[pyfunction]
pub fn sheet_follow_anchors<'py>(py: Python<'py>, comp: &Bound<'py, PyAny>, frames: Vec<Bound<'py, PyDict>>,
                                 snaps: Bound<'py, PyAny>, follow: Bound<'py, PyAny>, memo: Bound<'py, PyDict>)
    -> PyResult<usize> {
    py.import("core.doc_model")?.call_method0("flush")?;
    let mut by_uid: Vec<(String, Projector, Snaps<'py>)> = Vec::new();
    for d in &frames {
        let frame = d.get_item("frame")?.ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("frame"))?;
        let uid: String = frame.getattr("uid")?.extract()?;
        if uid.is_empty() {
            continue;
        }
        let (_f, proj) = Projector::of(&frame, d, false)?;
        by_uid.push((uid, proj, Snaps { get: snaps.clone(), frame: frame.clone(), got: None }));
    }
    let mut moved = 0;
    fn bound(by_uid: &[(String, Projector, Snaps<'_>)], item: &Bound<'_, PyAny>) -> Option<usize> {
        let uid: String = item.getattr("anchor_uid").and_then(|u| u.extract()).unwrap_or_default();
        by_uid.iter().position(|(u, _, _)| !uid.is_empty() && *u == uid)
    }
    let list = |name: &str| -> PyResult<Vec<Bound<'py, PyAny>>> {
        match comp.getattr_opt(name)? {
            Some(l) if !l.is_none() => l.try_iter()?.collect(),
            _ => Ok(Vec::new()),
        }
    };
    // Writes a field that changed; says whether it did.
    let set = |item: &Bound<'py, PyAny>, k: &str, v: f64| -> PyResult<bool> {
        let was: f64 = item.getattr(k)?.extract()?;
        if was != v {
            item.setattr(k, v)?;
        }
        Ok(was != v)
    };
    // Dimensions: both ends, then the item from its ends.
    for ct in list("cotas")? {
        let anchored = ct.getattr("anchored")?.is_truthy()?;
        let Some(i) = bound(&by_uid, &ct).filter(|_| anchored) else { continue };
        let (_, proj, snaps) = &mut by_uid[i];
        let a = follow_point(py, &ct, "a_world", proj, snaps, &follow, &memo)?;
        let b = follow_point(py, &ct, "b_world", proj, snaps, &follow, &memo)?;
        let (Some(a), Some(b)) = (a, b) else { continue };
        let (pa, pb) = (proj.page(a), proj.page(b));
        let changed = [set(&ct, "x_mm", pa[0])?, set(&ct, "y_mm", pa[1])?, set(&ct, "dx_mm", pb[0] - pa[0])?,
                       set(&ct, "dy_mm", pb[1] - pa[1])?, set(&ct, "scale_n", proj.scale_n)?];
        moved += usize::from(changed.iter().any(|c| *c));
    }
    // Labels: the pointed-at spot follows; the text stays. Then each bound leader.
    for et in list("etiquetas")? {
        let (x, y): (f64, f64) = (et.getattr("x_mm")?.extract()?, et.getattr("y_mm")?.extract()?);
        if et.getattr("anchored")?.is_truthy()? {
            if let Some(i) = bound(&by_uid, &et) {
                let (_, proj, snaps) = &mut by_uid[i];
                if let Some(a) = follow_point(py, &et, "a_world", proj, snaps, &follow, &memo)? {
                    let p = proj.page(a);
                    let changed = [set(&et, "ax_mm", p[0] - x)?, set(&et, "ay_mm", p[1] - y)?];
                    moved += usize::from(changed.iter().any(|c| *c));
                }
            }
        }
        let leaders = et.getattr("leaders")?;
        if leaders.is_none() || !leaders.is_truthy()? {
            continue;
        }
        let out = PyList::empty(py);
        let mut changed = false;
        for ld in leaders.try_iter()? {
            let ld = ld?;
            let copy = ld.call_method0("copy")?;
            let uid: String = ld.call_method1("get", ("anchor_uid", ""))?.extract::<Option<String>>()?.unwrap_or_default();
            let world = v3(&ld.call_method1("get", ("a_world",))?)?;
            if let (Some(i), Some(w)) = (by_uid.iter().position(|(u, _, _)| !uid.is_empty() && *u == uid), world) {
                let p = by_uid[i].1.page(w);
                let (ax, ay) = (p[0] - x, p[1] - y);
                let was: (f64, f64) = (ld.get_item("ax_mm")?.extract()?, ld.get_item("ay_mm")?.extract()?);
                if was != (ax, ay) {
                    copy.set_item("ax_mm", ax)?;
                    copy.set_item("ay_mm", ay)?;
                    changed = true;
                }
            }
            out.append(copy)?;
        }
        if changed {
            et.setattr("leaders", out)?;
            moved += 1;
        }
    }
    // Level marks: the point follows (and so does the height it reads); the
    // mark keeps its slide from the point.
    for nv in list("niveles")? {
        if !nv.getattr("anchored")?.is_truthy()? {
            continue;
        }
        let Some(i) = bound(&by_uid, &nv) else { continue };
        let (_, proj, snaps) = &mut by_uid[i];
        if let Some(a) = follow_point(py, &nv, "a_world", proj, snaps, &follow, &memo)? {
            let p = proj.page(a);
            let (ax, ay): (f64, f64) = (nv.getattr("ax_mm")?.extract()?, nv.getattr("ay_mm")?.extract()?);
            let changed = [set(&nv, "x_mm", p[0] - ax)?, set(&nv, "y_mm", p[1] - ay)?];
            moved += usize::from(changed.iter().any(|c| *c));
        }
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_corner_follows_a_near_point_and_an_edge_point_stays_on_its_edge() {
        let pts = [[1.0, 0.0, 0.0], [5.0, 5.0, 0.0]];
        let edges = [[[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]]];
        // A corner moved 2 cm: it follows.
        assert_eq!(resnap([1.02, 0.0, 0.0], &pts, &edges, 0.25, Some(false)).0, [1.0, 0.0, 0.0]);
        // On the edge and seen there first: it stays, a corner in reach or not.
        assert_eq!(resnap([1.03, 0.0, 0.0], &pts, &edges, 0.25, Some(true)).0, [1.03, 0.0, 0.0]);
        // Nothing in reach: it stays put.
        assert_eq!(resnap([9.0, 9.0, 0.0], &pts, &edges, 0.25, None).0, [9.0, 9.0, 0.0]);
    }
}
